//! The Postgres-backed storage layer over the [`blob`](crate::blob) store.
//!
//! It owns the sync state — accounts, vaults, the per-vault sequence, the current
//! state of every path, and the forever history — and ties the blob store into
//! it. Everything the HTTP routes (a later slice) will do lands here as a typed
//! method, so the storage logic is tested directly against Postgres with no HTTP
//! in the way.
//!
//! Queries are runtime-checked (`query_as`/`query_scalar`), not the compile-time
//! macros, so a build never needs a live database; the tests catch SQL mistakes
//! instead, each against a throwaway database via `#[sqlx::test]`.

use crate::blob::{hash as hash_bytes, Base, BlobStore, Encoding, Meta};
use booklet_sync_proto as proto;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Uuid;
use sqlx::{PgConnection, PgPool};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

/// Storage handle: a Postgres pool plus the blob store on disk.
pub struct Store {
    pool: PgPool,
    blobs: BlobStore,
    /// Where `PUT /blobs` parks uploaded content until an entity finalizes it.
    staging: PathBuf,
}

/// The outcome of a mutation: applied, or refused because the client's base
/// version was stale (the caller turns this into a 409 + merge).
pub enum PutOutcome {
    Applied(proto::PutResponse),
    Conflict(proto::Conflict),
}

impl Store {
    /// Connects, runs migrations, and opens the blob store — the production path.
    pub async fn connect(
        database_url: &str,
        blob_root: impl Into<PathBuf>,
        checkpoint_interval: u32,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new().connect(database_url).await?;
        sqlx::migrate!().run(&pool).await?;

        Ok(Self::over(pool, blob_root, checkpoint_interval))
    }

    /// Builds a store over an existing pool — how `#[sqlx::test]` wires one up,
    /// with the schema already migrated.
    pub fn from_parts(pool: PgPool, blob_root: impl Into<PathBuf>, checkpoint_interval: u32) -> Self {
        Self::over(pool, blob_root, checkpoint_interval)
    }

    fn over(pool: PgPool, blob_root: impl Into<PathBuf>, checkpoint_interval: u32) -> Self {
        let root = blob_root.into();
        let staging = root.join(".staging");
        Self { pool, blobs: BlobStore::new(root, checkpoint_interval), staging }
    }

    // --- accounts and vaults ---

    pub async fn create_user(&self, handle: &str, password_hash: &str) -> Result<i64> {
        let id = sqlx::query_scalar("INSERT INTO users (handle, password_hash) VALUES ($1, $2) RETURNING id")
            .bind(handle)
            .bind(password_hash)
            .fetch_one(&self.pool)
            .await?;

        Ok(id)
    }

    /// Publishes a new, empty vault, returning its server-assigned id.
    pub async fn create_vault(&self, owner: i64, name: &str) -> Result<Uuid> {
        let id = sqlx::query_scalar("INSERT INTO vaults (user_id, name) VALUES ($1, $2) RETURNING id")
            .bind(owner)
            .bind(name)
            .fetch_one(&self.pool)
            .await?;

        Ok(id)
    }

    pub async fn vault_seq(&self, vault: Uuid) -> Result<u64> {
        let seq: i64 = sqlx::query_scalar("SELECT seq FROM vaults WHERE id = $1")
            .bind(vault)
            .fetch_one(&self.pool)
            .await?;

        Ok(seq as u64)
    }

    /// The vaults a user owns, for `GET /vaults`.
    pub async fn list_vaults(&self, user_id: i64) -> Result<Vec<proto::VaultSummary>> {
        let rows: Vec<(Uuid, String, i64)> =
            sqlx::query_as("SELECT id, name, seq FROM vaults WHERE user_id = $1 ORDER BY created_at")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows
            .into_iter()
            .map(|(id, name, seq)| proto::VaultSummary { id: id.to_string(), name, seq: seq as u64 })
            .collect())
    }

    /// The owner of a vault, so a route can refuse a vault that is not the
    /// caller's — with a 404, never confirming it exists to a stranger.
    pub async fn vault_owner(&self, vault: Uuid) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT user_id FROM vaults WHERE id = $1")
            .bind(vault)
            .fetch_optional(&self.pool)
            .await?)
    }

    // --- accounts and devices (auth) ---

    /// A user's id, password hash, and disabled flag, for the sign-in check.
    pub async fn find_user_by_handle(&self, handle: &str) -> Result<Option<(i64, String, bool)>> {
        Ok(sqlx::query_as("SELECT id, password_hash, disabled FROM users WHERE handle = $1")
            .bind(handle)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Records a device and the hash of the token issued to it.
    pub async fn create_device(
        &self,
        user_id: i64,
        name: &str,
        platform: &str,
        token_hash: &str,
    ) -> Result<i64> {
        let id = sqlx::query_scalar(
            "INSERT INTO devices (user_id, name, platform, token_hash) VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(user_id)
        .bind(name)
        .bind(platform)
        .bind(token_hash)
        .fetch_one(&self.pool)
        .await?;

        Ok(id)
    }

    /// The device and its owner for a live (non-revoked) token hash.
    pub async fn device_for_token(&self, token_hash: &str) -> Result<Option<(i64, i64)>> {
        Ok(sqlx::query_as(
            "SELECT id, user_id FROM devices WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    // --- blobs ---

    /// Reconstructs a blob's content, verified against its hash. Fetches the
    /// whole delta chain in one recursive query, then walks it on disk.
    pub async fn get_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let metas = self.chain_metas(hash).await?;

        self.blobs.get(hash, &|hash| metas.get(hash).cloned()).map_err(Error::Blob)
    }

    /// A blob and every ancestor it deltas against, as a map for the store to
    /// reconstruct from.
    async fn chain_metas(&self, hash: &str) -> Result<HashMap<String, Meta>> {
        let rows: Vec<BlobRow> = sqlx::query_as(
            "WITH RECURSIVE chain AS (
                 SELECT hash, encoding, base_hash, depth, stored_size FROM blobs WHERE hash = $1
                 UNION ALL
                 SELECT b.hash, b.encoding, b.base_hash, b.depth, b.stored_size
                   FROM blobs b JOIN chain c ON b.hash = c.base_hash
             )
             SELECT hash, encoding, base_hash, depth, stored_size FROM chain",
        )
        .bind(hash)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|row| (row.hash.clone(), row.into_meta())).collect())
    }

    /// Whether a finalized blob exists, for `GET /blobs` and the client's dedup
    /// check before an upload.
    pub async fn has_blob(&self, hash: &str) -> Result<bool> {
        self.blob_exists(hash).await
    }

    async fn blob_exists(&self, hash: &str) -> Result<bool> {
        let found: Option<i32> = sqlx::query_scalar("SELECT 1 FROM blobs WHERE hash = $1")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await?;

        Ok(found.is_some())
    }

    /// Parks uploaded content under `.staging` until an entity finalizes it,
    /// returning its hash. Written atomically, like every other blob file.
    pub async fn stage_blob(&self, bytes: &[u8]) -> Result<String> {
        let hash = hash_bytes(bytes);

        fs::create_dir_all(&self.staging).map_err(Error::Blob)?;
        let path = self.staging.join(&hash);
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes).map_err(Error::Blob)?;
        fs::rename(&temp, &path).map_err(Error::Blob)?;

        Ok(hash)
    }

    fn staged_content(&self, hash: &str) -> Option<Vec<u8>> {
        fs::read(self.staging.join(hash)).ok()
    }

    fn unstage(&self, hash: &str) {
        let _ = fs::remove_file(self.staging.join(hash));
    }

    /// The bytes for a referenced hash: from staging if freshly uploaded, else
    /// reconstructed if the server already holds it, else a missing-blob error
    /// (the entity named content that was never uploaded).
    async fn resolve_content(&self, hash: &str) -> Result<Vec<u8>> {
        if let Some(bytes) = self.staged_content(hash) {
            return Ok(bytes);
        }
        if self.blob_exists(hash).await? {
            return self.get_blob(hash).await;
        }

        Err(Error::MissingBlob(hash.to_string()))
    }

    /// Encodes new content and writes its blob file, returning the chain metadata
    /// to persist. Deltas against `base` (the version being superseded); with no
    /// base it is a full checkpoint. The file is written here, before the mutation
    /// transaction, so a rolled-back mutation leaves only an orphan file — never a
    /// row that points at a file that does not match it.
    async fn finalize_blob(&self, hash: &str, content: &[u8], base: Option<&str>) -> Result<Meta> {
        let meta = match base {
            Some(base_hash) => {
                let base_content = self.get_blob(base_hash).await?;
                let base_depth = self.blob_depth(base_hash).await?;
                self.blobs.put(
                    hash,
                    content,
                    Some(Base { hash: base_hash, content: &base_content, depth: base_depth }),
                )
            }
            None => self.blobs.put(hash, content, None),
        };

        meta.map_err(Error::Blob)
    }

    async fn blob_depth(&self, hash: &str) -> Result<u32> {
        let depth: i32 = sqlx::query_scalar("SELECT depth FROM blobs WHERE hash = $1")
            .bind(hash)
            .fetch_one(&self.pool)
            .await?;

        Ok(depth as u32)
    }

    // --- mutations ---

    /// Creates, modifies, or moves an entity. `content` is the note's bytes
    /// (`None` for a folder); a move also carries `moved_from`, the old path,
    /// which is tombstoned under the same sequence so the two ride the feed as one
    /// move. A stale `base_version` yields [`PutOutcome::Conflict`] with no writes.
    pub async fn apply_put(
        &self,
        vault: Uuid,
        path: &str,
        kind: proto::EntityKind,
        base_version: u64,
        content: Option<&[u8]>,
        moved_from: Option<&str>,
    ) -> Result<PutOutcome> {
        let current = self.current(vault, path).await?;
        if base_version != current.version as u64 {
            return Ok(PutOutcome::Conflict(proto::Conflict {
                current_version: current.version as u64,
                current_blob: current.blob,
            }));
        }

        // Store the blob (encoded against the version being replaced) before the
        // transaction, but only if this content is new to the server.
        let mut fresh_blob: Option<(Meta, usize)> = None;
        let blob_hash = match content {
            Some(bytes) => {
                let hash = hash_bytes(bytes);
                if !self.blob_exists(&hash).await? {
                    let meta = self.finalize_blob(&hash, bytes, current.blob.as_deref()).await?;
                    fresh_blob = Some((meta, bytes.len()));
                }
                Some(hash)
            }
            None => None,
        };

        let moved = match moved_from {
            Some(old) => Some((old.to_string(), self.current(vault, old).await?)),
            None => None,
        };

        let mut tx = self.pool.begin().await?;

        if let Some((meta, size)) = &fresh_blob {
            insert_blob(&mut tx, blob_hash.as_deref().unwrap(), *size, meta).await?;
        }

        let seq = next_seq(&mut tx, vault).await?;
        let version = current.version + 1;

        // A move tombstones the old path under the same sequence.
        if let Some((old, old_current)) = &moved {
            let old_version = old_current.version + 1;
            write_state(&mut tx, vault, old, &old_current.kind, old_version, seq, None, true, None).await?;
        }

        write_state(&mut tx, vault, path, kind_to_db(kind), version, seq, blob_hash.as_deref(), false, moved_from).await?;

        tx.commit().await?;

        Ok(PutOutcome::Applied(proto::PutResponse { version: version as u64, seq: seq as u64 }))
    }

    /// The HTTP entrypoint: like [`apply_put`](Self::apply_put) but the blob
    /// arrives as a content hash (staged by a prior `PUT /blobs`, or already on
    /// the server), which is resolved to bytes here. On success the staged copy is
    /// dropped — it has been finalized into the blob store.
    pub async fn apply_put_ref(
        &self,
        vault: Uuid,
        path: &str,
        kind: proto::EntityKind,
        base_version: u64,
        blob: Option<&str>,
        moved_from: Option<&str>,
    ) -> Result<PutOutcome> {
        let content = match blob {
            Some(hash) => Some(self.resolve_content(hash).await?),
            None => None,
        };

        let outcome = self
            .apply_put(vault, path, kind, base_version, content.as_deref(), moved_from)
            .await?;

        if let (Some(hash), PutOutcome::Applied(_)) = (blob, &outcome) {
            self.unstage(hash);
        }

        Ok(outcome)
    }

    /// Deletes an entity, leaving a tombstone. Guarded by `base_version` the same
    /// way a put is.
    pub async fn apply_delete(&self, vault: Uuid, path: &str, base_version: u64) -> Result<PutOutcome> {
        let current = self.current(vault, path).await?;
        if base_version != current.version as u64 {
            return Ok(PutOutcome::Conflict(proto::Conflict {
                current_version: current.version as u64,
                current_blob: current.blob,
            }));
        }

        let mut tx = self.pool.begin().await?;

        let seq = next_seq(&mut tx, vault).await?;
        let version = current.version + 1;
        write_state(&mut tx, vault, path, &current.kind, version, seq, None, true, None).await?;

        tx.commit().await?;

        Ok(PutOutcome::Applied(proto::PutResponse { version: version as u64, seq: seq as u64 }))
    }

    async fn current(&self, vault: Uuid, path: &str) -> Result<Current> {
        let row: Option<(i64, Option<String>, bool, String)> = sqlx::query_as(
            "SELECT version, blob, deleted, kind FROM entities WHERE vault_id = $1 AND path = $2",
        )
        .bind(vault)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(match row {
            Some((version, blob, deleted, kind)) => Current { version, blob, deleted, kind },
            // An absent path reads as version 0, so a client's `base_version = 0`
            // (a create) matches.
            None => Current { version: 0, blob: None, deleted: true, kind: "note".into() },
        })
    }

    // --- reads ---

    /// The change feed: every path whose latest sequence is past `since`, plus the
    /// vault's current sequence to store as the next cursor.
    pub async fn changes_since(&self, vault: Uuid, since: u64) -> Result<proto::Changes> {
        let rows: Vec<ChangeRow> = sqlx::query_as(
            "SELECT path, kind, version, seq, deleted, blob, moved_from
               FROM entities WHERE vault_id = $1 AND seq > $2 ORDER BY seq",
        )
        .bind(vault)
        .bind(since as i64)
        .fetch_all(&self.pool)
        .await?;

        let cursor = self.vault_seq(vault).await?;
        let changes = rows.into_iter().map(ChangeRow::into_change).collect();

        Ok(proto::Changes { changes, cursor })
    }

    /// A path's full version history, oldest first, for the version modal.
    pub async fn history(&self, vault: Uuid, path: &str) -> Result<proto::History> {
        let rows: Vec<VersionRow> = sqlx::query_as(
            "SELECT ev.version, ev.seq, ev.blob, ev.deleted,
                    COALESCE(d.name, '') AS device,
                    (extract(epoch FROM ev.created_at) * 1000)::bigint AS created_ms
               FROM entity_versions ev
               LEFT JOIN devices d ON d.id = ev.device_id
              WHERE ev.vault_id = $1 AND ev.path = $2
              ORDER BY ev.version",
        )
        .bind(vault)
        .bind(path)
        .fetch_all(&self.pool)
        .await?;

        let versions = rows.into_iter().map(VersionRow::into_version).collect();

        Ok(proto::History { versions })
    }
}

/// The current state of a path, or the zero state if it has none yet.
struct Current {
    version: i64,
    blob: Option<String>,
    #[allow(dead_code)] // read in a later slice; kept for symmetry with the row
    deleted: bool,
    kind: String,
}

// --- transaction helpers (operate on the mutation's connection) ---

async fn next_seq(tx: &mut PgConnection, vault: Uuid) -> Result<i64> {
    let seq = sqlx::query_scalar("UPDATE vaults SET seq = seq + 1 WHERE id = $1 RETURNING seq")
        .bind(vault)
        .fetch_one(tx)
        .await?;

    Ok(seq)
}

async fn insert_blob(tx: &mut PgConnection, hash: &str, size: usize, meta: &Meta) -> Result<()> {
    sqlx::query(
        "INSERT INTO blobs (hash, size, stored_size, encoding, base_hash, depth)
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (hash) DO NOTHING",
    )
    .bind(hash)
    .bind(size as i64)
    .bind(meta.stored_size as i64)
    .bind(encoding_to_db(meta.encoding))
    .bind(meta.base.as_deref())
    .bind(meta.depth as i32)
    .execute(tx)
    .await?;

    Ok(())
}

/// Writes both the current-state row (upserted) and a history row, the pair that
/// every mutation leaves behind.
#[allow(clippy::too_many_arguments)]
async fn write_state(
    tx: &mut PgConnection,
    vault: Uuid,
    path: &str,
    kind: &str,
    version: i64,
    seq: i64,
    blob: Option<&str>,
    deleted: bool,
    moved_from: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO entities (vault_id, path, kind, version, seq, blob, deleted, moved_from)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (vault_id, path) DO UPDATE SET
             kind = EXCLUDED.kind, version = EXCLUDED.version, seq = EXCLUDED.seq,
             blob = EXCLUDED.blob, deleted = EXCLUDED.deleted, moved_from = EXCLUDED.moved_from",
    )
    .bind(vault)
    .bind(path)
    .bind(kind)
    .bind(version)
    .bind(seq)
    .bind(blob)
    .bind(deleted)
    .bind(moved_from)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO entity_versions (vault_id, path, version, seq, blob, deleted, moved_from)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(vault)
    .bind(path)
    .bind(version)
    .bind(seq)
    .bind(blob)
    .bind(deleted)
    .bind(moved_from)
    .execute(&mut *tx)
    .await?;

    Ok(())
}

// --- row types and mappings ---

#[derive(sqlx::FromRow)]
struct BlobRow {
    hash: String,
    encoding: String,
    base_hash: Option<String>,
    depth: i32,
    stored_size: i64,
}

impl BlobRow {
    fn into_meta(self) -> Meta {
        Meta {
            encoding: encoding_from_db(&self.encoding),
            base: self.base_hash,
            depth: self.depth as u32,
            stored_size: self.stored_size as u64,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ChangeRow {
    path: String,
    kind: String,
    version: i64,
    seq: i64,
    deleted: bool,
    blob: Option<String>,
    moved_from: Option<String>,
}

impl ChangeRow {
    fn into_change(self) -> proto::Change {
        proto::Change {
            path: self.path,
            kind: kind_from_db(&self.kind),
            version: self.version as u64,
            seq: self.seq as u64,
            deleted: self.deleted,
            blob: self.blob,
            moved_from: self.moved_from,
        }
    }
}

#[derive(sqlx::FromRow)]
struct VersionRow {
    version: i64,
    seq: i64,
    blob: Option<String>,
    deleted: bool,
    device: String,
    created_ms: i64,
}

impl VersionRow {
    fn into_version(self) -> proto::Version {
        proto::Version {
            version: self.version as u64,
            seq: self.seq as u64,
            blob: self.blob,
            deleted: self.deleted,
            device: self.device,
            created_at: self.created_ms,
        }
    }
}

fn kind_to_db(kind: proto::EntityKind) -> &'static str {
    match kind {
        proto::EntityKind::Note => "note",
        proto::EntityKind::BookMeta => "bookmeta",
        proto::EntityKind::Folder => "folder",
    }
}

fn kind_from_db(kind: &str) -> proto::EntityKind {
    match kind {
        "bookmeta" => proto::EntityKind::BookMeta,
        "folder" => proto::EntityKind::Folder,
        _ => proto::EntityKind::Note,
    }
}

fn encoding_to_db(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Full => "full",
        Encoding::Delta => "delta",
    }
}

fn encoding_from_db(encoding: &str) -> Encoding {
    match encoding {
        "delta" => Encoding::Delta,
        _ => Encoding::Full,
    }
}

// --- errors ---

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Db(sqlx::Error),
    Migrate(sqlx::migrate::MigrateError),
    Blob(io::Error),
    /// An entity referenced content that was never uploaded — a client protocol
    /// error, surfaced as a 400.
    MissingBlob(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(error) => write!(f, "database error: {error}"),
            Error::Migrate(error) => write!(f, "migration error: {error}"),
            Error::Blob(error) => write!(f, "blob store error: {error}"),
            Error::MissingBlob(hash) => write!(f, "referenced blob {hash} was not uploaded"),
        }
    }
}

impl std::error::Error for Error {}

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Error::Db(error)
    }
}

impl From<sqlx::migrate::MigrateError> for Error {
    fn from(error: sqlx::migrate::MigrateError) -> Self {
        Error::Migrate(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn blob_root() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("booklet-store-{}-{}", std::process::id(), unique))
    }

    /// A store over the test database, plus a user and an empty vault to work in.
    async fn seed(pool: PgPool) -> (Store, Uuid) {
        let store = Store::from_parts(pool, blob_root(), 50);
        let user = store.create_user("alice", "argon2-hash").await.unwrap();
        let vault = store.create_vault(user, "Personal").await.unwrap();
        (store, vault)
    }

    fn note() -> proto::EntityKind {
        proto::EntityKind::Note
    }

    #[sqlx::test]
    async fn a_published_vault_starts_at_sequence_zero(pool: PgPool) {
        let (store, vault) = seed(pool).await;

        assert_eq!(store.vault_seq(vault).await.unwrap(), 0);
        assert!(store.changes_since(vault, 0).await.unwrap().changes.is_empty());
    }

    #[sqlx::test]
    async fn a_create_then_edit_stores_a_delta_and_the_feed_reflects_it(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let base = "a body of prose that stays put. ".repeat(20);

        let v1 = format!("{base}\nfirst\n").into_bytes();
        let applied = store.apply_put(vault, "Note.md", note(), 0, Some(&v1), None).await.unwrap();
        assert!(matches!(applied, PutOutcome::Applied(_)));

        let v2 = format!("{base}\nsecond\n").into_bytes();
        store.apply_put(vault, "Note.md", note(), 1, Some(&v2), None).await.unwrap();

        // Both versions reconstruct exactly, through the delta chain.
        let v2_hash = hash_bytes(&v2);
        assert_eq!(store.get_blob(&v2_hash).await.unwrap(), v2);
        let v1_hash = hash_bytes(&v1);
        assert_eq!(store.get_blob(&v1_hash).await.unwrap(), v1);

        // The feed shows the note at version 2, and the cursor is 2 (two bumps).
        let changes = store.changes_since(vault, 0).await.unwrap();
        assert_eq!(changes.cursor, 2);
        assert_eq!(changes.changes.len(), 1);
        assert_eq!(changes.changes[0].path, "Note.md");
        assert_eq!(changes.changes[0].version, 2);
        assert_eq!(changes.changes[0].blob, Some(v2_hash));
    }

    #[sqlx::test]
    async fn a_stale_base_version_conflicts_without_writing(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        store.apply_put(vault, "Note.md", note(), 0, Some(b"one"), None).await.unwrap();

        // The client still thinks the note is at version 0.
        let outcome = store.apply_put(vault, "Note.md", note(), 0, Some(b"two"), None).await.unwrap();

        match outcome {
            PutOutcome::Conflict(conflict) => assert_eq!(conflict.current_version, 1),
            PutOutcome::Applied(_) => panic!("stale write should conflict"),
        }
        // The sequence did not move — nothing was written.
        assert_eq!(store.vault_seq(vault).await.unwrap(), 1);
    }

    #[sqlx::test]
    async fn the_feed_is_incremental(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        store.apply_put(vault, "A.md", note(), 0, Some(b"a"), None).await.unwrap();
        store.apply_put(vault, "B.md", note(), 0, Some(b"b"), None).await.unwrap();

        // A client caught up to seq 1 only sees what changed after it.
        let changes = store.changes_since(vault, 1).await.unwrap();
        assert_eq!(changes.changes.len(), 1);
        assert_eq!(changes.changes[0].path, "B.md");
        assert!(store.changes_since(vault, 2).await.unwrap().changes.is_empty());
    }

    #[sqlx::test]
    async fn history_lists_every_version(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        for edit in 0..3 {
            let base = edit as u64;
            store
                .apply_put(vault, "Note.md", note(), base, Some(format!("v{edit}").as_bytes()), None)
                .await
                .unwrap();
        }

        let history = store.history(vault, "Note.md").await.unwrap();
        let versions: Vec<_> = history.versions.iter().map(|v| v.version).collect();
        assert_eq!(versions, [1, 2, 3]);
    }

    #[sqlx::test]
    async fn a_delete_tombstones_the_note(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        store.apply_put(vault, "Note.md", note(), 0, Some(b"content"), None).await.unwrap();

        store.apply_delete(vault, "Note.md", 1).await.unwrap();

        let changes = store.changes_since(vault, 0).await.unwrap();
        assert_eq!(changes.changes.len(), 1);
        assert!(changes.changes[0].deleted);
        assert_eq!(changes.changes[0].blob, None);
    }

    #[sqlx::test]
    async fn a_move_tombstones_the_old_path_and_creates_the_new_under_one_sequence(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let content = b"the note that moves";
        store.apply_put(vault, "Old.md", note(), 0, Some(content), None).await.unwrap();

        store.apply_put(vault, "New.md", note(), 0, Some(content), Some("Old.md")).await.unwrap();

        let changes = store.changes_since(vault, 0).await.unwrap();
        let old = changes.changes.iter().find(|c| c.path == "Old.md").unwrap();
        let new = changes.changes.iter().find(|c| c.path == "New.md").unwrap();

        assert!(old.deleted);
        assert!(!new.deleted);
        assert_eq!(new.moved_from.as_deref(), Some("Old.md"));
        // Both sides of the move share the one sequence it was applied under.
        assert_eq!(old.seq, new.seq);
    }

    #[sqlx::test]
    async fn identical_content_is_stored_once(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let content = b"the very same bytes in two places";
        store.apply_put(vault, "One.md", note(), 0, Some(content), None).await.unwrap();
        store.apply_put(vault, "Two.md", note(), 0, Some(content), None).await.unwrap();

        let blobs: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(blobs, 1);
    }
}
