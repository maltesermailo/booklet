//! The reconciliation engine: push local changes, pull remote ones, and merge
//! where they collide.
//!
//! It leans entirely on `booklet-core`: [`Manifest`](booklet_core::sync::Manifest)
//! (2a) tells it what changed locally, and [`merge`](booklet_core::merge) (2b)
//! resolves a collision. The conflict model is push-driven, exactly as the
//! roadmap sets out: a stale write 409s and the *loser* reconciles — merging a
//! note against the ancestor it last synced, or, when there is no ancestor (two
//! devices created the same filename), writing a conflict copy.
//!
//! The per-device sync state — how far the feed has been read, and the server
//! version last confirmed for each path — lives in [`ClientState`]. `pull`
//! overwrites local files with remote content, so a caller should `push` local
//! edits (which merge on 409) before it pulls.

use crate::client::{Client, ClientError, PutResult};
use booklet_core::merge;
use booklet_core::sync::{Change, EntryKind, Manifest, SyncState};
use booklet_sync_proto as proto;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

/// One device's sync state against one vault.
#[derive(Default)]
pub struct ClientState {
    /// The feed sequence read so far (the next `since`).
    pub cursor: u64,
    /// The server version last confirmed for each path — the `base_version` a
    /// push sends, and the ancestor a merge fetches.
    pub versions: HashMap<String, u64>,
}

impl ClientState {
    /// Loads the cursor and per-path versions from `.booklet/sync.json`. A vault
    /// not yet synced reads as the default (cursor 0, no versions).
    pub fn load(vault_root: &Path) -> io::Result<ClientState> {
        let state = SyncState::load(vault_root)?;
        Ok(ClientState { cursor: state.cursor, versions: state.versions })
    }

    /// Persists the cursor and versions, preserving the vault's server binding
    /// (`vault_id` / `server_url`) that lives in the same file.
    pub fn save(&self, vault_root: &Path) -> io::Result<()> {
        let mut state = SyncState::load(vault_root)?;
        state.cursor = self.cursor;
        state.versions = self.versions.clone();
        state.save(vault_root)
    }
}

/// What a push had to reconcile, for the UI (2e) to surface.
#[derive(Default)]
pub struct PushOutcome {
    /// Notes whose merge was partial and need review.
    pub flagged: Vec<String>,
    /// Paths written as conflict copies (no common ancestor).
    pub conflict_copies: Vec<String>,
    /// Every local path the reconcile rewrote on disk — so the editor can reload
    /// the open note if it is among them.
    pub changed: Vec<String>,
}

/// Pushes every local change since `manifest`, reconciling any 409, then updates
/// `manifest` to the resulting on-disk state. `today` dates a conflict copy (kept
/// out of core, which stays time-free).
pub fn push(
    client: &Client,
    vault: &str,
    root: &Path,
    manifest: &mut Manifest,
    state: &mut ClientState,
    today: &str,
) -> Result<PushOutcome, ClientError> {
    let fresh = Manifest::scan(root, manifest);
    let mut outcome = PushOutcome::default();

    for change in Manifest::diff(manifest, &fresh) {
        match change {
            Change::Created { path, kind } | Change::Modified { path, kind } => {
                push_entity(client, vault, root, &path, kind, state, today, &mut outcome)?;
            }
            Change::Deleted { path, .. } => {
                let base = version_of(state, &path);
                if let PutResult::Applied(response) = client.delete_entity(vault, &path, base)? {
                    state.versions.insert(path, response.version);
                }
            }
            Change::Moved { from, to } => {
                push_move(client, vault, root, &from, &to, state, today, &mut outcome)?;
            }
        }
    }

    *manifest = Manifest::scan(root, manifest);
    Ok(outcome)
}

/// Applies every remote change since `state.cursor` to local files, returning the
/// paths it touched (so the editor can reload the open note), then updates
/// `manifest`. Deletes remove the local file (a production client trashes it).
pub fn pull(
    client: &Client,
    vault: &str,
    root: &Path,
    manifest: &mut Manifest,
    state: &mut ClientState,
) -> Result<Vec<String>, ClientError> {
    let changes = client.changes(vault, state.cursor)?;
    let mut touched = Vec::new();

    for change in &changes.changes {
        let local = root.join(&change.path);

        if change.deleted {
            // A folder only removes when empty on this side (its notes may be
            // ones this device never saw); a file just goes.
            if change.kind == proto::EntityKind::Folder {
                let _ = fs::remove_dir(&local);
            } else {
                let _ = fs::remove_file(&local);
            }
        } else if change.kind == proto::EntityKind::Folder {
            fs::create_dir_all(&local)?;
        } else if let Some(hash) = &change.blob {
            if let Some(parent) = local.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = client.get_blob(hash)?;
            fs::write(&local, content)?;
        }

        touched.push(change.path.clone());
        state.versions.insert(change.path.clone(), change.version);
    }

    state.cursor = changes.cursor;
    *manifest = Manifest::scan(root, manifest);
    Ok(touched)
}

#[allow(clippy::too_many_arguments)]
fn push_entity(
    client: &Client,
    vault: &str,
    root: &Path,
    path: &str,
    kind: EntryKind,
    state: &mut ClientState,
    today: &str,
    outcome: &mut PushOutcome,
) -> Result<(), ClientError> {
    // A folder is an entity with no content — pushing it only asserts it exists.
    // Reading its path as a file would fail with "is a directory".
    if kind == EntryKind::Folder {
        let request = proto::PutRequest {
            kind: to_proto(kind),
            base_version: version_of(state, path),
            blob: None,
            moved_from: None,
        };
        match client.put_entity(vault, path, &request)? {
            PutResult::Applied(response) => {
                state.versions.insert(path.to_string(), response.version);
            }
            // Nothing to merge for a folder; adopt the server's version and move on.
            PutResult::Conflict(conflict) => {
                state.versions.insert(path.to_string(), conflict.current_version);
            }
        }
        return Ok(());
    }

    let content = fs::read(root.join(path))?;
    let base = version_of(state, path);
    let hash = client.put_blob(&content)?;

    let request = proto::PutRequest {
        kind: to_proto(kind),
        base_version: base,
        blob: Some(hash),
        moved_from: None,
    };

    match client.put_entity(vault, path, &request)? {
        PutResult::Applied(response) => {
            state.versions.insert(path.to_string(), response.version);
        }
        PutResult::Conflict(conflict) => {
            reconcile(client, vault, root, path, kind, base, conflict, state, today, outcome)?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reconcile(
    client: &Client,
    vault: &str,
    root: &Path,
    path: &str,
    kind: EntryKind,
    base: u64,
    conflict: proto::Conflict,
    state: &mut ClientState,
    today: &str,
    outcome: &mut PushOutcome,
) -> Result<(), ClientError> {
    let local = fs::read(root.join(path))?;
    let remote = match &conflict.current_blob {
        Some(hash) => client.get_blob(hash)?,
        None => Vec::new(),
    };

    // base == 0 means we never synced this path: two devices created the same
    // name, with no ancestor to merge from.
    if base == 0 {
        return conflict_copy(client, vault, root, path, &local, &remote, conflict.current_version, state, today, outcome);
    }

    let ancestor = fetch_ancestor(client, vault, path, base)?;
    let merged = if kind == EntryKind::BookMeta {
        merge::merge_booklet_json(&text(&local), &text(&remote)).into_bytes()
    } else {
        let markdown = merge::merge_markdown(&text(&ancestor), &text(&local), &text(&remote))
            .map_err(ClientError::Merge)?;
        if !markdown.clean {
            outcome.flagged.push(path.to_string());
        }
        markdown.text.into_bytes()
    };

    fs::write(root.join(path), &merged)?;
    outcome.changed.push(path.to_string());

    // Re-push the merged result against the version we just lost to.
    let hash = client.put_blob(&merged)?;
    let request = proto::PutRequest {
        kind: to_proto(kind),
        base_version: conflict.current_version,
        blob: Some(hash),
        moved_from: None,
    };
    if let PutResult::Applied(response) = client.put_entity(vault, path, &request)? {
        state.versions.insert(path.to_string(), response.version);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn conflict_copy(
    client: &Client,
    vault: &str,
    root: &Path,
    path: &str,
    local: &[u8],
    remote: &[u8],
    current_version: u64,
    state: &mut ClientState,
    today: &str,
    outcome: &mut PushOutcome,
) -> Result<(), ClientError> {
    let (dir, stem) = split(path);
    let copy_name = merge::conflict_copy_name(&stem, today, &dir_names(root, &dir));
    let copy_path = if dir.is_empty() { copy_name } else { format!("{dir}/{copy_name}") };

    // Our losing text becomes the copy; the server's version keeps the name.
    fs::write(root.join(&copy_path), local)?;
    fs::write(root.join(path), remote)?;
    outcome.changed.push(path.to_string());
    outcome.changed.push(copy_path.clone());
    state.versions.insert(path.to_string(), current_version);

    // The copy has a fresh, unique name, so it applies without a conflict.
    let hash = client.put_blob(local)?;
    let request = proto::PutRequest {
        kind: proto::EntityKind::Note,
        base_version: 0,
        blob: Some(hash),
        moved_from: None,
    };
    if let PutResult::Applied(response) = client.put_entity(vault, &copy_path, &request)? {
        state.versions.insert(copy_path.clone(), response.version);
    }

    outcome.conflict_copies.push(copy_path);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_move(
    client: &Client,
    vault: &str,
    root: &Path,
    from: &str,
    to: &str,
    state: &mut ClientState,
    today: &str,
    outcome: &mut PushOutcome,
) -> Result<(), ClientError> {
    let content = fs::read(root.join(to))?;
    let base = version_of(state, to);
    let hash = client.put_blob(&content)?;

    let request = proto::PutRequest {
        kind: proto::EntityKind::Note,
        base_version: base,
        blob: Some(hash),
        moved_from: Some(from.to_string()),
    };

    match client.put_entity(vault, to, &request)? {
        PutResult::Applied(response) => {
            state.versions.insert(to.to_string(), response.version);
            state.versions.remove(from);
        }
        PutResult::Conflict(conflict) => {
            reconcile(client, vault, root, to, EntryKind::Note, base, conflict, state, today, outcome)?;
        }
    }

    Ok(())
}

fn fetch_ancestor(client: &Client, vault: &str, path: &str, base_version: u64) -> Result<Vec<u8>, ClientError> {
    let history = client.history(vault, path)?;
    let blob = history.versions.iter().find(|version| version.version == base_version).and_then(|v| v.blob.clone());

    match blob {
        Some(hash) => client.get_blob(&hash),
        None => Ok(Vec::new()),
    }
}

fn version_of(state: &ClientState, path: &str) -> u64 {
    state.versions.get(path).copied().unwrap_or(0)
}

fn to_proto(kind: EntryKind) -> proto::EntityKind {
    match kind {
        EntryKind::Note => proto::EntityKind::Note,
        EntryKind::BookMeta => proto::EntityKind::BookMeta,
        EntryKind::Folder => proto::EntityKind::Folder,
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Splits a `/`-joined path into its directory and the note's stem.
fn split(path: &str) -> (String, String) {
    let (dir, file) = match path.rsplit_once('/') {
        Some((dir, file)) => (dir.to_string(), file.to_string()),
        None => (String::new(), path.to_string()),
    };
    let stem = file.strip_suffix(".md").unwrap_or(&file).to_string();

    (dir, stem)
}

fn dir_names(root: &Path, dir: &str) -> Vec<String> {
    let target = if dir.is_empty() { root.to_path_buf() } else { root.join(dir) };

    fs::read_dir(target)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}
