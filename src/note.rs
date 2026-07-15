//! qtbridge adapter for the note editor.
//!
//! The document model (reading, saving, wiki-link resolution) lives in
//! `booklet_core::document`; this type drives it. The editor is a single
//! surface over the whole note — the markdown is what you edit, and the C++
//! highlighter (src/cpp/) styles it live — so nothing here renders markdown or
//! deals in blocks.

use booklet_core::document::{self, Document};
use booklet_core::{config, vault};
use qtbridge::qobject;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct NoteEditor {
    vaults: Vec<PathBuf>, // all configured vaults, for wiki-link resolution
    document: Option<Document>,
    /// The editor's text has outrun the disk. Held here rather than in QML so
    /// that opening another note can flush it first — otherwise switching notes
    /// mid-edit would drop the last keystrokes.
    unsaved: bool,
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
        // Never leave the note you are leaving half-written.
        self.flush();

        match Document::open(PathBuf::from(&id)) {
            Ok(document) => {
                let title = document.title();
                self.document = Some(document);
                // The editor loads the source off this signal.
                self.note_opened(id, title);
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

    /// Closes the open note, leaving the editor empty. Reports an empty id so
    /// the tree, breadcrumb, marginalia and window title clear with it.
    #[qslot]
    fn close(&mut self) {
        self.flush();
        self.document = None;
        self.note_opened(String::new(), String::new());
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

    /// The open note's markdown. The editor is one surface over the whole note,
    /// so this is what it shows.
    #[qslot]
    fn source(&self) -> String {
        self.document.as_ref().map(|document| document.source().to_string()).unwrap_or_default()
    }

    /// Takes the editor's text, without writing. Cheap enough to call on every
    /// keystroke; `flush` decides when it reaches the disk.
    #[qslot]
    fn set_source(&mut self, text: String) {
        let Some(document) = &mut self.document else {
            return;
        };

        document.set_source(&text);
        self.unsaved = true;
    }

    /// Writes the note if it has unsaved text. Called by the editor on a
    /// debounce and on focus loss, and by `open`/`close` before they move on.
    #[qslot]
    fn flush(&mut self) {
        if !self.unsaved {
            return;
        }

        let Some(document) = &self.document else {
            self.unsaved = false;
            return;
        };

        if let Err(error) = document.write() {
            // TODO (M3): surface this in the UI instead of the console.
            eprintln!("booklet: could not save note: {error}");
            return;
        }

        self.unsaved = false;
        self.saved();
    }

    /// The note was written. Deliberately does not carry the text: the editor
    /// already has it, and re-reading it would move the caret mid-typing.
    #[qsignal]
    fn saved(&mut self);

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
