//! Client engine: runs the CLAN programs and reads CHAT files.

pub mod catalog;
pub mod chat;
pub mod runner;
pub mod validate;

pub use chat::{FileInfo, Speaker};
pub use runner::{RunError, RunOutput};

use std::path::PathBuf;

/// Where the 70 CLAN programs live. In order: environment variable (for
/// developers and tests), the build directory next to the executable, the
/// executable's own directory (installed layout), and finally the PATH.
pub fn find_bin_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAN_BIN") {
        let p = PathBuf::from(dir);
        if p.join("freq").is_file() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    for rel in ["", "../../build", "../../../build", "../build"] {
        let p = if rel.is_empty() {
            exe_dir.to_path_buf()
        } else {
            exe_dir.join(rel)
        };
        if p.join("freq").is_file() {
            return p.canonicalize().ok();
        }
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .find(|p| p.join("freq").is_file())
}
