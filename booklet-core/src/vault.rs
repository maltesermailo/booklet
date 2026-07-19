//! The vault tree model.
//!
//! The library is a live tree read from disk (disk stays the source of truth):
//! [`Vault`] → [`Book`] → [`Section`]* → notes. The folder nodes share the
//! [`Folder`] trait — each owns its own `expanded` flag and reads its children
//! from disk when opened, so expansion is per-object state, never a shared
//! id-keyed set. [`Note`] is a leaf.

use crate::is_markdown;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DEFAULT_COLOR: &str = "#4A5560";

/// The binding palette. Books take a colour from it by hand; a vault is given
/// the first one going spare when it is added, so the picker's dots differ.
/// Theme.qml carries the same list for the book binding chips — QML cannot read
/// a Rust const, so the two are kept in step by hand.
pub const BINDING_PALETTE: [&str; 6] =
    ["#7C3128", "#2F3E5C", "#3C5240", "#A8842C", "#55364F", "#4A5560"];
const DEFAULT_SHELF: &str = "Library";
pub(crate) const BOOK_METADATA_FILE: &str = "booklet.json";

/// A row in the flattened tree handed to QML. A plain view DTO; behavior lives
/// on the nodes.
#[derive(Serialize, Clone)]
pub struct Row {
    pub id: String, // absolute path; unique, stable identifier
    pub title: String,
    pub depth: u32,
    pub kind: String,  // "vault" | "book" | "section" | "note"
    pub color: String, // binding color hex; books only
    pub expanded: bool,
    pub has_children: bool,
}

/// A vault's entry in the vault menu.
#[derive(Serialize)]
pub struct VaultInfo {
    pub id: String, // absolute path
    pub name: String,
    pub active: bool,
    pub color: String,
    /// Epoch milliseconds; 0 means it has never been opened.
    pub last_opened: u64,
}

/// A note's entry in the quick switcher.
#[derive(Serialize)]
pub struct NoteInfo {
    pub id: String,      // absolute path
    pub title: String,   // the note's file stem
    pub context: String, // breadcrumb, e.g. "Theologie / Lektüre"
}

/// A book's entry in the shelf view.
#[derive(Serialize)]
pub struct BookInfo {
    pub id: String,
    pub title: String,
    pub color: String,
    pub shelf: String,
    pub note_count: usize,
}

/// A child of a folder: either a nested folder or a note leaf.
pub enum Node {
    Folder(Box<dyn Folder>),
    Note(Note),
}

impl Node {
    fn append_rows(&self, depth: u32, out: &mut Vec<Row>) {
        match self {
            Node::Folder(folder) => folder.append_rows(depth, out),
            Node::Note(note) => note.append_row(depth, out),
        }
    }
}

/// A folder in the tree that can be expanded and reads its children from disk.
pub trait Folder {
    fn path(&self) -> &Path;
    fn kind(&self) -> &'static str;
    fn expanded(&self) -> bool;
    fn set_expanded(&mut self, value: bool);
    fn children(&self) -> &[Node];
    fn children_mut(&mut self) -> &mut Vec<Node>;

    /// Binding color for the row; empty except on books.
    fn color(&self) -> String {
        String::new()
    }

    /// Reads this folder's children from disk, hydrating each with `expanded`.
    fn load_children(&self, expanded: &HashSet<PathBuf>) -> Vec<Node>;

    /// Applies persisted expansion: if this folder is marked expanded, open it
    /// and read its children (which recurse into their own expanded state).
    fn hydrate(&mut self, expanded: &HashSet<PathBuf>) {
        if expanded.contains(self.path()) {
            self.set_expanded(true);
            let children = self.load_children(expanded);
            *self.children_mut() = children;
        }
    }

    /// Appends this folder's row and, when expanded, its descendants' rows.
    fn append_rows(&self, depth: u32, out: &mut Vec<Row>) {
        out.push(Row {
            id: path_id(self.path()),
            title: dir_name(self.path()),
            depth,
            kind: self.kind().into(),
            color: self.color(),
            expanded: self.expanded(),
            has_children: true,
        });

        if self.expanded() {
            for child in self.children() {
                child.append_rows(depth + 1, out);
            }
        }
    }
}

/// One configured library location; its children are books. It also carries
/// what the picker needs to list it: the colour of its dot and when it was last
/// opened.
pub struct Vault {
    path: PathBuf,
    color: String,
    /// Epoch milliseconds; 0 means never.
    last_opened: u64,
    expanded: bool,
    books: Vec<Node>,
}

impl Vault {
    pub fn new(path: PathBuf) -> Self {
        Self { path, color: String::new(), last_opened: 0, expanded: false, books: Vec::new() }
    }

    pub fn color(&self) -> &str {
        &self.color
    }

    pub fn set_color(&mut self, color: String) {
        self.color = color;
    }

    pub fn last_opened(&self) -> u64 {
        self.last_opened
    }

    pub fn set_last_opened(&mut self, seconds: u64) {
        self.last_opened = seconds;
    }

    /// The vault's folder name, as shown in the vault menu.
    pub fn name(&self) -> String {
        dir_name(&self.path)
    }

    /// Appends the vault's books, and their open descendants, starting at depth
    /// 0. The active vault *is* the tree — it never emits a row of its own.
    pub fn append_book_rows(&self, out: &mut Vec<Row>) {
        for book in &self.books {
            book.append_rows(0, out);
        }
    }
}

impl Folder for Vault {
    fn path(&self) -> &Path {
        &self.path
    }

    fn kind(&self) -> &'static str {
        "vault"
    }

    fn expanded(&self) -> bool {
        self.expanded
    }

    fn set_expanded(&mut self, value: bool) {
        self.expanded = value;
    }

    fn children(&self) -> &[Node] {
        &self.books
    }

    fn children_mut(&mut self) -> &mut Vec<Node> {
        &mut self.books
    }

    /// A vault's folders are its books. Loose markdown at the vault root is
    /// listed too — it is outside any book, but it is on disk, and the tree must
    /// never hide a file that exists.
    fn load_children(&self, expanded: &HashSet<PathBuf>) -> Vec<Node> {
        let mut nodes = Vec::new();

        for entry in sorted_entries(&self.path) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            if name.starts_with('.') || name == BOOK_METADATA_FILE {
                continue;
            }

            if path.is_dir() {
                let mut book = Book::new(path);
                book.hydrate(expanded);
                nodes.push(Node::Folder(Box::new(book)));
            } else if is_markdown(&path) {
                nodes.push(Node::Note(Note::new(path)));
            }
        }

        nodes
    }
}

/// A top-level folder within a vault. Its booklet.json carries the binding.
pub struct Book {
    path: PathBuf,
    binding: Binding,
    expanded: bool,
    entries: Vec<Node>,
}

impl Book {
    pub(crate) fn new(path: PathBuf) -> Self {
        let binding = Binding::read(&path);
        Self { path, binding, expanded: false, entries: Vec::new() }
    }

    /// The book's entry for the shelf view.
    pub fn info(&self) -> BookInfo {
        BookInfo {
            id: path_id(&self.path),
            title: dir_name(&self.path),
            color: self.binding.color.clone(),
            shelf: self.binding.shelf.clone(),
            note_count: note_count(&self.path),
        }
    }
}

impl Folder for Book {
    fn path(&self) -> &Path {
        &self.path
    }

    fn kind(&self) -> &'static str {
        "book"
    }

    fn color(&self) -> String {
        self.binding.color.clone()
    }

    fn expanded(&self) -> bool {
        self.expanded
    }

    fn set_expanded(&mut self, value: bool) {
        self.expanded = value;
    }

    fn children(&self) -> &[Node] {
        &self.entries
    }

    fn children_mut(&mut self) -> &mut Vec<Node> {
        &mut self.entries
    }

    fn load_children(&self, expanded: &HashSet<PathBuf>) -> Vec<Node> {
        folder_entries(&self.path, expanded)
    }
}

/// Any folder nested below a book, nesting without limit.
pub struct Section {
    path: PathBuf,
    expanded: bool,
    entries: Vec<Node>,
}

impl Section {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, expanded: false, entries: Vec::new() }
    }
}

impl Folder for Section {
    fn path(&self) -> &Path {
        &self.path
    }

    fn kind(&self) -> &'static str {
        "section"
    }

    fn expanded(&self) -> bool {
        self.expanded
    }

    fn set_expanded(&mut self, value: bool) {
        self.expanded = value;
    }

    fn children(&self) -> &[Node] {
        &self.entries
    }

    fn children_mut(&mut self) -> &mut Vec<Node> {
        &mut self.entries
    }

    fn load_children(&self, expanded: &HashSet<PathBuf>) -> Vec<Node> {
        folder_entries(&self.path, expanded)
    }
}

/// A markdown note; a leaf in the tree.
pub struct Note {
    path: PathBuf,
}

impl Note {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append_row(&self, depth: u32, out: &mut Vec<Row>) {
        out.push(Row {
            id: path_id(&self.path),
            title: note_title(&self.path),
            depth,
            kind: "note".into(),
            color: String::new(),
            expanded: false,
            has_children: false,
        });
    }
}

/// A book's binding: its color and shelf label from booklet.json.
pub struct Binding {
    pub color: String,
    pub shelf: String,
}

impl Binding {
    fn read(book_dir: &Path) -> Self {
        let metadata: Option<serde_json::Value> =
            std::fs::read_to_string(book_dir.join(BOOK_METADATA_FILE))
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok());

        match metadata {
            Some(metadata) => Binding {
                color: metadata["color"].as_str().unwrap_or(DEFAULT_COLOR).to_string(),
                shelf: metadata["shelf"].as_str().unwrap_or(DEFAULT_SHELF).to_string(),
            },
            None => Binding {
                color: DEFAULT_COLOR.into(),
                shelf: DEFAULT_SHELF.into(),
            },
        }
    }

    /// Writes the binding into the book's booklet.json, keeping whatever else
    /// the file holds: it is a plain file in the user's vault, and they may have
    /// put their own keys in it.
    pub fn write(book_dir: &Path, color: &str, shelf: &str) -> std::io::Result<()> {
        let path = book_dir.join(BOOK_METADATA_FILE);
        let existing: Option<serde_json::Value> =
            std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok());

        // Anything that is not an object (or is unreadable) is replaced rather
        // than indexed into, which would panic.
        let mut metadata = match existing {
            Some(value) if value.is_object() => value,
            _ => serde_json::json!({}),
        };

        metadata["color"] = color.into();
        metadata["shelf"] = shelf.into();

        // A map of strings, so serialization cannot fail.
        let text = serde_json::to_string_pretty(&metadata).expect("binding serializes to JSON");

        std::fs::write(&path, text)
    }
}

/// The children of a book or section: nested folders become sections, markdown
/// files become notes. Hidden entries and the metadata file are skipped.
fn folder_entries(dir: &Path, expanded: &HashSet<PathBuf>) -> Vec<Node> {
    let mut nodes = Vec::new();

    for entry in sorted_entries(dir) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        if name.starts_with('.') || name == BOOK_METADATA_FILE {
            continue;
        }

        if path.is_dir() {
            let mut section = Section::new(path);
            section.hydrate(expanded);
            nodes.push(Node::Folder(Box::new(section)));
        } else if is_markdown(&path) {
            nodes.push(Node::Note(Note::new(path)));
        }
    }

    nodes
}

/// The configured vault that contains `note`, if any. Vaults are self-contained
/// — links and backlinks never cross a vault boundary — so callers scope those
/// scans to the note's own vault.
pub fn vault_of<'a>(vaults: &'a [PathBuf], note: &Path) -> Option<&'a Path> {
    vaults.iter().map(PathBuf::as_path).find(|vault| note.starts_with(vault))
}

/// The books (immediate subfolders) of a vault, for the shelf view.
pub(crate) fn books_in(vault: &Path) -> Vec<BookInfo> {
    child_dirs(vault).into_iter().map(|path| Book::new(path).info()).collect()
}

/// Every note under `vault`, for the quick switcher.
pub(crate) fn notes_in(vault: &Path) -> Vec<NoteInfo> {
    walkdir::WalkDir::new(vault)
        .into_iter()
        .flatten()
        .map(|entry| entry.into_path())
        .filter(|path| is_markdown(path))
        .map(|path| NoteInfo {
            title: note_title(&path),
            context: context_of(vault, &path),
            id: path_id(&path),
        })
        .collect()
}

/// The note's location within its vault as breadcrumb segments — the book, any
/// sections, then the note itself. The vault is not included: one vault is
/// active at a time and the topbar already names it.
pub fn breadcrumb_of(vault: &Path, note: &Path) -> Vec<String> {
    let relative = note.strip_prefix(vault).unwrap_or(note);
    let mut segments: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();

    // The last segment is the file; show its title rather than "Note.md".
    if let Some(last) = segments.last_mut() {
        *last = note_title(note);
    }

    segments
}

/// The breadcrumb with each segment's absolute path, so the UI can make it
/// clickable — each folder (book, section) and the note itself carries the id to
/// reveal in the tree. Names match `breadcrumb_of`; the last segment is the note
/// title paired with the note's own path.
pub fn breadcrumb_with_paths(vault: &Path, note: &Path) -> Vec<(String, PathBuf)> {
    let relative = note.strip_prefix(vault).unwrap_or(note);
    let components: Vec<_> = relative.components().collect();

    let mut path = vault.to_path_buf();
    let mut out = Vec::with_capacity(components.len());
    for (index, part) in components.iter().enumerate() {
        path.push(part);
        let last = index == components.len() - 1;
        let name = if last {
            note_title(note)
        } else {
            part.as_os_str().to_string_lossy().into_owned()
        };
        out.push((name, path.clone()));
    }

    out
}

/// The note's location as a breadcrumb: the vault name followed by the folders
/// leading to it, e.g. "Theologie / Lektüre".
fn context_of(vault: &Path, note: &Path) -> String {
    let folder = note.parent().unwrap_or(note);
    let relative = folder.strip_prefix(vault).unwrap_or(folder);

    let mut parts = vec![dir_name(vault)];
    parts.extend(
        relative.components().map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );

    parts.join(" / ")
}

/// Directory entries sorted directories-first, then files, alphabetical within
/// each group. An unreadable directory yields an empty list rather than
/// blanking the whole tree.
fn sorted_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|entry| (entry.path().is_file(), entry.file_name()));

    entries
}

/// Immediate, non-hidden subdirectories of `dir`, alphabetically.
fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    sorted_entries(dir)
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| !is_hidden(path))
        .collect()
}

fn note_count(dir: &Path) -> usize {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|found| is_markdown(found.path()))
        .count()
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn path_id(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn note_title(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir_name(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Builds a throwaway vault and returns its path:
    ///   Vault/
    ///     Book/            booklet.json -> color, shelf
    ///       Section/
    ///         Deep Note.md
    ///       Top Note.md
    fn fixture() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let vault = std::env::temp_dir()
            .join(format!("booklet-vault-{}-{}", std::process::id(), unique))
            .join("Vault");

        let book = vault.join("Book");
        std::fs::create_dir_all(book.join("Section")).unwrap();
        std::fs::write(book.join("booklet.json"), r##"{ "color": "#3C5240", "shelf": "Work" }"##)
            .unwrap();
        std::fs::write(book.join("Top Note.md"), "# Top Note\n").unwrap();
        std::fs::write(book.join("Section/Deep Note.md"), "# Deep\n").unwrap();

        vault
    }

    /// The rows the app actually renders: the vault is not a row, so its books
    /// start at depth 0.
    fn rows_of(vault: &Vault) -> Vec<(String, u32, String, bool)> {
        let mut out = Vec::new();
        vault.append_book_rows(&mut out);
        out.into_iter().map(|row| (row.title, row.depth, row.kind, row.expanded)).collect()
    }

    #[test]
    fn an_unopened_vault_has_no_rows() {
        let path = fixture();
        let vault = Vault::new(path.clone());

        // Books are only read once the vault is opened.
        assert!(rows_of(&vault).is_empty());

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn hydrate_opens_marked_folders_and_reads_children() {
        let path = fixture();
        let expanded = HashSet::from([
            path.clone(),
            path.join("Book"),
            path.join("Book/Section"),
        ]);

        let mut vault = Vault::new(path.clone());
        vault.hydrate(&expanded);

        // Books are roots; directories sort before files, so Section (and its
        // note) precede Top Note.
        assert_eq!(
            rows_of(&vault),
            [
                ("Book".into(), 0, "book".into(), true),
                ("Section".into(), 1, "section".into(), true),
                ("Deep Note".into(), 2, "note".into(), false),
                ("Top Note".into(), 1, "note".into(), false),
            ]
        );

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn book_color_comes_from_binding() {
        let path = fixture();
        let expanded = HashSet::from([path.clone()]);

        let mut vault = Vault::new(path.clone());
        vault.hydrate(&expanded);
        let mut rows = Vec::new();
        vault.append_rows(0, &mut rows);

        let book = rows.iter().find(|row| row.kind == "book").unwrap();
        assert_eq!(book.color, "#3C5240");

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn vault_of_finds_the_containing_vault() {
        let vaults = vec![PathBuf::from("/notes/personal"), PathBuf::from("/work/notes")];

        assert_eq!(
            vault_of(&vaults, Path::new("/work/notes/Book/Note.md")),
            Some(Path::new("/work/notes"))
        );
        assert_eq!(vault_of(&vaults, Path::new("/elsewhere/Note.md")), None);
    }

    #[test]
    fn books_in_reports_counts_and_defaults() {
        let path = fixture();
        std::fs::create_dir_all(path.join("Plain")).unwrap(); // a book without booklet.json
        std::fs::write(path.join("Plain/A.md"), "# A\n").unwrap();

        let mut books = books_in(&path);
        books.sort_by(|left, right| left.title.cmp(&right.title));

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Book");
        assert_eq!(books[0].shelf, "Work");
        assert_eq!(books[0].note_count, 2); // Top Note + Section/Deep Note
        assert_eq!(books[1].title, "Plain");
        assert_eq!(books[1].shelf, DEFAULT_SHELF);
        assert_eq!(books[1].note_count, 1);

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn breadcrumb_with_paths_pairs_each_segment_with_its_path() {
        let vault = Path::new("/v");
        let note = Path::new("/v/Book/Section/Note.md");
        let crumbs = breadcrumb_with_paths(vault, note);

        let names: Vec<&str> = crumbs.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["Book", "Section", "Note"]); // last is the title, not the file

        // Each segment carries the absolute path to reveal in the tree, and the
        // last is the note's own path.
        assert_eq!(crumbs[0].1, PathBuf::from("/v/Book"));
        assert_eq!(crumbs[1].1, PathBuf::from("/v/Book/Section"));
        assert_eq!(crumbs[2].1, PathBuf::from("/v/Book/Section/Note.md"));
    }
}
