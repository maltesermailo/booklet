# CLAUDE.md — Booklet

## What this project is

Booklet is a personal note library, native desktop app. Obsidian-style shell
(dark flat file tree, block-based live-preview editor where clicking any
rendered block reveals its markdown source) with a bookish reading surface
(night paper, parchment ink, brass and ember accents, EB Garamond / Spectral
typography). Notes are plain markdown files on disk.

Stack: **QtQuick (QML) frontend, Rust backend via Qt Bridges for Rust**
(`qtbridge`, currently public beta).

**C++ is allowed only where qtbridge cannot reach.** The backend stays Rust; the
one sanctioned exception is a `QSyntaxHighlighter` shim for the editor's live
preview (`src/cpp/`), because highlighting requires attaching to
`TextEdit.textDocument` and qtbridge 0.2 exposes no text-document types at all
(its Qt surface is `qstring`/`qvariant`/`qobject`/`qjson*`/`qlist`/`qmetatype`/
`qguiapplication`/`qqmlapplicationengine`). Do not reach for C++ for anything
else without asking — every other feature so far has been reachable from Rust.

## Base features

1. **Library tree** — a configurable set of vaults (the list persisted in
   `~/.config/booklet/vaults.json`). **One vault is active at a time** (as in
   Obsidian); the tree shows *its* books as the roots — there is no vault row —
   and the topbar menu switches vaults. A start with no vault to reopen shows
   the vault picker (`VaultPicker.qml`); otherwise the app goes straight back to
   the vault you were last in. Each vault holds books (its top-level
   folders, each with a `booklet.json` carrying binding color and shelf label)
   containing sections (folders, nesting without limit) containing notes
   (`.md`). A book's binding is edited from the tree (right-click → Binding…),
   which writes that same `booklet.json` while keeping any keys the app does not
   know — it is a plain file the user may have edited. The shelf and the quick
   switcher scope to the active vault (the switcher matches titles *and* note
   text); links and backlinks scope to the note's own vault.
2. **Live preview editor** — the pane *is* the note: one always-editable
   surface over the whole markdown, as in Obsidian. There is no mode to enter
   and no block to click. The C++ highlighter (`src/cpp/`) styles it as you
   type — a heading takes its face immediately — and shows the syntax markers
   only on the line holding the caret, collapsing them to zero width elsewhere
   so the text reflows as if they were not written. **Clicking a wiki-link on a
   line you are not editing follows it** — its markers are collapsed, so it is
   rendered text, and rendered text navigates (as in Obsidian). On the line
   holding the caret the markers are showing and a plain click stays the
   caret's; ⌘+click follows a link there too. The pre-click caret line is
   captured in the tap handler's `onPressedChanged`, because the editor moves
   the caret to the click on press — by the time the tap completes, every click
   looks like it landed on the caret's own line. Typing reaches Rust on every
   keystroke (`set_source`) but the disk only on a pause (`flush`), and
   `open`/`close` flush first so switching notes cannot drop edits.
   *(This replaces the earlier click-a-block-to-reveal-source design, which the
   design reference still depicts as `.srcblock`.)*
3. **Wiki-links and backlinks** — `[[Title]]` / `[[Title|alias]]` resolve by
   file stem; the Marginalia panel lists notes referencing the current one.
   Both are **scoped to the note's own vault**: a vault is a self-contained
   graph and links never cross vaults (as in Obsidian). This keeps a vault
   portable, which the per-vault sync depends on.
4. **Server sync (base feature)** — the vault synchronizes against a
   self-hosted server. Requirements:
   - Plain markdown on disk stays the source of truth locally; sync must
     never require a proprietary container format.
   - Sync unit is the individual note file plus book metadata
     (`booklet.json`); moves/renames are tracked, not treated as delete+create
     when avoidable.
   - Offline-first: full functionality without a connection; sync reconciles
     on reconnect.
   - Conflict strategy for v1: last-write-wins per file with the losing
     version preserved as a conflict copy next to the note
     (`Note (conflict 2026-07-15).md`). CRDT/merge-based syncing is
     explicitly out of scope until there is a demonstrated need — do not
     build toward it speculatively.
   - Transport: HTTPS to a small self-hosted server; authentication via a
     per-device token. Server implementation lives in a separate crate in
     this workspace when work on it starts (`booklet-sync-server`).
   - Sync engine runs off the UI thread; UI is notified via
     `QmlMethodInvoker` (see qtbridge `host_monitor` example).

## Architecture notes (read before changing code)

- `booklet-core` — Qt-free, unit-tested engine. `Engine` (`engine.rs`) owns a
  live tree read from disk (disk is the source of truth) and controls
  persistence. It holds every configured vault but renders only the **active**
  one: `visible_rows` emits that vault's books at depth 0 via
  `Vault::append_book_rows`. Non-active vaults are still built so the folders you
  left open in them survive a switch. `vault.rs` holds the node types behind a
  shared `Folder` trait — `Vault` → `Book` → `Section`* → `Note` — where each
  folder **owns its own `expanded` flag** and reads children from disk when
  opened (so expansion is per-object, never a shared id-keyed set). `config.rs`
  persists the vaults, the **active** one, the expanded-folder paths, the
  editor's reading size, and the theme (`~/.config/booklet/vaults.json`, an
  object `{ "vaults": [{ "path", "color", "last_opened" }, ...], "active": ...,
  "expanded": [...], "editor_font_size": ..., "theme": ... }`). `last_opened` is
  epoch **milliseconds** — seconds tie when you click through the picker, and a
  tie makes the "recently opened" list fall back to alphabetical. `VaultEntry`
  hand-writes `Deserialize` so a **`vaults` list of bare path strings still
  loads**: that is what every config written before the picker holds, and
  dropping it would delete somebody's library list. Keep that path working.
  The engine keeps the theme's *name* without validating it: naming the themes
  is the UI's business, and `Theme.qml` falls back on its own for a name it does
  not know. A vault's colour and `last_opened` are **not on disk**, so anything
  rebuilding the tree must carry them across (`rebuild_with` does) — the file
  watcher calls `refresh` on every write, and a rebuild from paths alone would
  wipe them as you type. `Engine::refresh()` rebuilds from disk preserving open
  folders — driven by the file watcher via `QmlMethodInvoker`. `src/library.rs` is a thin qtbridge
  adapter that drives the `Engine` and serializes its rows for QML. qtbridge 0.2
  has no tree-model trait, so Rust owns the hierarchy and exposes only visible
  rows (each with `depth`); expand/collapse is a slot. Do not attempt a
  QAbstractItemModel from Rust.
- `src/cpp/markdown_highlighter.{h,cpp}` — the live preview. A
  `QSyntaxHighlighter` attached to the editing block's `TextEdit.textDocument`:
  it dims markdown's syntax markers and styles the text as it will render, so
  `# Test` reads as a heading while you type it. **The only C++ in the repo**,
  because the highlighter must reach the text document and qtbridge exposes no
  path to it. `build.rs` runs `moc` and compiles it using
  `qtbridge-build-utils`' `QtInstallation` (which knows the macOS framework
  layout); `main.rs` calls `booklet_register_highlighter()` to register the QML
  type before loading QML.
- `booklet-core::document` — the note model: `Document::open` reads a note,
  `set_source` takes the editor's text in memory, `write` puts it on disk (kept
  apart so the caller decides when to pay for I/O), `create_note` seeds a new
  one, `find_note` resolves `[[wiki-links]]` by file stem within a vault. It
  still parses top-level blocks (byte ranges via pulldown-cmark), which nothing
  currently consumes since the editor became one surface. Qt-free and
  unit-tested; `src/note.rs` is a thin qtbridge adapter over it.
- `booklet-core::links` — wiki-links both ways, scoped to a single vault:
  `backlinks_to` is an on-demand `[[..]]` scan over the vault, `outgoing_links`
  reads a note's own text and resolves each link (`target: None` = not written
  yet). `booklet_core::vault::vault_of` maps a note path to its vault. Fine at
  personal scale; a persistent index is a later optimization, not now.
  `src/links.rs` is a thin qtbridge adapter; `NoteEditor.outgoing_links()`
  serves the star map off the editor's in-memory text.
- **`Theme.qml` owns how big the chrome draws.** Every size in the reference was
  designed against 100%, so components write `Theme.px(13)` rather than `13` and
  the whole interface scales from one setting (`ui_scale` in the config).
  `Theme.gap(n)` is the room *between* things (`density`), and `Theme.row(n)` is
  anything sized to hold type — a tree row, a tab, a button — which scales by
  **both**, because text that grows inside a row that does not spills out of it.
  Never write a raw pixel size for type or a row; the two knobs only work if
  everything goes through them. Divider lines, `Icon`'s 24×24 path grid, and the
  star map's dots are deliberately exempt.
- **One motion vocabulary.** `Theme.quick`/`Theme.gentle` + `Theme.easing`, and
  `Theme.radiusSmall`/`radiusCard`. Selections warm into their highlight with a
  `Behavior on color`; menus and modals fade and scale in. Use these rather than
  new durations, so hovering a tree row and hovering a button feel like the same
  app.
- **Menus are `AppMenu` + `AppMenuItem`**, never stock `Menu`/`MenuItem`: the
  Basic style's menu is a square grey box that belongs to no theme, and the app
  runs under Basic.
- **`ScrollView` measures its content by the child's *implicit* size.** The
  editor's sheet has an explicit `height`, so `contentHeight` must be set by hand
  (`EditorView.qml`); left unset it sits at -1 and a note of any length reports
  as fitting — the scrollbar never appears and the wheel does nothing. Same trap
  as `contentWidth`, which was already pinned.
- **Full-window modes vs modals.** The shelf (⌘L) and the vault picker are
  full-window: the reading layout hides while one is up, and only one may be up.
  Settings (⌘,) and the quick switcher (⌘K) are modals over it. A modal closes
  itself, so it carries no flag in `Main.qml`.
- `booklet-core::search` — full-text search over the active vault, on demand
  like the backlink scan and for the same reason. It does **not** lowercase the
  haystack and search that: lowercasing can change a string's length, so the
  offset would not always point at the same place in the original — and the
  snippet is cut from the original. Shares `links::snippet_around`.
- `booklet-core::tags` — `#tag` parsing. A tag is `#` followed by a letter at a
  word start, which is what tells it from a `# Heading`, a URL `#anchor` and a
  `#3C5240`; code fences are skipped. Read-only: nothing indexes or filters by
  tag yet.
- Rust <-> QML crosses via **slots and signals with JSON payloads**. This is
  deliberate: the beta's most stable surface. Do not migrate to
  `qtbridge::QListModel` or `qproperty` without checking the current trait
  API against the examples in qt/qtbridge-rust first.
- The QML module name equals the crate name (`import booklet`). Renaming the
  crate breaks every QML import.

## qtbridge beta constraints

- Objects live in `Rc<RefCell<_>>`. Re-entrant call chains
  (QML → Rust → QML → same object) panic on the borrow. Never call back into
  the same backend object from inside a `&mut self` slot; emit signals after
  state has settled.
- The QML module lives in `src/booklet/` (a dir named after the crate, so
  `import booklet` resolves). `src/main.rs` registers each file — including the
  `qmldir` and `Main.qml` — with `include_bytes_qml!("booklet/<file>",
  "qt/qml")`, then `add_import_path("qrc:/qt/qml")` and
  `load_qml_from_file("qrc:/qt/qml/booklet/Main.qml")`. Paths are relative to
  `src/`, so keep the QML under `src/booklet/` (no `..`). This mirrors the
  `color_palette` example; `include_bytes_qml!`'s signature is the most likely
  API to shift between beta releases — re-verify against it on upgrade.
- Requires Qt >= 6.10, `qmake` in PATH (`QMAKE=qmake6` on Debian/Ubuntu).
  macOS arm64 is an experimental qtbridge target; when something fails only
  on macOS, cross-check on Linux before debugging deeply.

## Commands

- Build/run: `cargo run -- /absolute/path/to/vault`
  (sample vault: `cargo run -- "$(pwd)/vault"`)
- Docs of the bridge: `cargo doc --features serde_json --no-deps -p qtbridge`

---

# Development rules

The following rules are binding for all code written in this repository.

You are an experienced software developer with a deep understanding of
software architecture, language idioms, and long-term maintainability. You
write code that a colleague can understand six months from now without
asking questions.

## Guiding Principle

Simplicity is the primary quality criterion. The best code represents the
most direct path to the solution — no detours, no stockpiling, no
cleverness. If two solutions are functionally equivalent, always choose the
shorter and simpler one.

## Binding Rules

### 1. KISS — Keep It Simple
- Write the most direct code that solves the problem.
- Prefer the language's standard constructs over clever tricks.
- Avoid design patterns when a simple function is enough.
- Measure simplicity by how quickly an unfamiliar developer understands the
  code — not by how elegant it seems to you.

### 2. Abstractions Only After Asking
- Do not introduce abstractions (interfaces, base classes, generic types,
  wrappers, indirection layers) on your own initiative.
- If you believe an abstraction is necessary: **Stop and ask.** In doing so,
  state (a) the concrete problem the abstraction solves, (b) the costs (more
  indirection, more code), (c) the alternative without the abstraction.
- Duplicated code is cheaper than the wrong abstraction. Wait for the third
  use case (Rule of Three) before proposing a generalization.

### 3. DRY — With Good Judgment
- Extract repetitions only once the same *domain logic* (not just
  similar-looking code) occurs at least three times.
- Two pieces of code that happen to look identical but exist for different
  domain reasons may remain duplicated.

### 4. YAGNI — You Aren't Gonna Need It
- Implement only what is required right now.
- No configurability, extension points, or parameters "for later."
- No speculative error handling for cases that cannot occur.

### 5. Comments Explain the Why
- Comment decisions, trade-offs, non-obvious constraints, and workarounds —
  never *what* the code does.
- Exception: For complex algorithms (e.g. non-trivial math, state machines,
  parsers), step-by-step comments are allowed and encouraged.
- If you find yourself wanting to write a "what" comment, that is a signal:
  rename the variable or function instead.

### 6. Clean Code Fundamentals
- Descriptive names: The name says what the thing is or does. No
  abbreviations except established ones (id, url, db).
- Small functions with a single responsibility. Early returns instead of
  deep nesting.
- No magic numbers — named constants whenever the meaning is not obvious.
- Consistency with the existing codebase style takes precedence over
  personal preferences.

### 7. Error Handling
- Handle errors explicitly or propagate them explicitly — never swallow them
  silently.
- Error handling only where a meaningful response is possible. No defensive
  programming against impossible states.

## Way of Working

1. **Understand before acting:** If the requirement is ambiguous, ask
   exactly one precise clarifying question before writing code. Do not
   guess.
2. **Minimal changes:** Change only what is necessary for the task. No
   unsolicited refactorings, reformatting, or "improvements" to someone
   else's code.
3. **Justified decisions:** When choosing between variants, state the
   decision and the reason in one sentence.
4. **Honesty:** If you are unsure about an API, a behavior, or a version,
   say so explicitly instead of inventing plausible-sounding code.
5. **Output format:** For changes to existing code, show only the modified
   sections with sufficient context, not entire files — unless the user
   requests it.

## Self-Check Before Every Response

Check your code against these questions. If any is answered with No, revise:
- Is this the simplest solution that fully satisfies the requirement?
- Have I introduced an abstraction without asking?
- Does every comment explain a Why (or a step of a complex algorithm)?
- Does the code contain anything that is not needed right now?
