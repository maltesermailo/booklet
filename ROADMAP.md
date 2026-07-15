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

- Builds and launches with Qt 6.11.1 installed via Homebrew
  (`/opt/homebrew/bin/qmake` on PATH; no env exports needed).
- qtbridge 0.2 API findings already applied: `#[qsignal]` receivers must be
  `&mut self`; `#[qslot]`/`#[qsignal]` are consumed by `#[qobject]` and must
  not be imported; plain helper methods may stay in the `#[qobject]` impl.
- **Resolved:** QML module registration. The QML now lives in `src/booklet/`,
  each file registered at `qrc:/qt/qml/booklet/` with a real `qmldir`
  (`module booklet` + components); loaded via `add_import_path("qrc:/qt/qml")`
  + `load_qml_from_file(...)`. The app renders end-to-end (verified offscreen:
  QML loads with no errors and startup persists the vault list through the
  engine). Also fixed a latent `font.pixelSize: 13.5` (int expected).

## M1 — `booklet-core`: notes domain (Qt-free, incremental)

New crate `booklet-core` in the workspace holding the note logic, moved out of
the qtbridge types. Each step ships with unit tests and a thin app adapter.

- [x] **1a — Crate + vault model.** `booklet-core` owns a live tree read from
      disk: `Vault` → `Book` → `Section`* → `Note` behind a `Folder` trait, each
      folder holding its own `expanded` flag (no shared id set). Multiple
      **independently-located vaults**; ids are absolute paths. `config.rs`
      persists vault paths + open folders; `Engine::refresh()` reconciles with
      disk. `src/library.rs` is a thin adapter. **Pending:** a file watcher to
      call `refresh()` automatically (needs `QmlMethodInvoker`, next step).
- [x] **1b — Block parsing.** Moved to `booklet-core::document` (`Document`,
      `Block`, `find_note`), Qt-free and unit-tested (block boundaries/kinds,
      `commit_block` splice/reparse/save round-trip, cross-vault `find_note`).
      `src/note.rs` is a thin adapter that renders the `booklet://` scheme
      app-side. `find_note` resolves wiki-links within the open note's own
      vault (see 1c).
- [x] **1c — Links/backlinks.** Moved to `booklet-core::links`
      (`backlinks_to`), Qt-free and unit-tested (plain + alias links, self- and
      other-vault exclusion, snippets). Links and backlinks are **vault-scoped**
      (as in Obsidian) via `vault::vault_of`, keeping each vault self-contained
      for sync. `src/links.rs` is a thin adapter emitting absolute `source_id`s
      so clicking a backlink opens the note.
- [x] **Exit:** all note behavior lives in `booklet-core` with tests (19
      passing); the app builds against it and is a thin adapter layer;
      workspace clippy is clean.

## M2 — `booklet-core`: syncing (Qt-free, incremental)

Sync as a library module first, tested without the UI. Constraints from
CLAUDE.md: plain markdown stays the local source of truth, offline-first,
per-file sync unit, last-write-wins with conflict copies, no CRDT.

- [ ] **2a — Local change tracking.** Detect created/modified/moved/renamed
      notes and `booklet.json`; prefer move/rename over delete+create. Pure
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

- [x] **QML module registration** — QML in `src/booklet/` with a `qmldir`
      module, registered under `qrc:/qt/qml/booklet/` and loaded via
      `load_qml_from_file`. `TreePane`/`EditorView`/`Marginalia` load; app
      renders.
- [x] **Fonts** — the four OFL families are bundled in `src/booklet/fonts/`,
      compiled to a Qt resource by `build.rs` via `rcc` and loaded with
      `FontLoader` in `Theme.qml`. Not via `include_bytes_qml!`: it turns every
      byte into a token literal, which does not scale to 2.5 MB. Attribution in
      `COPYRIGHT.md`.
- [x] **File watcher** — `notify` watches every vault; changes schedule
      `Library.refresh` on the Qt thread via `QmlMethodInvoker` (verified: a file
      created on disk drives a refresh). Undebounced on purpose — `refresh` is
      idempotent, and a leading-edge throttle could drop the final event.
- [x] **Create-note-on-unresolved-link** — following a link to a missing note
      creates it beside the current note (`document::create_note`) and opens it.
      The `link_unresolved` signal is gone: nothing consumed it once the
      behavior landed.
- [x] **Books / shelf view** — `ShelfView.qml`, a full-window mode (⌘L, `Esc` to
      leave). Spines grouped by shelf label, sized by note count, colored by
      binding; picking one calls `Engine::reveal` to open it in the tree.
- [x] **Quick switcher (⌘K)** — `Engine::notes()` lists every note with a
      breadcrumb; `QuickSwitcher.qml` filters and opens. Navigation spans all
      vaults (unlike links, which stay in one vault).
- [x] **Qt Quick Controls style pinned to Basic** — the native macOS style
      refuses `background` customization, which Booklet relies on throughout.

## M5 — Finish the UI

M4 made the app *work*; it is not finished. There is currently **not one button,
menu, dialog, or splitter in the app** — every affordance is a keyboard shortcut
or a click on a row. Several Rust slots have no UI at all (`remove_vault` is
unreachable; `add_vault` only via a CLI argument).

**Design source of truth: `design/reference.html`.** Its token vocabulary maps
1:1 to `Theme.qml`; when this milestone and the reference disagree, the reference
wins **unless noted below**.

Deliberate deviations from the reference, and decisions that resolve conflicts
in it — do not "fix" these back:

- **New note prompts for a name** (inline in the tree) and seeds the heading from
  it. The reference instead writes `Untitled.md` and opens block 0 "so typing the
  title is the first act"; we take the prompt so the filename — the note's real
  identity — never drifts from the title.
- **The theme toggle loses its shortcut** and lives only in Settings. The
  reference binds `⌘T` to *new tab*, which collided with the existing theme
  toggle; tabs win the key (`⌘T` new tab, `⌘W` close).
- **`link_unresolved` no longer exists.** The reference's star-map notes reach
  for that hook, but M4 deleted the signal when create-on-link landed. Unresolved
  dots call `NoteEditor.open_by_title(title)`, which already creates and opens.
- **The sync pill ships inert.** Its status needs the sync engine (M2); until
  then it renders the offline state and gains real status in M2.

### 5a — One active vault (model change)

The reference's tree shows **books at depth 0 — there are no vault rows**.
Booklet adopts the Obsidian shape: one active vault at a time, switched from a
menu.

- [x] `Engine` holds an **active vault**; `visible_rows` emits its books at depth
      0 via `Vault::append_book_rows` — `kind: "vault"` rows are gone from the
      UI. Non-active vaults are still built, so folders left open inside them
      survive a switch (guarded by a test).
- [x] Persist the active vault in `config.rs` (`{ vaults, active, expanded }`).
      `load` falls back to the first vault if the stored active one is gone;
      `add_vault` makes the first vault active; `remove_vault` falls back.
- [x] Vault menu in the topbar (`TopBar.qml`) — switch, **add vault** via
      `FolderDialog`, **remove vault**. Built the topbar shell to the reference's
      spec; 5b fills in breadcrumb, ⌘K hint and sync pill.
- [x] **Knock-on resolved:** the shelf (`books()`) and quick switcher (`notes()`)
      now scope to the active vault, and the watcher only watches it. Links and
      backlinks were already scoped to *the note's own* vault via `vault_of`,
      which is strictly better — a note opened from another vault still resolves
      correctly — so they were left alone.

### 5b — Chrome and layout

- [x] **Topbar** (`TopBar.qml`) — wordmark, breadcrumb (`NoteEditor.breadcrumb()`
      → book / sections / note, last segment `--text-bright`), `⌘K` hint, vault
      menu, and the **sync pill**, inert until M2 as agreed.
- [x] **Sidebar icon toolbar** — the reference's five buttons, rendered from its
      SVG path data with `QtQuick.Shapes` (`Icon.qml` / `IconButton.qml`);
      scaling a 24×24 path to 15px reproduces its 1.8 stroke exactly. Search,
      collapse-all (`Engine::collapse_all`) and hide-sidebar work; **new note and
      new section are disabled** until 5d gives them create ops.
- [x] **Tab strip** (`TabStrip.qml`) — active tab fuses with the page; `×`,
      middle-click and `⌘W` close; `+`/`⌘T` open the switcher so the note you
      pick lands in a new tab. UI state only.
- [x] **Hideable panels** — `⌘\` sidebar, `⌘⇧\` Marginalia; the toolbar's
      hide-sidebar button too.
- [x] **Resizable panes** — `SplitView` at the reference's 230/220.
      **Widths are not persisted yet** (QtCore `Settings` needs an
      organizationName that qtbridge does not expose; parking it rather than
      polluting the core config with view state).
- [x] **Status bar** (`StatusBar.qml`) — active vault and note count. Saved state
      joins it in 5e, once the editor reports it.
- [x] Window title tracks the open note; **"Folio" is gone**.
- [x] Theme toggle lost `⌘T` (tabs took it). **Until Settings lands in 5g the
      `atlas` theme is unreachable** — a knowingly accepted gap.

### 5c — Fidelity to reference.html

Gaps found comparing the reference to the current QML:

- [x] Page is **centred, `max-width: 560px`** on 16/14px of space, with the
      reference's 22/26/28/34 padding.
- [x] Tree **indent guides** — each row draws a 1px `--sidebar-line` segment per
      ancestor level; stacked, they read as the reference's nested rules.
- [x] **Book rows** are `--text-bright` + weight 500; sections `--text`, notes
      `--text-soft`, whatever is open bright.
- [x] Marginalia **highlights the linked title** in each snippet — the `[[...]]`
      source is rendered as the text it reads as, on 24% accent blended against
      the card (Qt rich text needs a solid colour).
- [x] Stitch line at `left: 16px`, 5px dashes / 6px gaps. Pane widths (230/220)
      landed with the SplitView in 5b.
- [x] Type faces per the reference: EB Garamond for headings, Spectral for prose
      (15px — the reference's 14.5 is not expressible in Qt's int `pixelSize`),
      JetBrains Mono for code.
- [ ] **Unverified:** how Qt scales a markdown `#` heading from the block's base
      size, so the title may not land on the reference's 26px. Left the existing
      base alone rather than guess without a display — eyeball it and adjust.

### 5d — Library operations (new core work)

- [x] **New note / new section** — `Library.create_note(parent_id, name)` and
      `Library.create_section(parent_id, name)`, the hooks the reference names.
      They land in the selected row's section (a note's section, or the vault
      root with nothing selected), prompt for the name **inline in the tree**,
      and seed the note's heading from it. The toolbar's two buttons are live now,
      plus `⌘N`.
- [x] **Rename** notes and sections, inline in the tree.
      **Decided: links are left alone** — renaming never edits other notes. A
      `[[link]]` to the old title stops resolving; 5e's unresolved styling is
      what will make those visible. Note that following one still *creates* a
      note at the old name.
- [x] **Delete** → **system Trash** (the `trash` crate), behind a confirm
      dialog, so it stays recoverable from Finder.
- [x] Right-click context menus in the tree (new note / new section / rename /
      delete).
- [x] **Bug fixed on the way:** `document::create_note` used `fs::write`, which
      overwrites. Harmless while only wiki-links called it (`find_note` had
      already proven the note absent), but reachable — and destructive — once a
      person types the name. It now refuses to clobber, and `rename` likewise
      refuses to replace an existing file (`fs::rename` would have).

### 5e — Editor

- [ ] **Back / forward** history — following a wiki-link currently takes you away
      with no way back.
- [ ] **Note rename** — the filename is the note's identity and can drift from
      the `# Title` heading.
- [ ] **Save indicator** — saving is silent-on-commit today.
- [ ] **Preview / Source toggle** at the page's top-right — a whole-note source
      mode alongside the existing per-block click-to-edit.
- [ ] **Unresolved link styling** — `--text-soft` + dashed underline. Needs core:
      each `[[link]]` must be *resolved* per block, not rewritten blindly.
- [ ] **Note meta line** — "● PIXEL 7 · EDITED TODAY" above the title; needs the
      section name and mtime from core.

### 5f — Marginalia and the star map

- [ ] **Star map ("sightlines")** at the top of the pane on `--code-bg`: the open
      note is the centre star (`--link` ring, `--text-bright` core); incoming refs
      are `--text-soft` dots, outgoing links `--link` dots, unresolved hollow
      `--text-dim` on dashed sightlines. Lines 0.7px at ~55%, labels 7px mono
      uppercase. **Radial spread — angle by index with slight radius jitter, no
      force simulation.** Cap ~10 dots; the card list carries the overflow.
      Clicking a dot opens that note; unresolved dots create it via
      `open_by_title`.
- [ ] Needs one new slot: **`NoteEditor.outgoing_links()`**. Everything else is
      data the pane already has.
- [ ] **Tags** — `#tag` parsing in core, pills in the pane. No tag concept exists
      today.

### 5h — Vault picker (welcome screen)

The reference's welcome screen: a centred 520px column on `--sidebar` — book
mark, wordmark, version (`CARGO_PKG_VERSION`), one primary button, a *recently
opened* card, an actions card, and a language row.

- [ ] **Config model change.** Vault entries stop being plain paths and become
      `{ path, color, last_opened }`. **One list serves both** the picker
      (sorted most-recent-first, capped at 8) and the topbar menu — the
      reference's `recent.json` and `remove_recent()` collapse into the vault
      list we already persist, and `remove_vault` already means "forget it, do
      not touch the disk". `last_opened` is epoch seconds (QML formats the
      relative time); `color` is auto-assigned from the binding palette on add.
- [ ] **New core:** `recent_vaults()` (the capped, sorted view) and
      `create_vault(path)`. Opening a vault bumps its `last_opened`.
- [ ] **Picker QML** — mark (56px `Shape`), wordmark, version, recents rows (dot
      = the vault's colour, name in the display face, path in mono `--text-dim`,
      relative time, × removes from the list only), actions card: **Create** /
      **Open folder** / **Sign in**.
- [ ] **Quick start** — the app's *only* filled button: creates a starter vault
      (a default location, one book, one note) and opens it, so a first run lands
      somewhere instead of an empty picker.
- [ ] **Shown when there is no vault to reopen** — a bare start otherwise
      reopens the vault you were last in. Reachable any time from the vault menu
      ("Open another vault…").
- [ ] **Sign in** ships inert until the sync engine (M2), like the sync pill.

Deviations from the reference here, decided with the user:

- **Strings are English**, though the reference's picker is written in German.
  The language selector is built but **inert**; real i18n (`qsTr` + Qt
  translations, and checking whether qtbridge even exposes `QTranslator`) is a
  later milestone of its own.
- **`Library.open_vault` does not exist** — the reference reaches for it, but the
  multi-vault work replaced it with `add_vault` + `set_active`. Same drift as its
  `link_unresolved` note.

### 5g — Beyond

- [ ] **Full-text search** across the vault (the ⌘K switcher matches titles only).
      Starts as an on-demand scan — CLAUDE.md defers a persistent index until
      measured.
- [ ] **Book metadata editing** — binding color and shelf label are hand-edited
      in `booklet.json` today; needs core write support.
- [ ] **Settings screen** — vault list, config location, and the **theme picker**
      (its only home now that `⌘T` belongs to tabs).

Explicitly **not** in this milestone: block add/delete/reorder, tree filter
field, drag-and-drop moves, split view.

## M6 — Polish & hardening

- [ ] Persistent link index if the on-demand scan gets slow (measure first).
- [ ] Graduate the hot lists to `qtbridge::QListModel` once the trait API
      settles.
