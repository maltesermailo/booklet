# Design note — `booklet-sync-server` (before M2 · 2c)

## Why this note exists

2a (local change tracking) and 2b (merge/conflict rules) are done and live in
`booklet-core`, Qt-free and unit-tested. 2c is a step-change: a new server crate
with an account model, a blob store, and a version feed. The roadmap flags these
three as **expensive to change once written** — a schema or a route shape is not
something you refactor after two clients depend on it — so this note pins them
down before code. It consolidates the decisions the roadmap already settled and
makes the implementation-level calls it left open (DB engine, table shapes, route
signatures, wire types).

Scope: **2c is the server plus the shared wire crate.** The client engine (2d)
and the merge/history UI (2e) are described only where the contract touches them.
Admin auth, the admin panel, and history GC are **M7 / later** and are called out
where the schema must leave room for them.

## Crates and where code lives

| Crate | Kind | Depends on | Holds |
|---|---|---|---|
| `booklet-sync-proto` | lib (new) | `serde` only | The wire types, shared by server and client so the contract cannot drift. |
| `booklet-sync-server` | bin (new) | axum, tokio, sqlx, argon2, sha2, `booklet-sync-proto` | Routes, storage, auth, the CLI. |
| client engine (2d) | module in the app | `ureq`, `booklet-core`, `booklet-sync-proto` | Off-UI-thread sync loop. Not this note. |

- **`booklet-sync-proto` is not part of `booklet-core`.** Putting the wire types
  in core would drag `pulldown-cmark` and `trash` onto the server, and `trash`
  wants a desktop session a headless box does not have (roadmap 2d). It is a
  plain `serde` crate with two real consumers today, so it does not trip the
  "no abstraction until the third use" rule.
- **`EntryKind` is duplicated**, deliberately. `booklet-core::sync::EntryKind`
  (`Note` / `BookMeta` / `Folder`) and a proto `EntityKind` are the same three
  variants but live in crates that must not depend on each other. The client
  converts across the seam in one `match`. Duplication here is cheaper than the
  coupling that would remove it (Rule 3).

## Storage

**Content-addressed blobs on disk + a SQLite metadata DB.** Reversed from the
first design pass (which mirrored plain markdown on the server): keeping version
history forever makes a plain-file mirror expensive and makes content-addressing
nearly free — a version is a blob, dedup comes built in. The accepted cost: the
server's data directory is unreadable without the DB, and a corrupt DB is total
loss rather than partial. Local disk stays plain markdown; CLAUDE.md's rule is
about the client, and the client is untouched by this.

### Blobs

- One file per content hash: `blobs/<first two hex chars>/<full sha-256 hex>`.
  The shard directory keeps any one directory from holding every blob.
- **The hash is the SHA-256 the client already computed in 2a** (`sync.rs`
  hashes every note with `sha2`). The client sends the digest; the server stores
  the bytes only if that hash is absent. Same digest end to end, hashed once.
- Written atomically (temp file + rename), the same discipline as 2a's manifest.
- On upload the server **re-hashes and rejects a mismatch** — a blob whose name
  lies is the one thing content-addressing must never accept.
- Blobs are **never deleted** while history keeps every version (2c). No
  refcount column yet; it would be dead weight until a retention horizon exists
  (deferred, roadmap 2c). A blob, once stored, stays.

### SQLite, via `sqlx`

SQLite because a self-hosted personal server wants one file it can back up, not a
second daemon to run. `sqlx` (async, compile-checked queries, built-in
migrations) over `rusqlite` because the server is `tokio` and `sqlx::sqlite`
speaks async natively without a `spawn_blocking` wrapper around every query. WAL
mode on. *(This is the load-bearing dependency choice — see Open decisions.)*

```sql
CREATE TABLE users (
  id            INTEGER PRIMARY KEY,
  handle        TEXT UNIQUE NOT NULL,
  password_hash TEXT NOT NULL,              -- argon2id
  is_admin      INTEGER NOT NULL DEFAULT 0, -- M7 uses it; one flag, not a roles table
  disabled      INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  last_seen_at  INTEGER
);

CREATE TABLE devices (
  id           INTEGER PRIMARY KEY,
  user_id      INTEGER NOT NULL REFERENCES users(id),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
  token_hash   TEXT UNIQUE NOT NULL,        -- sha-256(token); see Auth
  issued_at    INTEGER NOT NULL,
  last_seen_at INTEGER,
  revoked_at   INTEGER                      -- non-null = dead; the reason M7 exists
);

CREATE TABLE vaults (
  id         TEXT PRIMARY KEY,              -- uuid; this is the vault_id the client stores in .booklet/sync.json
  user_id    INTEGER NOT NULL REFERENCES users(id),
  name       TEXT NOT NULL,
  seq        INTEGER NOT NULL DEFAULT 0,    -- monotonic per-vault sequence; every mutation bumps it
  created_at INTEGER NOT NULL
);

-- Current state, one row per path. This is what GET /changes scans.
CREATE TABLE entities (
  vault_id   TEXT NOT NULL REFERENCES vaults(id),
  path       TEXT NOT NULL,                 -- vault-relative, '/'-joined (matches 2a manifest keys)
  kind       TEXT NOT NULL,                 -- 'note' | 'bookmeta' | 'folder'
  version    INTEGER NOT NULL,              -- per-file, bumped each mutation; sent as the PUT base
  seq        INTEGER NOT NULL,              -- vault seq of the latest mutation
  blob       TEXT,                          -- content hash; NULL for folders and deletes
  deleted    INTEGER NOT NULL DEFAULT 0,    -- a tombstone stays as a row so the feed can carry it
  moved_from TEXT,                          -- set on the mutation that was a move; else NULL
  PRIMARY KEY (vault_id, path)
);
CREATE INDEX entities_feed ON entities(vault_id, seq);

-- History, forever. The recovery path for a bad merge and the base for the next one.
CREATE TABLE entity_versions (
  vault_id   TEXT NOT NULL,
  path       TEXT NOT NULL,
  version    INTEGER NOT NULL,
  seq        INTEGER NOT NULL,
  blob       TEXT,                          -- NULL = tombstone or folder
  deleted    INTEGER NOT NULL DEFAULT 0,
  moved_from TEXT,
  device_id  INTEGER REFERENCES devices(id),
  created_at INTEGER NOT NULL,
  PRIMARY KEY (vault_id, path, version)
);

CREATE TABLE blobs (
  hash       TEXT PRIMARY KEY,              -- sha-256 hex
  size       INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
```

`entities` is derivable from `entity_versions` (it is the max-version row per
path), but keeping it materialized makes the two hot operations — the feed scan
and the PUT conflict check — single-row/single-index reads instead of aggregates.

## The protocol

Bearer auth on every route except token issuance. Paths are vault-relative and
`/`-joined, identical to the 2a manifest keys, so no path translation is needed
on either side.

| Method & route | Body → Response | Purpose |
|---|---|---|
| `POST /auth/token` | `TokenRequest` → `TokenResponse` | Trade credentials for a device token. |
| `GET /vaults` | → `Vec<VaultSummary>` | The user's vaults, for clone/publish choice. |
| `POST /vaults` | `PublishRequest` → `PublishResponse` | Publish: create an empty server vault, return its `vault_id`. |
| `GET /vaults/:id/changes?since=N` | → `Changes` | The feed: every path with `seq > N`, plus the new cursor. |
| `PUT /vaults/:id/entities/*path` | `PutRequest` → `PutResponse` \| **409** `Conflict` | Upload a create/modify/move. Stale base ⇒ 409. |
| `DELETE /vaults/:id/entities/*path` | `{ base_version }` → `PutResponse` \| **409** | Tombstone a path. |
| `GET /vaults/:id/entities/*path/history` | → `History` | Version list for the 2e history modal. |
| `PUT /blobs/:hash` | bytes → 204 | Upload blob bytes (idempotent; hash verified). |
| `GET /blobs/:hash` | → bytes | Fetch a note's content or a merge base. |

### Wire types (`booklet-sync-proto`)

```rust
// --- auth ---
struct TokenRequest  { handle: String, password: String, device_name: String, platform: String }
struct TokenResponse { token: String, user: String }

// --- vaults ---
struct VaultSummary   { id: String, name: String, seq: u64 }
struct PublishRequest { name: String }
struct PublishResponse{ id: String }

enum EntityKind { Note, BookMeta, Folder }

// --- feed ---
struct Change {
    path: String,
    kind: EntityKind,
    version: u64,
    seq: u64,
    deleted: bool,
    blob: Option<String>,        // content hash; None for folders and deletes
    moved_from: Option<String>,  // lets the receiver treat a move as a move, not delete+create
}
struct Changes { changes: Vec<Change>, cursor: u64 }  // cursor = the new `since` value

// --- upload ---
struct PutRequest {
    kind: EntityKind,
    base_version: u64,           // the version the client last saw; 0 = client believes it is creating
    blob: Option<String>,        // content already uploaded via PUT /blobs; None for folders
    moved_from: Option<String>,
}
struct PutResponse { version: u64, seq: u64 }
struct Conflict    { current_version: u64, current_blob: Option<String> }  // 409 body: enough to fetch the base and merge

// --- history (2e) ---
struct Version { version: u64, seq: u64, blob: Option<String>, deleted: bool, device: String, created_at: i64 }
struct History { versions: Vec<Version> }
```

## Versioning and conflict mechanics

- **Per-vault monotonic `seq`; per-file `version`.** Both server-assigned — no
  clock is ever in the protocol, so two machines' disagreeing clocks can never
  pick a wrong winner. A mutation opens a transaction, does
  `UPDATE vaults SET seq = seq + 1 ... RETURNING seq`, sets the new file version
  to `previous + 1`, writes both the `entities` row and an `entity_versions` row,
  and inserts the blob row if new.
- **The feed is a `seq` scan of current state:** `SELECT ... FROM entities WHERE
  vault_id = ? AND seq > ? ORDER BY seq`. A client behind by many edits gets the
  *latest* state per path, not every intermediate version — which is what sync
  wants. `cursor` in the response is the vault's current `seq`, stored by the
  client as `SyncState.cursor` (2a already reserves the field).
- **409 is first-write-wins.** PUT carries `base_version`; if it does not equal
  the server's current version for that path, the server 409s with the current
  version and blob. The client fetches that blob as the merge base, runs
  `booklet-core::merge::merge_markdown` (or `merge_booklet_json`), and re-PUTs
  with the fresh base. `base_version = 0` against an existing path is the
  **no-common-ancestor** case — the client writes a `conflict_copy_name` file
  (2b) instead of merging.
- **A move is one mutation.** A PUT with `moved_from = Some(old)` tombstones the
  old path and creates the new one **under a single `seq` bump**, so a client
  reading the feed sees both rows together and `moved_from` pairs them. This is
  the roadmap's "a folder move must ride as one entry"; note the client cannot
  yet *detect* a folder move (deferred in 2a), so for now only note moves travel
  this way.
- **Tombstones are forever** — a `deleted` row in both tables, never purged.
  Years of deletions is a few thousand rows. GC (a horizon plus a full-resync
  fallback) is deferred, exactly as the roadmap defers history GC; the schema
  needs no change to add it later.
- **The receiver trashes, never hard-deletes.** When the feed carries a
  tombstone, the client removes the local file to the system Trash (the `trash`
  crate, already a `booklet-core` dep) — 2b's rule, applied in 2d.

## Auth and ownership

- **Device token = opaque high-entropy string** (32 random bytes, base64url),
  returned once from `POST /auth/token`. The server stores only `sha-256(token)`;
  a fast hash is right here because the token is already high-entropy, unlike a
  password. Sent as `Authorization: Bearer <token>`.
- **Passwords are argon2id** (M7's choice, adopted now so the hash format never
  changes).
- **Every `/vaults/:id/*` route checks the token's user owns `:id`**, returning
  404 (not 403 — do not confirm the vault exists to a stranger). Ownership is on
  the routes from the first commit; retrofitting it means auditing every route
  again (roadmap 2c).
- **Accounts are made from the shell, not the web.** The server binary carries a
  CLI: `serve`, and `user create <handle>` (prompts for a password) so 2c has a
  testable account. `admin grant <handle>` and the panel are M7. There is no
  self-registration, no invites, no email.
- **Admin sessions are a separate credential from device tokens** (M7). The
  schema's `is_admin` flag and the token/session split are noted here only so 2c
  does not build a single "is authenticated" helper that M7 would have to tear
  apart.

## Deployment

- Binds `127.0.0.1`, speaks **HTTP**; TLS terminates at a reverse proxy
  (Caddy/nginx), which already solves certificates. Transport stays HTTPS end to
  end, so CLAUDE.md holds; binding to loopback also hands M7 its localhost-only
  `/admin` for free. Cost: deploying is two things (server + proxy).
- Config via a small file or env: DB path, blob directory, bind address.

## How it ties to what already exists

- **2a** hashes with `sha2`; that digest *is* the blob hash — nothing is
  re-hashed. `SyncState { vault_id, server_url, cursor }` is already the client's
  half of this contract: `vault_id` ← `POST /vaults`, `cursor` ← the feed.
- **2b** `merge_markdown` / `merge_booklet_json` / `conflict_copy_name` are the
  client's reconciliation on a 409. The base they need is one `GET /blobs/:hash`
  away because history exists — which is the whole reason the store is
  history-shaped.
- **2d** drives it all from a dedicated thread with blocking `ureq`: poll
  `/changes`, diff the manifest, PUT blobs then entities, reconcile 409s.
- **2e** reads `GET .../history` for the version modal and surfaces the merge
  flag (2b's `clean == false`).

## Open decisions to confirm before coding 2c

1. **`sqlx` vs `rusqlite`.** Recommending `sqlx::sqlite` (async-native, migrations,
   compile-checked queries). `rusqlite` is lighter and synchronous — simpler, but
   every query then needs `spawn_blocking` under `tokio`. Load-bearing and
   awkward to switch later, so worth an explicit yes.
2. **Move representation.** `moved_from` on the PUT collapses a note move into one
   feed entry now, even though 2a only detects note moves. Fine to include the
   field and leave folder moves for later, or drop it from 2c and treat every
   move as delete+create until 2a can detect folder moves. Recommending: keep the
   field — it is cheap and the wire type is the expensive thing to change.
3. **One binary or a `serve` subcommand.** Recommending subcommands (`serve` /
   `user create`) so account bootstrap and the server share one deployable file,
   matching M7's `admin grant`.

## Test plan (2c/2d)

Integration-test the client against the server **in-process on a random port**
(roadmap 2d): publish a vault, PUT a note, poll `/changes` from a second
"device," assert it converges; drive a 409 and assert the merge path; drive a
no-ancestor create and assert a conflict copy. No UI, no external network.
```
