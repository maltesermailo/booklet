# Booklet — Roadmap

Milestones derived from `CLAUDE.md` and `README.md`. Suggested order:
M0 → M1 → M2 → M3 → M4. The scaffold rewrite (M1) sits right after
foundation-verification so every later feature is written to the idioms from
the start.

## M0 — Verify the beta foundation

The whole stack rests on `qtbridge 0.2`, which CLAUDE.md and the README both
flag as the most volatile surface. Nail this down before any feature work.

- [ ] Confirm `cargo run -- "$(pwd)/vault"` builds and launches on macOS arm64
      (an *experimental* qtbridge target).
- [ ] Verify `include_bytes_qml!` (`src/main.rs`) against the `color_palette`
      example in qt/qtbridge-rust.
- [ ] Verify `#[qobject(Singleton)]` / `#[qslot]` / `#[qsignal]` against
      `minimal_app`.
- [ ] Verify `QmlMethodInvoker` (for later off-UI-thread sync) against
      `host_monitor`.
- [ ] **Exit:** sample vault opens, tree renders, a note opens, a block edits
      and saves. Record any API drift in CLAUDE.md's beta-constraints section.

## M1 — Rewrite the scaffold to follow CLAUDE.md idioms

The scaffold works but violates several binding rules. Fixes grouped by rule.

### Rule 7 — never swallow errors (most serious)
- [ ] `note.rs` — `let _ = std::fs::write(...)` silently discards **save
      failures**. Surface via a `save_failed(reason)` signal + minimal QML.
- [ ] `note.rs` — `read_to_string(...).unwrap_or_default()` turns an
      unreadable note into a silently-empty editor that then overwrites the
      file on commit. Distinguish "empty note" from "couldn't read"; surface
      read failures to the UI too.
- [ ] `library.rs` / `note.rs` / `links.rs` — the
      `serde_json::to_string(...).unwrap_or_else(|_| "[]")` sites mask
      serialization bugs as empty results. Use `.expect()` with a
      why-it-can't-fail message, or propagate.
- [ ] `library.rs` — `let Ok(entries) = read_dir(..) else { return }`: a
      permission error yields a silently-empty tree. Decide log-and-skip vs.
      surface.

### Rule 6 — no abbreviations (except id/url/db)
- [ ] Rename single-letter bindings (`e`, `p`, `v`, `r`, `c`, `ev`, `hit`)
      across `library.rs` / `note.rs` to descriptive names.

### Global preference — blank lines between logical sections
- [ ] Break up dense functions (`library.rs::books`, `note.rs::open`,
      `note.rs::commit_block`) into setup → work → return groups.

### Efficiency / correctness
- [ ] `note.rs::rewrite_wikilinks` recompiles its regex on **every block
      render**. Hoist to a `LazyLock<Regex>` (also removes an `.unwrap()`).

### Latent re-entrancy bug (beta constraint)
- [ ] `note.rs::open` emits `note_opened` / `blocks_changed` while the
      `&mut self` borrow is held; QML handlers call back into `NoteEditor`
      (the `QML → Rust → QML → same object` borrow panic). Restructure so
      signals fire after the borrow is released. Depends on borrow semantics
      verified in M0.

### Comments
- [ ] Audit for any "what" comments introduced during the rewrite; keep only
      "why".

- [ ] **Exit:** behavior identical, no silent `let _ =` / `unwrap_or_default`
      on I/O, failed save/read visible in the UI, clean `cargo clippy`,
      re-entrancy path proven safe.

## M2 — Complete the core reading/writing UX

Features listed as present-or-stubbed but not wired.

- [ ] **Fonts** — `Theme.qml` names EB Garamond / Alegreya Sans / Spectral /
      JetBrains Mono, but nothing loads them. Add `FontLoader` from bundled
      resources.
- [ ] **Create-note-on-unresolved-link** — `NoteEditor.link_unresolved(title)`
      fires but nothing consumes it. Create the `.md` and open it.
- [ ] **Books / shelf view** — `Library.books()` has no QML consumer. Build
      the shelf view.
- [ ] **Quick switcher (⌘K)** — flatten the note list from `Library`, add a
      `Popup` + filter.

## M3 — Server sync (base feature, currently missing)

Largest chunk. Respect CLAUDE.md constraints: plain markdown stays the local
source of truth, offline-first, per-file sync unit, last-write-wins with
conflict copies, engine off the UI thread, HTTPS + per-device token.

- [ ] **3a** — `booklet-sync-server` crate in the workspace. Minimal HTTPS
      API: per-device token auth, per-file get/put with version metadata,
      list/since.
- [ ] **3b** — Client sync engine off the UI thread (tokio), notifying QML via
      `QmlMethodInvoker`. Track moves/renames rather than delete+create where
      possible.
- [ ] **3c** — Conflict handling: last-write-wins per file, losing copy kept
      as `Note (conflict YYYY-MM-DD).md`. **No CRDT** — out of scope until a
      demonstrated need.
- [ ] **3d** — Offline reconciliation on reconnect.

Recommend a short design note + one clarifying pass before 3a; the server
shape is underspecified in the current docs.

## M4 — Polish & hardening

- [ ] Persistent link index if/when the on-demand scan (`links.rs`) gets slow
      — CLAUDE.md marks this a *later* optimization (YAGNI until measured).
- [ ] Graduate the two hot lists to `qtbridge::QListModel` **only after** the
      trait API settles (verify the trait API first).
