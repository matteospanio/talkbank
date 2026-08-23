//! Esecuzione dei programmi CLAN.
//!
//! The CLAN programs behave like they do in a terminal only when stdin *and*
//! stdout are ttys: with a non-tty stdin they ignore the filenames on the
//! command line and read standard input instead, and with a non-tty stdout
//! they refuse `+f` ("can't be used with file redirect"). So they run on a
//! pseudo-terminal. stderr stays on a plain pipe, because CLAN writes the run
//! header and the diagnostics there while the real results go to stdout;
//! keeping them apart is what lets the UI show two separate tabs.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// Output past this size is truncated: some analyses over whole corpora produce
/// tens of megabytes that nobody is going to read on screen.
const MAX_OUT: usize = 8 << 20;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("program \u{201c}{0}\u{201d} not found in {1}")]
    NotFound(String, PathBuf),
    #[error("invalid program name: \u{201c}{0}\u{201d}")]
    BadName(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct RunOutput {
    /// Risultati dell'analisi (stdout, dal pty).
    pub stdout: String,
    /// Intestazione e diagnostica (stderr).
    pub stderr: String,
    pub exit_code: i32,
    /// Files that appeared in the working directory while the program ran.
    pub created: Vec<String>,
    pub seconds: f64,
    pub truncated: bool,
}

/// A command name has to be a CLAN program sitting next to us, not a path: this
/// is the only place where a name arrives from outside.
fn resolve(bin_dir: &Path, cmd: &str) -> Result<PathBuf, RunError> {
    if cmd.is_empty()
        || cmd.starts_with('.')
        || !cmd
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RunError::BadName(cmd.to_string()));
    }
    let path = bin_dir.join(cmd);
    if !path.is_file() {
        return Err(RunError::NotFound(cmd.to_string(), bin_dir.to_path_buf()));
    }
    Ok(path)
}

fn snapshot(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The tty turns every `\n` into `\r\n` on the way out; undo that.
fn undo_crlf(mut s: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\r' && s.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(s[i]);
        i += 1;
    }
    s.clear();
    out
}

fn read_capped(mut r: impl Read) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut truncated = false;
    loop {
        // On a pty master the child's exit shows up as EIO, not as EOF.
        match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() < MAX_OUT {
                    let room = MAX_OUT - buf.len();
                    buf.extend_from_slice(&chunk[..n.min(room)]);
                    if n > room {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (buf, truncated)
}

/// Runs a CLAN program in the given directory. Blocking: call it from a worker
/// thread, never from the UI thread.
pub fn run(bin_dir: &Path, cmd: &str, args: &[String], cwd: &Path) -> Result<RunOutput, RunError> {
    let prog = resolve(bin_dir, cmd)?;
    let before = snapshot(cwd);
    let started = Instant::now();

    let pty = nix::pty::openpty(None, None).map_err(std::io::Error::from)?;
    let slave_fd = pty.slave.as_raw_fd();

    let mut command = Command::new(&prog);
    command
        .args(args.iter().map(OsStr::new))
        .current_dir(cwd)
        .stderr(Stdio::piped());

    // SAFETY: between fork and exec we only call async-signal-safe functions.
    // dup2 clears the CLOEXEC flag on descriptors 0 and 1, so they survive the
    // exec even when the original descriptor had it set.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            for target in [0, 1] {
                if libc::dup2(slave_fd, target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }

    let mut child = command.spawn()?;
    // Close our copy of the slave: while it stays open, reading from the
    // master never finishes.
    drop(pty.slave);

    let stderr = child.stderr.take().expect("stderr requested as a pipe");
    let err_thread = std::thread::spawn(move || read_capped(stderr));

    let master = std::fs::File::from(pty.master);
    let (out_bytes, out_trunc) = read_capped(master);

    let status = child.wait()?;
    let (err_bytes, err_trunc) = err_thread.join().unwrap_or_default();

    let exit_code = status.code().unwrap_or_else(|| {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(0)
    });

    let after = snapshot(cwd);
    let created = after.difference(&before).cloned().collect();

    Ok(RunOutput {
        stdout: String::from_utf8_lossy(&undo_crlf(out_bytes)).into_owned(),
        stderr: String::from_utf8_lossy(&err_bytes).into_owned(),
        exit_code,
        created,
        seconds: started.elapsed().as_secs_f64(),
        truncated: out_trunc || err_trunc,
    })
}

/// The program's own help text: running it with no arguments prints the full
/// usage. Preferred over a hand-written list, which would drift out of date.
pub fn usage(bin_dir: &Path, cmd: &str) -> Result<String, RunError> {
    let tmp = std::env::temp_dir();
    let out = run(bin_dir, cmd, &[], &tmp)?;
    Ok(if out.stdout.trim().is_empty() {
        out.stderr
    } else {
        out.stdout
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_names_are_rejected() {
        let d = Path::new("/nonexistent");
        for bad in ["", "../freq", "a/b", ".hidden", "freq;rm"] {
            assert!(
                matches!(resolve(d, bad), Err(RunError::BadName(_))),
                "\u{201c}{bad}\u{201d} should have been rejected"
            );
        }
    }

    #[test]
    fn a_missing_program_differs_from_an_invalid_name() {
        let d = Path::new("/nonexistent");
        assert!(matches!(resolve(d, "freq"), Err(RunError::NotFound(..))));
    }

    #[test]
    fn terminal_crlf_is_undone() {
        let got = undo_crlf(b"line one\r\nline two\r\n".to_vec());
        assert_eq!(got, b"line one\nline two\n");
        // a lone \r is left alone: it can be data, not formatting
        assert_eq!(undo_crlf(b"a\rb".to_vec()), b"a\rb");
    }
}
