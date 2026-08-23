//! On-disk cache of the catalogue.
//!
//! The tree is 4.3 MB and changes rarely: re-fetching it on every start would be
//! wasteful, and would make the app useless without a network. The policy is
//! "show what you have straight away, refresh in the background if it is old".

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Past this age the tree is re-fetched in the background. Seven days: new
/// corpora arrive months apart.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub fn dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("talkbank")
}

pub fn tree_path() -> PathBuf {
    dir().join("talkbank-tree.json")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Nothing is cached.
    Absent,
    /// Cached and recent.
    Fresh,
    /// Cached but old: fine to show while it refreshes.
    Stale,
}

pub fn freshness(path: &std::path::Path) -> Freshness {
    let Ok(meta) = std::fs::metadata(path) else {
        return Freshness::Absent;
    };
    let age = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .unwrap_or(Duration::ZERO);
    if age > MAX_AGE {
        Freshness::Stale
    } else {
        Freshness::Fresh
    }
}

/// When the cached tree was last updated.
pub fn updated_at(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Stores the raw response bytes. Writes to a temporary file and renames, so an
/// interruption cannot leave behind a truncated cache that fails to parse.
pub fn store(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Reads the tree from the cache. `None` when it is missing or unreadable: both
/// cases fall back to the network, so there is no need to tell them apart.
pub fn load(path: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("catalogue cache unreadable ({e}); it will be re-fetched");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn absent_fresh_and_stale_are_distinguished() {
        let dir = tempdir::TempDir::new("talkbank-cache").unwrap();
        let p = dir.path().join("tree.json");
        assert_eq!(freshness(&p), Freshness::Absent);

        store(&p, br#"{"respMsg":{}}"#).unwrap();
        assert_eq!(freshness(&p), Freshness::Fresh);

        // Back-date the file to simulate a cache that has aged out.
        let old = SystemTime::now() - MAX_AGE - Duration::from_secs(60);
        let f = fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(old).unwrap();
        assert_eq!(freshness(&p), Freshness::Stale);
    }

    #[test]
    fn it_stores_and_reads_back() {
        let dir = tempdir::TempDir::new("talkbank-cache").unwrap();
        let p = dir.path().join("nested/tree.json");
        store(&p, br#"{"respMsg":{"childes":{}}}"#).unwrap();
        let v = load(&p).expect("read back");
        assert!(v.get("respMsg").is_some());
        // the temporary file must not be left lying around
        assert!(!p.with_extension("tmp").exists());
    }

    #[test]
    fn a_corrupt_cache_does_not_bring_the_app_down() {
        let dir = tempdir::TempDir::new("talkbank-cache").unwrap();
        let p = dir.path().join("tree.json");
        store(&p, b"{ not json").unwrap();
        assert!(load(&p).is_none(), "a corrupt cache is ignored, not propagated");
    }
}
