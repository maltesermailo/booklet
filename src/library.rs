//! Vault scanning and the flattened tree model.
//!
//! qtbridge 0.2 has no tree-model trait (only QListModel/QTableModel), so the
//! tree is flattened in Rust: we keep the full hierarchy implicit on disk and
//! emit only the *visible* rows, each carrying a `depth` for indentation.
//! Expand/collapse is a slot call that recomputes the list. This also handles
//! infinite nesting for free — it is just recursion depth.

use qtbridge::{qobject, qsignal, qslot};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone)]
pub struct Row {
    pub id: String,   // vault-relative path; unique, stable identifier
    pub title: String,
    pub depth: u32,
    pub kind: String, // "book" | "section" | "note"
    pub color: String, // binding color hex; books only
    pub expanded: bool,
    pub has_children: bool,
}

#[derive(Serialize)]
pub struct BookInfo {
    pub id: String,
    pub title: String,
    pub color: String,
    pub shelf: String,
    pub note_count: usize,
}

#[derive(Default)]
pub struct Library {
    vault: PathBuf,
    expanded: HashSet<String>,
}

#[qobject(Singleton)]
impl Library {
    #[qslot]
    fn open_vault(&mut self, path: String) {
        self.vault = PathBuf::from(path);
        self.expanded.clear();
        self.tree_changed();
    }

    #[qslot]
    fn vault_path(&self) -> String {
        self.vault.to_string_lossy().into_owned()
    }

    /// Currently visible rows as JSON.
    /// TODO(beta -> stable): graduate to qtbridge::QListModel once the trait
    /// API settles, so toggles become fine-grained row inserts/removals
    /// instead of a full rebuild. See minimal_app in qt/qtbridge-rust.
    #[qslot]
    fn visible_rows(&self) -> String {
        let mut rows = Vec::new();
        self.walk(&self.vault.clone(), 0, &mut rows);
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
    }

    #[qslot]
    fn toggle(&mut self, id: String) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        self.tree_changed();
    }

    /// Books for the shelf view: title, binding, shelf label, note count
    /// (note count drives spine width/height).
    #[qslot]
    fn books(&self) -> String {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.vault) else {
            return "[]".into();
        };
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let (color, shelf) = book_meta(&p);
            let note_count = walkdir::WalkDir::new(&p)
                .into_iter()
                .flatten()
                .filter(|x| x.path().extension().is_some_and(|ext| ext == "md"))
                .count();
            out.push(BookInfo {
                id: rel_id(&self.vault, &p),
                title: e.file_name().to_string_lossy().into_owned(),
                color,
                shelf,
                note_count,
            });
        }
        serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
    }

    #[qsignal]
    fn tree_changed(&self);

    fn walk(&self, dir: &Path, depth: u32, out: &mut Vec<Row>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        // Directories first, then files; alphabetical within each group.
        entries.sort_by_key(|e| (e.path().is_file(), e.file_name()));

        for e in entries {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "folio.json" {
                continue;
            }
            let id = rel_id(&self.vault, &p);

            if p.is_dir() {
                let kind = if depth == 0 { "book" } else { "section" };
                let expanded = self.expanded.contains(&id);
                let color = if depth == 0 { book_meta(&p).0 } else { String::new() };
                out.push(Row {
                    id: id.clone(),
                    title: name,
                    depth,
                    kind: kind.into(),
                    color,
                    expanded,
                    has_children: true,
                });
                if expanded {
                    // Infinite nesting is just this recursion.
                    self.walk(&p, depth + 1, out);
                }
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(Row {
                    id,
                    title: p
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or(name),
                    depth,
                    kind: "note".into(),
                    color: String::new(),
                    expanded: false,
                    has_children: false,
                });
            }
        }
    }
}

fn rel_id(vault: &Path, p: &Path) -> String {
    p.strip_prefix(vault)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Reads folio.json in a book folder: { "color": "#3C5240", "shelf": "Work and Study" }
fn book_meta(book_dir: &Path) -> (String, String) {
    let v: Option<serde_json::Value> = std::fs::read_to_string(book_dir.join("folio.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    match v {
        Some(v) => (
            v["color"].as_str().unwrap_or("#4A5560").to_string(),
            v["shelf"].as_str().unwrap_or("Library").to_string(),
        ),
        None => ("#4A5560".into(), "Library".into()),
    }
}
