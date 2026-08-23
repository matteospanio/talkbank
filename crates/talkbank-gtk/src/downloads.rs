//! Corpus downloads, in the background.
//!
//! Like a browser: they start, they carry on while you do something else, and a
//! button shows how many are in flight. The work is **not** tied to the page
//! that started it — changing section or going back to the list of banks does
//! not interrupt it.
//!
//! It is a **queue**, not a pool of parallel jobs: downloading a whole branch
//! can mean dozens of corpora, and opening dozens of connections to a small
//! academic server would be rude as well as pointless.
//!
//! A system notification arrives when a download finishes, **unless** the
//! archive is already on screen: announcing something that is right there is
//! noise, not information. A group is announced once, at the end: forty
//! notifications are not forty times more useful than one.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adw::prelude::*;
use gtk::gio;

use crate::i18n::{t, tn};
use crate::net::net;
use crate::window::App;

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Queued: accepted, not started yet.
    Queued,
    /// Bytes downloaded. The server sends no `Content-Length`, so there is no
    /// percentage: only how much has arrived.
    Downloading(u64),
    Extracting { done: usize, total: usize },
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

impl Phase {
    /// True until the job is finished: queued or running.
    pub fn pending(&self) -> bool {
        matches!(
            self,
            Phase::Queued | Phase::Downloading(_) | Phase::Extracting { .. }
        )
    }
    /// True only while actually transferring or extracting.
    pub fn running(&self) -> bool {
        matches!(self, Phase::Downloading(_) | Phase::Extracting { .. })
    }
}

/// What a queue entry actually transfers.
///
/// Both kinds share `path` — the corpus's archive path — so that `where_from()`
/// keeps working and a media file is shown under the corpus it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobKind {
    /// The zip of transcripts. `with_media` means: when this lands, read the
    /// `@Media` headers of what came out and queue the recordings too.
    Corpus { with_media: bool },
    /// One recording. Media are not in the zip, so each is its own transfer.
    Media { url: String, dest: PathBuf },
}

impl JobKind {
    pub fn is_corpus(&self) -> bool {
        matches!(self, JobKind::Corpus { .. })
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    /// Path in the archive, bank included.
    pub path: Vec<String>,
    pub phase: Phase,
    pub kind: JobKind,
    /// Where it gets extracted. Needed to start it when it leaves the queue.
    dest_root: PathBuf,
    /// Raised to cancel: it is read on the network thread, so it is an atomic
    /// rather than a plain cell.
    cancel: Arc<AtomicBool>,
    /// True once this outcome has been reported to the user.
    announced: bool,
    /// The root of the branch it came from, if it was queued as a group. That is
    /// the folder to open when the work finishes: whichever corpus happens to
    /// finish last says nothing about where the rest ended up.
    group_root: Option<Vec<String>>,
    /// How many corpora were queued together with this one. It is what allows
    /// "23 of 24": without the expected total, a lost corpus goes unnoticed.
    group_total: usize,
    /// When the interface was last told about this job's byte progress. Without
    /// it a panel of forty rows would rebuild on every network packet: it
    /// flickers, and it makes the cancel button hard to hit.
    last_notified: Option<std::time::Instant>,
}

/// Why the queue is paused. These are the two cases where carrying on would fail
/// everything else the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paused {
    SignInNeeded,
    DiskFull,
}

impl Job {
    /// The name to show: the corpus folder, or the recording's filename.
    pub fn title(&self) -> String {
        match &self.kind {
            JobKind::Corpus { .. } => self.path.last().cloned().unwrap_or_default(),
            JobKind::Media { dest, .. } => dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
    pub fn where_from(&self) -> String {
        let bank = self
            .path
            .first()
            .map(|b| talkbank_archive::catalog::bank_title(b).to_string())
            .unwrap_or_default();
        // A media file belongs *inside* the corpus, so its origin includes the
        // corpus name that a corpus job puts in the title instead.
        let upto = if self.kind.is_corpus() {
            self.path.len().saturating_sub(1)
        } else {
            self.path.len()
        };
        let rest = self.path[1..upto.max(1)].join(" / ");
        if rest.is_empty() {
            bank
        } else {
            format!("{bank} · {rest}")
        }
    }
    pub fn status(&self) -> String {
        match &self.phase {
            Phase::Queued => t("waiting"),
            Phase::Cancelled => t("cancelled"),
            Phase::Downloading(b) => {
                t("%s so far").replace("%s", &format!("{:.1} MB", *b as f64 / 1_048_576.0))
            }
            Phase::Extracting { done, total } => t("extracting %d of %t")
                .replace("%d", &done.to_string())
                .replace("%t", &total.to_string()),
            Phase::Done(_) => t("Finished"),
            Phase::Failed(e) => e.clone(),
        }
    }
}

/// How often byte progress reaches the interface, per job.
///
/// Four times a second is as much as a progress bar can usefully say.
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Whether a byte tick is worth redrawing for.
///
/// Time only, deliberately. The earlier rule was "250 ms **or** a megabyte",
/// and an `or` can only make a throttle fire more often, never less: on a
/// 400 MB recording over a fast link the megabyte clause meant dozens of
/// notifications a second, each one rebuilding every row of the panel.
fn progress_is_worth_showing(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    match last {
        Some(when) => now.duration_since(when) >= PROGRESS_INTERVAL,
        None => true,
    }
}

/// How many corpora at a time. Two keep the bandwidth busy without turning a
/// forty-corpus branch into forty connections against a small server.
const PARALLEL: usize = 2;

/// Anyone who wants to be told when the list changes.
///
/// It carries the widget it is attached to, and not on a whim: every archive
/// page visited registers its own watchers, and pages die while the manager
/// lives on. With no way of noticing, the closures would go on touching
/// destroyed widgets — and would keep them alive meanwhile, one page at a time,
/// for the whole session.
struct Watcher {
    id: u64,
    /// The widget whose death ends the watcher.
    anchor: gtk::Widget,
    f: Rc<dyn Fn()>,
}

struct Inner {
    app: App,
    jobs: RefCell<Vec<Job>>,
    next_id: RefCell<u64>,
    /// True while we are starting jobs: `pump` is also called from the
    /// job-finished handlers, and without a guard it would re-enter.
    pumping: Cell<bool>,
    /// When set, the queue is stopped and no job starts. **Not** cancelled: an
    /// expired session must not throw away thirty-five corpora still to
    /// download, and resuming must not cost a fresh planning run.
    paused: RefCell<Option<Paused>>,
    watchers: RefCell<Vec<Watcher>>,
}

#[derive(Clone)]
pub struct Manager(Rc<Inner>);

impl Manager {
    pub fn new(app: &App) -> Manager {
        Manager(Rc::new(Inner {
            app: app.clone(),
            jobs: RefCell::new(Vec::new()),
            next_id: RefCell::new(1),
            pumping: Cell::new(false),
            paused: RefCell::new(None),
            watchers: RefCell::new(Vec::new()),
        }))
    }

    pub fn jobs(&self) -> Vec<Job> {
        self.0.jobs.borrow().clone()
    }

    /// The corpus download for this path, if there is one.
    ///
    /// Media jobs share the corpus's `path`, so they have to be filtered out:
    /// `inline()` draws the corpus progress bar from this, and a recording
    /// would otherwise hijack it.
    pub fn job_for(&self, path: &[String]) -> Option<Job> {
        self.0
            .jobs
            .borrow()
            .iter()
            .find(|j| j.path == path && j.kind.is_corpus())
            .cloned()
    }

    /// Registers a watcher, tying it to the life of `anchor`.
    ///
    /// Removal is **notified**, not inferred: the widget signals when it leaves
    /// the tree, and the watcher disappears at that moment. Inferring it from a
    /// weak reference would not work, because the closure itself is what keeps
    /// its own widgets alive.
    pub fn watch_for(&self, anchor: &impl IsA<gtk::Widget>, f: impl Fn() + 'static) {
        let id = {
            let mut n = self.0.next_id.borrow_mut();
            *n += 1;
            *n
        };
        let w: gtk::Widget = anchor.clone().upcast();
        self.0.watchers.borrow_mut().push(Watcher {
            id,
            anchor: w.clone(),
            f: Rc::new(f),
        });

        // At registration the widget is not in the tree yet: it is built and only
        // then added to the page. So we wait to see it enter, and from that point
        // on its leaving counts as death.
        let entered = Cell::new(w.root().is_some());
        let m = self.clone();
        w.connect_root_notify(move |w| {
            if w.root().is_some() {
                entered.set(true);
            } else if entered.get() {
                m.unwatch(id);
            }
        });
    }

    fn unwatch(&self, id: u64) {
        self.0.watchers.borrow_mut().retain(|w| w.id != id);
    }

    fn notify_watchers(&self) {
        // Collect first, call afterwards: a closure can change the state and
        // re-enter here, and iterating while inside the `borrow` would explode.
        let to_call: Vec<Rc<dyn Fn()>> = self
            .0
            .watchers
            .borrow()
            .iter()
            // A page detached from the tree has nothing to update. Removal is
            // handled by `connect_root_notify`; here we only avoid touching it
            // in the window between the two events.
            .filter(|w| w.anchor.root().is_some())
            .map(|w| w.f.clone())
            .collect();
        for f in to_call {
            f();
        }
    }

    /// Drops the finished downloads from the list.
    pub fn clear_finished(&self) {
        self.0.jobs.borrow_mut().retain(|j| j.phase.pending());
        self.notify_watchers();
    }

    /// Cancels a job. If it is queued it disappears at once; if it is running,
    /// the progress callback notices and the download stops on its own, deleting
    /// the partial file.
    pub fn cancel(&self, id: u64) {
        if let Some(j) = self.0.jobs.borrow_mut().iter_mut().find(|j| j.id == id) {
            j.cancel.store(true, Ordering::Relaxed);
            // The phase changes immediately, even for a running job: waiting for
            // confirmation from the network thread would mean that, if it never
            // came, the job stayed "running" forever and held a slot in the
            // queue. Any late outcome is ignored.
            j.phase = Phase::Cancelled;
            j.announced = true;
        }
        self.notify_watchers();
        self.pump();
    }

    /// Cancels everything that has not finished yet.
    pub fn cancel_all(&self) {
        let ids: Vec<u64> = self
            .0
            .jobs
            .borrow()
            .iter()
            .filter(|j| j.phase.pending())
            .map(|j| j.id)
            .collect();
        for id in ids {
            self.cancel(id);
        }
    }

    fn set_phase(&self, id: u64, phase: Phase) {
        // A finished job does not go backwards: the outcome of a cancelled
        // download can arrive afterwards, and must not revive it.
        if self
            .0
            .jobs
            .borrow()
            .iter()
            .any(|j| j.id == id && j.phase == Phase::Cancelled)
        {
            return;
        }
        // Byte-level progress arrives with every packet: it is reported to the
        // interface at intervals. Everything else goes straight through, because
        // it is a state change and not a number ticking over.
        let mut notify = true;
        if let Some(j) = self.0.jobs.borrow_mut().iter_mut().find(|j| j.id == id) {
            if matches!(phase, Phase::Downloading(_)) {
                let now = std::time::Instant::now();
                notify = progress_is_worth_showing(j.last_notified, now);
                if notify {
                    j.last_notified = Some(now);
                }
            } else {
                j.last_notified = None;
            }
            j.phase = phase;
        }
        if notify {
            self.notify_watchers();
        }
    }

    pub fn paused(&self) -> Option<Paused> {
        self.0.paused.borrow().clone()
    }

    /// Stops the queue without discarding it: the jobs stay where they are.
    fn pause(&self, why: Paused) {
        *self.0.paused.borrow_mut() = Some(why);
        self.notify_watchers();
    }

    /// Resumes from where it stopped.
    pub fn resume(&self) {
        *self.0.paused.borrow_mut() = None;
        self.notify_watchers();
        self.pump();
    }

    /// Queues a corpus. Returns `false` if it was already there.
    fn enqueue(
        &self,
        path: &[String],
        kind: JobKind,
        dest_root: PathBuf,
        group_root: Option<&[String]>,
        group_total: usize,
    ) -> bool {
        // Identity is (path, kind): a corpus and its recordings share a path,
        // and deduplicating on the path alone would have each wipe the others.
        let same = |j: &Job| j.path == path && j.kind == kind;
        if self.0.jobs.borrow().iter().any(|j| same(j) && j.phase.pending()) {
            return false;
        }
        // An earlier attempt, finished or failed, makes way for the new one.
        self.0.jobs.borrow_mut().retain(|j| !same(j));

        let id = {
            let mut n = self.0.next_id.borrow_mut();
            let id = *n;
            *n += 1;
            id
        };
        self.0.jobs.borrow_mut().push(Job {
            id,
            path: path.to_vec(),
            phase: Phase::Queued,
            kind,
            dest_root,
            cancel: Arc::new(AtomicBool::new(false)),
            announced: false,
            group_root: group_root.map(<[String]>::to_vec),
            group_total,
            last_notified: None,
        });
        true
    }

    /// Queues a single corpus. `with_media` also fetches the recordings, once
    /// the transcripts are on disk and their `@Media` headers can be read.
    pub fn start(&self, path: &[String], dest_root: PathBuf, with_media: bool) {
        if !self.enqueue(path, JobKind::Corpus { with_media }, dest_root, None, 1) {
            self.0.app.toast(&t("This corpus is already downloading."));
            return;
        }
        self.notify_watchers();
        self.pump();
    }

    /// Queues only the recordings of a corpus that is already on disk.
    ///
    /// This is the repair path: someone who downloaded a corpus before turning
    /// the option on should not have to fetch the transcripts again.
    pub fn start_media_only(&self, path: &[String], dest_root: PathBuf) {
        let dest = talkbank_archive::download::destination(&dest_root, path);
        self.scan_and_queue_media(path, dest, dest_root, None, 1, |m, queued| {
            if queued == 0 {
                m.0.app
                    .toast(&t("No media to fetch: this corpus already has all of them."));
            }
        });
    }

    /// Queues a whole branch. The paths come from the plan, so they are already
    /// the minimal set covering it. `root` is the folder the user started from:
    /// it tells us where to open once the work is done.
    ///
    /// `again` re-queues even what is already on disk.
    pub fn start_many(
        &self,
        paths: &[Vec<String>],
        dest_root: PathBuf,
        root: &[String],
        again: bool,
        with_media: bool,
    ) {
        let mut queued = 0;
        // Corpora already on disk whose recordings are being looked for. Their
        // count arrives later, so the "nothing to do" message has to wait for
        // them rather than fire while scans are still running.
        let mut scanning = 0;
        for path in paths {
            // Anything already complete is skipped: these are megabytes, not
            // requests. With media wanted, "complete" is not enough — the
            // transcripts may be there while the recordings are not — so the
            // media are queued straight from what is already on disk.
            if !again && talkbank_archive::download::already_there(&dest_root, path) {
                if with_media {
                    // Its transcripts are here but its recordings may not be.
                    // The scan runs off-thread, so this only starts it.
                    let dest = talkbank_archive::download::destination(&dest_root, path);
                    scanning += 1;
                    self.scan_and_queue_media(
                        path,
                        dest,
                        dest_root.clone(),
                        Some(root),
                        paths.len(),
                        |_, _| {},
                    );
                }
                continue;
            }
            if self.enqueue(
                path,
                JobKind::Corpus { with_media },
                dest_root.clone(),
                Some(root),
                paths.len(),
            ) {
                queued += 1;
            }
        }
        self.notify_watchers();
        self.pump();
        if queued == 0 && scanning == 0 {
            self.0.app.toast(&t("Nothing left to download: it is all already here."));
        }
    }

    /// Reads the `@Media` headers of an extracted corpus and queues one job per
    /// recording. Returns how many were queued.
    ///
    /// This runs after extraction because the header is the only authority on
    /// the filename: it usually matches the transcript's own name, but the CHAT
    /// format does not promise it.
    /// Scans a corpus off the interface thread, then queues what it found on it.
    ///
    /// The scan reads and parses every transcript — 0.4 ms each, which is
    /// nothing for one corpus and twenty-two seconds for a branch the size of
    /// CHILDES. On the main thread that is a frozen window, so it goes to a
    /// worker and comes back.
    fn scan_and_queue_media(
        &self,
        path: &[String],
        dest: PathBuf,
        dest_root: PathBuf,
        group_root: Option<&[String]>,
        group_total: usize,
        then: impl FnOnce(&Manager, usize) + 'static,
    ) {
        let this = self.clone();
        let p = path.to_vec();
        let root = group_root.map(<[String]>::to_vec);
        net().spawn(
            async move { transcripts_with_media(&dest) },
            move |found| {
                let n = this.enqueue_media_from(&p, &dest_root, root.as_deref(), group_total, found);
                this.notify_watchers();
                this.pump();
                then(&this, n);
            },
        );
    }

    /// Queues one job per recording the scan turned up. Returns how many.
    fn enqueue_media_from(
        &self,
        path: &[String],
        dest_root: &std::path::Path,
        group_root: Option<&[String]>,
        group_total: usize,
        found: Vec<(PathBuf, talkbank_engine::chat::MediaRef)>,
    ) -> usize {
        let dest = talkbank_archive::download::destination(dest_root, path);
        let mut queued = 0;
        for (file, media) in found {
            // `missing` means the archive does not hold it: asking would only
            // earn a 404 and a row in the panel saying so.
            if !media.is_fetchable() {
                continue;
            }
            let Some(parent) = file.parent() else { continue };
            // The archive path of the folder the transcript sits in, which is
            // the corpus path plus whatever subfolders the zip created.
            let mut dir = path.to_vec();
            if let Ok(rel) = parent.strip_prefix(&dest) {
                dir.extend(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()));
            }
            let ext = talkbank_archive::download::extensions(media.video)[0];
            let url = talkbank_archive::api::media_url(&dir, &media.basename, ext);
            let target = parent.join(format!("{}.{ext}", media.basename));
            // Already there: nothing to do, and saying so keeps the panel honest.
            if target.is_file() {
                continue;
            }
            if self.enqueue(
                path,
                JobKind::Media { url, dest: target },
                dest_root.to_path_buf(),
                group_root,
                group_total,
            ) {
                queued += 1;
            }
        }
        queued
    }

    /// Starts queued jobs up to the concurrency limit.
    fn pump(&self) {
        if self.0.paused.borrow().is_some() {
            return;
        }
        if self.0.pumping.replace(true) {
            return;
        }
        // The flag lowers itself, even if something in between panics. Lowering
        // it by hand at the end meant that a mishap in one notification closure
        // turned the queue into a no-op for the rest of the session: jobs sat
        // waiting and nobody said so.
        struct Guard<'a>(&'a Cell<bool>);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _guard = Guard(&self.0.pumping);

        loop {
            let active = self
                .0
                .jobs
                .borrow()
                .iter()
                .filter(|j| j.phase.running())
                .count();
            if active >= PARALLEL {
                break;
            }
            // Transcripts jump the queue. A branch of twenty-four corpora
            // queues its first corpus's recordings — hundreds of megabytes —
            // before the twenty-third corpus's transcripts, which are 23 KB
            // each. In insertion order the small, immediately useful files
            // would wait hours behind the large ones.
            let pick = |only_corpora: bool| {
                self.0
                    .jobs
                    .borrow()
                    .iter()
                    .find(|j| {
                        j.phase == Phase::Queued && (!only_corpora || j.kind.is_corpus())
                    })
                    .map(|j| {
                        (
                            j.id,
                            j.path.clone(),
                            j.kind.clone(),
                            j.dest_root.clone(),
                            j.cancel.clone(),
                        )
                    })
            };
            let Some((id, path, kind, dest_root, cancel)) = pick(true).or_else(|| pick(false))
            else {
                break;
            };
            // Mark it started *before* launching: `spawn_with_progress` can
            // finish immediately on an error and re-enter here.
            self.set_phase(id, Phase::Downloading(0));
            self.launch(id, path, kind, dest_root, cancel);
        }
    }

    fn launch(
        &self,
        id: u64,
        path: Vec<String>,
        kind: JobKind,
        dest_root: PathBuf,
        cancel: Arc<AtomicBool>,
    ) {
        if let JobKind::Media { url, dest } = kind {
            self.launch_media(id, url, dest, cancel);
            return;
        }
        let with_media = matches!(kind, JobKind::Corpus { with_media: true });
        let p = path.clone();
        let this = self.clone();
        let done_path = path.clone();
        let root_dir = dest_root.clone();
        let stop = cancel.clone();
        net().spawn_with_progress(
            move |tx| async move {
                talkbank_archive::download::corpus(net().client(), &p, &dest_root, |pr| {
                    // **Only** an explicit cancellation stops a transfer. Tying
                    // continuation to the state of the progress channel meant
                    // that a mishap in the interface — a page closed, a widget
                    // disposed, a panic in an update closure — aborted the
                    // download in silence, without even logging it. Whoever is
                    // listening for progress is free to stop: that is not the
                    // transfer's business.
                    let _ = tx.try_send(pr);
                    !stop.load(Ordering::Relaxed)
                })
                .await
            },
            {
                let this = self.clone();
                move |pr| {
                    let phase = match pr {
                        talkbank_archive::download::Progress::Downloading(b) => Phase::Downloading(b),
                        talkbank_archive::download::Progress::Extracting { done, total } => {
                            Phase::Extracting { done, total }
                        }
                        talkbank_archive::download::Progress::Done => return,
                    };
                    this.set_phase(id, phase);
                }
            },
            move |res| {
                use talkbank_archive::download::DownloadError as E;
                if let Err(e) = &res {
                    // In a queue of twenty-four a failure is invisible: the panel
                    // shows it, but only to whoever opens the panel. At least let
                    // it be written down.
                    tracing::warn!("download failed: {} — {e}", done_path.join("/"));
                }
                match res {
                    Ok(dest) => {
                        // The transcripts are on disk, so their `@Media`
                        // headers can finally be read.
                        if with_media {
                            let (root, total) = this
                                .0
                                .jobs
                                .borrow()
                                .iter()
                                .find(|j| j.id == id)
                                .map(|j| (j.group_root.clone(), j.group_total))
                                .unwrap_or((None, 1));
                            this.scan_and_queue_media(
                                &done_path,
                                dest.clone(),
                                root_dir.clone(),
                                root.as_deref(),
                                total,
                                |_, _| {},
                            );
                        }
                        this.set_phase(id, Phase::Done(dest));
                    }
                    Err(E::Cancelled) => this.set_phase(id, Phase::Cancelled),
                    Err(E::AuthRequired) => {
                        // The session expired: the rest of the queue would fail
                        // the same way. Pause it — do not cancel: after signing
                        // back in the queue picks up where it was, with no
                        // replanning and no re-downloading.
                        this.set_phase(id, Phase::Queued);
                        this.pause(Paused::SignInNeeded);
                        this.0.app.ask_to_sign_in();
                    }
                    Err(E::NoSpace { .. }) => {
                        this.set_phase(id, Phase::Queued);
                        this.pause(Paused::DiskFull);
                    }
                    Err(E::NeedsPermission) => {
                        this.set_phase(id, Phase::Failed(t("This bank needs separate permission")))
                    }
                    Err(e) => this.set_phase(id, Phase::Failed(describe(&e))),
                }
                this.pump();
                this.announce_if_idle();
            },
        );
    }

    /// One recording. No zip and no extraction, so the phases are just
    /// `Downloading` and `Done`.
    fn launch_media(&self, id: u64, url: String, dest: PathBuf, cancel: Arc<AtomicBool>) {
        let this = self.clone();
        let stop = cancel.clone();
        let u = url.clone();
        let d = dest.clone();
        let log_url = url.clone();
        net().spawn_with_progress(
            move |tx| async move {
                talkbank_archive::download::media(net().client(), &u, &d, |pr| {
                    let _ = tx.try_send(pr);
                    !stop.load(Ordering::Relaxed)
                })
                .await
            },
            {
                let this = self.clone();
                move |pr| {
                    if let talkbank_archive::download::Progress::Downloading(b) = pr {
                        this.set_phase(id, Phase::Downloading(b));
                    }
                }
            },
            move |res| {
                use talkbank_archive::download::DownloadError as E;
                match res {
                    Ok(()) => this.set_phase(id, Phase::Done(dest)),
                    Err(E::Cancelled) => this.set_phase(id, Phase::Cancelled),
                    Err(E::AuthRequired) => {
                        this.set_phase(id, Phase::Queued);
                        this.pause(Paused::SignInNeeded);
                        this.0.app.ask_to_sign_in();
                    }
                    Err(E::NoSpace { .. }) => {
                        this.set_phase(id, Phase::Queued);
                        this.pause(Paused::DiskFull);
                    }
                    Err(E::NotAvailable) => {
                        // The header named a recording the archive does not
                        // serve. One row says so; the corpus stays a success.
                        tracing::info!("media not on the server: {log_url}");
                        this.set_phase(id, Phase::Failed(t("Media file not found")));
                    }
                    Err(e) => {
                        tracing::warn!("media download failed: {log_url} — {e}");
                        this.set_phase(id, Phase::Failed(describe(&e)));
                    }
                }
                this.pump();
                this.announce_if_idle();
            },
        );
    }

    /// Reports the outcome once the queue empties: once per group, not once per
    /// corpus.
    fn announce_if_idle(&self) {
        if self.0.jobs.borrow().iter().any(|j| j.phase.pending()) {
            return;
        }
        let outcome = {
            let mut jobs = self.0.jobs.borrow_mut();
            let mut e = Outcome::default();
            for j in jobs.iter_mut().filter(|j| !j.announced) {
                j.announced = true;
                // The expected total comes from the group: it is what allows
                // "23 of 24" instead of "23", which is the difference between
                // noticing a loss and not noticing it.
                if j.kind.is_corpus() {
                    e.expected = e.expected.max(j.group_total);
                }
                match &j.phase {
                    Phase::Done(dest) if !j.kind.is_corpus() => {
                        e.media += 1;
                        // A recording alone should still offer a way in.
                        e.where_to.get_or_insert_with(|| match &j.group_root {
                            Some(root) => {
                                talkbank_archive::download::destination(&j.dest_root, root)
                            }
                            None => dest.parent().map(Into::into).unwrap_or_else(|| dest.clone()),
                        });
                    }
                    Phase::Done(dest) => {
                        e.done += 1;
                        e.name = j.title();
                        // With a group we open the root of the branch, not the
                        // folder of whichever corpus happened to finish last: in
                        // a deep branch that one does not contain the others.
                        e.where_to = Some(match &j.group_root {
                            Some(root) => {
                                talkbank_archive::download::destination(&j.dest_root, root)
                            }
                            None => dest.clone(),
                        });
                    }
                    Phase::Failed(_) => e.failed += 1,
                    Phase::Cancelled => e.cancelled += 1,
                    _ => {}
                }
            }
            e
        };
        if outcome.done == 0 && outcome.failed == 0 && outcome.cancelled == 0 && outcome.media == 0
        {
            return;
        }
        self.notify_watchers();
        self.announce(outcome);
    }

    /// Announces that the queue has drained. If the archive is already on screen
    /// a toast at the bottom of the window is enough; otherwise a system
    /// notification is needed, because the app may be behind another window.
    fn announce(&self, e: Outcome) {
        let app = &self.0.app;
        let headline = e.headline();
        // A shortfall goes into the log as well: anyone who was not at the screen
        // when the toast appeared would otherwise never know.
        if e.incomplete() {
            tracing::warn!(
                "queue finished incomplete: {} expected, {} arrived, {} failed, {} cancelled",
                e.expected, e.done, e.failed, e.cancelled
            );
        }

        if app.window().is_active() && app.visible_section() == "archive" {
            let toast = adw::Toast::new(&headline);
            // A shortfall stays on screen until dismissed: vanishing after eight
            // seconds is as good as never having said it.
            toast.set_timeout(if e.incomplete() { 0 } else { 8 });
            if let Some(target) = e.where_to.clone() {
                toast.set_button_label(Some(&t("Open")));
                let a = app.clone();
                toast.connect_button_clicked(move |_| a.open_downloaded(&target));
            }
            app.show_toast(toast);
            return;
        }

        let notification = gio::Notification::new(&headline);
        if let Some(target) = e.where_to {
            notification.set_body(Some(
                &t("Ready in %p").replace("%p", &target.display().to_string()),
            ));
            // Activating the notification opens the downloaded folder: it is the
            // one thing anyone wants to do right afterwards.
            notification.set_default_action_and_target_value(
                "app.open-downloaded",
                Some(&target.display().to_string().to_variant()),
            );
        }
        if let Some(gapp) = app.window().application() {
            gapp.send_notification(Some("download-finished"), &notification);
        }
    }

    // ------------------------------------------------------------- interface

    /// The button for the header bars: it shows how many downloads are running
    /// and opens the panel.
    pub fn button(&self) -> gtk::Widget {
        let btn = gtk::MenuButton::new();
        btn.set_tooltip_text(Some(&t("Downloads")));

        let content = adw::ButtonContent::new();
        content.set_icon_name("folder-download-symbolic");
        btn.set_child(Some(&content));

        let popover = gtk::Popover::new();
        popover.set_width_request(400);
        btn.set_popover(Some(&popover));

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");

        // The pause row sits at the top: it explains why nothing is happening,
        // and has to be read before the forty rows below it.
        let paused_row = adw::ActionRow::new();
        paused_row.add_prefix(&gtk::Image::from_icon_name("media-playback-pause-symbolic"));
        let resume = gtk::Button::with_label(&t("Resume"));
        resume.set_valign(gtk::Align::Center);
        resume.add_css_class("suggested-action");
        let this = self.clone();
        resume.connect_clicked(move |_| this.resume());
        paused_row.add_suffix(&resume);
        paused_row.set_visible(false);

        let empty = gtk::Label::new(Some(&t("No downloads yet")));
        empty.add_css_class("dim-label");
        empty.set_margin_top(18);
        empty.set_margin_bottom(18);

        let clear = gtk::Button::with_label(&t("Clear finished"));
        clear.add_css_class("flat");
        let this = self.clone();
        clear.connect_clicked(move |_| this.clear_finished());

        // With a branch queued there has to be a way to stop everything at once:
        // one at a time, across forty, is not a way.
        let stop_all = gtk::Button::with_label(&t("Cancel all"));
        stop_all.add_css_class("flat");
        stop_all.add_css_class("destructive-action");
        let this = self.clone();
        stop_all.connect_clicked(move |_| this.cancel_all());

        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
        box_.set_margin_start(8);
        box_.set_margin_end(8);
        box_.set_margin_top(8);
        box_.set_margin_bottom(8);
        box_.append(&paused_row);
        box_.append(&empty);
        // A branch can queue dozens of corpora: the list scrolls instead of
        // growing the panel past the edge of the screen.
        let sw = gtk::ScrolledWindow::builder()
            .child(&list)
            .propagate_natural_height(true)
            .max_content_height(420)
            .build();
        box_.append(&sw);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        actions.append(&stop_all);
        actions.append(&clear);
        box_.append(&actions);
        popover.set_child(Some(&box_));

        let this = self.clone();
        let btn2 = btn.clone();
        let pop = popover.clone();
        let refresh = std::rc::Rc::new(move || {
            let jobs = this.jobs();

            // The badge and the button's own visibility are cheap and always
            // wanted; the rows are neither.
            let pending = jobs.iter().filter(|j| j.phase.pending()).count();
            content.set_label(&if pending == 0 { String::new() } else { pending.to_string() });
            btn2.set_visible(!jobs.is_empty());

            // Rebuilding the list means tearing down and re-creating an
            // AdwActionRow — with a spinner, buttons and their handlers — for
            // every job. Doing that for a popover nobody has open, several
            // times a second, for every archive page still on the navigation
            // stack, is what made a media download lock the interface up.
            if !pop.is_visible() {
                return;
            }
            match this.paused() {
                Some(Paused::SignInNeeded) => {
                    paused_row.set_title(&t("Paused — sign in to continue"));
                    paused_row.set_visible(true);
                }
                Some(Paused::DiskFull) => {
                    paused_row.set_title(&t("Paused — the download folder is full"));
                    paused_row.set_visible(true);
                }
                None => paused_row.set_visible(false),
            }
            while let Some(r) = list.first_child() {
                list.remove(&r);
            }
            empty.set_visible(jobs.is_empty());
            sw.set_visible(!jobs.is_empty());
            clear.set_visible(jobs.iter().any(|j| !j.phase.pending()));
            stop_all.set_visible(jobs.iter().filter(|j| j.phase.pending()).count() > 1);

            for job in &jobs {
                let row = adw::ActionRow::new();
                row.set_title(&job.title());
                row.set_subtitle(&format!("{} · {}", job.where_from(), job.status()));
                match &job.phase {
                    Phase::Done(dest) => {
                        row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
                        let open = gtk::Button::with_label(&t("Open"));
                        open.set_valign(gtk::Align::Center);
                        let a = this.0.app.clone();
                        let d = dest.clone();
                        open.connect_clicked(move |_| a.open_downloaded(&d));
                        row.add_suffix(&open);
                    }
                    Phase::Failed(_) => {
                        row.add_prefix(&gtk::Image::from_icon_name("dialog-error-symbolic"));
                    }
                    Phase::Cancelled => {
                        row.add_prefix(&gtk::Image::from_icon_name("process-stop-symbolic"));
                    }
                    Phase::Queued => {
                        row.add_prefix(&gtk::Image::from_icon_name("content-loading-symbolic"));
                    }
                    _ => {
                        let sp = adw::Spinner::new();
                        sp.set_valign(gtk::Align::Center);
                        row.add_prefix(&sp);
                    }
                }
                if job.phase.pending() {
                    let stop = gtk::Button::from_icon_name("window-close-symbolic");
                    stop.set_valign(gtk::Align::Center);
                    stop.add_css_class("flat");
                    stop.set_tooltip_text(Some(&t("Cancel this download")));
                    let m = this.clone();
                    let id = job.id;
                    stop.connect_clicked(move |_| m.cancel(id));
                    row.add_suffix(&stop);
                }
                list.append(&row);
            }

        });
        refresh();
        // Opening the popover is when the rows have to be right, because while
        // it was closed they were not being kept up to date.
        let r2 = refresh.clone();
        popover.connect_show(move |_| r2());
        self.watch_for(&btn, move || refresh());

        btn.upcast()
    }

    /// The strip to put at the top of a page while a download is running on
    /// *that* path: whoever is looking at the corpus wants to see it there.
    pub fn inline(&self, path: &[String]) -> gtk::Widget {
        let bar = gtk::ProgressBar::new();
        bar.set_show_text(true);
        bar.set_valign(gtk::Align::Center);

        let this = self.clone();
        let p = path.to_vec();
        let b = bar.clone();
        let refresh = move || match this.job_for(&p) {
            Some(job) if job.phase.pending() => {
                b.set_visible(true);
                match job.phase {
                    Phase::Queued => b.set_fraction(0.0),
                    Phase::Extracting { done, total } => {
                        b.set_fraction(done as f64 / total.max(1) as f64)
                    }
                    // With no total there is no percentage: the bar pulses rather
                    // than inventing progress.
                    _ => b.pulse(),
                }
                b.set_text(Some(&job.status()));
            }
            _ => b.set_visible(false),
        };
        refresh();
        self.watch_for(&bar, refresh);
        bar.upcast()
    }
}

/// How a queue went, so it can be said in one sentence.
#[derive(Debug, Default, PartialEq, Eq)]
struct Outcome {
    /// How many corpora were queued together.
    expected: usize,
    done: usize,
    failed: usize,
    cancelled: usize,
    /// Recordings that arrived. Counted apart from the corpora: they are a
    /// different unit, and "24 corpora" would be a lie if it meant 24 mp3s.
    media: usize,
    /// The name of the last one finished, for the singular phrasing.
    name: String,
    /// The folder to open.
    where_to: Option<PathBuf>,
}

impl Outcome {
    /// True when not everything asked for arrived.
    fn incomplete(&self) -> bool {
        self.failed > 0 || self.cancelled > 0 || (self.expected > 1 && self.done < self.expected)
    }

    fn headline(&self) -> String {
        // Recordings with no corpus beside them: the repair path, where someone
        // fetched the media of a corpus that was already on disk.
        if self.done == 0 && self.media > 0 && self.failed == 0 && self.cancelled == 0 {
            return tn("%u media file downloaded.", "%u media files downloaded.", self.media as u32)
                .replace("%u", &self.media.to_string());
        }
        // A single corpus is called by name; a group is counted, and if anything
        // is missing the count says so.
        if self.expected <= 1 && self.failed == 0 && self.cancelled == 0 && self.done == 1 {
            let corpus = t("%s downloaded.").replace("%s", &self.name);
            if self.media == 0 {
                return corpus;
            }
            return format!(
                "{corpus} {}",
                tn("%u media file too.", "%u media files too.", self.media as u32)
                    .replace("%u", &self.media.to_string())
            );
        }
        if self.done == 0 {
            return t("Download failed");
        }
        if !self.incomplete() {
            let corpora = tn("%u corpus downloaded.", "%u corpora downloaded.", self.done as u32)
                .replace("%u", &self.done.to_string());
            if self.media == 0 {
                return corpora;
            }
            return format!(
                "{corpora} {}",
                tn("%u media file too.", "%u media files too.", self.media as u32)
                    .replace("%u", &self.media.to_string())
            );
        }
        t("%d of %u corpora downloaded.")
            .replace("%d", &self.done.to_string())
            .replace("%u", &self.expected.max(self.done).to_string())
    }
}

/// Every transcript under `dir` that declares a recording, with the declaration.
///
/// Recursive: a corpus zip keeps its own subfolders (`Brown/Adam/adam01.cha`),
/// and the recording sits next to its transcript, not at the corpus root.
fn transcripts_with_media(dir: &std::path::Path) -> Vec<(PathBuf, talkbank_engine::chat::MediaRef)> {
    let mut out = Vec::new();
    collect_media(dir, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn collect_media(dir: &std::path::Path, out: &mut Vec<(PathBuf, talkbank_engine::chat::MediaRef)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_media(&path, out);
        } else if path.extension().is_some_and(|e| e == "cha") {
            if let Some(media) = talkbank_engine::chat::inspect(&path).media {
                out.push((path, media));
            }
        }
    }
}

/// The error message, translated.
///
/// `DownloadError`'s own `Display` is developer-facing English that never passes
/// through gettext; showing it as-is would put an untranslated sentence in a
/// translated interface.
pub fn describe(e: &talkbank_archive::download::DownloadError) -> String {
    use talkbank_archive::download::DownloadError as E;
    match e {
        E::AuthRequired => t("Sign in to download"),
        E::NotAvailable => t("This folder is not a downloadable corpus"),
        E::NeedsPermission => t("This bank needs separate permission"),
        E::Cancelled => t("Cancelled"),
        E::BadArchive(_) => t("The downloaded file is damaged"),
        E::NoSpace { .. } => t("Not enough free space"),
        E::Io(_) => t("Could not write to disk"),
        E::Api(_) => t("Network error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(path: &[&str], phase: Phase) -> Job {
        kinded_job(path, phase, JobKind::Corpus { with_media: false })
    }

    fn media_job(path: &[&str], name: &str, phase: Phase) -> Job {
        kinded_job(
            path,
            phase,
            JobKind::Media {
                url: format!("https://media.talkbank.org/{name}"),
                dest: PathBuf::from("/data").join(name),
            },
        )
    }

    fn kinded_job(path: &[&str], phase: Phase, kind: JobKind) -> Job {
        Job {
            id: 1,
            path: path.iter().map(|s| s.to_string()).collect(),
            phase,
            kind,
            dest_root: PathBuf::from("/data"),
            cancel: Arc::new(AtomicBool::new(false)),
            announced: false,
            group_root: None,
            group_total: 1,
            last_notified: None,
        }
    }

    #[test]
    fn the_title_is_the_folder_and_the_origin_is_the_rest() {
        let j = job(&["childes", "Eng-NA", "Brown"], Phase::Downloading(0));
        assert_eq!(j.title(), "Brown");
        assert_eq!(j.where_from(), "CHILDES · Eng-NA");

        // bank with no collection level: only the bank is left
        let j = job(&["ca", "ATC"], Phase::Downloading(0));
        assert_eq!(j.title(), "ATC");
        assert_eq!(j.where_from(), "CABank");
    }

    #[test]
    fn only_the_intermediate_phases_count_as_running() {
        assert!(Phase::Downloading(0).running());
        assert!(Phase::Extracting { done: 1, total: 2 }.running());
        assert!(!Phase::Done(PathBuf::from("/x")).running());
        assert!(!Phase::Failed("no".into()).running());
        // Queued is not "running" — no connection is open — but it is "pending":
        // the badge and the cancel button look at that.
        assert!(!Phase::Queued.running());
        assert!(Phase::Queued.pending());
        assert!(!Phase::Cancelled.pending());
        assert!(!Phase::Done(PathBuf::from("/x")).pending());
    }

    #[test]
    fn a_shortfall_in_the_queue_is_declared() {
        // The case this code was written for: 24 asked for, 23 arrived, and
        // nobody saying so.
        let e = Outcome { expected: 24, done: 23, failed: 1, ..Default::default() };
        assert!(e.incomplete());
        assert!(e.headline().contains("23"), "{}", e.headline());
        assert!(e.headline().contains("24"), "{}", e.headline());

        // And a loss with no recorded error — the worst case, because it leaves
        // no trace anywhere else.
        let silent = Outcome { expected: 24, done: 23, ..Default::default() };
        assert!(silent.incomplete(), "a silent loss still has to be reported");

        // A complete group raises no alarm.
        let full = Outcome { expected: 24, done: 24, ..Default::default() };
        assert!(!full.incomplete());
        assert!(full.headline().contains("24"));

        // A single corpus is called by name.
        let one = Outcome { expected: 1, done: 1, name: "Brown".into(), ..Default::default() };
        assert!(!one.incomplete());
        assert!(one.headline().contains("Brown"), "{}", one.headline());

        // Cancelling counts as a shortfall: the queue did not do what was asked,
        // and that has to be said even when it was a decision.
        let stopped = Outcome { expected: 5, done: 2, cancelled: 3, ..Default::default() };
        assert!(stopped.incomplete());
    }

    #[test]
    fn byte_progress_is_throttled_by_time_alone() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();

        // The first tick always shows: there is nothing on screen yet.
        assert!(progress_is_worth_showing(None, t0));

        // Within the interval, nothing — however many bytes moved. The old rule
        // also fired every megabyte, so a fast 400 MB transfer rebuilt the whole
        // panel dozens of times a second and the interface stopped responding.
        assert!(!progress_is_worth_showing(Some(t0), t0 + Duration::from_millis(1)));
        assert!(!progress_is_worth_showing(Some(t0), t0 + Duration::from_millis(249)));

        // At the interval, yes.
        assert!(progress_is_worth_showing(Some(t0), t0 + PROGRESS_INTERVAL));
        assert!(progress_is_worth_showing(Some(t0), t0 + Duration::from_secs(1)));

        // Four a second is the ceiling, so a transfer of any speed costs the
        // same to draw.
        let ticks = (0..1000)
            .filter(|i| progress_is_worth_showing(Some(t0), t0 + Duration::from_millis(*i)))
            .count();
        assert!(ticks <= 751, "one second of ticks should cap out, got {ticks}");
    }

    #[test]
    fn a_media_job_is_named_by_its_file_and_placed_under_its_corpus() {
        let j = media_job(&["childes", "Eng-NA", "Brown"], "adam01.mp3", Phase::Queued);
        assert_eq!(j.title(), "adam01.mp3");
        // The corpus keeps its name in the title, so a recording has to show it
        // in the origin instead — otherwise the panel never says which corpus.
        assert_eq!(j.where_from(), "CHILDES · Eng-NA / Brown");

        let c = job(&["childes", "Eng-NA", "Brown"], Phase::Queued);
        assert_eq!(c.title(), "Brown");
        assert_eq!(c.where_from(), "CHILDES · Eng-NA");
    }

    #[test]
    fn a_queue_of_recordings_is_counted_apart_from_the_corpora() {
        // "24 corpora" would be a lie if it meant 24 mp3s.
        let one = Outcome { expected: 1, done: 1, name: "Brown".into(), media: 27, ..Default::default() };
        let h = one.headline();
        assert!(h.contains("Brown"), "{h}");
        assert!(h.contains("27"), "{h}");
        assert!(!one.incomplete());

        // The repair path: recordings alone, no corpus beside them.
        let repair = Outcome { media: 27, ..Default::default() };
        assert!(repair.headline().contains("27"), "{}", repair.headline());

        // And a corpus with no media reads exactly as it did before.
        let plain = Outcome { expected: 1, done: 1, name: "Brown".into(), ..Default::default() };
        assert_eq!(plain.headline(), t("%s downloaded.").replace("%s", "Brown"));
    }

    #[test]
    fn download_errors_go_through_the_translations() {
        use talkbank_archive::download::DownloadError as E;
        // No message may come straight from the library's own Display.
        for e in [
            E::AuthRequired,
            E::NotAvailable,
            E::NeedsPermission,
            E::Cancelled,
            E::NoSpace { needed: 0 },
        ] {
            let d = describe(&e);
            assert!(!d.is_empty());
            assert_ne!(d, e.to_string(), "{e:?} shows the untranslated text");
        }
    }
}
