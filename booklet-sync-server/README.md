# booklet-sync-server

The self-hosted sync server (see `design/sync-server.md`). Built in slices:

- `src/blob.rs` — content-addressed, delta-chained blob store (no DB).
- `src/store.rs` — the Postgres-backed storage layer over it.
- `src/auth.rs` — argon2id passwords and device-token helpers.
- `src/http.rs` — the sync API's routes and bearer-token auth boundary.
- `src/admin/` — the whole web frontend (maud): the public marketing site and
  account portal (`src/admin/site.rs`), the operator admin, cookie web sessions,
  CSRF-guarded forms, fonts served from the binary.
- `src/app.rs` — merges the sync API and the web frontend into one app.
- `src/main.rs` — the `serve` / `user create` / `admin grant` CLI.

## Running the server

One app on one port serves the marketing site (`/`), the account portal
(`/account`), the sync API (`/auth/token`, `/vaults/…`, `/blobs/…`), the operator
admin (`/admin`), and the Stripe webhook (`/billing/webhook`). Put one HTTPS
subdomain in front of it — see `DEPLOY.md`.

Configuration is environment variables:

- `DATABASE_URL` — Postgres connection string (required).
- `BOOKLET_BLOB_DIR` — blob storage directory (default `data/blobs`).
- `BOOKLET_BIND` — listen address (default `127.0.0.1:8080`; loopback, with TLS
  terminated by a reverse proxy).

Optional — each feature stays off until its variables are set, and the site
degrades gracefully without them:

- `BOOKLET_PUBLIC_URL` — the site's externally reachable base URL, used in email
  links and Stripe return URLs.
- `BOOKLET_ALLOW_REGISTRATION` — `0`/`false` closes the otherwise-open public
  sign-up at `/signup` (new accounts are non-admin).
- Email (password-reset links at `/forgot` and invites):
  - `BOOKLET_SMTP_HOST` — SMTP server hostname (required to enable email).
  - `BOOKLET_SMTP_PORT` — port (default 587 for STARTTLS, 465 for implicit TLS).
  - `BOOKLET_SMTP_USER`, `BOOKLET_SMTP_PASSWORD` — login, if the server needs one.
  - `BOOKLET_SMTP_TLS` — `starttls` (default), `implicit`, or `none` (dev only).
  - `BOOKLET_MAIL_FROM` — the From address (`Name <addr>` accepted; required).
- `STRIPE_SECRET_KEY` + `STRIPE_WEBHOOK_SECRET` — enable billing. The webhook
  endpoint is `POST /billing/webhook`. Plans (each a storage quota and an optional
  Stripe price id) are created and edited on the **Plans** page in the panel — not
  via environment variables.

```sh
export DATABASE_URL="postgres://booklet:booklet@localhost:5433/booklet_sync"
# Bootstrap an account (prompts for a password on a terminal; reads stdin when piped):
cargo run -p booklet-sync-server -- user create alice
# Make the first operator (from the shell — nobody can sign in to make one):
cargo run -p booklet-sync-server -- admin grant alice
# Serve (sync + admin listeners):
cargo run -p booklet-sync-server -- serve
```

## Site, portal, and admin

The public site is at `/` (marketing, pricing, sign-up). A signed-in user manages
their account and buys plans at `/account`. The operator admin — users, devices,
bytes, plans, a log, and it **never reads note content** — is at `/admin`, open to
any signed-in user whose account has `is_admin` (granted from the shell with
`admin grant`).

A web-session cookie is a separate credential from a device token: the session
cookie cannot reach the bearer-token sync API, and a device token cannot present
as a session. `/admin` additionally requires `is_admin`; a signed-in non-admin
gets a 403 there.

## Quotas, billing, and accounts

- **Plans & quotas.** Plans are operator-defined on the **Plans** page — each is a
  name, a storage quota, and an optional Stripe price. Two are seeded (`free` 1 GiB,
  `pro` 50 GiB); add, retune, rename, or delete them freely (a plan with users on
  it can't be deleted). A user is assigned a plan (or a per-user quota override) on
  their detail page, and a sync write that would push them past their quota is
  refused with **507 Insufficient Storage**.
- **Billing (optional, Stripe).** With `STRIPE_*` set, a user's plan — and thus
  quota — is driven by a Stripe subscription. The panel generates hosted Checkout
  and Customer-Portal links (we never handle card data); a signature-verified
  webhook keeps the plan in sync. Live Stripe calls need a real test key and
  webhook forwarding (e.g. the Stripe CLI) to exercise end to end.
- **Accounts.** An operator can set a user's email or password directly (no email
  needed). With SMTP configured, the panel also offers email-based password
  resets and invites. Self-registration is opt-in via `BOOKLET_ALLOW_REGISTRATION`.
- **Observability.** The Overview page draws inline-SVG charts (storage growth,
  sign-ins), and the panel has a light theme alongside night (toggle in the top
  bar).

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
