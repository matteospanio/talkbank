//! Checks our validation wrapper against `testchat`, the TalkBank conformance
//! corpus (339 "good" files, 81 "bad" ones).
//!
//! **`testchat` is not an exact oracle for chatter v0.12.0.** Measured: 38 of
//! the "good" files are rejected and 13 of the "bad" ones pass. That is not a
//! defect in our code: they are genuine chatter verdicts, with precise codes
//! (E316 "Unparsable content on dependent tier", E605 "Unsupported dependent
//! tier %trn"). The corpus is frozen at December 2025, chatter is from August
//! 2026. TalkBank treats it the same way: their own test over `testchat/bad`
//! (`apps/chatter-desktop/src-tauri/tests/validation_bridge.rs`) only checks
//! that the scan finishes and prints the counts, without demanding that every
//! file land on the side the corpus says it should.
//!
//! So this uses thresholds, not equalities: the test catches a *regression* (a
//! chatter upgrade that makes things worse, or a wrong call on our side)
//! without pretending to a contract that does not exist.
//!
//! If `testchat` is not cloned, the tests skip. Point `TESTCHAT` at it, or keep
//! it in `~/testchat`.

use talkbank_engine::validate;
use std::path::{Path, PathBuf};

fn testchat() -> Option<PathBuf> {
    for base in [
        std::env::var_os("TESTCHAT").map(PathBuf::from),
        dirs_home().map(|h| h.join("testchat")),
        dirs_home().map(|h| h.join("projects/testchat")),
    ]
    .into_iter()
    .flatten()
    {
        if base.join("good").is_dir() && base.join("bad").is_dir() {
            return Some(base);
        }
    }
    None
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn cha_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "cha"))
        .collect();
    v.sort();
    v
}

#[test]
fn the_good_testchat_files_pass() {
    let Some(root) = testchat() else {
        eprintln!("testchat not present: test skipped");
        return;
    };
    let files = cha_files(&root.join("good"));
    assert!(files.len() > 300, "expected >300 good files, found {}", files.len());

    let mut rejected = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        if !validate::is_valid_at(f, &src) {
            rejected.push(f.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    let accepted = files.len() - rejected.len();
    eprintln!("good files accepted: {accepted}/{}", files.len());
    // Baseline measured with chatter v0.12.0: 301/339. The threshold leaves room
    // for a corpus update, but not for a collapse.
    assert!(
        accepted >= 295,
        "only {accepted} of {} accepted: worse than the baseline of 301.\nRejected: {:?}",
        files.len(),
        &rejected[..rejected.len().min(15)]
    );
}

#[test]
fn the_bad_testchat_files_are_rejected() {
    let Some(root) = testchat() else { return };
    let files = cha_files(&root.join("bad"));
    assert!(files.len() > 50, "expected >50 bad files, found {}", files.len());

    let mut passed = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        if validate::is_valid_at(f, &src) {
            passed.push(f.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    let rejected = files.len() - passed.len();
    eprintln!("bad files rejected: {rejected}/{}", files.len());
    // Baseline measured with chatter v0.12.0: 68/81.
    assert!(
        rejected >= 64,
        "only {rejected} of {} rejected: worse than the baseline of 68.\nPassed: {:?}",
        files.len(),
        &passed[..passed.len().min(15)]
    );
}

/// The property that actually matters for the UI: no real-world file may blow
/// up the validator, and every rejection has to be explainable to the user.
#[test]
fn no_real_file_blows_up_the_validator() {
    let Some(root) = testchat() else { return };
    let mut examined = 0usize;
    for dir in ["good", "bad"] {
        for f in cha_files(&root.join(dir)) {
            let src = std::fs::read_to_string(&f).unwrap_or_default();
            let v = validate::validate_at(&f, &src);
            examined += 1;
            if !v.ok {
                assert!(
                    v.errors().next().is_some(),
                    "{:?} declared invalid without a single error",
                    f.file_name().unwrap()
                );
            }
        }
    }
    assert!(examined > 400, "only {examined} files examined");
    eprintln!("files examined without a panic: {examined}");
}

#[test]
fn every_rejected_file_says_why_and_where() {
    let Some(root) = testchat() else { return };
    let mut without_line = 0usize;
    let mut total = 0usize;
    for f in cha_files(&root.join("bad")).iter().take(30) {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        let v = validate::validate_at(f, &src);
        if v.ok {
            continue;
        }
        total += 1;
        assert!(
            v.errors().next().is_some_and(|d| !d.message.is_empty()),
            "{:?} rejected with no explanation",
            f.file_name().unwrap()
        );
        if v.errors().next().and_then(|d| d.line).is_none() {
            without_line += 1;
        }
    }
    // The line is not always recoverable from the message: record the number
    // instead of demanding 100%, so a regression shows without breaking the test.
    eprintln!("diagnostics with no line number: {without_line}/{total}");
}
