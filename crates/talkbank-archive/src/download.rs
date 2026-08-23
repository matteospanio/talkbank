//! Downloading and extracting a corpus.
//!
//! Several things measured against the live service govern this module:
//!
//!  * **The access gate answers `200 text/html`**, not 401. The HTTP status says
//!    nothing: you have to look at the content type *and* the `PK` signature.
//!  * **Neither `Content-Length` nor `Accept-Ranges` arrive** (streamed
//!    response): progress is in bytes, with no percentage, and resuming from
//!    where a transfer stopped is not possible.
//!  * **The zip has no duplicated top-level folder**: it already holds the
//!    contents of the corpus. We create the destination folder ourselves.
//!  * **A corpus zip contains its whole subtree**, nested several levels deep
//!    too (verified on `Brown` → `Adam/`, `Eve/`, `Sarah/` and on `Demetras2`
//!    → `Jimmy/father/`). So a branch download can stop at the first corpus and
//!    never descend further.
//!  * **Media are not in the zip**: transcripts only. `McMillan` declares video
//!    on all three of its transcripts and its zip weighs 10 KB.
//!
//! Work happens in a `<destination>.incoming` folder and is moved into place
//! with a `rename` only once extraction has finished. That way **the existence
//! of the destination folder means "complete corpus"** — which is what lets
//! someone downloading forty corpora resume without redoing everything, and
//! keeps an interruption from leaving a half corpus that looks whole.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use futures_util::StreamExt;

use crate::api::{ApiError, Client};

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("signing in is required to download this corpus")]
    AuthRequired,
    #[error("this folder is not a downloadable corpus")]
    NotAvailable,
    #[error("this bank requires separate permission")]
    NeedsPermission,
    #[error("download cancelled")]
    Cancelled,
    #[error("invalid archive: {0}")]
    BadArchive(String),
    #[error("not enough disk space: at least {needed} bytes are needed")]
    NoSpace { needed: u64 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Api(#[from] ApiError),
}

/// Bytes per transcript, measured over four corpora on 2026-08-17: Brown
/// 5,094,456/214, Marrero 343,863/12, Demetras2 848,685/45, McMillan 10,726/3 —
/// 6,297,730 bytes over 274 transcripts. It only gives an order of magnitude
/// before downloading a branch, and should be presented as such.
pub const BYTES_PER_TRANSCRIPT: u64 = 23_000;

/// True when this corpus is already on disk, complete.
///
/// We can claim that because extraction is atomic: the folder only appears once
/// the work has finished.
pub fn already_there(root: &Path, path: &[String]) -> bool {
    let dest = destination(root, path);
    dest.is_dir()
        && std::fs::read_dir(&dest)
            .into_iter()
            .flatten()
            .flatten()
            .next()
            .is_some()
}

/// The temporary folder the work happens in.
///
/// Built by concatenation rather than with `with_extension`, which on a corpus
/// called `3.5` would replace the `5` instead of appending the suffix — and two
/// different corpora would end up writing over each other.
fn incoming_dir(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "corpus".into());
    dest.with_file_name(format!("{name}.incoming"))
}

/// Removes the temporary folder on every exit that is not success, a panic
/// included. Without it, an error halfway through a download would leave behind
/// folders that look like corpora.
struct Incoming {
    path: PathBuf,
    keep: bool,
}

impl Drop for Incoming {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Progress. There is no total because the server does not send one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Bytes downloaded so far.
    Downloading(u64),
    /// Files extracted so far, out of the archive's total entries.
    Extracting { done: usize, total: usize },
    Done,
}

/// Where a corpus lands: `<destination>/<archive path>/`.
///
/// It mirrors TalkBank's own layout, bank included, so corpora from different
/// banks do not get mixed up — and two same-named corpora in different banks
/// (`Eng-NA` exists in both CHILDES and PhonBank) stay apart.
pub fn destination(root: &Path, path: &[String]) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in path {
        out.push(part);
    }
    out
}

/// Downloads a corpus zip and extracts it. Reports progress, and can be
/// cancelled by returning `false` from the callback.
pub async fn corpus(
    client: &Client,
    path: &[String],
    dest_root: &Path,
    mut on_progress: impl FnMut(Progress) -> bool,
) -> Result<PathBuf, DownloadError> {
    let url = crate::api::corpus_zip_url(path);
    let resp = client
        .http()
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(DownloadError::NeedsPermission);
    }
    if !resp.status().is_success() {
        return Err(DownloadError::NotAvailable);
    }
    // The access gate arrives as 200 + text/html: this is where we find out.
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if content_type.starts_with("text/html") {
        return Err(DownloadError::AuthRequired);
    }

    let dest = destination(dest_root, path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Leftovers from an earlier abrupt exit must not be reused: we do not know
    // how far that run had got.
    let incoming = incoming_dir(&dest);
    let _ = std::fs::remove_dir_all(&incoming);
    std::fs::create_dir_all(&incoming)?;
    let mut guard = Incoming {
        path: incoming.clone(),
        keep: false,
    };

    // The zip lives inside the temporary folder: it disappears with it, and two
    // different jobs cannot share its name.
    let part = incoming.join(".download.zip");
    let mut file = std::fs::File::create(&part)?;

    let mut downloaded: u64 = 0;
    let mut first = Vec::new();
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError::Network(e.to_string()))?;
        if first.len() < 4 {
            first.extend_from_slice(&chunk[..chunk.len().min(4)]);
            if first.len() >= 4 && !first.starts_with(b"PK\x03\x04") {
                return Err(DownloadError::AuthRequired);
            }
        }
        write_chunk(&mut file, &chunk)?;
        downloaded += chunk.len() as u64;
        if !on_progress(Progress::Downloading(downloaded)) {
            return Err(DownloadError::Cancelled);
        }
    }
    file.flush().map_err(no_space)?;
    drop(file);

    if downloaded == 0 {
        return Err(DownloadError::NotAvailable);
    }

    extract(&part, &incoming, &mut on_progress)?;
    let _ = std::fs::remove_file(&part);

    // The swap: the destination appears whole or not at all.
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&incoming, &dest)?;
    guard.keep = true;

    on_progress(Progress::Done);
    Ok(dest)
}

/// Writes a chunk, telling a full disk apart from any other error: the user
/// deals with those two differently.
fn write_chunk(file: &mut std::fs::File, chunk: &[u8]) -> Result<(), DownloadError> {
    file.write_all(chunk).map_err(no_space)
}

fn no_space(e: std::io::Error) -> DownloadError {
    // `StorageFull` is the typed form; on some filesystems only a raw ENOSPC
    // arrives, so we check that too.
    let full = e.kind() == std::io::ErrorKind::StorageFull
        || e.raw_os_error() == Some(28);
    if full {
        DownloadError::NoSpace { needed: 0 }
    } else {
        DownloadError::Io(e)
    }
}

/// Extracts a zip, rejecting any path that would escape the destination.
///
/// This is not a theoretical precaution: the bytes come off the network, and a
/// zip holding `../../` would overwrite files outside the chosen folder.
pub fn extract(
    archive: &Path,
    dest: &Path,
    on_progress: &mut impl FnMut(Progress) -> bool,
) -> Result<(), DownloadError> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| DownloadError::BadArchive(e.to_string()))?;
    let total = zip.len();

    let mut written = 0usize;
    for i in 0..total {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| DownloadError::BadArchive(e.to_string()))?;
        let Some(rel) = safe_path(entry.name()) else {
            tracing::warn!("entry rejected from the archive: {}", entry.name());
            continue;
        };
        let out = dest.join(rel);

        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
            written += 1;
        }
        if !on_progress(Progress::Extracting { done: i + 1, total }) {
            return Err(DownloadError::Cancelled);
        }
    }
    if written == 0 {
        return Err(DownloadError::BadArchive(
            "the archive contains no files".into(),
        ));
    }
    Ok(())
}

/// Sanitises a path from inside the archive: drops absolute components, `..`
/// and prefixes. `None` when nothing usable is left.
fn safe_path(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for comp in Path::new(name).components() {
        match comp {
            Component::Normal(part) => out.push(part),
            // Everything else is discarded: climbing out is exactly what must
            // not be possible.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            Component::CurDir => {}
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn paths_escaping_the_destination_are_rejected() {
        assert_eq!(safe_path("Adam/adam01.cha"), Some(PathBuf::from("Adam/adam01.cha")));
        assert_eq!(safe_path("./a.cha"), Some(PathBuf::from("a.cha")));

        assert_eq!(safe_path("../outside.cha"), None);
        assert_eq!(safe_path("a/../../outside.cha"), None);
        assert_eq!(safe_path("/etc/passwd"), None);
        assert_eq!(safe_path(".."), None);
        assert_eq!(safe_path(""), None);
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            for (name, data) in entries {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn a_normal_archive_extracts_into_the_destination() {
        let dir = tempdir::TempDir::new("talkbank-zip").unwrap();
        let zip_path = dir.path().join("a.zip");
        std::fs::write(&zip_path, zip_with(&[
            ("0metadata.cdc", b"x"),
            ("Adam/adam01.cha", b"@UTF8\n"),
            ("Eve/eve01.cha", b"@UTF8\n"),
        ]))
        .unwrap();

        let dest = dir.path().join("out");
        let mut seen = Vec::new();
        extract(&zip_path, &dest, &mut |p| {
            seen.push(p);
            true
        })
        .unwrap();

        assert!(dest.join("Adam/adam01.cha").is_file());
        assert!(dest.join("Eve/eve01.cha").is_file());
        assert!(dest.join("0metadata.cdc").is_file());
        assert!(matches!(seen.last(), Some(Progress::Extracting { done: 3, total: 3 })));
    }

    #[test]
    fn a_malicious_archive_does_not_write_outside_the_destination() {
        let dir = tempdir::TempDir::new("talkbank-zip").unwrap();
        let zip_path = dir.path().join("evil.zip");
        std::fs::write(&zip_path, zip_with(&[
            ("../../evil.cha", b"nope"),
            ("/tmp/absolute-evil.cha", b"nope"),
            ("good.cha", b"@UTF8\n"),
        ]))
        .unwrap();

        let dest = dir.path().join("out");
        extract(&zip_path, &dest, &mut |_| true).unwrap();

        assert!(dest.join("good.cha").is_file(), "the legitimate file must be extracted");
        // Nothing may have landed outside: not above the destination, not in /tmp.
        assert!(!dir.path().join("evil.cha").exists());
        assert!(!dir.path().parent().unwrap().join("evil.cha").exists());
        assert!(!Path::new("/tmp/absolute-evil.cha").exists());
    }

    #[test]
    fn an_archive_with_no_files_is_an_error() {
        let dir = tempdir::TempDir::new("talkbank-zip").unwrap();
        let zip_path = dir.path().join("empty.zip");
        std::fs::write(&zip_path, zip_with(&[("../outside.cha", b"x")])).unwrap();
        let err = extract(&zip_path, &dir.path().join("out"), &mut |_| true).unwrap_err();
        assert!(matches!(err, DownloadError::BadArchive(_)));
    }

    #[test]
    fn extraction_can_be_cancelled() {
        let dir = tempdir::TempDir::new("talkbank-zip").unwrap();
        let zip_path = dir.path().join("a.zip");
        std::fs::write(&zip_path, zip_with(&[("a.cha", b"x"), ("b.cha", b"y")])).unwrap();
        let err = extract(&zip_path, &dir.path().join("out"), &mut |_| false).unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled));
    }

    #[test]
    fn the_temporary_folder_does_not_truncate_names_with_a_dot() {
        // `with_extension` on "3.5" would replace the "5": two different corpora
        // would end up writing over each other.
        assert_eq!(
            incoming_dir(Path::new("/data/childes/Biling/Hoff/3.5")),
            PathBuf::from("/data/childes/Biling/Hoff/3.5.incoming")
        );
        assert_eq!(
            incoming_dir(Path::new("/data/ca/ATC")),
            PathBuf::from("/data/ca/ATC.incoming")
        );
        assert_ne!(
            incoming_dir(Path::new("/d/3")),
            incoming_dir(Path::new("/d/3.5"))
        );
    }

    #[test]
    fn an_existing_non_empty_folder_counts_as_already_downloaded() {
        let d = tempdir::TempDir::new("talkbank-dl").unwrap();
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let path = p(&["childes", "Eng-NA", "Brown"]);

        assert!(!already_there(d.path(), &path), "nothing on disk");

        let dest = destination(d.path(), &path);
        std::fs::create_dir_all(&dest).unwrap();
        assert!(
            !already_there(d.path(), &path),
            "an empty folder is not a downloaded corpus"
        );
        std::fs::write(dest.join("adam01.cha"), "@UTF8\n").unwrap();
        assert!(already_there(d.path(), &path));
    }

    #[test]
    fn the_guard_removes_the_temporary_folder() {
        let d = tempdir::TempDir::new("talkbank-dl").unwrap();
        let inc = d.path().join("Brown.incoming");
        std::fs::create_dir_all(&inc).unwrap();
        std::fs::write(inc.join(".download.zip"), b"x").unwrap();
        {
            let _g = Incoming {
                path: inc.clone(),
                keep: false,
            };
        }
        assert!(!inc.exists(), "an error exit leaves nothing behind");

        std::fs::create_dir_all(&inc).unwrap();
        {
            let mut g = Incoming {
                path: inc.clone(),
                keep: false,
            };
            g.keep = true;
        }
        assert!(inc.exists(), "on success the folder stays, ready for the rename");
    }

    #[test]
    fn a_full_disk_is_told_apart_from_any_other_error() {
        let full = no_space(std::io::Error::from_raw_os_error(28));
        assert!(matches!(full, DownloadError::NoSpace { .. }));
        let other = no_space(std::io::Error::other("whatever"));
        assert!(matches!(other, DownloadError::Io(_)));
    }

    #[test]
    fn the_destination_mirrors_talkbanks_own_layout() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            destination(Path::new("/data"), &p(&["childes", "Eng-NA", "Brown"])),
            PathBuf::from("/data/childes/Eng-NA/Brown")
        );
        // two same-named corpora in different banks do not collide
        assert_ne!(
            destination(Path::new("/data"), &p(&["childes", "Eng-NA"])),
            destination(Path::new("/data"), &p(&["phon", "Eng-NA"]))
        );
        // and banks with no collection level give a shorter path
        assert_eq!(
            destination(Path::new("/data"), &p(&["ca", "ATC"])),
            PathBuf::from("/data/ca/ATC")
        );
    }
}
