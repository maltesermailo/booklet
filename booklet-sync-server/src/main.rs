//! The `booklet-sync-server` binary: `serve`, and `user create` to bootstrap an
//! account from the shell (no self-registration — the shell is the root of
//! trust). Configuration is environment variables, so a deployment carries no
//! config file:
//!
//! - `DATABASE_URL` — Postgres connection string (required).
//! - `BOOKLET_BLOB_DIR` — where blobs live on disk (default `data/blobs`).
//! - `BOOKLET_BIND` — address to listen on (default `127.0.0.1:8080`; loopback,
//!   with TLS terminated by a reverse proxy). One listener serves the sync API and
//!   the whole web frontend (site, account portal, `/admin`).
//!
//! Optional (each feature stays off until its vars are set):
//! - `BOOKLET_PUBLIC_URL` — the site's externally reachable base URL, used in
//!   email links and Stripe return URLs (default `http://<bind>`).
//! - `BOOKLET_ALLOW_REGISTRATION` — `0`/`false` to CLOSE the otherwise-open public
//!   sign-up.
//! - Email (reset links, invites): `BOOKLET_SMTP_HOST` (required to enable) plus
//!   `BOOKLET_SMTP_PORT`, `BOOKLET_SMTP_USER`, `BOOKLET_SMTP_PASSWORD`,
//!   `BOOKLET_SMTP_TLS` (`starttls`/`implicit`/`none`), and `BOOKLET_MAIL_FROM`.
//! - Billing: `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`. Plans (their quota and
//!   Stripe price) are created and edited on the admin Plans page, not via env.

use booklet_sync_server::{admin, app, auth, store::Store};
use std::env;
use std::io::{BufRead, IsTerminal};
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

/// A note revised this many times keeps one full checkpoint and the deltas
/// between; see `design/sync-server.md`.
const CHECKPOINT_INTERVAL: u32 = 50;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let command: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();

    match command.as_slice() {
        ["serve"] => serve().await,
        ["user", "create", handle] => create_user(handle).await,
        ["admin", "grant", handle] => grant_admin(handle).await,
        _ => {
            eprintln!(
                "usage:\n  booklet-sync-server serve\n  booklet-sync-server user create <handle>\n  booklet-sync-server admin grant <handle>"
            );
            ExitCode::from(2)
        }
    }
}

async fn serve() -> ExitCode {
    let store = match connect().await {
        Ok(store) => Arc::new(store),
        Err(code) => return code,
    };
    let started = Instant::now();

    let bind = env::var("BOOKLET_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("could not bind {bind}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let public_url = env::var("BOOKLET_PUBLIC_URL").unwrap_or_else(|_| format!("http://{bind}"));

    println!("booklet-sync-server listening on {bind}");

    // One app on one listener: the sync API and the whole web frontend (site,
    // account portal, operator admin at /admin). The connect-info service lets the
    // sign-in limiter see the client address.
    let state = admin::AppState::from_env(store, started, &public_url);
    let service = app::app(state).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(error) = axum::serve(listener, service).await {
        eprintln!("server stopped: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Grants admin to an existing user — how the first operator is bootstrapped,
/// from the shell (the root of trust), since nobody can sign in to make one.
async fn grant_admin(handle: &str) -> ExitCode {
    let store = match connect().await {
        Ok(store) => store,
        Err(code) => return code,
    };

    match store.grant_admin(handle).await {
        Ok(true) => {
            println!("granted admin to '{handle}'");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!("no such user '{handle}'");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("could not grant admin: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn create_user(handle: &str) -> ExitCode {
    let store = match connect().await {
        Ok(store) => store,
        Err(code) => return code,
    };

    // An interactive admin gets a hidden prompt; a provisioning script pipes the
    // password on stdin instead.
    let password = match read_password() {
        Ok(password) => password,
        Err(error) => {
            eprintln!("could not read password: {error}");
            return ExitCode::FAILURE;
        }
    };

    let hash = match auth::hash_password(&password) {
        Ok(hash) => hash,
        Err(error) => {
            eprintln!("could not hash password: {error}");
            return ExitCode::FAILURE;
        }
    };

    match store.create_user(handle, &hash).await {
        Ok(id) => {
            println!("created user '{handle}' (id {id})");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("could not create user: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads a password: a hidden prompt on an interactive terminal, one line from
/// stdin when piped.
fn read_password() -> std::io::Result<String> {
    if std::io::stdin().is_terminal() {
        return rpassword::prompt_password("Password: ");
    }

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// Connects and migrates, or prints why it could not and yields a failure code.
async fn connect() -> Result<Store, ExitCode> {
    let database_url = match env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL is required");
            return Err(ExitCode::from(2));
        }
    };
    let blob_dir = env::var("BOOKLET_BLOB_DIR").unwrap_or_else(|_| "data/blobs".into());

    Store::connect(&database_url, blob_dir, CHECKPOINT_INTERVAL).await.map_err(|error| {
        eprintln!("could not connect: {error}");
        ExitCode::FAILURE
    })
}
