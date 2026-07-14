# Booklet

A personal note library. Obsidian-style shell — flat dark file tree, block
editor where clicking any rendered block (including the title) reveals its
markdown — with a bookish page: night paper, parchment ink, brass and ember.

Built with QtQuick and a pure-Rust backend via **Qt Bridges for Rust**
(`qtbridge`, public beta).

## Requirements

- Rust >= 1.87 (rustup.rs)
- Qt >= 6.10 with qtbase, qtbase-private, qtdeclarative
- A C++ toolchain; `qmake` reachable in PATH

### macOS (Apple Silicon — experimental target for qtbridge)

Install Qt 6.10+ via the Qt Online Installer (or `brew install qt`, if the
bottled version is >= 6.10). Then make qmake and the frameworks findable:

    # Qt Online Installer default layout:
    export PATH="$HOME/Qt/6.10.1/macos/bin:$PATH"
    export DYLD_FRAMEWORK_PATH="$HOME/Qt/6.10.1/macos/lib:$DYLD_FRAMEWORK_PATH"

    # Homebrew instead:
    export PATH="$(brew --prefix qt)/bin:$PATH"
    export DYLD_FRAMEWORK_PATH="$(brew --prefix qt)/lib:$DYLD_FRAMEWORK_PATH"

Xcode command line tools are enough for the C++ side
(`xcode-select --install`).

Note: macOS arm64 is listed as *experimental* by the qtbridge project. If you
hit linker or runtime oddities on the Mac Studio, cross-check on Linux before
assuming the bug is yours.

### Linux (Debian/Ubuntu)

    sudo apt install qt6-base-dev qt6-declarative-dev qt6-base-private-dev
    export QMAKE=qmake6    # Debian names the binary qmake6

## Run

    cargo run -- /absolute/path/to/vault
    # or try the bundled sample:
    cargo run -- "$(pwd)/vault"

## Vault layout

    vault/
    ├── Systems Engineering/       <- top-level folder = book
    │   ├── folio.json             <- { "color": "#3C5240", "shelf": "Work and Study" }
    │   └── Kernel/                <- any folder below a book = section
    │       └── Debugging/         <- sections nest without limit
    │           └── Pixel 7/
    │               └── KGDB setup.md
    └── Theologie/
        └── ...

Notes are plain markdown. `[[Wiki links]]` (with optional `[[target|alias]]`)
resolve by file stem across the whole vault.

## Architecture

- `src/library.rs` — vault scan + **flattened tree**: qtbridge 0.2 has no
  tree-model trait, so Rust owns the hierarchy and hands QML only the visible
  rows, each with a `depth` for indentation. Expand/collapse is a slot call.
- `src/note.rs` — the **block editor**: pulldown-cmark parses the note into
  top-level blocks with byte ranges. QML renders each block as markdown
  (Qt renders markdown natively); clicking a block swaps in a TextArea with
  that block's raw source; leaving it commits, re-parses, saves.
- `src/links.rs` — on-demand `[[..]]` scan feeding the Marginalia panel.
- Rust <-> QML uses slots/signals with JSON payloads: the most stable surface
  of the beta API. Graduating the two hot lists to `qtbridge::QListModel`
  is a contained refactor once the trait API settles.

## Beta caveats (qtbridge 0.2)

- Verify `include_bytes_qml!` usage in `src/main.rs` against the
  `color_palette` example in <https://github.com/qt/qtbridge-rust> — it is
  the mechanism for shipping the extra QML files (Theme, panes) in the Qt
  resource system, and its signature is the most likely thing to have moved.
- Objects live in `Rc<RefCell<_>>`. A re-entrant call chain
  (QML -> Rust -> QML -> same object) panics on the borrow. Emit signals
  after state changes settle; do not call back into the same backend from
  inside a `&mut self` slot.
- For async work (file watcher, indexing), use `QmlMethodInvoker` to touch
  the UI thread from tokio — see the `host_monitor` example.

## Themes

Two built-in themes share one token vocabulary in `qml/Theme.qml`:
`night` (warm reading room, default) and `atlas` (Celestial Atlas: void
blue-black, starlight ink, gilt accents, comet links). Toggle at runtime
with Ctrl+T. Adding a theme = one palette object + one branch in `Theme.p`.

## Fonts

The theme expects EB Garamond, Alegreya Sans, Spectral, and JetBrains Mono.
Install them system-wide during development; for distribution, load them at
startup with QML FontLoader from bundled resources.

## Roadmap hooks already in place

- `NoteEditor.link_unresolved(title)` signal — wire "create note on click".
- `Library.books()` slot — feeds the shelf/library view (not yet built).
- Quick switcher: flatten note list is trivial from `Library`; add a ⌘K
  Popup and filter in QML or Rust.
