//! The device credential on disk.
//!
//! A device token is a live bearer credential, so it lives in its own file with
//! `0600` permissions — never in `vaults.json`, which is hand-editable and the
//! kind of thing pasted into a bug report (CLAUDE.md). Plaintext at rest is
//! accepted: anything running as the user can already read it.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// What sign-in yields and the client needs: which server, as whom, with what
/// token.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct Credentials {
    pub server_url: String,
    pub handle: String,
    pub token: String,
}

/// Reads the credential file, or `None` if the device is not signed in.
pub fn load(path: &Path) -> io::Result<Option<Credentials>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Writes the credential file with `0600` permissions.
pub fn save(path: &Path, credentials: &Credentials) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // A struct of strings, so serialization cannot fail.
    let text = serde_json::to_string_pretty(credentials).expect("credentials serialize to JSON");
    std::fs::write(path, text)?;

    restrict(path)
}

/// Removes the credential file on sign-out. A missing file is not an error.
pub fn clear(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn restrict(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_path() -> std::path::PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("booklet-secret-{}-{}", std::process::id(), unique))
            .join("sync-token.json")
    }

    #[test]
    fn a_missing_credential_reads_as_none() {
        assert_eq!(load(&temp_path()).unwrap(), None);
    }

    #[test]
    fn credentials_round_trip_and_the_file_is_private() {
        let path = temp_path();
        let credentials = Credentials {
            server_url: "https://notes.example".into(),
            handle: "alice".into(),
            token: "deadbeef".into(),
        };

        save(&path, &credentials).unwrap();

        assert_eq!(load(&path).unwrap(), Some(credentials));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "credential file must be owner-only");
        }
    }
}
