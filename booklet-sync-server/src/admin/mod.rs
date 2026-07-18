//! The whole server-rendered web frontend: the public marketing site, the user
//! account portal, and the operator admin area — all one app on one port.
//!
//! A **web session** cookie is a separate credential from a device token (a
//! signed-in person on the website, not a synced laptop): it is resolved by a
//! different extractor against a different table (`admin_sessions`), a `Bearer`
//! device token can never present as a session, and a session cookie is not a
//! `Bearer` token so the sync API's `Device` extractor rejects it. The operator
//! area (`/admin`) additionally requires `is_admin`; the admin never reads note
//! content.

mod accounts;
mod actions;
mod pages;
mod plans;
mod session;
mod site;
mod view;

use crate::auth;
use crate::billing::Stripe;
use crate::mail::Mailer;
use crate::store::{AdminSession, Store, WebSession};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use std::env;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub use site::routes;

/// The session cookie's name and lifetime.
const COOKIE: &str = "booklet_admin";
const SESSION_TTL_SECONDS: i64 = 8 * 60 * 60;
/// One-time tokens (reset, invite) live an hour.
const TOKEN_TTL_SECONDS: i64 = 60 * 60;

/// Shared state for the admin routes: the store, the process start (for uptime),
/// the sign-in rate limiter, and the optional email/billing integrations (each
/// `None` when unconfigured — the panel degrades gracefully).
#[derive(Clone)]
pub struct AppState {
    store: Arc<Store>,
    started: Instant,
    limiter: Arc<Mutex<session::Limiter>>,
    mailer: Arc<Option<Mailer>>,
    stripe: Arc<Option<Stripe>>,
    allow_registration: bool,
}

impl AppState {
    /// A minimal state — no email, no billing, registration off. Used by tests.
    pub fn new(store: Arc<Store>, started: Instant) -> Self {
        Self {
            store,
            started,
            limiter: Arc::new(Mutex::new(session::Limiter::default())),
            mailer: Arc::new(None),
            stripe: Arc::new(None),
            allow_registration: false,
        }
    }

    /// Layers the optional integrations on from the environment (`BOOKLET_SMTP_*`,
    /// `STRIPE_*`, `BOOKLET_ALLOW_REGISTRATION`). `public_url` is where the panel
    /// is reachable, for links in emails and Stripe return URLs.
    pub fn from_env(store: Arc<Store>, started: Instant, public_url: &str) -> Self {
        let mut state = Self::new(store, started);
        state.mailer = Arc::new(Mailer::from_env());
        state.stripe = Arc::new(Stripe::from_env(public_url));
        // Open by default — this is a public product site. An operator closes
        // sign-up explicitly.
        state.allow_registration = !matches!(env::var("BOOKLET_ALLOW_REGISTRATION").as_deref(), Ok("0") | Ok("false"));
        state
    }
}

/// Lets the sync handlers keep `State<Arc<Store>>` while the whole app shares one
/// `AppState`.
impl FromRef<AppState> for Arc<Store> {
    fn from_ref(state: &AppState) -> Self {
        state.store.clone()
    }
}

/// Resolves the request's session cookie to a live web session, or `None`.
async fn resolve_session(parts: &Parts, state: &AppState) -> Result<Option<WebSession>, Response> {
    let Some(token) = cookie(&parts.headers, COOKIE) else {
        return Ok(None);
    };

    state.store.web_session(&auth::token_hash(&token)).await.map_err(internal)
}

/// Any signed-in user (the portal). Absent session → redirect to sign-in.
impl FromRequestParts<AppState> for WebSession {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        resolve_session(parts, state).await?.ok_or_else(to_login)
    }
}

/// The session if there is one, or `None` — for public pages that render a
/// "Log in" vs "Account" nav without forcing sign-in.
pub struct MaybeSession(pub Option<WebSession>);

impl FromRequestParts<AppState> for MaybeSession {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        Ok(MaybeSession(resolve_session(parts, state).await?))
    }
}

/// A signed-in **admin** (the operator area). A signed-in non-admin is refused
/// with 403; nobody signed in is sent to sign-in.
impl FromRequestParts<AppState> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        match resolve_session(parts, state).await? {
            Some(session) if session.is_admin => {
                Ok(AdminSession { user_id: session.user_id, handle: session.handle, csrf: session.csrf })
            }
            Some(_) => Err((StatusCode::FORBIDDEN, "Admins only").into_response()),
            None => Err(to_login()),
        }
    }
}

// --- shared helpers ---

/// Reads one cookie value out of the `Cookie` header. One cookie, one header —
/// not worth a cookie-jar dependency.
pub(crate) fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

/// The rejection for a POST whose form CSRF token does not match the session's,
/// or `None` when it matches — the synchronizer-token defence for this
/// cookie-authenticated form surface.
pub(crate) fn csrf_rejection(expected: &str, provided: &str) -> Option<Response> {
    (provided != expected).then(|| (StatusCode::FORBIDDEN, "CSRF check failed").into_response())
}

fn to_login() -> Response {
    Redirect::to("/login").into_response()
}

/// A server-side failure: logged, returned opaquely.
pub(crate) fn internal(error: impl std::fmt::Display) -> Response {
    eprintln!("admin panel error: {error}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::app;
    use crate::auth;
    use axum::body::Body;
    use axum::http::header::{CONTENT_TYPE, SET_COOKIE};
    use axum::http::Request;
    use axum::Router;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tower::ServiceExt;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn blob_root() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("booklet-admin-{}-{}", std::process::id(), unique))
    }

    fn state(store: Arc<Store>) -> AppState {
        AppState::new(store, Instant::now())
    }

    fn form(method: &str, uri: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
        let mut builder =
            Request::builder().method(method).uri(uri).header(CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(token) = cookie {
            builder = builder.header(header::COOKIE, format!("{COOKIE}={token}"));
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(uri);
        if let Some(token) = cookie {
            builder = builder.header(header::COOKIE, format!("{COOKIE}={token}"));
        }
        builder.body(Body::empty()).unwrap()
    }

    /// Creates an admin, signs in, and returns (session token, csrf token).
    async fn sign_in_admin(app: &Router, store: &Store, handle: &str) -> (String, String) {
        store.create_user(handle, &auth::hash_password("pw").unwrap()).await.unwrap();
        store.grant_admin(handle).await.unwrap();

        let response = app
            .clone()
            .oneshot(form("POST", "/login", None, &format!("handle={handle}&password=pw")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let set = response.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
        let token = set.split(';').next().unwrap().strip_prefix(&format!("{COOKIE}=")).unwrap().to_string();
        let csrf = store.web_session(&auth::token_hash(&token)).await.unwrap().unwrap().csrf;
        (token, csrf)
    }

    #[sqlx::test]
    async fn the_admin_redirects_to_login_when_unauthenticated(pool: sqlx::PgPool) {
        let app = app(state(Arc::new(Store::from_parts(pool, blob_root(), 50))));

        let response = app.oneshot(get("/admin", None)).await.unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    }

    #[sqlx::test]
    async fn a_device_token_cannot_open_the_admin(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        let user = store.create_user("alice", &auth::hash_password("pw").unwrap()).await.unwrap();
        let device_token = auth::new_token();
        store.create_device(user, "laptop", "macos", &auth::token_hash(&device_token)).await.unwrap();
        let app = app(state(store));

        // A live device token in the cookie slot is not a web session.
        let response = app.oneshot(get("/admin", Some(&device_token))).await.unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    }

    #[sqlx::test]
    async fn a_session_cookie_cannot_reach_a_sync_route(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        let app = app(state(store.clone()));
        let (token, _csrf) = sign_in_admin(&app, &store, "alice").await;

        // The same cookie, sent to the bearer-token sync API, authenticates nothing.
        let request = Request::builder()
            .uri("/vaults")
            .header(header::COOKIE, format!("{COOKIE}={token}"))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn a_non_admin_signs_in_but_cannot_reach_the_admin(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        store.create_user("bob", &auth::hash_password("pw").unwrap()).await.unwrap(); // not an admin
        let app = app(state(store));

        // A normal user signs in fine (the portal is for everyone).
        let signed_in = app.clone().oneshot(form("POST", "/login", None, "handle=bob&password=pw")).await.unwrap();
        assert_eq!(signed_in.status(), StatusCode::SEE_OTHER);
        let set = signed_in.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
        let token = set.split(';').next().unwrap().strip_prefix(&format!("{COOKIE}=")).unwrap().to_string();

        // But the operator area refuses them.
        let admin = app.oneshot(get("/admin", Some(&token))).await.unwrap();
        assert_eq!(admin.status(), StatusCode::FORBIDDEN);
    }

    #[sqlx::test]
    async fn a_mutation_needs_a_matching_csrf_token_and_is_logged(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        let app = app(state(store.clone()));
        let (token, csrf) = sign_in_admin(&app, &store, "alice").await;

        // A wrong CSRF token is refused, and no user is created.
        let body = "csrf=wrong&handle=carol&password=pw";
        let refused = app.clone().oneshot(form("POST", "/admin/users", Some(&token), body)).await.unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert!(store.admin_login("carol").await.unwrap().is_none());

        // The matching token goes through and lands in the audit log.
        let body = format!("csrf={csrf}&handle=carol&password=pw");
        let ok = app.oneshot(form("POST", "/admin/users", Some(&token), &body)).await.unwrap();
        assert_eq!(ok.status(), StatusCode::SEE_OTHER);
        assert!(store.admin_login("carol").await.unwrap().is_some());

        let audit = store.recent_audit(10).await.unwrap();
        assert!(audit.iter().any(|row| row.action == "create-user" && row.detail == "carol"));
    }

    #[sqlx::test]
    async fn self_signup_is_gated_by_the_toggle(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));

        // Closed (the test state's default): signing up creates nothing.
        let closed = app(state(store.clone()));
        let refused = closed.oneshot(form("POST", "/signup", None, "handle=newbie&password=pw")).await.unwrap();
        assert_eq!(refused.status(), StatusCode::OK); // renders a "closed" card
        assert!(store.admin_login("newbie").await.unwrap().is_none());

        // Open: a public sign-up creates a non-admin account and logs it straight in.
        let mut open = state(store.clone());
        open.allow_registration = true;
        let app = app(open);
        let created = app.oneshot(form("POST", "/signup", None, "handle=newbie&password=pw")).await.unwrap();
        assert_eq!(created.status(), StatusCode::SEE_OTHER);
        assert!(created.headers().get(SET_COOKIE).is_some(), "sign-up logs the new account in");
        let account = store.admin_login("newbie").await.unwrap().unwrap();
        assert!(!account.2, "self-registered users must not be admins");
    }

    #[sqlx::test]
    async fn the_public_site_is_open_and_the_portal_needs_a_session(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        let app = app(state(store.clone()));

        // The landing page is public.
        assert_eq!(app.clone().oneshot(get("/", None)).await.unwrap().status(), StatusCode::OK);

        // The account portal redirects a stranger to sign-in.
        let anonymous = app.clone().oneshot(get("/account", None)).await.unwrap();
        assert_eq!(anonymous.status(), StatusCode::SEE_OTHER);
        assert_eq!(anonymous.headers().get(header::LOCATION).unwrap(), "/login");

        // A signed-in user reaches it.
        let (token, _csrf) = sign_in_admin(&app, &store, "alice").await;
        assert_eq!(app.oneshot(get("/account", Some(&token))).await.unwrap().status(), StatusCode::OK);
    }

    #[sqlx::test]
    async fn sign_in_is_rate_limited(pool: sqlx::PgPool) {
        let app = app(state(Arc::new(Store::from_parts(pool, blob_root(), 50))));

        // Ten wrong guesses are merely denied; the eleventh is throttled.
        for _ in 0..10 {
            let response = app
                .clone()
                .oneshot(form("POST", "/login", None, "handle=alice&password=wrong"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let throttled = app.oneshot(form("POST", "/login", None, "handle=alice&password=wrong")).await.unwrap();
        assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
