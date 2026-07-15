//! The stateful core engine.
//!
//! Owns the live vault tree (read from disk, the source of truth) and controls
//! persistence of the vault list and the expanded folders. Expansion lives on
//! each folder node; the engine only collects it into a flat path list when
//! saving. The qtbridge app drives an `Engine` and serializes its rows for QML.

use crate::config::{self, Config, VaultEntry};
use crate::document;
use crate::search::{self, Hit};
use crate::vault::{self, BookInfo, Folder, Node, NoteInfo, Row, Vault, VaultInfo, BINDING_PALETTE};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reading size for the editor, in pixels, and the range the UI may set.
pub const DEFAULT_FONT_SIZE: u32 = 18;
pub const MIN_FONT_SIZE: u32 = 11;
pub const MAX_FONT_SIZE: u32 = 40;

/// How large the chrome draws and how much room it gives itself, as whole
/// percentages of the designed size. The reference is 100; the range is what
/// still lays out, checked by eye at both ends.
pub const DEFAULT_UI_SCALE: u32 = 100;
pub const MIN_UI_SCALE: u32 = 80;
pub const MAX_UI_SCALE: u32 = 160;
pub const DEFAULT_DENSITY: u32 = 100;
pub const MIN_DENSITY: u32 = 80;
pub const MAX_DENSITY: u32 = 150;

/// How many vaults the picker lists. Past this it is a filing cabinet, not a
/// list of where you were.
pub const MAX_RECENT_VAULTS: usize = 8;

/// The theme the app wears until told otherwise. The UI knows the names; the
/// engine only carries this one across restarts.
pub const DEFAULT_THEME: &str = "night";

pub struct Engine {
    config_path: PathBuf,
    vaults: Vec<Vault>,
    /// The vault being read. Its books are the tree's roots; the others are kept
    /// so their open folders survive a switch, but they are never rendered.
    active: Option<PathBuf>,
    editor_font_size: u32,
    theme: String,
    ui_scale: u32,
    density: u32,
}

impl Engine {
    /// Creates an engine that persists to `config_path`. The caller chooses the
    /// location, so different apps or profiles can reuse the engine.
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            vaults: Vec::new(),
            active: None,
            editor_font_size: DEFAULT_FONT_SIZE,
            theme: DEFAULT_THEME.into(),
            ui_scale: DEFAULT_UI_SCALE,
            density: DEFAULT_DENSITY,
        }
    }

    /// Loads the persisted vaults and reopens the folders that were expanded.
    pub fn load(&mut self) -> io::Result<()> {
        let config = config::load(&self.config_path)?;
        let expanded: HashSet<PathBuf> = config.expanded.into_iter().collect();
        self.editor_font_size = config
            .editor_font_size
            .map(|size| size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE))
            .unwrap_or(DEFAULT_FONT_SIZE);
        self.theme = config.theme.unwrap_or_else(|| DEFAULT_THEME.into());
        self.ui_scale = config
            .ui_scale
            .map(|scale| scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE))
            .unwrap_or(DEFAULT_UI_SCALE);
        self.density = config
            .density
            .map(|density| density.clamp(MIN_DENSITY, MAX_DENSITY))
            .unwrap_or(DEFAULT_DENSITY);

        // Fall back to the first vault if the stored active one is gone.
        self.active = config
            .active
            .filter(|path| config.vaults.iter().any(|entry| entry.path == *path))
            .or_else(|| config.vaults.first().map(|entry| entry.path.clone()));

        let active = self.active.clone();
        self.vaults = config
            .vaults
            .into_iter()
            .map(|entry| {
                let is_active = active.as_deref() == Some(entry.path.as_path());
                let mut vault = build_vault(entry.path, &expanded, is_active);
                vault.set_color(entry.color);
                vault.set_last_opened(entry.last_opened);
                vault
            })
            .collect();

        // A config written before the picker has no colours; give them one now
        // rather than leaving the dots blank.
        self.color_unpainted_vaults();

        // Starting up reopens the vault you were last in, and that is an open:
        // without this the picker would say "never opened" about the very vault
        // on screen. Not persisted here — the next thing that saves carries it,
        // and load() reporting a *write* error would be a lie about what failed.
        let now = now_millis();
        if let Some(active) = self.active.clone() {
            if let Some(vault) = self.vaults.iter_mut().find(|vault| vault.path() == active) {
                vault.set_last_opened(now);
            }
        }

        Ok(())
    }

    /// The vault currently being read.
    pub fn active_path(&self) -> Option<&Path> {
        self.active.as_deref()
    }

    /// Every configured vault, for the vault menu.
    pub fn vaults(&self) -> Vec<VaultInfo> {
        self.vaults.iter().map(|vault| self.info_for(vault)).collect()
    }

    /// The vaults for the picker: most recently opened first, and only as many
    /// as it can show. One list serves both this and the vault menu.
    pub fn recent_vaults(&self) -> Vec<VaultInfo> {
        let mut recent = self.vaults();
        // Sort on the vault itself, not the info, so ordering never depends on
        // what the info happens to expose.
        recent.sort_by(|a, b| b.last_opened.cmp(&a.last_opened).then(a.name.cmp(&b.name)));
        recent.truncate(MAX_RECENT_VAULTS);

        recent
    }

    /// Switches which vault is being read, and persists. An unknown path is
    /// ignored.
    pub fn set_active(&mut self, path: &Path) -> io::Result<()> {
        if !self.vaults.iter().any(|vault| vault.path() == path) {
            return Ok(());
        }

        self.active = Some(path.to_path_buf());
        // Opening a vault is what "recently opened" means.
        let now = now_millis();
        if let Some(vault) = self.vaults.iter_mut().find(|vault| vault.path() == path) {
            vault.set_last_opened(now);
        }
        self.rebuild();

        self.persist()
    }

    /// Adds a vault (ignoring duplicates) and persists. The first vault added
    /// becomes the one you are reading.
    pub fn add_vault(&mut self, path: PathBuf) -> io::Result<()> {
        if self.vaults.iter().any(|vault| vault.path() == path) {
            return Ok(());
        }

        let is_first = self.vaults.is_empty();
        let mut vault = Vault::new(path.clone());
        vault.set_color(self.spare_color());
        self.vaults.push(vault);

        if is_first {
            return self.set_active(&path);
        }

        self.persist()
    }

    /// Creates a vault at `path` — the folder, one book in it, and one note in
    /// that — then adds it and opens it. This is what the picker's quick start
    /// does, so a first run lands somewhere to write rather than nowhere.
    ///
    /// Refuses a folder that already holds anything: turning someone's existing
    /// directory into a vault is the caller's decision to make, through
    /// `add_vault`, not a side effect of this.
    pub fn create_vault(&mut self, path: PathBuf, book: &str, note: &str) -> io::Result<()> {
        if path.exists() && path.read_dir().is_ok_and(|mut entries| entries.next().is_some()) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("'{}' already has something in it", path.display()),
            ));
        }

        let book_dir = path.join(validate_name(book)?);
        std::fs::create_dir_all(&book_dir)?;
        vault::Binding::write(&book_dir, BINDING_PALETTE[0], "Library")?;
        document::create_note(&book_dir, validate_name(note)?)?;

        self.add_vault(path.clone())?;

        // add_vault only opens the first vault added; a vault you just asked to
        // be made is one you meant to open.
        self.set_active(&path)
    }

    /// Removes the vault at `path` and persists. Removing the active vault falls
    /// back to whichever is left.
    pub fn remove_vault(&mut self, path: &Path) -> io::Result<()> {
        self.vaults.retain(|vault| vault.path() != path);

        if self.active.as_deref() == Some(path) {
            self.active = self.vaults.first().map(|vault| vault.path().to_path_buf());
            self.rebuild();
        }

        self.persist()
    }

    /// Toggles the expanded state of the folder at `path` and persists.
    pub fn toggle(&mut self, path: &Path) -> io::Result<()> {
        let expanded = self.collect_expanded();

        for vault in &mut self.vaults {
            if toggle_folder(vault, path, &expanded) {
                break;
            }
        }

        self.persist()
    }

    /// Expands every folder from the containing vault down to `path`, so it
    /// becomes visible in the tree, and persists. Used by the shelf view to jump
    /// to a book.
    pub fn reveal(&mut self, path: &Path) -> io::Result<()> {
        let expanded = self.collect_expanded();

        for vault in &mut self.vaults {
            if path.starts_with(vault.path()) {
                reveal_folder(vault, path, &expanded);
                break;
            }
        }

        self.persist()
    }

    /// Rebuilds the tree from disk, preserving which folders are open. Call this
    /// when the vault contents have changed underneath us.
    pub fn refresh(&mut self) {
        self.rebuild();
    }

    /// Creates a note called `name` inside `parent` and returns its path.
    pub fn create_note(&mut self, parent: &Path, name: &str) -> io::Result<PathBuf> {
        let path = document::create_note(parent, validate_name(name)?)?;
        self.rebuild();

        Ok(path)
    }

    /// Creates a section (a folder) called `name` inside `parent` and returns
    /// its path.
    pub fn create_section(&mut self, parent: &Path, name: &str) -> io::Result<PathBuf> {
        let path = parent.join(validate_name(name)?);
        std::fs::create_dir(&path)?;
        self.rebuild();

        Ok(path)
    }

    /// Renames a note or section in place and returns its new path.
    ///
    /// `[[wiki-links]]` pointing at the old title are deliberately left alone —
    /// renaming never edits other notes. They stop resolving, which the editor
    /// shows as an unresolved link.
    pub fn rename(&mut self, path: &Path, name: &str) -> io::Result<PathBuf> {
        let name = validate_name(name)?;
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "there is no parent"))?;

        let renamed =
            if path.is_dir() { parent.join(name) } else { parent.join(format!("{name}.md")) };
        if renamed == path {
            return Ok(renamed);
        }
        // std::fs::rename would happily replace the target.
        if renamed.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("'{name}' is already here"),
            ));
        }

        std::fs::rename(path, &renamed)?;
        self.rebuild();

        Ok(renamed)
    }

    /// Moves a note or section to the system Trash, so it stays recoverable.
    pub fn delete(&mut self, path: &Path) -> io::Result<()> {
        trash::delete(path).map_err(|error| io::Error::other(error.to_string()))?;
        self.rebuild();

        Ok(())
    }

    /// Closes every open folder in the active vault, and persists. Other vaults
    /// keep their open folders.
    pub fn collapse_all(&mut self) -> io::Result<()> {
        let Some(active) = self.active.clone() else {
            return Ok(());
        };

        let expanded: HashSet<PathBuf> = self
            .collect_expanded()
            .into_iter()
            .filter(|path| !path.starts_with(&active) || *path == active)
            .collect();
        self.rebuild_with(&expanded);

        self.persist()
    }

    /// Reading size for the editor, in pixels.
    pub fn editor_font_size(&self) -> u32 {
        self.editor_font_size
    }

    /// Sets the reading size, clamped to something usable, and persists.
    pub fn set_editor_font_size(&mut self, size: u32) -> io::Result<()> {
        let size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        if size == self.editor_font_size {
            return Ok(());
        }

        self.editor_font_size = size;
        self.persist()
    }

    /// How large the chrome draws, as a percentage.
    pub fn ui_scale(&self) -> u32 {
        self.ui_scale
    }

    /// Sets the chrome's size, clamped to what still lays out, and persists.
    pub fn set_ui_scale(&mut self, scale: u32) -> io::Result<()> {
        let scale = scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
        if scale == self.ui_scale {
            return Ok(());
        }

        self.ui_scale = scale;
        self.persist()
    }

    /// How much room the chrome gives itself, as a percentage.
    pub fn density(&self) -> u32 {
        self.density
    }

    /// Sets the chrome's roominess, clamped, and persists.
    pub fn set_density(&mut self, density: u32) -> io::Result<()> {
        let density = density.clamp(MIN_DENSITY, MAX_DENSITY);
        if density == self.density {
            return Ok(());
        }

        self.density = density;
        self.persist()
    }

    /// The theme the UI wears.
    pub fn theme(&self) -> &str {
        &self.theme
    }

    /// Remembers which theme the UI wears. Any name is accepted: naming the
    /// themes is the UI's business, and it already falls back for one it does
    /// not recognise.
    pub fn set_theme(&mut self, theme: &str) -> io::Result<()> {
        if theme == self.theme {
            return Ok(());
        }

        self.theme = theme.to_string();
        self.persist()
    }

    /// The configured vault paths.
    pub fn vault_paths(&self) -> Vec<PathBuf> {
        self.vaults.iter().map(|vault| vault.path().to_path_buf()).collect()
    }

    /// The rows currently visible: the active vault's books and their open
    /// descendants. The vault itself is not a row.
    pub fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(vault) = self.active_vault() {
            vault.append_book_rows(&mut rows);
        }

        rows
    }

    /// Every note in the active vault, for the quick switcher.
    pub fn notes(&self) -> Vec<NoteInfo> {
        self.active_path().map(vault::notes_in).unwrap_or_default()
    }

    /// The active vault's books, for the shelf view.
    pub fn books(&self) -> Vec<BookInfo> {
        self.active_path().map(vault::books_in).unwrap_or_default()
    }

    /// Sets a book's binding — the colour of its spine and the shelf it stands
    /// on — by writing the book's own booklet.json, then re-reads the tree.
    pub fn set_binding(&mut self, book: &Path, color: &str, shelf: &str) -> io::Result<()> {
        vault::Binding::write(book, color, shelf)?;
        self.refresh();

        Ok(())
    }

    /// Notes in the active vault whose text contains `query`.
    pub fn search(&self, query: &str) -> Vec<Hit> {
        self.active_path().map(|vault| search::search(vault, query)).unwrap_or_default()
    }

    fn info_for(&self, vault: &Vault) -> VaultInfo {
        VaultInfo {
            id: vault.path().to_string_lossy().into_owned(),
            name: vault.name(),
            active: self.active.as_deref() == Some(vault.path()),
            color: vault.color().to_string(),
            last_opened: vault.last_opened(),
        }
    }

    /// The first palette colour no vault is using, so two vaults added in a row
    /// do not get the same dot. Past six vaults the palette repeats, which is
    /// better than inventing colours outside it.
    fn spare_color(&self) -> String {
        let taken: Vec<&str> = self.vaults.iter().map(|vault| vault.color()).collect();

        BINDING_PALETTE
            .iter()
            .find(|color| !taken.contains(color))
            .unwrap_or(&BINDING_PALETTE[self.vaults.len() % BINDING_PALETTE.len()])
            .to_string()
    }

    /// Gives a colour to any vault that has none — one loaded from a config
    /// written before vaults had colours.
    fn color_unpainted_vaults(&mut self) {
        for index in 0..self.vaults.len() {
            if self.vaults[index].color().is_empty() {
                let color = self.spare_color();
                self.vaults[index].set_color(color);
            }
        }
    }

    fn active_vault(&self) -> Option<&Vault> {
        let active = self.active.as_deref()?;
        self.vaults.iter().find(|vault| vault.path() == active)
    }

    /// Rebuilds every vault node from disk, keeping open folders. Non-active
    /// vaults are rebuilt too so their open folders survive a switch.
    fn rebuild(&mut self) {
        let expanded = self.collect_expanded();
        self.rebuild_with(&expanded);
    }

    /// Re-reads every vault's tree from disk. What a vault *is* — its colour,
    /// when it was last opened — is not on disk and so is carried across;
    /// rebuilding from paths alone would quietly wipe both, and the file watcher
    /// calls this on every write.
    fn rebuild_with(&mut self, expanded: &HashSet<PathBuf>) {
        let active = self.active.clone();

        self.vaults = self
            .vaults
            .iter()
            .map(|vault| (vault.path().to_path_buf(), vault.color().to_string(), vault.last_opened()))
            .map(|(path, color, last_opened)| {
                let is_active = active.as_deref() == Some(path.as_path());
                let mut rebuilt = build_vault(path, expanded, is_active);
                rebuilt.set_color(color);
                rebuilt.set_last_opened(last_opened);
                rebuilt
            })
            .collect();
    }

    fn collect_expanded(&self) -> HashSet<PathBuf> {
        let mut expanded = HashSet::new();
        for vault in &self.vaults {
            collect_expanded(vault, &mut expanded);
        }

        expanded
    }

    fn persist(&self) -> io::Result<()> {
        let mut expanded: Vec<PathBuf> = self.collect_expanded().into_iter().collect();
        expanded.sort();

        let config = Config {
            vaults: self
                .vaults
                .iter()
                .map(|vault| VaultEntry {
                    path: vault.path().to_path_buf(),
                    color: vault.color().to_string(),
                    last_opened: vault.last_opened(),
                })
                .collect(),
            active: self.active.clone(),
            expanded,
            editor_font_size: Some(self.editor_font_size),
            theme: Some(self.theme.clone()),
            ui_scale: Some(self.ui_scale),
            density: Some(self.density),
        };
        config::save(&self.config_path, &config)
    }
}

/// Names are typed by a person, so guard the two inputs that would otherwise
/// quietly do the wrong thing on disk.
fn validate_name(name: &str) -> io::Result<&str> {
    let name = name.trim();

    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a name cannot be empty or contain a path separator",
        ));
    }

    Ok(name)
}

/// Builds a vault node. The active vault is forced open — its books are the
/// tree's roots, so they load whether or not it appears in `expanded`.
///
/// Forcing it open also lands its path in the saved `expanded` set, and that is
/// deliberate: it is what keeps a vault hydrated once it stops being active, so
/// the folders you left open inside it are still open when you switch back. A
/// vault that has never been active has nothing to preserve and stays unread.
/// Wall-clock milliseconds since the epoch.
///
/// Milliseconds, not seconds: clicking a vault in the picker and switching again
/// happens well inside one second, and at second resolution those opens tie and
/// the list falls back to alphabetical — showing an order the user did not
/// create. Only ever used to order that list, so a clock that jumps merely
/// reorders it.
fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_millis() as u64).unwrap_or_default()
}

fn build_vault(path: PathBuf, expanded: &HashSet<PathBuf>, is_active: bool) -> Vault {
    let mut vault = Vault::new(path);

    if is_active {
        vault.set_expanded(true);
        let books = vault.load_children(expanded);
        *vault.children_mut() = books;
    } else {
        vault.hydrate(expanded);
    }

    vault
}

/// Flips the expanded state of the folder at `path`, loading its children on
/// open. Returns whether the folder was found in this subtree.
fn toggle_folder(folder: &mut dyn Folder, path: &Path, expanded: &HashSet<PathBuf>) -> bool {
    if folder.path() == path {
        let open = !folder.expanded();
        folder.set_expanded(open);
        if open && folder.children().is_empty() {
            let children = folder.load_children(expanded);
            *folder.children_mut() = children;
        }
        return true;
    }

    for child in folder.children_mut() {
        if let Node::Folder(sub) = child {
            if toggle_folder(sub.as_mut(), path, expanded) {
                return true;
            }
        }
    }

    false
}

/// Opens `folder`, then descends toward `target`, opening each folder on the
/// way so the target ends up visible.
fn reveal_folder(folder: &mut dyn Folder, target: &Path, expanded: &HashSet<PathBuf>) {
    if !folder.expanded() {
        folder.set_expanded(true);
        if folder.children().is_empty() {
            let children = folder.load_children(expanded);
            *folder.children_mut() = children;
        }
    }

    if folder.path() == target {
        return;
    }

    for child in folder.children_mut() {
        if let Node::Folder(sub) = child {
            if target.starts_with(sub.path()) {
                reveal_folder(sub.as_mut(), target, expanded);
                return;
            }
        }
    }
}

fn collect_expanded(folder: &dyn Folder, out: &mut HashSet<PathBuf>) {
    if folder.expanded() {
        out.insert(folder.path().to_path_buf());
    }

    for child in folder.children() {
        if let Node::Folder(sub) = child {
            collect_expanded(sub.as_ref(), out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Returns (config_path, vault_path). The vault holds one book with a
    /// section and two notes.
    fn fixture() -> (PathBuf, PathBuf) {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("booklet-engine-{}-{}", std::process::id(), unique));

        let book = root.join("Vault/Book");
        std::fs::create_dir_all(book.join("Section")).unwrap();
        std::fs::write(book.join("Top Note.md"), "# Top\n").unwrap();
        std::fs::write(book.join("Section/Deep Note.md"), "# Deep\n").unwrap();

        (root.join("config.json"), root.join("Vault"))
    }

    fn titles(engine: &Engine) -> Vec<String> {
        engine.visible_rows().into_iter().map(|row| row.title).collect()
    }

    fn cleanup(config_path: &Path) {
        std::fs::remove_dir_all(config_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn load_without_config_is_empty() {
        let (config_path, _vault) = fixture();
        let mut engine = Engine::new(config_path.clone());

        engine.load().unwrap();

        assert!(engine.visible_rows().is_empty());
        cleanup(&config_path);
    }

    #[test]
    fn the_active_vaults_books_are_the_roots() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();

        // The vault itself is not a row: its books are the tree, already at
        // depth 0 with nothing toggled.
        let rows = engine.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Book");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].kind, "book");

        engine.toggle(&vault.join("Book")).unwrap();
        assert_eq!(titles(&engine), ["Book", "Section", "Top Note"]);

        cleanup(&config_path);
    }

    #[test]
    fn expansion_persists_across_reload() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        engine.toggle(&vault.join("Book")).unwrap();

        let mut reloaded = Engine::new(config_path.clone());
        reloaded.load().unwrap();

        assert_eq!(reloaded.active_path(), Some(vault.as_path()));
        assert_eq!(titles(&reloaded), ["Book", "Section", "Top Note"]);
        cleanup(&config_path);
    }

    #[test]
    fn reveal_opens_every_folder_down_to_the_target() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();

        assert_eq!(titles(&engine), ["Book"]);

        engine.reveal(&vault.join("Book/Section")).unwrap();

        // The book and the section are opened on the way down.
        assert_eq!(titles(&engine), ["Book", "Section", "Deep Note", "Top Note"]);

        cleanup(&config_path);
    }

    #[test]
    fn refresh_picks_up_a_note_added_on_disk() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        engine.toggle(&vault.join("Book")).unwrap();

        std::fs::write(vault.join("Book/Zebra Note.md"), "# Zebra\n").unwrap();
        assert!(!titles(&engine).contains(&"Zebra Note".to_string())); // cached until refresh

        engine.refresh();
        assert!(titles(&engine).contains(&"Zebra Note".to_string()));

        cleanup(&config_path);
    }

    /// A second vault beside the fixture's, holding one book with one note.
    fn extra_vault(config_path: &Path, name: &str, book: &str) -> PathBuf {
        let vault = config_path.parent().unwrap().join(name);
        std::fs::create_dir_all(vault.join(book)).unwrap();
        std::fs::write(vault.join(book).join("Note.md"), "# Note\n").unwrap();
        vault
    }

    #[test]
    fn set_active_switches_which_vault_is_shown_and_persists() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first.clone()).unwrap();
        engine.add_vault(second.clone()).unwrap();

        // The first vault added is the one you are reading.
        assert_eq!(engine.active_path(), Some(first.as_path()));
        assert_eq!(titles(&engine), ["Book"]);

        engine.set_active(&second).unwrap();
        assert_eq!(titles(&engine), ["Ledger"]);

        let mut reloaded = Engine::new(config_path.clone());
        reloaded.load().unwrap();
        assert_eq!(reloaded.active_path(), Some(second.as_path()));
        assert_eq!(titles(&reloaded), ["Ledger"]);

        cleanup(&config_path);
    }

    #[test]
    fn books_and_notes_scope_to_the_active_vault() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first.clone()).unwrap();
        engine.add_vault(second.clone()).unwrap();

        let books: Vec<String> = engine.books().into_iter().map(|book| book.title).collect();
        assert_eq!(books, ["Book"]);

        engine.set_active(&second).unwrap();

        let books: Vec<String> = engine.books().into_iter().map(|book| book.title).collect();
        assert_eq!(books, ["Ledger"]);
        // The other vault's notes are invisible to the switcher.
        let notes: Vec<String> = engine.notes().into_iter().map(|note| note.title).collect();
        assert_eq!(notes, ["Note"]);

        cleanup(&config_path);
    }

    #[test]
    fn expansion_survives_switching_vaults() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first.clone()).unwrap();
        engine.add_vault(second.clone()).unwrap();
        engine.toggle(&first.join("Book")).unwrap();
        assert_eq!(titles(&engine), ["Book", "Section", "Top Note"]);

        engine.set_active(&second).unwrap();
        engine.set_active(&first).unwrap();

        // The book we left open is still open on the way back.
        assert_eq!(titles(&engine), ["Book", "Section", "Top Note"]);
        cleanup(&config_path);
    }

    #[test]
    fn create_note_and_section_land_in_the_tree() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        let book = vault.join("Book");
        engine.toggle(&book).unwrap();

        let note = engine.create_note(&book, "Fresh Note").unwrap();
        let section = engine.create_section(&book, "Fresh Section").unwrap();

        assert_eq!(note, book.join("Fresh Note.md"));
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "# Fresh Note\n");
        assert!(section.is_dir());
        // Sections sort before notes.
        assert_eq!(titles(&engine), ["Book", "Fresh Section", "Section", "Fresh Note", "Top Note"]);

        cleanup(&config_path);
    }

    #[test]
    fn a_loose_note_at_the_vault_root_is_visible() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();

        // A note outside any book still exists on disk, so the tree must show
        // it — otherwise creating one there looks like it vanished.
        let loose = engine.create_note(&vault, "Loose Note").unwrap();

        assert_eq!(loose, vault.join("Loose Note.md"));
        // Books sort before loose notes.
        assert_eq!(titles(&engine), ["Book", "Loose Note"]);

        cleanup(&config_path);
    }

    #[test]
    fn create_refuses_to_clobber_or_take_a_bad_name() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        let book = vault.join("Book");

        // "Top Note" is already there — creating it again must not overwrite it.
        let clobbered = engine.create_note(&book, "Top Note");
        assert_eq!(clobbered.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(book.join("Top Note.md")).unwrap(), "# Top\n");

        assert_eq!(
            engine.create_note(&book, "   ").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            engine.create_section(&book, "a/b").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        cleanup(&config_path);
    }

    #[test]
    fn rename_moves_the_file_and_leaves_links_alone() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        let book = vault.join("Book");
        engine.toggle(&book).unwrap();
        std::fs::write(book.join("Top Note.md"), "# Top\n\nSee [[Deep Note]].\n").unwrap();

        let renamed = engine.rename(&book.join("Section/Deep Note.md"), "Deeper Note").unwrap();

        assert_eq!(renamed, book.join("Section/Deeper Note.md"));
        assert!(!book.join("Section/Deep Note.md").exists());
        // Renaming never edits other notes: the link still names the old title.
        let referrer = std::fs::read_to_string(book.join("Top Note.md")).unwrap();
        assert!(referrer.contains("[[Deep Note]]"));

        cleanup(&config_path);
    }

    #[test]
    fn rename_refuses_to_replace_something_that_is_there() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        let book = vault.join("Book");
        engine.create_note(&book, "Other").unwrap();

        let clash = engine.rename(&book.join("Other.md"), "Top Note");

        assert_eq!(clash.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(book.join("Other.md").exists());
        assert_eq!(std::fs::read_to_string(book.join("Top Note.md")).unwrap(), "# Top\n");

        cleanup(&config_path);
    }

    /// Ignored on purpose: `delete` moves the file to the real system Trash, and
    /// a test suite should not leave debris in your Trash on every run. Run it
    /// by hand with `cargo test -- --ignored` when touching `delete`.
    #[test]
    #[ignore]
    fn delete_takes_the_note_out_of_the_tree() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        let book = vault.join("Book");
        engine.toggle(&book).unwrap();

        engine.delete(&book.join("Top Note.md")).unwrap();

        assert!(!book.join("Top Note.md").exists());
        assert!(!titles(&engine).contains(&"Top Note".to_string()));

        cleanup(&config_path);
    }

    #[test]
    fn editor_font_size_is_clamped_and_persists() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault).unwrap();

        assert_eq!(engine.editor_font_size(), DEFAULT_FONT_SIZE);

        engine.set_editor_font_size(24).unwrap();
        assert_eq!(engine.editor_font_size(), 24);

        // Nobody gets a 500px or a 2px reading size.
        engine.set_editor_font_size(500).unwrap();
        assert_eq!(engine.editor_font_size(), MAX_FONT_SIZE);
        engine.set_editor_font_size(1).unwrap();
        assert_eq!(engine.editor_font_size(), MIN_FONT_SIZE);

        engine.set_editor_font_size(20).unwrap();
        let mut reloaded = Engine::new(config_path.clone());
        reloaded.load().unwrap();
        assert_eq!(reloaded.editor_font_size(), 20);

        cleanup(&config_path);
    }

    #[test]
    fn recent_vaults_put_the_one_you_were_last_in_first() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first.clone()).unwrap();
        engine.add_vault(second.clone()).unwrap();

        // Opening is what makes a vault recent, so open them in a known order.
        engine.set_active(&first).unwrap();
        engine.set_active(&second).unwrap();

        let names: Vec<String> = engine.recent_vaults().into_iter().map(|info| info.name).collect();
        assert_eq!(names, ["Second", "Vault"]);

        // And going back to the first puts it back on top.
        engine.set_active(&first).unwrap();
        let names: Vec<String> = engine.recent_vaults().into_iter().map(|info| info.name).collect();
        assert_eq!(names, ["Vault", "Second"]);

        cleanup(&config_path);
    }

    /// `refresh` re-reads the tree from disk, and the file watcher calls it on
    /// every write. Colour and last-opened are not on disk, so a rebuild that
    /// forgot to carry them over would wipe the picker's list as you typed.
    #[test]
    fn refreshing_keeps_what_is_not_on_disk() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        engine.set_active(&vault).unwrap();

        let before = engine.vaults().remove(0);
        engine.refresh();
        let after = engine.vaults().remove(0);

        assert_eq!(after.color, before.color);
        assert_eq!(after.last_opened, before.last_opened);
        assert_ne!(after.last_opened, 0);

        cleanup(&config_path);
    }

    #[test]
    fn vaults_are_given_different_colors() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first).unwrap();
        engine.add_vault(second).unwrap();

        let colors: Vec<String> = engine.vaults().into_iter().map(|info| info.color).collect();
        assert_eq!(colors.len(), 2);
        assert_ne!(colors[0], colors[1]);
        assert!(BINDING_PALETTE.contains(&colors[0].as_str()));

        cleanup(&config_path);
    }

    /// A config of bare paths is what every install had before the picker.
    /// Loading one must keep the vault, give it a dot, and count reopening it
    /// as an open — otherwise the picker calls the vault on screen "never
    /// opened".
    #[test]
    fn a_config_of_bare_paths_loads_complete() {
        let (config_path, vault) = fixture();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &config_path,
            format!(
                r#"{{ "vaults": ["{0}"], "active": "{0}" }}"#,
                vault.to_string_lossy()
            ),
        )
        .unwrap();

        let mut engine = Engine::new(config_path.clone());
        engine.load().unwrap();

        let info = &engine.vaults()[0];
        assert_eq!(engine.active_path(), Some(vault.as_path()));
        assert!(!info.color.is_empty());
        assert_ne!(info.last_opened, 0);
        // The tree came up, so the notes are reachable.
        assert_eq!(titles(&engine), ["Book"]);

        cleanup(&config_path);
    }

    #[test]
    fn create_vault_seeds_a_book_and_a_note_and_opens_it() {
        let (config_path, _) = fixture();
        let fresh = config_path.parent().unwrap().join("Fresh");
        let mut engine = Engine::new(config_path.clone());

        engine.create_vault(fresh.clone(), "First Book", "Welcome").unwrap();

        assert_eq!(engine.active_path(), Some(fresh.as_path()));
        assert!(fresh.join("First Book/Welcome.md").exists());
        assert_eq!(engine.books()[0].title, "First Book");
        // It landed somewhere you can write immediately.
        assert_eq!(titles(&engine), ["First Book"]);

        cleanup(&config_path);
    }

    #[test]
    fn create_vault_refuses_a_folder_that_already_holds_something() {
        let (config_path, existing) = fixture();
        let mut engine = Engine::new(config_path.clone());

        // `existing` is the sample vault, full of somebody's notes.
        let error = engine.create_vault(existing, "Book", "Note").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(engine.vaults().is_empty());

        cleanup(&config_path);
    }

    #[test]
    fn ui_scale_and_density_are_clamped_and_persist() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault).unwrap();

        assert_eq!(engine.ui_scale(), DEFAULT_UI_SCALE);
        assert_eq!(engine.density(), DEFAULT_DENSITY);

        // Nobody gets chrome at 10% or 900%.
        engine.set_ui_scale(900).unwrap();
        assert_eq!(engine.ui_scale(), MAX_UI_SCALE);
        engine.set_density(1).unwrap();
        assert_eq!(engine.density(), MIN_DENSITY);

        engine.set_ui_scale(125).unwrap();
        engine.set_density(120).unwrap();
        let mut reloaded = Engine::new(config_path.clone());
        reloaded.load().unwrap();
        assert_eq!(reloaded.ui_scale(), 125);
        assert_eq!(reloaded.density(), 120);

        cleanup(&config_path);
    }

    #[test]
    fn theme_persists() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault).unwrap();

        assert_eq!(engine.theme(), DEFAULT_THEME);

        engine.set_theme("atlas").unwrap();
        let mut reloaded = Engine::new(config_path.clone());
        reloaded.load().unwrap();
        assert_eq!(reloaded.theme(), "atlas");

        cleanup(&config_path);
    }

    #[test]
    fn setting_a_binding_keeps_the_rest_of_booklet_json() {
        let (config_path, vault) = fixture();
        let book = vault.join("Book");
        // A key the app knows nothing about, as a hand-edited file may well have.
        std::fs::write(book.join("booklet.json"), r##"{ "color": "#7C3128", "mine": "keep me" }"##)
            .unwrap();

        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault).unwrap();
        engine.set_binding(&book, "#2F3E5C", "Work").unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(book.join("booklet.json")).unwrap())
                .unwrap();
        assert_eq!(written["color"], "#2F3E5C");
        assert_eq!(written["shelf"], "Work");
        assert_eq!(written["mine"], "keep me");
        // The tree re-read the file, so the shelf sees the new binding.
        assert_eq!(engine.books()[0].color, "#2F3E5C");

        cleanup(&config_path);
    }

    #[test]
    fn search_reads_the_active_vault_only() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        std::fs::write(second.join("Ledger/Hidden.md"), "# Hidden\n\nA deep secret.\n").unwrap();
        std::fs::write(first.join("Book/Top Note.md"), "# Top\n\nA deep secret too.\n").unwrap();

        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first).unwrap();
        engine.add_vault(second).unwrap();

        let titles: Vec<String> =
            engine.search("deep secret").into_iter().map(|hit| hit.title).collect();

        // The second vault holds a match, but it is not the one being read.
        assert_eq!(titles, ["Top Note"]);

        cleanup(&config_path);
    }

    #[test]
    fn collapse_all_closes_the_active_vault_but_spares_the_others() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first.clone()).unwrap();
        engine.add_vault(second.clone()).unwrap();
        engine.toggle(&first.join("Book")).unwrap();
        engine.set_active(&second).unwrap();
        engine.toggle(&second.join("Ledger")).unwrap();
        assert_eq!(titles(&engine), ["Ledger", "Note"]);

        engine.collapse_all().unwrap();
        assert_eq!(titles(&engine), ["Ledger"]);

        // The first vault's open book was left alone.
        engine.set_active(&first).unwrap();
        assert_eq!(titles(&engine), ["Book", "Section", "Top Note"]);

        cleanup(&config_path);
    }

    #[test]
    fn removing_the_active_vault_falls_back_to_another() {
        let (config_path, first) = fixture();
        let second = extra_vault(&config_path, "Second", "Ledger");
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(first.clone()).unwrap();
        engine.add_vault(second.clone()).unwrap();

        engine.remove_vault(&first).unwrap();

        assert_eq!(engine.active_path(), Some(second.as_path()));
        assert_eq!(titles(&engine), ["Ledger"]);
        cleanup(&config_path);
    }

    #[test]
    fn removed_vault_disappears_and_persists_empty() {
        let (config_path, vault) = fixture();
        let mut engine = Engine::new(config_path.clone());
        engine.add_vault(vault.clone()).unwrap();
        engine.remove_vault(&vault).unwrap();

        let mut reloaded = Engine::new(config_path.clone());
        reloaded.load().unwrap();

        assert!(reloaded.visible_rows().is_empty());
        cleanup(&config_path);
    }
}
