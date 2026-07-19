//! Markdown → decoration list for the live-preview editor.
//!
//! The editor holds raw Markdown in an editable `QTextDocument`; the C++
//! highlighter styles it in place. Rather than hand-written regexes in C++, we
//! parse here with `pulldown-cmark` (CommonMark + GFM) and emit a flat list of
//! **decorations** — spans the highlighter applies as character formats, block
//! formats, or custom widgets.
//!
//! Offsets are **UTF-16 code units** (what Qt's document positions use), converted
//! from pulldown's UTF-8 byte ranges here, where the source string is in hand.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;
use std::ops::Range;
use std::sync::OnceLock;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

/// One styled span. `kind` names it; the optional fields carry its attributes.
/// All fields are always serialized so the C++ side can read a stable shape.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Decoration {
    /// UTF-16 offset into the document.
    pub start: usize,
    /// UTF-16 length.
    pub len: usize,
    pub kind: &'static str,
    /// Heading level, or list/quote nesting depth; 0 when not applicable.
    pub level: u8,
    /// Link href / wiki-link target / code-block language / image src; empty otherwise.
    pub text: String,
    /// Task checked / ordered list; false otherwise.
    pub flag: bool,
}

impl Decoration {
    fn new(kind: &'static str, start: usize, len: usize) -> Self {
        Decoration { start, len, kind, level: 0, text: String::new(), flag: false }
    }
}

/// The decorations for a note's source, ready to serialize to JSON for the
/// highlighter.
pub fn decorations(source: &str) -> Vec<Decoration> {
    let map = Utf16Map::new(source);
    let mut out = Vec::new();
    // Inline spans whose markers we bracket from their content bounds.
    let mut stack: Vec<Span> = Vec::new();
    // Byte ranges of code (span + block), so the wiki-link scan ignores `[[` there.
    let mut code_ranges: Vec<Range<usize>> = Vec::new();
    // Where the current table began; pulldown's `Start(Table)` range covers only
    // the header row, so the full span is bracketed from `Start` to `End`.
    let mut table_start: Option<usize> = None;

    let parser = Parser::new_ext(source, Options::all()).into_offset_iter();
    for (event, range) in parser {
        match event {
            // --- inline spans (marker-bracketed) ---
            Event::Start(Tag::Emphasis) => stack.push(Span::open("em", range.start)),
            Event::Start(Tag::Strong) => stack.push(Span::open("strong", range.start)),
            Event::Start(Tag::Strikethrough) => stack.push(Span::open("strike", range.start)),
            Event::Start(Tag::Link { dest_url, .. }) => {
                stack.push(Span::open("link", range.start).with_text(dest_url.into_string()));
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                stack.push(Span::open("image", range.start).with_text(dest_url.into_string()));
            }

            Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image) => {
                if let Some(span) = stack.pop() {
                    let elem = span.elem_start..range.end;
                    emit_span(&mut out, &map, span, range.end);
                    // The finished element counts as content of any enclosing span.
                    note_content(&mut stack, elem);
                }
            }

            // --- inline leaves ---
            Event::Code(_) => {
                code_ranges.push(range.clone());
                emit_code_span(&mut out, &map, source, range.clone());
                note_content(&mut stack, range);
            }
            Event::InlineMath(_) => {
                push_simple(&mut out, &map, "math", range.clone());
                note_content(&mut stack, range);
            }
            Event::DisplayMath(_) => push_simple(&mut out, &map, "math_block", range),
            Event::InlineHtml(_) => {
                push_simple(&mut out, &map, "html", range.clone());
                note_content(&mut stack, range);
            }
            Event::Html(_) => push_simple(&mut out, &map, "html_block", range),
            Event::FootnoteReference(_) => {
                push_simple(&mut out, &map, "footnote_ref", range.clone());
                note_content(&mut stack, range);
            }
            Event::TaskListMarker(checked) => {
                let mut deco = span_deco("task", &map, range.start, range.end);
                deco.flag = checked;
                out.push(deco);
            }
            Event::Rule => push_simple(&mut out, &map, "rule", range),
            Event::Text(_) | Event::SoftBreak | Event::HardBreak => note_content(&mut stack, range),

            // --- blocks ---
            Event::Start(Tag::Heading { level, .. }) => {
                let mut deco = span_deco("heading", &map, range.start, range.end);
                deco.level = level as u8;
                out.push(deco);
                emit_heading_marker(&mut out, &map, source, range, level as u8);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                out.push(span_deco("blockquote", &map, range.start, range.end));
                emit_blockquote_markers(&mut out, &map, source, range);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                code_ranges.push(range.clone());
                let mut deco = span_deco("code_block", &map, range.start, range.end);
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.into_string(),
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
                deco.text = lang.clone();
                out.push(deco);
                emit_code_tokens(&mut out, &map, source, range, &lang);
            }
            Event::Start(Tag::Item) => {
                let ordered = source[range.clone()].trim_start().starts_with(|c: char| c.is_ascii_digit());
                let mut deco = span_deco("list_item", &map, range.start, range.end);
                deco.flag = ordered;
                out.push(deco);
                emit_list_marker(&mut out, &map, source, range);
            }
            Event::Start(Tag::Table(_)) => table_start = Some(range.start),
            Event::End(TagEnd::Table) => {
                if let Some(start) = table_start.take() {
                    out.push(span_deco("table", &map, start, range.end));
                }
            }
            _ => {}
        }
    }

    // pulldown 0.12 has no wiki-links; scan for `[[Title]]` / `[[Title|alias]]`
    // ourselves, skipping any that fall inside code.
    scan_wikilinks(&mut out, &map, source, &code_ranges);

    out
}

/// Finds `[[Title]]` / `[[Title|alias]]` spans and emits a wiki-link over the
/// shown text (the alias if present) with the resolvable target, bracketing the
/// `[[` / `]]` (and `Title|`) as markers. Matches inside code are ignored.
fn scan_wikilinks(out: &mut Vec<Decoration>, map: &Utf16Map, source: &str, code: &[Range<usize>]) {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'[' || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }
        let Some(rel) = source[i + 2..].find("]]") else { break };
        let inner_start = i + 2;
        let inner_end = inner_start + rel;
        let end = inner_end + 2;

        let in_code = code.iter().any(|r| r.start <= i && end <= r.end);
        if in_code || inner_end <= inner_start {
            i = end;
            continue;
        }

        let inner = &source[inner_start..inner_end];
        let target = inner.split('|').next().unwrap_or(inner).to_string();
        // Show the alias if there is one; otherwise the title.
        let content_start = inner.find('|').map_or(inner_start, |p| inner_start + p + 1);

        let mut deco = span_deco("wikilink", map, content_start, inner_end);
        deco.text = target;
        out.push(deco);
        out.push(span_deco("marker", map, i, content_start));
        out.push(span_deco("marker", map, inner_end, end));

        i = end;
    }
}

/// An open inline span, tracking the tightest content range seen inside it, so its
/// markers = the element range minus the content.
struct Span {
    kind: &'static str,
    elem_start: usize,
    content_start: Option<usize>,
    content_end: Option<usize>,
    text: String,
}

impl Span {
    fn open(kind: &'static str, start: usize) -> Self {
        Span { kind, elem_start: start, content_start: None, content_end: None, text: String::new() }
    }
    fn with_text(mut self, text: String) -> Self {
        self.text = text;
        self
    }
    fn note(&mut self, range: &std::ops::Range<usize>) {
        self.content_start = Some(self.content_start.map_or(range.start, |s| s.min(range.start)));
        self.content_end = Some(self.content_end.map_or(range.end, |e| e.max(range.end)));
    }
}

/// Records a leaf/child range as content of the innermost open span.
fn note_content(stack: &mut [Span], range: std::ops::Range<usize>) {
    if let Some(top) = stack.last_mut() {
        top.note(&range);
    }
}

/// Emits a finished inline span: the styled content plus a collapsible marker on
/// each side (the delimiters = the element range minus the content).
fn emit_span(out: &mut Vec<Decoration>, map: &Utf16Map, span: Span, elem_end: usize) {
    let content_start = span.content_start.unwrap_or(span.elem_start);
    let content_end = span.content_end.unwrap_or(elem_end);

    let mut deco = span_deco(span.kind, map, content_start, content_end);
    deco.text = span.text;
    out.push(deco);

    if content_start > span.elem_start {
        out.push(span_deco("marker", map, span.elem_start, content_start));
    }
    if elem_end > content_end {
        out.push(span_deco("marker", map, content_end, elem_end));
    }
}

fn emit_code_span(out: &mut Vec<Decoration>, map: &Utf16Map, source: &str, range: std::ops::Range<usize>) {
    let slice = &source[range.clone()];
    let ticks = slice.chars().take_while(|&c| c == '`').count();
    let inner_start = range.start + ticks;
    let inner_end = range.end.saturating_sub(ticks);

    if inner_end > inner_start {
        out.push(span_deco("code", map, inner_start, inner_end));
        out.push(span_deco("marker", map, range.start, inner_start));
        out.push(span_deco("marker", map, inner_end, range.end));
    } else {
        out.push(span_deco("code", map, range.start, range.end));
    }
}

fn emit_heading_marker(out: &mut Vec<Decoration>, map: &Utf16Map, source: &str, range: std::ops::Range<usize>, _level: u8) {
    let slice = &source[range.clone()];
    let trimmed = slice.trim_start();
    if !trimmed.starts_with('#') {
        return; // setext heading: no leading marker
    }
    let leading_ws = slice.len() - trimmed.len();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    let after = &trimmed[hashes..];
    let spaces = after.len() - after.trim_start().len();
    let marker_end = range.start + leading_ws + hashes + spaces;

    out.push(span_deco("marker", map, range.start, marker_end));
}

fn emit_list_marker(out: &mut Vec<Decoration>, map: &Utf16Map, source: &str, range: std::ops::Range<usize>) {
    let slice = &source[range.clone()];
    let leading_ws = slice.len() - slice.trim_start().len();
    let rest = &slice[leading_ws..];

    // "- ", "* ", "+ ", or "12. " / "12) ".
    let marker_len = if rest.starts_with(['-', '*', '+']) {
        1
    } else {
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && rest[digits..].starts_with(['.', ')']) {
            digits + 1
        } else {
            0
        }
    };
    if marker_len == 0 {
        return;
    }
    let after = &rest[marker_len..];
    let spaces = after.len() - after.trim_start().len();
    let end = range.start + leading_ws + marker_len + spaces;

    // A visible bullet (not collapsed like other markers) — a list reads as a
    // list even though we can't insert a real "•".
    out.push(span_deco("list_marker", map, range.start + leading_ws, end));
}

/// A collapsible marker over each line's leading `>` (possibly nested) inside a
/// blockquote, so the quote reads as prose while the caret's line still shows its
/// `>`.
fn emit_blockquote_markers(out: &mut Vec<Decoration>, map: &Utf16Map, source: &str, range: Range<usize>) {
    let mut pos = range.start;
    for line in source[range.clone()].split_inclusive('\n') {
        let bytes = line.as_bytes();
        let ws = line.len() - line.trim_start().len();
        let mut j = ws;
        while j < bytes.len() && bytes[j] == b'>' {
            j += 1;
            if j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
        }
        if j > ws {
            out.push(span_deco("marker", map, pos + ws, pos + j));
        }
        pos += line.len();
    }
}

fn push_simple(out: &mut Vec<Decoration>, map: &Utf16Map, kind: &'static str, range: std::ops::Range<usize>) {
    out.push(span_deco(kind, map, range.start, range.end));
}

/// The bundled syntax definitions, loaded once (the parse is ~tens of ms).
fn syntax_set() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Highlights a fenced code block's body: syntect classifies each token by its
/// TextMate scope, which we fold into a handful of semantic classes the UI colours
/// from its own theme (so highlighting follows the app's light/dark themes rather
/// than a bundled syntect theme). Emits `code_token` decorations carrying the class
/// name in `text`. Unknown languages and indented blocks get no tokens (they stay
/// plain monospace).
fn emit_code_tokens(out: &mut Vec<Decoration>, map: &Utf16Map, source: &str, range: Range<usize>, lang: &str) {
    if lang.is_empty() {
        return;
    }
    let syntaxes = syntax_set();
    let Some(syntax) = syntaxes.find_syntax_by_token(lang) else {
        return;
    };

    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut byte = range.start;
    let mut lines = source[range.clone()].split_inclusive('\n');

    // The opening fence line (```lang) carries no code.
    if let Some(fence) = lines.next() {
        byte += fence.len();
    }

    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            break; // the closing fence
        }

        let ops = state.parse_line(line, syntaxes).unwrap_or_default();
        let mut idx = 0;
        for (offset, op) in ops {
            emit_token(out, map, &stack, byte + idx, byte + offset);
            let _ = stack.apply(&op);
            idx = offset;
        }
        emit_token(out, map, &stack, byte + idx, byte + line.len());

        byte += line.len();
    }
}

/// Emits one `code_token` decoration for `[start, end)` if the current scope maps
/// to a semantic class; a no-op for plain text or an empty range.
fn emit_token(out: &mut Vec<Decoration>, map: &Utf16Map, stack: &ScopeStack, start: usize, end: usize) {
    if end <= start {
        return;
    }
    if let Some(class) = classify(stack) {
        let mut deco = span_deco("code_token", map, start, end);
        deco.text = class.to_string();
        out.push(deco);
    }
}

/// Folds a TextMate scope stack into one of the semantic classes the UI colours.
/// The innermost (last-pushed) scope wins, matched by scope prefix.
fn classify(stack: &ScopeStack) -> Option<&'static str> {
    // (scope prefix, class). Order matters only within one scope: the first
    // matching prefix wins, so more specific prefixes come first.
    const RULES: &[(&str, &str)] = &[
        ("comment", "comment"),
        ("string", "string"),
        ("constant.numeric", "number"),
        ("constant.character", "string"),
        ("keyword", "keyword"),
        ("storage.type", "type"),
        ("storage.modifier", "keyword"),
        ("storage", "keyword"),
        ("entity.name.function", "function"),
        ("support.function", "function"),
        ("variable.function", "function"),
        ("entity.name.type", "type"),
        ("entity.name.class", "type"),
        ("entity.name.namespace", "type"),
        ("support.type", "type"),
        ("support.class", "type"),
        ("constant", "constant"),
    ];

    for scope in stack.as_slice().iter().rev() {
        for (prefix, class) in RULES {
            if Scope::new(prefix).is_ok_and(|target| target.is_prefix_of(*scope)) {
                return Some(class);
            }
        }
    }
    None
}

/// A decoration over a byte range, converted to UTF-16.
fn span_deco(kind: &'static str, map: &Utf16Map, byte_start: usize, byte_end: usize) -> Decoration {
    let start = map.at(byte_start);
    let end = map.at(byte_end);
    Decoration::new(kind, start, end.saturating_sub(start))
}

/// Byte-offset → UTF-16-offset lookup, built once per parse.
struct Utf16Map {
    marks: Vec<(usize, usize)>, // (byte offset, utf16 offset) at each char start, plus the end
}

impl Utf16Map {
    fn new(source: &str) -> Self {
        let mut marks = Vec::with_capacity(source.len() + 1);
        let mut u16 = 0usize;
        for (byte, ch) in source.char_indices() {
            marks.push((byte, u16));
            u16 += ch.len_utf16();
        }
        marks.push((source.len(), u16));
        Utf16Map { marks }
    }

    /// The UTF-16 offset for a byte offset that lies on a char boundary (which
    /// every pulldown range does).
    fn at(&self, byte: usize) -> usize {
        match self.marks.binary_search_by_key(&byte, |&(b, _)| b) {
            Ok(i) => self.marks[i].1,
            // Not on a boundary (shouldn't happen): fall to the mark just before.
            Err(i) => self.marks.get(i.saturating_sub(1)).map_or(0, |&(_, u)| u),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(decos: &'a [Decoration], kind: &str) -> Vec<&'a Decoration> {
        decos.iter().filter(|d| d.kind == kind).collect()
    }

    #[test]
    fn emphasis_brackets_its_markers() {
        let decos = decorations("a *word* b");
        let em = find(&decos, "em");
        assert_eq!(em.len(), 1);
        // "*word*" — content "word" is offsets 3..7, markers the two '*'.
        assert_eq!((em[0].start, em[0].len), (3, 4));
        let markers = find(&decos, "marker");
        assert!(markers.iter().any(|m| m.start == 2 && m.len == 1));
        assert!(markers.iter().any(|m| m.start == 7 && m.len == 1));
    }

    #[test]
    fn strong_uses_two_char_markers() {
        let decos = decorations("**bold**");
        let strong = find(&decos, "strong");
        assert_eq!((strong[0].start, strong[0].len), (2, 4));
        assert!(find(&decos, "marker").iter().any(|m| m.start == 0 && m.len == 2));
    }

    #[test]
    fn nested_emphasis_resolves_both() {
        let decos = decorations("*a **b** c*");
        assert_eq!(find(&decos, "em").len(), 1);
        assert_eq!(find(&decos, "strong").len(), 1);
    }

    #[test]
    fn code_span_keeps_backticks_as_markers() {
        let decos = decorations("x `code` y");
        let code = find(&decos, "code");
        assert_eq!((code[0].start, code[0].len), (3, 4)); // "code"
    }

    #[test]
    fn headings_carry_their_level_and_marker() {
        let decos = decorations("### Title\n");
        let h = find(&decos, "heading");
        assert_eq!(h[0].level, 3);
        // "### " is the marker.
        assert!(find(&decos, "marker").iter().any(|m| m.start == 0 && m.len == 4));
    }

    #[test]
    fn wiki_links_are_distinguished_from_links() {
        let decos = decorations("see [[Note]] and [text](http://x)");
        assert_eq!(find(&decos, "wikilink").len(), 1);
        assert_eq!(find(&decos, "wikilink")[0].text, "Note");
        assert_eq!(find(&decos, "link").len(), 1);
    }

    #[test]
    fn utf16_offsets_account_for_multibyte() {
        // "café" is 5 bytes but 4 UTF-16 units; the emphasis after it must land
        // at the right UTF-16 offset, not the byte offset.
        let decos = decorations("café *x*");
        let em = find(&decos, "em");
        assert_eq!(em[0].start, 6); // "café " = 5 utf16 units + space; '*' at 5, x at 6
    }

    #[test]
    fn task_items_report_checked() {
        let decos = decorations("- [x] done\n- [ ] todo\n");
        let tasks = find(&decos, "task");
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().any(|t| t.flag));
        assert!(tasks.iter().any(|t| !t.flag));
    }

    #[test]
    fn blocks_are_emitted_with_ranges() {
        let decos = decorations("> quote\n\n```rust\nfn x() {}\n```\n\n- item\n");
        assert_eq!(find(&decos, "blockquote").len(), 1);
        let code = find(&decos, "code_block");
        assert_eq!(code.len(), 1);
        assert_eq!(code[0].text, "rust");
        assert_eq!(find(&decos, "list_item").len(), 1);
    }

    #[test]
    fn table_spans_every_row_not_just_the_header() {
        // pulldown's Start(Table) range is only the header row; the deco must
        // bracket Start..End so the widget knows the table's full extent.
        let src = "x\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n";
        let decos = decorations(src);
        let table = find(&decos, "table");
        assert_eq!(table.len(), 1);
        // The covered text runs from the header through the last data row.
        let chars: Vec<char> = src.chars().collect();
        let covered: String = chars[table[0].start..table[0].start + table[0].len].iter().collect();
        assert!(covered.starts_with("| A | B |"));
        assert!(covered.contains("| 3 | 4 |"));
    }

    #[test]
    fn fenced_code_emits_semantic_tokens() {
        let decos = decorations("```rust\nfn main() { let x = 42; }\n```\n");
        let classes: Vec<&str> = find(&decos, "code_token").iter().map(|d| d.text.as_str()).collect();
        // The language keyword, the numeric literal, and the function name should
        // each be classified.
        assert!(classes.contains(&"keyword"), "got: {classes:?}"); // fn / let
        assert!(classes.contains(&"number"), "got: {classes:?}"); // 42
    }

    #[test]
    fn unknown_language_and_indented_get_no_tokens() {
        assert_eq!(find(&decorations("```wingdingscript\nzzz\n```\n"), "code_token").len(), 0);
        assert_eq!(find(&decorations("    indented code\n"), "code_token").len(), 0);
    }

    #[test]
    fn reference_style_links_are_covered_by_the_parser() {
        // Reference links resolve to the same Link event as inline ones, so they
        // need no special handling — pulldown gives us a link deco with the href.
        let decos = decorations("See [the docs][ref].\n\n[ref]: https://example.com\n");
        let links = find(&decos, "link");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].text, "https://example.com");
    }

    #[test]
    fn footnote_reference_and_math_are_emitted() {
        let decos = decorations("A claim.[^1] And $x^2$ inline.\n\n[^1]: note\n");
        assert_eq!(find(&decos, "footnote_ref").len(), 1);
        assert_eq!(find(&decos, "math").len(), 1);
    }
}

