//! Qt-free core logic for Booklet. The [`Engine`] owns the live vault tree
//! (read from disk) and controls persistence; [`Document`] is the block editor
//! model, [`links`] resolves wiki-links both ways, and [`tags`] reads a note's
//! tags. [`sync`] tracks local changes against a remembered snapshot and
//! [`merge`] reconciles two devices' edits, both for the sync engine. The
//! qtbridge app layers thin adapters over these, so all domain logic is testable
//! without Qt.

pub mod config;
pub mod document;
pub mod engine;
pub mod links;
pub mod merge;
pub mod render;
pub mod search;
pub mod sync;
pub mod tags;
pub mod vault;

pub use document::{Block, Document};
pub use engine::Engine;
pub use links::{Backlink, OutgoingLink};
pub use merge::{
    conflict_copy_name, diff_segments, edit_script, map_caret, merge_booklet_json, merge_markdown,
    Edit, MarkdownMerge, Segment,
};
pub use search::Hit;
pub use sync::{Change, EntryKind, Manifest, SyncState};
pub use vault::{Book, BookInfo, Folder, Node, Note, NoteInfo, Row, Section, Vault};

use std::path::Path;

/// Notes are plain markdown files. Shared by the vault tree, the document
/// model, and the backlink scan.
pub(crate) fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "md")
}
