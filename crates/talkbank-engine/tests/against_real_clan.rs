//! Checks the runner against the real CLAN programs.
//!
//! This is the test that justifies the whole pseudo-terminal machinery: without
//! it CLAN ignores the filenames, reads standard input, and refuses `+f`.
//! If the binaries are missing (workspace not built) the tests skip instead of
//! failing, so `cargo test` stays green on a machine without CLAN.

use talkbank_engine::{find_bin_dir, runner};
use std::path::{Path, PathBuf};

fn setup() -> Option<(PathBuf, tempdir::TempDir)> {
    let bin = find_bin_dir()?;
    let dir = tempdir::TempDir::new("talkbank-test").ok()?;
    std::fs::write(
        dir.path().join("sample.cha"),
        "@UTF8\n@Begin\n@Languages:\teng\n\
         @Participants:\tCHI Nicky Target_Child, MOT Kelly Mother\n\
         @ID:\teng|x|CHI|2;00.00|female|||Target_Child|||\n\
         @ID:\teng|x|MOT|||female|||Mother|||\n\
         *CHI:\tsee the chalk .\n\
         *MOT:\tyes that is chalk .\n\
         *CHI:\twant more chalk .\n\
         @End\n",
    )
    .ok()?;
    Some((bin, dir))
}

#[test]
fn freq_counts_the_words_of_the_named_file() {
    let Some((bin, dir)) = setup() else {
        eprintln!("CLAN not built: test skipped");
        return;
    };
    let out = runner::run(&bin, "freq", &["sample.cha".into()], dir.path()).expect("freq ran");

    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    // If the pseudo-terminal did not work, CLAN would read from the empty stdin
    // and say "From pipe input" without counting anything.
    assert!(
        !out.stdout.contains("From pipe input"),
        "CLAN read from stdin instead of the file:\n{}",
        out.stdout
    );
    assert!(out.stdout.contains("chalk"), "unexpected output:\n{}", out.stdout);
    assert!(!out.stdout.contains('\r'), "the tty CRLF was not stripped");
}

#[test]
fn filtering_by_speaker_changes_the_result() {
    let Some((bin, dir)) = setup() else { return };
    let everyone = runner::run(&bin, "freq", &["sample.cha".into()], dir.path()).unwrap();
    let child_only = runner::run(
        &bin,
        "freq",
        &["+t*CHI".into(), "sample.cha".into()],
        dir.path(),
    )
    .unwrap();

    assert!(everyone.stdout.contains("yes"), "expected the mother's speech");
    assert!(
        !child_only.stdout.contains("yes"),
        "with +t*CHI the mother should not appear:\n{}",
        child_only.stdout
    );
}

#[test]
fn with_plus_f_the_result_goes_to_a_file() {
    let Some((bin, dir)) = setup() else { return };
    // "+f" is the option CLAN refuses when stdout is not a terminal: if this
    // test passes, the pseudo-terminal works on the way out too.
    let out = runner::run(
        &bin,
        "freq",
        &["+f".into(), "sample.cha".into()],
        dir.path(),
    )
    .expect("freq +f ran");

    assert!(
        !out.stderr.contains("file redirect"),
        "CLAN refused +f: stdout was not a terminal\n{}",
        out.stderr
    );
    assert!(
        out.created.iter().any(|f| f.ends_with(".cex")),
        "no .cex file created; created: {:?}\nstderr: {}",
        out.created,
        out.stderr
    );
}

#[test]
fn usage_returns_the_program_options() {
    let Some(bin) = find_bin_dir() else { return };
    let text = runner::usage(&bin, "freq").expect("usage of freq");
    assert!(text.contains("Usage:"), "expected the usage text:\n{text}");
    assert!(text.contains("+t"), "expected the +t option in the list");
}

#[test]
fn a_nonexistent_program_errors_instead_of_panicking() {
    let bin = find_bin_dir().unwrap_or_else(|| Path::new("/nonexistent").to_path_buf());
    assert!(runner::run(&bin, "doesnotexist", &[], Path::new("/tmp")).is_err());
}
