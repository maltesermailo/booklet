//! Persistence of the library configuration: the vault paths and which folders
//! are expanded. Stored as plain JSON, matching Booklet's plain-files-on-disk
//! principle.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub vaults: Vec<PathBuf>,
    /// The vault currently being read. One vault is active at a time; the tree
    /// shows its books as roots.
    #[serde(default)]
    pub active: Option<PathBuf>,
    #[serde(default)]
    pub expanded: Vec<PathBuf>,
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

    #[test]
    fn config_round_trips() {
        let path = temp_config();
        let config = Config {
            vaults: vec![PathBuf::from("/notes/personal"), PathBuf::from("/work/notes")],
            active: Some(PathBuf::from("/work/notes")),
            expanded: vec![PathBuf::from("/notes/personal/Book")],
        };

        save(&path, &config).unwrap();
        let loaded = load(&path).unwrap();

        assert_eq!(loaded.vaults, config.vaults);
        assert_eq!(loaded.active, config.active);
        assert_eq!(loaded.expanded, config.expanded);

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
