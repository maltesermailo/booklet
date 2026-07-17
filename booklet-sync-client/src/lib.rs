//! The Booklet sync client engine — Qt-free, so it is tested without the UI.
//!
//! [`Client`] is a blocking HTTP client for the server's routes; [`engine`]
//! reconciles a local vault against a server vault, pushing local changes
//! (merging on a 409) and pulling remote ones. It builds on `booklet-core`'s
//! change tracking (2a) and merge rules (2b); the app (2d/2e) drives it from a
//! dedicated thread and wires the results into the UI.

pub mod client;
pub mod engine;
pub mod secret;

pub use client::{Client, ClientError, PutResult};
pub use engine::{pull, push, ClientState, PushOutcome};
pub use secret::Credentials;
