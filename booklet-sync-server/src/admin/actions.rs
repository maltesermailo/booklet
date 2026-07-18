//! The mutating POST handlers an operator drives. Each verifies the session's
//! CSRF token, makes one change, writes an audit-log row, and either redirects
//! (POST-redirect-GET) or renders a small notice. The public Stripe webhook lives
//! here too — it has no session, but is authenticated by its signature.

use super::pages::{confirm_delete_body, users_body};
use super::view::{self, Theme};
use super::{csrf_rejection, internal, AppState};
use crate::auth;
use crate::store::{self, AdminSession};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use maud::html;
use serde::Deserialize;
use sqlx::types::Uuid;

/// The bare CSRF token, for actions that carry no other field.
#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

#[derive(Deserialize)]
pub struct CreateUserForm {
    csrf: String,
    handle: String,
    #[serde(default)]
    email: String,
    password: String,
}

pub async fn create_user(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<CreateUserForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let handle = form.handle.trim();
    if handle.is_empty() {
        return reshow_users(&state, &session, Some("Handle cannot be empty.")).await;
    }

    let hash = match auth::hash_password(&form.password) {
        Ok(hash) => hash,
        Err(error) => return internal(error),
    };

    match state.store.create_user(handle, &hash).await {
        Ok(id) => {
            let email = form.email.trim();
            if !email.is_empty() {
                let _ = state.store.set_email(id, email).await;
            }
            let _ = state.store.log_event(&session.handle, "create-user", handle).await;
            Redirect::to("/admin/users").into_response()
        }
        Err(store::Error::Db(sqlx::Error::Database(db))) if db.is_unique_violation() => {
            reshow_users(&state, &session, Some("That handle is already taken.")).await
        }
        Err(error) => internal(error),
    }
}

pub async fn disable_user(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let handle = state.store.user_row(id).await.ok().flatten().map(|user| user.handle).unwrap_or_default();
    if let Err(error) = state.store.disable_user(id).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "disable-user", &handle).await;

    Redirect::to(&format!("/admin/users/{id}")).into_response()
}

#[derive(Deserialize)]
pub struct DeleteUserForm {
    csrf: String,
    confirm: String,
}

pub async fn delete_user(
    State(state): State<AppState>,
    session: AdminSession,
    Theme(theme): Theme,
    Path(id): Path<i64>,
    Form(form): Form<DeleteUserForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let impact = match state.store.delete_user_impact(id).await {
        Ok(Some(impact)) => impact,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => return internal(error),
    };

    // The typed handle must match, exactly, before anything is removed.
    if form.confirm.trim() != impact.handle {
        let body = confirm_delete_body(&session, id, &impact, Some("The typed handle did not match."));
        return (StatusCode::BAD_REQUEST, view::layout(theme, "Delete user", "/admin/users", &session, body))
            .into_response();
    }

    if let Err(error) = state.store.delete_user(id).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "delete-user", &impact.handle).await;

    Redirect::to("/admin/users").into_response()
}

pub async fn revoke_device(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    match state.store.revoke_device(id).await {
        Ok(Some(handle)) => {
            let _ = state.store.log_event(&session.handle, "revoke-device", &format!("{handle} #{id}")).await;
        }
        Ok(None) => {} // Already revoked or gone — nothing to log.
        Err(error) => return internal(error),
    }

    Redirect::to("/admin/devices").into_response()
}

// --- quota / plan ---

#[derive(Deserialize)]
pub struct QuotaForm {
    csrf: String,
    plan: String,
    quota_gib: String,
}

pub async fn set_quota(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<QuotaForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let plan = form.plan.trim();
    let quota = parse_gib(&form.quota_gib);

    if let Err(error) = state.store.set_plan(id, plan).await {
        return internal(error);
    }
    if let Err(error) = state.store.set_quota(id, quota).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "set-quota", &format!("id {id} plan {plan}")).await;

    Redirect::to(&format!("/admin/users/{id}")).into_response()
}

// --- billing (operator drives Stripe's hosted flows) ---

#[derive(Deserialize)]
pub struct PlanForm {
    csrf: String,
    plan: String,
}

pub async fn checkout(
    State(state): State<AppState>,
    session: AdminSession,
    Theme(theme): Theme,
    Path(id): Path<i64>,
    Form(form): Form<PlanForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let Some(stripe) = state.stripe.as_ref() else {
        return notice(theme, &session, "Billing is not configured (set STRIPE_SECRET_KEY).");
    };
    let billing = match state.store.user_billing(id).await {
        Ok(Some(billing)) => billing,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => return internal(error),
    };
    let Some(email) = billing.email else {
        return notice(theme, &session, "Set the user's email before creating a checkout link.");
    };
    let plan = form.plan.trim();
    let price = match state.store.plan_price(plan).await {
        Ok(Some(price)) => price,
        Ok(None) => return notice(theme, &session, &format!("Plan '{plan}' has no Stripe price configured.")),
        Err(error) => return internal(error),
    };

    match stripe.checkout_url(&email, &price, id, plan).await {
        Ok(url) => {
            let _ = state.store.log_event(&session.handle, "billing-checkout", &format!("id {id} {plan}")).await;
            link_notice(theme, &session, "Checkout link", &url)
        }
        Err(error) => notice(theme, &session, &format!("Stripe error: {error}")),
    }
}

pub async fn portal(
    State(state): State<AppState>,
    session: AdminSession,
    Theme(theme): Theme,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let Some(stripe) = state.stripe.as_ref() else {
        return notice(theme, &session, "Billing is not configured (set STRIPE_SECRET_KEY).");
    };
    let billing = match state.store.user_billing(id).await {
        Ok(Some(billing)) => billing,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => return internal(error),
    };
    let Some(customer) = billing.customer else {
        return notice(theme, &session, "No Stripe customer yet — the user must complete checkout first.");
    };

    match stripe.portal_url(&customer).await {
        Ok(url) => link_notice(theme, &session, "Customer portal link", &url),
        Err(error) => notice(theme, &session, &format!("Stripe error: {error}")),
    }
}

/// The Stripe webhook: public, but authenticated by its signature. It reconciles
/// our copy of a user's plan/status with Stripe's subscription events.
pub async fn stripe_webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(stripe) = state.stripe.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    let signature = headers.get("stripe-signature").and_then(|value| value.to_str().ok()).unwrap_or_default();
    if stripe.verify_webhook(&body, signature).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let event: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    apply_event(&state, &event).await;

    StatusCode::OK.into_response()
}

async fn apply_event(state: &AppState, event: &serde_json::Value) {
    let object = &event["data"]["object"];

    match event["type"].as_str().unwrap_or_default() {
        "checkout.session.completed" => {
            let user_id = object["client_reference_id"].as_str().and_then(|id| id.parse::<i64>().ok());
            let customer = object["customer"].as_str();
            let plan = object["metadata"]["plan"].as_str().unwrap_or("pro");
            if let (Some(user_id), Some(customer)) = (user_id, customer) {
                if state.store.bind_subscription(user_id, customer, plan).await.is_ok() {
                    let _ = state.store.log_event("stripe", "subscription-active", &format!("user {user_id} {plan}")).await;
                }
            }
        }
        "customer.subscription.updated" => {
            if let (Some(customer), Some(status)) = (object["customer"].as_str(), object["status"].as_str()) {
                if let Ok(Some(handle)) = state.store.set_subscription_status(customer, status).await {
                    let _ = state.store.log_event("stripe", "subscription-updated", &format!("{handle} {status}")).await;
                }
            }
        }
        "customer.subscription.deleted" => {
            if let Some(customer) = object["customer"].as_str() {
                if let Ok(Some(handle)) = state.store.set_subscription_by_customer(customer, "free", "canceled").await {
                    let _ = state.store.log_event("stripe", "subscription-canceled", &handle).await;
                }
            }
        }
        _ => {}
    }
}

// --- user edits (rename, admin flag, enable) ---

#[derive(Deserialize)]
pub struct HandleForm {
    csrf: String,
    handle: String,
}

pub async fn rename_user(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<HandleForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    let handle = form.handle.trim();
    if !handle.is_empty() {
        match state.store.rename_user(id, handle).await {
            Ok(_) => {
                let _ = state.store.log_event(&session.handle, "rename-user", &format!("id {id} → {handle}")).await;
            }
            Err(store::Error::Db(sqlx::Error::Database(db))) if db.is_unique_violation() => {}
            Err(error) => return internal(error),
        }
    }
    Redirect::to(&format!("/admin/users/{id}")).into_response()
}

pub async fn toggle_admin(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    let was_admin = state.store.user_row(id).await.ok().flatten().map(|user| user.is_admin).unwrap_or(false);
    if let Err(error) = state.store.set_admin(id, !was_admin).await {
        return internal(error);
    }
    let action = if was_admin { "revoke-admin" } else { "grant-admin" };
    let _ = state.store.log_event(&session.handle, action, &format!("id {id}")).await;

    Redirect::to(&format!("/admin/users/{id}")).into_response()
}

pub async fn enable_user(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    if let Err(error) = state.store.enable_user(id).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "enable-user", &format!("id {id}")).await;

    Redirect::to(&format!("/admin/users/{id}")).into_response()
}

pub async fn delete_device(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    if let Err(error) = state.store.delete_device(id).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "delete-device", &format!("#{id}")).await;

    Redirect::to("/admin/devices").into_response()
}

// --- vault edits/deletes (soft by default, hard behind a typed confirm) ---

#[derive(Deserialize)]
pub struct VaultNameForm {
    csrf: String,
    name: String,
}

pub async fn rename_vault(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<String>,
    Form(form): Form<VaultNameForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    let Ok(vault) = Uuid::parse_str(&id) else { return StatusCode::NOT_FOUND.into_response() };
    let name = form.name.trim();
    if !name.is_empty() {
        if let Err(error) = state.store.rename_vault(vault, name).await {
            return internal(error);
        }
        let _ = state.store.log_event(&session.handle, "rename-vault", name).await;
    }
    Redirect::to("/admin/vaults").into_response()
}

pub async fn delete_vault(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    let Ok(vault) = Uuid::parse_str(&id) else { return StatusCode::NOT_FOUND.into_response() };
    if let Err(error) = state.store.admin_soft_delete_vault(vault).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "delete-vault", &id).await;

    Redirect::to("/admin/vaults").into_response()
}

pub async fn restore_vault(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    let Ok(vault) = Uuid::parse_str(&id) else { return StatusCode::NOT_FOUND.into_response() };
    if let Err(error) = state.store.restore_vault(vault).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "restore-vault", &id).await;

    Redirect::to("/admin/vaults").into_response()
}

pub async fn purge_vault(
    State(state): State<AppState>,
    session: AdminSession,
    Theme(theme): Theme,
    Path(id): Path<String>,
    Form(form): Form<DeleteUserForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    let Ok(vault) = Uuid::parse_str(&id) else { return StatusCode::NOT_FOUND.into_response() };
    let Ok(Some((name, _owner))) = state.store.vault_label(vault).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    // Irreversible — the typed name must match.
    if form.confirm.trim() != name {
        let body = super::pages::confirm_purge_body(&session, &id, &name, Some("The typed name did not match."));
        return (StatusCode::BAD_REQUEST, view::layout(theme, "Delete vault", "/admin/vaults", &session, body))
            .into_response();
    }
    if let Err(error) = state.store.purge_vault(vault).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "purge-vault", &format!("{name} ({id})")).await;

    Redirect::to("/admin/vaults").into_response()
}

// --- helpers ---

async fn reshow_users(state: &AppState, session: &AdminSession, error: Option<&str>) -> Response {
    match state.store.list_users().await {
        Ok(users) => {
            let status = if error.is_some() { StatusCode::BAD_REQUEST } else { StatusCode::OK };
            let body = users_body(session, &users, state.mailer.is_some(), error);
            // Notices from actions render in the night theme by default; the page
            // still honours the cookie on the next navigation.
            (status, view::layout("night", "Users", "/admin/users", session, body)).into_response()
        }
        Err(error) => internal(error),
    }
}

/// Parses a GiB field into bytes; blank or unparseable means "no override".
pub(super) fn parse_gib(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    trimmed.parse::<f64>().ok().map(|gib| (gib * 1024.0 * 1024.0 * 1024.0) as i64)
}

fn notice(theme: &str, session: &AdminSession, message: &str) -> Response {
    let body = html! {
        div.head { h1 { "Billing" } }
        section { p { (message) } p { a href="/admin/users" { "Back to users" } } }
    };
    view::layout(theme, "Billing", "/admin/users", session, body).into_response()
}

fn link_notice(theme: &str, session: &AdminSession, title: &str, url: &str) -> Response {
    let body = html! {
        div.head { h1 { (title) } }
        section {
            p { "Send this link to the user:" }
            p.mono { a href=(url) { (url) } }
            p { a href="/admin/users" { "Back to users" } }
        }
    };
    view::layout(theme, title, "/admin/users", session, body).into_response()
}
