# booklet-sync-server

The self-hosted sync server (see `design/sync-server.md`). Built in slices:

- `src/blob.rs` — content-addressed, delta-chained blob store (no DB).
- `src/store.rs` — the Postgres-backed storage layer over it.
- `src/auth.rs` — argon2id passwords and device-token helpers.
- `src/http.rs` — the axum router, handlers, and bearer-token auth boundary.
- `src/main.rs` — the `serve` / `user create` CLI.

## Running the server

Configuration is environment variables:

- `DATABASE_URL` — Postgres connection string (required).
- `BOOKLET_BLOB_DIR` — blob storage directory (default `data/blobs`).
- `BOOKLET_BIND` — listen address (default `127.0.0.1:8080`; loopback, with TLS
  terminated by a reverse proxy).

```sh
export DATABASE_URL="postgres://booklet:booklet@localhost:5433/booklet_sync"
# Bootstrap an account (prompts for a password on a terminal; reads stdin when piped):
cargo run -p booklet-sync-server -- user create alice
# Serve:
cargo run -p booklet-sync-server -- serve
```

## Development database

The storage tests use `#[sqlx::test]`, which creates a throwaway database per
test off a running PostgreSQL. Start one with Docker:

```sh
docker run -d --name booklet-pg \
  -e POSTGRES_USER=booklet -e POSTGRES_PASSWORD=booklet -e POSTGRES_DB=booklet_sync \
  -p 5433:5432 postgres:16-alpine
```

Then point the tests at it (the URL is only needed at test time — the build uses
runtime-checked queries and never touches a database):

```sh
export DATABASE_URL="postgres://booklet:booklet@localhost:5433/booklet_sync"
cargo test -p booklet-sync-server
```

`cargo build` and the other crates' tests need neither Docker nor `DATABASE_URL`.
Migrations live in `migrations/` and run automatically (via `#[sqlx::test]` in
tests, and `Store::connect` in production).
