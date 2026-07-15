# Booklet — Roadmap

**Direction:** build a Qt-free core **library** that owns notes and syncing,
and grow it incrementally with the app layered on top. The QtQuick/qtbridge
binary stays a thin frontend that delegates to the library; every domain
capability lands in the library first (with unit tests, no Qt needed to run
them) and is wired into the app as it becomes available.

## Why a library first

- The domain logic (vault model, block parsing, links, sync/conflict rules)
  has nothing to do with Qt and is far easier to test without it.
- The scaffold currently couples that logic directly to qtbridge
  (`library.rs` / `note.rs` / `links.rs` are `#[qobject]` types). Pulling the
  logic into `booklet-core` leaves the app with thin adapters that translate
  between QML and the library.
- Sync is the largest piece and the most valuable to have covered by tests
  before any UI depends on it.

## Foundation status (from the M0 build bring-up)

- Builds and launches with Qt 6.11.1 at `/Volumes/Daten/Qt/6.11.1/macos`
  (`QMAKE`, `PATH`, `DYLD_FRAMEWORK_PATH` pointed there).
- qtbridge 0.2 API findings already applied: `#[qsignal]` receivers must be
  `&mut self`; `#[qslot]`/`#[qsignal]` are consumed by `#[qobject]` and must
  not be imported; plain helper methods may stay in the `#[qobject]` impl.
- **Open item:** QML component registration — `Main.qml` fails with
  `TreePane is not a type`. `include_bytes_qml` only exposes files under
  `qrc:/`; the `booklet` module still needs a discoverable `qmldir`. Resolve
  when the app frontend is next touched (M4), not before.

## M1 — `booklet-core`: notes domain (Qt-free, incremental)

New crate `booklet-core` in the workspace holding the note logic, moved out of
the qtbridge types. Each step ships with unit tests and a thin app adapter.

- [ ] **1a — Crate + vault model.** Move the vault scan and flattened tree out
      of `src/library.rs` into `booklet-core` (plain structs/functions, no
      qtbridge). App keeps a `#[qobject]` adapter that calls into it.
- [ ] **1b — Block parsing.** Move the pulldown-cmark block/byte-range logic
      out of `src/note.rs`. Unit-test block boundaries and `commit_block`
      splice/re-parse round-trips on fixture notes.
- [ ] **1c — Links/backlinks.** Move the `[[..]]` scan out of `src/links.rs`.
      Unit-test resolution by file stem and alias handling.
- [ ] **Exit:** all note behavior lives in `booklet-core` with tests; the app
      builds against it; behavior unchanged from today.

## M2 — `booklet-core`: syncing (Qt-free, incremental)

Sync as a library module first, tested without the UI. Constraints from
CLAUDE.md: plain markdown stays the local source of truth, offline-first,
per-file sync unit, last-write-wins with conflict copies, no CRDT.

- [ ] **2a — Local change tracking.** Detect created/modified/moved/renamed
      notes and `folio.json`; prefer move/rename over delete+create. Pure
      library state + tests.
- [ ] **2b — Conflict rules.** Last-write-wins per file; losing copy preserved
      as `Note (conflict YYYY-MM-DD).md`. Unit-test the resolution matrix.
- [ ] **2c — `booklet-sync-server` crate.** Minimal HTTPS API: per-device
      token auth, per-file get/put with version metadata, list/since.
- [ ] **2d — Client sync engine.** Wire 2a/2b against the server; offline
      reconcile on reconnect. Integration-test client↔server without the UI.
- [ ] **Exit:** two clients reconcile through the server with conflict copies,
      all verified by tests before any UI wiring.

Recommend a short design note + one clarifying pass before 2c — the server
shape is underspecified in the current docs.

## M3 — App adapters follow CLAUDE.md idioms

With the logic in the library, the qtbridge layer becomes thin adapters.
Apply the idiom fixes here (they were the original scaffold-rewrite items):

- [ ] **Never swallow errors (Rule 7).** Surface failures the library returns
      via signals + minimal QML — notably failed save/read
      (`save_failed(reason)` / `read_failed`), which the scaffold currently
      drops with `let _ = fs::write` and `unwrap_or_default`. *(User decision:
      signals + UI, not just propagate/log.)*
- [ ] **No abbreviations (Rule 6).** Any short bindings surviving into the
      adapters get descriptive names.
- [ ] **Blank lines between logical sections** in adapter methods.
- [ ] **Efficiency.** Hoist the `rewrite_wikilinks` regex to a `LazyLock`
      (once it moves to the display/adapter layer).
- [ ] **Re-entrancy.** Confirm signals fire after the `&mut` borrow settles —
      `NoteEditor::open` emits while borrowed; verify against qtbridge's
      documented borrow-caching rule so QML handlers can't re-borrow-panic.

## M4 — Complete the core reading/writing UX

- [ ] **QML module registration** — resolve the open M0 item (`qmldir` for the
      `booklet` module) so `TreePane`/`EditorView`/`Marginalia` load.
- [ ] **Fonts** — wire `FontLoader` for EB Garamond / Alegreya Sans /
      Spectral / JetBrains Mono.
- [ ] **Create-note-on-unresolved-link** — consume `link_unresolved(title)`.
- [ ] **Books / shelf view** — build a consumer for `Library.books()`.
- [ ] **Quick switcher (⌘K)** — flatten the note list, `Popup` + filter.

## M5 — Polish & hardening

- [ ] Persistent link index if the on-demand scan gets slow (measure first).
- [ ] Graduate the hot lists to `qtbridge::QListModel` once the trait API
      settles.
