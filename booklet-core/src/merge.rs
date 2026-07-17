//! Merge and conflict resolution, as pure functions.
//!
//! When two devices edit the same note, Booklet reconciles them with a
//! three-way merge rather than letting one silently overwrite the other. This
//! module holds the resolution logic and nothing else — no network, no disk, no
//! Qt. The sync engine (a later step) fetches the common ancestor from the
//! server's history, calls in here, and writes the result.
//!
//! Three primitives, one per thing that can diverge:
//! - [`merge_markdown`] — a note's text, via `diff-match-patch`.
//! - [`merge_booklet_json`] — a book's metadata, by key overlay.
//! - [`conflict_copy_name`] — the fallback name when there is no ancestor to
//!   merge from.

use diff_match_patch_rs::{Compat, DiffMatchPatch, PatchInput};

/// The outcome of a three-way markdown merge.
pub struct MarkdownMerge {
    /// The merged text.
    pub text: String,
    /// Whether every one of the local edits applied cleanly. A `false` here is a
    /// *partial* merge: it is still accepted (the text is usable), but the note
    /// must be flagged for review, because `diff-match-patch` matches fuzzily and
    /// a rejected or misplaced hunk is how a section ends up duplicated.
    pub clean: bool,
}

/// Merges a note three ways: the local and remote edits of a common `base`.
///
/// `diff-match-patch` is not a merge library; the merge is the set of edits that
/// turned `base` into `local`, replayed onto `remote` (which already carries the
/// other device's edits). `patch_apply` reports, per hunk, whether it landed —
/// all landed means a clean merge, any rejected means a partial one to flag.
///
/// `Compat` mode operates on `char`s, so multi-byte text (the vault has German
/// and Greek in it) merges correctly rather than splitting a codepoint.
///
/// A failing `Result` means the merge could not be computed at all — the engine
/// treats that like the no-ancestor case and writes a conflict copy. It covers
/// two things: the library returning an error, and the library *panicking*.
/// `diff-match-patch` can hit an internal arithmetic underflow on some inputs
/// and unwind rather than return `Err`; a merge runs on the sync thread and must
/// never take the process down over one pathological note, so the panic is
/// caught and reported like any other failure.
pub fn merge_markdown(base: &str, local: &str, remote: &str) -> Result<MarkdownMerge, String> {
    let outcome = std::panic::catch_unwind(|| {
        let dmp = DiffMatchPatch::new();

        let patches = dmp
            .patch_make(PatchInput::<Compat>::new_text_text(base, local))
            .map_err(|error| format!("could not diff the note: {error:?}"))?;

        dmp.patch_apply(&patches, remote)
            .map_err(|error| format!("could not merge the note: {error:?}"))
    });

    match outcome {
        Ok(Ok((text, applied))) => {
            Ok(MarkdownMerge { text, clean: applied.iter().all(|&landed| landed) })
        }
        Ok(Err(message)) => Err(message),
        Err(_) => Err("the merge library panicked on this note".to_string()),
    }
}

/// Merges a book's `booklet.json` by overlaying `local`'s keys onto `remote`'s,
/// local winning any key they share.
///
/// Key grain is the right grain here — binding colour and shelf label are
/// independent fields — and it keeps the promise that a key the app does not
/// know survives, whichever side wrote it. Because the result is a re-serialized
/// JSON object, this can never emit invalid JSON the way a text merge could.
///
/// The overlay needs both sides to be JSON objects. If either is not (a file a
/// person hand-edited into something malformed), no merge is attempted and the
/// local file is returned unchanged — the reconciliation never turns a valid
/// file invalid, and never discards local's content.
pub fn merge_booklet_json(local: &str, remote: &str) -> String {
    let (Ok(local_value), Ok(remote_value)) = (
        serde_json::from_str::<serde_json::Value>(local),
        serde_json::from_str::<serde_json::Value>(remote),
    ) else {
        return local.to_string();
    };

    let (Some(local_map), Some(remote_map)) = (local_value.as_object(), remote_value.as_object())
    else {
        return local.to_string();
    };

    let mut merged = remote_map.clone();
    for (key, value) in local_map {
        merged.insert(key.clone(), value.clone());
    }

    // A map of JSON values re-serializes without fail.
    serde_json::to_string_pretty(&merged).expect("merged metadata serializes to JSON")
}

/// The filename for a conflict copy of a note whose stem is `stem`, dated
/// `date` (e.g. `2026-07-15`, formatted by the caller — this module stays
/// time-free and testable). `taken` is the set of filenames already in the
/// note's folder; a same-day second conflict gets a numeric suffix so it does
/// not collide.
///
/// This is the narrow fallback for the one case a merge cannot serve: two
/// devices independently created the same filename, so there is no common
/// ancestor to merge from.
pub fn conflict_copy_name(stem: &str, date: &str, taken: &[String]) -> String {
    let mut candidate = format!("{stem} (conflict {date}).md");

    let mut suffix = 2;
    while taken.iter().any(|name| name == &candidate) {
        candidate = format!("{stem} (conflict {date} {suffix}).md");
        suffix += 1;
    }

    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "# Title\n\nAlpha paragraph.\n\nBeta paragraph.\n";

    #[test]
    fn a_note_unchanged_on_both_sides_merges_to_itself() {
        let merged = merge_markdown(BASE, BASE, BASE).unwrap();

        assert_eq!(merged.text, BASE);
        assert!(merged.clean);
    }

    #[test]
    fn a_local_only_edit_is_kept() {
        let local = "# Title\n\nAlpha paragraph, edited locally.\n\nBeta paragraph.\n";

        let merged = merge_markdown(BASE, local, BASE).unwrap();

        assert_eq!(merged.text, local);
        assert!(merged.clean);
    }

    #[test]
    fn a_remote_only_edit_is_kept() {
        let remote = "# Title\n\nAlpha paragraph.\n\nBeta paragraph, edited remotely.\n";

        let merged = merge_markdown(BASE, BASE, remote).unwrap();

        assert_eq!(merged.text, remote);
        assert!(merged.clean);
    }

    #[test]
    fn edits_to_different_paragraphs_both_survive_a_clean_merge() {
        let local = "# Title\n\nAlpha paragraph, edited locally.\n\nBeta paragraph.\n";
        let remote = "# Title\n\nAlpha paragraph.\n\nBeta paragraph, edited remotely.\n";

        let merged = merge_markdown(BASE, local, remote).unwrap();

        assert!(merged.clean);
        assert!(merged.text.contains("edited locally"));
        assert!(merged.text.contains("edited remotely"));
    }

    /// Both devices retitled the same one-line note differently. Local's edit
    /// cannot be located in remote's version, so the hunk is rejected and the
    /// merge is partial — the note gets flagged. (When two edits stay similar
    /// enough, `diff-match-patch` instead matches fuzzily and reports clean; that
    /// leniency is the duplicated-sections risk the flag exists to catch, so this
    /// test drives the hard-failure path deliberately.)
    #[test]
    fn conflicting_edits_to_the_same_line_are_a_partial_merge() {
        let base = "# Draft note\n";
        let local = "# Published note by local\n";
        let remote = "# Archived note by remote\n";

        let merged = merge_markdown(base, local, remote).unwrap();

        assert!(!merged.clean);
        // The text is still usable — remote's version, with local's rejected edit
        // recoverable from history.
        assert_eq!(merged.text, remote);
    }

    /// `diff-match-patch` can panic on some inputs rather than return an error.
    /// The merge must contain that and report a failure, never crash the caller.
    #[test]
    fn a_library_panic_becomes_an_error_not_a_crash() {
        let base = "keep this. REMOVE THIS SENTENCE ENTIRELY. keep that.\n";
        let local = "keep this.  keep that.\n";
        let remote = "keep this. keep that changed by remote wildly differently now.\n";

        assert!(merge_markdown(base, local, remote).is_err());
    }

    #[test]
    fn booklet_json_overlays_local_over_remote_and_keeps_unknown_keys() {
        let local = r##"{ "color": "#7C3128", "shelf": "Work" }"##;
        let remote = r##"{ "color": "#2F3E5C", "reading_order": 3 }"##;

        let merged = merge_booklet_json(local, remote);
        let value: serde_json::Value = serde_json::from_str(&merged).unwrap();

        // Local wins the shared key; each side's own keys survive.
        assert_eq!(value["color"], "#7C3128");
        assert_eq!(value["shelf"], "Work");
        assert_eq!(value["reading_order"], 3);
    }

    #[test]
    fn booklet_json_merge_leaves_local_untouched_when_remote_is_malformed() {
        let local = r##"{ "color": "#7C3128" }"##;
        let remote = "{ not json at all";

        assert_eq!(merge_booklet_json(local, remote), local);
    }

    #[test]
    fn a_conflict_copy_is_named_for_the_day() {
        assert_eq!(
            conflict_copy_name("Port log", "2026-07-15", &[]),
            "Port log (conflict 2026-07-15).md"
        );
    }

    #[test]
    fn a_second_conflict_the_same_day_gets_a_suffix() {
        let taken = vec!["Port log (conflict 2026-07-15).md".to_string()];

        assert_eq!(
            conflict_copy_name("Port log", "2026-07-15", &taken),
            "Port log (conflict 2026-07-15 2).md"
        );
    }
}
