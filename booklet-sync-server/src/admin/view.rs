//! Shared markup: the page shell, the sign-in / public auth pages, byte
//! formatting, inline-SVG charts, the theme toggle, and the CSS/font asset
//! routes. Everything is served from the binary — no external requests, so an
//! air-gapped box still renders the panel in Booklet's face.

use crate::store::{AdminSession, Point, WebSession};
use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use maud::{html, Markup, DOCTYPE};
use serde::Deserialize;
use std::convert::Infallible;

/// The panel's tabs, in order.
const NAV: &[(&str, &str)] = &[
    ("/admin", "Overview"),
    ("/admin/users", "Users"),
    ("/admin/devices", "Devices"),
    ("/admin/vaults", "Vaults"),
    ("/admin/plans", "Plans"),
    ("/admin/log", "Log"),
];

pub const THEME_COOKIE: &str = "booklet_admin_theme";

/// The signed-in page shell: wordmark, nav (with `current` highlighted), a theme
/// toggle, and the operator with a sign-out form.
pub fn layout(theme: &str, title: &str, current: &str, session: &AdminSession, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html data-theme=(theme) {
            (head_element(title))
            body {
                header.topbar {
                    span.wordmark { "Booklet" }
                    nav {
                        @for (href, label) in NAV {
                            a.active[current == *href] href=(href) { (label) }
                        }
                    }
                    (theme_toggle(theme, current))
                    form.signout method="post" action="/logout" {
                        input type="hidden" name="csrf" value=(session.csrf);
                        span.who { (session.handle) }
                        button type="submit" { "Sign out" }
                    }
                }
                main { (body) }
            }
        }
    }
}

/// The public site / account-portal shell: wordmark home link, Pricing, and
/// either Log in / Sign up (a stranger) or Account / Sign out (signed in, plus an
/// Admin link when the user is an operator).
pub fn site_layout(theme: &str, title: &str, session: Option<&WebSession>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html data-theme=(theme) {
            (head_element(title))
            body.site {
                header.topbar {
                    a.wordmark href="/" { "Booklet" }
                    nav {
                        a href="/pricing" { "Pricing" }
                        @if let Some(session) = session {
                            @if session.is_admin { a href="/admin" { "Admin" } }
                            a href="/account" { "Account" }
                        } @else {
                            a href="/login" { "Log in" }
                        }
                    }
                    (theme_toggle(theme, "/"))
                    @if let Some(session) = session {
                        form.signout method="post" action="/logout" {
                            input type="hidden" name="csrf" value=(session.csrf);
                            button type="submit" { "Sign out" }
                        }
                    } @else {
                        a.button href="/signup" { "Sign up" }
                    }
                }
                main { (body) }
            }
        }
    }
}

/// A bare, centered-card shell for the pages a stranger can reach — sign-in,
/// forgot/reset, register. No nav, no session.
pub fn shell(theme: &str, title: &str, card: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html data-theme=(theme) {
            (head_element(title))
            body.login { (card) }
        }
    }
}

/// The sign-in page. Every failure reuses it with one generic message.
pub fn login_page(theme: &str, error: Option<&str>) -> Markup {
    shell(
        theme,
        "Sign in",
        html! {
            form.card method="post" action="/login" {
                h1 { "Sign in to Booklet" }
                @if let Some(message) = error {
                    p.error { (message) }
                }
                label { "Handle" input type="text" name="handle" autocomplete="username" autofocus; }
                label { "Password" input type="password" name="password" autocomplete="current-password"; }
                button type="submit" { "Sign in" }
                p.aside { a href="/forgot" { "Forgot password?" } " · " a href="/signup" { "Sign up" } }
            }
        },
    )
}

fn theme_toggle(theme: &str, current: &str) -> Markup {
    let (next, glyph) = if theme == "light" { ("night", "☾") } else { ("light", "☀") };
    html! {
        a.themetoggle href=(format!("/theme?set={next}&from={current}")) title="Toggle theme" { (glyph) }
    }
}

fn head_element(title: &str) -> Markup {
    html! {
        head {
            meta charset="utf-8";
            meta name="viewport" content="width=device-width, initial-scale=1";
            title { "Booklet — " (title) }
            link rel="stylesheet" href="/assets/panel.css";
        }
    }
}

/// A byte count as a human-readable size.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Renders an optional timestamp string, dimming a missing one to an em dash.
pub fn when(value: &Option<String>) -> Markup {
    match value {
        Some(text) => html! { (text) },
        None => html! { span.dim { "—" } },
    }
}

/// A self-contained inline-SVG bar chart, coloured by `currentColor` so it obeys
/// the theme. No JavaScript, no external library.
pub fn chart(points: &[Point]) -> Markup {
    if points.is_empty() {
        return html! { p.empty { "No data yet." } };
    }

    let max = points.iter().map(|point| point.value).max().unwrap_or(1).max(1) as f64;
    let (width, height) = (640.0, 120.0);
    let bar = width / points.len() as f64;

    html! {
        svg.chart viewBox=(format!("0 0 {width} {height}")) preserveAspectRatio="none" role="img" {
            @for (index, point) in points.iter().enumerate() {
                @let bar_height = (point.value as f64 / max) * (height - 4.0);
                rect
                    x=(format!("{:.1}", index as f64 * bar + 1.0))
                    y=(format!("{:.1}", height - bar_height))
                    width=(format!("{:.1}", (bar - 2.0).max(0.5)))
                    height=(format!("{:.1}", bar_height)) {
                    title { (point.label) ": " (point.value) }
                }
            }
        }
        p.chartcaption { span { (points.first().map(|p| p.label.as_str()).unwrap_or("")) }
                         span.dim { "peak " (max as i64) }
                         span { (points.last().map(|p| p.label.as_str()).unwrap_or("")) } }
    }
}

// --- theme toggle ---

/// The theme the request carries, from the cookie; night by default.
pub struct Theme(pub &'static str);

impl<S: Send + Sync> FromRequestParts<S> for Theme {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Infallible> {
        let light = super::cookie(&parts.headers, THEME_COOKIE).as_deref() == Some("light");
        Ok(Theme(if light { "light" } else { "night" }))
    }
}

#[derive(Deserialize)]
pub struct ThemeQuery {
    set: String,
    #[serde(default)]
    from: String,
}

/// Sets the theme cookie and returns to the page the toggle was on.
pub async fn set_theme(Query(query): Query<ThemeQuery>) -> Response {
    let theme = if query.set == "light" { "light" } else { "night" };
    // Only same-site relative paths, so the toggle can't be an open redirect.
    let back = if query.from.starts_with('/') && !query.from.starts_with("//") { query.from.as_str() } else { "/" };
    let cookie = format!("{THEME_COOKIE}={theme}; Path=/; Max-Age=31536000; SameSite=Lax");

    ([(header::SET_COOKIE, cookie)], Redirect::to(back)).into_response()
}

// --- asset routes (no auth: the login page needs them) ---

pub async fn stylesheet() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("panel.css")).into_response()
}

/// Serves one of the bundled OFL fonts, embedded from the app crate's font
/// directory (a deliberate cross-crate path — no duplication).
pub async fn font(Path(file): Path<String>) -> Response {
    let bytes: &[u8] = match file.as_str() {
        "EBGaramond.ttf" => include_bytes!("../../../src/booklet/fonts/EBGaramond.ttf"),
        "AlegreyaSans-Regular.ttf" => include_bytes!("../../../src/booklet/fonts/AlegreyaSans-Regular.ttf"),
        "JetBrainsMono.ttf" => include_bytes!("../../../src/booklet/fonts/JetBrainsMono.ttf"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    (
        [
            (header::CONTENT_TYPE, "font/ttf"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response()
}
