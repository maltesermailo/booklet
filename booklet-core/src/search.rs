//! Full-text search across a vault.
//!
//! An on-demand scan, like the backlink scan and for the same reason: at
//! personal scale reading the files is fast enough, and a persistent index is
//! a cost to pay once there is something to measure.

use crate::is_markdown;
use crate::links::snippet_around;
use std::path::{Path, PathBuf};

/// Enough to find what you meant; past this, refine the words instead.
const MAX_HITS: usize = 40;

/// A note whose text contains the query.
pub struct Hit {
    pub path: PathBuf,
    pub title: String,
    pub snippet: String,
}

/// Every note in `vault` whose text contains `query`, case-insensitively, with a
/// snippet around the first match. Results are ordered by title so the list does
/// not shuffle between searches for the same word.
pub fn search(vault: &Path, query: &str) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut hits = Vec::new();

    for entry in walkdir::WalkDir::new(vault).into_iter().flatten() {
        let path = entry.path();
        if !is_markdown(path) {
            continue;
        }

        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some(position) = find_ignoring_case(&text, &needle) else {
            continue;
        };

        hits.push(Hit {
            path: path.to_path_buf(),
            title: path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default(),
            snippet: snippet_around(&text, position),
        });
    }

    hits.sort_by(|a, b| a.title.cmp(&b.title));
    hits.truncate(MAX_HITS);

    hits
}

/// The byte offset of the first case-insensitive match of `needle` (already
/// lowercased) in `haystack`.
///
/// Deliberately not `haystack.to_lowercase().find(needle)`: lowercasing can
/// change a string's length, so an offset into the lowercased text would not
/// always point at the same place in the original — and the snippet is cut from
/// the original.
fn find_ignoring_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .char_indices()
        .map(|(index, _)| index)
        .find(|index| starts_with_ignoring_case(&haystack[*index..], needle))
}

fn starts_with_ignoring_case(text: &str, needle: &str) -> bool {
    let mut lowered = text.chars().flat_map(char::to_lowercase);

    needle.chars().all(|wanted| lowered.next() == Some(wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn fixture() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let vault =
            std::env::temp_dir().join(format!("booklet-search-{}-{}", std::process::id(), unique));
        std::fs::create_dir_all(&vault).unwrap();

        std::fs::write(vault.join("Kernel.md"), "# Kernel\n\nThe SERIAL rig is wired up.\n").unwrap();
        std::fs::write(vault.join("Serial.md"), "# Serial\n\nA serial console note.\n").unwrap();
        std::fs::write(vault.join("Other.md"), "# Other\n\nNothing to see.\n").unwrap();
        std::fs::write(vault.join("notes.txt"), "serial, but not a note\n").unwrap();

        vault
    }

    #[test]
    fn finds_notes_containing_the_query_ignoring_case() {
        let vault = fixture();

        let titles: Vec<String> = search(&vault, "SeRiAl").into_iter().map(|hit| hit.title).collect();

        // Sorted by title; notes.txt is not markdown, so it is not a note.
        assert_eq!(titles, ["Kernel", "Serial"]);

        std::fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn a_hit_carries_a_snippet_around_the_match() {
        let vault = fixture();

        let hits = search(&vault, "serial");
        let kernel = hits.iter().find(|hit| hit.title == "Kernel").unwrap();

        // The snippet comes from the original text, so its case survives.
        assert!(kernel.snippet.contains("The SERIAL rig is wired up."));
        assert_eq!(kernel.path, vault.join("Kernel.md"));

        std::fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        let vault = fixture();

        assert!(search(&vault, "   ").is_empty());

        std::fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn a_word_nobody_wrote_finds_nothing() {
        let vault = fixture();

        assert!(search(&vault, "wayland").is_empty());

        std::fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn matching_past_a_multi_byte_character_keeps_the_snippet_intact() {
        let vault = fixture();
        std::fs::write(vault.join("Umlaut.md"), "# Umlaut\n\nÜber die Schlüssel im Gerät.\n")
            .unwrap();

        let hits = search(&vault, "SCHLÜSSEL");
        let hit = hits.iter().find(|hit| hit.title == "Umlaut").unwrap();

        // The offset must index the original text: cutting on a byte that is
        // mid-character would panic rather than merely read oddly.
        assert!(hit.snippet.contains("Über die Schlüssel im Gerät."));

        std::fs::remove_dir_all(&vault).unwrap();
    }
}
