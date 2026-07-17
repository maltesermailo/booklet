//! Sign-in, sign-out, and the in-memory sign-in rate limiter.

use super::view::Theme;
use super::{cookie, csrf_rejection, internal, view, AdminState, COOKIE, SESSION_TTL_SECONDS};
use crate::auth;
use crate::store::AdminSession;
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
    State(state): State<AdminState>,
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
    let (id, password_hash, is_admin, disabled) = account;

    // One generic failure for every reason, so the form never reveals whether a
    // handle exists or holds admin. Only an admin may sign in here.
    if disabled || !is_admin || !auth::verify_password(&form.password, &password_hash) {
        return denied(theme);
    }

    let token = auth::new_token();
    let csrf = auth::new_token();
    if let Err(error) =
        state.store.create_admin_session(id, &auth::token_hash(&token), &csrf, SESSION_TTL_SECONDS).await
    {
        return internal(error);
    }
    let _ = state.store.log_event(&form.handle, "sign-in", "").await;

    ([(header::SET_COOKIE, session_cookie(&token))], Redirect::to("/admin")).into_response()
}

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn logout(
    State(state): State<AdminState>,
    session: AdminSession,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session, &form.csrf) {
        return response;
    }

    if let Some(token) = cookie(&headers, COOKIE) {
        let _ = state.store.delete_admin_session(&auth::token_hash(&token)).await;
    }
    let _ = state.store.log_event(&session.handle, "sign-out", "").await;

    ([(header::SET_COOKIE, clear_cookie())], Redirect::to("/admin/login")).into_response()
}

fn denied(theme: &str) -> Response {
    (StatusCode::UNAUTHORIZED, view::login_page(theme, Some("Invalid handle or password."))).into_response()
}

fn session_cookie(token: &str) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={SESSION_TTL_SECONDS}")
}

fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0")
}
