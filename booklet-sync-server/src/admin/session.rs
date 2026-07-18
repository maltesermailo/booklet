//! Sign-in, sign-out, and the in-memory sign-in rate limiter.

use super::view::Theme;
use super::{cookie, csrf_rejection, internal, view, AppState, COOKIE, SESSION_TTL_SECONDS};
use crate::auth;
use crate::store::WebSession;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use maud::Markup;
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// The client address for rate limiting, from the connect-info service when the
/// server is bound to a socket, and loopback otherwise (e.g. an in-process test).
/// Never fails, so a page always renders.
pub(crate) struct ClientIp(IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Infallible> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));

        Ok(ClientIp(ip))
    }
}

const MAX_ATTEMPTS: u32 = 10;
const WINDOW: Duration = Duration::from_secs(60);

/// A fixed-window sign-in limiter keyed by client address. Sign-in is the one
/// admin surface a stranger can reach, so guesses are capped — in memory, no
/// dependency and no table.
#[derive(Default)]
pub struct Limiter {
    windows: HashMap<IpAddr, (u32, Instant)>,
}

impl Limiter {
    fn allow(&mut self, ip: IpAddr, now: Instant) -> bool {
        let window = self.windows.entry(ip).or_insert((0, now));
        if now.duration_since(window.1) > WINDOW {
            *window = (0, now);
        }
        window.0 += 1;

        window.0 <= MAX_ATTEMPTS
    }
}

pub async fn login_form(Theme(theme): Theme) -> Markup {
    view::login_page(theme, None)
}

#[derive(Deserialize)]
pub struct LoginForm {
    handle: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Theme(theme): Theme,
    Form(form): Form<LoginForm>,
) -> Response {
    if !state.limiter.lock().unwrap().allow(ip, Instant::now()) {
        let page = view::login_page(theme, Some("Too many attempts. Wait a minute."));
        return (StatusCode::TOO_MANY_REQUESTS, page).into_response();
    }

    let account = match state.store.admin_login(&form.handle).await {
        Ok(Some(account)) => account,
        Ok(None) => return denied(theme),
        Err(error) => return internal(error),
    };
    let (id, password_hash, _is_admin, disabled) = account;

    // One generic failure for every reason, so the form never reveals whether a
    // handle exists. Any non-disabled user may sign in; the operator gate lives
    // on the /admin routes, not here.
    if disabled || !auth::verify_password(&form.password, &password_hash) {
        return denied(theme);
    }

    let cookie = match open_session(&state, id).await {
        Ok(cookie) => cookie,
        Err(response) => return response,
    };
    let _ = state.store.log_event(&form.handle, "sign-in", "").await;

    ([(header::SET_COOKIE, cookie)], Redirect::to("/account")).into_response()
}

/// Creates a session row and returns the `Set-Cookie` value for it. Shared by
/// sign-in and sign-up (which logs a new account straight in).
pub(super) async fn open_session(state: &AppState, user_id: i64) -> Result<String, Response> {
    let token = auth::new_token();
    let csrf = auth::new_token();
    state
        .store
        .create_admin_session(user_id, &auth::token_hash(&token), &csrf, SESSION_TTL_SECONDS)
        .await
        .map_err(internal)?;

    Ok(session_cookie(&token))
}

#[derive(Deserialize)]
pub struct CsrfForm {
    pub(super) csrf: String,
}

pub async fn logout(
    State(state): State<AppState>,
    session: WebSession,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    if let Some(token) = cookie(&headers, COOKIE) {
        let _ = state.store.delete_admin_session(&auth::token_hash(&token)).await;
    }
    let _ = state.store.log_event(&session.handle, "sign-out", "").await;

    ([(header::SET_COOKIE, clear_cookie())], Redirect::to("/login")).into_response()
}

fn denied(theme: &str) -> Response {
    (StatusCode::UNAUTHORIZED, view::login_page(theme, Some("Invalid handle or password."))).into_response()
}

// SameSite=Lax (not Strict) so the session survives the top-level redirect back
// from Stripe Checkout; POST forms are still protected by the CSRF token.
fn session_cookie(token: &str) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={SESSION_TTL_SECONDS}")
}

fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0")
}
