//! `booklet-sync-server` — the self-hosted sync server.
//!
//! Built in slices (see `design/sync-server.md`). Landed so far:
//! - [`blob`] — the content-addressed, delta-chained blob store.
//! - [`store`] — the Postgres-backed storage layer over it.
//! - [`auth`] — password hashing and device-token helpers.
//! - [`http`] — the axum router, handlers, and the bearer-token auth boundary.
//! - [`admin`] — the localhost-only, cookie-authenticated web admin panel.

pub mod admin;
pub mod auth;
pub mod billing;
pub mod blob;
pub mod http;
pub mod mail;
pub mod store;
