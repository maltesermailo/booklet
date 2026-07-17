//! The read-only pages: Overview, Users, User detail, Devices, Vaults, Log.
//! Each renders through [`view::layout`]; the render helpers that a POST needs to
//! re-show with an error live here too, so the page has one definition.

use super::view::{self, bytes, chart, when, Theme};
use super::{internal, AdminState};
use crate::store::{AdminSession, Billing, DeleteImpact, UserRow};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use maud::{html, Markup};
use std::time::Duration;

pub async fn overview(State(state): State<AdminState>, session: AdminSession, Theme(theme): Theme) -> Response {
    let overview = match state.store.overview().await {
        Ok(overview) => overview,
        Err(error) => return internal(error),
    };
    let conflicts = match state.store.recent_conflict_copies(10).await {
        Ok(conflicts) => conflicts,
        Err(error) => return internal(error),
    };
    let growth = state.store.storage_growth().await.unwrap_or_default();
    let signins = state.store.signins_by_day().await.unwrap_or_default();

    let body = html! {
        div.head { h1 { "Overview" } p.sub { "Booklet sync server " (env!("CARGO_PKG_VERSION")) " · up " (uptime(state.started.elapsed())) } }

        section.cards {
            (card("Users", &overview.users.to_string(), Some(&format!("{} admin", overview.admins))))
            (card("Devices", &overview.live_devices.to_string(), Some(&format!("{} live of {}", overview.live_devices, overview.devices))))
            (card("Vaults", &overview.vaults.to_string(), None))
        }

        section {
            h2 { "Blob store" }
            table.grid {
                tbody {
                    tr { th { "Blobs" } td { (overview.blobs) } }
                    tr { th { "Content" } td { (bytes(overview.blob_size as u64)) } }
                    tr { th { "On disk" } td { (bytes(overview.blob_stored as u64)) } }
                    tr { th { "Disk free" } td { (bytes(overview.disk_free)) } }
                }
            }
        }

        section {
            h2 { "Storage growth" }
            div.chartbox { (chart(&growth)) }
        }
        section {
            h2 { "Sign-ins (30 days)" }
            div.chartbox { (chart(&signins)) }
        }

        section {
            h2 { "Recent conflict copies" }
            @if conflicts.is_empty() {
                p.empty { "None. Sync has not had to fall back to a conflict copy." }
            } @else {
                table {
                    thead { tr { th { "Vault" } th { "File" } th { "When" } } }
                    tbody {
                        @for copy in &conflicts {
                            tr { td { (copy.vault) } td { (copy.path) } td { (when(&copy.at)) } }
                        }
                    }
                }
            }
        }
    };

    view::layout(theme, "Overview", "/admin", &session, body).into_response()
}

pub async fn users(State(state): State<AdminState>, session: AdminSession, Theme(theme): Theme) -> Response {
    let users = match state.store.list_users().await {
        Ok(users) => users,
        Err(error) => return internal(error),
    };

    let body = users_body(&session, &users, state.mailer.is_some(), None);
    view::layout(theme, "Users", "/admin/users", &session, body).into_response()
}

/// The Users page body, reused by the create-user / invite POSTs to re-show with
/// an error. `mail` gates the email-invite form.
pub(super) fn users_body(session: &AdminSession, users: &[UserRow], mail: bool, error: Option<&str>) -> Markup {
    html! {
        div.head { h1 { "Users" } }

        section {
            h2 { "Create user" }
            @if let Some(message) = error {
                p.error { (message) }
            }
            form.inline method="post" action="/admin/users" {
                input type="hidden" name="csrf" value=(session.csrf);
                input type="text" name="handle" placeholder="handle" required;
                input type="email" name="email" placeholder="email (optional)";
                input type="password" name="password" placeholder="password" required;
                button type="submit" { "Create" }
            }
        }

        @if mail {
            section {
                h2 { "Invite by email" }
                form.inline method="post" action="/admin/invites" {
                    input type="hidden" name="csrf" value=(session.csrf);
                    input type="email" name="email" placeholder="email" required;
                    button type="submit" { "Send invite" }
                }
            }
        }

        section {
            table {
                thead { tr { th { "Handle" } th { "Plan" } th { "Created" } th { "Last seen" } th.num { "Vaults" } th.num { "Bytes" } th {} } }
                tbody {
                    @for user in users {
                        tr {
                            td { a href=(format!("/admin/users/{}", user.id)) { (user.handle) }
                                 @if user.is_admin { span.tag { "admin" } }
                                 @if user.disabled { span.tag.warn { "disabled" } } }
                            td.mono { (user.plan) }
                            td { (user.created) }
                            td { (when(&user.last_seen)) }
                            td.num { (user.vaults) }
                            td.num { (bytes(user.bytes as u64)) }
                            td { a href=(format!("/admin/users/{}", user.id)) { "View" } }
                        }
                    }
                }
            }
        }
    }
}

pub async fn user_detail(
    State(state): State<AdminState>,
    session: AdminSession,
    Theme(theme): Theme,
    Path(id): Path<i64>,
) -> Response {
    let user = match state.store.user_row(id).await {
        Ok(Some(user)) => user,
        Ok(None) => return not_found(),
        Err(error) => return internal(error),
    };
    let billing = match state.store.user_billing(id).await {
        Ok(Some(billing)) => billing,
        Ok(None) => return not_found(),
        Err(error) => return internal(error),
    };
    let usage = state.store.user_usage_bytes(id).await.unwrap_or(0);
    let limit = state.store.user_effective_quota(id).await.unwrap_or(None);
    let plans = state.store.list_plans().await.unwrap_or_default();
    let devices = state.store.list_devices(Some(id)).await.unwrap_or_default();
    let vaults = state.store.list_vaults_admin(Some(id)).await.unwrap_or_default();

    let body = html! {
        div.head {
            h1 { (user.handle)
                 @if user.is_admin { span.tag { "admin" } }
                 @if user.disabled { span.tag.warn { "disabled" } } }
            p.sub { "Joined " (user.created) " · last seen " (when(&user.last_seen)) }
        }

        section.actions {
            @if !user.disabled {
                form method="post" action=(format!("/admin/users/{}/disable", user.id)) {
                    input type="hidden" name="csrf" value=(session.csrf);
                    button type="submit" { "Disable" }
                }
            }
            a.danger href=(format!("/admin/users/{}/delete", user.id)) { "Delete…" }
        }

        (storage_and_plan(&session, &user, &billing, usage, limit, &plans, state.stripe.is_some()))
        (account_forms(&session, &user, &billing))

        section {
            h2 { "Vaults" }
            (vaults_table(&vaults, false))
        }

        section {
            h2 { "Devices" }
            (devices_table(&session, &devices, false))
        }
    };

    view::layout(theme, "User", "/admin/users", &session, body).into_response()
}

/// The storage meter, plan/quota form, and (when billing is on) subscription
/// actions. `plans` populates the plan picker; `limit` is the effective quota
/// (`None` = unlimited).
fn storage_and_plan(
    session: &AdminSession,
    user: &UserRow,
    billing: &Billing,
    usage: i64,
    limit: Option<i64>,
    plans: &[crate::store::PlanRow],
    stripe: bool,
) -> Markup {
    let percent = match limit {
        Some(limit) if limit > 0 => (usage as f64 / limit as f64 * 100.0).min(100.0),
        _ => 0.0,
    };

    html! {
        section {
            h2 { "Storage & plan" }
            div.meter { div.fill style=(format!("width:{percent:.0}%")) {} }
            p.sub { (bytes(usage as u64)) " of "
                    @match limit { Some(limit) => (bytes(limit as u64)), None => "unlimited" }
                    " · plan " b { (billing.plan) }
                    @if let Some(status) = &billing.status { " · " (status) } }

            form.inline method="post" action=(format!("/admin/users/{}/quota", user.id)) {
                input type="hidden" name="csrf" value=(session.csrf);
                label.inline-label { "Plan"
                    select name="plan" {
                        @for candidate in plans {
                            option value=(candidate.name) selected[candidate.name == billing.plan] { (candidate.name) }
                        }
                    }
                }
                label.inline-label { "Quota override (GiB, blank = plan default)"
                    input type="number" name="quota_gib" min="0" step="0.1"
                          value=(billing.quota_bytes.map(gib_string).unwrap_or_default());
                }
                button type="submit" { "Save" }
            }

            @if stripe {
                div.actions {
                    form method="post" action=(format!("/admin/users/{}/billing/checkout", user.id)) {
                        input type="hidden" name="csrf" value=(session.csrf);
                        input type="hidden" name="plan" value="pro";
                        button type="submit" { "Create Pro checkout link" }
                    }
                    @if billing.customer.is_some() {
                        form method="post" action=(format!("/admin/users/{}/billing/portal", user.id)) {
                            input type="hidden" name="csrf" value=(session.csrf);
                            button type="submit" { "Customer portal link" }
                        }
                    }
                }
            }
        }
    }
}

/// Email and password forms an operator can set directly (no email needed).
fn account_forms(session: &AdminSession, user: &UserRow, billing: &Billing) -> Markup {
    html! {
        section {
            h2 { "Account" }
            form.inline method="post" action=(format!("/admin/users/{}/email", user.id)) {
                input type="hidden" name="csrf" value=(session.csrf);
                label.inline-label { "Email"
                    input type="email" name="email" value=(billing.email.clone().unwrap_or_default());
                }
                button type="submit" { "Save" }
            }
            form.inline method="post" action=(format!("/admin/users/{}/password", user.id)) {
                input type="hidden" name="csrf" value=(session.csrf);
                label.inline-label { "Set password"
                    input type="password" name="password" placeholder="new password" required;
                }
                button type="submit" { "Reset" }
            }
        }
    }
}

pub async fn confirm_delete_user(
    State(state): State<AdminState>,
    session: AdminSession,
    Theme(theme): Theme,
    Path(id): Path<i64>,
) -> Response {
    let impact = match state.store.delete_user_impact(id).await {
        Ok(Some(impact)) => impact,
        Ok(None) => return not_found(),
        Err(error) => return internal(error),
    };

    let body = confirm_delete_body(&session, id, &impact, None);
    view::layout(theme, "Delete user", "/admin/users", &session, body).into_response()
}

/// The typed-confirmation page, reused by the POST when the typed handle does not
/// match.
pub(super) fn confirm_delete_body(
    session: &AdminSession,
    id: i64,
    impact: &DeleteImpact,
    error: Option<&str>,
) -> Markup {
    html! {
        div.head { h1 { "Delete " (impact.handle) } }

        section {
            p.warn-box {
                "This permanently removes the account and all of its sync state: "
                b { (impact.vaults) } " vault(s), " b { (impact.notes) } " note(s), "
                b { (bytes(impact.bytes as u64)) } " of content. It cannot be undone."
            }
            @if let Some(message) = error {
                p.error { (message) }
            }
            form method="post" action=(format!("/admin/users/{}/delete", id)) {
                input type="hidden" name="csrf" value=(session.csrf);
                label { "Type the handle " b { (impact.handle) } " to confirm"
                        input type="text" name="confirm" autocomplete="off"; }
                div.actions {
                    button.danger type="submit" { "Delete permanently" }
                    a href=(format!("/admin/users/{}", id)) { "Cancel" }
                }
            }
        }
    }
}

pub async fn devices(State(state): State<AdminState>, session: AdminSession, Theme(theme): Theme) -> Response {
    let devices = match state.store.list_devices(None).await {
        Ok(devices) => devices,
        Err(error) => return internal(error),
    };

    let body = html! {
        div.head { h1 { "Devices" } }
        section { (devices_table(&session, &devices, true)) }
    };

    view::layout(theme, "Devices", "/admin/devices", &session, body).into_response()
}

pub async fn vaults(State(state): State<AdminState>, session: AdminSession, Theme(theme): Theme) -> Response {
    let vaults = match state.store.list_vaults_admin(None).await {
        Ok(vaults) => vaults,
        Err(error) => return internal(error),
    };

    let body = html! {
        div.head { h1 { "Vaults" } }
        section { (vaults_table(&vaults, true)) }
    };

    view::layout(theme, "Vaults", "/admin/vaults", &session, body).into_response()
}

pub async fn log(State(state): State<AdminState>, session: AdminSession, Theme(theme): Theme) -> Response {
    let rows = match state.store.recent_audit(200).await {
        Ok(rows) => rows,
        Err(error) => return internal(error),
    };

    let body = html! {
        div.head { h1 { "Log" } }
        section {
            @if rows.is_empty() {
                p.empty { "Nothing logged yet." }
            } @else {
                table {
                    thead { tr { th { "When" } th { "Actor" } th { "Action" } th { "Detail" } } }
                    tbody {
                        @for row in &rows {
                            tr { td.mono { (row.at) } td { (row.actor) } td { (row.action) } td { (row.detail) } }
                        }
                    }
                }
            }
        }
    };

    view::layout(theme, "Log", "/admin/log", &session, body).into_response()
}

// --- shared table fragments ---

/// `owner` adds an owner column (the all-users lists); a per-user page omits it.
fn vaults_table(vaults: &[crate::store::VaultRow], owner: bool) -> Markup {
    html! {
        @if vaults.is_empty() {
            p.empty { "No vaults." }
        } @else {
            table {
                thead { tr {
                    th { "Vault" } @if owner { th { "Owner" } }
                    th.num { "Notes" } th.num { "Bytes" } th { "Last sync" }
                } }
                tbody {
                    @for vault in vaults {
                        tr {
                            td { (vault.name) }
                            @if owner { td { (vault.handle) } }
                            td.num { (vault.notes) }
                            td.num { (bytes(vault.bytes as u64)) }
                            td { (when(&vault.last_sync)) }
                        }
                    }
                }
            }
        }
    }
}

fn devices_table(session: &AdminSession, devices: &[crate::store::DeviceRow], owner: bool) -> Markup {
    html! {
        @if devices.is_empty() {
            p.empty { "No devices." }
        } @else {
            table {
                thead { tr {
                    @if owner { th { "Owner" } }
                    th { "Device" } th { "Platform" } th { "Issued" } th { "Last seen" } th { "Status" } th {}
                } }
                tbody {
                    @for device in devices {
                        tr {
                            @if owner { td { (device.handle) } }
                            td { (device.name) }
                            td { (device.platform) }
                            td { (device.issued) }
                            td { (when(&device.last_seen)) }
                            td { @if device.revoked { span.tag.warn { "revoked" } } @else { span.tag.ok { "live" } } }
                            td {
                                @if !device.revoked {
                                    form method="post" action=(format!("/admin/devices/{}/revoke", device.id)) {
                                        input type="hidden" name="csrf" value=(session.csrf);
                                        button.link type="submit" { "Revoke" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn card(label: &str, value: &str, note: Option<&str>) -> Markup {
    html! {
        div.card {
            span.value { (value) }
            span.label { (label) }
            @if let Some(note) = note { span.note { (note) } }
        }
    }
}

fn uptime(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3_600 % 24, seconds / 60 % 60);

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// A byte count rendered as a GiB number for the quota-override field.
fn gib_string(bytes: i64) -> String {
    format!("{:.2}", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}
