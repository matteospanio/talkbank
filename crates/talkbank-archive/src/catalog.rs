//! The TalkBank archive tree.
//!
//! It comes from `getAnnoPathTrees`: 4.3 MB covering all 15 banks, 1,897 corpus
//! folders and 97,052 transcripts.
//!
//! ```text
//! respMsg.<bank>.<bank>.<folder>.<folder>…<file>
//! ```
//!
//! **The number of levels is not fixed.** CHILDES and PhonBank have a collection
//! above the corpus (`childes/Eng-NA/Brown`); in CABank, ClassBank, BilingBank,
//! FluencyBank and SamtaleBank the corpus sits directly under the bank
//! (`ca/ATC`), and in those banks most of what looks like a corpus at the second
//! level is really a transcript. Leaves sit between the second and the eighth
//! level.
//!
//! So there is no "collection/corpus" hierarchy here, just a tree of folders:
//! which folder is downloadable is the server's answer, not a rule that would
//! guess wrong in five banks out of fifteen.

use serde::{Deserialize, Serialize};

/// Media presence, as a set of comma-separated flags.
///
/// Values seen in the archive: `audio`, `audio,missing`, `audio,notrans`,
/// `audio,unlinked`, `video`, and the same combinations. An enum would be wrong.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    pub audio: bool,
    pub video: bool,
    pub missing: bool,
    pub notrans: bool,
    pub unlinked: bool,
}

impl Media {
    pub fn parse(s: &str) -> Media {
        let mut m = Media::default();
        for part in s.split(',') {
            match part.trim() {
                "audio" => m.audio = true,
                "video" => m.video = true,
                "missing" => m.missing = true,
                "notrans" => m.notrans = true,
                "unlinked" => m.unlinked = true,
                "" => {}
                other => tracing::debug!("unknown media flag: {other}"),
            }
        }
        m
    }
    pub fn any(&self) -> bool {
        self.audio || self.video
    }
    pub fn incomplete(&self) -> bool {
        self.missing || self.notrans || self.unlinked
    }
    fn merge(&mut self, o: Media) {
        self.audio |= o.audio;
        self.video |= o.video;
        self.missing |= o.missing;
        self.notrans |= o.notrans;
        self.unlinked |= o.unlinked;
    }
}

/// A folder in the archive. Files are not nodes: they are counted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    /// Subfolders, in alphabetical order.
    pub children: Vec<Folder>,
    /// Transcripts contained, counting those in subfolders too.
    pub transcripts: usize,
    /// Transcripts immediately inside this folder.
    pub direct_files: usize,
    pub media: Media,
}

impl Folder {
    pub fn child(&self, name: &str) -> Option<&Folder> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Archive {
    pub banks: Vec<Folder>,
}

impl Archive {
    pub fn bank(&self, name: &str) -> Option<&Folder> {
        self.banks.iter().find(|b| b.name == name)
    }

    /// Walks a full path, bank included.
    pub fn at(&self, path: &[String]) -> Option<&Folder> {
        let (first, rest) = path.split_first()?;
        let mut node = self.bank(first)?;
        for step in rest {
            node = node.child(step)?;
        }
        Some(node)
    }

    /// Searches every folder of a bank, at any depth.
    ///
    /// Returns the full path of each hit, because without it you would know
    /// neither where it sits nor what to download.
    pub fn search(&self, bank: &str, query: &str) -> Vec<(Vec<String>, &Folder)> {
        let q = query.trim().to_lowercase();
        let Some(root) = self.bank(bank) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        walk_search(root, &mut vec![bank.to_string()], &q, &mut out);
        out
    }
}

fn walk_search<'a>(
    node: &'a Folder,
    path: &mut Vec<String>,
    q: &str,
    out: &mut Vec<(Vec<String>, &'a Folder)>,
) {
    for child in &node.children {
        path.push(child.name.clone());
        // Folders with no transcripts are of no interest: there is nothing to
        // download and nothing to analyse.
        if child.transcripts > 0 && (q.is_empty() || child.name.to_lowercase().contains(q)) {
            out.push((path.clone(), child));
        }
        walk_search(child, path, q, out);
        path.pop();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("the response has no respMsg")]
    NoRespMsg,
    #[error("respMsg is not an object")]
    NotAnObject,
}

/// Builds a folder from the JSON node, counting files and merging media flags.
fn build(name: &str, value: &serde_json::Value) -> Folder {
    let mut folder = Folder {
        name: name.to_string(),
        children: Vec::new(),
        transcripts: 0,
        direct_files: 0,
        media: Media::default(),
    };
    let Some(map) = value.as_object() else {
        return folder;
    };
    for (key, child) in map {
        // `file` and `media` describe the node, they are not children. And a
        // value that is not an object is not a folder: dropping it avoids
        // conjuring up empty ghost folders.
        if key == "file" || key == "media" || !child.is_object() {
            continue;
        }
        let is_file = child
            .get("file")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if is_file {
            folder.direct_files += 1;
            folder.transcripts += 1;
            if let Some(m) = child.get("media").and_then(serde_json::Value::as_str) {
                folder.media.merge(Media::parse(m));
            }
        } else {
            let sub = build(key, child);
            folder.transcripts += sub.transcripts;
            folder.media.merge(sub.media);
            folder.children.push(sub);
        }
    }
    folder.children.sort_by(|a, b| a.name.cmp(&b.name));
    folder
}

/// Parses the `getAnnoPathTrees` response.
pub fn parse(response: &serde_json::Value) -> Result<Archive, ParseError> {
    let root = response.get("respMsg").ok_or(ParseError::NoRespMsg)?;
    let banks_obj = root.as_object().ok_or(ParseError::NotAnObject)?;

    let mut banks: Vec<Folder> = banks_obj
        .iter()
        .map(|(name, doubled)| {
            // The doubled level (`childes.childes`). If it were missing we carry
            // on from the node we have: a partial tree beats no tree.
            let inner = doubled.get(name).unwrap_or(doubled);
            build(name, inner)
        })
        .collect();
    banks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Archive { banks })
}

/// Human-readable name of a bank. The tree keys are abbreviations (`phon`, `ca`,
/// `tbi`) that mean nothing on their own to someone arriving from the website.
pub fn bank_title(key: &str) -> &str {
    match key {
        "aphasia" => "AphasiaBank",
        "asd" => "ASDBank",
        "biling" => "BilingBank",
        "ca" => "CABank",
        "childes" => "CHILDES",
        "class" => "ClassBank",
        "dementia" => "DementiaBank",
        "fluency" => "FluencyBank",
        "homebank" => "HomeBank",
        "phon" => "PhonBank",
        "psychosis" => "PsychosisBank",
        "rhd" => "RHDBank",
        "samtale" => "SamtaleBank",
        "slabank" => "SLABank",
        "tbi" => "TBIBank",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Reproduces the two shapes seen in the archive: with a collection above
    /// the corpus (CHILDES) and without one (CABank).
    fn tree() -> serde_json::Value {
        json!({"respMsg": {
            "childes": {"childes": {
                "Eng-NA": {
                    "Brown": {
                        "Adam": {"a1": {"file": true, "media": null},
                                 "a2": {"file": true, "media": "audio"}},
                        "Eve":  {"e1": {"file": true, "media": "audio,unlinked"}}
                    },
                    "Haggerty": {"haggerty": {"file": true, "media": null}}
                }
            }},
            "ca": {"ca": {
                // here the corpus sits directly under the bank, and holds both
                // files and a subfolder
                "ATC": {
                    "katl": {"file": true, "media": "audio"},
                    "kbna": {"file": true, "media": "audio"},
                    "disasters": {"d1": {"file": true, "media": null}}
                }
            }},
            "phon": {"phon": {"Eng-NA": {"Davis": {"x": {"file": true, "media": null}}}}}
        }})
    }

    #[test]
    fn it_counts_transcripts_at_any_depth() {
        let a = parse(&tree()).unwrap();
        let brown = a
            .at(&["childes".into(), "Eng-NA".into(), "Brown".into()])
            .unwrap();
        assert_eq!(brown.transcripts, 3);
        assert_eq!(brown.direct_files, 0, "Brown holds only subfolders");

        let atc = a.at(&["ca".into(), "ATC".into()]).unwrap();
        assert_eq!(atc.transcripts, 3, "two direct files plus one in the subfolder");
        assert_eq!(atc.direct_files, 2);
    }

    #[test]
    fn the_doubled_level_is_stepped_through() {
        let a = parse(&tree()).unwrap();
        let childes = a.bank("childes").unwrap();
        assert_eq!(
            childes.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Eng-NA"],
            "if the doubling were not handled, \"childes\" would show up"
        );
    }

    #[test]
    fn banks_with_one_level_fewer_work_the_same() {
        let a = parse(&tree()).unwrap();
        // In CABank the corpus is a direct child of the bank: the path has two
        // elements instead of three, and has to walk just the same.
        assert!(a.at(&["ca".into(), "ATC".into()]).is_some());
        assert!(a.at(&["ca".into(), "ATC".into(), "disasters".into()]).is_some());
        assert!(a.at(&["ca".into(), "nosuchthing".into()]).is_none());
    }

    #[test]
    fn media_flags_merge_on_the_way_up() {
        let a = parse(&tree()).unwrap();
        let brown = a.at(&["childes".into(), "Eng-NA".into(), "Brown".into()]).unwrap();
        assert!(brown.media.audio);
        assert!(brown.media.unlinked);
        assert!(!brown.media.video);
        assert!(brown.media.incomplete());
    }

    #[test]
    fn every_media_string_seen_in_the_archive_parses() {
        for (s, audio, video) in [
            ("audio", true, false),
            ("audio,missing", true, false),
            ("audio,notrans", true, false),
            ("audio,unlinked", true, false),
            ("video", false, true),
            ("video,missing", false, true),
            ("", false, false),
        ] {
            let m = Media::parse(s);
            assert_eq!(m.audio, audio, "audio in \"{s}\"");
            assert_eq!(m.video, video, "video in \"{s}\"");
        }
    }

    #[test]
    fn search_crosses_every_depth_and_gives_the_path() {
        let a = parse(&tree()).unwrap();
        let hits = a.search("childes", "brown");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, ["childes", "Eng-NA", "Brown"]);

        // a subfolder inside a corpus is reachable too
        let hits = a.search("ca", "disasters");
        assert_eq!(hits[0].0, ["ca", "ATC", "disasters"]);

        // empty query: every folder that holds transcripts
        let all = a.search("childes", "");
        assert_eq!(all.len(), 5, "Eng-NA, Brown, Adam, Eve, Haggerty");
        assert!(a.search("no-such-bank", "").is_empty());
    }

    #[test]
    fn banks_have_a_readable_name() {
        assert_eq!(bank_title("phon"), "PhonBank");
        assert_eq!(bank_title("ca"), "CABank");
        assert_eq!(bank_title("childes"), "CHILDES");
        // a new bank we do not know about must not disappear
        assert_eq!(bank_title("newbank"), "newbank");
    }

    #[test]
    fn a_malformed_response_errors_instead_of_panicking() {
        assert!(parse(&json!({})).is_err());
        assert!(parse(&json!({"respMsg": "text"})).is_err());
        let a = parse(&json!({"respMsg": {"x": {"x": {"Coll": 42}}}})).unwrap();
        assert_eq!(a.banks.len(), 1);
        assert_eq!(a.banks[0].children.len(), 0);
    }
}
