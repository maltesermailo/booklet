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
| `booklet-sync-server` | bin (new) | axum, tokio, sqlx (postgres), argon2, sha2, uuid, zstd + a delta codec, `booklet-sync-proto` | Routes, blob store, auth, the CLI. |
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

**Content-addressed blobs on disk + a PostgreSQL metadata DB.** Reversed from the
first design pass (which mirrored plain markdown on the server): keeping version
history forever makes a plain-file mirror expensive and makes content-addressing
nearly free — a version is a blob, stored **Git-style as a delta against the
previous version**, and dedup comes built in. The accepted cost: the
server's data directory is unreadable without the DB — the blobs are named only
by their content hash, so the metadata is what makes them a vault. **The blob
directory and the database must be backed up as one unit**, and a restore is only
consistent if they were captured together. Local disk stays plain markdown;
CLAUDE.md's rule is about the client, and the client is untouched by this.

### Blobs — content-addressed, stored as Git-style delta chains

The store's *interface* is content-addressed: `get(hash) -> bytes` and
`put(hash, bytes)`. The client only ever sees hashes — the SHA-256 it already
computed in 2a (`sync.rs` hashes every note with `sha2`) — so how the bytes are
physically kept is entirely the server's business and can change without touching
the protocol.

Physically, a note revised many times is the common case, and storing each
version whole would grow history by (versions × note size). So versions are
packed **Git-style: a full checkpoint every K versions, and a delta against the
previous version in between.** Full checkpoints are plain zstd; the deltas are
where the storage saving comes from.

- **One file per content hash**, sharded: `blobs/<first two hex chars>/<full
  sha-256 hex>`, so no directory holds every blob. The file's *bytes* are a full
  zstd blob or a delta; the metadata row says which.
- **Interface & dedup.** `put(hash, bytes)` is a no-op when `hash` already
  exists — identical content (a revert, or the same note in two books) is stored
  once. The digest is the SHA-256 from 2a; hashed once, end to end.
- **Two-step upload; the base is the path's own lineage.** The client always
  sends whole content and never computes deltas. First it `PUT /blobs/:hash`s the
  full bytes, which the server stores **loose — one full, zstd-compressed file** —
  knowing nothing yet about which path or version this is. Then it
  `PUT /vaults/:id/entities/*path`s the "commit" naming that hash as version N of
  a path. **That** is where the server learns the lineage, so that is where it
  deltafies: version N's base is version N-1's blob (history is linear per path,
  so no similarity search — Git's hard part — is needed), and the just-uploaded
  full blob is re-encoded as a delta against it. The hash never changes; only the
  on-disk representation goes full → delta.
- **References cannot dangle; orphans are pruned.** `entities.blob` and
  `entity_versions.blob` are a **foreign key** to `blobs.hash`, so an entity PUT
  naming a hash the server does not have is rejected — that is why blobs upload
  first. The reverse (a blob no entity ever references — a client that crashed
  between the two steps, or one just spraying `PUT /blobs`) is an **orphan**, and
  a sweep deletes any blob unreferenced by an `entity_versions` row and older than
  a short grace window (long enough to cover an in-flight sync batch), exactly as
  `git gc` prunes unreachable objects. Every route is authenticated and a per-blob
  size cap bounds a single upload, so orphans are transient junk, not an attack
  surface. This prune is **not** the deferred history horizon (which prunes
  *referenced* old versions); it is basic hygiene and ships with 2c.
- **Bounded chains.** Each blob row carries `depth` (deltas back to a full). If
  encoding version N as a delta would push depth past K, it is stored as a full
  checkpoint (`depth = 0`) instead. Reconstruction walks `base_hash` back to a
  full and applies deltas forward — at most K small applies.
- **Exact and verified.** The delta must reconstruct byte-for-byte — the merge
  base and the recovery path depend on it — so **`diff-match-patch` is *not*
  usable here** (it matches fuzzily; that is 2b's job, not storage). The codec is
  an exact binary delta (`zstd --patch-from` or `qbsdiff`; see Open decisions).
  After reconstructing, the server **re-hashes and checks the result equals the
  requested hash**, so a broken chain is caught, never served. Upload likewise
  rejects a blob whose bytes do not hash to its name.
- **Forever-history keeps chains stable.** No *referenced* blob is ever deleted
  while history is kept (2c) — the orphan prune above only touches blobs nothing
  points at — so a delta's base never vanishes and chains never need repair. A
  later retention horizon (deferred, roadmap 2c), which *would* remove referenced
  old versions, must re-materialize any delta whose base it removes — noted so the
  coupling is not a surprise.
- Files are written atomically (temp + rename), the discipline from 2a's manifest.

This is the store's most intricate part, and also its most isolated — everything
above `get`/`put` deals only in hashes.

### PostgreSQL, via `sqlx`

PostgreSQL (chosen over the earlier SQLite pick). It costs a second daemon to run
and back up, but this is a **multi-user** server where several devices PUT at
once, and Postgres handles concurrent writers properly rather than serializing
them behind SQLite's single-writer lock. It also turns "a corrupt DB is total
loss" from a single-file gamble into an operational problem with known tools
(PITR, replication, `pg_dump`). `sqlx::postgres` for the driver — async-native
under `tokio`, compile-checked queries, built-in migrations. Requires
**PostgreSQL 13+** so `gen_random_uuid()` is in core (older versions need the
`pgcrypto` extension enabled).

```sql
CREATE TABLE users (
  id            BIGSERIAL PRIMARY KEY,
  handle        TEXT UNIQUE NOT NULL,
  password_hash TEXT NOT NULL,                    -- argon2id
  is_admin      BOOLEAN NOT NULL DEFAULT FALSE,   -- M7 uses it; one flag, not a roles table
  disabled      BOOLEAN NOT NULL DEFAULT FALSE,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at  TIMESTAMPTZ
);

CREATE TABLE devices (
  id           BIGSERIAL PRIMARY KEY,
  user_id      BIGINT NOT NULL REFERENCES users(id),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
  token_hash   TEXT UNIQUE NOT NULL,              -- sha-256(token); see Auth
  issued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at TIMESTAMPTZ,
  revoked_at   TIMESTAMPTZ                         -- non-null = dead; the reason M7 exists
);

CREATE TABLE vaults (
  id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),  -- the vault_id the client stores in .booklet/sync.json
  user_id    BIGINT NOT NULL REFERENCES users(id),
  name       TEXT NOT NULL,
  seq        BIGINT NOT NULL DEFAULT 0,            -- monotonic per-vault sequence; every mutation bumps it
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Current state, one row per path. This is what GET /changes scans.
CREATE TABLE entities (
  vault_id   UUID NOT NULL REFERENCES vaults(id),
  path       TEXT NOT NULL,                        -- vault-relative, '/'-joined (matches 2a manifest keys)
  kind       TEXT NOT NULL,                        -- 'note' | 'bookmeta' | 'folder'
  version    BIGINT NOT NULL,                      -- per-file, bumped each mutation; sent as the PUT base
  seq        BIGINT NOT NULL,                      -- vault seq of the latest mutation
  blob       TEXT,                                 -- content hash; NULL for folders and deletes
  deleted    BOOLEAN NOT NULL DEFAULT FALSE,       -- a tombstone stays as a row so the feed can carry it
  moved_from TEXT,                                 -- set on the mutation that was a move; else NULL
  PRIMARY KEY (vault_id, path)
);
CREATE INDEX entities_feed ON entities(vault_id, seq);

-- History, forever. The recovery path for a bad merge and the base for the next one.
CREATE TABLE entity_versions (
  vault_id   UUID NOT NULL,
  path       TEXT NOT NULL,
  version    BIGINT NOT NULL,
  seq        BIGINT NOT NULL,
  blob       TEXT,                                 -- NULL = tombstone or folder
  deleted    BOOLEAN NOT NULL DEFAULT FALSE,
  moved_from TEXT,
  device_id  BIGINT REFERENCES devices(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (vault_id, path, version)
);

CREATE TABLE blobs (
  hash        TEXT PRIMARY KEY,                    -- sha-256 hex of the ORIGINAL (uncompressed) content
  size        BIGINT NOT NULL,                     -- original content length
  stored_size BIGINT NOT NULL,                     -- bytes actually on disk (the delta or the zstd full)
  encoding    TEXT NOT NULL,                       -- 'full' | 'delta'
  base_hash   TEXT REFERENCES blobs(hash),         -- NULL for a full checkpoint; else the delta base
  depth       INTEGER NOT NULL DEFAULT 0,          -- deltas back to a full; bounds reconstruction (<= K)
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

`size` vs `stored_size` is also what feeds M7's blob-store-health page: the ratio
is how the operator sees the delta packing working (or not).

`entities` is derivable from `entity_versions` (it is the max-version row per
path), but keeping it materialized makes the two hot operations — the feed scan
and the PUT conflict check — single-row/single-index reads instead of aggregates.

The vault UUID is **server-generated** (`gen_random_uuid()` on `POST /vaults`),
which is why the client's `SyncState.vault_id` is `None` until publish. `seq` and
`version` are `BIGINT`; `created_at` is a real `TIMESTAMPTZ` (display metadata
only — ordering is by `seq`/`version`, never by clock), which the wire types
carry as an epoch `i64`.

## The protocol

Bearer auth on every route except token issuance. Paths are vault-relative and
`/`-joined, identical to the 2a manifest keys, so no path translation is needed
on either side. (History is `GET /vaults/:id/history/*path`, not
`…/entities/*path/history` — axum's trailing wildcard must be the last segment,
so history could not hang off the same `entities/*path`.)

| Method & route | Body → Response | Purpose |
|---|---|---|
| `POST /auth/token` | `TokenRequest` → `TokenResponse` | Trade credentials for a device token. |
| `GET /vaults` | → `Vec<VaultSummary>` | The user's vaults, for clone/publish choice. |
| `POST /vaults` | `PublishRequest` → `PublishResponse` | Publish: create an empty server vault, return its `vault_id`. |
| `GET /vaults/:id/changes?since=N` | → `Changes` | The feed: every path with `seq > N`, plus the new cursor. |
| `PUT /vaults/:id/entities/*path` | `PutRequest` → `PutResponse` \| **409** `Conflict` | Upload a create/modify/move. Stale base ⇒ 409. |
| `DELETE /vaults/:id/entities/*path` | `{ base_version }` → `PutResponse` \| **409** | Tombstone a path. |
| `GET /vaults/:id/history/*path` | → `History` | Version list for the 2e history modal. |
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
  `/admin` for free.
- **Deploying is three moving parts** now: the server, the reverse proxy, and a
  PostgreSQL instance the server can reach. Postgres can sit on the same box
  (loopback) for a personal deployment.
- Config via a small file or env: the **database URL**, the blob directory, and
  the bind address.

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

## Decisions (all five confirmed 2026-07-17 — recorded with rationale)

1. **Postgres driver.** Recommending `sqlx::postgres` (async-native, migrations,
   compile-checked queries against a live schema). The alternative is
   `tokio-postgres` + a pool (`deadpool`/`bb8`) — lighter, but hand-rolled
   migrations and no compile-time query checking. Awkward to switch later, so
   worth an explicit yes.
2. **Move representation.** `moved_from` on the PUT collapses a note move into one
   feed entry now, even though 2a only detects note moves. Fine to include the
   field and leave folder moves for later, or drop it from 2c and treat every
   move as delete+create until 2a can detect folder moves. Recommending: keep the
   field — it is cheap and the wire type is the expensive thing to change.
3. **One binary or a `serve` subcommand.** Recommending subcommands (`serve` /
   `user create`) so account bootstrap and the server share one deployable file,
   matching M7's `admin grant`.
4. **Delta codec.** The chain design is codec-independent; the codec just has to
   be an exact, lossless binary delta. `qbsdiff` (bsdiff/bspatch, pure Rust,
   purpose-built for deltas between similar files) is the straightforward pick;
   `zstd --patch-from` (reference-prefix mode) folds delta and compression into
   the one crate but its Rust binding for ref-prefix is less ergonomic. Small,
   swappable, but worth naming before coding.
5. **Checkpoint interval K.** How many deltas before a full checkpoint. Suggesting
   **~50** — reconstruction then touches at most 50 small applies, and a note
   revised 50 times costs one full plus 49 deltas rather than 50 fulls. Pure
   tuning, changeable any time (it only affects newly written chains).

## Test plan (2c/2d)

Integration-test the client against the server **in-process on a random port**
(roadmap 2d): publish a vault, PUT a note, poll `/changes` from a second
"device," assert it converges; drive a 409 and assert the merge path; drive a
no-ancestor create and assert a conflict copy. No UI, no external network.
```
