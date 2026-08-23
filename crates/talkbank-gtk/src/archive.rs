//! The TalkBank "Archive" section: browse, filter, download.
//!
//! It lives inside the main window rather than a separate one: browsing the
//! archive and working on the files are two moments of the same task, and
//! splitting them across two windows meant tiling them by hand. A download keeps
//! going while you change section.
//!
//! The catalogue is public, the downloads are not. So the whole of the browsing
//! works without an account, and signing in is only asked for at download time.
//!
//! **There is no fixed hierarchy.** In CHILDES and PhonBank a collection sits
//! above the corpus; in CABank or ClassBank the corpus sits directly under the
//! bank, with loose transcripts in between. So browsing is an arbitrary tree of
//! folders, and *which* folder is a downloadable corpus is decided by the server
//! through a HEAD request, not by a rule that would be wrong in five banks out
//! of fifteen.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use talkbank_archive::api::{self, Downloadable};
use talkbank_archive::catalog::{bank_title, Archive, Folder};
use talkbank_archive::cache;

use crate::i18n::{t, tn};
use crate::net::net;
use crate::window::App;

pub struct Inner {
    /// The main window: used for the clipboard and as the dialogs' parent, not
    /// because the archive has a window of its own.
    win: adw::ApplicationWindow,
    parent: App,
    toasts: adw::ToastOverlay,
    nav: adw::NavigationView,
    banner: adw::Banner,
    archive: RefCell<Option<Archive>>,
    /// The bank being browsed. Search, filters and index are all relative to it.
    bank: RefCell<String>,
    index: RefCell<Option<talkbank_archive::index::Index>>,
    filter: RefCell<talkbank_archive::index::Filter>,
    /// The results group and the rows we put in it: search and filters share the
    /// same view, and therefore the same clean-up.
    results: RefCell<Option<adw::PreferencesGroup>>,
    result_rows: RefCell<Vec<adw::ActionRow>>,
    browse_group: RefCell<Option<adw::PreferencesGroup>>,
    search_text: RefCell<String>,
    logged_in: Cell<bool>,
    /// The "include audio and video" row of the corpus page on screen, so the
    /// metadata request that page already makes can fill in the estimate
    /// instead of a second one being fired for it.
    media_row: RefCell<Option<adw::SwitchRow>>,
    /// What that row is set to, read when Download is pressed.
    media_wanted: Rc<Cell<bool>>,
}

#[derive(Clone)]
pub struct ArchiveWindow(Rc<Inner>);

impl std::ops::Deref for ArchiveWindow {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// Builds the section. The catalogue loads on its own: by the time you get here
/// the list is there, or on its way.
///
/// It also returns the section itself, because the window needs to be able to
/// talk to it — for instance to have it re-check the session when you come back.
pub fn page(parent: &App) -> (gtk::Widget, ArchiveWindow) {
    let this = ArchiveWindow(Rc::new(Inner {
        win: parent.window().clone(),
        parent: parent.clone(),
        toasts: adw::ToastOverlay::new(),
        nav: adw::NavigationView::new(),
        banner: adw::Banner::new(""),
        archive: RefCell::new(None),
        bank: RefCell::new(String::new()),
        index: RefCell::new(None),
        filter: RefCell::new(Default::default()),
        results: RefCell::new(None),
        result_rows: RefCell::new(Vec::new()),
        browse_group: RefCell::new(None),
        search_text: RefCell::new(String::new()),
        logged_in: Cell::new(false),
        media_row: RefCell::new(None),
        media_wanted: Rc::new(Cell::new(false)),
    }));

    let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
    this.banner.set_revealed(false);
    vb.append(&this.banner);
    this.nav.set_vexpand(true);
    vb.append(&this.nav);
    this.toasts.set_child(Some(&vb));

    this.show_loading();
    this.load_catalogue();
    this.check_login();

    // The section keeps itself alive for as long as the widget exists: without
    // this the only reference would be the closures', and it would vanish on the
    // first page change.
    let holder = this.clone();
    unsafe { this.toasts.set_data("archive-section", holder) };
    (this.toasts.clone().upcast(), this.clone())
}

impl ArchiveWindow {
    // ----------------------------------------------------------- loading

    fn show_loading(&self) {
        let sp = adw::StatusPage::new();
        sp.set_title(&t("Loading the archive catalogue…"));
        sp.set_description(Some(&t(
            "The list of corpora is public: no account is needed to browse it.",
        )));
        sp.set_child(Some(&adw::Spinner::new()));
        self.set_root_page(&t("TalkBank"), &sp.upcast());
    }

    /// Shows the cache straight away if there is one, and refreshes in the
    /// background when it is old.
    fn load_catalogue(&self) {
        let path = cache::tree_path();
        let fresh = cache::freshness(&path);

        if fresh != cache::Freshness::Absent {
            if let Some(v) = cache::load(&path) {
                if let Ok(a) = talkbank_archive::catalog::parse(&v) {
                    self.set_archive(a);
                    if let Some(when) = cache::updated_at(&path) {
                        self.note_cache_age(when);
                    }
                }
            }
        }
        if fresh == cache::Freshness::Fresh && self.archive.borrow().is_some() {
            return;
        }

        let this = self.clone();
        net().spawn(async { net().client().tree().await }, move |res| match res {
            Ok(value) => {
                if let Ok(bytes) = serde_json::to_vec(&value) {
                    let _ = cache::store(&cache::tree_path(), &bytes);
                }
                match talkbank_archive::catalog::parse(&value) {
                    Ok(a) => {
                        this.banner.set_revealed(false);
                        this.set_archive(a);
                    }
                    Err(e) => this.show_failure(&e.to_string()),
                }
            }
            Err(e) => {
                if this.archive.borrow().is_some() {
                    // The cache is already on screen: just log it, without
                    // wrecking a working page.
                    tracing::info!("catalogue refresh failed: {e}");
                } else {
                    this.show_failure(&e.to_string());
                }
            }
        });
    }

    fn note_cache_age(&self, when: std::time::SystemTime) {
        let secs = std::time::SystemTime::now()
            .duration_since(when)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if secs < cache::MAX_AGE.as_secs() {
            return;
        }
        let days = secs / 86_400;
        self.banner.set_title(
            &t("Showing the list saved %d days ago; updating in the background.")
                .replace("%d", &days.to_string()),
        );
        self.banner.set_revealed(true);
    }

    fn show_failure(&self, detail: &str) {
        let sp = adw::StatusPage::new();
        sp.set_icon_name(Some("network-offline-symbolic"));
        sp.set_title(&t("Could not reach the archive"));
        sp.set_description(Some(&format!(
            "{}\n\n{detail}",
            t("Check your internet connection. The catalogue will be saved once downloaded, so this only needs to work the first time.")
        )));
        let retry = gtk::Button::with_label(&t("Try again"));
        retry.add_css_class("suggested-action");
        retry.add_css_class("pill");
        retry.set_halign(gtk::Align::Center);
        let this = self.clone();
        retry.connect_clicked(move |_| {
            this.show_loading();
            this.load_catalogue();
        });
        sp.set_child(Some(&retry));
        self.set_root_page(&t("TalkBank"), &sp.upcast());
    }

    fn set_archive(&self, a: Archive) {
        let first = self.archive.borrow().is_none();
        *self.archive.borrow_mut() = Some(a);
        // `show_banks` replaces the whole navigation stack. Doing that when the
        // catalogue refresh lands in the background — which can be minutes later
        // — would throw the user back to the list of banks from whatever page
        // they were on, and would destroy pages out from under requests still in
        // flight. So we only rebuild the first time, while the loading screen is
        // still what is on screen.
        if first || self.nav.navigation_stack().n_items() <= 1 {
            self.show_banks();
        }
    }

    /// Opens the session, reusing the saved credentials.
    ///
    /// The TalkBank cookie lives in the HTTP client, which is created with the
    /// app: without this step you would have to press "Test the connection" in
    /// the preferences on every start, and the keyring would be of little use.
    fn check_login(&self) {
        let this = self.clone();
        let saved = crate::config::with(|c| c.email.clone())
            .filter(|e| !e.is_empty())
            .and_then(|e| crate::net::load_password(&e).map(|p| (e, p)));
        net().spawn(
            async move {
                if net().client().is_logged_in().await.unwrap_or(false) {
                    return true;
                }
                match saved {
                    Some((email, pass)) => matches!(
                        net().client().login(&email, &pass).await,
                        Ok(api::LoginOutcome::Success)
                    ),
                    None => false,
                }
            },
            move |ok| this.logged_in.set(ok),
        );
    }

    /// Re-checks the sign-in state.
    ///
    /// The local copy goes stale: someone who signs in from the preferences
    /// after opening the archive would otherwise be told "you need to sign in"
    /// when that is no longer true. Called when returning to the archive and
    /// after a successful sign-in.
    pub fn recheck_login(&self) {
        self.check_login();
    }

    // -------------------------------------------------------------- pages

    /// Builds a navigation page.
    ///
    /// `root` separates the screens that *replace* the root (loading, error, the
    /// list of banks) from those that *stack* on top. Inferring that from the
    /// stack size does not work: after a replacement the stack is back to one
    /// item, and every following page would destroy the previous one together
    /// with its "back" button.
    fn make_page(&self, title: &str, child: &gtk::Widget) -> adw::NavigationPage {
        let head = adw::HeaderBar::new();
        // Downloads are visible from every archive page: whoever started one
        // should not have to go back to find out how it is doing.
        head.pack_end(&self.parent.menu_button());
        head.pack_end(&self.parent.downloads().button());
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&head);
        tv.set_content(Some(child));
        adw::NavigationPage::new(&tv, title)
    }

    fn set_root_page(&self, title: &str, child: &gtk::Widget) {
        self.nav.replace(&[self.make_page(title, child)]);
    }

    fn push_page(&self, title: &str, child: &gtk::Widget) {
        self.nav.push(&self.make_page(title, child));
    }

    // -------------------------------------------------------------- banks

    /// The root: the fifteen TalkBank banks.
    fn show_banks(&self) {
        let Some(archive) = self.archive.borrow().clone() else { return };

        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::new();
        group.set_title(&t("TalkBank collections"));
        group.set_description(Some(&t(
            "Each bank gathers the transcripts of one research area. CHILDES is child language; \
             PhonBank is phonological development; the others cover aphasia, dementia, \
             bilingualism, classrooms and more.",
        )));

        for bank in &archive.banks {
            let corpora = archive.search(&bank.name, "").len();
            let row = adw::ActionRow::new();
            row.set_title(bank_title(&bank.name));
            row.set_subtitle(&format!(
                "{} · {}",
                tn("%u folder", "%u folders", corpora as u32).replace("%u", &corpora.to_string()),
                tn("%u transcript", "%u transcripts", bank.transcripts as u32)
                    .replace("%u", &bank.transcripts.to_string())
            ));
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row.set_activatable(true);
            let this = self.clone();
            let name = bank.name.clone();
            row.connect_activated(move |_| this.open_bank(&name));
            group.add(&row);
        }
        page.add(&group);
        self.set_root_page(&t("TalkBank"), &page.upcast());
    }

    /// Enters a bank: clears search and filters, and loads the index for *that*
    /// bank — CHILDES's index says nothing about PhonBank.
    fn open_bank(&self, bank: &str) {
        *self.bank.borrow_mut() = bank.to_string();
        *self.index.borrow_mut() = talkbank_archive::index::load(bank);
        *self.filter.borrow_mut() = Default::default();
        self.search_text.borrow_mut().clear();
        self.push_page(bank_title(bank), &self.bank_page().upcast());
    }

    /// A bank's page: search across the whole bank, filters, and the list of
    /// what sits at the first level.
    fn bank_page(&self) -> adw::PreferencesPage {
        let bank = self.bank.borrow().clone();
        let Some(archive) = self.archive.borrow().clone() else {
            return adw::PreferencesPage::new();
        };
        let Some(root) = archive.bank(&bank).cloned() else {
            return adw::PreferencesPage::new();
        };

        let page = adw::PreferencesPage::new();
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&t("Search in this bank")));

        let group = adw::PreferencesGroup::new();
        group.set_title(&t("Browse"));
        group.set_description(Some(&t(
            "A corpus is one research project's transcripts. Depending on the bank it sits \
             directly here or one level down, inside a language or clinical group.",
        )));
        group.set_header_suffix(Some(&search));

        let results = adw::PreferencesGroup::new();
        results.set_visible(false);

        for child in &root.children {
            group.add(&self.folder_row(&[bank.clone(), child.name.clone()], child, false));
        }
        page.add(&self.filter_group());
        page.add(&group);
        page.add(&results);

        // Search and filters write into the same view: one function recomputes
        // it, so they cannot contradict each other.
        *self.results.borrow_mut() = Some(results.clone());
        *self.browse_group.borrow_mut() = Some(group.clone());

        let this = self.clone();
        search.connect_search_changed(move |e| {
            *this.search_text.borrow_mut() = e.text().to_string();
            this.refresh_results();
        });
        page
    }

    /// A row for a folder. `show_path` shows where it sits: needed in search
    /// results, where folders come from different depths.
    fn folder_row(&self, path: &[String], folder: &Folder, show_path: bool) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(&folder.name);

        let mut bits = Vec::new();
        if show_path && path.len() > 2 {
            bits.push(path[1..path.len() - 1].join(" / "));
        }
        bits.push(
            tn("%u transcript", "%u transcripts", folder.transcripts as u32)
                .replace("%u", &folder.transcripts.to_string()),
        );
        if folder.media.audio {
            bits.push(t("audio"));
        }
        if folder.media.video {
            bits.push(t("video"));
        }
        row.set_subtitle(&bits.join(" · "));

        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_activatable(true);
        let this = self.clone();
        let p = path.to_vec();
        row.connect_activated(move |_| this.show_folder(&p));
        row
    }
}

// ------------------------------------------------------------ folder page

impl ArchiveWindow {
    /// Any folder in the archive.
    ///
    /// We do not know in advance whether it is a corpus: the page shows what it
    /// contains and asks the server, in parallel, whether it is downloadable.
    fn show_folder(&self, path: &[String]) {
        let Some(archive) = self.archive.borrow().clone() else { return };
        let Some(folder) = archive.at(path).cloned() else { return };
        let page = adw::PreferencesPage::new();

        // 1. What the tree already tells us: this part makes no calls at all, so
        //    the page is never empty while waiting.
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("What this is"));
        let info = adw::ActionRow::new();
        info.set_title(&folder.name);
        info.set_subtitle(&format!(
            "{} · {}",
            bank_title(&path[0]),
            path[1..].join(" / ")
        ));
        info.set_subtitle_selectable(true);
        g.add(&info);

        let n = adw::ActionRow::new();
        n.set_title(&t("Transcripts"));
        n.set_subtitle(&folder.transcripts.to_string());
        g.add(&n);

        if folder.media.any() {
            let m = adw::ActionRow::new();
            m.set_title(&t("Media"));
            let mut what = Vec::new();
            if folder.media.audio { what.push(t("audio")); }
            if folder.media.video { what.push(t("video")); }
            if folder.media.incomplete() {
                what.push(t("partly missing or not aligned"));
            }
            m.set_subtitle(&what.join(", "));
            g.add(&m);
        }
        page.add(&g);

        // 2. The subfolders, if any. A folder can be both a downloadable corpus
        //    and hold subfolders (Brown → Adam, Eve).
        if !folder.children.is_empty() {
            let sub = adw::PreferencesGroup::new();
            sub.set_title(&t("Inside"));
            sub.set_description(Some(
                &tn("%u folder", "%u folders", folder.children.len() as u32)
                    .replace("%u", &folder.children.len().to_string()),
            ));
            for child in folder.children.iter().take(200) {
                let mut p = path.to_vec();
                p.push(child.name.clone());
                sub.add(&self.folder_row(&p, child, false));
            }
            page.add(&sub);
        }

        // 3. Documentation and citation. Citing the corpus is not a courtesy:
        //    it is a condition of using the archive.
        page.add(&self.documentation_group(path));

        // 4. Metadata, fetched on demand. If the route disappears or the network
        //    is down, the group simply does not appear.
        let meta = adw::PreferencesGroup::new();
        meta.set_title(&t("Contents"));
        let loading = adw::ActionRow::new();
        loading.set_title(&t("Loading details…"));
        loading.add_prefix(&adw::Spinner::new());
        meta.add(&loading);
        page.add(&meta);

        let this = self.clone();
        let p = path.to_vec();
        let p2 = path.to_vec();
        let meta2 = meta.clone();
        let loading2 = loading.clone();
        net().spawn(
            async move { net().client().transcript_summary(&p).await },
            move |res| {
                // The page may have been closed while the request was in flight.
                // On an already-disposed AdwPreferencesGroup, `remove` looks for
                // its internal listbox — which is NULL — and trips two libadwaita
                // assertions; the update would be lost anyway, and meanwhile the
                // closure keeps alive the widget tree that should have been
                // freed.
                if meta2.root().is_none() {
                    return;
                }
                meta2.remove(&loading2);
                match res {
                    Ok(table) => {
                        this.fill_contents(&meta2, &table);
                        this.estimate_media(&p2, &table);
                    }
                    Err(e) => {
                        if e.is_degradable() {
                            tracing::debug!("metadata unavailable: {e}");
                            meta2.set_visible(false);
                        } else {
                            let r = adw::ActionRow::new();
                            r.set_title(&t("Details are not available right now"));
                            r.set_subtitle(&e.to_string());
                            meta2.add(&r);
                        }
                    }
                }
            },
        );

        // 5. Participants: behind an expander row, because on a large corpus this
        //    call took 36 seconds.
        page.add(&self.participants_group(path));

        // 6. Download.
        page.add(&self.download_group(path, &folder));

        self.push_page(&folder.name, &page.upcast());
    }

    fn documentation_group(&self, path: &[String]) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("Documentation"));
        let url = api::corpus_page_url(path);

        let doc = adw::ActionRow::new();
        doc.set_title(&t("Open the corpus page"));
        doc.set_subtitle(&t("Description, history and the citation to use"));
        doc.add_suffix(&gtk::Image::from_icon_name("external-link-symbolic"));
        doc.set_activatable(true);
        let u = url.clone();
        let win = self.win.clone();
        doc.connect_activated(move |_| {
            gtk::UriLauncher::new(&u).launch(Some(&win), gio::Cancellable::NONE, |_| {});
        });
        g.add(&doc);

        let cite = adw::ActionRow::new();
        cite.set_title(&t("Copy the reference"));
        cite.set_subtitle(&url);
        cite.set_subtitle_selectable(true);
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_valign(gtk::Align::Center);
        copy.add_css_class("flat");
        let this = self.clone();
        let text = format!(
            "{} ({}). {}",
            path.last().cloned().unwrap_or_default(),
            bank_title(&path[0]),
            url
        );
        copy.connect_clicked(move |_| {
            this.win.clipboard().set_text(&text);
            this.toast(&t("Reference copied."));
        });
        cite.add_suffix(&copy);
        g.add(&cite);
        g
    }

    fn fill_contents(&self, group: &adw::PreferencesGroup, table: &api::Table) {
        let mut added = false;
        for (col, label) in [
            ("languages", t("Languages")),
            ("designType", t("Study design")),
            ("activityType", t("Activity")),
            ("groupType", t("Group")),
        ] {
            let values = table.distinct(col);
            if values.is_empty() {
                continue;
            }
            let r = adw::ActionRow::new();
            r.set_title(&label);
            r.set_subtitle(&values.join(", "));
            group.add(&r);
            added = true;
        }
        let with_media = table
            .rows
            .iter()
            .filter(|row| table.get(row, "media").is_some())
            .count();
        if with_media > 0 {
            let r = adw::ActionRow::new();
            r.set_title(&t("Transcripts with media"));
            r.set_subtitle(&format!("{with_media} / {}", table.rows.len()));
            group.add(&r);
            added = true;
        }
        if !added {
            group.set_visible(false);
        }
    }

    fn participants_group(&self, path: &[String]) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("Participants"));

        let exp = adw::ExpanderRow::new();
        exp.set_title(&t("Show who was recorded"));
        exp.set_subtitle(&t("Loaded on demand: on a large corpus this can take a while"));
        g.add(&exp);

        let done = Cell::new(false);
        let this = self.clone();
        let p = path.to_vec();
        exp.connect_expanded_notify(move |e| {
            if !e.is_expanded() || done.replace(true) {
                return;
            }
            let loading = adw::ActionRow::new();
            loading.set_title(&t("Loading…"));
            loading.add_prefix(&adw::Spinner::new());
            e.add_row(&loading);

            let exp2 = e.clone();
            let this2 = this.clone();
            let p2 = p.clone();
            net().spawn(
                async move { net().client().participant_summary(&p2).await },
                move |res| {
                    if exp2.root().is_none() {
                        return;
                    }
                    exp2.remove(&loading);
                    match res {
                        Ok(table) => this2.fill_participants(&exp2, &table),
                        Err(e) => {
                            let r = adw::ActionRow::new();
                            r.set_title(&t("Could not load the participants"));
                            r.set_subtitle(&e.to_string());
                            exp2.add_row(&r);
                        }
                    }
                },
            );
        });
        g
    }

    fn fill_participants(&self, exp: &adw::ExpanderRow, table: &api::Table) {
        let roles = table.distinct("role");
        if roles.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title(&t("No participant information"));
            exp.add_row(&r);
            return;
        }
        for role in roles.iter().take(12) {
            let count = table
                .rows
                .iter()
                .filter(|row| table.get(row, "role") == Some(role.as_str()))
                .count();
            let r = adw::ActionRow::new();
            r.set_title(role);
            r.set_subtitle(
                &tn("%u recording", "%u recordings", count as u32)
                    .replace("%u", &count.to_string()),
            );
            exp.add_row(&r);
        }
        // Ages are present on only a minority of rows: we show them when they
        // are there, rather than building a column that would stay empty.
        let ages: Vec<u32> = table
            .rows
            .iter()
            .filter_map(|row| table.get(row, "monthage").and_then(|v| v.parse().ok()))
            .collect();
        if !ages.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title(&t("Age of the target child"));
            let min = ages.iter().min().unwrap() / 12;
            let max = ages.iter().max().unwrap() / 12;
            r.set_subtitle(&t("from %a to %b years").replace("%a", &min.to_string()).replace("%b", &max.to_string()));
            exp.add_row(&r);
        }
    }
}

// -------------------------------------------------------------- download

impl ArchiveWindow {
    fn download_dir(&self) -> PathBuf {
        crate::config::with(|c| c.download_dir.clone()).unwrap_or_else(|| {
            glib::user_special_dir(glib::UserDirectory::Documents)
                .unwrap_or_else(glib::home_dir)
                .join("TalkBank")
        })
    }

    fn download_group(&self, path: &[String], folder: &Folder) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("Get it"));

        let dest_row = adw::ActionRow::new();
        dest_row.set_title(&t("Save into"));
        dest_row.set_subtitle(&self.download_dir().display().to_string());
        dest_row.set_subtitle_selectable(true);
        let change = gtk::Button::with_label(&t("Change…"));
        change.set_valign(gtk::Align::Center);
        let this = self.clone();
        let row2 = dest_row.clone();
        change.connect_clicked(move |_| this.choose_download_dir(&row2));
        dest_row.add_suffix(&change);
        g.add(&dest_row);

        // The media switch. It sits above the action row because it changes what
        // that row is about to do, and the size it announces is the reason
        // someone would leave it off.
        let media_row = adw::SwitchRow::new();
        media_row.set_title(&t("Include audio and video"));
        media_row.set_subtitle(&t("Checking how much that adds…"));
        media_row.set_sensitive(false);
        media_row.set_active(crate::config::with(|c| c.download_media));
        self.media_wanted.set(media_row.is_active());
        let wanted = self.media_wanted.clone();
        media_row.connect_active_notify(move |s| {
            let on = s.is_active();
            wanted.set(on);
            // Also the default for next time: the choice is nearly always the
            // same for one person's way of working.
            crate::config::update(|c| c.download_media = on);
        });
        g.add(&media_row);
        *self.media_row.borrow_mut() = Some(media_row.clone());

        let action = adw::ActionRow::new();
        action.set_title(&t("Download this corpus"));
        action.set_subtitle(&t("Checking whether this folder can be downloaded…"));
        let spinner = adw::Spinner::new();
        action.add_prefix(&spinner);

        // Progress is drawn by the manager: the same bar updates even when the
        // download was started by another visit to this page.
        action.add_suffix(&self.parent.downloads().inline(path));

        let button = gtk::Button::with_label(&t("Download"));
        button.add_css_class("suggested-action");
        button.set_valign(gtk::Align::Center);
        // Until the server has answered we cannot know whether this folder is a
        // corpus: a button disabled for a moment beats an error.
        button.set_sensitive(false);
        action.add_suffix(&button);
        // Pressing the row does the obvious thing, instead of demanding the
        // small target of the button.
        action.set_activatable_widget(Some(&button));
        g.add(&action);

        // The button does two different things depending on the probe result:
        // download *this* corpus, or download everything below it. Which of the
        // two is decided by the probe, which arrives later.
        let branch = Rc::new(Cell::new(false));
        let this = self.clone();
        let p = path.to_vec();
        let r = branch.clone();
        let media_only = Rc::new(Cell::new(false));
        let mo = media_only.clone();
        button.connect_clicked(move |_| {
            if mo.get() {
                this.start_media_only(&p);
            } else if r.get() {
                this.start_branch(&p);
            } else {
                this.start_download(&p);
            }
        });

        // How many subfolders have something to download: if there are any, a
        // "not downloadable" folder is not a dead end but a branch.
        let useful_children = folder
            .children
            .iter()
            .filter(|c| c.transcripts > 0)
            .count();

        let on_disk = talkbank_archive::download::already_there(&self.download_dir(), path);

        // The probe: the only thing that separates a corpus from a collection.
        let p = path.to_vec();
        let n = folder.transcripts;
        let act = action.clone();
        let btn = button.clone();
        let probe = p.clone();
        net().spawn(
            async move { net().client().is_downloadable(&probe).await },
            move |outcome| {
                if act.root().is_none() {
                    return;
                }
                // Away with the spinner: whatever the answer, the wait is over.
                // It has to be removed by reference: `add_prefix` puts it in an
                // internal box, so looking for it among the direct children fails.
                act.remove(&spinner);
                match outcome {
                    Downloadable::Yes => {
                        btn.set_sensitive(true);
                        if on_disk {
                            // Already downloaded. The interesting question is no
                            // longer "get this?" but "get what is missing?",
                            // which for most people means the recordings.
                            media_only.set(true);
                            act.set_title(&t("On your disk"));
                            act.set_subtitle(
                                &tn("%u transcript", "%u transcripts", n as u32)
                                    .replace("%u", &n.to_string()),
                            );
                            btn.set_label(&t("Get the media"));
                            btn.remove_css_class("suggested-action");
                        } else {
                            act.set_subtitle(
                                &tn("%u transcript", "%u transcripts", n as u32)
                                    .replace("%u", &n.to_string()),
                            );
                        }
                    }
                    Downloadable::No if useful_children > 0 => {
                        // Not a corpus, but there are corpora below: the button
                        // downloads the branch instead of disappearing.
                        branch.set(true);
                        act.set_title(&t("Download everything in here"));
                        // Two counts in a single string do not decline in any
                        // language: they are two sentences, joined by a separator.
                        act.set_subtitle(&format!(
                            "{} · {}",
                            tn("%c folder below", "%c folders below", useful_children as u32)
                                .replace("%c", &useful_children.to_string()),
                            tn("%t transcript in all", "%t transcripts in all", n as u32)
                                .replace("%t", &n.to_string())
                        ));
                        btn.set_label(&t("Download all"));
                        btn.set_sensitive(true);
                    }
                    Downloadable::No if n > 0 => {
                        // No subfolders but some transcripts: we are *inside* a
                        // corpus (Brown/Adam). The button does not disappear; it
                        // looks for the corpus these files belong to — which is
                        // the useful thing to know here.
                        branch.set(true);
                        act.set_title(&t("This folder is inside a corpus"));
                        act.set_subtitle(
                            &tn("%t transcript here", "%t transcripts here", n as u32)
                                .replace("%t", &n.to_string()),
                        );
                        btn.set_label(&t("Find the corpus"));
                        btn.set_sensitive(true);
                    }
                    Downloadable::No => {
                        act.set_title(&t("Nothing to download here"));
                        act.set_subtitle(&t("This folder has no transcripts under it."));
                        btn.set_visible(false);
                    }
                    Downloadable::NeedsPermission => {
                        act.set_title(&t("Written permission required"));
                        act.set_subtitle(&t(
                            "This bank is restricted: ask macw@cmu.edu for access, then sign in again.",
                        ));
                        btn.set_visible(false);
                    }
                    Downloadable::SignInRequired => {
                        // Without a session the server answers a corpus and a
                        // collection identically: it cannot be known in advance.
                        btn.set_sensitive(true);
                        act.set_subtitle(&t("Sign in to download"));
                    }
                    Downloadable::Unknown(e) => {
                        // We could not ask: let them try, the real error is more
                        // informative than a disabled button.
                        tracing::debug!("probe failed: {e}");
                        btn.set_sensitive(true);
                        act.set_subtitle(&t("Could not check; try anyway"));
                    }
                }
            },
        );
        g
    }

    fn choose_download_dir(&self, row: &adw::ActionRow) {
        let dialog = gtk::FileDialog::new();
        dialog.set_title(&t("Where to save the corpora"));
        dialog.set_initial_folder(Some(&gio::File::for_path(self.download_dir())));
        let row = row.clone();
        dialog.select_folder(Some(&self.win), gio::Cancellable::NONE, move |res| {
            if let Ok(f) = res {
                if let Some(p) = f.path() {
                    row.set_subtitle(&p.display().to_string());
                    crate::config::update(|c| c.download_dir = Some(p));
                }
            }
        });
    }

    /// Downloads everything under a folder that is not itself a corpus.
    ///
    /// A plan is needed first, and the plan costs one request per folder: we show
    /// progress and allow cancelling. The confirmation comes afterwards, because
    /// before that neither the corpus count nor the transcript count is known.
    fn start_branch(&self, path: &[String]) {
        // No pre-emptive session check: the local copy can be stale, and the plan
        // finds out for itself on the first request by answering `needs_sign_in`.
        // An up-to-date truth is worth more than a shortcut.
        let Some(archive) = self.archive.borrow().clone() else { return };

        // The maximum cost is known already, without sending anything: it is the
        // number of folders with data in the subtree, which the cached tree knows.
        let to_examine = archive
            .at(path)
            .map(|f| {
                fn count(f: &talkbank_archive::catalog::Folder) -> usize {
                    1 + f.children.iter().filter(|c| c.transcripts > 0).map(count).sum::<usize>()
                }
                count(f)
            })
            .unwrap_or(0);
        let waiting = adw::AlertDialog::new(
            Some(&t("Looking for corpora…")),
            Some(
                &t("Checking up to %u folders to see which ones can be downloaded.")
                    .replace("%u", &to_examine.to_string()),
            ),
        );
        waiting.add_response("cancel", &t("Cancel"));
        waiting.set_close_response("cancel");

        // The flag is read by the network thread: it has to be atomic.
        let cancel = Arc::new(AtomicBool::new(false));
        let a2 = cancel.clone();
        waiting.connect_response(None, move |_, _| a2.store(true, Ordering::Relaxed));
        waiting.present(Some(&self.win));

        let p = path.to_vec();
        let this = self.clone();
        let dlg = waiting.clone();
        let dlg2 = waiting.clone();
        let stop = cancel.clone();
        let root = p.clone();

        net().spawn_with_progress(
            move |tx| async move {
                talkbank_archive::batch::plan(
                    net().client(),
                    &archive,
                    &p,
                    |done, queued| {
                        let _ = tx.try_send((done, queued));
                    },
                    move || !stop.load(Ordering::Relaxed),
                )
                .await
            },
            move |(done, queued)| {
                // Only the folders examined: the queue grows as the work advances,
                // and a number going up makes it look like nothing is progressing.
                let _ = queued;
                dlg.set_body(&t("%n folders checked").replace("%n", &done.to_string()));
            },
            move |plan| {
                // Closing this dialog emits its close response, which is
                // "cancel": reading the flag *after* closing it would always find
                // it raised. Whether the user cancelled is what the plan says.
                tracing::debug!(
                    "branch plan for {}: {} corpora, {} probes, cancelled={} \
                     unreliable={} truncated={}",
                    root.join("/"), plan.corpora.len(), plan.probed, plan.cancelled,
                    plan.unreliable, plan.truncated
                );
                dlg2.close();
                this.confirm_branch(&root, plan);
            },
        );
    }

    /// Shows how much is about to arrive and asks for confirmation. A branch can
    /// be hundreds of megabytes: starting without saying so would be an unwelcome
    /// surprise.
    ///
    /// Before the numbers come the cases where the plan must **not** be offered:
    /// if it is incomplete because something failed, showing it as though it were
    /// everything would be the worse of the two lies.
    fn confirm_branch(&self, root: &[String], plan: talkbank_archive::batch::Plan) {
        if plan.cancelled {
            return; // the user has already said no
        }
        if plan.needs_sign_in {
            self.parent.ask_to_sign_in();
            return;
        }
        if plan.unreliable {
            let dlg = adw::AlertDialog::new(
                Some(&t("The archive is not answering right now")),
                Some(&t(
                    "Some folders could not be checked, so this list would be incomplete. Try again in a few minutes.",
                )),
            );
            dlg.add_response("close", &t("Close"));
            dlg.present(Some(&self.win));
            return;
        }
        if plan.is_empty() {
            self.offer_ancestor(root, &plan);
            return;
        }

        let n = plan.corpora.len();
        let dest_root = self.download_dir();
        let dest = talkbank_archive::download::destination(&dest_root, root);
        let already = plan
            .corpora
            .iter()
            .filter(|c| talkbank_archive::download::already_there(&dest_root, c))
            .count();
        let to_do = n - already;

        let mut body = vec![
            tn("%u transcript in all", "%u transcripts in all", plan.transcripts as u32)
                .replace("%u", &plan.transcripts.to_string()),
            t("About %s of transcripts.").replace("%s", &human_size(plan.transcripts)),
            t("Into %p").replace("%p", &dest.display().to_string()),
        ];
        if already > 0 {
            body.push(
                tn(
                    "%u of these is already on disk and will be skipped.",
                    "%u of these are already on disk and will be skipped.",
                    already as u32,
                )
                .replace("%u", &already.to_string()),
            );
        }
        // Skipped items are counted in transcripts, not folders: under a 401 we
        // do not know how many corpora there are, and the transcript count is
        // comparable with what the button's row had promised.
        if plan.locked_transcripts() > 0 {
            body.push(
                tn(
                    "%t transcripts in %n folder need a separate permission and will be skipped.",
                    "%t transcripts in %n folders need a separate permission and will be skipped.",
                    plan.locked_folders() as u32,
                )
                .replace("%t", &plan.locked_transcripts().to_string())
                .replace("%n", &plan.locked_folders().to_string()),
            );
        }
        if plan.unverified() > 0 {
            body.push(
                tn(
                    "%u folder could not be checked and was left out.",
                    "%u folders could not be checked and were left out.",
                    plan.unverified() as u32,
                )
                .replace("%u", &plan.unverified().to_string()),
            );
        }
        if plan.truncated {
            body.push(
                t("Partial list: stopped after checking %u folders.")
                    .replace("%u", &plan.probed.to_string()),
            );
        }

        let dlg = adw::AlertDialog::new(
            Some(
                &tn("Download %u corpus?", "Download %u corpora?", to_do.max(1) as u32)
                    .replace("%u", &to_do.to_string()),
            ),
            Some(&body.join("\n")),
        );
        dlg.add_response("cancel", &t("Cancel"));
        if already > 0 {
            dlg.add_response("again", &t("Download again"));
        }
        if plan.truncated {
            // The list is partial: carrying on looking is more likely what is
            // wanted, so that is the highlighted action.
            dlg.add_response("more", &t("Keep looking"));
            dlg.set_response_appearance("more", adw::ResponseAppearance::Suggested);
            dlg.add_response("go", &t("Download these"));
            dlg.set_default_response(Some("more"));
        } else {
            dlg.add_response("go", &t("Download all"));
            dlg.set_response_appearance("go", adw::ResponseAppearance::Suggested);
            dlg.set_default_response(Some("go"));
        }
        dlg.set_close_response("cancel");

        // The media choice rides in the dialog rather than in a preference: on a
        // branch it is the difference between 27 MB and 14 GB, so it belongs in
        // front of the person about to commit to it.
        let media = gtk::CheckButton::with_label(&t("Include audio and video"));
        media.set_active(crate::config::with(|c| c.download_media));
        media.set_margin_top(6);
        dlg.set_extra_child(Some(&media));

        // Sampling costs a handful of requests, so it only runs if the box is
        // actually ticked.
        let this = self.clone();
        let sampled = Rc::new(Cell::new(false));
        let corpora = plan.corpora.clone();
        let m2 = media.clone();
        media.connect_toggled(move |c| {
            let on = c.is_active();
            crate::config::update(|cfg| cfg.download_media = on);
            if !on || sampled.replace(true) {
                return;
            }
            c.set_label(Some(&t("Include audio and video — checking size…")));
            this.estimate_branch_media(&corpora, &m2);
        });
        if media.is_active() {
            media.emit_by_name::<()>("toggled", &[]);
        }

        let this = self.clone();
        let r = root.to_vec();
        let m3 = media.clone();
        dlg.choose(Some(&self.win), gio::Cancellable::NONE, move |resp| {
            let with_media = m3.is_active();
            match resp.as_str() {
                "go" => this.parent.downloads().start_many(
                    &plan.corpora,
                    this.download_dir(),
                    &r,
                    false,
                    with_media,
                ),
                "again" => this.parent.downloads().start_many(
                    &plan.corpora,
                    this.download_dir(),
                    &r,
                    true,
                    with_media,
                ),
                "more" => this.keep_looking(&r, plan.clone()),
                _ => {}
            }
        });
    }

    /// Resumes a planning run cut short by the ceiling, without paying again for
    /// the probes already made.
    fn keep_looking(&self, root: &[String], mut previous: talkbank_archive::batch::Plan) {
        let Some(archive) = self.archive.borrow().clone() else { return };
        let waiting = adw::AlertDialog::new(Some(&t("Looking for corpora…")), None);
        waiting.add_response("cancel", &t("Cancel"));
        waiting.set_close_response("cancel");
        let cancel = Arc::new(AtomicBool::new(false));
        let a2 = cancel.clone();
        waiting.connect_response(None, move |_, _| a2.store(true, Ordering::Relaxed));
        waiting.present(Some(&self.win));

        let resume = std::mem::take(&mut previous.resume);
        let this = self.clone();
        let dlg = waiting.clone();
        let dlg2 = waiting.clone();
        let stop = cancel.clone();
        let r = root.to_vec();
        let base = previous;

        net().spawn_with_progress(
            move |tx| async move {
                talkbank_archive::batch::plan_from(
                    net().client(),
                    &archive,
                    &resume,
                    |done| {
                        let _ = tx.try_send(done);
                    },
                    move || !stop.load(Ordering::Relaxed),
                )
                .await
            },
            move |done| {
                dlg.set_body(&t("%n folders checked").replace("%n", &done.to_string()));
            },
            move |rest| {
                dlg2.close();
                this.confirm_branch(&r, base.merged(rest));
            },
        );
    }

    /// When there is nothing below, look up: very often the folder is *inside* a
    /// corpus, and the useful thing to say is which one.
    fn offer_ancestor(&self, root: &[String], plan: &talkbank_archive::batch::Plan) {
        if root.len() < 2 {
            self.toast(&t("No downloadable corpus found under this folder."));
            return;
        }
        // From the nearest upwards: a handful of requests at most, and only after
        // descending has already failed.
        let ancestors: Vec<Vec<String>> = (1..root.len()).rev().map(|k| root[..k].to_vec()).collect();
        let locked = plan.locked_transcripts() > 0;
        let this = self.clone();
        net().spawn(
            async move {
                for a in ancestors {
                    if net().client().is_downloadable(&a).await == Downloadable::Yes {
                        return Some(a);
                    }
                }
                None
            },
            move |found| match found {
                Some(a) => this.suggest_corpus(&a),
                None if locked => this.toast(&t(
                    "Everything here needs a separate permission: write to macw@cmu.edu.",
                )),
                None => this.toast(&t("No downloadable corpus found under this folder.")),
            },
        );
    }

    /// "These files are part of X": takes you to the corpus page, where the size
    /// is visible before deciding.
    fn suggest_corpus(&self, path: &[String]) {
        let name = path.last().cloned().unwrap_or_default();
        let how_many = self
            .archive
            .borrow()
            .as_ref()
            .and_then(|a| a.at(path).map(|f| f.transcripts))
            .unwrap_or(0);
        let dlg = adw::AlertDialog::new(
            Some(&t("These files are part of %s").replace("%s", &name)),
            Some(
                &tn(
                    "Download the whole corpus instead: %u transcript.",
                    "Download the whole corpus instead: %u transcripts.",
                    how_many as u32,
                )
                .replace("%u", &how_many.to_string()),
            ),
        );
        dlg.add_response("cancel", &t("Cancel"));
        dlg.add_response("go", &t("Go to the corpus"));
        dlg.set_response_appearance("go", adw::ResponseAppearance::Suggested);
        dlg.set_default_response(Some("go"));
        dlg.set_close_response("cancel");
        let this = self.clone();
        let p = path.to_vec();
        dlg.choose(Some(&self.win), gio::Cancellable::NONE, move |resp| {
            if resp == "go" {
                this.show_folder(&p);
            }
        });
    }

    /// Starts the download by handing it to the manager: from here on the page
    /// is out of the picture, and you can change section without stopping it.
    /// Fills in what the media would cost, from the metadata the page already
    /// fetched plus a small sample of real file sizes.
    ///
    /// The count is exact and free: the table says which transcripts declare a
    /// recording. The size is sampled, because it cannot be guessed — audio runs
    /// from half a megabyte to seventy, and one corpus of video reaches ten
    /// gigabytes. A per-corpus sample is the only estimate worth showing.
    fn estimate_media(&self, path: &[String], table: &api::Table) {
        let Some(row) = self.media_row.borrow().clone() else {
            return;
        };
        // Names of the transcripts that declare a recording, and whether it is
        // video. `missing` means the archive does not hold it.
        let candidates: Vec<(String, bool)> = table
            .rows
            .iter()
            .filter_map(|r| {
                let flags = table.get(r, "media")?;
                if flags.contains("missing") {
                    return None;
                }
                let name = table.get(r, "filename")?;
                Some((name.to_string(), flags.contains("video")))
            })
            .collect();

        if candidates.is_empty() {
            row.set_subtitle(&t("This corpus has no media"));
            row.set_active(false);
            row.set_sensitive(false);
            self.media_wanted.set(false);
            return;
        }
        row.set_sensitive(true);

        let total = candidates.len();
        // Five is enough for an order of magnitude and cheap enough to run on
        // every corpus page.
        let sample: Vec<(Vec<String>, String, bool)> = candidates
            .iter()
            .take(5)
            .map(|(name, video)| (path.to_vec(), name.clone(), *video))
            .collect();

        let r2 = row.clone();
        net().spawn(
            async move {
                let mut sizes = Vec::new();
                for (dir, name, video) in sample {
                    let ext = talkbank_archive::download::extensions(video)[0];
                    let url = talkbank_archive::api::media_url(&dir, &name, ext);
                    if let Some(n) = talkbank_archive::download::media_size(net().client(), &url).await
                    {
                        sizes.push(n);
                    }
                }
                sizes
            },
            move |sizes| {
                if r2.root().is_none() {
                    return;
                }
                let counted = tn("%n file", "%n files", total as u32)
                    .replace("%n", &total.to_string());
                if sizes.is_empty() {
                    // Signed out, or the names do not line up with the server.
                    // Say the count and stop guessing at the size.
                    r2.set_subtitle(&counted);
                    return;
                }
                let mean = sizes.iter().sum::<u64>() / sizes.len() as u64;
                r2.set_subtitle(
                    &t("%n, about %s more")
                        .replace("%n", &counted)
                        .replace("%s", &human_bytes(mean * total as u64)),
                );
            },
        );
    }

    /// What the recordings of a whole branch would cost, roughly.
    ///
    /// Sampling every corpus would be dozens of requests, so three are taken and
    /// scaled by the transcript count. The label says "estimate" because with a
    /// 25x spread between corpora it genuinely is one.
    fn estimate_branch_media(&self, corpora: &[Vec<String>], label: &gtk::CheckButton) {
        let sample: Vec<Vec<String>> = corpora.iter().take(3).cloned().collect();
        let total_corpora = corpora.len();
        let lbl = label.clone();
        net().spawn(
            async move {
                let client = net().client();
                let (mut bytes, mut files, mut seen_corpora) = (0u64, 0usize, 0usize);
                for path in &sample {
                    let Ok(table) = client.transcript_summary(path).await else {
                        continue;
                    };
                    seen_corpora += 1;
                    let candidates: Vec<(String, bool)> = table
                        .rows
                        .iter()
                        .filter_map(|r| {
                            let flags = table.get(r, "media")?;
                            if flags.contains("missing") {
                                return None;
                            }
                            Some((table.get(r, "filename")?.to_string(), flags.contains("video")))
                        })
                        .collect();
                    files += candidates.len();
                    for (name, video) in candidates.iter().take(2) {
                        let ext = talkbank_archive::download::extensions(*video)[0];
                        let url = talkbank_archive::api::media_url(path, name, ext);
                        if let Some(n) =
                            talkbank_archive::download::media_size(client, &url).await
                        {
                            bytes += n;
                            seen_corpora = seen_corpora.max(1);
                        }
                    }
                }
                (bytes, files, seen_corpora)
            },
            move |(bytes, files, seen)| {
                if lbl.root().is_none() {
                    return;
                }
                if bytes == 0 || files == 0 || seen == 0 {
                    lbl.set_label(Some(&t("Include audio and video")));
                    return;
                }
                // Mean per file from the sample, times the files the sampled
                // corpora hold, scaled to the whole branch.
                let sampled_files = files.min(seen * 2).max(1) as u64;
                let per_file = bytes / sampled_files;
                let est = per_file * files as u64 * (total_corpora as u64) / seen.max(1) as u64;
                lbl.set_label(Some(
                    &t("Include audio and video — about %s more (estimate)")
                        .replace("%s", &human_bytes(est)),
                ));
            },
        );
    }

    /// Fetches only the recordings of a corpus whose transcripts are already
    /// on disk.
    fn start_media_only(&self, path: &[String]) {
        self.parent
            .downloads()
            .start_media_only(path, self.download_dir());
    }

    fn start_download(&self, path: &[String]) {
        // No pre-emptive check here either: if there is no session the queue
        // notices, pauses and asks for a sign-in — then resumes on its own,
        // without restarting anything.
        self.parent
            .downloads()
            .start(path, self.download_dir(), self.media_wanted.get());
    }

    fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }
}

/// A readable size estimate, from the transcript count.
///
/// It is avowedly approximate: the measurement comes from four corpora, and
/// corpora vary. It exists to tell "twenty megabytes" from "a gigabyte", which
/// is the difference that matters before pressing.
fn human_size(transcripts: usize) -> String {
    let bytes = transcripts as u64 * talkbank_archive::download::BYTES_PER_TRANSCRIPT;
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else {
        format!("{} MB", (bytes / 1_048_576).max(1))
    }
}

/// A readable size from a byte count. Unlike `human_size` this starts from real
/// bytes, because media sizes are measured rather than derived from a constant.
fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else {
        format!("{} MB", (bytes / 1_048_576).max(1))
    }
}

/// Corpora extract into subfolders (`Brown/Adam`, `Brown/Eve`): if the `.cha`
/// files all sit in one of them we open that one; otherwise the root, where the
/// "include subfolders" option covers the rest.
pub fn analysis_folder(dest: &std::path::Path) -> PathBuf {
    let has_cha = |d: &std::path::Path| {
        std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "cha"))
    };
    if has_cha(dest) {
        return dest.to_path_buf();
    }
    let subdirs: Vec<PathBuf> = std::fs::read_dir(dest)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && has_cha(p))
        .collect();
    match subdirs.as_slice() {
        [only] => only.clone(),
        _ => dest.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::analysis_folder;

    #[test]
    fn the_estimate_separates_orders_of_magnitude() {
        use super::human_size;
        // It is there to tell twenty megabytes from a gigabyte before pressing,
        // not to be exact.
        assert_eq!(human_size(1), "1 MB");
        assert_eq!(human_size(171), "3 MB");
        assert!(human_size(53_431).ends_with(" GB"), "{}", human_size(53_431));
    }

    #[test]
    fn with_cha_files_in_the_root_the_root_is_opened() {
        let d = tempdir::TempDir::new("talkbank-af").unwrap();
        std::fs::write(d.path().join("a.cha"), "").unwrap();
        assert_eq!(analysis_folder(d.path()), d.path());
    }

    #[test]
    fn with_a_single_subfolder_that_one_is_opened() {
        let d = tempdir::TempDir::new("talkbank-af").unwrap();
        let sub = d.path().join("Adam");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("a.cha"), "").unwrap();
        std::fs::write(d.path().join("0metadata.cdc"), "").unwrap();
        assert_eq!(analysis_folder(d.path()), sub);
    }

    #[test]
    fn with_several_subfolders_the_root_is_opened() {
        let d = tempdir::TempDir::new("talkbank-af").unwrap();
        for name in ["Adam", "Eve", "Sarah"] {
            let sub = d.path().join(name);
            std::fs::create_dir(&sub).unwrap();
            std::fs::write(sub.join("a.cha"), "").unwrap();
        }
        assert_eq!(analysis_folder(d.path()), d.path());
    }
}

// ---------------------------------------------------------------- filters

use talkbank_archive::index::{self as idx, Filter};

impl ArchiveWindow {
    /// Recomputes the view from the search text and the filters together.
    ///
    /// When neither is active we go back to the top-level list: showing hundreds
    /// of flat rows as the resting state would be a wall.
    fn refresh_results(&self) {
        let (Some(results), Some(browse)) = (
            self.results.borrow().clone(),
            self.browse_group.borrow().clone(),
        ) else {
            return;
        };
        for r in self.result_rows.borrow_mut().drain(..) {
            results.remove(&r);
        }

        let query = self.search_text.borrow().clone();
        let filter = self.filter.borrow().clone();
        if query.trim().is_empty() && filter.is_empty() {
            browse.set_visible(true);
            results.set_visible(false);
            return;
        }

        let Some(archive) = self.archive.borrow().clone() else { return };
        let bank = self.bank.borrow().clone();
        let mut hits = archive.search(&bank, &query);

        // The filters need the index: without it they stay inert, and the
        // interface says so rather than ignoring them in silence.
        if !filter.is_empty() {
            if let Some(index) = self.index.borrow().as_ref() {
                let allowed: std::collections::HashSet<Vec<String>> = index
                    .matching(&filter)
                    .into_iter()
                    .map(|c| c.path.clone())
                    .collect();
                hits.retain(|(path, _)| allowed.contains(path));
            }
        }

        browse.set_visible(false);
        results.set_visible(true);
        results.set_title(&t("Results"));
        let total = hits.len();
        results.set_description(Some(
            &tn("%u folder found", "%u folders found", total as u32)
                .replace("%u", &total.to_string()),
        ));
        // A ceiling on the rows built: in CABank an empty search returns eight
        // hundred, and building them all would freeze the interface.
        for (path, folder) in hits.into_iter().take(120) {
            let row = self.folder_row(&path, folder, true);
            results.add(&row);
            self.result_rows.borrow_mut().push(row);
        }
        if total > 120 {
            let r = adw::ActionRow::new();
            r.set_title(&t("Showing the first 120; narrow the search to see the rest"));
            results.add(&r);
            self.result_rows.borrow_mut().push(r);
        }
    }

    fn filter_group(&self) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        let exp = adw::ExpanderRow::new();
        exp.set_title(&t("Filter by metadata"));

        match self.index.borrow().clone() {
            None => {
                exp.set_subtitle(&t("Needs a one-off index of this bank"));
                exp.add_row(&self.build_index_row());
            }
            Some(index) => {
                exp.set_subtitle(
                    &tn("indexed: %u corpus", "indexed: %u corpora", index.corpora.len() as u32)
                        .replace("%u", &index.corpora.len().to_string()),
                );
                exp.add_row(&self.facet_row(&t("Language"), index.languages(), |f, v| {
                    f.language = v
                }));
                exp.add_row(&self.facet_row(&t("Study design"), index.designs(), |f, v| {
                    f.design = v
                }));
                exp.add_row(&self.facet_row(&t("Group"), index.groups(), |f, v| f.group = v));

                let media = adw::SwitchRow::new();
                media.set_title(&t("Only corpora with audio or video"));
                let this = self.clone();
                media.connect_active_notify(move |s| {
                    this.filter.borrow_mut().only_with_media = s.is_active();
                    this.refresh_results();
                });
                exp.add_row(&media);

                exp.add_row(&self.build_index_row());
            }
        }
        g.add(&exp);
        g
    }

    /// A chooser row for one facet. The first entry is "all".
    fn facet_row(
        &self,
        title: &str,
        values: Vec<String>,
        set: fn(&mut Filter, Option<String>),
    ) -> adw::ComboRow {
        let model = gtk::StringList::new(&[]);
        model.append(&t("all"));
        for v in &values {
            model.append(v);
        }
        let row = adw::ComboRow::new();
        row.set_title(title);
        row.set_model(Some(&model));

        let this = self.clone();
        row.connect_selected_notify(move |r| {
            let i = r.selected();
            let chosen = (i > 0).then(|| values[(i - 1) as usize].clone());
            set(&mut this.filter.borrow_mut(), chosen);
            this.refresh_results();
        });
        row
    }

    fn build_index_row(&self) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(&t("Build the metadata index"));
        row.set_subtitle(&t(
            "Asks the archive about every folder of this bank once, so language and study type \
             can be filtered across it. Takes a few minutes; the result is saved.",
        ));
        let progress = gtk::ProgressBar::new();
        progress.set_valign(gtk::Align::Center);
        progress.set_visible(false);
        row.add_suffix(&progress);

        let button = gtk::Button::with_label(&t("Build"));
        button.set_valign(gtk::Align::Center);
        row.add_suffix(&button);

        let this = self.clone();
        let prog = progress.clone();
        let btn = button.clone();
        button.connect_clicked(move |_| {
            let Some(archive) = this.archive.borrow().clone() else { return };
            // Telling a corpus from a collection needs a session: without one the
            // index would come back empty after thousands of pointless requests.
            // The check is here rather than at row construction because the
            // automatic sign-in can finish after the page exists.
            if !this.logged_in.get() {
                this.parent.ask_to_sign_in();
                return;
            }
            let bank = this.bank.borrow().clone();
            btn.set_sensitive(false);
            prog.set_visible(true);
            prog.set_fraction(0.0);

            let p2 = prog.clone();
            let b2 = btn.clone();
            let t2 = this.clone();
            net().spawn_with_progress(
                move |tx| async move {
                    idx::build(net().client(), &archive, &bank, |done, total| {
                        let _ = tx.try_send((done, total));
                    })
                    .await
                },
                move |(done, total)| {
                    p2.set_fraction(done as f64 / total.max(1) as f64);
                    p2.set_text(Some(&format!("{done}/{total}")));
                },
                move |index| {
                    b2.set_sensitive(true);
                    if let Err(e) = idx::store(&index) {
                        tracing::warn!("index not saved: {e}");
                    }
                    let n = index.corpora.len();
                    *t2.index.borrow_mut() = Some(index);
                    t2.toast(
                        &tn("Indexed %u corpus.", "Indexed %u corpora.", n as u32)
                            .replace("%u", &n.to_string()),
                    );
                    // Rebuild the page: the filter menus only exist once there is
                    // an index to take their values from.
                    t2.nav.pop();
                    let bank = t2.bank.borrow().clone();
                    t2.push_page(bank_title(&bank), &t2.bank_page().upcast());
                },
            );
        });
        row
    }
}
