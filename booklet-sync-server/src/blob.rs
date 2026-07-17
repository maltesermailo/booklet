//! The content-addressed blob store, with Git-style delta chains.
//!
//! Notes are revised over and over, so storing each version whole would grow
//! history by (versions × note size). Instead a version is kept as a **delta
//! against the previous version**, with a **full checkpoint every K versions** to
//! bound how far a read has to walk. History is linear per note, so the base of
//! a delta is simply the prior version — no similarity search (Git's hard part)
//! is needed.
//!
//! The store owns only *bytes on disk*; the chain metadata ([`Meta`]) is the
//! caller's to persist (in Postgres, a later slice) and to hand back on a read.
//! Keeping the metadata out of the store is what lets it — and its whole delta
//! machinery — be tested with an in-memory map, no database in sight.
//!
//! Everything is addressed by the SHA-256 of the *original* content, the same
//! digest `booklet-core` computes client-side; the delta encoding is invisible
//! above [`BlobStore::get`]/[`BlobStore::put`].

use qbsdiff::{Bsdiff, Bspatch};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

/// zstd level for full checkpoints. High, because notes are small and the point
/// is to save space; the cost is microseconds at this scale.
const ZSTD_LEVEL: i32 = 19;

/// How a blob's bytes on disk are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// A whole, zstd-compressed copy of the content.
    Full,
    /// A binary delta (bsdiff) against [`Meta::base`].
    Delta,
}

/// A blob's place in its delta chain. The store returns this from [`BlobStore::put`]
/// for the caller to persist, and reads it back through the resolver in
/// [`BlobStore::get`]; the store keeps none of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub encoding: Encoding,
    /// The base hash a `Delta` is against; `None` for a `Full`.
    pub base: Option<String>,
    /// Number of deltas back to a full. A `Full` is depth 0; the chain never
    /// exceeds `checkpoint_interval - 1`.
    pub depth: u32,
    /// Bytes actually written to disk.
    pub stored_size: u64,
}

/// The previous version a new one deltas against: its hash, its (already
/// reconstructed) content, and its depth.
pub struct Base<'a> {
    pub hash: &'a str,
    pub content: &'a [u8],
    pub depth: u32,
}

/// A directory of content-addressed blobs, sharded by the first byte of the hash.
pub struct BlobStore {
    root: PathBuf,
    /// K — a full checkpoint is written once a chain would reach this depth.
    checkpoint_interval: u32,
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>, checkpoint_interval: u32) -> Self {
        Self { root: root.into(), checkpoint_interval }
    }

    /// Whether this hash is already stored — the caller's dedup check before an
    /// upload, so identical content is never stored twice.
    pub fn has(&self, hash: &str) -> bool {
        self.path(hash).exists()
    }

    /// Stores `content` (which must hash to `hash`) and returns its chain
    /// metadata for the caller to persist.
    ///
    /// With no `base`, or when the chain would reach a checkpoint, the content is
    /// stored whole; otherwise it is stored as a delta against `base` — but only
    /// if that delta is actually smaller than a fresh full, so a total rewrite
    /// (whose delta would be larger than the content) becomes a checkpoint rather
    /// than bloat.
    ///
    /// Precondition: call only when `!has(hash)`. Storing is a no-op the caller
    /// skips for content already present.
    pub fn put(&self, hash: &str, content: &[u8], base: Option<Base>) -> io::Result<Meta> {
        if self::hash(content) != hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "blob content does not match its hash",
            ));
        }

        let full = zstd::encode_all(content, ZSTD_LEVEL)?;

        let (meta, bytes) = match base {
            Some(base) if base.depth + 1 < self.checkpoint_interval => {
                let delta = diff(base.content, content)?;
                if delta.len() < full.len() {
                    let meta = Meta {
                        encoding: Encoding::Delta,
                        base: Some(base.hash.to_string()),
                        depth: base.depth + 1,
                        stored_size: delta.len() as u64,
                    };
                    (meta, delta)
                } else {
                    (checkpoint_meta(full.len()), full)
                }
            }
            _ => (checkpoint_meta(full.len()), full),
        };

        self.write_atomic(hash, &bytes)?;
        Ok(meta)
    }

    /// Reconstructs the content for `hash`, walking its delta chain back to a full
    /// via `resolve` (which supplies each blob's [`Meta`]). The result is verified
    /// against `hash`, so a corrupt or mis-linked chain errors rather than
    /// returning wrong bytes.
    pub fn get(&self, hash: &str, resolve: &impl Fn(&str) -> Option<Meta>) -> io::Result<Vec<u8>> {
        let content = self.reconstruct(hash, resolve)?;

        if self::hash(&content) != hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("reconstructed blob {hash} does not match its hash"),
            ));
        }

        Ok(content)
    }

    fn reconstruct(&self, hash: &str, resolve: &impl Fn(&str) -> Option<Meta>) -> io::Result<Vec<u8>> {
        let meta = resolve(hash).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no metadata for blob {hash}"))
        })?;
        let raw = fs::read(self.path(hash))?;

        match meta.encoding {
            Encoding::Full => Ok(zstd::decode_all(&raw[..])?),
            Encoding::Delta => {
                let base_hash = meta.base.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "delta blob without a base")
                })?;
                // Only the base is verified at the top-level get; intermediate
                // reconstructions are trusted and re-hashed once at the end.
                let base = self.reconstruct(&base_hash, resolve)?;
                patch(&base, &raw)
            }
        }
    }

    /// Deletes any blob whose hash is not in `keep`, returning how many were
    /// removed. This is the orphan sweep — junk uploaded but never referenced.
    ///
    /// `keep` **must** contain every hash still referenced by a version, which
    /// includes every delta base (a base is always a referenced version), so a
    /// prune can never strand a chain.
    pub fn prune(&self, keep: &HashSet<String>) -> io::Result<usize> {
        let mut removed = 0;

        for hash in self.list()? {
            if !keep.contains(&hash) {
                self.remove(&hash)?;
                removed += 1;
            }
        }

        Ok(removed)
    }

    /// Removes one blob. Idempotent: a hash already gone is not an error.
    pub fn remove(&self, hash: &str) -> io::Result<()> {
        match fs::remove_file(self.path(hash)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Every stored hash, for the orphan sweep to diff against the referenced set.
    pub fn list(&self) -> io::Result<Vec<String>> {
        let mut hashes = Vec::new();

        if !self.root.exists() {
            return Ok(hashes);
        }

        for shard in fs::read_dir(&self.root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            // Shard directories are exactly two hex chars; skip anything else
            // (e.g. the sibling `.staging` area the store keeps under the root).
            if shard.file_name().to_string_lossy().len() != 2 {
                continue;
            }

            for entry in fs::read_dir(shard.path())? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                // Skip a half-written temp file from an interrupted write.
                if !name.ends_with(".tmp") {
                    hashes.push(name);
                }
            }
        }

        Ok(hashes)
    }

    /// `root/<first two hex chars>/<full hash>`, so no directory holds every blob.
    fn path(&self, hash: &str) -> PathBuf {
        self.root.join(&hash[..2]).join(hash)
    }

    /// Writes to a temp file then renames, so a reader never sees a partial blob.
    fn write_atomic(&self, hash: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(hash);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes)?;
        fs::rename(&temp, &path)
    }
}

fn checkpoint_meta(stored_size: usize) -> Meta {
    Meta { encoding: Encoding::Full, base: None, depth: 0, stored_size: stored_size as u64 }
}

/// The SHA-256 of some bytes, hex-encoded — the content address. Public because
/// route handlers hash uploaded bytes to check them against the URL.
pub fn hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn diff(source: &[u8], target: &[u8]) -> io::Result<Vec<u8>> {
    let mut patch = Vec::new();
    Bsdiff::new(source, target).compare(io::Cursor::new(&mut patch))?;
    Ok(patch)
}

fn patch(source: &[u8], patch: &[u8]) -> io::Result<Vec<u8>> {
    let patcher = Bspatch::new(patch)?;

    let mut target = Vec::new();
    patcher.apply(source, io::Cursor::new(&mut target))?;

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_root() -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("booklet-blob-{}-{}", std::process::id(), unique))
    }

    /// Deterministic, poorly-compressible text — stands in for real prose, where
    /// a full is not tiny and so a small-edit delta genuinely wins. (Repetitive
    /// text compresses so well that a full beats any delta, which is correct but
    /// makes for a misleading fixture.)
    fn varied(len: usize, seed: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut state = seed | 1;
        while out.len() < len {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            out.push(b'a' + ((state >> 33) % 26) as u8);
            if out.len() % 7 == 0 {
                out.push(b' ');
            }
        }
        out.truncate(len);
        out
    }

    /// A base document with a short distinct suffix appended — successive edits
    /// share the whole body, so the delta between them is tiny.
    fn revision(base: &[u8], edit: usize) -> Vec<u8> {
        let mut content = base.to_vec();
        content.extend_from_slice(format!("\nappended note {edit}\n").as_bytes());
        content
    }

    /// Drives a note's version chain through the store, holding the metadata the
    /// server would keep in Postgres in a plain map instead.
    struct Chain {
        store: BlobStore,
        metas: HashMap<String, Meta>,
        versions: Vec<(String, Vec<u8>)>,
    }

    impl Chain {
        fn new(checkpoint_interval: u32) -> Self {
            Self {
                store: BlobStore::new(temp_root(), checkpoint_interval),
                metas: HashMap::new(),
                versions: Vec::new(),
            }
        }

        fn commit(&mut self, content: &[u8]) -> String {
            let digest = hash(content);
            let base = self.versions.last().map(|(hash, content)| Base {
                hash,
                content,
                depth: self.metas[hash].depth,
            });

            let meta = self.store.put(&digest, content, base).unwrap();
            self.metas.insert(digest.clone(), meta);
            self.versions.push((digest.clone(), content.to_vec()));

            digest
        }

        fn read(&self, hash: &str) -> Vec<u8> {
            self.store.get(hash, &|h| self.metas.get(h).cloned()).unwrap()
        }
    }

    #[test]
    fn a_single_full_blob_round_trips() {
        let mut chain = Chain::new(50);
        let content = b"# Note\n\nJust the one version.\n";

        let hash = chain.commit(content);

        assert_eq!(chain.metas[&hash].encoding, Encoding::Full);
        assert_eq!(chain.read(&hash), content);
    }

    #[test]
    fn a_delta_chain_reconstructs_every_version_and_saves_space() {
        let mut chain = Chain::new(50);
        let base = varied(2000, 7);

        let mut hashes = Vec::new();
        let mut contents = Vec::new();
        for edit in 0..8 {
            let content = revision(&base, edit);
            hashes.push(chain.commit(&content));
            contents.push(content);
        }

        // Every version comes back byte-for-byte.
        for (hash, content) in hashes.iter().zip(&contents) {
            assert_eq!(&chain.read(hash), content);
        }

        // First is a full; the rest are deltas, each far smaller than a full.
        assert_eq!(chain.metas[&hashes[0]].encoding, Encoding::Full);
        let full_size = chain.metas[&hashes[0]].stored_size;
        for hash in &hashes[1..] {
            assert_eq!(chain.metas[hash].encoding, Encoding::Delta);
            assert!(chain.metas[hash].stored_size < full_size / 4);
        }
    }

    #[test]
    fn a_full_checkpoint_lands_every_k_versions() {
        let mut chain = Chain::new(3);
        let base = varied(2000, 11);

        let mut hashes = Vec::new();
        for edit in 0..6 {
            hashes.push(chain.commit(&revision(&base, edit)));
        }

        // depth < K=3, so: full, delta(1), delta(2), then depth would be 3 -> full.
        let depths: Vec<_> = hashes.iter().map(|hash| chain.metas[hash].depth).collect();
        assert_eq!(depths, [0, 1, 2, 0, 1, 2]);
        assert_eq!(chain.metas[&hashes[3]].encoding, Encoding::Full);

        // The checkpoint does not break reconstruction across it.
        assert_eq!(chain.read(&hashes[5]), revision(&base, 5));
    }

    #[test]
    fn a_stored_blob_is_never_larger_than_a_plain_full() {
        let mut chain = Chain::new(50);
        chain.commit(&varied(3000, 1));

        // An unrelated rewrite: a delta against the old version buys nothing, so
        // the store must fall back to a full rather than store a bloated delta.
        let rewrite = varied(3000, 2);
        let hash = chain.commit(&rewrite);

        let plain_full = zstd::encode_all(&rewrite[..], 19).unwrap();
        assert!(chain.metas[&hash].stored_size <= plain_full.len() as u64);
        assert_eq!(chain.read(&hash), rewrite);
    }

    #[test]
    fn a_corrupt_blob_is_caught_not_served() {
        let mut chain = Chain::new(50);
        let hash = chain.commit(b"# Note\n\ncontent that will be corrupted on disk\n");

        // Scribble over the stored bytes.
        let path = chain.store.path(&hash);
        fs::write(&path, b"garbage that is not a valid zstd frame").unwrap();

        assert!(chain.store.get(&hash, &|h| chain.metas.get(h).cloned()).is_err());
    }

    #[test]
    fn prune_removes_orphans_and_leaves_chains_intact() {
        let mut chain = Chain::new(50);
        let body = "kept content ".repeat(30);
        let v1 = chain.commit(format!("{body} one").as_bytes());
        let v2 = chain.commit(format!("{body} two").as_bytes());

        // An orphan: stored, referenced by nothing.
        let orphan = hash(b"unreferenced junk");
        chain.store.put(&orphan, b"unreferenced junk", None).unwrap();

        let keep: HashSet<String> = [v1.clone(), v2.clone()].into_iter().collect();
        let removed = chain.store.prune(&keep).unwrap();

        assert_eq!(removed, 1);
        assert!(!chain.store.has(&orphan));
        // v2 is a delta on v1; both survive and still reconstruct.
        assert_eq!(chain.read(&v2), format!("{body} two").as_bytes());
    }

    #[test]
    fn put_rejects_content_that_does_not_match_its_hash() {
        let store = BlobStore::new(temp_root(), 50);

        let result = store.put(&hash(b"the real content"), b"different content", None);

        assert!(result.is_err());
    }
}
