//! Metadata index of the archive.
//!
//! Real filters have a problem: `getTranscriptSummary` answers **per corpus**,
//! and CHILDES has 291 corpora. Filtering the whole archive by language or by
//! study type therefore takes 291 requests. Doing that on every search would be
//! absurd; doing it once and keeping the result is reasonable: measured at about
//! 1.5 seconds per corpus, four at a time, it is a couple of one-off minutes.
//!
//! So the index is **optional and on demand**: without it you still have the
//! instant filters the tree already allows (name, collection, media presence).

use std::collections::BTreeSet;
use std::path::PathBuf;

use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, Client};
use crate::catalog::Archive;

/// How many requests run in parallel. The server is small and academic: four is
/// a compromise between not waiting half an hour and not being rude.
const CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusFacets {
    /// Full path in the archive, bank included. Depth differs from bank to bank,
    /// so a path is the only reliable identity.
    pub path: Vec<String>,
    pub languages: Vec<String>,
    pub design: Vec<String>,
    pub activity: Vec<String>,
    pub group: Vec<String>,
    /// Transcripts that declare a media file.
    pub with_media: usize,
    pub transcripts: usize,
    /// True when the server agrees to produce a zip for it. Verified with a HEAD
    /// request while building: that is the only way to know.
    pub downloadable: bool,
}

impl CorpusFacets {
    pub fn name(&self) -> &str {
        self.path.last().map(String::as_str).unwrap_or("")
    }
    /// The path without the bank, as it reads in the interface.
    pub fn label(&self) -> String {
        self.path.iter().skip(1).cloned().collect::<Vec<_>>().join(" / ")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    pub bank: String,
    pub corpora: Vec<CorpusFacets>,
}

/// Filter criteria. An empty field does not filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub language: Option<String>,
    pub design: Option<String>,
    pub activity: Option<String>,
    pub group: Option<String>,
    pub only_with_media: bool,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        *self == Filter::default()
    }
}

impl Index {
    /// The corpora that satisfy the criteria.
    pub fn matching(&self, f: &Filter) -> Vec<&CorpusFacets> {
        self.corpora
            .iter()
            .filter(|c| {
                // Languages also arrive in compound form ("eng,fra"): the
                // comparison is by content, not by string equality.
                let lang_ok = f.language.as_ref().is_none_or(|want| {
                    c.languages
                        .iter()
                        .any(|l| l.split(',').any(|part| part.trim() == want))
                });
                let design_ok = f.design.as_ref().is_none_or(|w| c.design.contains(w));
                let activity_ok = f.activity.as_ref().is_none_or(|w| c.activity.contains(w));
                let group_ok = f.group.as_ref().is_none_or(|w| c.group.contains(w));
                let media_ok = !f.only_with_media || c.with_media > 0;
                lang_ok && design_ok && activity_ok && group_ok && media_ok
            })
            .collect()
    }

    /// The available values for one facet, to populate the menus.
    pub fn values(&self, pick: impl Fn(&CorpusFacets) -> &Vec<String>) -> Vec<String> {
        let mut set = BTreeSet::new();
        for c in &self.corpora {
            for v in pick(c) {
                // "eng,fra" counts as two distinct languages
                for part in v.split(',') {
                    let part = part.trim();
                    if !part.is_empty() {
                        set.insert(part.to_string());
                    }
                }
            }
        }
        set.into_iter().collect()
    }

    pub fn languages(&self) -> Vec<String> {
        self.values(|c| &c.languages)
    }
    pub fn designs(&self) -> Vec<String> {
        self.values(|c| &c.design)
    }
    pub fn groups(&self) -> Vec<String> {
        self.values(|c| &c.group)
    }
}

/// One file per bank: indexing CHILDES says nothing about PhonBank, and keeping
/// them apart means never rebuilding one to get the other.
pub fn path(bank: &str) -> PathBuf {
    // The bank name ends up in a path: sanitising it keeps a new bank with an
    // odd name from writing outside the cache directory.
    let safe: String = bank
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    crate::cache::dir().join(format!("talkbank-index-{safe}.json"))
}

pub fn load(bank: &str) -> Option<Index> {
    let text = std::fs::read_to_string(path(bank)).ok()?;
    let index: Index = serde_json::from_str(&text).ok()?;
    (index.bank == bank).then_some(index)
}

pub fn store(index: &Index) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(index)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    crate::cache::store(&path(&index.bank), &bytes)
}

/// Builds the index by querying every corpus.
///
/// `on_progress(done, total)` is called as each corpus finishes. Corpora that
/// fail are skipped: a partial index beats no index, and the route may vanish
/// the way others already have.
pub async fn build(
    client: &Client,
    archive: &Archive,
    bank: &str,
    mut on_progress: impl FnMut(usize, usize),
) -> Index {
    // Every folder that holds transcripts, at any depth: in CABank the corpus is
    // at the first level, in CHILDES at the second, and some banks use both
    // shapes.
    let targets: Vec<(Vec<String>, usize)> = archive
        .search(bank, "")
        .into_iter()
        .map(|(path, node)| (path, node.transcripts))
        .collect();
    let total = targets.len();

    // Without a session the probe tells nothing apart: firing 1,900 of them only
    // to discard them all would just be rude to their server.
    if let Some((first, _)) = targets.first() {
        if client.is_downloadable(first).await == crate::api::Downloadable::SignInRequired {
            tracing::info!("the index cannot be built without signing in");
            return Index {
                bank: bank.to_string(),
                corpora: Vec::new(),
            };
        }
    }

    let results = stream::iter(targets.into_iter().map(|(path, n)| {
        let client = client.clone();
        async move {
            // The downloadability probe first: it is cheap and separates a
            // corpus from a collection or from an inner subfolder.
            let can = client.is_downloadable(&path).await;
            let table = if can == crate::api::Downloadable::Yes {
                client.transcript_summary(&path).await.ok()
            } else {
                None
            };
            (path, n, can, table)
        }
    }))
    .buffer_unordered(CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let mut corpora = Vec::new();
    let mut done = 0;
    for (path, transcripts, can, table) in results {
        done += 1;
        on_progress(done, total);
        if can != crate::api::Downloadable::Yes {
            continue;
        }
        let Some(t) = table else { continue };
        let with_media = t.rows.iter().filter(|r| t.get(r, "media").is_some()).count();
        corpora.push(CorpusFacets {
            path,
            languages: t.distinct("languages"),
            design: t.distinct("designType"),
            activity: t.distinct("activityType"),
            group: t.distinct("groupType"),
            with_media,
            transcripts,
            downloadable: true,
        });
    }
    corpora.sort_by(|a, b| a.path.cmp(&b.path));
    Index {
        bank: bank.to_string(),
        corpora,
    }
}

/// An error worth telling apart: if the metadata route disappears, the index
/// cannot be built and that should be said once.
pub fn is_unavailable(e: &ApiError) -> bool {
    e.is_degradable()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Index {
        Index {
            bank: "childes".into(),
            corpora: vec![
                CorpusFacets {
                    path: vec!["childes".into(), "Eng-NA".into(), "Brown".into()],
                    downloadable: true,
                    languages: vec!["eng".into(), "eng,fra".into()],
                    design: vec!["long".into()],
                    activity: vec!["toyplay".into()],
                    group: vec!["TD".into()],
                    with_media: 0,
                    transcripts: 214,
                },
                CorpusFacets {
                    path: vec!["childes".into(), "Spanish".into(), "Ornat".into()],
                    downloadable: true,
                    languages: vec!["spa".into()],
                    design: vec!["long".into()],
                    activity: vec![],
                    group: vec!["TD".into()],
                    with_media: 12,
                    transcripts: 30,
                },
                CorpusFacets {
                    path: vec!["ca".into(), "ATC".into()],
                    downloadable: true,
                    languages: vec!["eng".into()],
                    design: vec!["cross".into()],
                    activity: vec![],
                    group: vec!["HL".into()],
                    with_media: 7,
                    transcripts: 7,
                },
            ],
        }
    }

    #[test]
    fn with_no_criteria_everything_passes() {
        let i = index();
        assert_eq!(i.matching(&Filter::default()).len(), 3);
        assert!(Filter::default().is_empty());
    }

    #[test]
    fn language_is_matched_inside_compound_lists() {
        let i = index();
        let f = Filter {
            language: Some("fra".into()),
            ..Default::default()
        };
        // Brown declares "eng,fra" on some transcripts: it must show up
        let hits = i.matching(&f);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name(), "Brown");

        let f = Filter {
            language: Some("eng".into()),
            ..Default::default()
        };
        assert_eq!(i.matching(&f).len(), 2, "Brown and ATC");
    }

    #[test]
    fn criteria_add_up() {
        let i = index();
        let f = Filter {
            language: Some("eng".into()),
            design: Some("cross".into()),
            ..Default::default()
        };
        let hits = i.matching(&f);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name(), "ATC");
    }

    #[test]
    fn the_media_filter_excludes_those_without_any() {
        let i = index();
        let f = Filter {
            only_with_media: true,
            ..Default::default()
        };
        let hits = i.matching(&f);
        assert_eq!(hits.len(), 2, "Brown has no media");
        assert!(hits.iter().all(|c| c.with_media > 0));
    }

    #[test]
    fn a_criterion_with_no_matches_gives_zero_not_everything() {
        let i = index();
        let f = Filter {
            language: Some("jpn".into()),
            ..Default::default()
        };
        assert!(i.matching(&f).is_empty());
    }

    #[test]
    fn menu_values_are_distinct_and_split_apart() {
        let i = index();
        // "eng,fra" must give two separate entries, not a third label
        assert_eq!(i.languages(), ["eng", "fra", "spa"]);
        assert_eq!(i.designs(), ["cross", "long"]);
        assert_eq!(i.groups(), ["HL", "TD"]);
    }

    #[test]
    fn the_index_round_trips_through_disk() {
        let json = serde_json::to_string(&index()).unwrap();
        let back: Index = serde_json::from_str(&json).unwrap();
        assert_eq!(back.corpora.len(), 3);
        assert_eq!(back.corpora[0].name(), "Brown");
    }
}
