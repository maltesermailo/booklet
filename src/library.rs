//! qtbridge adapter over the core [`Engine`].
//!
//! The engine (in `booklet-core`) owns the library state and its persistence;
//! this type is a thin Qt shell that drives the engine, serializes its output
//! for QML, surfaces I/O errors, and keeps a file watcher pointed at the
//! configured vaults.

use booklet_core::Engine;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use qtbridge::{qobject, QObjectHolder};
use std::path::{Path, PathBuf};

pub struct Library {
    engine: Engine,
    /// Kept alive to keep watching; replacing it drops the previous watches.
    watcher: Option<RecommendedWatcher>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            engine: Engine::new(default_config_path()),
            watcher: None,
        }
    }
}

#[qobject(Singleton)]
impl Library {
    /// Loads the persisted vault list. Call once at startup.
    #[qslot]
    fn load(&mut self) {
        if let Err(error) = self.engine.load() {
            self.failed(format!("Could not read vault list: {error}"));
        }

        self.watch_vaults();
        self.tree_changed();
    }

    #[qslot]
    fn add_vault(&mut self, path: String) {
        if let Err(error) = self.engine.add_vault(PathBuf::from(path)) {
            self.failed(format!("Could not save vault list: {error}"));
        }

        self.watch_vaults();
        self.tree_changed();
    }

    #[qslot]
    fn remove_vault(&mut self, path: String) {
        if let Err(error) = self.engine.remove_vault(Path::new(&path)) {
            self.failed(format!("Could not save vault list: {error}"));
        }

        self.watch_vaults();
        self.tree_changed();
    }

    /// Rebuilds the tree from disk, preserving open folders. Invoked by the file
    /// watcher whenever a vault changes underneath us.
    #[qslot]
    fn refresh(&mut self) {
        self.engine.refresh();

        self.tree_changed();
    }

    #[qslot]
    fn visible_rows(&self) -> String {
        // Rows hold only strings, numbers, and bools, so serialization cannot fail.
        serde_json::to_string(&self.engine.visible_rows()).expect("visible rows serialize to JSON")
    }

    #[qslot]
    fn toggle(&mut self, id: String) {
        if let Err(error) = self.engine.toggle(Path::new(&id)) {
            self.failed(format!("Could not save expansion state: {error}"));
        }

        self.tree_changed();
    }

    #[qslot]
    fn books(&self) -> String {
        // Same as visible_rows: only plain fields, so serialization cannot fail.
        serde_json::to_string(&self.engine.books()).expect("books serialize to JSON")
    }

    /// Every note in the active vault, for the quick switcher.
    #[qslot]
    fn notes(&self) -> String {
        serde_json::to_string(&self.engine.notes()).expect("notes serialize to JSON")
    }

    /// Every configured vault, for the vault menu.
    #[qslot]
    fn vaults(&self) -> String {
        serde_json::to_string(&self.engine.vaults()).expect("vaults serialize to JSON")
    }

    /// The vault currently being read, or "" if none.
    #[qslot]
    fn active_vault(&self) -> String {
        self.engine
            .active_path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Reading size for the editor, in pixels.
    #[qslot]
    fn editor_font_size(&self) -> i32 {
        self.engine.editor_font_size() as i32
    }

    /// Sets the reading size. The engine clamps it to something usable.
    #[qslot]
    fn set_editor_font_size(&mut self, size: i32) {
        if let Err(error) = self.engine.set_editor_font_size(size.max(0) as u32) {
            self.failed(format!("Could not save the reading size: {error}"));
        }

        self.font_size_changed();
    }

    /// The reading size changed; the editor re-reads it.
    #[qsignal]
    fn font_size_changed(&mut self);

    /// Switches which vault is being read.
    #[qslot]
    fn set_active(&mut self, id: String) {
        if let Err(error) = self.engine.set_active(Path::new(&id)) {
            self.failed(format!("Could not save the active vault: {error}"));
        }

        self.watch_vaults();
        self.tree_changed();
    }

    /// Creates a note called `name` inside `parent_id`. Returns its path, or ""
    /// if it could not be created.
    #[qslot]
    fn create_note(&mut self, parent_id: String, name: String) -> String {
        let created = self.engine.create_note(Path::new(&parent_id), &name);
        self.tree_changed();

        match created {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => {
                self.failed(format!("Could not create note '{name}': {error}"));
                String::new()
            }
        }
    }

    /// Creates a section called `name` inside `parent_id`.
    #[qslot]
    fn create_section(&mut self, parent_id: String, name: String) {
        if let Err(error) = self.engine.create_section(Path::new(&parent_id), &name) {
            self.failed(format!("Could not create section '{name}': {error}"));
        }

        self.tree_changed();
    }

    /// Renames the note or section at `id`. Returns its new path, or "" if it
    /// could not be renamed.
    #[qslot]
    fn rename(&mut self, id: String, name: String) -> String {
        let renamed = self.engine.rename(Path::new(&id), &name);
        self.tree_changed();

        match renamed {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => {
                self.failed(format!("Could not rename to '{name}': {error}"));
                String::new()
            }
        }
    }

    /// Moves the note or section at `id` to the system Trash. Named for QML's
    /// sake — `delete` is a JavaScript keyword.
    #[qslot]
    fn delete_entry(&mut self, id: String) {
        if let Err(error) = self.engine.delete(Path::new(&id)) {
            self.failed(format!("Could not delete '{id}': {error}"));
        }

        self.tree_changed();
    }

    /// Closes every open folder in the active vault.
    #[qslot]
    fn collapse_all(&mut self) {
        if let Err(error) = self.engine.collapse_all() {
            self.failed(format!("Could not save expansion state: {error}"));
        }

        self.tree_changed();
    }

    /// Opens every folder down to `id` so it shows in the tree. The shelf view
    /// uses this to jump to a book.
    #[qslot]
    fn reveal(&mut self, id: String) {
        if let Err(error) = self.engine.reveal(Path::new(&id)) {
            self.failed(format!("Could not save expansion state: {error}"));
        }

        self.tree_changed();
    }

    // qtbridge 0.2 requires a signal's receiver to be `&mut self`, even though
    // emitting it does not mutate our state.
    #[qsignal]
    fn tree_changed(&mut self);

    /// Something the user should see went wrong. The UI shows it; nothing here
    /// writes to a console nobody is reading.
    #[qsignal]
    fn failed(&mut self, message: String);
}

impl Library {
    /// (Re)starts watching the active vault — the only one whose tree is on
    /// screen. The watcher callback runs on its own thread, so
    /// `QmlMethodInvoker` is the only safe way back in: it schedules `refresh`
    /// on the Qt event loop rather than touching this object directly.
    fn watch_vaults(&mut self) {
        let invoker = self.get_qml_method_invoker();
        // Deliberately undebounced: `refresh` is idempotent and re-reads the
        // whole tree, so the final event always leaves us consistent. A
        // leading-edge throttle would risk dropping that final event and
        // leaving a stale tree.
        let handler = move |result: notify::Result<notify::Event>| {
            if result.is_ok() {
                invoker.invoke_method("refresh");
            }
        };

        let mut watcher = match notify::recommended_watcher(handler) {
            Ok(watcher) => watcher,
            Err(error) => {
                self.failed(format!("Could not start the file watcher: {error}"));
                return;
            }
        };

        if let Some(vault) = self.engine.active_path() {
            if let Err(error) = watcher.watch(vault, RecursiveMode::Recursive) {
                eprintln!("booklet: could not watch '{}': {error}", vault.display());
            }
        }

        self.watcher = Some(watcher);
    }
}

/// Where the vault list is stored. Defaults to ~/.config/booklet/vaults.json —
/// plain JSON the user can inspect and edit. Set BOOKLET_CONFIG to point at
/// another location so a different app or profile can reuse the same engine.
pub(crate) fn default_config_path() -> PathBuf {
    if let Some(override_path) = std::env::var_os("BOOKLET_CONFIG") {
        return PathBuf::from(override_path);
    }

    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".config/booklet/vaults.json")
}
