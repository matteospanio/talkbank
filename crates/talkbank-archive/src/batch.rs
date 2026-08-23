//! Planning the download of a whole branch.
//!
//! A folder such as `childes/Eng-NA` is not downloadable: it holds corpora, it
//! is not a corpus. Opening its sixty subfolders one at a time to download them
//! all is exactly the work a program ought to save you.
//!
//! The catch is that *which* descendants are corpora cannot be deduced from the
//! tree — only the server knows, one HEAD request at a time. So the walk
//! alternates: probe a level, descend only where the answer was "not a corpus".
//!
//! The walk is deliberately separate from the network. [`Walk`] is a pure state
//! machine: it says which paths to probe, takes the answers, and decides where
//! to descend. That way the pruning rules can be tested without touching the
//! service, and [`plan`] stays a thin shell that only adds the I/O.

use std::collections::VecDeque;

use crate::api::{Client, Downloadable};
use crate::catalog::Archive;

/// Why a folder was left out of the plan.
///
/// It carries the weight of what is being lost. Saying "1 folder skipped" tells
/// nobody anything: a thousand transcripts can sit under a 401, and that number
/// is already known from the tree, without asking the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// Restricted-access bank or corpus (401).
    NeedsPermission { folders: usize, transcripts: usize },
    /// We could not ask: network, timeout, server failure.
    Unverifiable {
        reason: String,
        folders: usize,
        transcripts: usize,
    },
}

impl Skipped {
    pub fn transcripts(&self) -> usize {
        match self {
            Skipped::NeedsPermission { transcripts, .. }
            | Skipped::Unverifiable { transcripts, .. } => *transcripts,
        }
    }
    pub fn folders(&self) -> usize {
        match self {
            Skipped::NeedsPermission { folders, .. } | Skipped::Unverifiable { folders, .. } => {
                *folders
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The corpora to download. This is the **minimal** set covering the branch:
    /// if a folder is downloadable its zip already contains everything below it,
    /// so its descendants do not appear.
    pub corpora: Vec<Vec<String>>,
    /// Folders left out, with the reason.
    pub skipped: Vec<(Vec<String>, Skipped)>,
    /// Transcripts covered by the chosen corpora.
    pub transcripts: usize,
    /// How many folders were probed to get here.
    pub probed: usize,
    /// True when the service answered "sign-in required": the plan is not
    /// trustworthy and has to be redone after signing in.
    pub needs_sign_in: bool,
    /// True when the walk stopped at the probe ceiling: the plan is partial, and
    /// saying so beats letting the user believe it is everything.
    pub truncated: bool,
    /// True when the user cancelled. Different from `truncated`: there the
    /// archive is deeper than one sweep covers, here it was a decision.
    pub cancelled: bool,
    /// True when the service stopped answering usefully. A plan like that must
    /// not be offered: it would look complete and is not.
    pub unreliable: bool,
    /// What was still to be examined when the walk stopped. It allows resuming
    /// without paying again for the probes already made.
    pub resume: Vec<Vec<String>>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.corpora.is_empty()
    }
    /// How many folders were left out, counting their subtrees.
    pub fn skipped_folders(&self) -> usize {
        self.skipped.iter().map(|(_, s)| s.folders()).sum()
    }
    /// How many transcripts will not arrive.
    pub fn skipped_transcripts(&self) -> usize {
        self.skipped.iter().map(|(_, s)| s.transcripts()).sum()
    }
    /// The transcripts left out because separate permission would be needed.
    pub fn locked_transcripts(&self) -> usize {
        self.skipped
            .iter()
            .filter(|(_, s)| matches!(s, Skipped::NeedsPermission { .. }))
            .map(|(_, s)| s.transcripts())
            .sum()
    }
    pub fn locked_folders(&self) -> usize {
        self.skipped
            .iter()
            .filter(|(_, s)| matches!(s, Skipped::NeedsPermission { .. }))
            .count()
    }
    /// Merges the continuation of a resumed walk into this plan.
    ///
    /// The two pieces do not overlap: the continuation starts exactly from what
    /// the first one put in `resume`.
    pub fn merged(mut self, rest: Plan) -> Plan {
        self.corpora.extend(rest.corpora);
        self.corpora.sort();
        self.corpora.dedup();
        self.skipped.extend(rest.skipped);
        self.skipped.sort_by(|a, b| a.0.cmp(&b.0));
        self.skipped.dedup_by(|a, b| a.0 == b.0);
        self.transcripts += rest.transcripts;
        self.probed += rest.probed;
        // The continuation's flags are the ones that count: they say whether
        // anything is left to examine *now*.
        self.truncated = rest.truncated;
        self.cancelled = rest.cancelled;
        self.unreliable = rest.unreliable;
        self.needs_sign_in = rest.needs_sign_in;
        self.resume = rest.resume;
        self
    }

    /// The folders that could not be verified: they are holes in the plan, and
    /// whoever decides needs to know.
    pub fn unverified(&self) -> usize {
        self.skipped
            .iter()
            .filter(|(_, s)| matches!(s, Skipped::Unverifiable { .. }))
            .count()
    }
}

/// The probe ceiling for a single planning run.
///
/// It exists for one real case: "download everything" pressed on the root of a
/// large bank. CHILDES has 1,933 folders; without a ceiling, planning would
/// become a burst of thousands of requests against an academic server. A partial
/// plan is declared as such and can be resumed.
///
/// Measured: the whole of CHILDES costs 315 probes with pruning (423 in the
/// worst case, counting the corpora that sit one level deeper than expected).
/// Five hundred leaves room without opening the door to thousands.
pub const MAX_PROBES: usize = 500;

/// How many uncertain answers in a row before giving up. If the service stops
/// answering, hammering it helps neither it nor us.
const MAX_CONSECUTIVE_UNKNOWN: usize = 3;

/// The subtree walk, without networking.
///
/// Usage: ask for [`Walk::next_batch`], probe those paths, report each answer
/// with [`Walk::record`], repeat until [`Walk::done`].
pub struct Walk<'a> {
    archive: &'a Archive,
    queue: VecDeque<Vec<String>>,
    plan: Plan,
    budget: usize,
    /// Paths whose answer was uncertain: retried once, at the end, because a
    /// network hiccup must not drop a corpus from the plan.
    retry: Vec<Vec<String>>,
    /// True while we are serving the retry queue: from there on a second
    /// uncertain answer is final.
    retrying: bool,
    consecutive_unknown: usize,
    /// Paths handed to the prober and not yet returned. Without tracking them,
    /// stopping mid-round would lose them: they would be neither in the plan nor
    /// among those to resume.
    in_flight: Vec<Vec<String>>,
}

impl<'a> Walk<'a> {
    pub fn new(archive: &'a Archive, root: &[String]) -> Walk<'a> {
        Walk::with_budget(archive, root, MAX_PROBES)
    }

    pub fn with_budget(archive: &'a Archive, root: &[String], budget: usize) -> Walk<'a> {
        let mut queue = VecDeque::new();
        queue.push_back(root.to_vec());
        Walk {
            archive,
            queue,
            plan: Plan::default(),
            budget,
            retry: Vec::new(),
            retrying: false,
            consecutive_unknown: 0,
            in_flight: Vec::new(),
        }
    }

    pub fn done(&self) -> bool {
        (self.queue.is_empty() && self.retry.is_empty())
            || self.plan.needs_sign_in
            || self.plan.truncated
            || self.plan.unreliable
    }

    /// True once the main queue is done and only retries are left. Whoever
    /// drives the walk can wait a moment before asking again.
    pub fn is_retrying(&self) -> bool {
        self.retrying
    }

    /// The next paths to probe, at most `n`.
    pub fn next_batch(&mut self, n: usize) -> Vec<Vec<String>> {
        if self.done() {
            return Vec::new();
        }
        // With the main queue empty, retry once what came back uncertain.
        if self.queue.is_empty() && !self.retry.is_empty() {
            self.retrying = true;
            self.queue.extend(self.retry.drain(..));
        }
        let take = n.min(self.queue.len()).min(self.budget.saturating_sub(self.plan.probed));
        let batch: Vec<Vec<String>> = (0..take).filter_map(|_| self.queue.pop_front()).collect();
        self.in_flight.extend(batch.iter().cloned());
        batch
    }

    /// Records the answer for a path and decides whether to descend.
    pub fn record(&mut self, path: Vec<String>, outcome: Downloadable) {
        // Once we have decided the service is not answering, answers still in
        // flight change nothing: they stay among those to resume.
        if self.plan.unreliable {
            return;
        }
        self.in_flight.retain(|p| p != &path);
        self.plan.probed += 1;
        if !matches!(outcome, Downloadable::Unknown(_)) {
            self.consecutive_unknown = 0;
        }
        match outcome {
            Downloadable::Yes => {
                // Pruning: this folder's zip already holds everything below it,
                // so the children need neither probing nor downloading. It is
                // also what keeps the same material from being downloaded twice.
                self.plan.transcripts += self
                    .archive
                    .at(&path)
                    .map(|f| f.transcripts)
                    .unwrap_or(0);
                self.plan.corpora.push(path);
            }
            Downloadable::NeedsPermission => {
                // Measured: permission propagates to descendants
                // (aphasia/English/Protocol and /Protocol/Adler both answer
                // 401). Descending would be wasted politeness.
                let (folders, transcripts) = self.weight(&path);
                self.plan.skipped.push((
                    path,
                    Skipped::NeedsPermission {
                        folders,
                        transcripts,
                    },
                ));
            }
            Downloadable::Unknown(e) => {
                self.consecutive_unknown += 1;
                if self.retrying {
                    let (folders, transcripts) = self.weight(&path);
                    self.plan.skipped.push((
                        path,
                        Skipped::Unverifiable {
                            reason: e,
                            folders,
                            transcripts,
                        },
                    ));
                } else {
                    self.retry.push(path);
                }
                if self.consecutive_unknown >= MAX_CONSECUTIVE_UNKNOWN {
                    // The service is not answering: insisting makes it worse.
                    self.plan.unreliable = true;
                    self.stash_remaining();
                }
                return;
            }
            Downloadable::SignInRequired => {
                // Without a session every answer is the same: carrying on would
                // produce a made-up plan.
                self.plan.needs_sign_in = true;
                self.queue.clear();
            }
            Downloadable::No => {
                let Some(node) = self.archive.at(&path) else { return };
                for child in &node.children {
                    // Folders with no transcripts have nothing to download:
                    // probing them would be a wasted request.
                    if child.transcripts == 0 {
                        continue;
                    }
                    let mut p = path.clone();
                    p.push(child.name.clone());
                    self.queue.push_back(p);
                }
            }
        }
        if self.plan.probed >= self.budget && !(self.queue.is_empty() && self.retry.is_empty()) {
            self.plan.truncated = true;
            self.stash_remaining();
        }
    }

    /// The weight of a subtree, read from the tree we already have: no extra
    /// request needed to know how much is being lost.
    fn weight(&self, path: &[String]) -> (usize, usize) {
        let Some(node) = self.archive.at(path) else {
            return (0, 0);
        };
        fn count(f: &crate::catalog::Folder) -> usize {
            1 + f
                .children
                .iter()
                .filter(|c| c.transcripts > 0)
                .map(count)
                .sum::<usize>()
        }
        (count(node), node.transcripts)
    }

    /// Sets aside what is left, so the walk can be resumed.
    fn stash_remaining(&mut self) {
        self.plan.resume = self
            .in_flight
            .drain(..)
            .chain(self.queue.drain(..))
            .chain(self.retry.drain(..))
            .collect();
        self.plan.resume.sort();
        self.plan.resume.dedup();
    }

    /// Replaces the queue with a known frontier. Used when resuming: the root
    /// does not need re-probing, we pick up where we left off.
    pub fn reset_queue(&mut self, frontier: &[Vec<String>]) {
        self.queue.clear();
        self.queue.extend(frontier.iter().cloned());
    }

    /// Marks that it was the user who stopped.
    pub fn cancel(&mut self) {
        self.plan.cancelled = true;
        self.stash_remaining();
    }

    /// How many folders are still queued. Only used for progress reporting.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    pub fn finish(mut self) -> Plan {
        // What was still queued is neither done nor skipped: it is to be resumed.
        if !self.queue.is_empty() || !self.retry.is_empty() || !self.in_flight.is_empty() {
            self.stash_remaining();
        }
        // A stable order makes the confirmation readable and the tests
        // deterministic.
        self.plan.corpora.sort();
        self.plan.skipped.sort_by(|a, b| a.0.cmp(&b.0));
        self.plan
    }
}

/// How many probes run in parallel. Same value as the metadata index: the server
/// is small and academic.
const CONCURRENCY: usize = 4;

/// Plans the download of the branch rooted at `root`.
///
/// `on_progress(probed, queued)` is called as each level finishes.
/// `should_continue` allows cancellation: returning `false` stops the walk and
/// the plan comes back partial.
pub async fn plan(
    client: &Client,
    archive: &Archive,
    root: &[String],
    on_progress: impl FnMut(usize, usize),
    should_continue: impl Fn() -> bool,
) -> Plan {
    drive(Walk::new(archive, root), client, on_progress, should_continue).await
}

/// Resumes a planning run from where it stopped.
///
/// `frontier` is the previous plan's `resume`: the probes already made are not
/// repeated, and on a large bank those number in the hundreds.
pub async fn plan_from(
    client: &Client,
    archive: &Archive,
    frontier: &[Vec<String>],
    mut on_progress: impl FnMut(usize),
    should_continue: impl Fn() -> bool,
) -> Plan {
    let mut walk = Walk::with_budget(archive, &[], MAX_PROBES);
    walk.reset_queue(frontier);
    drive(
        walk,
        client,
        move |done, _| on_progress(done),
        should_continue,
    )
    .await
}

async fn drive<'a>(
    mut walk: Walk<'a>,
    client: &Client,
    mut on_progress: impl FnMut(usize, usize),
    should_continue: impl Fn() -> bool,
) -> Plan {
    use futures_util::stream::{self, StreamExt};

    while !walk.done() {
        if !should_continue() {
            walk.cancel();
            return walk.finish();
        }
        // Before retrying what came back uncertain it is worth letting a moment
        // pass: if the hiccup was the server's, repeating at once finds it in
        // the same state.
        if walk.is_retrying() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        let batch = walk.next_batch(CONCURRENCY);
        if batch.is_empty() {
            break;
        }
        let answers: Vec<(Vec<String>, Downloadable)> = stream::iter(batch.into_iter().map(|path| {
            let client = client.clone();
            async move {
                let out = client.is_downloadable(&path).await;
                (path, out)
            }
        }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;

        for (path, out) in answers {
            walk.record(path, out);
        }
        on_progress(walk.plan.probed, walk.pending());
    }
    walk.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    /// A tree with the two real shapes: CHILDES with a collection above the
    /// corpus, CABank without one.
    fn tree() -> Archive {
        let v = json!({"respMsg": {
            "childes": {"childes": {
                "Eng-NA": {
                    "Brown": {
                        "Adam": {"a1": {"file": true, "media": null}},
                        "Eve":  {"e1": {"file": true, "media": null}}
                    },
                    "Haggerty": {"h1": {"file": true, "media": null}},
                    "Empty": {}
                },
                "Clinical": {
                    "Bliss": {"b1": {"file": true, "media": null}}
                }
            }},
            "ca": {"ca": {
                "ATC": {
                    "katl": {"file": true, "media": null},
                    "disasters": {"d1": {"file": true, "media": null}}
                }
            }}
        }});
        crate::catalog::parse(&v).unwrap()
    }

    fn p(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    /// Runs the walk with answers decided up front. Also returns the order in
    /// which the paths were probed.
    fn run(
        archive: &Archive,
        root: &[&str],
        answers: &HashMap<String, Downloadable>,
    ) -> (Plan, Vec<String>) {
        let mut walk = Walk::new(archive, &p(root));
        let mut seen = Vec::new();
        while !walk.done() {
            let batch = walk.next_batch(4);
            if batch.is_empty() {
                break;
            }
            for path in batch {
                let key = path.join("/");
                let out = answers.get(&key).cloned().unwrap_or(Downloadable::No);
                seen.push(key);
                walk.record(path, out);
            }
        }
        (walk.finish(), seen)
    }

    fn answers(pairs: &[(&str, Downloadable)]) -> HashMap<String, Downloadable> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn it_descends_only_where_there_is_no_corpus() {
        let a = tree();
        let e = answers(&[
            ("childes/Eng-NA", Downloadable::No),
            ("childes/Eng-NA/Brown", Downloadable::Yes),
            ("childes/Eng-NA/Haggerty", Downloadable::Yes),
        ]);
        let (plan, seen) = run(&a, &["childes", "Eng-NA"], &e);

        assert_eq!(
            plan.corpora,
            vec![
                p(&["childes", "Eng-NA", "Brown"]),
                p(&["childes", "Eng-NA", "Haggerty"])
            ]
        );
        // Nothing below Brown was probed: Brown's zip already holds Adam and Eve.
        assert!(
            !seen.iter().any(|v| v.contains("Brown/")),
            "it must not probe the children of a corpus: {seen:?}"
        );
        assert_eq!(plan.transcripts, 3, "Brown 2 + Haggerty 1");
        assert!(!plan.truncated && !plan.needs_sign_in);
    }

    #[test]
    fn folders_with_no_transcripts_are_not_probed() {
        let a = tree();
        let e = answers(&[("childes/Eng-NA", Downloadable::No)]);
        let (_, seen) = run(&a, &["childes", "Eng-NA"], &e);
        assert!(
            !seen.iter().any(|v| v.ends_with("Empty")),
            "an empty folder has nothing to download: {seen:?}"
        );
    }

    #[test]
    fn a_multi_level_branch_flattens_to_the_corpora_alone() {
        let a = tree();
        let e = answers(&[
            ("childes", Downloadable::No),
            ("childes/Eng-NA", Downloadable::No),
            ("childes/Clinical", Downloadable::No),
            ("childes/Eng-NA/Brown", Downloadable::Yes),
            ("childes/Eng-NA/Haggerty", Downloadable::Yes),
            ("childes/Clinical/Bliss", Downloadable::Yes),
        ]);
        let (plan, _) = run(&a, &["childes"], &e);
        assert_eq!(plan.corpora.len(), 3);
        assert_eq!(plan.transcripts, 4);
    }

    #[test]
    fn a_bank_with_no_collection_level_works_the_same() {
        let a = tree();
        let e = answers(&[("ca", Downloadable::No), ("ca/ATC", Downloadable::Yes)]);
        let (plan, seen) = run(&a, &["ca"], &e);
        assert_eq!(plan.corpora, vec![p(&["ca", "ATC"])]);
        assert!(!seen.iter().any(|v| v.contains("disasters")));
    }

    #[test]
    fn if_the_root_is_already_a_corpus_the_plan_holds_only_it() {
        let a = tree();
        let e = answers(&[("childes/Eng-NA/Brown", Downloadable::Yes)]);
        let (plan, seen) = run(&a, &["childes", "Eng-NA", "Brown"], &e);
        assert_eq!(plan.corpora, vec![p(&["childes", "Eng-NA", "Brown"])]);
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn restricted_corpora_are_skipped_and_counted() {
        let a = tree();
        let e = answers(&[
            ("childes/Eng-NA", Downloadable::No),
            ("childes/Eng-NA/Brown", Downloadable::NeedsPermission),
            ("childes/Eng-NA/Haggerty", Downloadable::Yes),
        ]);
        let (plan, seen) = run(&a, &["childes", "Eng-NA"], &e);
        assert_eq!(plan.corpora, vec![p(&["childes", "Eng-NA", "Haggerty"])]);
        assert_eq!(plan.locked_folders(), 1);
        assert_eq!(plan.locked_transcripts(), 2, "Brown had two");
        assert_eq!(plan.transcripts, 1, "the skipped corpus does not count");
        // and we do not descend into a branch that cannot be downloaded anyway
        assert!(!seen.iter().any(|v| v.contains("Brown/")));
    }

    #[test]
    fn without_sign_in_the_walk_stops_instead_of_inventing() {
        let a = tree();
        let e = answers(&[("childes/Eng-NA", Downloadable::SignInRequired)]);
        let (plan, seen) = run(&a, &["childes", "Eng-NA"], &e);
        assert!(plan.needs_sign_in);
        assert!(plan.is_empty());
        assert_eq!(seen.len(), 1, "one request and no more");
    }

    #[test]
    fn an_uncertain_answer_is_retried_exactly_once() {
        let a = tree();
        let e = answers(&[
            ("childes/Eng-NA", Downloadable::No),
            ("childes/Eng-NA/Brown", Downloadable::Unknown("dns".into())),
            ("childes/Eng-NA/Haggerty", Downloadable::Yes),
        ]);
        let (plan, seen) = run(&a, &["childes", "Eng-NA"], &e);
        assert_eq!(plan.corpora.len(), 1, "Haggerty still gets through");
        assert_eq!(plan.skipped.len(), 1);
        assert!(matches!(plan.skipped[0].1, Skipped::Unverifiable { .. }));
        // Brown shows up twice: once in the normal round, once in the retry.
        // Not three times.
        assert_eq!(
            seen.iter().filter(|v| v.ends_with("Brown")).count(),
            2,
            "one retry, and only one: {seen:?}"
        );
    }

    #[test]
    fn an_uncertain_answer_that_succeeds_on_retry_enters_the_plan() {
        let a = tree();
        // The same folder answers badly the first time and well the second.
        let mut times = std::collections::HashMap::new();
        let mut walk = Walk::new(&a, &p(&["childes", "Eng-NA"]));
        while !walk.done() {
            let batch = walk.next_batch(4);
            if batch.is_empty() {
                break;
            }
            for path in batch {
                let key = path.join("/");
                let n = times.entry(key.clone()).or_insert(0);
                *n += 1;
                let out = if key.ends_with("Brown") && *n == 1 {
                    Downloadable::Unknown("timeout".into())
                } else if key.ends_with("Brown") || key.ends_with("Haggerty") {
                    Downloadable::Yes
                } else {
                    Downloadable::No
                };
                walk.record(path, out);
            }
        }
        let plan = walk.finish();
        assert_eq!(plan.corpora.len(), 2, "the retry recovers Brown");
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn three_uncertain_answers_in_a_row_stop_the_walk() {
        let a = tree();
        // The root answers, then the service breaks: everything is uncertain
        // from there on. The walk must not keep knocking.
        let mut walk = Walk::new(&a, &p(&["childes"]));
        let mut first = true;
        while !walk.done() {
            let batch = walk.next_batch(4);
            if batch.is_empty() {
                break;
            }
            for path in batch {
                let out = if first {
                    first = false;
                    Downloadable::No
                } else {
                    Downloadable::Unknown("HTTP 503".into())
                };
                walk.record(path, out);
            }
        }
        let plan = walk.finish();
        assert!(plan.unreliable, "a service that stops answering must be declared");
        assert!(plan.is_empty(), "and no plan built on that is offered");
        assert!(
            !plan.resume.is_empty(),
            "what was left is kept, to try again later"
        );
    }

    #[test]
    fn cancelling_is_not_the_same_as_hitting_the_ceiling() {
        let a = tree();
        let mut walk = Walk::new(&a, &p(&["childes"]));
        let batch = walk.next_batch(4);
        for path in batch {
            walk.record(path, Downloadable::No);
        }
        walk.cancel();
        let plan = walk.finish();
        assert!(plan.cancelled);
        assert!(!plan.truncated, "the ceiling has nothing to do with it");
        assert!(!plan.resume.is_empty(), "it must be possible to resume");
    }

    #[test]
    fn skipped_entries_carry_the_weight_of_what_is_lost() {
        let a = tree();
        let e = answers(&[
            ("childes", Downloadable::No),
            ("childes/Eng-NA", Downloadable::NeedsPermission),
            ("childes/Clinical", Downloadable::No),
            ("childes/Clinical/Bliss", Downloadable::Yes),
        ]);
        let (plan, _) = run(&a, &["childes"], &e);
        // Eng-NA holds Brown (2) and Haggerty (1): three transcripts lost.
        assert_eq!(plan.locked_transcripts(), 3);
        // The folders with data under Eng-NA are five: Eng-NA, Brown, Adam, Eve,
        // Haggerty. "Empty" does not count, it has no transcripts.
        assert_eq!(plan.skipped_folders(), 5);
        // But there is only one skipped entry: that is what the user is told.
        assert_eq!(plan.locked_folders(), 1);
        assert_eq!(plan.transcripts, 1, "only Bliss is left");
    }

    #[test]
    fn the_ceiling_stops_the_walk_and_says_so() {
        let a = tree();
        let e = answers(&[
            ("childes", Downloadable::No),
            ("childes/Eng-NA", Downloadable::No),
            ("childes/Clinical", Downloadable::No),
        ]);
        let mut walk = Walk::with_budget(&a, &p(&["childes"]), 2);
        while !walk.done() {
            let batch = walk.next_batch(4);
            if batch.is_empty() {
                break;
            }
            for path in batch {
                let out = e.get(&path.join("/")).cloned().unwrap_or(Downloadable::No);
                walk.record(path, out);
            }
        }
        let plan = walk.finish();
        assert!(plan.truncated, "the plan is partial and must say so");
        assert_eq!(plan.probed, 2);
    }

    #[test]
    fn a_nonexistent_path_does_not_panic() {
        let a = tree();
        let e = answers(&[("childes/MadeUp", Downloadable::No)]);
        let (plan, _) = run(&a, &["childes", "MadeUp"], &e);
        assert!(plan.is_empty());
        assert_eq!(plan.probed, 1);
    }
}
