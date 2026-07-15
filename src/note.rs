//! qtbridge adapter for the block editor.
//!
//! The document model (block parsing, commit, wiki-link resolution) lives in
//! `booklet_core::document`; this type drives it, serializes blocks for QML, and
//! renders the `[[wiki-link]]` scheme — which stays app-side so the core stays
//! scheme-agnostic. The note title is block 0 (`# Title`), so clicking it
//! reveals its markdown like any other block.

use booklet_core::document::{self, Document};
use booklet_core::{config, vault};
use qtbridge::qobject;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// URL scheme used to smuggle wiki-links through Qt's markdown renderer so QML
/// can intercept them in onLinkActivated.
const LINK_SCHEME: &str = "booklet://";

/// A block as handed to QML: the core block plus its rendered `display`.
#[derive(Serialize)]
struct BlockView {
    index: usize,
    kind: String,
    source: String,  // raw markdown slice — shown when the block is clicked
    display: String, // markdown with [[x]] -> [x](booklet://x) for rendering
}

#[derive(Default)]
pub struct NoteEditor {
    vaults: Vec<PathBuf>, // all configured vaults, for wiki-link resolution
    document: Option<Document>,
}

#[qobject(Singleton)]
impl NoteEditor {
    /// Loads the configured vaults so wiki-links resolve across all of them.
    #[qslot]
    fn load(&mut self) {
        self.vaults = match config::load(&crate::library::default_config_path()) {
            Ok(config) => config.vaults,
            Err(error) => {
                // TODO (M3): surface this in the UI instead of the console.
                eprintln!("booklet: could not read vault list: {error}");
                Vec::new()
            }
        };
    }

    #[qslot]
    fn open(&mut self, id: String) {
        match Document::open(PathBuf::from(&id)) {
            Ok(document) => {
                let title = document.title();
                self.document = Some(document);
                self.note_opened(id, title);
                self.blocks_changed();
            }
            Err(error) => {
                // TODO (M3): surface this in the UI instead of the console.
                eprintln!("booklet: could not open note '{id}': {error}");
            }
        }
    }

    /// Follow a wiki-link: open the note whose file stem matches, searching only
    /// the open note's own vault. A link to a note that does not exist yet
    /// creates it beside the note being read (as Obsidian does).
    #[qslot]
    fn open_by_title(&mut self, title: String) {
        if let Some(path) = self.current_vault().and_then(|vault| document::find_note(vault, &title))
        {
            self.open(path.to_string_lossy().into_owned());
            return;
        }

        let created = match self.current_folder() {
            Some(folder) => document::create_note(folder, &title),
            None => return, // no open note, so nowhere to put it
        };

        match created {
            Ok(path) => self.open(path.to_string_lossy().into_owned()),
            Err(error) => {
                // TODO (M3): surface this in the UI instead of the console.
                eprintln!("booklet: could not create note '{title}': {error}");
            }
        }
    }

    #[qslot]
    fn current_id(&self) -> String {
        self.document
            .as_ref()
            .map(|document| document.path().to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    #[qslot]
    fn title(&self) -> String {
        self.document.as_ref().map(Document::title).unwrap_or_default()
    }

    /// The open note's place in its vault as breadcrumb segments (book, any
    /// sections, then the note), for the topbar.
    #[qslot]
    fn breadcrumb(&self) -> String {
        let segments = match (self.document.as_ref(), self.current_vault()) {
            (Some(document), Some(vault)) => vault::breadcrumb_of(vault, document.path()),
            _ => Vec::new(),
        };

        serde_json::to_string(&segments).expect("breadcrumb serializes to JSON")
    }

    #[qslot]
    fn blocks(&self) -> String {
        let Some(document) = &self.document else {
            return "[]".into();
        };

        let blocks: Vec<BlockView> = document
            .blocks()
            .into_iter()
            .map(|block| BlockView {
                display: rewrite_wikilinks(&block.source),
                index: block.index,
                kind: block.kind,
                source: block.source,
            })
            .collect();
        // Blocks hold only strings and numbers, so serialization cannot fail.
        serde_json::to_string(&blocks).expect("blocks serialize to JSON")
    }

    /// Called by QML when the user leaves an edited block.
    #[qslot]
    fn commit_block(&mut self, index: usize, new_source: String) {
        let Some(document) = &mut self.document else {
            return;
        };

        if let Err(error) = document.commit_block(index, &new_source) {
            // TODO (M3): surface this in the UI instead of the console.
            eprintln!("booklet: could not save note: {error}");
        }

        self.blocks_changed();
    }

    #[qsignal]
    fn blocks_changed(&mut self);

    #[qsignal]
    fn note_opened(&mut self, id: String, title: String);
}

impl NoteEditor {
    /// The vault holding the open note. Links resolve only inside it, so a
    /// vault stays self-contained.
    fn current_vault(&self) -> Option<&Path> {
        let document = self.document.as_ref()?;
        vault::vault_of(&self.vaults, document.path())
    }

    /// The folder holding the open note — where a note created by following an
    /// unresolved link lands.
    fn current_folder(&self) -> Option<&Path> {
        self.document.as_ref()?.path().parent()
    }
}

/// [[Title]] -> [Title](booklet://Title) so MarkdownText renders a link that
/// QML can intercept via onLinkActivated.
fn rewrite_wikilinks(source: &str) -> String {
    let pattern = regex::Regex::new(r"\[\[([^\]\|]+)(\|[^\]]+)?\]\]").unwrap();
    pattern
        .replace_all(source, |caps: &regex::Captures| {
            let target = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let label = caps
                .get(2)
                .map(|m| m.as_str().trim_start_matches('|'))
                .unwrap_or(target);
            format!("[{label}]({LINK_SCHEME}{target})")
        })
        .into_owned()
}
