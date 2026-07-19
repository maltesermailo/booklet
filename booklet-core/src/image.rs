//! Bringing image files into a note's folder.
//!
//! Images live beside the note that references them, so a vault stays a
//! self-contained, portable graph (the same reason links never cross vaults) and
//! sync carries an image as it carries a note. Two ways in: copy an existing file
//! (drag-and-drop, the file picker) keeping its name, or write raw bytes (a
//! clipboard paste, which has no file name) under a content-hashed name.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;

/// The image extensions Booklet handles — what the add methods accept and what
/// sync treats as an image rather than a note.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"];

/// Whether a path names an image file Booklet handles (case-insensitive).
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

/// Copies `source` into `folder`, keeping its file name, and returns the name to
/// write into the `![](…)` link (relative to the note beside it). A name already
/// taken is reused when the bytes are identical (the same picture dropped twice)
/// and disambiguated with a counter otherwise (`shot.png` → `shot-1.png`).
pub fn import_file(folder: &Path, source: &Path) -> io::Result<String> {
    let bytes = fs::read(source)?;
    let stem = source.file_stem().and_then(|stem| stem.to_str()).unwrap_or("image");
    let extension = source.extension().and_then(|extension| extension.to_str()).unwrap_or("");

    save_unique(folder, stem, extension, &bytes)
}

/// Writes raw image `bytes` into `folder` under a content-hashed name (a paste has
/// no file name of its own), so an identical paste dedups to a single file.
/// Returns the file name for the link.
pub fn save_bytes(folder: &Path, bytes: &[u8], extension: &str) -> io::Result<String> {
    let digest = Sha256::digest(bytes);
    let stem = format!("image-{:x}", digest);

    save_unique(folder, &stem[..stem.len().min(18)], extension, bytes)
}

/// Writes `bytes` into `folder` under `stem.extension`, stepping the stem with a
/// counter until it lands on a free name or one already holding these exact bytes.
fn save_unique(folder: &Path, stem: &str, extension: &str, bytes: &[u8]) -> io::Result<String> {
    let mut candidate = with_extension(stem, extension);
    let mut suffix = 1;

    loop {
        let path = folder.join(&candidate);
        match fs::read(&path) {
            // The name is taken by this very image — reuse it.
            Ok(existing) if existing == bytes => return Ok(candidate),
            // Taken by a different image — try the next numbered name.
            Ok(_) => {
                candidate = with_extension(&format!("{stem}-{suffix}"), extension);
                suffix += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::write(&path, bytes)?;
                return Ok(candidate);
            }
            Err(error) => return Err(error),
        }
    }
}

fn with_extension(stem: &str, extension: &str) -> String {
    if extension.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{extension}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("booklet-image-{}-{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn is_image_matches_by_extension_case_insensitively() {
        assert!(is_image(Path::new("a/b/photo.PNG")));
        assert!(is_image(Path::new("shot.jpeg")));
        assert!(!is_image(Path::new("note.md")));
        assert!(!is_image(Path::new("booklet.json")));
    }

    #[test]
    fn import_keeps_the_name_then_dedups_and_disambiguates() {
        let src = temp_dir();
        let dst = temp_dir();
        let source = src.join("shot.png");
        fs::write(&source, b"first-image").unwrap();

        // First import keeps the name.
        assert_eq!(import_file(&dst, &source).unwrap(), "shot.png");

        // The same bytes under the same name reuse the file, not a copy.
        assert_eq!(import_file(&dst, &source).unwrap(), "shot.png");

        // A different image wanting the same name gets a counter.
        let other = src.join("shot.png");
        fs::write(&other, b"second-image").unwrap();
        assert_eq!(import_file(&dst, &other).unwrap(), "shot-1.png");
    }

    #[test]
    fn save_bytes_hashes_the_name_and_dedups_identical_pastes() {
        let dst = temp_dir();

        let first = save_bytes(&dst, b"clipboard-bytes", "png").unwrap();
        let again = save_bytes(&dst, b"clipboard-bytes", "png").unwrap();
        assert_eq!(first, again); // same content → same hashed name, one file

        let different = save_bytes(&dst, b"other-bytes", "png").unwrap();
        assert_ne!(first, different);
        assert!(first.starts_with("image-") && first.ends_with(".png"));
    }
}
