//! Tests against the live TalkBank service.
//!
//! Marked `#[ignore]`: a plain `cargo test` never touches the network. Run them
//! with
//!     cargo test -p talkbank-archive --test network -- --ignored --nocapture
//!
//! Most work **without credentials**, because the catalogue and the metadata are
//! public. The ones that need an account read `.env` and skip if it is missing.
//!
//! The assertions are thresholds: the archive grows, and one extra corpus must
//! not break the suite.

use talkbank_archive::api::{ApiError, Client, Downloadable, LoginOutcome, media_url};
use talkbank_archive::download;

fn client() -> Client {
    Client::new().expect("client created")
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("runtime")
}

fn p(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Reads `.env` by hand: `USERNAME` is also a system environment variable, so
/// reading it from the environment would give the Unix login name, not the file.
fn credentials() -> Option<(String, String)> {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../.env");
    let text = std::fs::read_to_string(path).ok()?;
    let mut user = None;
    let mut pass = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        // Strips quotes only when balanced: a password can contain an
        // apostrophe, which is exactly the case here.
        let v = if v.len() >= 2
            && v.starts_with(['"', '\''])
            && v.ends_with(v.chars().next().unwrap())
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

#[test]
#[ignore]
fn the_catalogue_is_public_and_covers_every_bank() {
    rt().block_on(async {
        let c = client();
        let v = c.tree().await.expect("tree downloaded");
        let a = talkbank_archive::catalog::parse(&v).expect("tree parsed");

        assert!(a.banks.len() >= 14, "banks: {}", a.banks.len());
        for b in &a.banks {
            eprintln!(
                "{:12} {:>6} folders with data, {:>6} transcripts",
                talkbank_archive::catalog::bank_title(&b.name),
                a.search(&b.name, "").len(),
                b.transcripts
            );
        }

        // CHILDES is the largest bank, but not the only one that has to work.
        let childes = a.bank("childes").expect("childes bank");
        assert!(childes.transcripts >= 50_000, "{}", childes.transcripts);
        let phon = a.bank("phon").expect("phon bank");
        assert!(phon.transcripts >= 10_000, "{}", phon.transcripts);

        // The shape without a collection level really exists: in CABank the
        // corpus sits directly under the bank.
        assert!(a.at(&p(&["ca", "ATC"])).is_some(), "ca/ATC should exist");
    });
}

#[test]
#[ignore]
fn transcript_metadata_are_public_in_every_bank() {
    rt().block_on(async {
        let c = client();
        // Three different path shapes: with a collection, without, and short.
        for path in [
            p(&["childes", "Eng-NA", "Brown"]),
            p(&["phon", "Eng-NA", "Davis"]),
            p(&["ca", "ATC"]),
        ] {
            let t = c
                .transcript_summary(&path)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", path.join("/")));

            for expected in ["path", "filename", "languages", "media", "designType"] {
                assert!(
                    t.column(expected).is_some(),
                    "column \"{expected}\" missing; present: {:?}",
                    t.headings
                );
            }
            assert!(!t.is_empty(), "{} cannot have zero transcripts", path.join("/"));
            eprintln!(
                "{}: {} rows, languages {:?}",
                path.join("/"),
                t.rows.len(),
                t.distinct("languages")
            );
        }
    });
}

#[test]
#[ignore]
fn participants_are_public_and_the_role_is_populated() {
    rt().block_on(async {
        let t = client()
            .participant_summary(&p(&["childes", "Eng-NA", "Haggerty"]))
            .await
            .expect("participants downloaded");
        assert!(t.column("role").is_some());
        eprintln!("roles: {:?}", t.distinct("role"));

        // Recording a measured limitation: the word counts are null on every row
        // sampled. If they ever arrive, this test will say so.
        let with_counts = t.rows.iter().filter(|r| t.get(r, "numwords").is_some()).count();
        eprintln!("rows with numwords populated: {with_counts}/{}", t.rows.len());
    });
}

/// Without a session the probe cannot tell anything apart: the access gate
/// answers `200 text/html` for any path, corpus or not.
#[test]
#[ignore]
fn without_sign_in_the_probe_says_so_instead_of_guessing() {
    rt().block_on(async {
        let c = client();
        for path in [
            p(&["childes", "Eng-NA", "Brown"]),
            p(&["childes", "Eng-NA"]),
        ] {
            assert_eq!(
                c.is_downloadable(&path).await,
                Downloadable::SignInRequired,
                "{}",
                path.join("/")
            );
        }
    });
}

/// The downloadability probe is the only thing that separates a corpus from a
/// collection: neither depth nor the presence of direct files is enough.
#[test]
#[ignore]
fn the_downloadability_probe_separates_corpora_from_collections() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        for (path, expected) in [
            (p(&["childes", "Eng-NA", "Brown"]), Downloadable::Yes),
            (p(&["phon", "Eng-NA", "Davis"]), Downloadable::Yes),
            (p(&["ca", "ATC"]), Downloadable::Yes),
            (p(&["class", "APT"]), Downloadable::Yes),
            // a collection is not downloadable, even though it holds only
            // subfolders exactly like Brown
            (p(&["childes", "Eng-NA"]), Downloadable::No),
            // nor is a subfolder inside a corpus
            (p(&["childes", "Eng-NA", "Brown", "Adam"]), Downloadable::No),
            // nor the bank itself
            (p(&["childes"]), Downloadable::No),
        ] {
            let outcome = c.is_downloadable(&path).await;
            eprintln!("{:40} {outcome:?}", path.join("/"));
            assert_eq!(outcome, expected, "{}", path.join("/"));
        }
    });
}

#[test]
#[ignore]
fn a_wrong_password_gives_wrong_credentials() {
    rt().block_on(async {
        // Exercises the whole error path without needing an account.
        let outcome = client()
            .login("nobody@example.invalid", "definitely-the-wrong-password")
            .await
            .expect("call succeeded");
        assert_eq!(outcome, LoginOutcome::WrongCredentials);
    });
}

#[test]
#[ignore]
fn a_vanished_route_is_recognised_as_such() {
    rt().block_on(async {
        // getNgrams is documented by the official clients but answers 404 today.
        let err = client()
            .post("getNgrams", serde_json::json!({"queryVals": {}}))
            .await
            .expect_err("expected a missing route");
        assert!(
            matches!(err, ApiError::RouteGone(_)),
            "expected RouteGone, got {err:?}"
        );
        assert!(err.is_degradable());
    });
}

#[test]
#[ignore]
fn without_sign_in_the_download_returns_the_login_page() {
    rt().block_on(async {
        // The access gate answers 200 with text/html: the HTTP status cannot
        // decide, and this test pins that down.
        let url = talkbank_archive::api::corpus_zip_url(&p(&["childes", "Eng-NA", "Haggerty"]));
        let resp = client().http().get(&url).send().await.expect("request");
        assert_eq!(resp.status(), 200, "the gate answers 200, not 401");
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.bytes().await.expect("body");
        assert!(
            ct.starts_with("text/html"),
            "signed out we expect HTML, not {ct}"
        );
        assert!(
            !body.starts_with(b"PK\x03\x04"),
            "signed out no zip should arrive"
        );
        eprintln!("access gate: {ct}, {} bytes", body.len());
    });
}

#[test]
#[ignore]
fn with_credentials_it_signs_in_and_downloads() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(
            c.login(&email, &password).await.expect("call succeeded"),
            LoginOutcome::Success,
            "credentials not accepted"
        );
        assert!(c.is_logged_in().await.expect("session state"));
        assert!(
            c.has_access("childes/Eng-NA/Haggerty").await.expect("access"),
            "the session should be authorised on an open corpus"
        );

        // Haggerty has a single transcript: the cheapest real download.
        let url = talkbank_archive::api::corpus_zip_url(&p(&["childes", "Eng-NA", "Haggerty"]));
        let resp = c.http().get(&url).send().await.expect("request");
        assert_eq!(resp.status(), 200);
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // Verified: the server sends no Content-Length on this route, so
        // progress cannot be a percentage.
        let len = resp.headers().get("content-length").is_some();
        let ranges = resp.headers().get("accept-ranges").is_some();
        let body = resp.bytes().await.expect("body");

        assert_eq!(ct, "application/zip");
        assert!(body.starts_with(b"PK\x03\x04"), "not a zip");
        eprintln!(
            "Haggerty: {} bytes, content-length={len}, accept-ranges={ranges}",
            body.len()
        );

        // And the zip holds the corpus directly, with no duplicated top-level
        // folder: that is what decides where to extract.
        let reader = std::io::Cursor::new(body.to_vec());
        let mut zip = zip::ZipArchive::new(reader).expect("zip opens");
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        eprintln!("contents: {names:?}");
        assert!(
            names.iter().any(|n| n.ends_with(".cha")),
            "expected at least one .cha"
        );
        assert!(
            !names.iter().all(|n| n.starts_with("Haggerty/")),
            "there must be no duplicated top-level folder"
        );
    });
}

/// The same URL works in banks other than CHILDES: the whole download module
/// rests on that.
#[test]
#[ignore]
fn downloads_work_outside_childes_too() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);

        for path in [
            p(&["phon", "Eng-NA", "Davis"]),
            p(&["slabank", "Classroom", "VanComp"]),
            p(&["ca", "ATC"]),
        ] {
            let url = talkbank_archive::api::corpus_zip_url(&path);
            let resp = c.http().head(&url).send().await.expect("request");
            let ct = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            eprintln!("{:40} {} {ct}", path.join("/"), resp.status());
            assert!(resp.status().is_success(), "{} not downloadable", path.join("/"));
        }
    });
}

/// Some banks (aphasia, samtale, psychosis) answer 401 even to a valid account:
/// they need separate permission, and that has to be said differently from
/// "you need to sign in".
#[test]
#[ignore]
fn restricted_banks_are_told_apart_from_the_access_gate() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);

        let v = c.tree().await.expect("tree");
        let a = talkbank_archive::catalog::parse(&v).expect("tree parsed");
        // The first aphasia corpus, whichever it is: names change, the 401 does not.
        let Some((path, _)) = a.search("aphasia", "").into_iter().next() else {
            eprintln!("aphasia missing from the tree: test skipped");
            return;
        };
        let outcome = c.is_downloadable(&path).await;
        eprintln!("{}: {outcome:?}", path.join("/"));
        assert!(
            matches!(outcome, Downloadable::NeedsPermission | Downloadable::No),
            "a restricted bank must not come back freely downloadable"
        );
    });
}

#[test]
#[ignore]
fn a_full_download_extracts_the_corpus() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);

        let dir = tempdir::TempDir::new("talkbank-dl").expect("temporary directory");
        let mut last_bytes = 0u64;
        let mut extracted = 0usize;
        let path = p(&["childes", "Eng-NA", "Haggerty"]);

        let dest = talkbank_archive::download::corpus(&c, &path, dir.path(), |p| {
            match p {
                talkbank_archive::download::Progress::Downloading(n) => last_bytes = n,
                talkbank_archive::download::Progress::Extracting { done, .. } => extracted = done,
                talkbank_archive::download::Progress::Done => {}
            }
            true
        })
        .await
        .expect("download succeeded");

        eprintln!("downloaded {last_bytes} bytes, extracted {extracted} entries into {}", dest.display());
        assert!(last_bytes > 1000, "progress was not reported");
        // The bank is part of the path: same-named corpora from different banks
        // must not land in the same folder.
        assert_eq!(dest, dir.path().join("childes/Eng-NA/Haggerty"));

        let cha: Vec<_> = std::fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".cha"))
            .collect();
        assert!(!cha.is_empty(), "no .cha extracted");
        eprintln!("transcripts extracted: {cha:?}");

        // The temporary folder must not survive: the existence of the
        // destination means "complete corpus".
        assert!(!dir.path().join("childes/Eng-NA/Haggerty.incoming").exists());
        assert!(talkbank_archive::download::already_there(
            dir.path(),
            &p(&["childes", "Eng-NA", "Haggerty"])
        ));
    });
}

#[test]
#[ignore]
fn without_sign_in_the_download_says_so_plainly() {
    rt().block_on(async {
        let dir = tempdir::TempDir::new("talkbank-dl").unwrap();
        // Fresh client, no session: it has to recognise the access gate instead
        // of saving 319 bytes of HTML and calling it a zip.
        let err = talkbank_archive::download::corpus(
            &client(),
            &p(&["childes", "Eng-NA", "Haggerty"]),
            dir.path(),
            |_| true,
        )
        .await
        .expect_err("expected an auth error");
        assert!(
            matches!(err, talkbank_archive::download::DownloadError::AuthRequired),
            "expected AuthRequired, got {err}"
        );
        assert!(
            !dir.path().join("childes/Eng-NA/Haggerty.incoming").exists(),
            "leftover temporary folder"
        );
        assert!(
            !dir.path().join("childes/Eng-NA/Haggerty").exists(),
            "a failed attempt must not leave a destination that looks good"
        );
    });
}

#[test]
#[ignore]
fn the_index_is_built_from_the_real_metadata() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);

        // An archive cut down to a few nodes: exercises the machinery without
        // firing 1,897 requests at the TalkBank server. It deliberately contains
        // a non-downloadable collection, which the index has to discard itself.
        let v = c.tree().await.expect("tree");
        let whole = talkbank_archive::catalog::parse(&v).expect("tree parsed");
        let eng_na = whole
            .at(&p(&["childes", "Eng-NA"]))
            .expect("Eng-NA")
            .clone();
        let mut trimmed = eng_na.clone();
        trimmed.children.retain(|c| c.name == "Haggerty" || c.name == "Bliss");
        assert_eq!(trimmed.children.len(), 2, "test corpora not found");

        let archive = talkbank_archive::catalog::Archive {
            banks: vec![talkbank_archive::catalog::Folder {
                name: "childes".into(),
                children: vec![trimmed],
                transcripts: 8,
                direct_files: 0,
                media: Default::default(),
            }],
        };

        let mut progress = Vec::new();
        let index = talkbank_archive::index::build(&c, &archive, "childes", |done, total| {
            progress.push((done, total));
        })
        .await;

        // Eng-NA is in the tree but is not a corpus: the HEAD probe discards it.
        assert_eq!(index.corpora.len(), 2, "indexed: {:?}", index.corpora.iter().map(|c| c.label()).collect::<Vec<_>>());
        assert!(index.corpora.iter().all(|c| c.path.len() == 3));
        assert_eq!(progress.last(), Some(&(3, 3)), "three folders examined");

        let languages = index.languages();
        assert!(languages.contains(&"eng".to_string()), "languages found: {languages:?}");
        eprintln!("languages: {languages:?}  study types: {:?}", index.designs());

        // And the filter then has to select for real
        let f = talkbank_archive::index::Filter {
            language: Some("eng".into()),
            ..Default::default()
        };
        assert_eq!(index.matching(&f).len(), 2);

        let f = talkbank_archive::index::Filter {
            language: Some("jpn".into()),
            ..Default::default()
        };
        assert!(index.matching(&f).is_empty());
    });
}

/// Planning a branch: descend where there is no corpus, stop where there is one.
#[test]
#[ignore]
fn a_branch_plan_finds_the_corpora_and_stops_there() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        let a = talkbank_archive::catalog::parse(&c.tree().await.expect("tree")).expect("tree");

        // phon/Eng-UK is small: two corpora, 204 transcripts. The plan should
        // cost three requests — the root and the two children — and never
        // descend inside the corpora.
        let mut progress = Vec::new();
        let plan = talkbank_archive::batch::plan(
            &c,
            &a,
            &p(&["phon", "Eng-UK"]),
            |done, left| progress.push((done, left)),
            || true,
        )
        .await;

        eprintln!(
            "plan: {} corpora, {} transcripts, {} probes, {} skipped",
            plan.corpora.len(),
            plan.transcripts,
            plan.probed,
            plan.skipped.len()
        );
        for c in &plan.corpora {
            eprintln!("  {}", c.join("/"));
        }

        assert!(!plan.needs_sign_in, "the session should be open");
        assert!(!plan.truncated, "a branch this small never hits the ceiling");
        assert!(plan.corpora.len() >= 2, "expected at least Cruttenden and Smith");
        assert!(
            plan.corpora.iter().all(|c| c.len() == 3),
            "PhonBank corpora sit at the third level: {:?}",
            plan.corpora
        );
        assert!(plan.transcripts >= 200, "transcripts: {}", plan.transcripts);
        // The probe count is the measure of the pruning: without it, we would
        // descend into every corpus.
        assert!(
            plan.probed <= plan.corpora.len() + 2,
            "too many probes ({}) for {} corpora",
            plan.probed,
            plan.corpora.len()
        );
        assert!(!progress.is_empty(), "progress was not reported");
    });
}

/// On a folder that already is a corpus, the plan holds only that folder.
#[test]
#[ignore]
fn the_plan_for_a_corpus_is_the_corpus_itself() {
    let Some((email, password)) = credentials() else { return };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        let a = talkbank_archive::catalog::parse(&c.tree().await.unwrap()).unwrap();

        let plan = talkbank_archive::batch::plan(
            &c,
            &a,
            &p(&["childes", "Eng-NA", "Brown"]),
            |_, _| {},
            || true,
        )
        .await;
        assert_eq!(plan.corpora, vec![p(&["childes", "Eng-NA", "Brown"])]);
        assert_eq!(plan.probed, 1, "one request and no more");
        assert_eq!(plan.transcripts, 214);
    });
}

/// Cancelling stops the planning run and says so.
#[test]
#[ignore]
fn planning_can_be_cancelled() {
    let Some((email, password)) = credentials() else { return };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        let a = talkbank_archive::catalog::parse(&c.tree().await.unwrap()).unwrap();

        let plan = talkbank_archive::batch::plan(
            &c,
            &a,
            &p(&["childes"]),
            |_, _| {},
            || false, // cancelled immediately
        )
        .await;
        // Cancelling is not the same as hitting the ceiling: the confirmation has
        // to tell "you pressed Cancel" from "the archive is deeper than this".
        assert!(plan.cancelled);
        assert!(!plan.truncated);
        assert_eq!(plan.probed, 0);
        assert!(!plan.resume.is_empty(), "it must be possible to resume");
    });
}

/// The full round trip: plan a branch, download all its corpora, and recognise
/// that the second time round there is nothing left to do.
#[test]
#[ignore]
fn a_branch_is_planned_downloaded_and_not_downloaded_again() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        let a = talkbank_archive::catalog::parse(&c.tree().await.expect("tree")).expect("tree");

        // slabank/Classroom is the smallest branch we know: a collection with one
        // corpus inside, two transcripts in total.
        let root = p(&["slabank", "Classroom"]);
        let plan = talkbank_archive::batch::plan(&c, &a, &root, |_, _| {}, || true).await;
        eprintln!(
            "plan: {:?}, {} transcripts, {} probes",
            plan.corpora.iter().map(|c| c.join("/")).collect::<Vec<_>>(),
            plan.transcripts,
            plan.probed
        );
        assert!(!plan.is_empty(), "the branch must hold at least one corpus");
        assert!(!plan.truncated && !plan.unreliable && !plan.needs_sign_in);

        let dir = tempdir::TempDir::new("talkbank-branch").expect("temporary directory");

        // None of the corpora is there yet.
        for corpus in &plan.corpora {
            assert!(!talkbank_archive::download::already_there(dir.path(), corpus));
        }

        for corpus in &plan.corpora {
            let dest = talkbank_archive::download::corpus(&c, corpus, dir.path(), |_| true)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", corpus.join("/")));
            eprintln!("downloaded into {}", dest.display());

            // Extraction is atomic: no leftovers, and the destination exists only
            // because it is complete.
            let inc = dest.with_file_name(format!(
                "{}.incoming",
                dest.file_name().unwrap().to_string_lossy()
            ));
            assert!(!inc.exists(), "temporary leftover: {}", inc.display());
            assert!(talkbank_archive::download::already_there(dir.path(), corpus));
        }

        // The transcripts on disk must be the ones the plan promised.
        fn count_cha(d: &std::path::Path) -> usize {
            std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let p = e.path();
                    if p.is_dir() {
                        count_cha(&p)
                    } else {
                        usize::from(p.extension().is_some_and(|x| x == "cha"))
                    }
                })
                .sum()
        }
        let on_disk = count_cha(&talkbank_archive::download::destination(dir.path(), &root));
        eprintln!("transcripts on disk: {on_disk}, promised: {}", plan.transcripts);
        assert_eq!(
            on_disk, plan.transcripts,
            "a corpus zip holds its whole subtree: the numbers have to add up"
        );

        // Second time round: everything is there, nothing to download again.
        let left = plan
            .corpora
            .iter()
            .filter(|c| !talkbank_archive::download::already_there(dir.path(), c))
            .count();
        assert_eq!(left, 0, "an already downloaded branch is not downloaded again");
    });
}

/// The media live on their own host and are not in the corpus zip.
///
/// Sizes measured on 2026-08-23. Thresholds, not equalities: the archive is
/// re-encoded from time to time.
#[test]
#[ignore]
fn media_are_served_from_their_own_host() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        use talkbank_archive::api::media_url;
        use talkbank_archive::download::media_size;

        // audio, two corpora an order of magnitude apart
        for (dir, name, floor_mb) in [
            (p(&["ca", "ATC", "disasters"]), "alaska261_2000", 1u64),
            (p(&["class", "Bradford"]), "14", 20),
        ] {
            let url = media_url(&dir, name, "mp3");
            let n = media_size(&c, &url).await.unwrap_or_else(|| panic!("{url}"));
            eprintln!("{url} -> {:.1} MB", n as f64 / 1_048_576.0);
            assert!(n / 1_048_576 >= floor_mb, "{url}: {n} bytes");
        }

        // video is .mp4 and is very much larger — this is what the size
        // estimate has to warn about before a branch download starts.
        let url = media_url(&p(&["childes", "Biling", "Bailleul"]), "020408", "mp4");
        let n = media_size(&c, &url).await.expect("Bailleul video");
        eprintln!("{url} -> {:.1} MB", n as f64 / 1_048_576.0);
        assert!(n / 1_048_576 >= 100, "video should be hundreds of MB: {n}");

        // and the transcripts route does not serve them
        let wrong = format!(
            "{}/ca/ATC/disasters/alaska261_2000.mp3",
            talkbank_archive::api::DATA_BASE
        );
        assert!(media_size(&c, &wrong).await.is_none(), "{wrong} should 404");
    });
}

/// A name that does not exist must come back as absent, not as a login page.
#[test]
#[ignore]
fn a_missing_media_name_is_not_mistaken_for_the_access_gate() {
    let Some((email, password)) = credentials() else { return };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        let url = talkbank_archive::api::media_url(&p(&["ca", "ATC"]), "no-such-recording", "mp3");
        assert!(talkbank_archive::download::media_size(&c, &url).await.is_none());

        let dir = tempdir::TempDir::new("talkbank-media").unwrap();
        let err = talkbank_archive::download::media(&c, &url, &dir.path().join("x.mp3"), |_| true)
            .await
            .expect_err("expected a missing file");
        assert!(
            matches!(err, talkbank_archive::download::DownloadError::NotAvailable),
            "expected NotAvailable, got {err}"
        );
    });
}

/// Signed out, the media host answers with the gate, exactly like the zip
/// route. This is the test that protects the content-type check.
#[test]
#[ignore]
fn without_sign_in_the_media_host_answers_the_gate() {
    rt().block_on(async {
        let c = client();
        let url = talkbank_archive::api::media_url(
            &p(&["ca", "ATC", "disasters"]),
            "alaska261_2000",
            "mp3",
        );
        assert!(
            talkbank_archive::download::media_size(&c, &url).await.is_none(),
            "signed out there is nothing to measure"
        );
        let dir = tempdir::TempDir::new("talkbank-media").unwrap();
        let err = talkbank_archive::download::media(&c, &url, &dir.path().join("x.mp3"), |_| true)
            .await
            .expect_err("expected the access gate");
        assert!(
            matches!(err, talkbank_archive::download::DownloadError::AuthRequired),
            "expected AuthRequired, got {err}"
        );
        assert!(!dir.path().join("x.mp3").exists());
        assert!(!dir.path().join("x.mp3.part").exists(), "no partial left behind");
    });
}

/// The whole media path, end to end: a corpus, then the recordings its
/// transcripts name, landing next to them.
///
/// This is the test that caught the media host answering a plain GET with
/// eleven bytes — a unit test could not have seen it.
#[test]
#[ignore]
fn a_corpus_and_its_media_land_side_by_side() {
    let Some((email, password)) = credentials() else {
        eprintln!("no .env: test skipped");
        return;
    };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        let dir = tempdir::TempDir::new("e2e").unwrap();
        let path: Vec<String> = ["ca", "ATC"].iter().map(|s| s.to_string()).collect();

        let dest = download::corpus(&c, &path, dir.path(), |_| true).await.expect("corpus");
        // Recursive: ATC keeps a `disasters` subfolder, and a recording sits
        // next to its transcript rather than at the corpus root.
        fn chas_in(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() { chas_in(&p, out); }
                else if p.extension().is_some_and(|x| x == "cha") { out.push(p); }
            }
        }
        let mut chas = Vec::new();
        chas_in(&dest, &mut chas);
        chas.sort();
        eprintln!("transcripts: {}", chas.len());
        assert!(!chas.is_empty());

        let mut got = 0;
        for cha in chas.iter().take(3) {
            let info = talkbank_engine::chat::inspect(cha);
            let m = info.media.expect("every ATC transcript declares a recording");
            assert!(m.is_fetchable());
            let ext = download::extensions(m.video)[0];
            let parent = cha.parent().unwrap();
            let mut dir = path.clone();
            if let Ok(rel) = parent.strip_prefix(&dest) {
                dir.extend(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()));
            }
            let url = media_url(&dir, &m.basename, ext);
            let target = parent.join(format!("{}.{ext}", m.basename));
            download::media(&c, &url, &target, |_| true).await.expect(&url);
            let n = std::fs::metadata(&target).unwrap().len();
            eprintln!("  {} -> {:.1} MB beside {}", target.file_name().unwrap().to_string_lossy(),
                      n as f64 / 1048576.0, cha.file_name().unwrap().to_string_lossy());
            assert!(n > 100_000, "media too small, the preview bug is back: {n}");
            assert!(!parent.join(format!("{}.{ext}.part", m.basename)).exists(), "a .part file was left behind");
            got += 1;
        }
        assert_eq!(got, 3);

        // Idempotence: this is what makes a repeat download a repair.
        let cha = &chas[0];
        let m = talkbank_engine::chat::inspect(cha).media.unwrap();
        let ext = download::extensions(m.video)[0];
        let parent = cha.parent().unwrap();
        let mut dir = path.clone();
        if let Ok(rel) = parent.strip_prefix(&dest) {
            dir.extend(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()));
        }
        let target = parent.join(format!("{}.{ext}", m.basename));
        let before = std::fs::metadata(&target).unwrap().modified().unwrap();
        download::media(&c, &media_url(&dir, &m.basename, ext), &target, |_| true).await.unwrap();
        assert_eq!(std::fs::metadata(&target).unwrap().modified().unwrap(), before,
                   "a recording already on disk must not be fetched again");
        eprintln!("idempotence: ok");
    });
}

/// Remembering the answers avoids asking the same question twice.
#[test]
#[ignore]
fn the_probe_is_not_repeated_within_one_session() {
    let Some((email, password)) = credentials() else { return };
    rt().block_on(async {
        let c = client();
        assert_eq!(c.login(&email, &password).await.unwrap(), LoginOutcome::Success);
        let path = p(&["childes", "Eng-NA", "Brown"]);

        assert!(c.cached_downloadable(&path).is_none(), "nothing remembered yet");
        let first = c.is_downloadable(&path).await;
        assert_eq!(first, Downloadable::Yes);
        assert_eq!(c.cached_downloadable(&path), Some(Downloadable::Yes));

        // The second answer comes from memory: measurable by the elapsed time.
        let started = std::time::Instant::now();
        assert_eq!(c.is_downloadable(&path).await, Downloadable::Yes);
        let elapsed = started.elapsed();
        eprintln!("second probe in {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_millis(20),
            "the second probe repeated the request: {elapsed:?}"
        );
    });
}
