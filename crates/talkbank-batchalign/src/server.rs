//! Finding, starting and stopping the local Batchalign3 server.
//!
//! Batchalign3 is **optional**: the app has to work in full without it, and say
//! so clearly instead of failing. "Not installed" is therefore the first state
//! most people see, and it is treated as normal rather than as an error.
//!
//! The server is never started when the app opens: on first use it downloads
//! several gigabytes of models, and nobody expects that as a side effect of a
//! double click.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

/// The names the binary may go by.
const BINARY_NAMES: [&str; 2] = ["batchalign3", "batchalign"];

/// Default port of their control plane, the one their own desktop app uses.
pub const DEFAULT_PORT: u16 = 18000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// The binary is missing: the section explains itself and offers to install.
    NotInstalled,
    /// Present but not listening: we can start it ourselves.
    Installed(PathBuf),
    /// Already listening: we attach instead of starting a second one.
    Running(u16),
}

/// Looks for the binary on the PATH.
pub fn find_binary() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in BINARY_NAMES {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Control-plane URL for a port.
pub fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// A server we started. Dropping it shuts it down: a process left behind eats
/// memory and holds the port on the next start.
pub struct Server {
    child: Option<std::process::Child>,
    port: u16,
}

impl Server {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        base_url(self.port)
    }

    /// Runs `batchalign3 serve start --foreground --port N`.
    ///
    /// `--foreground` keeps the process our child: in daemon mode it would
    /// detach and we could no longer shut it down.
    pub fn spawn(binary: &std::path::Path, port: u16) -> std::io::Result<Server> {
        let child = std::process::Command::new(binary)
            .args(["serve", "start", "--foreground", "--port"])
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Server {
            child: Some(child),
            port,
        })
    }

    /// If the process died on its own, returns its exit status.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.as_mut()?.try_wait().ok().flatten()
    }

    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Waits for the server to answer on `/health`, with a time limit.
///
/// The first start loads the models and can take a long while: the limit is
/// generous, and the caller must show that work is happening rather than block
/// the interface.
pub async fn wait_until_healthy(
    http: &reqwest::Client,
    port: u16,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    let url = format!("{}/health", base_url(port));
    let mut last = String::from("no response");
    while std::time::Instant::now() < deadline {
        match http.get(&url).send().await {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => last = format!("HTTP {}", r.status()),
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Err(last)
}

/// What is available, without starting anything.
pub async fn availability(http: &reqwest::Client, port: u16) -> Availability {
    let url = format!("{}/health", base_url(port));
    if let Ok(r) = http.get(&url).timeout(Duration::from_millis(700)).send().await {
        if r.status().is_success() {
            return Availability::Running(port);
        }
    }
    match find_binary() {
        Some(p) => Availability::Installed(p),
        None => Availability::NotInstalled,
    }
}

/// The official install command, to show to the user.
///
/// We never run it ourselves: it downloads gigabytes and installs into the
/// user's home, and that has to stay their explicit decision.
pub const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/TalkBank/talkbank-tools/main/installers/install-batchalign3.sh | sh";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_binary_on_the_path_the_state_is_not_installed() {
        // An empty PATH simulates a machine without Batchalign, which is the
        // default situation and has to be handled as normal.
        let previous = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", "") };
        let found = find_binary();
        if let Some(v) = previous {
            unsafe { std::env::set_var("PATH", v) };
        }
        assert!(found.is_none());
    }

    #[test]
    fn the_binary_is_found_on_the_path() {
        let dir = tempdir::TempDir::new("talkbank-ba").unwrap();
        let fake = dir.path().join("batchalign3");
        std::fs::write(&fake, "#!/bin/sh\n").unwrap();

        let previous = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", dir.path()) };
        let found = find_binary();
        if let Some(v) = previous {
            unsafe { std::env::set_var("PATH", v) };
        }
        assert_eq!(found, Some(fake));
    }

    #[test]
    fn the_url_is_local_only() {
        // The control plane must never be reachable from the network.
        assert_eq!(base_url(18000), "http://127.0.0.1:18000");
        assert!(base_url(DEFAULT_PORT).starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn the_install_command_points_at_their_repo() {
        assert!(INSTALL_COMMAND.contains("TalkBank/talkbank-tools"));
        assert!(INSTALL_COMMAND.contains("install-batchalign3.sh"));
    }
}
