//! Probes the live service on a handful of paths.
//!
//!     cargo run -p talkbank-archive --example probe -- childes/Eng-NA/Brown ca/ATC
//!
//! Logs in if `.env` holds `USERNAME` and `PASSWORD`, because without a session
//! the access gate answers `200 text/html` for any path and the check can no
//! longer tell anything apart.

use talkbank_archive::api::{Client, LoginOutcome};

fn credentials() -> Option<(String, String)> {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../.env");
    let text = std::fs::read_to_string(path).ok()?;
    let (mut user, mut pass) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        let v = if v.len() >= 2 && v.starts_with(['"', '\'']) && v.ends_with(v.chars().next().unwrap())
        {
            &v[1..v.len() - 1]
        } else {
            v
        };
        match k.trim() {
            "USERNAME" => user = Some(v.to_string()),
            "PASSWORD" => pass = Some(v.to_string()),
            _ => {}
        }
    }
    Some((user?, pass?))
}

#[tokio::main]
async fn main() {
    let c = Client::new().expect("client");
    match credentials() {
        Some((u, p)) => match c.login(&u, &p).await {
            Ok(LoginOutcome::Success) => eprintln!("logged in"),
            Ok(other) => eprintln!("login failed: {other:?}"),
            Err(e) => eprintln!("login failed: {e}"),
        },
        None => eprintln!("no .env: the check will answer \"No\" to everything"),
    }

    for arg in std::env::args().skip(1) {
        let path: Vec<String> = arg.split('/').map(String::from).collect();
        let outcome = c.is_downloadable(&path).await;
        println!("{arg:40} {outcome:?}");
    }
}
