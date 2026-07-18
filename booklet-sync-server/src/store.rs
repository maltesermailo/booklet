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
    /// The blob directory root, kept so the admin panel can report free space.
    blob_root: PathBuf,
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
        Self { pool, blobs: BlobStore::new(root.clone(), checkpoint_interval), blob_root: root, staging }
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

    /// The vaults a user owns, for `GET /vaults`. Soft-deleted vaults are hidden.
    pub async fn list_vaults(&self, user_id: i64) -> Result<Vec<proto::VaultSummary>> {
        let rows: Vec<(Uuid, String, i64)> = sqlx::query_as(
            "SELECT id, name, seq FROM vaults WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, name, seq)| proto::VaultSummary { id: id.to_string(), name, seq: seq as u64 })
            .collect())
    }

    /// The owner of a live vault, so a route can refuse a vault that is not the
    /// caller's — with a 404, never confirming it exists to a stranger. A
    /// soft-deleted vault reads as absent, so every sync route 404s on it.
    pub async fn vault_owner(&self, vault: Uuid) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT user_id FROM vaults WHERE id = $1 AND deleted_at IS NULL")
            .bind(vault)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Soft-deletes a vault the user owns: it vanishes from their list and every
    /// sync route, but its rows and blobs stay as a backup. Returns whether one
    /// was deleted.
    pub async fn soft_delete_vault(&self, vault: Uuid, owner: i64) -> Result<bool> {
        let updated = sqlx::query(
            "UPDATE vaults SET deleted_at = now() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
        )
        .bind(vault)
        .bind(owner)
        .execute(&self.pool)
        .await?;

        Ok(updated.rows_affected() == 1)
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

    /// The device and its owner for a live (non-revoked) token hash. A hit also
    /// stamps `last_seen_at` on both the device and its user, so the admin panel
    /// shows real activity — one extra write per authenticated request, cheap at
    /// personal scale.
    pub async fn device_for_token(&self, token_hash: &str) -> Result<Option<(i64, i64)>> {
        Ok(sqlx::query_as(
            "WITH d AS (
                 UPDATE devices SET last_seen_at = now()
                  WHERE token_hash = $1 AND revoked_at IS NULL
              RETURNING id, user_id
             ), u AS (
                 UPDATE users SET last_seen_at = now() WHERE id = (SELECT user_id FROM d)
             )
             SELECT id, user_id FROM d",
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

        // A write that grows the owner's stored content past their quota is
        // refused before anything is stored. Deletes/moves/folders carry no new
        // content and so never hit this.
        if let Some(bytes) = content {
            self.enforce_quota(vault, path, bytes.len()).await?;
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

// --- admin panel (M7): reads the whole box, mutates accounts and tokens ---

impl Store {
    // --- admin identity and sessions ---

    /// Grants admin to a user by handle, returning whether one was found.
    pub async fn grant_admin(&self, handle: &str) -> Result<bool> {
        let updated = sqlx::query("UPDATE users SET is_admin = TRUE WHERE handle = $1")
            .bind(handle)
            .execute(&self.pool)
            .await?;

        Ok(updated.rows_affected() == 1)
    }

    /// Opens an admin session for a user, storing only the token's hash. The
    /// session expires after `ttl_seconds` and carries its own CSRF token.
    pub async fn create_admin_session(
        &self,
        user_id: i64,
        token_hash: &str,
        csrf: &str,
        ttl_seconds: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO admin_sessions (user_id, token_hash, csrf, expires_at)
             VALUES ($1, $2, $3, now() + ($4 * interval '1 second'))",
        )
        .bind(user_id)
        .bind(token_hash)
        .bind(csrf)
        .bind(ttl_seconds)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// The live web session for a token hash: present only if unexpired and its
    /// user is not disabled, so a disabled user's cookie dies at once. Carries
    /// `is_admin` so the admin extractor can gate operator routes.
    pub async fn web_session(&self, token_hash: &str) -> Result<Option<WebSession>> {
        Ok(sqlx::query_as(
            "SELECT s.user_id, u.handle, s.csrf, u.is_admin
               FROM admin_sessions s JOIN users u ON u.id = s.user_id
              WHERE s.token_hash = $1 AND s.expires_at > now() AND NOT u.disabled",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Ends a session (logout), or clears one already gone.
    pub async fn delete_admin_session(&self, token_hash: &str) -> Result<()> {
        sqlx::query("DELETE FROM admin_sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// A user's id, admin flag, and disabled flag by handle, for the sign-in gate.
    pub async fn admin_login(&self, handle: &str) -> Result<Option<(i64, String, bool, bool)>> {
        Ok(sqlx::query_as(
            "SELECT id, password_hash, is_admin, disabled FROM users WHERE handle = $1",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?)
    }

    // --- overview ---

    /// The front-page tallies in one round of scalar queries.
    pub async fn overview(&self) -> Result<Overview> {
        let (users, admins): (i64, i64) =
            sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE is_admin) FROM users")
                .fetch_one(&self.pool)
                .await?;
        let (devices, live_devices): (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE revoked_at IS NULL) FROM devices",
        )
        .fetch_one(&self.pool)
        .await?;
        let vaults: i64 = sqlx::query_scalar("SELECT count(*) FROM vaults").fetch_one(&self.pool).await?;
        let (blobs, blob_size, blob_stored): (i64, i64, i64) = sqlx::query_as(
            "SELECT count(*), coalesce(sum(size), 0)::bigint, coalesce(sum(stored_size), 0)::bigint FROM blobs",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Overview {
            users,
            admins,
            devices,
            live_devices,
            vaults,
            blobs,
            blob_size,
            blob_stored,
            disk_free: disk_free_bytes(&self.blob_root),
        })
    }

    /// Recent conflict copies — the one thing sync writes into a vault that the
    /// server can see, spotted by the `(conflict …)` filename 2b writes.
    pub async fn recent_conflict_copies(&self, limit: i64) -> Result<Vec<ConflictCopy>> {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT v.name, e.path,
                    to_char((
                        SELECT max(created_at) FROM entity_versions ev
                         WHERE ev.vault_id = e.vault_id AND ev.path = e.path
                    ), 'YYYY-MM-DD HH24:MI')
               FROM entities e JOIN vaults v ON v.id = e.vault_id
              WHERE NOT e.deleted AND e.path LIKE '% (conflict %'
              ORDER BY 3 DESC NULLS LAST
              LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(vault, path, at)| ConflictCopy { vault, path, at }).collect())
    }

    // --- users ---

    pub async fn list_users(&self) -> Result<Vec<UserRow>> {
        let rows: Vec<UserRow> = sqlx::query_as(&format!("{USER_SELECT} GROUP BY u.id ORDER BY u.handle"))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn user_row(&self, id: i64) -> Result<Option<UserRow>> {
        Ok(sqlx::query_as(&format!("{USER_SELECT} WHERE u.id = $1 GROUP BY u.id"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// What deleting a user would remove, for the typed-confirmation summary.
    pub async fn delete_user_impact(&self, id: i64) -> Result<Option<DeleteImpact>> {
        let row: Option<(String, i64, i64, i64)> = sqlx::query_as(
            "SELECT u.handle,
                    count(DISTINCT v.id),
                    count(e.path) FILTER (WHERE NOT e.deleted AND e.kind = 'note'),
                    coalesce(sum(b.size) FILTER (WHERE NOT e.deleted), 0)::bigint
               FROM users u
               LEFT JOIN vaults v ON v.user_id = u.id
               LEFT JOIN entities e ON e.vault_id = v.id
               LEFT JOIN blobs b ON b.hash = e.blob
              WHERE u.id = $1
              GROUP BY u.handle",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(handle, vaults, notes, bytes)| DeleteImpact { handle, vaults, notes, bytes }))
    }

    // --- devices ---

    /// Devices, all of them or one user's (`user_filter`), most recent first.
    pub async fn list_devices(&self, user_filter: Option<i64>) -> Result<Vec<DeviceRow>> {
        let rows: Vec<DeviceRow> = sqlx::query_as(
            "SELECT d.id, u.handle, d.name, d.platform,
                    to_char(d.issued_at, 'YYYY-MM-DD') AS issued,
                    to_char(d.last_seen_at, 'YYYY-MM-DD HH24:MI') AS last_seen,
                    d.revoked_at IS NOT NULL AS revoked
               FROM devices d JOIN users u ON u.id = d.user_id
              WHERE ($1::bigint IS NULL OR d.user_id = $1)
              ORDER BY d.issued_at DESC",
        )
        .bind(user_filter)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // --- vaults ---

    /// Vaults, all or one user's, with note count, bytes, and last-sync time —
    /// names and sizes, never contents.
    pub async fn list_vaults_admin(&self, user_filter: Option<i64>) -> Result<Vec<VaultRow>> {
        let rows: Vec<VaultRow> = sqlx::query_as(
            "SELECT v.id, v.name, u.handle, v.deleted_at IS NOT NULL AS deleted,
                    count(e.path) FILTER (WHERE NOT e.deleted AND e.kind = 'note') AS notes,
                    coalesce(sum(b.size) FILTER (WHERE NOT e.deleted), 0)::bigint AS bytes,
                    to_char((
                        SELECT max(created_at) FROM entity_versions ev WHERE ev.vault_id = v.id
                    ), 'YYYY-MM-DD HH24:MI') AS last_sync
               FROM vaults v
               JOIN users u ON u.id = v.user_id
               LEFT JOIN entities e ON e.vault_id = v.id
               LEFT JOIN blobs b ON b.hash = e.blob
              WHERE ($1::bigint IS NULL OR v.user_id = $1)
              GROUP BY v.id, u.handle
              ORDER BY u.handle, v.name",
        )
        .bind(user_filter)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // --- audit log ---

    /// Appends one row to the operations log. Every mutation and sign-in calls it.
    pub async fn log_event(&self, actor: &str, action: &str, detail: &str) -> Result<()> {
        sqlx::query("INSERT INTO audit_log (actor, action, detail) VALUES ($1, $2, $3)")
            .bind(actor)
            .bind(action)
            .bind(detail)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn recent_audit(&self, limit: i64) -> Result<Vec<AuditRow>> {
        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT to_char(at, 'YYYY-MM-DD HH24:MI:SS') AS at, actor, action, detail
               FROM audit_log ORDER BY at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // --- account actions ---

    /// Disables a user and revokes their live device tokens in one transaction —
    /// keeps the files, kills the tokens.
    pub async fn disable_user(&self, id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("UPDATE users SET disabled = TRUE WHERE id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE devices SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1").bind(id).execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }

    /// Deletes a user and all their sync state in one transaction. Blobs are
    /// content-addressed and shared across users, so they are left for the
    /// deferred orphan sweep rather than removed here.
    pub async fn delete_user(&self, id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let vaults = "SELECT id FROM vaults WHERE user_id = $1";
        sqlx::query(&format!("DELETE FROM entity_versions WHERE vault_id IN ({vaults})"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(&format!("DELETE FROM entities WHERE vault_id IN ({vaults})"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM vaults WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM devices WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM users WHERE id = $1").bind(id).execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }

    /// Revokes a device token. Returns the owner's handle for the log, or None if
    /// the device was already revoked or gone.
    pub async fn revoke_device(&self, id: i64) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "UPDATE devices d SET revoked_at = now()
               FROM users u
              WHERE d.id = $1 AND d.user_id = u.id AND d.revoked_at IS NULL
          RETURNING u.handle",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    // --- admin operational edits/deletes (M7 "edit/delete everything") ---

    pub async fn rename_user(&self, id: i64, handle: &str) -> Result<()> {
        sqlx::query("UPDATE users SET handle = $2 WHERE id = $1").bind(id).bind(handle).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_admin(&self, id: i64, is_admin: bool) -> Result<()> {
        sqlx::query("UPDATE users SET is_admin = $2 WHERE id = $1")
            .bind(id)
            .bind(is_admin)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Re-enables a disabled user (the inverse of `disable_user`). Their revoked
    /// device tokens stay revoked — they sign in fresh.
    pub async fn enable_user(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE users SET disabled = FALSE WHERE id = $1").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Hard-deletes a device row, first detaching it from any history rows.
    pub async fn delete_device(&self, id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE entity_versions SET device_id = NULL WHERE device_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM devices WHERE id = $1").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// A vault's name and owner handle, for confirmations and the log.
    pub async fn vault_label(&self, vault: Uuid) -> Result<Option<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT v.name, u.handle FROM vaults v JOIN users u ON u.id = v.user_id WHERE v.id = $1",
        )
        .bind(vault)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn rename_vault(&self, vault: Uuid, name: &str) -> Result<()> {
        sqlx::query("UPDATE vaults SET name = $2 WHERE id = $1").bind(vault).bind(name).execute(&self.pool).await?;
        Ok(())
    }

    /// Soft-deletes any vault (admin, no owner check) — recoverable via
    /// [`restore_vault`](Self::restore_vault).
    pub async fn admin_soft_delete_vault(&self, vault: Uuid) -> Result<()> {
        sqlx::query("UPDATE vaults SET deleted_at = now() WHERE id = $1").bind(vault).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn restore_vault(&self, vault: Uuid) -> Result<()> {
        sqlx::query("UPDATE vaults SET deleted_at = NULL WHERE id = $1").bind(vault).execute(&self.pool).await?;
        Ok(())
    }

    /// Permanently removes a vault and all its sync rows. Blobs are
    /// content-addressed and shared, so they are left for the orphan sweep.
    pub async fn purge_vault(&self, vault: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM entity_versions WHERE vault_id = $1").bind(vault).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM entities WHERE vault_id = $1").bind(vault).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM vaults WHERE id = $1").bind(vault).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

// --- quotas, billing, and email-driven account flows (M7 deferred features) ---

impl Store {
    // --- quota ---

    /// Refuses a write that would push the vault owner's stored content past their
    /// quota. Usage is the sum of current blob sizes across the owner's vaults,
    /// with the blob being replaced at this path discounted. No effective quota
    /// (a plan with no match, and no override) means unlimited.
    async fn enforce_quota(&self, vault: Uuid, path: &str, new_size: usize) -> Result<()> {
        let Some(owner) = self.vault_owner(vault).await? else {
            return Ok(());
        };
        let Some(limit) = self.user_effective_quota(owner).await? else {
            return Ok(());
        };

        let used = self.user_usage_bytes(owner).await?;
        let existing = self.path_blob_size(vault, path).await?;
        let projected = used - existing + new_size as i64;

        if projected > limit {
            return Err(Error::QuotaExceeded { used: projected.max(0) as u64, limit: limit.max(0) as u64 });
        }

        Ok(())
    }

    /// The quota that applies to a user: their explicit override, else their
    /// plan's quota, else `None` (unlimited — the plan was deleted or unknown).
    pub async fn user_effective_quota(&self, user_id: i64) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT coalesce(u.quota_bytes, p.quota_bytes)
               FROM users u LEFT JOIN plans p ON p.name = u.plan
              WHERE u.id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?)
    }

    // --- plans (operator-managed) ---

    /// Every plan with a count of the users on it, for the Plans page and the
    /// public pricing page, cheapest first.
    pub async fn list_plans(&self) -> Result<Vec<PlanRow>> {
        Ok(sqlx::query_as(
            "SELECT p.name, p.quota_bytes, p.stripe_price_id, p.price_cents, p.description,
                    count(u.id) AS users
               FROM plans p LEFT JOIN users u ON u.plan = p.name
              GROUP BY p.name ORDER BY p.quota_bytes",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_plan(&self, plan: NewPlan<'_>) -> Result<()> {
        sqlx::query(
            "INSERT INTO plans (name, quota_bytes, stripe_price_id, price_cents, description)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(plan.name)
        .bind(plan.quota_bytes)
        .bind(plan.stripe_price_id)
        .bind(plan.price_cents)
        .bind(plan.description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_plan(&self, name: &str, plan: NewPlan<'_>) -> Result<()> {
        sqlx::query(
            "UPDATE plans SET quota_bytes = $2, stripe_price_id = $3, price_cents = $4, description = $5
              WHERE name = $1",
        )
        .bind(name)
        .bind(plan.quota_bytes)
        .bind(plan.stripe_price_id)
        .bind(plan.price_cents)
        .bind(plan.description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_plan(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM plans WHERE name = $1").bind(name).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn count_users_on_plan(&self, name: &str) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM users WHERE plan = $1").bind(name).fetch_one(&self.pool).await?)
    }

    /// The Stripe price id configured for a plan, if it is sold via Stripe.
    pub async fn plan_price(&self, name: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT stripe_price_id FROM plans WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
            .flatten())
    }

    /// The sum of current (non-deleted) blob sizes across a user's vaults.
    pub async fn user_usage_bytes(&self, user_id: i64) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT coalesce(sum(b.size), 0)::bigint
               FROM vaults v
               JOIN entities e ON e.vault_id = v.id AND NOT e.deleted AND e.blob IS NOT NULL
               JOIN blobs b ON b.hash = e.blob
              WHERE v.user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn path_blob_size(&self, vault: Uuid, path: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT coalesce(b.size, 0)::bigint
               FROM entities e LEFT JOIN blobs b ON b.hash = e.blob
              WHERE e.vault_id = $1 AND e.path = $2 AND NOT e.deleted",
        )
        .bind(vault)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0))
    }

    pub async fn set_quota(&self, user_id: i64, quota_bytes: Option<i64>) -> Result<()> {
        sqlx::query("UPDATE users SET quota_bytes = $2 WHERE id = $1")
            .bind(user_id)
            .bind(quota_bytes)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_plan(&self, user_id: i64, plan: &str) -> Result<()> {
        sqlx::query("UPDATE users SET plan = $2 WHERE id = $1")
            .bind(user_id)
            .bind(plan)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A user's billing state, for the detail page and portal action.
    pub async fn user_billing(&self, user_id: i64) -> Result<Option<Billing>> {
        Ok(sqlx::query_as(
            "SELECT plan, quota_bytes,
                    subscription_status AS status, stripe_customer_id AS customer, email
               FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    // --- account email + password ---

    pub async fn set_email(&self, user_id: i64, email: &str) -> Result<()> {
        sqlx::query("UPDATE users SET email = $2 WHERE id = $1")
            .bind(user_id)
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn find_user_by_email(&self, email: &str) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT id FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Sets a user's password and severs their live credentials — every device
    /// token and admin session — so a reset actually locks out whoever held them.
    pub async fn set_password(&self, user_id: i64, password_hash: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(user_id)
            .bind(password_hash)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE devices SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }

    // --- password reset tokens (email flow) ---

    pub async fn create_password_reset(&self, user_id: i64, token_hash: &str, ttl_seconds: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO password_resets (user_id, token_hash, expires_at)
             VALUES ($1, $2, now() + ($3 * interval '1 second'))",
        )
        .bind(user_id)
        .bind(token_hash)
        .bind(ttl_seconds)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Redeems a reset token if live and unused, marking it used, and returns the
    /// user it belongs to.
    pub async fn consume_password_reset(&self, token_hash: &str) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar(
            "UPDATE password_resets SET used = TRUE
              WHERE token_hash = $1 AND NOT used AND expires_at > now()
          RETURNING user_id",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    // --- invites (email flow) ---

    pub async fn create_invite(
        &self,
        email: &str,
        invited_by: i64,
        token_hash: &str,
        ttl_seconds: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO invites (email, invited_by, token_hash, expires_at)
             VALUES ($1, $2, $3, now() + ($4 * interval '1 second'))",
        )
        .bind(email)
        .bind(invited_by)
        .bind(token_hash)
        .bind(ttl_seconds)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Redeems an invite token if live and unaccepted, marking it accepted, and
    /// returns the address it was issued to.
    pub async fn consume_invite(&self, token_hash: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "UPDATE invites SET accepted = TRUE
              WHERE token_hash = $1 AND NOT accepted AND expires_at > now()
          RETURNING email",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    // --- subscription (Stripe webhook) ---

    /// Binds a completed checkout to a user: records the Stripe customer and sets
    /// the plan active.
    pub async fn bind_subscription(&self, user_id: i64, customer: &str, plan: &str) -> Result<()> {
        sqlx::query(
            "UPDATE users SET stripe_customer_id = $2, plan = $3, subscription_status = 'active' WHERE id = $1",
        )
        .bind(user_id)
        .bind(customer)
        .bind(plan)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Updates only the subscription status (a `customer.subscription.updated`
    /// event), keyed by Stripe customer id. Returns the user's handle for the log.
    pub async fn set_subscription_status(&self, customer: &str, status: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "UPDATE users SET subscription_status = $2 WHERE stripe_customer_id = $1 RETURNING handle",
        )
        .bind(customer)
        .bind(status)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Applies a subscription lifecycle change keyed by Stripe customer id. A
    /// canceled subscription drops the user back to the free plan. Returns the
    /// affected user's handle for the log, if any.
    pub async fn set_subscription_by_customer(
        &self,
        customer: &str,
        plan: &str,
        status: &str,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "UPDATE users SET plan = $2, subscription_status = $3
               WHERE stripe_customer_id = $1
           RETURNING handle",
        )
        .bind(customer)
        .bind(plan)
        .bind(status)
        .fetch_optional(&self.pool)
        .await?)
    }

    // --- chart series (overview) ---

    /// Cumulative on-disk bytes by day — the storage-growth chart.
    pub async fn storage_growth(&self) -> Result<Vec<Point>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT to_char(day, 'MM-DD') AS label,
                    sum(sum(stored_size)) OVER (ORDER BY day)::bigint AS total
               FROM (SELECT date_trunc('day', created_at) AS day, stored_size FROM blobs) b
              GROUP BY day ORDER BY day",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(label, value)| Point { label, value }).collect())
    }

    /// Sign-ins per day over the last 30 days — the activity chart.
    pub async fn signins_by_day(&self) -> Result<Vec<Point>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT to_char(date_trunc('day', at), 'MM-DD') AS label, count(*)::bigint AS total
               FROM audit_log
              WHERE action = 'sign-in' AND at > now() - interval '30 days'
              GROUP BY 1 ORDER BY 1",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(label, value)| Point { label, value }).collect())
    }
}

/// A user's billing state for the panel.
#[derive(sqlx::FromRow)]
pub struct Billing {
    pub plan: String,
    pub quota_bytes: Option<i64>,
    pub status: Option<String>,
    pub customer: Option<String>,
    pub email: Option<String>,
}

/// One labelled data point for a chart.
pub struct Point {
    pub label: String,
    pub value: i64,
}

/// A plan row for the Plans page and the pricing page, with a count of the users
/// on it.
#[derive(sqlx::FromRow)]
pub struct PlanRow {
    pub name: String,
    pub quota_bytes: i64,
    pub stripe_price_id: Option<String>,
    pub price_cents: Option<i32>,
    pub description: Option<String>,
    pub users: i64,
}

/// The fields of a plan an operator sets when creating or editing one.
pub struct NewPlan<'a> {
    pub name: &'a str,
    pub quota_bytes: i64,
    pub stripe_price_id: Option<&'a str>,
    pub price_cents: Option<i32>,
    pub description: Option<&'a str>,
}

/// The nine columns every user row carries; `list_users` and `user_row` share it.
const USER_SELECT: &str = "SELECT u.id, u.handle, u.is_admin, u.disabled, u.plan,
        to_char(u.created_at, 'YYYY-MM-DD') AS created,
        to_char(u.last_seen_at, 'YYYY-MM-DD HH24:MI') AS last_seen,
        count(DISTINCT v.id) AS vaults,
        coalesce(sum(b.size) FILTER (WHERE NOT e.deleted), 0)::bigint AS bytes
   FROM users u
   LEFT JOIN vaults v ON v.user_id = u.id
   LEFT JOIN entities e ON e.vault_id = v.id
   LEFT JOIN blobs b ON b.hash = e.blob";

/// Free bytes on the filesystem holding the blob directory, 0 if it cannot be
/// read (a fresh box before the first blob).
#[allow(clippy::unnecessary_cast)] // libc statvfs field widths vary by platform.
fn disk_free_bytes(root: &std::path::Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;

    let Ok(path) = std::ffi::CString::new(root.as_os_str().as_bytes()) else {
        return 0;
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return 0;
    }

    stat.f_bavail as u64 * stat.f_frsize as u64
}

/// A live web session, resolved from the request cookie. `is_admin` decides
/// whether the operator area is reachable.
#[derive(sqlx::FromRow, Clone)]
pub struct WebSession {
    pub user_id: i64,
    pub handle: String,
    pub csrf: String,
    pub is_admin: bool,
}

/// A live session known to belong to an admin, produced by the admin extractor.
/// The operator handlers take this so their access is typed, not conventional.
pub struct AdminSession {
    pub user_id: i64,
    pub handle: String,
    pub csrf: String,
}

/// The Overview page's tallies.
pub struct Overview {
    pub users: i64,
    pub admins: i64,
    pub devices: i64,
    pub live_devices: i64,
    pub vaults: i64,
    pub blobs: i64,
    pub blob_size: i64,
    pub blob_stored: i64,
    pub disk_free: u64,
}

pub struct ConflictCopy {
    pub vault: String,
    pub path: String,
    pub at: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct UserRow {
    pub id: i64,
    pub handle: String,
    pub is_admin: bool,
    pub disabled: bool,
    pub plan: String,
    pub created: String,
    pub last_seen: Option<String>,
    pub vaults: i64,
    pub bytes: i64,
}

#[derive(sqlx::FromRow)]
pub struct DeviceRow {
    pub id: i64,
    pub handle: String,
    pub name: String,
    pub platform: String,
    pub issued: String,
    pub last_seen: Option<String>,
    pub revoked: bool,
}

#[derive(sqlx::FromRow)]
pub struct VaultRow {
    pub id: Uuid,
    pub name: String,
    pub handle: String,
    pub deleted: bool,
    pub notes: i64,
    pub bytes: i64,
    pub last_sync: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct AuditRow {
    pub at: String,
    pub actor: String,
    pub action: String,
    pub detail: String,
}

pub struct DeleteImpact {
    pub handle: String,
    pub vaults: i64,
    pub notes: i64,
    pub bytes: i64,
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
    /// A write would push the owner's stored content past their quota — surfaced
    /// as a 507.
    QuotaExceeded { used: u64, limit: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(error) => write!(f, "database error: {error}"),
            Error::Migrate(error) => write!(f, "migration error: {error}"),
            Error::Blob(error) => write!(f, "blob store error: {error}"),
            Error::MissingBlob(hash) => write!(f, "referenced blob {hash} was not uploaded"),
            Error::QuotaExceeded { used, limit } => {
                write!(f, "quota exceeded: {used} bytes would exceed the {limit}-byte limit")
            }
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
    async fn deleting_a_user_removes_their_state_but_keeps_blobs(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        store.apply_put(vault, "Note.md", note(), 0, Some(b"a note worth a blob"), None).await.unwrap();
        let user: i64 = sqlx::query_scalar("SELECT user_id FROM vaults WHERE id = $1")
            .bind(vault)
            .fetch_one(&store.pool)
            .await
            .unwrap();

        store.delete_user(user).await.unwrap();

        // The account and every trace of its sync state are gone.
        assert!(store.user_row(user).await.unwrap().is_none());
        let vaults: i64 = sqlx::query_scalar("SELECT count(*) FROM vaults WHERE user_id = $1")
            .bind(user)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(vaults, 0);
        let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM entity_versions WHERE vault_id = $1")
            .bind(vault)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(versions, 0);

        // Blobs are content-addressed and shared, so they outlive the user and
        // wait for the orphan sweep.
        let blobs: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs").fetch_one(&store.pool).await.unwrap();
        assert_eq!(blobs, 1);
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

    async fn owner_of(store: &Store, vault: Uuid) -> i64 {
        sqlx::query_scalar("SELECT user_id FROM vaults WHERE id = $1").bind(vault).fetch_one(&store.pool).await.unwrap()
    }

    #[sqlx::test]
    async fn a_write_past_the_quota_is_refused(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let owner = owner_of(&store, vault).await;
        store.set_quota(owner, Some(10)).await.unwrap();

        let outcome = store.apply_put(vault, "Note.md", note(), 0, Some(b"far more than ten bytes of prose"), None).await;
        assert!(matches!(outcome, Err(Error::QuotaExceeded { .. })));

        // Raising the quota lets the same write through, and usage reflects it.
        store.set_quota(owner, Some(10_000)).await.unwrap();
        store.apply_put(vault, "Note.md", note(), 0, Some(b"now within quota"), None).await.unwrap();
        assert_eq!(store.user_usage_bytes(owner).await.unwrap(), 16);
    }

    #[sqlx::test]
    async fn a_reset_token_is_single_use_and_expires(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let user = owner_of(&store, vault).await;

        store.create_password_reset(user, "live", 3600).await.unwrap();
        assert_eq!(store.consume_password_reset("live").await.unwrap(), Some(user));
        // A second use, and an already-expired token, both fail.
        assert_eq!(store.consume_password_reset("live").await.unwrap(), None);
        store.create_password_reset(user, "stale", -1).await.unwrap();
        assert_eq!(store.consume_password_reset("stale").await.unwrap(), None);
    }

    #[sqlx::test]
    async fn an_invite_is_single_use_and_yields_its_email(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let inviter = owner_of(&store, vault).await;

        store.create_invite("bob@example.com", inviter, "tok", 3600).await.unwrap();
        assert_eq!(store.consume_invite("tok").await.unwrap().as_deref(), Some("bob@example.com"));
        assert!(store.consume_invite("tok").await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn a_subscription_binds_updates_and_cancels(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let user = owner_of(&store, vault).await;

        store.bind_subscription(user, "cus_1", "pro").await.unwrap();
        let billing = store.user_billing(user).await.unwrap().unwrap();
        assert_eq!(billing.plan, "pro");
        assert_eq!(billing.status.as_deref(), Some("active"));
        assert_eq!(billing.customer.as_deref(), Some("cus_1"));

        assert_eq!(store.set_subscription_status("cus_1", "past_due").await.unwrap().as_deref(), Some("alice"));
        assert_eq!(store.set_subscription_by_customer("cus_1", "free", "canceled").await.unwrap().as_deref(), Some("alice"));
        assert_eq!(store.user_billing(user).await.unwrap().unwrap().plan, "free");
    }

    #[sqlx::test]
    async fn setting_a_password_revokes_the_user_s_tokens(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let user = owner_of(&store, vault).await;
        store.create_device(user, "laptop", "macos", "token-hash").await.unwrap();

        store.set_password(user, "new-argon2-hash").await.unwrap();

        assert!(store.device_for_token("token-hash").await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn storage_growth_has_a_point_after_an_upload(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        store.apply_put(vault, "Note.md", note(), 0, Some(b"content worth charting"), None).await.unwrap();

        assert!(!store.storage_growth().await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn admin_edits_and_deletes_operational_entities(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let user = owner_of(&store, vault).await;
        store.apply_put(vault, "Note.md", note(), 0, Some(b"a note worth a blob"), None).await.unwrap();

        // User edits.
        store.rename_user(user, "alice2").await.unwrap();
        assert_eq!(store.user_row(user).await.unwrap().unwrap().handle, "alice2");
        store.set_admin(user, true).await.unwrap();
        assert!(store.user_row(user).await.unwrap().unwrap().is_admin);
        store.disable_user(user).await.unwrap();
        assert!(store.user_row(user).await.unwrap().unwrap().disabled);
        store.enable_user(user).await.unwrap();
        assert!(!store.user_row(user).await.unwrap().unwrap().disabled);

        // Vault: soft-delete hides it, restore brings it back, purge removes it.
        store.admin_soft_delete_vault(vault).await.unwrap();
        assert!(store.list_vaults(user).await.unwrap().is_empty());
        store.restore_vault(vault).await.unwrap();
        assert_eq!(store.list_vaults(user).await.unwrap().len(), 1);

        store.purge_vault(vault).await.unwrap();
        let entities: i64 = sqlx::query_scalar("SELECT count(*) FROM entities WHERE vault_id = $1")
            .bind(vault)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(entities, 0);
        // The blob outlives the purge (content-addressed, left for the orphan sweep).
        let blobs: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs").fetch_one(&store.pool).await.unwrap();
        assert_eq!(blobs, 1);
    }

    #[sqlx::test]
    async fn a_custom_plan_drives_a_user_s_quota(pool: PgPool) {
        let (store, vault) = seed(pool).await;
        let user = owner_of(&store, vault).await;

        // A seeded plan resolves the quota; a per-user override still wins.
        assert_eq!(store.user_effective_quota(user).await.unwrap(), Some(1024 * 1024 * 1024)); // free = 1 GiB

        store
            .create_plan(NewPlan {
                name: "team",
                quota_bytes: 5_000,
                stripe_price_id: Some("price_123"),
                price_cents: Some(500),
                description: None,
            })
            .await
            .unwrap();
        store.set_plan(user, "team").await.unwrap();
        assert_eq!(store.user_effective_quota(user).await.unwrap(), Some(5_000));
        assert_eq!(store.plan_price("team").await.unwrap().as_deref(), Some("price_123"));

        store.set_quota(user, Some(42)).await.unwrap();
        assert_eq!(store.user_effective_quota(user).await.unwrap(), Some(42));

        // The plan is in use, so the panel's delete guard would refuse it.
        assert_eq!(store.count_users_on_plan("team").await.unwrap(), 1);
    }
}
