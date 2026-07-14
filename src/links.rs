//! Wiki-link scan for the Marginalia (backlinks) panel.
//!
//! Deliberately simple for a personal-scale vault: scan every .md file for
//! [[Title]] occurrences on demand. If the vault grows large, replace with a
//! persistent index rebuilt on save (and later SQLite FTS5 for full-text
//! quick-switcher search).

use qtbridge::{qobject, qslot};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
struct Backlink {
    source_id: String,
    source_title: String,
    snippet: String,
}

#[derive(Default)]
pub struct Backlinks {
    vault: PathBuf,
}

#[qobject(Singleton)]
impl Backlinks {
    #[qslot]
    fn set_vault(&mut self, path: String) {
        self.vault = PathBuf::from(path);
    }

    /// All notes containing [[title]] (or [[title|alias]]), as JSON.
    #[qslot]
    fn for_note(&self, title: String) -> String {
        if title.is_empty() {
            return "[]".into();
        }
        let needle_plain = format!("[[{title}]]");
        let needle_alias = format!("[[{title}|");
        let mut out = Vec::new();

        for entry in walkdir::WalkDir::new(&self.vault).into_iter().flatten() {
            let p = entry.path();
            if !p.extension().is_some_and(|x| x == "md") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            let hit = text.find(&needle_plain).or_else(|| text.find(&needle_alias));
            let Some(pos) = hit else { continue };

            let source_title = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if source_title == title {
                continue; // a note is not its own marginalia
            }

            out.push(Backlink {
                source_id: p
                    .strip_prefix(&self.vault)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned(),
                source_title,
                snippet: snippet_around(&text, pos),
            });
        }
        serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
    }
}

/// A short context window around the match, ellipsized, on char boundaries.
fn snippet_around(text: &str, pos: usize) -> String {
    let mut start = pos.saturating_sub(60);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (pos + 90).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    let core = text[start..end].replace('\n', " ");
    format!("…{}…", core.trim())
}
