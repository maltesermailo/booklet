//! The `booklet-sync-server` binary: `serve`, and `user create` to bootstrap an
//! account from the shell (no self-registration — the shell is the root of
//! trust). Configuration is environment variables, so a deployment carries no
//! config file:
//!
//! - `DATABASE_URL` — Postgres connection string (required).
//! - `BOOKLET_BLOB_DIR` — where blobs live on disk (default `data/blobs`).
//! - `BOOKLET_BIND` — address to listen on (default `127.0.0.1:8080`; loopback,
//!   with TLS terminated by a reverse proxy).

use booklet_sync_server::{auth, http, store::Store};
use std::env;
use std::io::{BufRead, IsTerminal};
use std::process::ExitCode;
use std::sync::Arc;

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
        _ => {
            eprintln!("usage:\n  booklet-sync-server serve\n  booklet-sync-server user create <handle>");
            ExitCode::from(2)
        }
    }
}

async fn serve() -> ExitCode {
    let store = match connect().await {
        Ok(store) => store,
        Err(code) => return code,
    };

    let bind = env::var("BOOKLET_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("could not bind {bind}: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!("booklet-sync-server listening on {bind}");
    if let Err(error) = axum::serve(listener, http::router(Arc::new(store))).await {
        eprintln!("server stopped: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
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
