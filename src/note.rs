//! The block editor backend.
//!
//! Obsidian-style live preview: the note is parsed into top-level blocks with
//! byte ranges into the source string. QML shows rendered markdown per block
//! (Qt renders markdown natively via TextFormat.MarkdownText); clicking a
//! block swaps it to a raw TextArea holding exactly that block's source
//! slice; leaving the block commits the slice back, we re-parse and save.
//! The note title is simply block 0 (`# Title`), so clicking it reveals its
//! markdown like any other block.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use qtbridge::qobject;
use serde::Serialize;
use std::ops::Range;
use std::path::PathBuf;

#[derive(Serialize)]
struct Block {
    index: usize,
    kind: String,    // "heading" | "paragraph" | "code" | "quote" | "list"
    source: String,  // raw markdown slice — shown when the block is clicked
    display: String, // markdown with [[x]] -> [x](folio://x) for rendering
}

#[derive(Default)]
pub struct NoteEditor {
    vault: PathBuf,
    path: PathBuf,   // absolute path of the open note
    id: String,      // vault-relative id
    source: String,
    ranges: Vec<Range<usize>>, // byte range of each top-level block
}

#[qobject(Singleton)]
impl NoteEditor {
    #[qslot]
    fn set_vault(&mut self, path: String) {
        self.vault = PathBuf::from(path);
    }

    #[qslot]
    fn open(&mut self, id: String) {
        self.path = self.vault.join(&id);
        self.id = id;
        self.source = std::fs::read_to_string(&self.path).unwrap_or_default();
        self.reparse();
        self.note_opened(self.id.clone(), self.title());
        self.blocks_changed();
    }

    /// Follow a wiki-link: find the first note whose file stem matches.
    #[qslot]
    fn open_by_title(&mut self, title: String) {
        let hit = walkdir::WalkDir::new(&self.vault)
            .into_iter()
            .flatten()
            .find(|e| {
                e.path().extension().is_some_and(|x| x == "md")
                    && e.path()
                        .file_stem()
                        .is_some_and(|s| s.to_string_lossy() == title)
            })
            .map(|e| e.path().to_path_buf());
        match hit {
            Some(p) => {
                let id = p
                    .strip_prefix(&self.vault)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .into_owned();
                self.open(id);
            }
            None => self.link_unresolved(title),
        }
    }

    #[qslot]
    fn current_id(&self) -> String {
        self.id.clone()
    }

    #[qslot]
    fn title(&self) -> String {
        self.path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    #[qslot]
    fn blocks(&self) -> String {
        let blocks: Vec<Block> = self
            .ranges
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let src = self.source[r.clone()].trim_end().to_string();
                Block {
                    index: i,
                    kind: classify(&src),
                    display: rewrite_wikilinks(&src),
                    source: src,
                }
            })
            .collect();
        serde_json::to_string(&blocks).unwrap_or_else(|_| "[]".into())
    }

    /// Called by QML when the user leaves an edited block.
    #[qslot]
    fn commit_block(&mut self, index: usize, new_source: String) {
        let Some(r) = self.ranges.get(index).cloned() else { return };
        let trimmed = new_source.trim_end();
        let replacement = if r.end >= self.source.len() {
            trimmed.to_string()
        } else {
            format!("{trimmed}\n")
        };
        self.source.replace_range(r, &replacement);
        self.reparse();
        let _ = std::fs::write(&self.path, &self.source); // save-on-commit
        self.blocks_changed();
    }

    #[qsignal]
    fn blocks_changed(&mut self);

    #[qsignal]
    fn note_opened(&mut self, id: String, title: String);

    /// Emitted when a wiki-link points at a note that does not exist yet.
    /// Hook note creation here later.
    #[qsignal]
    fn link_unresolved(&mut self, title: String);

    /// Compute top-level block boundaries with byte offsets — this is what
    /// makes per-block click-to-reveal-source possible.
    fn reparse(&mut self) {
        self.ranges.clear();
        let parser = Parser::new_ext(&self.source, Options::all()).into_offset_iter();
        let mut depth = 0usize;
        let mut start = 0usize;
        for (ev, range) in parser {
            match ev {
                Event::Start(tag) if is_block(&tag) => {
                    if depth == 0 {
                        start = range.start;
                    }
                    depth += 1;
                }
                Event::End(tag) if is_block_end(&tag) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        self.ranges.push(start..range.end);
                    }
                }
                _ => {}
            }
        }
    }
}

fn is_block(t: &Tag) -> bool {
    matches!(
        t,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::CodeBlock(..)
            | Tag::BlockQuote(..)
            | Tag::List(..)
            | Tag::Table(..)
    )
}

fn is_block_end(t: &TagEnd) -> bool {
    matches!(
        t,
        TagEnd::Paragraph
            | TagEnd::Heading(..)
            | TagEnd::CodeBlock
            | TagEnd::BlockQuote(..)
            | TagEnd::List(..)
            | TagEnd::Table
    )
}

fn classify(src: &str) -> String {
    let s = src.trim_start();
    if s.starts_with('#') {
        "heading"
    } else if s.starts_with("```") || s.starts_with("~~~") {
        "code"
    } else if s.starts_with('>') {
        "quote"
    } else if s.starts_with('-') || s.starts_with('*') || s.starts_with("1.") {
        "list"
    } else {
        "paragraph"
    }
    .into()
}

/// [[Title]] -> [Title](folio://Title) so MarkdownText renders a link that
/// QML can intercept via onLinkActivated.
fn rewrite_wikilinks(src: &str) -> String {
    let re = regex::Regex::new(r"\[\[([^\]\|]+)(\|[^\]]+)?\]\]").unwrap();
    re.replace_all(src, |c: &regex::Captures| {
        let target = c.get(1).map(|m| m.as_str()).unwrap_or_default();
        let label = c
            .get(2)
            .map(|m| m.as_str().trim_start_matches('|'))
            .unwrap_or(target);
        format!("[{label}](folio://{target})")
    })
    .into_owned()
}
