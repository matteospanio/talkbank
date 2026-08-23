//! Shows chatter's diagnostics for one or more CHAT files.
//!
//! A service tool, used to work out why a file gets rejected:
//!     cargo run -p talkbank-engine --example diag -- file.cha …

fn main() {
    let files: Vec<_> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: diag <file.cha> [...]");
        std::process::exit(2);
    }
    for f in files {
        let path = std::path::Path::new(&f);
        let src = std::fs::read_to_string(path).unwrap_or_default();
        let v = talkbank_engine::validate::validate_at(path, &src);
        println!(
            "\n=== {}  {}  ({} utterances, {} diagnostics)",
            path.display(),
            if v.ok { "valid" } else { "INVALID" },
            v.utterance_count,
            v.diagnostics.len()
        );
        for d in &v.diagnostics {
            println!(
                "   {} [{}] line {}: {}",
                if d.is_error { "error  " } else { "warning" },
                d.code,
                d.line.map_or("?".into(), |l| l.to_string()),
                d.message
            );
            if let Some(s) = &d.suggestion {
                println!("            hint: {s}");
            }
        }
    }
}
