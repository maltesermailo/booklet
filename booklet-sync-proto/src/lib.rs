//! The sync wire contract, shared by `booklet-sync-server` and the client engine
//! so the two cannot drift. Nothing but `serde` types — no logic, no I/O.
//!
//! It is a crate of its own, not part of `booklet-core`, so the server does not
//! inherit core's `pulldown-cmark` and `trash` dependencies (`trash` wants a
//! desktop session a headless box has not got). See `design/sync-server.md`.
//!
//! Everything the client sends or receives is addressed by **content hash**
//! (the SHA-256 `booklet-core` already computes in `sync.rs`); the server's
//! delta-chained blob storage is invisible here. IDs that are UUIDs on the
//! server travel as plain strings, so this crate stays `serde`-only.

use serde::{Deserialize, Serialize};

/// Which kind of tracked entity a change concerns. The same variants as
/// `booklet_core::sync::EntryKind`, duplicated on purpose: the two crates must
/// not depend on each other, and the client converts across the seam in one
/// `match`. Serializes lowercase (`"note"` / `"bookmeta"` / `"folder"` /
/// `"image"`).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Note,
    BookMeta,
    Folder,
    Image,
}

// --- auth ---

/// Credentials traded for a device token. A device names itself so a person can
/// tell their laptop from their phone when revoking one (M7).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TokenRequest {
    pub handle: String,
    pub password: String,
    pub device_name: String,
    pub platform: String,
}

/// The issued token, shown once. `user` echoes the handle it belongs to.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TokenResponse {
    pub token: String,
    pub user: String,
}

// --- vaults ---

/// A vault the authenticated user owns, for the clone/publish choice. `seq` is
/// the vault's current sequence — a client with a lower cursor is behind.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VaultSummary {
    pub id: String,
    pub name: String,
    pub seq: u64,
}

/// Publish a local vault into a fresh server slot.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PublishRequest {
    pub name: String,
}

/// The server-assigned vault id, which the client stores in
/// `.booklet/sync.json` as `vault_id`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PublishResponse {
    pub id: String,
}

// --- change feed ---

/// One entity's current state in the feed. `blob` is the content hash, absent
/// for folders and for deletes; `moved_from` is set when this landed as a move,
/// so the receiver can treat it as one rather than a delete-plus-create.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Change {
    pub path: String,
    pub kind: EntityKind,
    pub version: u64,
    pub seq: u64,
    pub deleted: bool,
    pub blob: Option<String>,
    pub moved_from: Option<String>,
}

/// The `GET /changes?since=N` response: every path with `seq > N`, plus the new
/// cursor to store and pass as `since` next time.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Changes {
    pub changes: Vec<Change>,
    pub cursor: u64,
}

// --- upload ---

/// A create, modify, or move of one entity. `base_version` is the version the
/// client last saw (0 means it believes it is creating); a mismatch is a 409.
/// `blob` is a content hash already uploaded via `PUT /blobs`, absent for
/// folders. `moved_from` collapses a move into a single mutation.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PutRequest {
    pub kind: EntityKind,
    pub base_version: u64,
    pub blob: Option<String>,
    pub moved_from: Option<String>,
}

/// A delete of one entity. `base_version` guards it the same way a `PutRequest`
/// does — deleting a version you have not seen is a 409.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DeleteRequest {
    pub base_version: u64,
}

/// The server-assigned version and sequence after a successful mutation.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PutResponse {
    pub version: u64,
    pub seq: u64,
}

/// The body of a 409: the server's current version and content hash for the
/// path, enough for the client to fetch that base and merge (2b) before
/// retrying. `base_version = 0` against an existing path is the no-ancestor
/// case, where the client writes a conflict copy instead.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Conflict {
    pub current_version: u64,
    pub current_blob: Option<String>,
}

// --- history (for the 2e version modal) ---

/// One entry in a note's version history. `blob` is absent for a tombstone or a
/// folder; `created_at` is epoch milliseconds, shown to the user but never used
/// to order — ordering is by `seq`/`version`, never by a clock.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Version {
    pub version: u64,
    pub seq: u64,
    pub blob: Option<String>,
    pub deleted: bool,
    pub device: String,
    pub created_at: i64,
}

/// A path's full version list, newest last.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct History {
    pub versions: Vec<Version>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind must ride the wire as the lowercase strings the server's `kind`
    /// column uses, so a round-trip through JSON is stable across the seam.
    #[test]
    fn entity_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&EntityKind::BookMeta).unwrap(), "\"bookmeta\"");
        assert_eq!(serde_json::to_string(&EntityKind::Image).unwrap(), "\"image\"");
        assert_eq!(
            serde_json::from_str::<EntityKind>("\"folder\"").unwrap(),
            EntityKind::Folder
        );
        assert_eq!(
            serde_json::from_str::<EntityKind>("\"image\"").unwrap(),
            EntityKind::Image
        );
    }

    /// A folder change and a delete both omit the blob; the contract must carry
    /// that faithfully rather than inventing a hash.
    #[test]
    fn a_change_round_trips_through_json() {
        let change = Change {
            path: "Book/Section/Note.md".into(),
            kind: EntityKind::Note,
            version: 7,
            seq: 42,
            deleted: false,
            blob: Some("abc123".into()),
            moved_from: Some("Book/Old.md".into()),
        };

        let json = serde_json::to_string(&change).unwrap();
        assert_eq!(serde_json::from_str::<Change>(&json).unwrap(), change);
    }
}
