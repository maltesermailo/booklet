//! The localhost-only web admin panel: server-rendered HTML, cookie sessions,
//! CSRF-guarded forms.
//!
//! An admin session is a **separate credential from a device token** — a
//! signed-in operator, not a synced laptop. It is checked by a different
//! extractor against a different table ([`admin_sessions`]), and `serve` binds it
//! to its own loopback listener. A `Bearer` device token can never open `/admin`;
//! an admin cookie is not a `Bearer` token, so the sync API's `Device` extractor
//! rejects it. Both facts are tested.
//!
//! The panel reads the whole box — users, devices, bytes, errors — but never note
//! content. That line is what keeps it small.

mod accounts;
mod actions;
mod pages;
mod plans;
mod session;
mod view;

use crate::auth;
use crate::billing::Stripe;
use crate::mail::Mailer;
use crate::store::{AdminSession, Store};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The session cookie's name and lifetime.
const COOKIE: &str = "booklet_admin";
const SESSION_TTL_SECONDS: i64 = 8 * 60 * 60;
/// One-time tokens (reset, invite) live an hour.
const TOKEN_TTL_SECONDS: i64 = 60 * 60;

/// Shared state for the admin routes: the store, the process start (for uptime),
/// the sign-in rate limiter, and the optional email/billing integrations (each
/// `None` when unconfigured — the panel degrades gracefully).
#[derive(Clone)]
pub struct AdminState {
    store: Arc<Store>,
    started: Instant,
    limiter: Arc<Mutex<session::Limiter>>,
    mailer: Arc<Option<Mailer>>,
    stripe: Arc<Option<Stripe>>,
    allow_registration: bool,
}

impl AdminState {
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
        state.allow_registration =
            matches!(env::var("BOOKLET_ALLOW_REGISTRATION").as_deref(), Ok("1") | Ok("true"));
        state
    }
}

/// Builds the `/admin` router. Serve it behind
/// `into_make_service_with_connect_info::<SocketAddr>()` so the sign-in limiter
/// can see the client address.
pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/admin", get(pages::overview))
        .route("/admin/login", get(session::login_form).post(session::login))
        .route("/admin/logout", post(session::logout))
        .route("/admin/theme", get(view::set_theme))
        .route("/admin/users", get(pages::users).post(actions::create_user))
        .route("/admin/users/{id}", get(pages::user_detail))
        .route("/admin/users/{id}/disable", post(actions::disable_user))
        .route("/admin/users/{id}/delete", get(pages::confirm_delete_user).post(actions::delete_user))
        .route("/admin/users/{id}/quota", post(actions::set_quota))
        .route("/admin/users/{id}/password", post(accounts::set_password))
        .route("/admin/users/{id}/email", post(accounts::set_email))
        .route("/admin/users/{id}/billing/checkout", post(actions::checkout))
        .route("/admin/users/{id}/billing/portal", post(actions::portal))
        .route("/admin/invites", post(accounts::send_invite))
        .route("/admin/devices", get(pages::devices))
        .route("/admin/devices/{id}/revoke", post(actions::revoke_device))
        .route("/admin/vaults", get(pages::vaults))
        .route("/admin/plans", get(plans::list).post(plans::create))
        .route("/admin/plans/{name}/update", post(plans::update))
        .route("/admin/plans/{name}/delete", post(plans::delete))
        .route("/admin/log", get(pages::log))
        // Public routes — no session (a stranger resets a password or registers).
        .route("/admin/forgot", get(accounts::forgot_form).post(accounts::forgot))
        .route("/admin/reset/{token}", get(accounts::reset_form).post(accounts::reset))
        .route("/register", get(accounts::register_form).post(accounts::register))
        .route("/billing/webhook", post(actions::stripe_webhook))
        .route("/admin/assets/panel.css", get(view::stylesheet))
        .route("/admin/assets/fonts/{file}", get(view::font))
        .with_state(state)
}

/// A live admin session, or a redirect to the sign-in page. Reads the session
/// cookie and resolves it against the store, where an expired session or a
/// no-longer-admin user reads as absent.
impl FromRequestParts<AdminState> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AdminState) -> Result<Self, Response> {
        let token = cookie(&parts.headers, COOKIE).ok_or_else(to_login)?;

        match state.store.admin_session(&auth::token_hash(&token)).await {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(to_login()),
            Err(error) => Err(internal(error)),
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
pub(crate) fn csrf_rejection(session: &AdminSession, provided: &str) -> Option<Response> {
    (provided != session.csrf).then(|| (StatusCode::FORBIDDEN, "CSRF check failed").into_response())
}

fn to_login() -> Response {
    Redirect::to("/admin/login").into_response()
}

/// A server-side failure: logged, returned opaquely.
pub(crate) fn internal(error: impl std::fmt::Display) -> Response {
    eprintln!("admin panel error: {error}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth, http};
    use axum::body::Body;
    use axum::http::header::{CONTENT_TYPE, SET_COOKIE};
    use axum::http::Request;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tower::ServiceExt;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn blob_root() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("booklet-admin-{}-{}", std::process::id(), unique))
    }

    fn state(store: Arc<Store>) -> AdminState {
        AdminState::new(store, Instant::now())
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
            .oneshot(form("POST", "/admin/login", None, &format!("handle={handle}&password=pw")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let set = response.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
        let token = set.split(';').next().unwrap().strip_prefix(&format!("{COOKIE}=")).unwrap().to_string();
        let csrf = store.admin_session(&auth::token_hash(&token)).await.unwrap().unwrap().csrf;
        (token, csrf)
    }

    #[sqlx::test]
    async fn the_panel_redirects_to_login_when_unauthenticated(pool: sqlx::PgPool) {
        let app = router(state(Arc::new(Store::from_parts(pool, blob_root(), 50))));

        let response = app.oneshot(get("/admin", None)).await.unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/admin/login");
    }

    #[sqlx::test]
    async fn a_device_token_cannot_open_the_panel(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        let user = store.create_user("alice", &auth::hash_password("pw").unwrap()).await.unwrap();
        let device_token = auth::new_token();
        store.create_device(user, "laptop", "macos", &auth::token_hash(&device_token)).await.unwrap();
        let app = router(state(store));

        // A live device token in the cookie slot is not an admin session.
        let response = app.oneshot(get("/admin", Some(&device_token))).await.unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/admin/login");
    }

    #[sqlx::test]
    async fn an_admin_cookie_cannot_reach_a_sync_route(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        let admin_app = router(state(store.clone()));
        let (token, _csrf) = sign_in_admin(&admin_app, &store, "alice").await;

        // The same cookie, sent to the bearer-token sync API, authenticates nothing.
        let sync_app = http::router(store);
        let request = Request::builder()
            .uri("/vaults")
            .header(header::COOKIE, format!("{COOKIE}={token}"))
            .body(Body::empty())
            .unwrap();
        let response = sync_app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn only_an_admin_may_sign_in(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        store.create_user("bob", &auth::hash_password("pw").unwrap()).await.unwrap(); // not an admin
        let app = router(state(store));

        let response = app.oneshot(form("POST", "/admin/login", None, "handle=bob&password=pw")).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn a_mutation_needs_a_matching_csrf_token_and_is_logged(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));
        let app = router(state(store.clone()));
        let (token, csrf) = sign_in_admin(&app, &store, "alice").await;

        // A wrong CSRF token is refused, and no user is created.
        let body = "csrf=wrong&handle=carol&password=pw";
        let refused = app.clone().oneshot(form("POST", "/admin/users", Some(&token), body)).await.unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert!(store.user_row(2).await.unwrap().is_none());

        // The matching token goes through and lands in the audit log.
        let body = format!("csrf={csrf}&handle=carol&password=pw");
        let ok = app.oneshot(form("POST", "/admin/users", Some(&token), &body)).await.unwrap();
        assert_eq!(ok.status(), StatusCode::SEE_OTHER);
        assert!(store.admin_login("carol").await.unwrap().is_some());

        let audit = store.recent_audit(10).await.unwrap();
        assert!(audit.iter().any(|row| row.action == "create-user" && row.detail == "carol"));
    }

    #[sqlx::test]
    async fn self_registration_is_gated_by_the_toggle(pool: sqlx::PgPool) {
        let store = Arc::new(Store::from_parts(pool, blob_root(), 50));

        // Off by default: registering creates nothing.
        let closed = router(state(store.clone()));
        let refused = closed.oneshot(form("POST", "/register", None, "handle=newbie&password=pw")).await.unwrap();
        assert_eq!(refused.status(), StatusCode::OK); // renders a "closed" card
        assert!(store.admin_login("newbie").await.unwrap().is_none());

        // On: a public sign-up creates a non-admin account.
        let mut open = state(store.clone());
        open.allow_registration = true;
        let app = router(open);
        let created = app.oneshot(form("POST", "/register", None, "handle=newbie&password=pw")).await.unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let account = store.admin_login("newbie").await.unwrap().unwrap();
        assert!(!account.2, "self-registered users must not be admins");
    }

    #[sqlx::test]
    async fn sign_in_is_rate_limited(pool: sqlx::PgPool) {
        let app = router(state(Arc::new(Store::from_parts(pool, blob_root(), 50))));

        // Ten wrong guesses are merely denied; the eleventh is throttled.
        for _ in 0..10 {
            let response = app
                .clone()
                .oneshot(form("POST", "/admin/login", None, "handle=alice&password=wrong"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let throttled = app.oneshot(form("POST", "/admin/login", None, "handle=alice&password=wrong")).await.unwrap();
        assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
