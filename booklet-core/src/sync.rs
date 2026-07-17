//! Local change tracking for sync.
//!
//! Before anything leaves the machine, Booklet needs to know what changed in a
//! vault since the last sync — cheaply for a quiet vault, and precisely enough
//! that a rename reads as a *move* rather than a delete-and-create. This module
//! keeps a [`Manifest`] snapshot of every synced entity (notes, `booklet.json`
//! files, and folders) and diffs a fresh scan against it to produce a
//! [`Change`] list.
//!
//! All of it lives in a `.booklet/` directory inside the vault. That directory
//! is **device-local and never syncs**; delete it and the next scan rebuilds it
//! from disk at the cost of rehashing every file. There is no consumer yet — the
//! upload engine is a later step — so this module is pure `booklet-core` with
//! unit tests and no Qt, no network.

use crate::is_markdown;
use crate::vault::BOOK_METADATA_FILE;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// The device-local sync directory inside a vault. Hidden, so the vault tree and
/// the scan below both skip it, and it never travels to the server.
pub const STATE_DIR: &str = ".booklet";
const MANIFEST_FILE: &str = "manifest.json";
const STATE_FILE: &str = "sync.json";

/// What a tracked path is. A note and its book's `booklet.json` are the sync
/// units CLAUDE.md names; folders are entities of their own so a section made on
/// one device appears on another before it holds a note.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Debug)]
pub enum EntryKind {
    Note,
    BookMeta,
    Folder,
}

/// One tracked entity's remembered state. `size` and `mtime_ns` are the gate
/// that decides whether to rehash; `hash` is the SHA-256 of the content and
/// doubles as the server's content-address later. Folders have no content, so
/// they carry a zero size, a zero mtime, and an empty hash.
///
/// The mtime is kept at nanosecond resolution on purpose: a millisecond gate
/// would miss a same-length in-place edit that lands in the same millisecond —
/// the same tie that pushed `last_opened` from seconds to milliseconds. A
/// coarse-mtime filesystem still leaves that one case to `hash`, which is
/// inherent to any mtime gate.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct Entry {
    kind: EntryKind,
    size: u64,
    mtime_ns: u64,
    hash: String,
}

/// A remembered snapshot of a vault's synced entities, keyed by vault-relative
/// path (`/`-joined, so it is stable across platforms).
#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
pub struct Manifest {
    entries: BTreeMap<String, Entry>,
}

/// The vault's binding to a server: which server vault it is, where the server
/// lives, how far its change feed has been read, and the server version last
/// confirmed for each path (the `base_version` a push sends). Persisted in
/// `.booklet/sync.json`; the client engine loads it on start and saves it after
/// each cycle.
#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
pub struct SyncState {
    pub vault_id: Option<String>,
    pub server_url: Option<String>,
    pub cursor: u64,
    #[serde(default)]
    pub versions: std::collections::HashMap<String, u64>,
    /// Notes whose last merge was partial, awaiting the user's review. Persisted
    /// so a flag survives a restart until it is dismissed.
    #[serde(default)]
    pub flagged: Vec<String>,
}

/// A single difference between two manifests. `Moved` is inferred for notes only
/// (see [`Manifest::diff`]).
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Change {
    Created { path: String, kind: EntryKind },
    Modified { path: String, kind: EntryKind },
    Deleted { path: String, kind: EntryKind },
    Moved { from: String, to: String },
}

impl Manifest {
    /// Reads the vault's current state, reusing `previous`'s hashes wherever a
    /// file's size and mtime are unchanged. A quiet vault therefore costs one
    /// `stat` per file and hashes nothing; a touched-but-unedited file is
    /// rehashed but produces no change (its hash still matches).
    pub fn scan(vault: &Path, previous: &Manifest) -> Manifest {
        let mut entries = BTreeMap::new();

        let walker = WalkDir::new(vault)
            .min_depth(1)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry.file_name()));

        for entry in walker.flatten() {
            let Some(key) = relative_key(vault, entry.path()) else {
                continue;
            };

            if entry.file_type().is_dir() {
                entries.insert(key, folder_entry());
            } else if let Some(kind) = file_kind(entry.path()) {
                entries.insert(key.clone(), file_entry(&entry, &key, kind, previous));
            }
        }

        Manifest { entries }
    }

    /// The changes that turn `old` into `new`.
    ///
    /// A path only in `new` is created, only in `old` is deleted, and in both
    /// with a different hash is modified — so a rewrite that lands on the same
    /// bytes produces nothing. Folders carry an empty hash and so are never
    /// reported as modified, only created or deleted.
    ///
    /// Moves are inferred for **notes**: a deleted note and a created note whose
    /// content hashes match are one move, which is CLAUDE.md's "tracked, not
    /// treated as delete+create when avoidable".
    ///
    /// Known gap: a *folder* move degrades to delete-plus-create (its notes ride
    /// along as their own creates and deletes). A folder has no content hash to
    /// match on, and coalescing the move only means anything at upload time
    /// against the server's folder semantics, which do not exist yet. Same
    /// concession the roadmap already makes for a note renamed *and* edited.
    pub fn diff(old: &Manifest, new: &Manifest) -> Vec<Change> {
        let mut changes = Vec::new();
        let mut created_notes = Vec::new();
        let mut deleted_notes = Vec::new();

        for (path, entry) in &new.entries {
            match old.entries.get(path) {
                None if entry.kind == EntryKind::Note => {
                    created_notes.push((path.clone(), entry.hash.clone()));
                }
                None => changes.push(Change::Created { path: path.clone(), kind: entry.kind }),
                Some(previous) if previous.hash != entry.hash => {
                    changes.push(Change::Modified { path: path.clone(), kind: entry.kind });
                }
                Some(_) => {}
            }
        }

        for (path, entry) in &old.entries {
            if new.entries.contains_key(path) {
                continue;
            }
            if entry.kind == EntryKind::Note {
                deleted_notes.push((path.clone(), entry.hash.clone()));
            } else {
                changes.push(Change::Deleted { path: path.clone(), kind: entry.kind });
            }
        }

        changes.extend(match_note_moves(deleted_notes, created_notes));
        changes
    }

    /// Loads the manifest from `.booklet/`; a missing file means nothing has been
    /// tracked yet, so we return an empty manifest and the next scan rehashes.
    pub fn load(vault: &Path) -> io::Result<Manifest> {
        read_json(&vault.join(STATE_DIR).join(MANIFEST_FILE))
    }

    /// Writes the manifest into `.booklet/`, atomically (temp file + rename) so a
    /// crash mid-write cannot corrupt it.
    pub fn save(&self, vault: &Path) -> io::Result<()> {
        write_json_atomic(&vault.join(STATE_DIR).join(MANIFEST_FILE), self)
    }
}

impl SyncState {
    pub fn load(vault: &Path) -> io::Result<SyncState> {
        read_json(&vault.join(STATE_DIR).join(STATE_FILE))
    }

    pub fn save(&self, vault: &Path) -> io::Result<()> {
        write_json_atomic(&vault.join(STATE_DIR).join(STATE_FILE), self)
    }
}

/// Pairs deleted notes with created notes of identical content into moves,
/// leaving the unmatched ones as plain deletes and creates.
fn match_note_moves(
    deleted: Vec<(String, String)>,
    created: Vec<(String, String)>,
) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut claimed = vec![false; created.len()];

    for (from, deleted_hash) in deleted {
        // An empty hash means the file could not be read, not that two notes are
        // the same; never call that a move.
        let mate = created.iter().enumerate().position(|(index, (_, created_hash))| {
            !claimed[index] && !deleted_hash.is_empty() && *created_hash == deleted_hash
        });

        match mate {
            Some(index) => {
                claimed[index] = true;
                changes.push(Change::Moved { from, to: created[index].0.clone() });
            }
            None => changes.push(Change::Deleted { path: from, kind: EntryKind::Note }),
        }
    }

    for (index, (path, _)) in created.into_iter().enumerate() {
        if !claimed[index] {
            changes.push(Change::Created { path, kind: EntryKind::Note });
        }
    }

    changes
}

/// Which kind of tracked file this is, or `None` for anything sync ignores.
fn file_kind(path: &Path) -> Option<EntryKind> {
    if is_markdown(path) {
        Some(EntryKind::Note)
    } else if path.file_name() == Some(OsStr::new(BOOK_METADATA_FILE)) {
        Some(EntryKind::BookMeta)
    } else {
        None
    }
}

/// A file's entry, reusing the previous hash when the size+mtime gate says the
/// content cannot have changed. An unreadable file falls through to an empty
/// hash rather than aborting the whole scan.
fn file_entry(
    entry: &walkdir::DirEntry,
    key: &str,
    kind: EntryKind,
    previous: &Manifest,
) -> Entry {
    let (size, mtime_ns) = match entry.metadata() {
        Ok(metadata) => (metadata.len(), mtime_ns(&metadata)),
        Err(_) => (0, 0),
    };

    if let Some(prior) = previous.entries.get(key) {
        if prior.kind == kind && prior.size == size && prior.mtime_ns == mtime_ns {
            return Entry { kind, size, mtime_ns, hash: prior.hash.clone() };
        }
    }

    let hash = hash_file(entry.path()).unwrap_or_default();
    Entry { kind, size, mtime_ns, hash }
}

fn folder_entry() -> Entry {
    Entry { kind: EntryKind::Folder, size: 0, mtime_ns: 0, hash: String::new() }
}

/// The SHA-256 of a file's bytes, hex-encoded. This is the same digest the
/// server content-addresses by, so it is computed once and reused.
fn hash_file(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);

    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn mtime_ns(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
}

fn is_hidden(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| name.starts_with('.'))
}

/// A path's key in the manifest: relative to the vault root, `/`-joined so it
/// reads the same on every platform. The vault root itself has no key.
fn relative_key(vault: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(vault).ok()?;

    let key = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    (!key.is_empty()).then_some(key)
}

/// Reads JSON, treating a missing file as the default value — the same contract
/// as `config::load`.
fn read_json<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error),
    }
}

/// Writes JSON atomically: to a temp file, then rename over the target, so a
/// reader never sees a half-written file.
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // These types are maps and strings, so serialization cannot fail.
    let text = serde_json::to_string_pretty(value).expect("sync state serializes to JSON");

    let temp = path.with_extension("tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_vault() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("booklet-sync-{}-{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Scans, then diffs the fresh scan against `previous`.
    fn changes(vault: &Path, previous: &Manifest) -> (Manifest, Vec<Change>) {
        let scanned = Manifest::scan(vault, previous);
        let changes = Manifest::diff(previous, &scanned);
        (scanned, changes)
    }

    #[test]
    fn create_modify_delete_are_reported() {
        let vault = temp_vault();
        fs::write(vault.join("Note.md"), "# One\n").unwrap();

        let (baseline, created) = changes(&vault, &Manifest::default());
        assert!(created.contains(&Change::Created {
            path: "Note.md".into(),
            kind: EntryKind::Note,
        }));

        fs::write(vault.join("Note.md"), "# One, edited\n").unwrap();
        let (baseline, modified) = changes(&vault, &baseline);
        assert_eq!(modified, vec![Change::Modified {
            path: "Note.md".into(),
            kind: EntryKind::Note,
        }]);

        fs::remove_file(vault.join("Note.md")).unwrap();
        let (_, deleted) = changes(&vault, &baseline);
        assert_eq!(deleted, vec![Change::Deleted {
            path: "Note.md".into(),
            kind: EntryKind::Note,
        }]);

        fs::remove_dir_all(&vault).unwrap();
    }

    /// The size+mtime gate must trust its verdict and not rehash. Seeded with a
    /// deliberately wrong hash for an unchanged file, the scan keeps that hash —
    /// proving it never read the file.
    #[test]
    fn gate_skips_rehash_when_size_and_mtime_match() {
        let vault = temp_vault();
        let note = vault.join("Note.md");
        fs::write(&note, "# real content\n").unwrap();

        let metadata = fs::metadata(&note).unwrap();
        let mut previous = Manifest::default();
        previous.entries.insert(
            "Note.md".into(),
            Entry {
                kind: EntryKind::Note,
                size: metadata.len(),
                mtime_ns: mtime_ns(&metadata),
                hash: "deadbeef".into(),
            },
        );

        let scanned = Manifest::scan(&vault, &previous);

        assert_eq!(scanned.entries["Note.md"].hash, "deadbeef");

        fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn rename_with_same_content_is_one_move() {
        let vault = temp_vault();
        fs::write(vault.join("Old.md"), "# Same\n").unwrap();

        let (baseline, _) = changes(&vault, &Manifest::default());

        fs::rename(vault.join("Old.md"), vault.join("New.md")).unwrap();
        let (_, moved) = changes(&vault, &baseline);

        assert_eq!(moved, vec![Change::Moved {
            from: "Old.md".into(),
            to: "New.md".into(),
        }]);

        fs::remove_dir_all(&vault).unwrap();
    }

    /// The documented gap: a note renamed *and* edited has no matching hash, so
    /// it degrades to delete-plus-create rather than a move.
    #[test]
    fn rename_plus_edit_degrades_to_delete_and_create() {
        let vault = temp_vault();
        fs::write(vault.join("Old.md"), "# Same\n").unwrap();

        let (baseline, _) = changes(&vault, &Manifest::default());

        fs::rename(vault.join("Old.md"), vault.join("New.md")).unwrap();
        fs::write(vault.join("New.md"), "# Different now\n").unwrap();
        let (_, mut result) = changes(&vault, &baseline);
        result.sort_by_key(|change| format!("{change:?}"));

        assert_eq!(result, vec![
            Change::Created { path: "New.md".into(), kind: EntryKind::Note },
            Change::Deleted { path: "Old.md".into(), kind: EntryKind::Note },
        ]);

        fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn folders_and_book_metadata_are_tracked() {
        let vault = temp_vault();
        let book = vault.join("Book");
        fs::create_dir_all(&book).unwrap();
        fs::write(book.join(BOOK_METADATA_FILE), r##"{ "color": "#3C5240" }"##).unwrap();

        let (baseline, created) = changes(&vault, &Manifest::default());
        assert!(created.contains(&Change::Created {
            path: "Book".into(),
            kind: EntryKind::Folder,
        }));
        assert!(created.contains(&Change::Created {
            path: "Book/booklet.json".into(),
            kind: EntryKind::BookMeta,
        }));

        fs::write(book.join(BOOK_METADATA_FILE), r##"{ "color": "#7C3128" }"##).unwrap();
        let (_, modified) = changes(&vault, &baseline);
        assert_eq!(modified, vec![Change::Modified {
            path: "Book/booklet.json".into(),
            kind: EntryKind::BookMeta,
        }]);

        fs::remove_dir_all(&vault).unwrap();
    }

    /// `.booklet/` is device-local; the scan must never see it, or the manifest
    /// would track and try to sync its own state.
    #[test]
    fn state_directory_is_never_scanned() {
        let vault = temp_vault();
        fs::write(vault.join("Note.md"), "# One\n").unwrap();
        fs::create_dir_all(vault.join(STATE_DIR)).unwrap();
        fs::write(vault.join(STATE_DIR).join("junk.md"), "# hidden\n").unwrap();

        let (scanned, changed) = changes(&vault, &Manifest::default());

        assert!(scanned.entries.keys().all(|key| !key.starts_with(STATE_DIR)));
        assert!(changed.iter().all(|change| !matches!(
            change,
            Change::Created { path, .. } if path.starts_with(STATE_DIR)
        )));

        fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn manifest_round_trips_through_disk() {
        let vault = temp_vault();
        fs::write(vault.join("Note.md"), "# One\n").unwrap();
        let scanned = Manifest::scan(&vault, &Manifest::default());

        scanned.save(&vault).unwrap();
        let loaded = Manifest::load(&vault).unwrap();

        assert_eq!(loaded, scanned);

        fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn sync_state_round_trips_and_defaults_when_absent() {
        let vault = temp_vault();

        assert_eq!(SyncState::load(&vault).unwrap(), SyncState::default());

        let state = SyncState {
            vault_id: Some("vault-42".into()),
            server_url: Some("https://notes.example".into()),
            cursor: 17,
            versions: std::collections::HashMap::from([
                ("Book/Note.md".to_string(), 3),
                ("booklet.json".to_string(), 1),
            ]),
            flagged: vec!["Book/Note.md".to_string()],
        };
        state.save(&vault).unwrap();

        assert_eq!(SyncState::load(&vault).unwrap(), state);

        fs::remove_dir_all(&vault).unwrap();
    }
}
