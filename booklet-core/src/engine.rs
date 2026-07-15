//! The stateful core engine.
//!
//! Owns the live vault tree (read from disk, the source of truth) and controls
//! persistence of the vault list and the expanded folders. Expansion lives on
//! each folder node; the engine only collects it into a flat path list when
//! saving. The qtbridge app drives an `Engine` and serializes its rows for QML.

use crate::config::{self, Config};
use crate::document;
use crate::vault::{self, BookInfo, Folder, Node, NoteInfo, Row, Vault, VaultInfo};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

pub struct Engine {
    config_path: PathBuf,
    vaults: Vec<Vault>,
    /// The vault being read. Its books are the tree's roots; the others are kept
    /// so their open folders survive a switch, but they are never rendered.
    active: Option<PathBuf>,
}

impl Engine {
    /// Creates an engine that persists to `config_path`. The caller chooses the
    /// location, so different apps or profiles can reuse the engine.
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path, vaults: Vec::new(), active: None }
    }

    /// Loads the persisted vaults and reopens the folders that were expanded.
    pub fn load(&mut self) -> io::Result<()> {
        let config = config::load(&self.config_path)?;
        let expanded: HashSet<PathBuf> = config.expanded.into_iter().collect();

        // Fall back to the first vault if the stored active one is gone.
        self.active = config
            .active
            .filter(|path| config.vaults.contains(path))
            .or_else(|| config.vaults.first().cloned());

        let active = self.active.clone();
        self.vaults = config
            .vaults
            .into_iter()
            .map(|path| {
                let is_active = active.as_deref() == Some(path.as_path());
                build_vault(path, &expanded, is_active)
            })
            .collect();

        Ok(())
    }

    /// The vault currently being read.
    pub fn active_path(&self) -> Option<&Path> {
        self.active.as_deref()
    }

    /// Every configured vault, for the vault menu.
    pub fn vaults(&self) -> Vec<VaultInfo> {
        self.vaults
            .iter()
            .map(|vault| VaultInfo {
                id: vault.path().to_string_lossy().into_owned(),
                name: vault.name(),
                active: self.active.as_deref() == Some(vault.path()),
            })
            .collect()
    }

    /// Switches which vault is being read, and persists. An unknown path is
    /// ignored.
    pub fn set_active(&mut self, path: &Path) -> io::Result<()> {
        if !self.vaults.iter().any(|vault| vault.path() == path) {
            return Ok(());
        }

        self.active = Some(path.to_path_buf());
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
        self.vaults.push(Vault::new(path.clone()));
        if is_first {
            return self.set_active(&path);
        }

        self.persist()
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

    fn rebuild_with(&mut self, expanded: &HashSet<PathBuf>) {
        let active = self.active.clone();

        self.vaults = self
            .vault_paths()
            .into_iter()
            .map(|path| {
                let is_active = active.as_deref() == Some(path.as_path());
                build_vault(path, expanded, is_active)
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

        let config =
            Config { vaults: self.vault_paths(), active: self.active.clone(), expanded };
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
