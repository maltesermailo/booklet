//! Backlinks: which notes reference a given note.
//!
//! Deliberately simple for a personal-scale library: scan every markdown file
//! under the configured vaults for `[[Title]]` occurrences on demand. If a
//! library grows large, replace this with a persistent index rebuilt on save.

use crate::is_markdown;
use std::path::{Path, PathBuf};

/// How much context to show either side of the link in a snippet.
const SNIPPET_BEFORE: usize = 60;
const SNIPPET_AFTER: usize = 90;

/// A note that references the target note.
pub struct Backlink {
    pub source: PathBuf,
    pub title: String, // the referencing note's title (its file stem)
    pub snippet: String,
}

/// A `[[wiki-link]]` written in a note.
pub struct OutgoingLink {
    pub title: String,
    /// Where the link leads, or `None` if no note by that title exists yet.
    pub target: Option<PathBuf>,
}

/// The `[[wiki-links]]` in `text`, in the order written, each resolved against
/// `vault`. A title linked more than once appears once: these describe where the
/// note points, not how often.
pub fn outgoing_links(vault: &Path, text: &str) -> Vec<OutgoingLink> {
    let mut links: Vec<OutgoingLink> = Vec::new();
    let mut rest = text;

    while let Some(open) = rest.find("[[") {
        rest = &rest[open + 2..];

        let Some(close) = rest.find("]]") else {
            break;
        };

        let inner = &rest[..close];
        rest = &rest[close + 2..];

        // Stray brackets across a line break are not a link.
        if inner.contains('\n') {
            continue;
        }

        let title = inner.split('|').next().unwrap_or_default().trim();
        if title.is_empty() || links.iter().any(|link| link.title == title) {
            continue;
        }

        links.push(OutgoingLink {
            title: title.to_string(),
            target: crate::document::find_note(vault, title),
        });
    }

    links
}

/// Every note within `vault` that links to `title` via `[[title]]` or
/// `[[title|alias]]`. The target note itself is excluded — a note is not its own
/// marginalia. A vault is self-contained (as in Obsidian), so notes in other
/// vaults are never scanned.
pub fn backlinks_to(vault: &Path, title: &str) -> Vec<Backlink> {
    if title.is_empty() {
        return Vec::new();
    }

    let plain = format!("[[{title}]]");
    let aliased = format!("[[{title}|");
    let mut backlinks = Vec::new();

    for entry in walkdir::WalkDir::new(vault).into_iter().flatten() {
        let path = entry.path();
        if !is_markdown(path) {
            continue;
        }

        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some(position) = text.find(&plain).or_else(|| text.find(&aliased)) else {
            continue;
        };

        let source_title = note_title(path);
        if source_title == title {
            continue;
        }

        backlinks.push(Backlink {
            source: path.to_path_buf(),
            title: source_title,
            snippet: snippet_around(&text, position),
        });
    }

    backlinks
}

fn note_title(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A short context window around the match, ellipsized, on char boundaries.
/// Shared with the full-text search, which cuts its snippets the same way.
pub(crate) fn snippet_around(text: &str, position: usize) -> String {
    let mut start = position.saturating_sub(SNIPPET_BEFORE);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }

    let mut end = (position + SNIPPET_AFTER).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }

    let window = text[start..end].replace('\n', " ");
    format!("…{}…", window.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Two vaults:
    ///   A/ Target.md    — links to itself (must be excluded)
    ///      Refers.md    — plain [[Target]]
    ///      Aliased.md   — [[Target|the target]]
    ///      Unrelated.md — no links
    ///   B/ Outsider.md  — [[Target]], but in another vault
    fn fixture() -> (PathBuf, Vec<PathBuf>) {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("booklet-links-{}-{}", std::process::id(), unique));

        let vault_a = root.join("A");
        let vault_b = root.join("B");
        std::fs::create_dir_all(&vault_a).unwrap();
        std::fs::create_dir_all(&vault_b).unwrap();

        std::fs::write(vault_a.join("Target.md"), "# Target\n\nI mention [[Target]] myself.\n")
            .unwrap();
        std::fs::write(vault_a.join("Refers.md"), "# Refers\n\nSee [[Target]] for more.\n").unwrap();
        std::fs::write(vault_a.join("Aliased.md"), "# Aliased\n\nCheck [[Target|the target]] out.\n")
            .unwrap();
        std::fs::write(vault_a.join("Unrelated.md"), "# Unrelated\n\nNothing here.\n").unwrap();
        std::fs::write(vault_b.join("Outsider.md"), "# Outsider\n\nAlso [[Target]].\n").unwrap();

        (root, vec![vault_a, vault_b])
    }

    #[test]
    fn finds_plain_and_aliased_links_excluding_self_and_other_vaults() {
        let (root, vaults) = fixture();

        let mut titles: Vec<String> =
            backlinks_to(&vaults[0], "Target").into_iter().map(|link| link.title).collect();
        titles.sort();

        // Target itself and Unrelated are out; Outsider lives in vault B and is
        // invisible here — vaults are self-contained.
        assert_eq!(titles, ["Aliased", "Refers"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_vault_only_sees_its_own_notes() {
        let (root, vaults) = fixture();

        let titles: Vec<String> =
            backlinks_to(&vaults[1], "Target").into_iter().map(|link| link.title).collect();

        // Vault B sees only its own referrer, never vault A's.
        assert_eq!(titles, ["Outsider"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn backlink_points_at_the_source_file_with_a_snippet() {
        let (root, vaults) = fixture();

        let links = backlinks_to(&vaults[0], "Target");
        let refers = links.iter().find(|link| link.title == "Refers").unwrap();

        assert_eq!(refers.source, vaults[0].join("Refers.md"));
        assert!(refers.snippet.contains("See [[Target]] for more."));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn empty_title_has_no_backlinks() {
        let (root, vaults) = fixture();

        assert!(backlinks_to(&vaults[0], "").is_empty());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn outgoing_links_resolve_within_the_vault() {
        let (root, vaults) = fixture();
        let text = "Read [[Target]], then [[Refers|this one]], then [[Nowhere]].";

        let links = outgoing_links(&vaults[0], text);

        let titles: Vec<&str> = links.iter().map(|link| link.title.as_str()).collect();
        assert_eq!(titles, ["Target", "Refers", "Nowhere"]);
        assert_eq!(links[0].target, Some(vaults[0].join("Target.md")));
        assert_eq!(links[1].target, Some(vaults[0].join("Refers.md")));
        // Unresolved: the note has not been written yet.
        assert_eq!(links[2].target, None);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn outgoing_links_skip_repeats_and_stray_brackets() {
        let (root, vaults) = fixture();
        let text = "[[Target]] again [[Target]], an array a[[i]\n]j, and [[ ]].";

        let links = outgoing_links(&vaults[0], text);

        let titles: Vec<&str> = links.iter().map(|link| link.title.as_str()).collect();
        assert_eq!(titles, ["Target"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_note_without_links_points_nowhere() {
        let (root, vaults) = fixture();

        assert!(outgoing_links(&vaults[0], "# Plain\n\nNo links here.").is_empty());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
