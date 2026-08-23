//! Checks the parser against the real archive tree (4.3 MB).
//!
//! The file comes from `TALKBANK_TREE_JSON`, otherwise from the app's cache. If
//! it is missing the test skips: `cargo test` must not depend on the network.
//!
//! The assertions are **thresholds**, not equalities: the archive grows, and one
//! extra corpus must not fail the suite. The numbers measured on 2026-08-17 were
//! 15 banks, 1,897 corpus folders, 97,052 transcripts.

use std::path::PathBuf;

fn tree() -> Option<serde_json::Value> {
    let candidates = [
        std::env::var_os("TALKBANK_TREE_JSON").map(PathBuf::from),
        std::env::var_os("XDG_CACHE_HOME")
            .map(|c| PathBuf::from(c).join("talkbank/talkbank-tree.json")),
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".cache/talkbank/talkbank-tree.json")),
        Some(PathBuf::from("/tmp/trees.json")),
    ];
    for p in candidates.into_iter().flatten() {
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str(&text) {
                eprintln!("tree read from {}", p.display());
                return Some(v);
            }
        }
    }
    None
}

fn p(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_real_tree_parses_end_to_end() {
    let Some(json) = tree() else {
        eprintln!("tree not available: test skipped");
        return;
    };
    let a = talkbank_archive::catalog::parse(&json).expect("tree parsed");

    assert!(a.banks.len() >= 14, "expected >=14 banks, found {}", a.banks.len());

    let mut total = 0;
    for b in &a.banks {
        let folders = a.search(&b.name, "").len();
        eprintln!(
            "{:12} {:>5} folders with data, {:>6} transcripts",
            talkbank_archive::catalog::bank_title(&b.name),
            folders,
            b.transcripts
        );
        // A bank with no transcripts would be the symptom of a level the parser
        // skipped: the doubled level is not uniform across banks.
        assert!(b.transcripts > 0, "empty bank: {}", b.name);
        total += b.transcripts;
    }
    assert!(total >= 80_000, "total transcripts: {total}");

    let childes = a.bank("childes").expect("the childes bank must be there");
    assert!(
        childes.children.len() >= 20,
        "expected >=20 collections in CHILDES, found {}",
        childes.children.len()
    );
    assert!(childes.transcripts >= 45_000, "{}", childes.transcripts);

    // PhonBank matters as much as CHILDES: it has to browse the same way.
    let phon = a.bank("phon").expect("the phon bank must be there");
    assert!(phon.transcripts >= 10_000, "{}", phon.transcripts);
    assert!(!phon.children.is_empty());
}

#[test]
fn banks_with_no_collection_level_browse_the_same() {
    let Some(json) = tree() else { return };
    let a = talkbank_archive::catalog::parse(&json).unwrap();

    // In CABank the corpus sits directly under the bank, and loose transcripts
    // sit next to the corpora: a parser assuming three fixed levels would lose
    // everything here.
    let ca = a.bank("ca").expect("ca bank");
    assert!(ca.direct_files > 0 || !ca.children.is_empty());
    assert!(
        a.at(&p(&["ca", "ATC"])).is_some(),
        "ca/ATC should be reachable with a two-element path"
    );
    assert_eq!(a.at(&p(&["ca", "ATC"])).unwrap().name, "ATC");
}

#[test]
fn known_corpora_have_the_expected_counts() {
    let Some(json) = tree() else { return };
    let a = talkbank_archive::catalog::parse(&json).unwrap();

    let hag = a.at(&p(&["childes", "Eng-NA", "Haggerty"]));
    assert_eq!(
        hag.map(|c| c.transcripts),
        Some(1),
        "Haggerty is the smallest corpus: a single transcript"
    );
    let brown = a.at(&p(&["childes", "Eng-NA", "Brown"])).unwrap();
    assert_eq!(brown.transcripts, 214, "Brown: Adam 55 + Eve 20 + Sarah 139");
    assert_eq!(brown.direct_files, 0, "Brown holds only subfolders");
    assert_eq!(brown.children.len(), 3, "Adam, Eve, Sarah");
}

#[test]
fn search_finds_same_named_corpora_in_different_banks() {
    let Some(json) = tree() else { return };
    let a = talkbank_archive::catalog::parse(&json).unwrap();

    // "Eng-NA" exists in both CHILDES and PhonBank: the full path is the only
    // thing that tells them apart.
    let in_childes = a.search("childes", "Eng-NA");
    let in_phon = a.search("phon", "Eng-NA");
    assert!(!in_childes.is_empty() && !in_phon.is_empty());
    assert_ne!(in_childes[0].0, in_phon[0].0);
}
