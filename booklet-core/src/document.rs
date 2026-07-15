//! The block editor's document model.
//!
//! A note is parsed into top-level blocks addressed by byte ranges into the
//! source string (Obsidian-style live preview: each block renders as markdown
//! and reveals its raw source when clicked). Editing a block splices the new
//! text back into the source, re-parses, and saves. The title is simply block 0
//! (`# Title`). Rendering the `[[wiki-link]]` scheme is left to the caller, so
//! this stays app-agnostic.

use crate::is_markdown;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// One top-level block of a note.
pub struct Block {
    pub index: usize,
    pub kind: String,   // "heading" | "paragraph" | "code" | "quote" | "list"
    pub source: String, // raw markdown slice, shown when the block is clicked
}

/// An open note: its source and the byte range of each top-level block.
pub struct Document {
    path: PathBuf,
    source: String,
    ranges: Vec<Range<usize>>,
}

impl Document {
    /// Opens the note at `path`, reading and parsing it into blocks.
    pub fn open(path: PathBuf) -> io::Result<Document> {
        let source = std::fs::read_to_string(&path)?;
        let ranges = parse_ranges(&source);

        Ok(Document { path, source, ranges })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The note's title — its file stem (block 0 is the `# Title` heading).
    pub fn title(&self) -> String {
        self.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The note's markdown, as it stands.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Takes the editor's text without touching the disk. Kept apart from
    /// [`Document::write`] so the caller can decide when to pay for I/O.
    pub fn set_source(&mut self, source: &str) {
        self.source = source.to_string();
        self.ranges = parse_ranges(&self.source);
    }

    /// Writes the note as it stands.
    pub fn write(&self) -> io::Result<()> {
        std::fs::write(&self.path, &self.source)
    }

    /// Replaces the note's markdown and writes it to disk.
    pub fn save(&mut self, source: &str) -> io::Result<()> {
        self.set_source(source);

        self.write()
    }

    /// The current blocks, each with its raw source and classified kind.
    pub fn blocks(&self) -> Vec<Block> {
        self.ranges
            .iter()
            .enumerate()
            .map(|(index, range)| {
                let source = self.source[range.clone()].trim_end().to_string();
                Block { index, kind: classify(&source), source }
            })
            .collect()
    }

    /// Replaces block `index` with `new_source`, re-parses, and saves. A block
    /// index that no longer exists is a no-op.
    pub fn commit_block(&mut self, index: usize, new_source: &str) -> io::Result<()> {
        let Some(range) = self.ranges.get(index).cloned() else {
            return Ok(());
        };

        let trimmed = new_source.trim_end();
        // Keep the trailing newline that separated this block from the next; the
        // final block has none.
        let replacement = if range.end >= self.source.len() {
            trimmed.to_string()
        } else {
            format!("{trimmed}\n")
        };
        self.source.replace_range(range, &replacement);
        self.ranges = parse_ranges(&self.source);

        std::fs::write(&self.path, &self.source)
    }
}

/// Creates a note titled `title` in `folder`, seeded with its title heading,
/// and returns its path. Used when a wiki-link points at a note that does not
/// exist yet, and when a note is created from the tree.
pub fn create_note(folder: &Path, title: &str) -> io::Result<PathBuf> {
    let path = folder.join(format!("{title}.md"));

    // Never write over a note that is already there: the title can come from
    // someone typing it, so this is reachable.
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("a note called '{title}' is already here"),
        ));
    }

    std::fs::write(&path, format!("# {title}\n"))?;

    Ok(path)
}

/// Finds the first note within `vault` whose file stem matches `title` — the
/// resolution used for `[[wiki-links]]`. A vault is self-contained (as in
/// Obsidian), so this never looks outside it.
pub fn find_note(vault: &Path, title: &str) -> Option<PathBuf> {
    walkdir::WalkDir::new(vault)
        .into_iter()
        .flatten()
        .map(|entry| entry.into_path())
        .find(|path| {
            is_markdown(path) && path.file_stem().is_some_and(|stem| stem.to_string_lossy() == title)
        })
}

/// Computes the byte range of each top-level block. This is what makes
/// per-block click-to-reveal-source possible.
fn parse_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let parser = Parser::new_ext(source, Options::all()).into_offset_iter();

    let mut depth = 0usize;
    let mut start = 0usize;
    for (event, range) in parser {
        match event {
            Event::Start(tag) if is_block(&tag) => {
                if depth == 0 {
                    start = range.start;
                }
                depth += 1;
            }
            Event::End(tag) if is_block_end(&tag) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    ranges.push(start..range.end);
                }
            }
            _ => {}
        }
    }

    ranges
}

fn is_block(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::CodeBlock(..)
            | Tag::BlockQuote(..)
            | Tag::List(..)
            | Tag::Table(..)
    )
}

fn is_block_end(tag: &TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Paragraph
            | TagEnd::Heading(..)
            | TagEnd::CodeBlock
            | TagEnd::BlockQuote(..)
            | TagEnd::List(..)
            | TagEnd::Table
    )
}

fn classify(source: &str) -> String {
    let start = source.trim_start();
    if start.starts_with('#') {
        "heading"
    } else if start.starts_with("```") || start.starts_with("~~~") {
        "code"
    } else if start.starts_with('>') {
        "quote"
    } else if start.starts_with('-') || start.starts_with('*') || start.starts_with("1.") {
        "list"
    } else {
        "paragraph"
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    const NOTE: &str = "# Title\n\nA paragraph.\n\n- item 1\n- item 2\n\n> a quote\n";

    fn temp_dir() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("booklet-doc-{}-{}", std::process::id(), unique))
    }

    #[test]
    fn parses_top_level_blocks_with_kinds() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Note.md");
        std::fs::write(&path, NOTE).unwrap();

        let document = Document::open(path).unwrap();
        let blocks = document.blocks();

        let kinds: Vec<&str> = blocks.iter().map(|block| block.kind.as_str()).collect();
        assert_eq!(kinds, ["heading", "paragraph", "list", "quote"]);
        assert_eq!(blocks[0].source, "# Title");
        assert_eq!(blocks[1].source, "A paragraph.");
        assert_eq!(blocks[2].source, "- item 1\n- item 2");
        assert_eq!(document.title(), "Note");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_replaces_the_note_and_writes_it() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Note.md");
        std::fs::write(&path, NOTE).unwrap();

        let mut document = Document::open(path.clone()).unwrap();
        document.save("# Retitled\n\nRewritten whole.\n").unwrap();

        assert_eq!(document.source(), "# Retitled\n\nRewritten whole.\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Retitled\n\nRewritten whole.\n");
        // The blocks follow the new source.
        assert_eq!(document.blocks()[0].source, "# Retitled");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn commit_block_splices_reparses_and_saves() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Note.md");
        std::fs::write(&path, NOTE).unwrap();

        let mut document = Document::open(path.clone()).unwrap();
        document.commit_block(1, "An edited paragraph.").unwrap();

        // In-memory re-parse reflects the edit...
        assert_eq!(document.blocks()[1].source, "An edited paragraph.");
        // ...and it is written to disk, leaving the other blocks intact.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("An edited paragraph."));
        assert!(on_disk.contains("# Title"));
        assert!(on_disk.contains("> a quote"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_note_writes_a_titled_note_that_opens_cleanly() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();

        let path = create_note(&dir, "New Note").unwrap();

        assert_eq!(path, dir.join("New Note.md"));
        // Block 0 is the title heading, like any other note.
        let document = Document::open(path).unwrap();
        assert_eq!(document.title(), "New Note");
        assert_eq!(document.blocks()[0].source, "# New Note");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_note_matches_stem_within_the_vault_only() {
        let root = temp_dir();
        let vault_a = root.join("A");
        let vault_b = root.join("B");
        std::fs::create_dir_all(vault_a.join("Sub")).unwrap();
        std::fs::create_dir_all(&vault_b).unwrap();
        std::fs::write(vault_a.join("Sub/Alpha.md"), "# Alpha\n").unwrap();
        std::fs::write(vault_b.join("Beta.md"), "# Beta\n").unwrap();

        // Nested notes resolve...
        assert_eq!(find_note(&vault_a, "Alpha"), Some(vault_a.join("Sub/Alpha.md")));
        // ...but a note in another vault never does: vaults are self-contained.
        assert_eq!(find_note(&vault_a, "Beta"), None);
        assert_eq!(find_note(&vault_a, "Missing"), None);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
