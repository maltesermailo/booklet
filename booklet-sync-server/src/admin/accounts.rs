//! Account flows: an operator setting a user's email or password and issuing
//! invites, plus the public email-driven password reset and self-registration.
//!
//! The public pages (`/admin/forgot`, `/admin/reset/{token}`, `/register`) carry
//! no session — a stranger reaches them — but each mutation is gated on a live,
//! single-use token or a config flag.

use super::view::{self, Theme};
use super::{csrf_rejection, internal, AdminState, TOKEN_TTL_SECONDS};
use crate::auth;
use crate::store::{self, AdminSession};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use maud::html;
use serde::Deserialize;

// --- operator actions on a user (session-guarded) ---

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf: String,
    password: String,
}

pub async fn set_password(
    State(state): State<AdminState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<PasswordForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session, &form.csrf) {
        return response;
    }
    if form.password.trim().is_empty() {
        return Redirect::to(&format!("/admin/users/{id}")).into_response();
    }

    let hash = match auth::hash_password(&form.password) {
        Ok(hash) => hash,
        Err(error) => return internal(error),
    };
    if let Err(error) = state.store.set_password(id, &hash).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "set-password", &format!("id {id}")).await;

    Redirect::to(&format!("/admin/users/{id}")).into_response()
}

#[derive(Deserialize)]
pub struct EmailForm {
    csrf: String,
    email: String,
}

pub async fn set_email(
    State(state): State<AdminState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<EmailForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session, &form.csrf) {
        return response;
    }

    // A duplicate email belongs to another account — a no-op, not a 500.
    match state.store.set_email(id, form.email.trim()).await {
        Ok(_) => {
            let _ = state.store.log_event(&session.handle, "set-email", &format!("id {id}")).await;
        }
        Err(store::Error::Db(sqlx::Error::Database(db))) if db.is_unique_violation() => {}
        Err(error) => return internal(error),
    }

    Redirect::to(&format!("/admin/users/{id}")).into_response()
}

#[derive(Deserialize)]
pub struct InviteForm {
    csrf: String,
    email: String,
}

pub async fn send_invite(
    State(state): State<AdminState>,
    session: AdminSession,
    Form(form): Form<InviteForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session, &form.csrf) {
        return response;
    }

    if let Some(mailer) = state.mailer.as_ref() {
        let email = form.email.trim();
        let token = auth::new_token();
        if state.store.create_invite(email, session.user_id, &auth::token_hash(&token), TOKEN_TTL_SECONDS).await.is_ok()
        {
            if let Err(error) = mailer.send_invite(email, &token).await {
                eprintln!("admin panel: invite email to {email} failed: {error}");
            }
            let _ = state.store.log_event(&session.handle, "invite", email).await;
        }
    }

    Redirect::to("/admin/users").into_response()
}

// --- public: forgot / reset password ---

pub async fn forgot_form(Theme(theme): Theme) -> Response {
    view::shell(
        theme,
        "Forgot password",
        html! {
            form.card method="post" action="/admin/forgot" {
                h1 { "Reset password" }
                label { "Email" input type="email" name="email" autocomplete="email" required autofocus; }
                button type="submit" { "Send reset link" }
                p.aside { a href="/admin/login" { "Back to sign in" } }
            }
        },
    )
    .into_response()
}

#[derive(Deserialize)]
pub struct ForgotForm {
    email: String,
}

pub async fn forgot(State(state): State<AdminState>, Theme(theme): Theme, Form(form): Form<ForgotForm>) -> Response {
    // Only send if email is configured and the address matches an account — but
    // the response never reveals which, so it cannot enumerate accounts.
    if let Some(mailer) = state.mailer.as_ref() {
        let email = form.email.trim();
        if let Ok(Some(user_id)) = state.store.find_user_by_email(email).await {
            let token = auth::new_token();
            if state.store.create_password_reset(user_id, &auth::token_hash(&token), TOKEN_TTL_SECONDS).await.is_ok() {
                if let Err(error) = mailer.send_reset(email, &token).await {
                    eprintln!("admin panel: reset email to {email} failed: {error}");
                }
            }
        }
    }

    view::shell(
        theme,
        "Forgot password",
        html! {
            div.card {
                h1 { "Check your email" }
                p { "If that address has an account, a reset link is on its way." }
                p.aside { a href="/admin/login" { "Back to sign in" } }
            }
        },
    )
    .into_response()
}

pub async fn reset_form(Theme(theme): Theme, Path(token): Path<String>) -> Response {
    view::shell(
        theme,
        "Reset password",
        html! {
            form.card method="post" action=(format!("/admin/reset/{token}")) {
                h1 { "Choose a new password" }
                label { "New password" input type="password" name="password" autocomplete="new-password" required autofocus; }
                button type="submit" { "Set password" }
            }
        },
    )
    .into_response()
}

#[derive(Deserialize)]
pub struct ResetForm {
    password: String,
}

pub async fn reset(
    State(state): State<AdminState>,
    Theme(theme): Theme,
    Path(token): Path<String>,
    Form(form): Form<ResetForm>,
) -> Response {
    match state.store.consume_password_reset(&auth::token_hash(&token)).await {
        Ok(Some(user_id)) => {
            let hash = match auth::hash_password(&form.password) {
                Ok(hash) => hash,
                Err(error) => return internal(error),
            };
            if let Err(error) = state.store.set_password(user_id, &hash).await {
                return internal(error);
            }
            let _ = state.store.log_event("system", "password-reset", &format!("user {user_id}")).await;

            done(theme, "Password updated", "You can now sign in.")
        }
        _ => view::shell(
            theme,
            "Reset password",
            html! {
                div.card {
                    h1 { "Link expired" }
                    p.error { "This reset link is invalid or already used." }
                    p.aside { a href="/admin/forgot" { "Request a new one" } }
                }
            },
        )
        .into_response(),
    }
}

// --- public: self-registration ---

#[derive(Deserialize)]
pub struct RegisterQuery {
    #[serde(default)]
    invite: String,
}

pub async fn register_form(
    State(state): State<AdminState>,
    Theme(theme): Theme,
    Query(query): Query<RegisterQuery>,
) -> Response {
    if !state.allow_registration && query.invite.is_empty() {
        return view::shell(
            theme,
            "Register",
            html! { div.card { h1 { "Registration closed" } p { "Ask an operator for an invite." } } },
        )
        .into_response();
    }

    view::shell(
        theme,
        "Register",
        html! {
            form.card method="post" action="/register" {
                h1 { "Create your Booklet account" }
                input type="hidden" name="invite" value=(query.invite);
                label { "Handle" input type="text" name="handle" autocomplete="username" required autofocus; }
                label { "Email" input type="email" name="email" autocomplete="email"; }
                label { "Password" input type="password" name="password" autocomplete="new-password" required; }
                button type="submit" { "Create account" }
            }
        },
    )
    .into_response()
}

#[derive(Deserialize)]
pub struct RegisterForm {
    handle: String,
    #[serde(default)]
    email: String,
    password: String,
    #[serde(default)]
    invite: String,
}

pub async fn register(State(state): State<AdminState>, Theme(theme): Theme, Form(form): Form<RegisterForm>) -> Response {
    // Gate: a live invite (whose email we adopt), or open registration.
    let invited_email = if !form.invite.is_empty() {
        match state.store.consume_invite(&auth::token_hash(&form.invite)).await {
            Ok(Some(email)) => Some(email),
            _ => return register_error(theme, "That invite is invalid or expired."),
        }
    } else if state.allow_registration {
        None
    } else {
        return register_error(theme, "Registration is closed.");
    };

    let handle = form.handle.trim();
    if handle.is_empty() {
        return register_error(theme, "Handle cannot be empty.");
    }

    let hash = match auth::hash_password(&form.password) {
        Ok(hash) => hash,
        Err(error) => return internal(error),
    };
    let id = match state.store.create_user(handle, &hash).await {
        Ok(id) => id,
        Err(store::Error::Db(sqlx::Error::Database(db))) if db.is_unique_violation() => {
            return register_error(theme, "That handle is taken.");
        }
        Err(error) => return internal(error),
    };

    let email = invited_email.unwrap_or_else(|| form.email.trim().to_string());
    if !email.is_empty() {
        let _ = state.store.set_email(id, &email).await;
    }
    let _ = state.store.log_event("system", "register", handle).await;

    done(theme, "Welcome", "Your account is ready.")
}

fn register_error(theme: &str, message: &str) -> Response {
    view::shell(theme, "Register", html! { div.card { h1 { "Register" } p.error { (message) } p.aside { a href="/register" { "Try again" } } } })
        .into_response()
}

fn done(theme: &str, title: &str, message: &str) -> Response {
    view::shell(theme, title, html! { div.card { h1 { (title) } p { (message) } p.aside { a href="/admin/login" { "Sign in" } } } })
        .into_response()
}
