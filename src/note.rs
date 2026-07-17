//! qtbridge adapter for the note editor.
//!
//! The document model (reading, saving, wiki-link resolution) lives in
//! `booklet_core::document`; this type drives it. The editor is a single
//! surface over the whole note — the markdown is what you edit, and the C++
//! highlighter (src/cpp/) styles it live — so nothing here renders markdown or
//! deals in blocks.

use booklet_core::document::{self, Document};
use booklet_core::{config, links, tags, vault};
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
            Ok(config) => config.vault_paths(),
            Err(error) => {
                self.failed(format!("Could not read vault list: {error}"));
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
                self.failed(format!("Could not open note '{id}': {error}"));
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
                self.failed(format!("Could not create note '{title}': {error}"));
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

    /// The line above the note: which section it sits in, and when it was last
    /// written (epoch seconds; -1 when unknown). QML does the wording.
    #[qslot]
    fn meta(&self) -> String {
        let Some(document) = &self.document else {
            return "{}".into();
        };

        let section = document
            .path()
            .parent()
            .and_then(|folder| folder.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let modified = document.modified().map(|seconds| seconds as f64).unwrap_or(-1.0);

        serde_json::json!({ "section": section, "modified": modified }).to_string()
    }

    /// The `[[wiki-links]]` this note points at, each with the id to open and
    /// whether a note by that title exists. Read from the editor's text rather
    /// than the disk, so the star map keeps up with what has been typed.
    #[qslot]
    fn outgoing_links(&self) -> String {
        let links = match (self.document.as_ref(), self.current_vault()) {
            (Some(document), Some(vault)) => links::outgoing_links(vault, document.source()),
            _ => Vec::new(),
        };

        let rows: Vec<serde_json::Value> = links
            .into_iter()
            .map(|link| {
                serde_json::json!({
                    "title": link.title,
                    // "" for an unresolved link: there is no note to open yet,
                    // so QML creates it by title instead.
                    "id": link.target.map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                })
            })
            .collect();

        serde_json::to_string(&rows).expect("outgoing links serialize to JSON")
    }

    /// The `#tags` written in this note.
    #[qslot]
    fn tags(&self) -> String {
        let found = self.document.as_ref().map(|document| tags::tags_in(document.source()));

        serde_json::to_string(&found.unwrap_or_default()).expect("tags serialize to JSON")
    }

    /// A diff of `other` (an older version) against the open note's current text,
    /// as labelled runs `[{op, text}]` (`equal`/`insert`/`delete`) — for the
    /// version history's colored diff.
    #[qslot]
    fn diff_segments(&self, other: String) -> String {
        let current = self.document.as_ref().map(|document| document.source()).unwrap_or("");
        let segments = booklet_core::diff_segments(&other, current);

        serde_json::to_string(&segments).expect("diff segments serialize to JSON")
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
        self.set_unsaved(true);
    }

    /// Writes the note if it has unsaved text. Called by the editor on a
    /// debounce and on focus loss, and by `open`/`close` before they move on.
    #[qslot]
    fn flush(&mut self) {
        if !self.unsaved {
            return;
        }

        // Take the result before touching `self` again: emitting a signal needs
        // `&mut self`, which the borrow on `document` would block.
        let written = match &self.document {
            Some(document) => document.write(),
            None => {
                self.set_unsaved(false);
                return;
            }
        };

        if let Err(error) = written {
            // The text stays unsaved, so the indicator keeps saying so.
            self.failed(format!("Could not save note: {error}"));
            return;
        }

        self.set_unsaved(false);
    }

    /// Whether the editor's text has outrun the disk — drives the status bar.
    #[qslot]
    fn is_unsaved(&self) -> bool {
        self.unsaved
    }

    /// Sync merged this note's file on disk while it was open. Re-reads the merged
    /// text, adopts it as the in-memory source (now synced, so not unsaved), and
    /// returns the minimal edits — `[{pos, remove, insert}]` in UTF-16 units — that
    /// turn `current` (the live editor buffer) into it. The editor replays them in
    /// place, keeping the undo stack and the caret rather than reassigning the
    /// whole document.
    #[qslot]
    fn reload_edits(&mut self, current: String) -> String {
        let disk = match self.document.as_ref().map(|document| std::fs::read_to_string(document.path())) {
            Some(Ok(text)) => text,
            Some(Err(error)) => {
                self.failed(format!("Could not reload note: {error}"));
                return "[]".into();
            }
            None => return "[]".into(),
        };

        let edits = booklet_core::edit_script(&current, &disk);

        if let Some(document) = &mut self.document {
            document.set_source(&disk);
        }
        self.set_unsaved(false);

        serde_json::to_string(&edits).expect("edits serialize to JSON")
    }

    /// Emitted only when the state flips, not on every keystroke. `false` also
    /// means "just written", which is when backlinks are worth re-reading.
    #[qsignal]
    fn save_state_changed(&mut self, unsaved: bool);

    /// Something the user should see went wrong. The UI shows it; nothing here
    /// writes to a console nobody is reading.
    #[qsignal]
    fn failed(&mut self, message: String);

    #[qsignal]
    fn note_opened(&mut self, id: String, title: String);
}

impl NoteEditor {
    fn set_unsaved(&mut self, unsaved: bool) {
        if self.unsaved == unsaved {
            return;
        }

        self.unsaved = unsaved;
        self.save_state_changed(unsaved);
    }

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
