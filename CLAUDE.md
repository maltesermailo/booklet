# CLAUDE.md — Booklet

## What this project is

Booklet is a personal note library, native desktop app. Obsidian-style shell
(dark flat file tree, block-based live-preview editor where clicking any
rendered block reveals its markdown source) with a bookish reading surface
(night paper, parchment ink, brass and ember accents, EB Garamond / Spectral
typography). Notes are plain markdown files on disk.

Stack: **QtQuick (QML) frontend, pure-Rust backend via Qt Bridges for Rust**
(`qtbridge`, currently public beta). No C++ code in this repository.

## Base features

1. **Library tree** — vault of books (top-level folders, each with a
   `folio.json` carrying binding color and shelf label) containing sections
   (folders, nesting without limit) containing notes (`.md`).
2. **Block editor** — Obsidian-style live preview. Rendered markdown per
   block; click swaps that block to raw source; leaving the block commits,
   re-parses, saves. The title is block 0 and behaves like any block.
3. **Wiki-links and backlinks** — `[[Title]]` / `[[Title|alias]]` resolve by
   file stem; the Marginalia panel lists notes referencing the current one.
4. **Server sync (base feature)** — the vault synchronizes against a
   self-hosted server. Requirements:
   - Plain markdown on disk stays the source of truth locally; sync must
     never require a proprietary container format.
   - Sync unit is the individual note file plus book metadata
     (`folio.json`); moves/renames are tracked, not treated as delete+create
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

- `src/library.rs` — vault scan and the **flattened tree**. qtbridge 0.2 has
  no tree-model trait, so Rust owns the hierarchy and exposes only the
  visible rows (each with `depth`); expand/collapse is a slot that
  recomputes the list. Do not attempt a QAbstractItemModel from Rust.
- `src/note.rs` — block parsing with pulldown-cmark; blocks are byte ranges
  into the source string. `commit_block` splices the edited slice back,
  re-parses, writes to disk.
- `src/links.rs` — on-demand `[[..]]` scan for backlinks. Fine at personal
  scale; a persistent index is a later optimization, not now.
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
- `include_bytes_qml!` in `src/main.rs` ships the non-Main QML files via the
  Qt resource system; its signature is the most likely API to shift between
  beta releases — verify against the `color_palette` example on upgrade.
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
