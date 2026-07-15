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

Booklet keeps its list of vaults in `~/.config/booklet/vaults.json`. **One vault
is active at a time** — the tree shows its books — and the vault menu in the
topbar switches between them, adds one (folder picker), or removes one. Passing
a path on the command line also adds it:

    cargo run -- /absolute/path/to/a/vault
    # first run with the bundled sample:
    cargo run -- "$(pwd)/vault"

Set `BOOKLET_CONFIG` to store the vault list elsewhere — so a separate profile,
or another app built on `booklet-core`, can keep its own list:

    BOOKLET_CONFIG=/path/to/vaults.json cargo run

## Vaults and layout

The library is a set of independently-located vaults (the paths listed in
`~/.config/booklet/vaults.json`), one of them active at a time. Each vault is a
folder of books:

    ~/Notes/Personal/                  <- a vault (one entry in the config)
    ├── Systems Engineering/           <- top-level folder in a vault = book
    │   ├── booklet.json               <- { "color": "#3C5240", "shelf": "Work and Study" }
    │   └── Kernel/                    <- any folder below a book = section
    │       └── Debugging/             <- sections nest without limit
    │           └── Pixel 7/
    │               └── KGDB setup.md
    └── Theologie/
        └── ...

Notes are plain markdown. `[[Wiki links]]` (with optional `[[target|alias]]`)
resolve by file stem **within their own vault** — as in Obsidian, a vault is a
self-contained graph and links never cross vaults, which keeps each vault
portable.

## Architecture

- `booklet-core` — Qt-free, unit-tested engine. `Engine` owns a live tree read
  from disk. `Vault` → `Book` → `Section`* → `Note` share a `Folder` trait; each
  folder owns its own `expanded` flag and reads children on demand. `config.rs`
  persists the vault paths and the open folders; `Engine::refresh()` reconciles
  with disk. qtbridge 0.2 has no tree-model trait, so Rust hands QML only the
  visible rows (each with a `depth`). `src/library.rs` is a thin qtbridge adapter
  that drives the engine.
- `booklet-core::document` — the **block editor** model: `Document` parses a
  note into top-level blocks with byte ranges, `commit_block` splices/reparses/
  saves, `find_note` resolves wiki-links across vaults. `src/note.rs` is a thin
  qtbridge adapter: QML renders each block as markdown (Qt renders markdown
  natively), clicking a block swaps in a TextArea with its raw source, leaving
  it commits. The adapter renders the `booklet://` wiki-link scheme.
- `booklet-core::links` — on-demand `[[..]]` scan feeding the Marginalia panel,
  scoped to the note's own vault (`vault::vault_of` maps a note to its vault).
  `src/links.rs` is a thin qtbridge adapter.
- Rust <-> QML uses slots/signals with JSON payloads: the most stable surface
  of the beta API. Graduating the two hot lists to `qtbridge::QListModel`
  is a contained refactor once the trait API settles.

## Beta caveats (qtbridge 0.2)

- The QML module lives in `src/booklet/`; `src/main.rs` registers each file
  with `include_bytes_qml!("booklet/<file>", "qt/qml")` (including the `qmldir`
  and `Main.qml`), then loads `qrc:/qt/qml/booklet/Main.qml` after
  `add_import_path("qrc:/qt/qml")`. This follows the `color_palette` example in
  <https://github.com/qt/qtbridge-rust>; `include_bytes_qml!`'s signature is the
  most likely thing to move between beta releases, so re-verify on upgrade.
- Objects live in `Rc<RefCell<_>>`. A re-entrant call chain
  (QML -> Rust -> QML -> same object) panics on the borrow. Emit signals
  after state changes settle; do not call back into the same backend from
  inside a `&mut self` slot.
- For async work (file watcher, indexing), use `QmlMethodInvoker` to touch
  the UI thread from tokio — see the `host_monitor` example.

## Themes

Two built-in themes share one token vocabulary in `src/booklet/Theme.qml`:
`night` (warm reading room, default) and `atlas` (Celestial Atlas: void
blue-black, starlight ink, gilt accents, comet links). Toggle at runtime
with Ctrl+T. Adding a theme = one palette object + one branch in `Theme.p`.

## Fonts

EB Garamond, Alegreya Sans, Spectral, and JetBrains Mono are **bundled** in
`src/booklet/fonts/` — nothing to install. `build.rs` compiles them into a Qt
binary resource with `rcc` (`src/booklet/fonts.qrc`), `main.rs` registers the
blob, and `Theme.qml` loads the families with `FontLoader`.

They deliberately do *not* go through `include_bytes_qml!`: that macro expands
every byte into a token literal, which does not scale to megabytes of font data.

All four are SIL Open Font License 1.1 — see [COPYRIGHT.md](COPYRIGHT.md).

## Keys

- `⌘K` — quick switcher: find a note in the active vault.
- `⌘L` — the shelf: a full-window library of book spines, grouped by shelf
  label, sized by note count. Pick one to jump to it in the tree (`Esc` leaves).
- `⌘T` / `⌘W` — open a note in a new tab / close the current tab.
- `⌘\` / `⌘⇧\` — hide or show the sidebar / the Marginalia panel.

The `night` / `atlas` theme toggle has no shortcut; it moves to the Settings
screen, which is not built yet — so `atlas` is currently unreachable.

## Behaviors worth knowing

- **Live tree.** A file watcher (`notify`) watches every configured vault; a
  change on disk schedules `Library.refresh` on the Qt thread via
  `QmlMethodInvoker`, so the tree tracks the filesystem without polling.
- **Create on link.** Following a `[[wiki-link]]` to a note that does not exist
  yet creates it beside the note you are reading (same folder) and opens it —
  as Obsidian does.
