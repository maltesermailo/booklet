//! Persistence of the library configuration: the vaults, which folders are
//! expanded, and the UI's remembered choices. Stored as plain JSON, matching
//! Booklet's plain-files-on-disk principle.

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// A vault as the config remembers it: where it is, the colour of its dot in
/// the picker, and when it was last opened (epoch milliseconds; 0 = never).
#[derive(Serialize, Default, PartialEq, Debug)]
pub struct VaultEntry {
    pub path: PathBuf,
    pub color: String,
    pub last_opened: u64,
}

/// Reads both shapes this field has had: a bare path string, as configs written
/// before the vault picker held, and the object it is now. Without this, every
/// vault already configured would vanish on upgrade.
impl<'de> Deserialize<'de> for VaultEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Object {
            path: PathBuf,
            #[serde(default)]
            color: String,
            #[serde(default)]
            last_opened: u64,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Bare(PathBuf),
            Object(Object),
        }

        Ok(match Stored::deserialize(deserializer)? {
            // Colour and last-opened are simply unknown for these; the engine
            // gives the vault a colour the next time it saves.
            Stored::Bare(path) => VaultEntry { path, color: String::new(), last_opened: 0 },
            Stored::Object(object) => VaultEntry {
                path: object.path,
                color: object.color,
                last_opened: object.last_opened,
            },
        })
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub vaults: Vec<VaultEntry>,
    /// The vault currently being read. One vault is active at a time; the tree
    /// shows its books as roots.
    #[serde(default)]
    pub active: Option<PathBuf>,
    #[serde(default)]
    pub expanded: Vec<PathBuf>,
    /// Reading size for the editor, in pixels. `None` means the default.
    #[serde(default)]
    pub editor_font_size: Option<u32>,
    /// Which theme the UI wears. `None` means the default. Not validated here:
    /// the UI falls back on its own for a name it does not know.
    #[serde(default)]
    pub theme: Option<String>,
    /// How large the chrome draws, as a percentage. Kept as a whole percent
    /// rather than a float so the file stays something a person can edit.
    #[serde(default)]
    pub ui_scale: Option<u32>,
    /// How much room the chrome gives itself, as a percentage.
    #[serde(default)]
    pub density: Option<u32>,
}

impl Config {
    /// Just the vault locations. Callers that only resolve links do not care
    /// what a vault looks like in the picker.
    pub fn vault_paths(&self) -> Vec<PathBuf> {
        self.vaults.iter().map(|entry| entry.path.clone()).collect()
    }
}

/// Loads the config. A missing file is not an error — it means nothing has been
/// configured yet, so we return the default (empty) config.
pub fn load(config_path: &Path) -> io::Result<Config> {
    let text = match std::fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error),
    };

    serde_json::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Writes the config, creating the parent directory if needed.
pub fn save(config_path: &Path, config: &Config) -> io::Result<()> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // The config holds only paths, so serialization cannot fail.
    let text = serde_json::to_string_pretty(config).expect("config serializes to JSON");

    std::fs::write(config_path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_config() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "booklet-config-{}-{}/config.json",
            std::process::id(),
            unique
        ))
    }

    #[test]
    fn missing_config_loads_default() {
        let path = temp_config();

        let config = load(&path).unwrap();

        assert!(config.vaults.is_empty());
        assert!(config.expanded.is_empty());
    }

    /// The shape every config on disk had before the vault picker. Reading it
    /// must keep the vaults; anything else loses somebody's library.
    #[test]
    fn a_config_of_bare_paths_still_loads() {
        let path = temp_config();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{ "vaults": ["/notes/personal", "/work/notes"],
                 "active": "/work/notes",
                 "expanded": ["/notes/personal/Book"] }"#,
        )
        .unwrap();

        let config = load(&path).unwrap();

        assert_eq!(
            config.vaults.iter().map(|entry| entry.path.clone()).collect::<Vec<_>>(),
            [PathBuf::from("/notes/personal"), PathBuf::from("/work/notes")]
        );
        assert_eq!(config.active, Some(PathBuf::from("/work/notes")));
        // Nothing knew these yet, so they read as unset rather than as an error.
        assert_eq!(config.vaults[0].color, "");
        assert_eq!(config.vaults[0].last_opened, 0);

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// A config half-migrated by hand, or written by a version in between.
    #[test]
    fn bare_paths_and_entries_can_share_one_list() {
        let path = temp_config();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r##"{ "vaults": ["/bare/one",
                             { "path": "/full/two", "color": "#3C5240", "last_opened": 1700 }] }"##,
        )
        .unwrap();

        let config = load(&path).unwrap();

        assert_eq!(config.vaults[0].path, PathBuf::from("/bare/one"));
        assert_eq!(config.vaults[1].color, "#3C5240");
        assert_eq!(config.vaults[1].last_opened, 1700);

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn config_round_trips() {
        let path = temp_config();
        let config = Config {
            vaults: vec![
                VaultEntry {
                    path: PathBuf::from("/notes/personal"),
                    color: "#7C3128".into(),
                    last_opened: 1700,
                },
                VaultEntry {
                    path: PathBuf::from("/work/notes"),
                    color: "#2F3E5C".into(),
                    last_opened: 1800,
                },
            ],
            active: Some(PathBuf::from("/work/notes")),
            expanded: vec![PathBuf::from("/notes/personal/Book")],
            editor_font_size: Some(20),
            theme: Some("atlas".into()),
            ui_scale: Some(115),
            density: Some(120),
        };

        save(&path, &config).unwrap();
        let loaded = load(&path).unwrap();

        assert_eq!(loaded.vaults, config.vaults);
        assert_eq!(loaded.active, config.active);
        assert_eq!(loaded.expanded, config.expanded);
        assert_eq!(loaded.theme, config.theme);
        assert_eq!(loaded.ui_scale, config.ui_scale);
        assert_eq!(loaded.density, config.density);

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
