//! The public product site and the user account portal, plus the one router that
//! wires the whole web frontend (site + portal + operator admin + auth + assets +
//! the Stripe webhook) onto a single app.

use super::session::open_session;
use super::view::{self, Theme};
use super::{accounts, actions, csrf_rejection, internal, pages, plans, session, AppState, MaybeSession};
use crate::store::{self, PlanRow, WebSession};
use crate::auth;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use maud::{html, Markup};
use serde::Deserialize;

/// The whole web frontend as one `Router`. `app()` applies the shared state.
pub fn routes() -> Router<AppState> {
    Router::new()
        // Public marketing + auth.
        .route("/", get(landing))
        .route("/pricing", get(pricing))
        .route("/signup", get(signup_form).post(signup))
        .route("/login", get(session::login_form).post(session::login))
        .route("/logout", post(session::logout))
        .route("/forgot", get(accounts::forgot_form).post(accounts::forgot))
        .route("/reset/{token}", get(accounts::reset_form).post(accounts::reset))
        .route("/theme", get(view::set_theme))
        .route("/assets/panel.css", get(view::stylesheet))
        .route("/assets/fonts/{file}", get(view::font))
        .route("/billing/webhook", post(actions::stripe_webhook))
        // User account portal.
        .route("/account", get(account))
        .route("/account/email", post(set_email))
        .route("/account/password", post(set_password))
        .route("/account/subscribe", post(subscribe))
        .route("/account/portal", post(portal))
        // Operator admin (each handler additionally requires the AdminSession).
        .route("/admin", get(pages::overview))
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
}

// --- marketing ---

pub async fn landing(
    Theme(theme): Theme,
    MaybeSession(session): MaybeSession,
    State(state): State<AppState>,
) -> Response {
    let plans = state.store.list_plans().await.unwrap_or_default();

    let body = html! {
        section.hero {
            h1 { "Your library, bound in software." }
            p.lead { "Booklet is a personal note library — a live-preview editor over plain Markdown, wiki-links that weave your notes together, and offline-first sync you can host yourself." }
            div.cta {
                @if session.is_some() {
                    a.button href="/account" { "Go to your account" }
                } @else {
                    a.button href="/signup" { "Get started" }
                    a.ghost href="/pricing" { "See plans" }
                }
            }
        }
        section.features {
            (feature("Live preview", "Click into any block to edit its Markdown; everywhere else it reads like a finished page."))
            (feature("Linked thought", "[[Wiki-links]] and backlinks turn a folder of notes into a graph you can walk."))
            (feature("Offline-first sync", "Write on a plane; reconcile on reconnect. Three-way merge, with version history behind it."))
            (feature("Yours to host", "Plain Markdown on disk, an open sync protocol, and a server you run."))
        }
        section id="pricing" {
            h2 { "Pricing" }
            (pricing_cards(&plans, session.as_ref()))
        }
    };

    view::site_layout(theme, "Booklet — your library, bound in software", session.as_ref(), body).into_response()
}

pub async fn pricing(
    Theme(theme): Theme,
    MaybeSession(session): MaybeSession,
    State(state): State<AppState>,
) -> Response {
    let plans = state.store.list_plans().await.unwrap_or_default();
    let body = html! {
        div.head { h1 { "Pricing" } p.sub { "Every plan syncs across your devices. Upgrade for more room." } }
        (pricing_cards(&plans, session.as_ref()))
    };

    view::site_layout(theme, "Pricing", session.as_ref(), body).into_response()
}

/// Plan cards. A logged-in visitor gets a Subscribe button for purchasable plans;
/// a stranger gets a sign-up CTA.
fn pricing_cards(plans: &[PlanRow], session: Option<&WebSession>) -> Markup {
    html! {
        div.plans {
            @for plan in plans {
                div.plan {
                    h3 { (plan.name) }
                    p.price {
                        @match plan.price_cents {
                            Some(cents) => { "$" (dollars(cents)) span.per { "/mo" } }
                            None => { "Free" }
                        }
                    }
                    p.quota { (view::bytes(plan.quota_bytes as u64)) " storage" }
                    @if let Some(description) = &plan.description { p.desc { (description) } }
                    div.cta {
                        @match session {
                            Some(session) if plan.stripe_price_id.is_some() => {
                                form method="post" action="/account/subscribe" {
                                    input type="hidden" name="csrf" value=(session.csrf);
                                    input type="hidden" name="plan" value=(plan.name);
                                    button type="submit" { "Subscribe" }
                                }
                            }
                            Some(_) => { span.dim { "Included" } }
                            None => { a.button href="/signup" { "Sign up" } }
                        }
                    }
                }
            }
        }
    }
}

// --- account portal ---

pub async fn account(Theme(theme): Theme, session: WebSession, State(state): State<AppState>) -> Response {
    render_account(&state, &session, theme, None).await
}

async fn render_account(state: &AppState, session: &WebSession, theme: &str, error: Option<&str>) -> Response {
    let billing = match state.store.user_billing(session.user_id).await {
        Ok(Some(billing)) => billing,
        Ok(None) => return internal("account vanished"),
        Err(error) => return internal(error),
    };
    let usage = state.store.user_usage_bytes(session.user_id).await.unwrap_or(0);
    let limit = state.store.user_effective_quota(session.user_id).await.unwrap_or(None);
    let plans = state.store.list_plans().await.unwrap_or_default();
    let percent = match limit {
        Some(limit) if limit > 0 => (usage as f64 / limit as f64 * 100.0).min(100.0),
        _ => 0.0,
    };

    let body = html! {
        div.head {
            h1 { "Your account" }
            p.sub { "Signed in as " b { (session.handle) }
                    @if session.is_admin { " · " a href="/admin" { "Operator panel" } } }
        }
        @if let Some(message) = error { p.error { (message) } }

        section {
            h2 { "Storage" }
            div.meter { div.fill style=(format!("width:{percent:.0}%")) {} }
            p.sub { (view::bytes(usage as u64)) " of "
                    @match limit { Some(limit) => (view::bytes(limit as u64)), None => "unlimited" }
                    " · plan " b { (billing.plan) }
                    @if let Some(status) = &billing.status { " · " (status) } }
        }

        section {
            h2 { "Plans" }
            (pricing_cards(&plans, Some(session)))
            @if billing.customer.is_some() {
                form.inline method="post" action="/account/portal" {
                    input type="hidden" name="csrf" value=(session.csrf);
                    button type="submit" { "Manage billing" }
                }
            }
        }

        section {
            h2 { "Email" }
            form.inline method="post" action="/account/email" {
                input type="hidden" name="csrf" value=(session.csrf);
                input type="email" name="email" value=(billing.email.clone().unwrap_or_default()) placeholder="you@example.com";
                button type="submit" { "Save" }
            }
        }

        section {
            h2 { "Password" }
            form.inline method="post" action="/account/password" {
                input type="hidden" name="csrf" value=(session.csrf);
                input type="password" name="password" placeholder="new password" autocomplete="new-password" required;
                button type="submit" { "Change" }
            }
        }
    };

    view::site_layout(theme, "Account", Some(session), body).into_response()
}

#[derive(Deserialize)]
pub struct EmailForm {
    csrf: String,
    email: String,
}

pub async fn set_email(session: WebSession, State(state): State<AppState>, Form(form): Form<EmailForm>) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    match state.store.set_email(session.user_id, form.email.trim()).await {
        Ok(_) | Err(store::Error::Db(sqlx::Error::Database(_))) => {}
        Err(error) => return internal(error),
    }
    Redirect::to("/account").into_response()
}

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf: String,
    password: String,
}

pub async fn set_password(
    session: WebSession,
    State(state): State<AppState>,
    Form(form): Form<PasswordForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }
    if form.password.trim().is_empty() {
        return Redirect::to("/account").into_response();
    }

    let hash = match auth::hash_password(&form.password) {
        Ok(hash) => hash,
        Err(error) => return internal(error),
    };
    // This revokes the user's device tokens and other sessions (including this
    // one), so re-issue a fresh cookie to keep them signed in on the web.
    if let Err(error) = state.store.set_password(session.user_id, &hash).await {
        return internal(error);
    }
    let cookie = match open_session(&state, session.user_id).await {
        Ok(cookie) => cookie,
        Err(response) => return response,
    };

    ([(header::SET_COOKIE, cookie)], Redirect::to("/account")).into_response()
}

// --- self-service billing ---

#[derive(Deserialize)]
pub struct SubscribeForm {
    csrf: String,
    plan: String,
}

pub async fn subscribe(
    Theme(theme): Theme,
    session: WebSession,
    State(state): State<AppState>,
    Form(form): Form<SubscribeForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let Some(stripe) = state.stripe.as_ref() else {
        return render_account(&state, &session, theme, Some("Billing isn't available right now.")).await;
    };
    let email = match state.store.user_billing(session.user_id).await {
        Ok(Some(billing)) => billing.email,
        Ok(None) => return internal("account vanished"),
        Err(error) => return internal(error),
    };
    let Some(email) = email else {
        return render_account(&state, &session, theme, Some("Add your email below before subscribing.")).await;
    };
    let plan = form.plan.trim();
    let price = match state.store.plan_price(plan).await {
        Ok(Some(price)) => price,
        Ok(None) => return render_account(&state, &session, theme, Some("That plan isn't purchasable.")).await,
        Err(error) => return internal(error),
    };

    match stripe.checkout_url(&email, &price, session.user_id, plan).await {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(error) => render_account(&state, &session, theme, Some(&format!("Payment error: {error}"))).await,
    }
}

pub async fn portal(
    Theme(theme): Theme,
    session: WebSession,
    State(state): State<AppState>,
    Form(form): Form<session::CsrfForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session.csrf, &form.csrf) {
        return response;
    }

    let (Some(stripe), Ok(Some(billing))) = (state.stripe.as_ref(), state.store.user_billing(session.user_id).await)
    else {
        return render_account(&state, &session, theme, Some("Billing isn't available right now.")).await;
    };
    let Some(customer) = billing.customer else {
        return render_account(&state, &session, theme, Some("No subscription to manage yet.")).await;
    };

    match stripe.portal_url(&customer).await {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(error) => render_account(&state, &session, theme, Some(&format!("Payment error: {error}"))).await,
    }
}

// --- sign-up (open self-registration, auto-login) ---

#[derive(Deserialize)]
pub struct SignupQuery {
    #[serde(default)]
    invite: String,
}

pub async fn signup_form(
    Theme(theme): Theme,
    MaybeSession(session): MaybeSession,
    State(state): State<AppState>,
    Query(query): Query<SignupQuery>,
) -> Response {
    if session.is_some() {
        return Redirect::to("/account").into_response();
    }
    if !state.allow_registration && query.invite.is_empty() {
        return view::shell(
            theme,
            "Sign up",
            html! { div.card { h1 { "Sign-up is closed" } p { "Ask an operator for an invite." } p.aside { a href="/login" { "Sign in" } } } },
        )
        .into_response();
    }

    view::shell(
        theme,
        "Sign up",
        html! {
            form.card method="post" action="/signup" {
                h1 { "Create your Booklet account" }
                input type="hidden" name="invite" value=(query.invite);
                label { "Handle" input type="text" name="handle" autocomplete="username" required autofocus; }
                label { "Email" input type="email" name="email" autocomplete="email"; }
                label { "Password" input type="password" name="password" autocomplete="new-password" required; }
                button type="submit" { "Create account" }
                p.aside { a href="/login" { "Already have an account?" } }
            }
        },
    )
    .into_response()
}

#[derive(Deserialize)]
pub struct SignupForm {
    handle: String,
    #[serde(default)]
    email: String,
    password: String,
    #[serde(default)]
    invite: String,
}

pub async fn signup(Theme(theme): Theme, State(state): State<AppState>, Form(form): Form<SignupForm>) -> Response {
    // Gate: a live invite (whose email we adopt), or open sign-up.
    let invited_email = if !form.invite.is_empty() {
        match state.store.consume_invite(&auth::token_hash(&form.invite)).await {
            Ok(Some(email)) => Some(email),
            _ => return signup_error(theme, "That invite is invalid or expired."),
        }
    } else if state.allow_registration {
        None
    } else {
        return signup_error(theme, "Sign-up is closed.");
    };

    let handle = form.handle.trim();
    if handle.is_empty() {
        return signup_error(theme, "Handle cannot be empty.");
    }

    let hash = match auth::hash_password(&form.password) {
        Ok(hash) => hash,
        Err(error) => return internal(error),
    };
    let id = match state.store.create_user(handle, &hash).await {
        Ok(id) => id,
        Err(store::Error::Db(sqlx::Error::Database(db))) if db.is_unique_violation() => {
            return signup_error(theme, "That handle is taken.");
        }
        Err(error) => return internal(error),
    };

    let email = invited_email.unwrap_or_else(|| form.email.trim().to_string());
    if !email.is_empty() {
        let _ = state.store.set_email(id, &email).await;
    }
    let _ = state.store.log_event("system", "signup", handle).await;

    // Log the new account straight in.
    let cookie = match open_session(&state, id).await {
        Ok(cookie) => cookie,
        Err(response) => return response,
    };
    ([(header::SET_COOKIE, cookie)], Redirect::to("/account")).into_response()
}

fn signup_error(theme: &str, message: &str) -> Response {
    view::shell(
        theme,
        "Sign up",
        html! { div.card { h1 { "Sign up" } p.error { (message) } p.aside { a href="/signup" { "Try again" } } } },
    )
    .into_response()
}

fn feature(title: &str, body: &str) -> Markup {
    html! {
        div.feature {
            h3 { (title) }
            p { (body) }
        }
    }
}

fn dollars(cents: i32) -> String {
    format!("{:.2}", cents as f64 / 100.0)
}
