//! The one axum app: the bearer-token sync API and the whole cookie-authenticated
//! web frontend (marketing site, account portal, operator admin), sharing a single
//! [`AppState`] on one listener.

use crate::admin::{self, AppState};
use crate::http;
use axum::Router;

pub fn app(state: AppState) -> Router {
    Router::new().merge(http::routes()).merge(admin::routes()).with_state(state)
}
