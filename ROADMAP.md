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

## M2 — Syncing (`booklet-core` + two new crates)

Sync as a library module first, tested without the UI — which holds for 2a–2d,
and cannot hold for 2e. Constraints kept from CLAUDE.md: plain markdown stays the
**local** source of truth, offline-first, the sync unit is the note file plus
`booklet.json`.

**The conflict strategy is now merge-based, and CLAUDE.md was amended to match.**
It read "last-write-wins per file … CRDT/merge-based syncing is explicitly out of
scope — do not build toward it speculatively", and this milestone builds squarely
toward it. Decided with the user with the cost on the table: Obsidian merges
markdown with Google's diff-match-patch and only offers conflict files as an
opt-out, and a note that silently loses an edit is worse than one that
occasionally merges clumsily. **Conflict copies did not die — they narrowed** to
the one case no merge can serve (2b).

Everything below was settled in a design pass. Where a decision went against the
recommendation it says so and says why — **those are the ones to revisit first if
this milestone hurts.**

### 2a — Local change tracking

**Done 2026-07-17** in `booklet-core/src/sync.rs` (Qt-free, 7 unit tests, no
consumer yet by design — 2d is where the changeset gets uploaded). `Manifest`
snapshots every synced entity; `Manifest::scan(vault, &previous)` reads the vault
against a prior snapshot and `Manifest::diff(old, new)` produces the `Change`
list. `SyncState` carries `vault_id` / `server_url` / `cursor`, settled now but
unset until publish (2c/2d populate them). Hash is **SHA-256** (`sha2`), so the
digest doubles as 2c's content-address. Two deliberate deltas from the notes
below: mtime is stored at **nanosecond** resolution, not milliseconds — a
millisecond gate missed a same-length in-place edit landing in the same
millisecond (the `last_opened` seconds→millis tie, one rung finer); and the
manifest is written with a **real temp-file+rename**, since `config.rs::save`
turned out to use a plain `fs::write` despite the claim below.

- [x] **A note is its path; a move is inferred from content.** A delete and a
      create in one sync window whose hashes match is recorded as a move, which is
      what CLAUDE.md's "when avoidable" asks for. Nothing is written into the
      markdown: a UUID in frontmatter would survive rename+edit, but the editor is
      a live-preview surface with no frontmatter support, so the id would render
      as literal text atop every note. **Known gap:** a note renamed *and* edited
      offline degrades to delete+create. That is the case "when avoidable"
      concedes.
- [x] **State lives in `.booklet/` inside the vault** — manifest, server binding,
      cursor, vault id. Obsidian's `.obsidian/` precedent. It travels with the
      vault, so a copied or restored folder keeps its binding, and the **vault id
      is how a cloned folder knows which server vault it is** rather than guessing
      from an absolute path that differs per machine. All of it is derived: delete
      `.booklet/` and you pay a rescan, nothing more. **`.booklet/` never syncs** —
      it is device-local by definition. (Split across `manifest.json` and
      `sync.json`, by how often each changes.)
- [x] **Manifest is plain JSON**, written and renamed atomically (a real
      temp+rename — see the note above; `config.rs` does not actually do this). No
      new dependency for the manifest itself, readable by a person; a few thousand
      notes is a few hundred KB.
- [x] **mtime+size gate, hash decides.** Hash only what the gate flags, so a quiet
      vault costs one stat per file, and a touched-but-unchanged file never
      uploads. A backup restore resets mtimes and triggers a rehash — correct, and
      briefly slow. (Gate stores mtime in nanoseconds; see the note above.)
- [x] **Folders are sync entities, not an accident of paths.** *Against the
      recommendation, deliberately:* 5d gives the user a real "New section" button,
      and a git-style file-only protocol would leave a section made on one device
      invisible on the other until a note landed in it. It also fixes books — a
      book is a folder plus `booklet.json`, so file-only sync would have no book
      until it had a note. Costs, all real: folders need manifest entries, feed
      entries and tombstones of their own; a folder move must ride as **one** entry
      or it is not a move; and **a folder delete propagates only when the folder is
      empty on the receiving side**, or a delete takes files with it that the
      deleting device never saw.
      - **Deferred within 2a:** folders are tracked and emit created/deleted, but
        *folder-move-as-one-entry* is not built — a renamed folder degrades to
        delete+create (its notes ride along). A folder has no content hash to
        match on, and coalescing the move only means something at upload time
        against the server's folder semantics, which do not exist until 2c/2d.
        Same shape as the note rename+edit concession above; a test pins the
        degraded behavior so 2d revisits it knowingly.
- [ ] The watcher already exists (`src/library.rs`) and already calls `refresh` on
      every write. Sync hangs off those events rather than growing a second
      watcher. **(Deferred to 2d — 2a has no consumer, so nothing is wired to the
      watcher yet.)**

### 2b — Merge and conflict rules

**Done 2026-07-17** in `booklet-core/src/merge.rs` (Qt-free, 10 unit tests). The
pure resolution primitives the sync engine will call: `merge_markdown`,
`merge_booklet_json`, `conflict_copy_name`. Crate is **`diff-match-patch-rs`
0.5.1** in **`Compat` mode** (operates on `char`s, so the vault's German/Greek
survives; `Efficient`/`u8` mode would risk splitting a codepoint). Two findings
worth carrying forward, both proven with a throwaway probe:
- The fuzzy apply is **very** lenient — two edits that share surrounding text
  merge to a garbled result reported as *clean*. That is the duplicated-sections
  bug in the flesh; `clean = false` only fires on a hard match failure, which is
  exactly why the flag alone is not enough and version history (2c) is the real
  safety net.
- `patch_apply` can **panic** (an internal `attempt to subtract with overflow`)
  on some inputs, not just return `Err`. `merge_markdown` wraps the call in
  `catch_unwind` and reports a panic as a failure, so one pathological note can
  never take the sync thread down. A test pins this.

- [x] **Three-way merge via `diff-match-patch-rs`**, computed in Qt-free core as a
      pure `(base, local, remote) -> (merged, per-hunk applied)`. Note DMP is *not*
      a merge library: the merge is `patch_make(base→local)` applied onto remote,
      matching fuzzily. **That fuzziness is Obsidian's duplicated-sections bug**,
      not a side effect of it. Chosen anyway over `diffy`'s diff3, which never
      corrupts silently but writes `<<<<<<<` markers into prose and diffs by line —
      coarse when a paragraph is one long line. (`MarkdownMerge { text, clean }`;
      `clean` is `applied.iter().all(..)`.)
- [x] **A partial merge is accepted and the note is flagged**, rather than falling
      back to a conflict copy. *Against the recommendation.* It is defensible only
      because history exists — a merge that duplicates a section is recoverable
      from the version before it. **These two decisions are load-bearing for each
      other: dropping history makes this one reckless.** (`clean == false` is the
      flag signal; the merged text is still returned and usable.)
- [x] **Conflict copies survive for exactly one case — no common ancestor.** Two
      devices independently create the same filename, so there is nothing to merge
      from. `Note (conflict 2026-07-15).md`; a second conflict the same day needs a
      suffix (`… 2).md`, inside the parenthetical). Obsidian punts on this same
      case. Conflict copies are ordinary markdown and so sync everywhere for free,
      which puts the losing text on whichever machine you happen to be at.
      (`conflict_copy_name(stem, date, taken)`; `date` is passed in so the module
      stays time-free and deterministic — the caller formats it.)
- [x] **`booklet.json` merges by key overlay** — local keys over remote, Obsidian's
      approach for its settings JSON. It cannot emit invalid JSON (a text merge
      can), binding colour and shelf label are independent fields so key grain is
      the right grain, and it preserves 5g's promise that unknown keys survive.
      (If either side is not a JSON object — a hand-mangled file — no merge is
      attempted and local is returned unchanged, so the merge never manufactures
      invalid JSON nor drops local content.)
- [x] Unit-test the resolution matrix over `(base, local, remote)` triples. No
      network, no Qt.

Deferred to the sync engine (2c/2d) — orchestration and I/O, not pure resolution:

- [ ] **The base comes from the server**, not a local shadow copy: 2c keeps
      history, so the ancestor is a fetch away. Merging only happens during a sync
      and a sync means a connection, so this costs offline-first nothing. It does
      mean **the server's storage is history-shaped from the first commit** (2c).
- [ ] **Whoever syncs first keeps the name**, when it comes to that: the server
      409s a stale PUT and the loser reconciles. Deterministic, no clock trust —
      two machines' clocks disagree and mtime-LWW picks the wrong winner silently.
      This is really *first*-write-wins, a second deviation from CLAUDE.md's
      wording; with merging primary it now only decides the no-ancestor case.
- [ ] **Deletes land in the system Trash** on the receiving device — the same
      `trash` crate 5d uses (already a `booklet-core` dependency). A sync bug or a
      mis-click on another machine then costs nothing permanent, which matters most
      while the sync code is newest. This is an apply step during sync, so it lands
      with the client engine rather than in the pure merge module.

### 2c — `booklet-sync-server`

**Done 2026-07-17 (built in slices, see `design/sync-server.md`).** Crates:
`booklet-sync-proto` (wire types), `booklet-sync-server` (blob store + Postgres +
axum). The **delta-chained content-addressed blob store** (`src/blob.rs`, `qbsdiff`
deltas + zstd checkpoints every K=50, orphan prune, corrupt-chain caught by
re-hash) sits behind the content-hash interface. **PostgreSQL** schema
(`migrations/0001_initial.sql`) with per-vault monotonic `seq` + per-file
`version`, history + tombstones kept forever, blob chain metadata. **Multi-user
auth**: argon2id passwords, device tokens (`sha256` at rest), ownership on every
`/vaults/{id}/*` route (404 for another user's), CLI `serve` / `user create`.
~9 routes, verified end-to-end over real TCP. **Deferred as planned:** the
history-retention horizon / blob GC (bounded for now by the push debounce), and
the M7 admin surface. The checkboxes below are all met except those.

- [ ] **Multi-user** (decided while planning M7): a vault belongs to a user, a
      device token to a user's device, every route scoped by owner. Ownership has
      to be on the routes from the first commit — retrofitting means auditing every
      one again. Accounts are made by an admin (M7); no self-registration, no
      invites, no email. This is what gives the app's inert **Sign in** (5h)
      something to mean: credentials buy a device token.
- [ ] **Content-addressed blobs + a metadata DB.** *Reversed from the first pass*,
      which mirrored current files as plain markdown on disk: history makes
      content-addressing nearly free (a version is a blob, dedup comes built in)
      and makes the mirror expensive. **The cost, plainly: the server's data
      directory is no longer readable without the DB.** Backup stops being
      rsync-and-relax and a corrupt DB is total loss rather than partial. Local
      disk is still plain markdown — CLAUDE.md's rule is about the client — but the
      server no longer shares that property, and that was a real reason to like the
      mirror.
- [ ] **History kept forever**, with a UI in 2e. *Against the recommendation on
      both counts.* What bounds it is the push debounce in 2d, not a retention
      policy: versions track lulls in writing rather than keystrokes. **Revisit
      first if the blob store grows faster than expected** — the fallback is a
      horizon plus "your cursor is too old, resync fully".
- [ ] **Per-vault monotonic sequence + per-file version.** Every mutation bumps the
      sequence; `GET /changes?since=N` is the feed. Each file carries its own
      version, sent as the base on PUT so a stale write is caught. Both
      server-assigned — no clocks in the protocol.
- [ ] **Tombstones forever.** A path, a version, a timestamp — years of deletions
      is a few thousand rows. GC needs a horizon *plus* a full-resync fallback,
      i.e. both mechanisms, and a device offline past the horizon re-uploads
      everything you deleted.
- [ ] **axum + tokio** — extractors and middleware for the auth boundary, and it
      serves M7's admin HTML from the same binary without a second stack. A large
      tree for ~eight routes; accepted.
- [ ] **TLS terminates at a reverse proxy**; the server binds `127.0.0.1` and
      speaks HTTP. Caddy/nginx already solve certificates and renewal, and binding
      to loopback hands M7 its localhost-only `/admin` for free. Transport stays
      HTTPS end-to-end, so CLAUDE.md holds. Cost: deploying is two things.

### 2d — Client sync engine

**Core done 2026-07-17** in `booklet-sync-client` (Qt-free, blocking `ureq`).
`Client` maps to every route; `engine::push`/`pull` reconcile a local vault
against a server vault using 2a's `Manifest` and 2b's merge functions — a 409
merges against the ancestor fetched from history, and a no-ancestor collision
writes a conflict copy. **The integration test stands the real server up on a
random port and drives two devices through publish → sync → 409-merge →
conflict-copy** (the 2a–2d exit criterion, no UI). **Deferred to the app/UI
wiring (2e / M5 sync pill):** the polling cadence (poll every ~30s + on
focus/startup/after-push), the push debounce, the device-token file (chmod 0600),
the publish/clone-refuse-non-empty UX, and driving it from a dedicated thread via
`QmlMethodInvoker`. The engine itself is complete and tested.

- [ ] **Blocking `ureq` on a dedicated thread.** The engine must be off the UI
      thread regardless, and a blocking call inside a thread you own is the
      simplest thing that works. No async runtime in the QML process, and
      `booklet-core` stays free of Qt *and* tokio — so its tests stay plain
      `#[test]`.
- [ ] **`booklet-sync-proto`** — a workspace crate of wire types depended on by
      both sides, so the contract cannot drift. A shared contract with two real
      consumers today, so Rule 2 does not bite. Not in `booklet-core`: the server
      would then pull `pulldown-cmark` and `trash`, and `trash` wants a desktop
      session a headless box does not have.
- [ ] **Poll `/changes?since=N`** every ~30s, plus on startup, on window focus and
      after a push. One route, no persistent connection, no reconnect logic. Up to
      ~30s of latency, accepted for a personal server.
- [ ] **Push on a long debounce (10–30s idle)** — deliberately *not* the flush
      debounce. The disk flush stays fast because that is local safety; the push
      waits for a real lull, so a version means "a moment you stopped writing"
      rather than "you paused to think". **This is the only thing bounding the
      forever-history.**
- [ ] **Only `.md` and `booklet.json` sync**, per CLAUDE.md; everything else in the
      vault is ignored. **Open question: whether an ignored file is silent.** A
      user who drops a PDF in and finds it missing on the other device deserves to
      be told. Decide when the UI lands.
- [ ] **Publish and clone; refuse to merge two non-empty vaults.** Publish a local
      vault into an empty server slot; clone a server vault into an empty local
      folder. Both unambiguous. A non-empty local vault pointed at a non-empty
      server vault is refused with an explanation — consistent with 5h, where
      `create_vault` already refuses a folder holding anything. Known gap: "I set
      both up before signing in" needs a manual fix.
- [ ] **The device token gets its own file, chmod 0600** — not `vaults.json`, which
      is hand-editable by design and the kind of thing pasted into a bug report; a
      live credential should not ride along. Plaintext at rest accepted (anything
      running as you can read it). The OS keychain needs a Secret Service daemon a
      minimal Linux box may not run, so it would mean shipping both paths.
- [ ] Integration-test client↔server in-process on a random port. No UI.

### 2e — Live merge and version history (the UI step)

Split out because 2a–2d keep CLAUDE.md's no-UI exit and this cannot: merging into
the document someone is typing in is UI work by definition. **The algorithm stays
in Qt-free core and is unit-tested there; only its application lives here.**

**Core done 2026-07-17.** The `booklet-sync-client` engine is hosted on a `Sync`
`#[qobject]` adapter's background thread (`src/sync.rs`), reporting to the UI via
`QmlMethodInvoker` exactly as the file watcher does. Sign-in (device token in its
own 0600 file), publish, clone (`CloneDialog.qml` — server vault list + folder
picker), the live sync pill (synced / syncing / offline / error + flagged count +
menu), a Settings **"Sync"** pane, the flagged-merge banner, and the
version-history modal with a **colored diff** (`merge::diff_segments`) all landed.
**Verified end-to-end**: a running app with a bound vault auto-pulls a remote note
to disk and persists its cursor/versions. **Only remaining:** an interactive human
pass to eyeball the in-editor merge, banner, and diff on a real display (offscreen
runs can't drive clicks).

- [x] **Merge applies to the live `TextEdit` document**, Obsidian-style — text
      appears in place, no reload, no flicker. **Mechanism found:** QML `TextArea`
      already exposes `insert(pos, text)` / `remove(start, end)`, so the merge
      lands as **range edits** replayed back-to-front from a Rust
      `merge::edit_script` (a `diff-match-patch` diff, UTF-16-addressed to match
      Qt), under the `loading` guard so `set_source` fires once. The caret is
      transformed by walking the same hunks (`merge::map_caret`). This kept the
      undo stack and avoided the full-reassignment flicker — the roadmap's
      original worry, now closed. `NoteEditor.reload_edits` bridges disk→hunks.
- [x] **Re-check local at apply time; recompute if it moved.** The edit-script is
      computed against the *current* buffer at apply time, so untouched regions
      keep post-flush keystrokes and only the merged region is rewritten. (A full
      mid-burst defer is noted but not built; the flush-before-sync window is tiny.)
- [x] **Flagged notes get a banner** above the text by the meta line, plus a
      **count on the sync pill**. The banner is unmissable when you open the note;
      the pill is how you find a flagged note you have *not* opened. Dismissing it
      is the "I checked it" action (`Sync.dismiss_flag`). Flags persist in
      `.booklet/sync.json` until dismissed. **This flag is the only thing between a
      silently duplicated section and the user — it is not decoration.**
- [x] **Version history is a modal** (`VersionHistory.qml`), like Settings: a
      version list beside a **colored diff** of the selected version against the
      note's current text (`NoteEditor.diff_segments` → `merge::diff_segments`;
      inserts green, deletions struck through in ember), with restore
      (`Sync.restore` → fetch the blob → write as a fresh local edit → reload the
      editor). It ships here because it is the recovery path for an accepted
      partial merge — **history the user cannot reach does not make a bad merge
      recoverable.**
- [x] **Sync pill goes live** — synced / syncing / offline / error, plus the
      flagged count and a menu (sync now / history / publish / sign in / out).
- [x] Notify the UI from the sync thread via `QmlMethodInvoker` — the thread
      deposits events into an `Arc<Mutex<_>>` and calls the argument-less `pump`
      slot, which emits the QML signals (the file-watcher pattern).

**Exit:** 2a–2d — two clients reconcile through the server, merging where they
can and writing conflict copies where there is no ancestor, all verified by tests
with no UI. ✅ **Met 2026-07-17** — `booklet-sync-client/tests/integration.rs`
drives two devices through the real server (basic sync, a 409-driven merge that
converges, and a no-ancestor conflict copy). 2e — the same driven through the
app. ✅ **Met 2026-07-17** — the engine is hosted in-app and verified to auto-sync
a bound vault; the in-editor merge, flag banner, sign-in/publish/clone, Settings
sync pane, and the history modal with a colored diff are all built. Only an
interactive visual pass on a real display remains. **This completes M2.**

**Design note written before 2c: `design/sync-server.md`** (2026-07-17). It pins
the account model, the blob store, and the version feed — the parts expensive to
change once written — into a buildable spec: the PostgreSQL schema, a **Git-style
delta-chained blob store** (a full checkpoint every K versions, exact binary
deltas between — behind the content-hash interface, so the client never sees it),
the ~9 routes, the `booklet-sync-proto` wire types, and how they meet 2a's hashes
and 2b's merge functions. Five implementation calls are flagged there for a yes
before coding (the Postgres driver, the `moved_from` wire field, CLI subcommands,
the delta codec, and the checkpoint interval K).

## M3 — App adapters follow CLAUDE.md idioms

With the logic in the library, the qtbridge layer becomes thin adapters.
Apply the idiom fixes here (they were the original scaffold-rewrite items):

- [x] **Never swallow errors (Rule 7).** Every adapter emits `failed(message)`
      and `Notice.qml` shows it — 16 console-only failures are now visible. The
      status bar carries saved/unsaved (`save_state_changed`), which matters
      because saving is debounced and silent. Verified against a real failure: a
      read-only note reports "Could not save note: Permission denied" and stays
      marked unsaved rather than falsely claiming it saved.
      Two `eprintln!` remain on purpose — font registration and watch-start
      happen before/outside any note context and have no UI to land in.
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
      - **It shipped with no button** — ⌘L was the only way in, so the one screen
        that shows a vault's books could only be found by being told about it.
        Reported as "I can switch between Vaults but not between booklets", which
        is exactly right: switching vaults had a visible topbar menu and browsing
        the books inside one had nothing. It now has a topbar button beside that
        menu. The reference draws no shelf at all, so it dictates no placement
        here; beside the vault menu is ours, on the grounds that the two are the
        same errand a level apart.
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
  then it renders the offline state and gains real status in M2. *(Resolved in
  M2/2e: the pill is live — synced / syncing / offline / error + a flagged count
  and a menu.)*
- **The tree row's hover tint is the theme's ink at 3%, not white.** The
  reference hardcodes `rgba(255,255,255,.03)` for `.row:hover` — the one hover in
  the whole document that is not `var(--active-pill)` — and that reads as nothing
  on `vellum`'s paper. Deriving the tint from `Theme.text` lightens on a dark
  theme and darkens on a light one, which is what the 3% was for. **This is a bug
  in the reference, not a disagreement with it:** vellum was added to the
  reference without revisiting the rule.

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
      hide-sidebar button too. A hidden panel's toggle goes with it, so the
      topbar grows a **show** button while a panel is hidden — otherwise the
      only way back was `⌘\`, which is `⌘⌥⇧7` on a German layout. The reference
      never depicts the hidden state, so this is a deliberate addition to it.
- [x] **Tooltips** on every button the reference gives a `title=`: the five
      toolbar icons, the tab `×` and `+`, the sync pill, and the vault menu.
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

- [x] **Back / forward** history — topbar buttons and `⌘⌥←` / `⌘⌥→` (arrows,
      not `⌘[`/`⌘]`: brackets need `⌥5`/`⌥6` on a German layout). Going somewhere
      new drops the forward trail, as a browser does.
- [x] **Note rename** — landed in 5d instead: renamed inline in the tree.
- [x] **Save indicator** — in the status bar, driven by `save_state_changed`.
      A failed write stays `unsaved` rather than lying (done with M3).
- [x] **Preview / Source toggle** at the page's top-right. Now that the editor
      is one live-preview surface, "Source" detaches the highlighter: the
      markdown exactly as written, in mono, with nothing hidden.
- [x] **Unresolved link styling** — `--text-soft` + dashed underline. The
      highlighter takes the vault's titles (`knownTitles`, refreshed on
      `tree_changed`) and resolves `[[Title|alias]]` on the title. Verified:
      `[[Port log]]` resolves, `[[TRACE32 notes]]` does not. This is what makes
      links broken by a rename visible.
- [x] **Note meta line** — "● PIXEL 7 · EDITED TODAY" above the text, from
      `NoteEditor.meta()` (section + mtime; QML does the wording). Verified:
      `{"modified":…, "section":"Pixel 7"}`.

### 5f — Marginalia and the star map

- [x] **Star map ("sightlines")** at the top of the pane on `--code-bg`: the open
      note is the centre star (`--link` ring, `--text-bright` core); incoming refs
      are `--text-soft` dots, outgoing links `--link` dots, unresolved hollow
      `--text-dim` on dashed sightlines. Lines 0.7px at ~55%, labels 7px mono
      uppercase — labels stay uniformly `--text-dim` (the dot carries the kind),
      as the reference has them. **Radial spread — angle by index with slight
      radius jitter, no force simulation.** Cap ~10 dots; the card list carries
      the overflow. Clicking a dot opens that note; unresolved dots create it via
      `open_by_title`. `StarMap.qml` draws the sightlines on a `Canvas` but the
      dots are real Items, so they hit-test themselves.
      - **A mutual link is one star, not two.** A note that links back and is
        linked to appears once, keeping the outgoing colour — that dot answers to
        a `[[link]]` visible in the text being read. Found by probing the real
        data: it had drawn two same-named dots at different angles.
- [x] Needs one new slot: **`NoteEditor.outgoing_links()`**. Everything else is
      data the pane already has. Reads the editor's in-memory text, not the disk,
      so the map keeps up with what has been typed.
- [x] **Tags** — `#tag` parsing in core (`booklet-core/src/tags.rs`), pills at the
      foot of the pane. A tag is `#` + a letter starting a word, which is what
      separates it from a `# Heading`, a `#anchor` in a URL and a `#3C5240`;
      fenced code blocks hold no tags. Tags are read-only for now — no filtering
      or tag index yet.

### 5h — Vault picker (welcome screen)

The reference's welcome screen: a centred 520px column on `--sidebar` — book
mark, wordmark, version (`CARGO_PKG_VERSION`), one primary button, a *recently
opened* card, an actions card, and a language row.

- [x] **Config model change.** Vault entries stopped being plain paths and became
      `{ path, color, last_opened }`. **One list serves both** the picker
      (sorted most-recent-first, capped at 8) and the topbar menu; `remove_vault`
      already meant "forget it, do not touch the disk". `color` is auto-assigned
      from the binding palette on add — the first one going spare, so two vaults
      never share a dot until the palette runs out.
      - **`last_opened` is epoch *milliseconds*, not seconds** as first planned.
        A test caught it: clicking through the picker opens two vaults well
        inside one second, those opens tie at second resolution, and the list
        falls back to alphabetical — showing an order the user did not create.
      - **Configs of bare paths still load.** `VaultEntry` has a hand-written
        `Deserialize` that reads a path string *or* the object; without it every
        vault already configured would vanish on upgrade. Colourless vaults are
        painted at load, and reopening the last vault counts as an open, so the
        picker never calls the vault on screen "never opened". Verified against
        a copy of the real `~/.config/booklet/vaults.json`.
- [x] **New core:** `recent_vaults()` (the capped, sorted view) and
      `create_vault(path, book, note)`. Opening a vault bumps its `last_opened`.
      `create_vault` **refuses a folder that already holds anything** — turning
      someone's directory into a vault is `add_vault`'s job to be asked for, not
      a side effect. (Known edge: quick start onto an existing `~/Documents/
      Booklet` reports that rather than adopting it. Use *Open* for that.)
- [x] **Picker QML** — mark (56px `Shape`), wordmark, version
      (`CARGO_PKG_VERSION` via `Library.version()`), recents rows (dot = the
      vault's colour, name in the display face, path in mono `--text-dim` with
      `~` for home, relative time, × removes from the list only), actions card:
      **Create** / **Open folder** / **Sign in**.
- [x] **Quick start** — the app's only filled button: creates a starter vault at
      `~/Documents/Booklet` (one book, one note) and opens it, so a first run
      lands somewhere instead of an empty picker.
- [x] **Shown when there is no vault to reopen** — a bare start otherwise
      reopens the vault you were last in. Reachable any time from the vault menu
      ("Open another vault…"), which replaced its own folder dialog with this.
      Escape backs out **only when there is a vault to go back to**.
- [x] **Sign in** ships inert until the sync engine (M2), like the sync pill.
      *(Resolved in M2/2e: the picker's "Sign in" opens the real `SignInDialog`
      → device token.)*

Deviations from the reference here, decided with the user:

- **Strings are English**, though the reference's picker is written in German.
  The language selector is built but **inert**; real i18n (`qsTr` + Qt
  translations, and checking whether qtbridge even exposes `QTranslator`) is a
  later milestone of its own.
- **`Library.open_vault` does not exist** — the reference reaches for it, but the
  multi-vault work replaced it with `add_vault` + `set_active`. Same drift as its
  `link_unresolved` note.

**Fixed while building this:** `Engine::rebuild_with` rebuilt every vault from
its path alone, so it silently dropped colour and last-opened. `refresh()` calls
it and the **file watcher calls `refresh` on every write** — the picker's recency
would have been wiped as you typed, and the next save would have persisted the
zeros. It now carries across what is not on disk, with a regression test proven
to fail against the old code.

### 5g — Beyond

- [x] **Full-text search** across the vault, in the ⌘K switcher: titles match as
      you type, and notes whose *writing* holds the query follow under an
      `IN TEXT` marker, each with the line it says it on. A note matched both
      ways is listed once, under its title. The scan reads every note, so it
      waits 180ms for a pause and ignores queries under 2 characters — an
      on-demand scan, as CLAUDE.md defers a persistent index until measured.
      `search.rs` finds matches without lowercasing the haystack: that can change
      a string's length, and the snippet is cut from the original at that offset.
- [x] **Book metadata editing** — right-click a book → **Binding…**: the six
      binding colours and the shelf label, written to the book's own
      `booklet.json`. The write is read-modify-write, so **keys the app knows
      nothing about survive** — it is a plain file a person may have edited.
- [x] **Reading size in the settings screen** — a slider (11–40, the range the
      engine clamps to), with a line of prose set at the chosen size next to it.
      ⌘+ / ⌘− still work.
- [x] **Interface size and density** — Settings → Appearance, both persisted
      (`ui_scale`, `density`, whole percents so the config stays hand-editable;
      clamped 80–160 and 80–150 by the engine). `Theme.px()` scales type and
      furniture, `Theme.gap()` the room between things, `Theme.row()` anything
      holding type (by both, or text outgrows its row). ~80 hardcoded sizes were
      swept onto them. Verified live: at 150%/140% the topbar goes 38→80px and
      `px(13)`→20.
- [x] **Rounded, animated chrome** — one motion vocabulary in `Theme`
      (`quick`/`gentle`/`easing`, `radiusSmall`/`radiusCard`). Selections warm
      into their highlight instead of snapping; menus and modals fade and scale
      in. **The two context menus were stock `Menu`** — square grey boxes from
      the Basic style, belonging to no theme — and are now `AppMenu`/
      `AppMenuItem`. `SettingSlider` replaced the third copy of hand-styled
      slider internals.
- [x] **Settings** (`⌘,` or the topbar gear) — a **modal** (`Popup`, 760×520,
      capped to the window) with the categories down a sidebar and the chosen one
      in the right pane: **Vaults** (open / forget / add via a folder dialog),
      **Appearance** (the theme picker), **Editor** (reading size, with a sample
      line set at it), **About** (version, config location). The reference draws
      no settings screen, so the vocabulary is borrowed from the parts that do
      exist — the tree's sidebar for the rail, the picker's cards for the panes.
      Being a modal, it closes itself on Escape or a click outside and needs no
      full-window flag; a × sits top-right, since neither of those is visible.
      - **Four themes**, as the reference now carries: `night`, `atlas`,
        `graphite` and `vellum`. The picker is a 2-column `Grid` — four 190px
        swatches in a row is 790px and the pane is 592. **`vellum` is the first
        light theme**, which makes a rule out of what used to be free: a tint has
        to derive from a token (`Theme.text`, `Theme.brass`), because hardcoded
        white or black inverts on paper. That caught the tree's row hover; see the
        deviations above.
      - **The theme now persists** (`theme` in the config). A picker whose choice
        is forgotten on restart is not a setting, so this went in with it. The
        engine stores the name without validating it — naming the themes is the
        UI's business, and `Theme.qml` already falls back for a name it does not
        know.
      - `TextButton.qml` joins `IconButton.qml`: stock Controls buttons speak
        none of the reference's language, and the app runs under Basic style.

Explicitly **not** in this milestone: block add/delete/reorder, tree filter
field, drag-and-drop moves, split view.

## M6 — Polish & hardening

- [ ] Persistent link index if the on-demand scan gets slow (measure first).
- [ ] Graduate the hot lists to `qtbridge::QListModel` once the trait API
      settles.

## M7 — Sync server admin panel

Numbered last but **gated on 2c/2d, not on M6**: the panel needs a server with
real accounts and real sync state to look at, and nothing else. Decided with the
user: **a web UI served by `booklet-sync-server` itself**, over the CLI and over
a screen in the app, because the box you want to inspect is usually not the box
you are reading notes on.

A self-hosted server nobody can see into is a server nobody can debug. The panel
answers four questions and stops: who has an account, which devices hold live
tokens, what is on disk and how big, and what recently went wrong. Every feature
below earns its place against one of those; anything that does not is listed as
out of scope at the foot.

**The panel never reads note content.** It is an operations surface — users,
devices, bytes, errors. An admin panel that can open somebody's notes is a
second, weaker way into every vault on the box, and Booklet's own editor is
already the way in for the person who owns them. This is the line that keeps the
panel small.

### 7a — Admin identity (server work, before any HTML)

- [x] **An admin session is not a device token.** A device token is bearer
      credential shipped to a laptop to sync a vault; it must not open `/admin`,
      and an admin cookie must not sync files. Different audience, different
      lifetime, checked separately — one shared "is authenticated" helper across
      both is exactly the bug to avoid.
- [x] **Sessions:** signed `HttpOnly` + `Secure` + `SameSite=Strict` cookie,
      short expiry, server-side store so revocation is real. Argon2id for
      password hashes. Rate-limit sign-in — the panel is the one surface a
      stranger can reach with a guess.
- [x] **Bootstrap the first admin from the shell**, not the web:
      `booklet-sync-server admin grant <handle>`. Nobody can sign in to make the
      first admin, and a self-registering "first visitor becomes root" page is a
      race with whoever finds the port first. Shell access to the box is the root
      of trust.
- [x] **One flag, not roles.** `is_admin` on the user. Roles are a table, a
      policy check on every route and a UI to edit them; there are two kinds of
      person here (an operator and everybody else) and Rule of Three has not been
      met.

### 7b — Rendering and assets

- [x] **Server-rendered HTML, no SPA, no npm, no build step.** Forms that POST
      and redirect. The panel is a handful of tables and about six buttons; a
      frontend toolchain would be larger than the thing it builds, and it would
      be the only JS build in a repo that is Rust, QML and one C++ file.
- [x] **Templates and CSS `include_str!`'d into the binary**, so deploying stays
      "copy one file to the box". Templating crate (`maud` / `askama`) is a
      dependency choice worth one sentence at implementation time — `format!`
      over a `String` is a real option at this size and does not need HTML
      escaping bolted on later, which is the argument against it.
- [x] **Reuse `design/reference.html`'s `:root` block verbatim.** Its custom
      properties are already the design language and already map 1:1 to
      `Theme.qml`. Lifting the token block gives the panel Booklet's face for
      free and avoids a second vocabulary drifting from the first.
- [x] **Serve the bundled OFL fonts from the binary; no external requests.**
      `reference.html` pulls Google Fonts over the network — fine for a design
      doc opened on a dev machine, wrong for an admin page on a self-hosted box
      that may have no route to the internet and whose operator did not ask to
      tell Google when they log in. The four families are already in
      `src/booklet/fonts/`; fall back to `system-ui` rather than block on them.

### 7c — The pages

- [x] **Overview** — server version and uptime, storage used and free, counts of
      users / devices / vaults, and recent conflict copies. Conflict copies are
      rarer now (no-ancestor only) but are still the loudest thing sync does to a
      vault. **Flagged merges are NOT shown (deferred):** 2b's flag (`clean ==
      false`) is purely client-side state in `.booklet/sync.json`; the server sees
      only a normal re-PUT, so surfacing it would need a protocol + client change
      — out of scope for a server-only milestone. Conflict copies are detected by
      their `(conflict …)` filename, which the server *can* see.
- [x] **Blob store health** — total blobs, bytes, and growth. History is kept
      forever (2c) bounded only by the client's push debounce, so this number is
      the one that tells you the bound stopped holding.
- [x] **Users** — list (handle, created, last seen, vaults, bytes) and a detail
      page carrying that user's devices and vaults.
- [x] **Devices** — name, platform, issued, last seen, per user.
- [x] **Vaults** — owner, note count, bytes, last sync. Names and sizes; not
      contents.
- [x] **Log** — recent sign-ins, token issues and revocations, user and vault
      deletions, conflicts. Append-only, capped, plain rows.

### 7d — Actions

Every mutation is a POST behind a CSRF token, and every one lands in the log.

- [x] Create a user; disable a user (keeps the files, kills the tokens); delete a
      user.
- [x] Revoke a device token — the reason the panel exists at all, for a laptop
      that walked off.
- [x] **Deleting a user touches the disk and is the one irreversible thing here.**
      It gets a typed-confirmation, and it says what it is about to remove and how
      many bytes, before it does.

### 7e — Hardening

- [x] **CSRF token on every mutating form.** A cookie-authenticated HTML form
      surface is the textbook case, and this is the whole reason the panel is
      riskier than the sync API (which is bearer-token and immune by
      construction).
- [x] **`/admin` binds to `127.0.0.1` by default**, reached over an SSH tunnel;
      exposing it publicly is an explicit config change with a comment saying
      what it costs. Sync itself needs the world; administration does not.
- [x] Device token rejected on `/admin`, admin cookie rejected on sync routes —
      as a test, not as a review comment.
- [x] Integration-test the auth boundary the way 2d tests the sync one: no UI,
      just requests that should be refused.

Explicitly **not** in this milestone — each is a real feature someone could want,
and none is needed to answer the four questions:

- **Quotas and billing.** Self-hosted; the operator owns the disk and can see the
  bytes on the Overview page.
- **Reading or editing notes through the panel.** See above — this is the line.
- **Invites, email, password reset, self-registration.** The admin makes accounts
  by hand. Email is a whole subsystem (deliverability, secrets, a queue) bought
  for one form.
- **Metrics dashboards and graphs.** Counters and numbers first. If the numbers
  stop being enough, that is a measurement, and it argues for exporting to
  Prometheus rather than drawing charts in Rust.
- **A second theme.** The panel takes `night` and stops; the toggle is the app's
  business.

> **Update (2026-07-18, on request):** four of these five were built afterwards —
> **quotas + billing** (Stripe subscriptions drive per-user quota; a 507 enforces
> it), **email/invites/self-registration** (optional SMTP; admin-set reset needs
> no email), and **in-panel SVG charts + a light theme**. Each optional
> integration degrades gracefully when unconfigured. **Reading/editing notes
> through the panel stays unbuilt** — the security line held.
