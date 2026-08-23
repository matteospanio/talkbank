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

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    /// Path in the archive, bank included.
    pub path: Vec<String>,
    pub phase: Phase,
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
    /// When, and at what point, the interface was last notified. Without this a
    /// panel of forty rows would rebuild on every network packet: it flickers,
    /// and it makes the cancel button hard to hit.
    last_notified: Option<(std::time::Instant, u64)>,
}

/// Why the queue is paused. These are the two cases where carrying on would fail
/// everything else the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paused {
    SignInNeeded,
    DiskFull,
}

impl Job {
    /// The name to show: the folder, with its origin above it.
    pub fn title(&self) -> String {
        self.path.last().cloned().unwrap_or_default()
    }
    pub fn where_from(&self) -> String {
        let bank = self
            .path
            .first()
            .map(|b| talkbank_archive::catalog::bank_title(b).to_string())
            .unwrap_or_default();
        let rest = self.path[1..self.path.len().saturating_sub(1)].join(" / ");
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

    /// The download for this path, if there is one.
    pub fn job_for(&self, path: &[String]) -> Option<Job> {
        self.0.jobs.borrow().iter().find(|j| j.path == path).cloned()
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
            if let Phase::Downloading(bytes) = phase {
                let now = std::time::Instant::now();
                notify = match j.last_notified {
                    Some((when, how_many)) => {
                        now.duration_since(when) >= std::time::Duration::from_millis(250)
                            || bytes.saturating_sub(how_many) >= 1_048_576
                    }
                    None => true,
                };
                if notify {
                    j.last_notified = Some((now, bytes));
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
        dest_root: PathBuf,
        group_root: Option<&[String]>,
        group_total: usize,
    ) -> bool {
        if self.job_for(path).is_some_and(|j| j.phase.pending()) {
            return false;
        }
        // An earlier attempt, finished or failed, makes way for the new one.
        self.0.jobs.borrow_mut().retain(|j| j.path != path);

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
            dest_root,
            cancel: Arc::new(AtomicBool::new(false)),
            announced: false,
            group_root: group_root.map(<[String]>::to_vec),
            group_total,
            last_notified: None,
        });
        true
    }

    /// Queues a single corpus.
    pub fn start(&self, path: &[String], dest_root: PathBuf) {
        if !self.enqueue(path, dest_root, None, 1) {
            self.0.app.toast(&t("This corpus is already downloading."));
            return;
        }
        self.notify_watchers();
        self.pump();
    }

    /// Queues a whole branch. The paths come from the plan, so they are already
    /// the minimal set covering it. `root` is the folder the user started from:
    /// it tells us where to open once the work is done.
    ///
    /// `again` re-queues even what is already on disk.
    pub fn start_many(&self, paths: &[Vec<String>], dest_root: PathBuf, root: &[String], again: bool) {
        let mut queued = 0;
        for path in paths {
            // Anything already complete is skipped: these are megabytes, not
            // requests.
            if !again && talkbank_archive::download::already_there(&dest_root, path) {
                continue;
            }
            if self.enqueue(path, dest_root.clone(), Some(root), paths.len()) {
                queued += 1;
            }
        }
        self.notify_watchers();
        self.pump();
        if queued == 0 {
            self.0.app.toast(&t("Nothing left to download: it is all already here."));
        }
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
            let next = self
                .0
                .jobs
                .borrow()
                .iter()
                .find(|j| j.phase == Phase::Queued)
                .map(|j| (j.id, j.path.clone(), j.dest_root.clone(), j.cancel.clone()));
            let Some((id, path, dest_root, cancel)) = next else {
                break;
            };
            // Mark it started *before* launching: `spawn_with_progress` can
            // finish immediately on an error and re-enter here.
            self.set_phase(id, Phase::Downloading(0));
            self.launch(id, path, dest_root, cancel);
        }
    }

    fn launch(&self, id: u64, path: Vec<String>, dest_root: PathBuf, cancel: Arc<AtomicBool>) {
        let p = path.clone();
        let this = self.clone();
        let done_path = path.clone();
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
                    Ok(dest) => this.set_phase(id, Phase::Done(dest)),
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
                e.expected = e.expected.max(j.group_total);
                match &j.phase {
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
        if outcome.done == 0 && outcome.failed == 0 && outcome.cancelled == 0 {
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
        let refresh = move || {
            let jobs = this.jobs();
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

            // The badge on the button: only there while something is pending.
            let n = jobs.iter().filter(|j| j.phase.pending()).count();
            content.set_label(&if n == 0 { String::new() } else { n.to_string() });
        };
        refresh();
        self.watch_for(&btn, refresh);

        // The button only appears when there is something to show: in an archive
        // nobody has used yet it would be a meaningless icon.
        let this = self.clone();
        let b = btn.clone();
        let show = move || b.set_visible(!this.jobs().is_empty());
        show();
        self.watch_for(&btn, show);

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
        // A single corpus is called by name; a group is counted, and if anything
        // is missing the count says so.
        if self.expected <= 1 && self.failed == 0 && self.cancelled == 0 && self.done == 1 {
            return t("%s downloaded.").replace("%s", &self.name);
        }
        if self.done == 0 {
            return t("Download failed");
        }
        if !self.incomplete() {
            return tn("%u corpus downloaded.", "%u corpora downloaded.", self.done as u32)
                .replace("%u", &self.done.to_string());
        }
        t("%d of %u corpora downloaded.")
            .replace("%d", &self.done.to_string())
            .replace("%u", &self.expected.max(self.done).to_string())
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
        Job {
            id: 1,
            path: path.iter().map(|s| s.to_string()).collect(),
            phase,
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
