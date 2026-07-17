//! The axum router, its handlers, and the bearer-token auth boundary.
//!
//! Every route but `POST /auth/token` requires a device token; every
//! `/vaults/{id}/…` route additionally checks the token's user owns that vault,
//! answering 404 (never 403) so the server does not confirm a vault it will not
//! serve. Blobs are content-addressed and global, so their routes require a token
//! but no per-vault check.
//!
//! Handlers are thin: they authenticate, authorize, call [`Store`], and map the
//! result to a status. Content is addressed by hash throughout — the delta store
//! underneath is invisible here.

use crate::auth;
use crate::store::{Error, PutOutcome, Store};
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use booklet_sync_proto as proto;
use serde::Deserialize;
use sqlx::types::Uuid;
use std::sync::Arc;

type AppState = Arc<Store>;

/// Builds the router over a shared store.
pub fn router(store: Arc<Store>) -> Router {
    Router::new()
        .route("/auth/token", post(issue_token))
        .route("/vaults", get(list_vaults).post(publish_vault))
        .route("/vaults/{id}/changes", get(changes))
        .route("/vaults/{id}/entities/{*path}", put(put_entity).delete(delete_entity))
        .route("/vaults/{id}/history/{*path}", get(history))
        .route("/blobs/{hash}", put(put_blob).get(get_blob))
        .with_state(store)
}

/// An authenticated device, extracted from the `Authorization: Bearer` token.
struct Device {
    user_id: i64,
    #[allow(dead_code)] // attributed to history rows in a later slice
    device_id: i64,
}

impl FromRequestParts<AppState> for Device {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, store: &AppState) -> Result<Self, Response> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;

        match store.device_for_token(&auth::token_hash(token)).await {
            Ok(Some((device_id, user_id))) => Ok(Device { user_id, device_id }),
            Ok(None) => Err(StatusCode::UNAUTHORIZED.into_response()),
            Err(error) => Err(internal(error)),
        }
    }
}

// --- auth ---

async fn issue_token(State(store): State<AppState>, Json(request): Json<proto::TokenRequest>) -> Response {
    let user = match store.find_user_by_handle(&request.handle).await {
        Ok(Some(user)) => user,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(error) => return internal(error),
    };

    let (user_id, password_hash, disabled) = user;
    if disabled || !auth::verify_password(&request.password, &password_hash) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let token = auth::new_token();
    if let Err(error) = store
        .create_device(user_id, &request.device_name, &request.platform, &auth::token_hash(&token))
        .await
    {
        return internal(error);
    }

    Json(proto::TokenResponse { token, user: request.handle }).into_response()
}

// --- vaults ---

async fn list_vaults(State(store): State<AppState>, device: Device) -> Response {
    match store.list_vaults(device.user_id).await {
        Ok(vaults) => Json(vaults).into_response(),
        Err(error) => internal(error),
    }
}

async fn publish_vault(
    State(store): State<AppState>,
    device: Device,
    Json(request): Json<proto::PublishRequest>,
) -> Response {
    match store.create_vault(device.user_id, &request.name).await {
        Ok(id) => Json(proto::PublishResponse { id: id.to_string() }).into_response(),
        Err(error) => internal(error),
    }
}

#[derive(Deserialize)]
struct ChangesQuery {
    #[serde(default)]
    since: u64,
}

async fn changes(
    State(store): State<AppState>,
    device: Device,
    Path(id): Path<String>,
    Query(query): Query<ChangesQuery>,
) -> Response {
    let vault = match authorize(&store, &device, &id).await {
        Ok(vault) => vault,
        Err(response) => return response,
    };

    match store.changes_since(vault, query.since).await {
        Ok(changes) => Json(changes).into_response(),
        Err(error) => internal(error),
    }
}

async fn put_entity(
    State(store): State<AppState>,
    device: Device,
    Path((id, path)): Path<(String, String)>,
    Json(request): Json<proto::PutRequest>,
) -> Response {
    let vault = match authorize(&store, &device, &id).await {
        Ok(vault) => vault,
        Err(response) => return response,
    };

    let outcome = store
        .apply_put_ref(
            vault,
            &path,
            request.kind,
            request.base_version,
            request.blob.as_deref(),
            request.moved_from.as_deref(),
        )
        .await;

    put_response(outcome)
}

/// `base_version` rides as a query parameter, not a body: DELETE-with-body is
/// widely discouraged, and some clients cannot send one.
#[derive(Deserialize)]
struct DeleteQuery {
    base_version: u64,
}

async fn delete_entity(
    State(store): State<AppState>,
    device: Device,
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DeleteQuery>,
) -> Response {
    let vault = match authorize(&store, &device, &id).await {
        Ok(vault) => vault,
        Err(response) => return response,
    };

    put_response(store.apply_delete(vault, &path, query.base_version).await)
}

async fn history(
    State(store): State<AppState>,
    device: Device,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let vault = match authorize(&store, &device, &id).await {
        Ok(vault) => vault,
        Err(response) => return response,
    };

    match store.history(vault, &path).await {
        Ok(history) => Json(history).into_response(),
        Err(error) => internal(error),
    }
}

// --- blobs (global, authenticated but not vault-scoped) ---

async fn put_blob(
    State(store): State<AppState>,
    _device: Device,
    Path(hash): Path<String>,
    body: Bytes,
) -> Response {
    // The blob's name must be its content hash, or the store is not content-
    // addressed. This is the one check that keeps a blob's name honest.
    if crate::blob::hash(&body) != hash {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match store.stage_blob(&body).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal(error),
    }
}

async fn get_blob(State(store): State<AppState>, _device: Device, Path(hash): Path<String>) -> Response {
    match store.has_blob(&hash).await {
        Ok(true) => match store.get_blob(&hash).await {
            Ok(bytes) => bytes.into_response(),
            Err(error) => internal(error),
        },
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => internal(error),
    }
}

// --- helpers ---

/// Parses the vault id and checks the device owns it, or returns the response to
/// send back (404 for a bad id, a missing vault, or another user's vault).
async fn authorize(store: &Store, device: &Device, id: &str) -> Result<Uuid, Response> {
    let vault = Uuid::parse_str(id).map_err(|_| StatusCode::NOT_FOUND.into_response())?;

    match store.vault_owner(vault).await {
        Ok(Some(owner)) if owner == device.user_id => Ok(vault),
        Ok(_) => Err(StatusCode::NOT_FOUND.into_response()),
        Err(error) => Err(internal(error)),
    }
}

fn put_response(outcome: Result<PutOutcome, Error>) -> Response {
    match outcome {
        Ok(PutOutcome::Applied(response)) => Json(response).into_response(),
        Ok(PutOutcome::Conflict(conflict)) => (StatusCode::CONFLICT, Json(conflict)).into_response(),
        Err(Error::MissingBlob(_)) => StatusCode::BAD_REQUEST.into_response(),
        Err(error) => internal(error),
    }
}

/// A server-side failure: logged, and returned opaquely so no internals leak.
fn internal(error: impl std::fmt::Display) -> Response {
    eprintln!("sync server error: {error}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn blob_root() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("booklet-http-{}-{}", std::process::id(), unique))
    }

    async fn json<T: serde::de::DeserializeOwned>(response: Response) -> T {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn get(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn json_request(method: &str, uri: &str, token: &str, body: &impl serde::Serialize) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap()
    }

    #[sqlx::test]
    async fn requests_without_a_valid_token_are_rejected(pool: sqlx::PgPool) {
        let app = router(Arc::new(Store::from_parts(pool, blob_root(), 50)));

        let anonymous = app
            .clone()
            .oneshot(Request::builder().uri("/vaults").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

        let bad_token = app.oneshot(get("/vaults", "not-a-real-token")).await.unwrap();
        assert_eq!(bad_token.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn a_device_signs_in_publishes_and_syncs_a_note(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        store.create_user("alice", &auth::hash_password("secret").unwrap()).await.unwrap();
        let app = router(store);

        // Sign in for a token.
        let request = proto::TokenRequest {
            handle: "alice".into(),
            password: "secret".into(),
            device_name: "laptop".into(),
            platform: "macos".into(),
        };
        let response = app.clone().oneshot(json_request("POST", "/auth/token", "", &request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let token: proto::TokenResponse = json(response).await;

        // Publish a vault.
        let response = app
            .clone()
            .oneshot(json_request("POST", "/vaults", &token.token, &proto::PublishRequest { name: "Personal".into() }))
            .await
            .unwrap();
        let vault: proto::PublishResponse = json(response).await;

        // Upload a note's content, then commit the entity referencing it.
        let content = b"# Note\n\nsynced over http\n";
        let hash = crate::blob::hash(content);
        let upload = Request::builder()
            .method("PUT")
            .uri(format!("/blobs/{hash}"))
            .header(header::AUTHORIZATION, format!("Bearer {}", token.token))
            .body(Body::from(content.to_vec()))
            .unwrap();
        assert_eq!(app.clone().oneshot(upload).await.unwrap().status(), StatusCode::NO_CONTENT);

        let put = proto::PutRequest {
            kind: proto::EntityKind::Note,
            base_version: 0,
            blob: Some(hash.clone()),
            moved_from: None,
        };
        let response = app
            .clone()
            .oneshot(json_request("PUT", &format!("/vaults/{}/entities/Note.md", vault.id), &token.token, &put))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // The feed shows the note, and the blob reads back byte-for-byte.
        let response = app
            .clone()
            .oneshot(get(&format!("/vaults/{}/changes?since=0", vault.id), &token.token))
            .await
            .unwrap();
        let changes: proto::Changes = json(response).await;
        assert_eq!(changes.changes.len(), 1);
        assert_eq!(changes.changes[0].path, "Note.md");
        assert_eq!(changes.changes[0].blob.as_deref(), Some(hash.as_str()));

        let response = app.oneshot(get(&format!("/blobs/{hash}"), &token.token)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), content);
    }

    #[sqlx::test]
    async fn another_users_vault_is_not_found(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        // Alice owns a vault; Bob holds a valid token but does not.
        let alice = store.create_user("alice", &auth::hash_password("a").unwrap()).await.unwrap();
        let alice_vault = store.create_vault(alice, "Personal").await.unwrap();
        store.create_user("bob", &auth::hash_password("b").unwrap()).await.unwrap();
        let app = router(store);

        let sign_in = proto::TokenRequest {
            handle: "bob".into(),
            password: "b".into(),
            device_name: "phone".into(),
            platform: "android".into(),
        };
        let response = app.clone().oneshot(json_request("POST", "/auth/token", "", &sign_in)).await.unwrap();
        let bob: proto::TokenResponse = json(response).await;

        let response = app
            .oneshot(get(&format!("/vaults/{alice_vault}/changes?since=0"), &bob.token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
