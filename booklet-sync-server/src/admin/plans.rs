//! The Plans page and its create / edit / delete actions. A plan is an
//! operator-defined tier: a storage quota and an optional Stripe price. Users are
//! assigned a plan on their own page; a plan with users on it cannot be deleted.

use super::actions::parse_gib;
use super::view::{self, Theme};
use super::{csrf_rejection, internal, AdminState};
use crate::store::{self, AdminSession, PlanRow};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use maud::{html, Markup};
use serde::Deserialize;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

pub async fn list(State(state): State<AdminState>, session: AdminSession, Theme(theme): Theme) -> Response {
    let plans = match state.store.list_plans().await {
        Ok(plans) => plans,
        Err(error) => return internal(error),
    };

    view::layout(theme, "Plans", "/admin/plans", &session, body(&session, &plans, None)).into_response()
}

fn body(session: &AdminSession, plans: &[PlanRow], error: Option<&str>) -> Markup {
    html! {
        div.head { h1 { "Plans" } p.sub { "A plan is a storage quota and an optional Stripe price. Assign one on a user's page." } }

        section {
            h2 { "New plan" }
            @if let Some(message) = error { p.error { (message) } }
            form.inline method="post" action="/admin/plans" {
                input type="hidden" name="csrf" value=(session.csrf);
                input type="text" name="name" placeholder="name (e.g. team)" required;
                input type="number" name="quota_gib" min="0" step="0.1" placeholder="quota GiB" required;
                input type="text" name="stripe_price_id" placeholder="stripe price id (optional)";
                button type="submit" { "Create" }
            }
        }

        section {
            table {
                thead { tr { th { "Plan" } th { "Quota & price" } th.num { "Users" } th {} } }
                tbody {
                    @for plan in plans {
                        tr {
                            td.mono { (plan.name) }
                            td {
                                form.inline method="post" action=(format!("/admin/plans/{}/update", plan.name)) {
                                    input type="hidden" name="csrf" value=(session.csrf);
                                    input type="number" name="quota_gib" min="0" step="0.1" value=(gib(plan.quota_bytes));
                                    input type="text" name="stripe_price_id"
                                          value=(plan.stripe_price_id.clone().unwrap_or_default())
                                          placeholder="stripe price id";
                                    button type="submit" { "Save" }
                                }
                            }
                            td.num { (plan.users) }
                            td {
                                @if plan.users == 0 {
                                    form method="post" action=(format!("/admin/plans/{}/delete", plan.name)) {
                                        input type="hidden" name="csrf" value=(session.csrf);
                                        button.link type="submit" { "Delete" }
                                    }
                                } @else {
                                    span.dim { "in use" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Deserialize)]
pub struct CreatePlanForm {
    csrf: String,
    name: String,
    quota_gib: String,
    #[serde(default)]
    stripe_price_id: String,
}

pub async fn create(State(state): State<AdminState>, session: AdminSession, Form(form): Form<CreatePlanForm>) -> Response {
    if let Some(response) = csrf_rejection(&session, &form.csrf) {
        return response;
    }

    let name = form.name.trim();
    if name.is_empty() {
        return reshow(&state, &session, Some("Name cannot be empty.")).await;
    }
    let Some(quota) = parse_gib(&form.quota_gib) else {
        return reshow(&state, &session, Some("Quota must be a number of GiB.")).await;
    };

    match state.store.create_plan(name, quota, nonempty(&form.stripe_price_id)).await {
        Ok(_) => {
            let _ = state.store.log_event(&session.handle, "create-plan", name).await;
            Redirect::to("/admin/plans").into_response()
        }
        Err(store::Error::Db(sqlx::Error::Database(db))) if db.is_unique_violation() => {
            reshow(&state, &session, Some("A plan with that name already exists.")).await
        }
        Err(error) => internal(error),
    }
}

#[derive(Deserialize)]
pub struct UpdatePlanForm {
    csrf: String,
    quota_gib: String,
    #[serde(default)]
    stripe_price_id: String,
}

pub async fn update(
    State(state): State<AdminState>,
    session: AdminSession,
    Path(name): Path<String>,
    Form(form): Form<UpdatePlanForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session, &form.csrf) {
        return response;
    }

    let Some(quota) = parse_gib(&form.quota_gib) else {
        return Redirect::to("/admin/plans").into_response();
    };
    if let Err(error) = state.store.update_plan(&name, quota, nonempty(&form.stripe_price_id)).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "update-plan", &name).await;

    Redirect::to("/admin/plans").into_response()
}

#[derive(Deserialize)]
pub struct DeletePlanForm {
    csrf: String,
}

pub async fn delete(
    State(state): State<AdminState>,
    session: AdminSession,
    Path(name): Path<String>,
    Form(form): Form<DeletePlanForm>,
) -> Response {
    if let Some(response) = csrf_rejection(&session, &form.csrf) {
        return response;
    }

    // A plan in use is not deleted from under its users — reassign them first.
    match state.store.count_users_on_plan(&name).await {
        Ok(0) => {}
        Ok(_) => return reshow(&state, &session, Some("That plan still has users — reassign them first.")).await,
        Err(error) => return internal(error),
    }
    if let Err(error) = state.store.delete_plan(&name).await {
        return internal(error);
    }
    let _ = state.store.log_event(&session.handle, "delete-plan", &name).await;

    Redirect::to("/admin/plans").into_response()
}

async fn reshow(state: &AdminState, session: &AdminSession, error: Option<&str>) -> Response {
    match state.store.list_plans().await {
        Ok(plans) => {
            (StatusCode::BAD_REQUEST, view::layout("night", "Plans", "/admin/plans", session, body(session, &plans, error)))
                .into_response()
        }
        Err(error) => internal(error),
    }
}

fn gib(bytes: i64) -> String {
    format!("{:.2}", bytes as f64 / GIB)
}

fn nonempty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}
