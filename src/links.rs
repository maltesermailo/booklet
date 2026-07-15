//! qtbridge adapter for the Marginalia (backlinks) panel.
//!
//! The scan lives in `booklet_core::links`; this type holds the configured
//! vaults and serializes the results for QML.

use booklet_core::{config, links, vault};
use qtbridge::qobject;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A backlink as handed to QML. `source_id` is an absolute path, which is what
/// `NoteEditor.open` expects.
#[derive(Serialize)]
struct BacklinkView {
    source_id: String,
    source_title: String,
    snippet: String,
}

#[derive(Default)]
pub struct Backlinks {
    vaults: Vec<PathBuf>,
}

#[qobject(Singleton)]
impl Backlinks {
    /// Loads the configured vaults so backlinks span all of them.
    #[qslot]
    fn load(&mut self) {
        self.vaults = match config::load(&crate::library::default_config_path()) {
            Ok(config) => config.vaults,
            Err(error) => {
                self.failed(format!("Could not read vault list: {error}"));
                Vec::new()
            }
        };
    }

    /// Notes containing [[title]] (or [[title|alias]]), as JSON. `id` is the
    /// note's absolute path; the scan is scoped to that note's own vault, so
    /// backlinks never cross a vault boundary.
    #[qslot]
    fn for_note(&self, id: String, title: String) -> String {
        let Some(vault) = vault::vault_of(&self.vaults, Path::new(&id)) else {
            return "[]".into();
        };

        let views: Vec<BacklinkView> = links::backlinks_to(vault, &title)
            .into_iter()
            .map(|backlink| BacklinkView {
                source_id: backlink.source.to_string_lossy().into_owned(),
                source_title: backlink.title,
                snippet: backlink.snippet,
            })
            .collect();

        // Backlinks hold only strings, so serialization cannot fail.
        serde_json::to_string(&views).expect("backlinks serialize to JSON")
    }

    /// Something the user should see went wrong.
    #[qsignal]
    fn failed(&mut self, message: String);
}
