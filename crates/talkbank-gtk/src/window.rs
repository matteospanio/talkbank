//! The main window: a shell with four sections.
//!
//! **Start** resumes the work, **Transcripts** opens and corrects the files,
//! **Analysis** runs the programs, **Archive** downloads from TalkBank. They all
//! live in here, in one place: browsing the archive and correcting a transcript
//! are moments of the same work, and keeping them in separate windows meant
//! arranging those windows by hand.
//!
//! The order inside "Analysis" is the manual's own (Part 2, ch. 3): working
//! folder → files → command → options → run. The difference from the original
//! Commands window is that each command's requirements are checked *first*,
//! instead of turning up as cryptic messages afterwards.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use talkbank_engine::catalog::{self, Command, Req};
use talkbank_engine::{chat, runner};

use crate::config;
use crate::i18n::{t, tn};
use crate::state::{self, Analysis, Hint, Preflight, TIERS};

pub struct Inner {
    win: adw::ApplicationWindow,
    bin_dir: PathBuf,
    lib_dir: PathBuf,
    workdir: RefCell<PathBuf>,
    an: RefCell<Analysis>,

    toasts: adw::ToastOverlay,
    cmd_list: gtk::ListBox,
    search: gtk::SearchEntry,
    title: adw::WindowTitle,
    page_holder: adw::Bin,
    banner: adw::Banner,
    banner_fix: RefCell<Option<Preflight>>,
    cmdline: gtk::Label,
    cmdline_box: gtk::Widget,
    run_btn: gtk::Button,
    spinner: gtk::Widget,
    status: gtk::Label,
    out_buf: gtk::TextBuffer,
    err_buf: gtk::TextBuffer,
    out_stack: adw::ViewStack,
    created_list: gtk::ListBox,
    files_group: RefCell<Option<adw::PreferencesGroup>>,
    who_group: RefCell<Option<adw::PreferencesGroup>>,
    who_rows: RefCell<Vec<gtk::Widget>>,
    /// The selected Batchalign task: an alternative to the CLAN command, never
    /// both. The two engines have different pages and different requirements.
    ba_task: RefCell<Option<&'static crate::batchalign::Task>>,
    css: gtk::CssProvider,

    // -------------------------------------------------------------- shell
    /// The four sections. The sidebar lists them, this stack shows them.
    stack: adw::ViewStack,
    sections: gtk::ListBox,
    /// The separator between the sections and the lower part of the sidebar.
    context_sep: gtk::Separator,
    /// The folder's file list, for opening them in the editor.
    file_list: gtk::ListBox,
    /// True while we are repopulating the list: the selection changes on its own
    /// and must not reopen a file under the hands of someone typing.
    filling: Cell<bool>,
    /// Built on first visit: `Editor` needs `App`, which does not exist yet at
    /// this point in construction.
    editor: RefCell<Option<crate::editor::Editor>>,
    /// The two context sidebars, built once: recreating them on every section
    /// change would reparent the same list and add one more signal handler each
    /// time.
    cmd_pane: RefCell<Option<gtk::Widget>>,
    file_pane: RefCell<Option<gtk::Widget>>,
    /// The downloads: they live here rather than in the archive page, so they
    /// keep going when you change section.
    downloads: RefCell<Option<crate::downloads::Manager>>,
    /// The archive section, so it can be spoken to from outside.
    archive_section: RefCell<Option<crate::archive::ArchiveWindow>>,
}

#[derive(Clone)]
pub struct App(Rc<Inner>);

impl std::ops::Deref for App {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

// ---------------------------------------------------------------- costruzione

impl App {
    pub fn build(app: &adw::Application, start: Option<PathBuf>) -> App {
        let bin_dir = talkbank_engine::find_bin_dir().unwrap_or_else(|| PathBuf::from("."));
        let lib_dir = find_lib_dir(&bin_dir);
        let workdir = start
            .or_else(|| config::with(|c| c.workdir.clone()))
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| default_workdir(&lib_dir));

        let win = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(1180)
            .default_height(800)
            .title("CLAN")
            .build();

        let inner = Inner {
            win: win.clone(),
            bin_dir,
            lib_dir,
            workdir: RefCell::new(workdir),
            an: RefCell::new(Analysis {
                opt_repetitions: config::with(|c| c.default_r6),
                opt_save: config::with(|c| c.default_save),
                ..Default::default()
            }),
            toasts: adw::ToastOverlay::new(),
            cmd_list: gtk::ListBox::new(),
            search: gtk::SearchEntry::new(),
            title: adw::WindowTitle::new("CLAN", ""),
            page_holder: adw::Bin::new(),
            banner: adw::Banner::new(""),
            banner_fix: RefCell::new(None),
            cmdline: gtk::Label::new(None),
            cmdline_box: gtk::ScrolledWindow::new().upcast(),
            run_btn: gtk::Button::new(),
            spinner: adw::Spinner::new().upcast(),
            status: gtk::Label::new(None),
            out_buf: gtk::TextBuffer::new(None),
            err_buf: gtk::TextBuffer::new(None),
            out_stack: adw::ViewStack::new(),
            created_list: gtk::ListBox::new(),
            files_group: RefCell::new(None),
            who_group: RefCell::new(None),
            who_rows: RefCell::new(Vec::new()),
            ba_task: RefCell::new(None),
            css: gtk::CssProvider::new(),
            stack: adw::ViewStack::new(),
            sections: gtk::ListBox::new(),
            context_sep: gtk::Separator::new(gtk::Orientation::Horizontal),
            file_list: gtk::ListBox::new(),
            filling: Cell::new(false),
            editor: RefCell::new(None),
            cmd_pane: RefCell::new(None),
            file_pane: RefCell::new(None),
            downloads: RefCell::new(None),
            archive_section: RefCell::new(None),
        };
        let this = App(Rc::new(inner));
        this.assemble();
        this
    }

    fn assemble(&self) {
        let split = adw::NavigationSplitView::builder()
            .sidebar_width_fraction(0.26)
            .max_sidebar_width(340.0)
            .build();
        split.set_sidebar(Some(&adw::NavigationPage::new(&self.shell_sidebar(), "CLAN")));
        split.set_content(Some(&adw::NavigationPage::new(&self.shell_content(), "CLAN")));

        self.toasts.set_child(Some(&split));
        self.win.set_content(Some(&self.toasts));

        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("nessun display"),
            &self.css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        self.apply_font();
        apply_theme();

        self.refresh_files();
        // The sidebar already exists: it has to be filled now, after reading the
        // folder. Building it earlier and leaving it empty is as good as not
        // having it.
        self.refresh_file_list();
        self.restore_last_command();
        self.rebuild_content();
        // We start on "Start": the first question on opening is what to do, not
        // which option of which program.
        self.show_section("home");

        // Closing with unsaved changes: ask, do not lose.
        let this = self.clone();
        self.win.connect_close_request(move |_| {
            if let Some(editor) = this.editor.borrow().clone() {
                if editor.is_dirty() {
                    editor.confirm_then_close();
                    return glib::Propagation::Stop;
                }
            }
            // A queued download leaves no rubbish behind when stopped —
            // extraction is atomic — but it has to be said, because the work is
            // lost.
            let in_flight = this
                .downloads
                .borrow()
                .as_ref()
                .map(|d| d.jobs().iter().filter(|j| j.phase.pending()).count())
                .unwrap_or(0);
            if in_flight == 0 {
                return glib::Propagation::Proceed;
            }
            this.confirm_close_with_downloads(in_flight);
            glib::Propagation::Stop
        });

        self.win.present();
    }

    // ------------------------------------------------------------------- shell

    /// The sidebar: the sections on top, what the section needs below.
    fn shell_sidebar(&self) -> gtk::Widget {
        let bar = adw::HeaderBar::new();
        bar.set_title_widget(Some(&adw::WindowTitle::new("TalkBank", "")));

        self.sections.set_selection_mode(gtk::SelectionMode::Single);
        self.sections.add_css_class("navigation-sidebar");
        for (name, icon, label) in Self::SECTIONS {
            let row = gtk::ListBoxRow::new();
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            b.set_margin_start(6);
            b.set_margin_end(6);
            b.set_margin_top(9);
            b.set_margin_bottom(9);
            let _ = label;
            b.append(&gtk::Image::from_icon_name(icon));
            let l = gtk::Label::new(Some(&section_label(name)));
            l.set_xalign(0.0);
            b.append(&l);
            row.set_child(Some(&b));
            unsafe { row.set_data("section", name.to_string()) };
            self.sections.append(&row);
        }

        let this = self.clone();
        self.sections.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let name: String = unsafe { row.data::<String>("section") }
                .map(|p| unsafe { p.as_ref() }.clone())
                .unwrap_or_default();
            this.enter_section(&name);
        });

        // The two context panes sit in the same box as the sections and light up
        // in turn. Swapping them inside a container reparented them on every
        // section change, and a freshly reparented list stops receiving clicks
        // and drops out of the focus chain: it was visible, but unresponsive.
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.append(&self.sections);
        vbox.append(&self.context_sep);
        vbox.append(&self.file_pane());
        vbox.append(&self.command_pane());

        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&bar);
        tv.set_content(Some(&vbox));
        tv.upcast()
    }

    const SECTIONS: [(&'static str, &'static str, &'static str); 4] = [
        ("home", "go-home-symbolic", "Home"),
        ("transcripts", "text-x-generic-symbolic", "Transcripts"),
        ("analysis", "view-list-symbolic", "Analyses"),
        ("archive", "folder-download-symbolic", "Archive"),
    ];

    fn shell_content(&self) -> gtk::Widget {
        // Pages are expensive: they are built on first visit. Especially the
        // archive, which downloads 4.3 MB of catalogue the first time you look.
        self.stack.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("home"));
        self.stack.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("transcripts"));
        self.stack.add_named(&self.content(), Some("analysis"));
        self.stack.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("archive"));
        self.stack.clone().upcast()
    }

    /// Switches to a section, building it if this is the first time.
    pub fn show_section(&self, name: &str) {
        let mut child = self.sections.first_child();
        while let Some(w) = child {
            if let Ok(row) = w.clone().downcast::<gtk::ListBoxRow>() {
                let got: String = unsafe { row.data::<String>("section") }
                    .map(|p| unsafe { p.as_ref() }.clone())
                    .unwrap_or_default();
                if got == name {
                    self.sections.select_row(Some(&row));
                    return;
                }
            }
            child = w.next_sibling();
        }
    }

    fn enter_section(&self, name: &str) {
        self.build_section(name);
        // Coming back to the archive re-checks the session: it may have been
        // signed in from the preferences meanwhile, or have expired.
        if name == "archive" {
            if let Some(a) = self.archive_section.borrow().as_ref() {
                a.recheck_login();
            }
        }
        self.stack.set_visible_child_name(name);
        self.file_pane().set_visible(name == "transcripts");
        self.command_pane().set_visible(name == "analysis");
        self.context_sep
            .set_visible(name == "transcripts" || name == "analysis");
    }

    /// Builds a section's page the first time it is opened.
    fn build_section(&self, name: &str) {
        let Some(page) = self.stack.child_by_name(name) else { return };
        // An empty container is the placeholder `shell_content` put there.
        let empty = page
            .clone()
            .downcast::<gtk::Box>()
            .is_ok_and(|b| b.first_child().is_none());
        if !empty {
            return;
        }
        let built = match name {
            "home" => crate::home::page(self),
            "transcripts" => self.editor().widget(),
            "archive" => {
                let (w, sezione) = crate::archive::page(self);
                *self.archive_section.borrow_mut() = Some(sezione);
                w
            }
            _ => return,
        };
        self.stack.remove(&page);
        let icon = Self::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, i, _)| *i)
            .unwrap_or("view-list-symbolic");
        self.stack
            .add_titled_with_icon(&built, Some(name), &section_label(name), icon);
    }

    /// The download manager, built on first request.
    pub fn downloads(&self) -> crate::downloads::Manager {
        if let Some(d) = self.downloads.borrow().clone() {
            return d;
        }
        let d = crate::downloads::Manager::new(self);
        *self.downloads.borrow_mut() = Some(d.clone());
        d
    }

    /// Asks whether to close with downloads still pending.
    fn confirm_close_with_downloads(&self, how_many: usize) {
        let dlg = adw::AlertDialog::new(
            Some(
                &tn(
                    "%u download is still running.",
                    "%u downloads are still running.",
                    how_many as u32,
                )
                .replace("%u", &how_many.to_string()),
            ),
            Some(&t("Closing now stops them. Nothing half-downloaded is left behind.")),
        );
        dlg.add_response("cancel", &t("Cancel"));
        dlg.add_response("close", &t("Close anyway"));
        dlg.set_response_appearance("close", adw::ResponseAppearance::Destructive);
        dlg.set_close_response("cancel");
        let win = self.win.clone();
        let this = self.clone();
        dlg.choose(Some(&self.win), gio::Cancellable::NONE, move |resp| {
            if resp == "close" {
                if let Some(d) = this.downloads.borrow().as_ref() {
                    d.cancel_all();
                }
                win.destroy();
            }
        });
    }

    /// Invites the user to sign in. It lives here rather than in the archive
    /// because a download started from another section can ask for it too.
    pub fn ask_to_sign_in(&self) {
        let toast = adw::Toast::new(&t(
            "Sign in to download. Nearly all TalkBank data needs a free account.",
        ));
        toast.set_button_label(Some(&t("Preferences")));
        toast.set_timeout(8);
        let win = self.win.clone();
        toast.connect_button_clicked(move |_| {
            let _ = WidgetExt::activate_action(&win, "app.preferences", None);
        });
        self.toasts.add_toast(toast);
    }

    /// Fa ricontrollare l'accesso alla sezione dell'archivio, se esiste.
    pub fn recheck_archive_login(&self) {
        if let Some(a) = self.archive_section.borrow().as_ref() {
            a.recheck_login();
        }
    }

    /// The section currently on screen. Used to decide whether a
    /// download-finished notification is information or noise.
    pub fn visible_section(&self) -> String {
        self.stack.visible_child_name().map(|s| s.to_string()).unwrap_or_default()
    }

    /// Points the analyses at a freshly downloaded folder. Nothing changes
    /// behind the user's back: this is only called when they ask for it.
    pub fn open_downloaded(&self, dest: &Path) {
        let target = crate::archive::analysis_folder(dest);
        self.set_workdir(&target);
        self.show_section("transcripts");
        self.win.present();
    }

    /// The editor, built on first request.
    pub fn editor(&self) -> crate::editor::Editor {
        if let Some(e) = self.editor.borrow().clone() {
            return e;
        }
        let e = crate::editor::new(self);
        *self.editor.borrow_mut() = Some(e.clone());
        e
    }

    fn file_pane(&self) -> gtk::Widget {
        if let Some(w) = self.file_pane.borrow().clone() {
            return w;
        }
        let w = self.build_file_pane();
        *self.file_pane.borrow_mut() = Some(w.clone());
        w
    }

    /// The folder's file list, for opening them in the editor.
    fn build_file_pane(&self) -> gtk::Widget {
        self.file_list.set_selection_mode(gtk::SelectionMode::Single);
        self.file_list.add_css_class("navigation-sidebar");
        self.refresh_file_list();

        let this = self.clone();
        // `row-selected` and not `row-activated`: in a list a single click only
        // selects, and whoever clicks a filename expects it to open. The
        // `filling` guard covers the one case where the selection changes
        // without anyone asking for it.
        self.file_list.connect_row_selected(move |_, row| {
            if this.filling.get() {
                return;
            }
            let Some(row) = row else { return };
            let name: String = unsafe { row.data::<String>("file") }
                .map(|p| unsafe { p.as_ref() }.clone())
                .unwrap_or_default();
            if name.is_empty() {
                return;
            }
            let path = this.workdir.borrow().join(&name);
            this.editor().open(&path);
        });

        self.file_list.set_vexpand(true);
        gtk::ScrolledWindow::builder()
            .child(&self.file_list)
            .vexpand(true)
            .build()
            .upcast()
    }

    pub fn refresh_file_list(&self) {
        self.filling.set(true);
        self.do_refresh_file_list();
        self.filling.set(false);
    }

    fn do_refresh_file_list(&self) {
        while let Some(row) = self.file_list.first_child() {
            self.file_list.remove(&row);
        }
        let files: Vec<String> = self
            .an
            .borrow()
            .files
            .iter()
            .filter(|n| is_transcript(n))
            .cloned()
            .collect();
        if files.is_empty() {
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            row.set_activatable(false);
            let l = gtk::Label::new(Some(&t("No transcript in this folder")));
            l.add_css_class("dim-label");
            l.set_wrap(true);
            l.set_margin_start(12);
            l.set_margin_end(12);
            l.set_margin_top(12);
            l.set_margin_bottom(12);
            row.set_child(Some(&l));
            self.file_list.append(&row);
            return;
        }
        for name in files {
            let row = gtk::ListBoxRow::new();
            let l = gtk::Label::new(Some(&name));
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            l.set_margin_start(12);
            l.set_margin_end(12);
            l.set_margin_top(8);
            l.set_margin_bottom(8);
            row.set_child(Some(&l));
            unsafe { row.set_data("file", name) };
            self.file_list.append(&row);
        }
    }

    /// The commands sidebar. Built once and reused: recreating it on every
    /// section change would lose the search text and the selection.
    fn command_pane(&self) -> gtk::Widget {
        if let Some(w) = self.cmd_pane.borrow().clone() {
            return w;
        }
        let w = self.command_list_pane();
        *self.cmd_pane.borrow_mut() = Some(w.clone());
        w
    }

    /// The application menu. Every section puts one in its header bar: it is the
    /// same menu, not three different ones.
    pub fn menu_button(&self) -> gtk::MenuButton {
        let menu = gio::Menu::new();
        menu.append(Some(&t("Recent commands")), Some("app.history"));
        menu.append(Some(&t("Preferences")), Some("app.preferences"));
        menu.append(Some(&t("About TalkBank")), Some("app.about"));
        let mb = gtk::MenuButton::new();
        mb.set_icon_name("open-menu-symbolic");
        mb.set_menu_model(Some(&menu));
        mb
    }

    // ------------------------------------------------------------- barra laterale

    fn command_list_pane(&self) -> gtk::Widget {
        self.search.set_placeholder_text(Some(&t("Search an analysis")));
        self.search.set_margin_start(8);
        self.search.set_margin_end(8);
        self.search.set_margin_bottom(6);

        self.cmd_list.set_selection_mode(gtk::SelectionMode::Single);
        self.cmd_list.add_css_class("navigation-sidebar");
        self.fill_command_list();

        let sw = gtk::ScrolledWindow::builder()
            .child(&self.cmd_list)
            .vexpand(true)
            .build();
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.set_margin_top(6);
        vbox.append(&self.search);
        vbox.append(&sw);

        let this = self.clone();
        self.search.connect_search_changed(move |_| {
            this.cmd_list.invalidate_filter();
        });
        let this = self.clone();
        self.cmd_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            if let Some(task) = row_task(row) {
                this.select_task(task);
                return;
            }
            let name: String = unsafe { row.data::<String>("cmd") }
                .map(|p| unsafe { p.as_ref() }.clone())
                .unwrap_or_default();
            this.select_command(catalog::find(&name));
        });

        vbox.upcast()
    }

    fn fill_command_list(&self) {
        for cmd in catalog::COMMANDS {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 1);
            b.set_margin_start(12);
            b.set_margin_end(12);
            b.set_margin_top(7);
            b.set_margin_bottom(7);

            let title = gtk::Label::new(Some(&t(cmd.title)));
            title.set_halign(gtk::Align::Start);
            title.set_wrap(true);
            title.set_xalign(0.0);
            let sub = gtk::Label::new(Some(cmd.name));
            sub.set_halign(gtk::Align::Start);
            for c in ["caption", "dim-label", "monospace"] {
                sub.add_css_class(c);
            }
            b.append(&title);
            b.append(&sub);

            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&b));
            unsafe { row.set_data("cmd", cmd.name.to_string()) };
            self.cmd_list.append(&row);
        }

        // Batchalign tasks live in the same sidebar but go through a different
        // engine: we mark them so they are not confused with the CLAN commands.
        for task in crate::batchalign::TASKS {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 1);
            b.set_margin_start(12);
            b.set_margin_end(12);
            b.set_margin_top(7);
            b.set_margin_bottom(7);
            let title = gtk::Label::new(Some(&t(task.title)));
            title.set_halign(gtk::Align::Start);
            title.set_wrap(true);
            title.set_xalign(0.0);
            let sub = gtk::Label::new(Some(&format!("batchalign {}", task.command.as_str())));
            sub.set_halign(gtk::Align::Start);
            for c in ["caption", "dim-label", "monospace"] {
                sub.add_css_class(c);
            }
            b.append(&title);
            b.append(&sub);

            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&b));
            unsafe { row.set_data("ba", task.command.as_str().to_string()) };
            self.cmd_list.append(&row);
        }

        let this = self.clone();
        self.cmd_list.set_filter_func(move |row| {
            let q = this.search.text().to_lowercase();
            if q.is_empty() {
                return true;
            }
            if let Some(task) = row_task(row) {
                let hay = format!("{} {} {}", task.command.as_str(), t(task.title), t(task.desc))
                    .to_lowercase();
                return hay.contains(&q);
            }
            let Some(cmd) = row_command(row) else { return false };
            let hay = format!(
                "{} {} {} {}",
                cmd.name,
                t(cmd.title),
                t(cmd.desc),
                t(cmd.cat.label())
            )
            .to_lowercase();
            hay.contains(&q)
        });

        self.cmd_list.set_header_func(|row, before| {
            if row_task(row).is_some() {
                if before.is_some_and(|b| row_task(b).is_some()) {
                    row.set_header(None::<&gtk::Widget>);
                    return;
                }
                let l = gtk::Label::new(Some(&t("Media and transcription")));
                l.add_css_class("heading");
                l.add_css_class("dim-label");
                l.set_halign(gtk::Align::Start);
                l.set_margin_start(12);
                l.set_margin_top(14);
                l.set_margin_bottom(4);
                row.set_header(Some(&l));
                return;
            }
            let Some(cmd) = row_command(row) else { return };
            let same = before.and_then(row_command).is_some_and(|p| p.cat == cmd.cat);
            if same {
                row.set_header(None::<&gtk::Widget>);
                return;
            }
            let l = gtk::Label::new(Some(&t(cmd.cat.label())));
            l.add_css_class("heading");
            l.add_css_class("dim-label");
            l.set_halign(gtk::Align::Start);
            l.set_margin_start(12);
            l.set_margin_top(if before.is_some() { 14 } else { 6 });
            l.set_margin_bottom(4);
            row.set_header(Some(&l));
        });
    }

    fn restore_last_command(&self) {
        let Some(last) = config::with(|c| c.command.clone()) else {
            return;
        };
        let mut child = self.cmd_list.first_child();
        while let Some(w) = child {
            if let Ok(row) = w.clone().downcast::<gtk::ListBoxRow>() {
                if row_command(&row).is_some_and(|c| c.name == last) {
                    self.cmd_list.select_row(Some(&row));
                    break;
                }
            }
            child = w.next_sibling();
        }
    }

    // ---------------------------------------------------------------- contenuto

    fn content(&self) -> gtk::Widget {
        let head = adw::HeaderBar::new();
        head.set_title_widget(Some(&self.title));

        let folder = gtk::Button::from_icon_name("folder-open-symbolic");
        folder.set_tooltip_text(Some(&t("Working folder")));
        let this = self.clone();
        folder.connect_clicked(move |_| this.choose_folder());
        head.pack_start(&folder);

        head.pack_end(&self.menu_button());

        self.banner.set_revealed(false);
        let this = self.clone();
        self.banner.connect_button_clicked(move |_| this.apply_fix());

        let paned = gtk::Paned::new(gtk::Orientation::Vertical);
        paned.set_start_child(Some(&self.page_holder));
        paned.set_end_child(Some(&self.output_pane()));
        paned.set_position(470);
        paned.set_resize_start_child(true);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        paned.set_vexpand(true);

        let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vb.append(&self.banner);
        vb.append(&paned);

        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&head);
        tv.set_content(Some(&vb));
        tv.add_bottom_bar(&self.run_bar());
        tv.upcast()
    }

    fn run_bar(&self) -> gtk::Widget {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("toolbar");

        self.cmdline.set_xalign(0.0);
        self.cmdline.set_selectable(true);
        self.cmdline.add_css_class("monospace");
        self.cmdline.add_css_class("dim-label");
        let sw = self
            .cmdline_box
            .clone()
            .downcast::<gtk::ScrolledWindow>()
            .expect("scrolled");
        sw.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        sw.set_child(Some(&self.cmdline));
        sw.set_hexpand(true);
        bar.append(&sw);

        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.add_css_class("flat");
        copy.set_tooltip_text(Some(&t("Copy the command line")));
        let this = self.clone();
        copy.connect_clicked(move |_| {
            let line = this.an.borrow().command_line();
            this.win.clipboard().set_text(&line);
            this.toast(&t("Command line copied."));
        });
        bar.append(&copy);

        self.spinner.set_visible(false);
        bar.append(&self.spinner);

        self.status.add_css_class("dim-label");
        self.status.add_css_class("caption");
        bar.append(&self.status);

        self.run_btn.set_label(&t("Run"));
        self.run_btn.add_css_class("suggested-action");
        self.run_btn.set_sensitive(false);
        let this = self.clone();
        self.run_btn.connect_clicked(move |_| this.run());
        bar.append(&self.run_btn);

        bar.upcast()
    }

    fn output_pane(&self) -> gtk::Widget {
        let mk_view = |buf: &gtk::TextBuffer| {
            let tv = gtk::TextView::with_buffer(buf);
            tv.set_editable(false);
            tv.set_monospace(true);
            tv.set_left_margin(12);
            tv.set_top_margin(8);
            tv.add_css_class("clanout");
            gtk::ScrolledWindow::builder().child(&tv).build()
        };

        self.out_stack
            .add_titled_with_icon(&mk_view(&self.out_buf), Some("output"), &t("Output"), "view-list-symbolic");
        self.out_stack.add_titled_with_icon(
            &mk_view(&self.err_buf),
            Some("messages"),
            &t("Messages"),
            "dialog-information-symbolic",
        );

        self.created_list.set_selection_mode(gtk::SelectionMode::None);
        self.created_list.add_css_class("boxed-list");
        for m in [12, 12, 12] {
            let _ = m;
        }
        self.created_list.set_margin_start(12);
        self.created_list.set_margin_end(12);
        self.created_list.set_margin_top(12);
        self.created_list.set_valign(gtk::Align::Start);
        let this = self.clone();
        self.created_list.connect_row_activated(move |_, row| {
            if let Some(name) = unsafe { row.data::<String>("name") } {
                this.preview(&unsafe { name.as_ref() }.clone());
            }
        });
        self.out_stack.add_titled_with_icon(
            &gtk::ScrolledWindow::builder().child(&self.created_list).build(),
            Some("created"),
            &t("New files"),
            "folder-symbolic",
        );

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("toolbar");
        let sw = adw::ViewSwitcher::new();
        sw.set_stack(Some(&self.out_stack));
        sw.set_policy(adw::ViewSwitcherPolicy::Wide);
        bar.append(&sw);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        bar.append(&spacer);
        bar.append(&self.status_placeholder());

        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
        b.append(&bar);
        self.out_stack.set_vexpand(true);
        b.append(&self.out_stack);
        b.upcast()
    }

    /// The status lives in the run bar; all that is needed here is a placeholder
    /// to keep the layout symmetric.
    fn status_placeholder(&self) -> gtk::Widget {
        gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast()
    }
}

// -------------------------------------------------------------------- helpers

/// A section's translated name.
///
/// The strings sit here as literals inside `t()`, not in the sections table: the
/// translation extractor reads the source, and a `t(variable)` tells it nothing
/// about which text to translate.
fn section_label(name: &str) -> String {
    match name {
        "home" => t("Home"),
        "transcripts" => t("Transcripts"),
        "analysis" => t("Analyses"),
        "archive" => t("Archive"),
        other => other.to_string(),
    }
}

fn row_task(row: &gtk::ListBoxRow) -> Option<&'static crate::batchalign::Task> {
    let name = unsafe { row.data::<String>("ba") }?;
    let name = unsafe { name.as_ref() }.clone();
    talkbank_batchalign::Command::from_str(&name).and_then(crate::batchalign::find_task)
}

fn row_command(row: &gtk::ListBoxRow) -> Option<&'static Command> {
    let name = unsafe { row.data::<String>("cmd") }?;
    catalog::find(&unsafe { name.as_ref() }.clone())
}

fn find_lib_dir(bin_dir: &Path) -> PathBuf {
    if let Some(d) = std::env::var_os("CLAN_LIB") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return p;
        }
    }
    for rel in ["../lib", "../../lib", "../share/clan/lib"] {
        let p = bin_dir.join(rel);
        if p.is_dir() {
            return p.canonicalize().unwrap_or(p);
        }
    }
    PathBuf::from("lib")
}

fn default_workdir(lib_dir: &Path) -> PathBuf {
    let ex = lib_dir.join("../examples/transcripts");
    if ex.is_dir() {
        return ex.canonicalize().unwrap_or(ex);
    }
    glib::home_dir()
}

pub fn apply_theme() {
    let scheme = match config::with(|c| c.theme.clone()) {
        config::Theme::Light => adw::ColorScheme::ForceLight,
        config::Theme::Dark => adw::ColorScheme::ForceDark,
        config::Theme::System => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

// ------------------------------------------------------------ pagina comando

impl App {
    fn select_command(&self, cmd: Option<&'static Command>) {
        {
            let mut an = self.an.borrow_mut();
            an.cmd = cmd;
            an.opt_sheet = false;
        }
        *self.ba_task.borrow_mut() = None;
        if let Some(c) = cmd {
            config::update(|cfg| cfg.command = Some(c.name.to_string()));
        }
        self.rebuild_content();
    }

    /// Rebuilding the page from scratch is simpler and more reliable than
    /// removing and re-adding individual groups. `AdwPreferencesPage` already
    /// scrolls itself, so it lives inside an `AdwBin`, not a `GtkScrolledWindow`.
    fn rebuild_content(&self) {
        let page = adw::PreferencesPage::new();
        self.page_holder.set_child(Some(&page));
        *self.files_group.borrow_mut() = None;
        *self.who_group.borrow_mut() = None;
        self.who_rows.borrow_mut().clear();

        let Some(cmd) = self.an.borrow().cmd else {
            let sp = adw::StatusPage::new();
            sp.set_icon_name(Some("system-search-symbolic"));
            sp.set_title(&t("Choose an analysis"));
            sp.set_description(Some(&t(
                "Pick one from the list on the left. “Start here” holds the six commands you need most often.",
            )));
            let g = adw::PreferencesGroup::new();
            g.add(&sp);
            page.add(&g);
            self.title.set_title(&t("TalkBank"));
            self.title.set_subtitle("");
            self.update_cmdline();
            return;
        };

        self.title.set_title(&t(cmd.title));
        self.title
            .set_subtitle(&format!("{} · {}", cmd.name, self.workdir.borrow().display()));

        page.add(&self.group_what(cmd));
        page.add(&self.group_files());
        page.add(&self.group_who());
        page.add(&self.group_options(cmd));
        self.update_cmdline();
    }

    fn group_what(&self, cmd: &'static Command) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("What it does"));
        g.set_description(Some(&t(cmd.desc)));

        let row = adw::ActionRow::new();
        row.set_title(&t("Example from the manual"));
        row.set_subtitle(cmd.example);
        row.set_subtitle_selectable(true);
        row.add_css_class("monospace");

        let help = gtk::Button::from_icon_name("help-about-symbolic");
        help.set_valign(gtk::Align::Center);
        help.add_css_class("flat");
        help.set_tooltip_text(Some(&t("All options for this program")));
        let this = self.clone();
        help.connect_clicked(move |_| this.show_usage());
        row.add_suffix(&help);
        g.add(&row);
        g
    }

    fn group_files(&self) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("Files"));
        *self.files_group.borrow_mut() = Some(g.clone());

        let hdr = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let fb = gtk::Button::with_label(&t("Folder…"));
        let this = self.clone();
        fb.connect_clicked(move |_| this.choose_folder());
        let sa = gtk::Button::with_label(&t("Select all"));
        let this = self.clone();
        sa.connect_clicked(move |_| this.toggle_select_all());
        hdr.append(&fb);
        hdr.append(&sa);
        g.set_header_suffix(Some(&hdr));

        let an = self.an.borrow();
        if an.files.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title(&t("This folder has no files"));
            r.set_subtitle(&t("Use the Folder button to go where your transcripts are."));
            g.add(&r);
        }
        for name in &an.files {
            let r = adw::ActionRow::new();
            r.set_title(name);
            if !is_transcript(name) {
                r.set_subtitle(&t("not a transcript"));
            }
            let chk = gtk::CheckButton::new();
            chk.set_valign(gtk::Align::Center);
            chk.set_active(an.sel_files.contains(name));
            let this = self.clone();
            let n = name.clone();
            chk.connect_toggled(move |c| this.on_file_toggled(&n, c.is_active()));
            r.add_prefix(&chk);
            r.set_activatable_widget(Some(&chk));

            let eye = gtk::Button::from_icon_name("document-open-symbolic");
            eye.set_valign(gtk::Align::Center);
            eye.add_css_class("flat");
            eye.set_tooltip_text(Some(&t("Show the file")));
            let this = self.clone();
            let n = name.clone();
            eye.connect_clicked(move |_| this.preview(&n));
            r.add_suffix(&eye);
            g.add(&r);
        }
        drop(an);
        self.update_files_description();
        g
    }

    fn group_who(&self) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("Who to analyse"));
        g.set_description(Some(&t(
            "Read from the @Participants header of the files you selected. Leave everything off to analyse all speakers.",
        )));
        *self.who_group.borrow_mut() = Some(g.clone());
        self.refresh_who_group();
        g
    }

    /// The speakers change with every change of file selection: we repopulate
    /// only this group, so the page does not rebuild under the user's hands.
    fn refresh_who_group(&self) {
        let Some(g) = self.who_group.borrow().clone() else { return };
        for w in self.who_rows.borrow_mut().drain(..) {
            g.remove(&w);
        }
        let an = self.an.borrow();
        if an.speakers.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title(&t("No speaker found yet"));
            r.set_subtitle(&t("Select a CHAT file above and the speakers will appear here."));
            g.add(&r);
            self.who_rows.borrow_mut().push(r.upcast());
            return;
        }
        for sp in &an.speakers {
            let r = adw::ActionRow::new();
            r.set_title(&format!("*{}", sp.code));
            if let Some(role) = &sp.role {
                r.set_subtitle(role);
            }
            let chk = gtk::CheckButton::new();
            chk.set_valign(gtk::Align::Center);
            chk.set_active(an.sel_speakers.contains(&sp.code));
            let this = self.clone();
            let code = sp.code.clone();
            chk.connect_toggled(move |c| {
                {
                    let mut a = this.an.borrow_mut();
                    if c.is_active() {
                        a.sel_speakers.insert(code.clone());
                    } else {
                        a.sel_speakers.remove(&code);
                    }
                }
                this.update_cmdline();
            });
            r.add_prefix(&chk);
            r.set_activatable_widget(Some(&chk));
            g.add(&r);
            self.who_rows.borrow_mut().push(r.upcast());
        }
    }

    fn group_options(&self, cmd: &'static Command) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::new();
        g.set_title(&t("Options"));

        if cmd.req.takes_language() {
            let langs = state::languages_for(&self.lib_dir, cmd);
            let model = gtk::StringList::new(&[]);
            let optional = cmd.req.has(Req::LANG_OPT);
            if optional {
                model.append("—");
            }
            let current = self.an.borrow().lang.clone();
            let mut selected = 0u32;
            for (i, l) in langs.iter().enumerate() {
                model.append(l);
                if current.as_deref() == Some(l.as_str()) {
                    selected = i as u32 + u32::from(optional);
                }
            }
            let row = adw::ComboRow::new();
            row.set_title(&t("Language of the transcript"));
            row.set_subtitle(&t("Rule set this analysis needs (+l)"));
            row.set_model(Some(&model));
            row.set_selected(selected);
            let this = self.clone();
            let model2 = model.clone();
            row.connect_selected_notify(move |r| {
                let v = model2.string(r.selected()).map(|s| s.to_string());
                this.an.borrow_mut().lang = v.filter(|s| s != "—");
                this.update_cmdline();
            });
            g.add(&row);
        }

        let word = adw::EntryRow::new();
        word.set_title(&t("Word to search for (+s)"));
        word.set_text(&self.an.borrow().word);
        let this = self.clone();
        word.connect_changed(move |e| {
            this.an.borrow_mut().word = e.text().to_string();
            this.update_cmdline();
        });
        g.add(&word);

        let tiers = adw::ExpanderRow::new();
        tiers.set_title(&t("Extra tiers to include"));
        tiers.set_subtitle(&t(
            "Annotation lines below each utterance: %mor is morphology, %gra syntax",
        ));
        for tier in TIERS {
            let r = adw::ActionRow::new();
            r.set_title(tier);
            let chk = gtk::CheckButton::new();
            chk.set_valign(gtk::Align::Center);
            chk.set_active(self.an.borrow().sel_tiers.contains(*tier));
            let this = self.clone();
            let name = tier.to_string();
            chk.connect_toggled(move |c| {
                {
                    let mut a = this.an.borrow_mut();
                    if c.is_active() {
                        a.sel_tiers.insert(name.clone());
                    } else {
                        a.sel_tiers.remove(&name);
                    }
                }
                this.update_cmdline();
            });
            r.add_prefix(&chk);
            r.set_activatable_widget(Some(&chk));
            tiers.add_row(&r);
        }
        g.add(&tiers);

        let switches: Vec<(String, String, fn(&mut Analysis) -> &mut bool)> = vec![
            (
                t("Include repetitions and retracings"),
                t("CLAN leaves them out by default (+r6)"),
                |a| &mut a.opt_repetitions,
            ),
            (t("Ignore upper and lower case"), "+k".into(), |a| &mut a.opt_case),
            (
                t("Treat all files as one corpus"),
                t("One combined result instead of one per file (+u)"),
                |a| &mut a.opt_merge,
            ),
            (t("Include subfolders"), "+re".into(), |a| &mut a.opt_recursive),
            (
                t("Also save the result to a file"),
                t("Creates a .cex file next to your data (+f)"),
                |a| &mut a.opt_save,
            ),
        ];
        for (title, sub, field) in switches {
            let r = adw::SwitchRow::new();
            r.set_title(&title);
            r.set_subtitle(&sub);
            r.set_active(*field(&mut self.an.borrow_mut()));
            let this = self.clone();
            r.connect_active_notify(move |s| {
                *field(&mut this.an.borrow_mut()) = s.is_active();
                this.update_cmdline();
            });
            g.add(&r);
        }

        if let Some(flag) = cmd.sheet_flag {
            let r = adw::SwitchRow::new();
            r.set_title(&t("Spreadsheet layout"));
            r.set_subtitle(flag);
            r.set_active(self.an.borrow().opt_sheet);
            let this = self.clone();
            r.connect_active_notify(move |s| {
                this.an.borrow_mut().opt_sheet = s.is_active();
                this.update_cmdline();
            });
            g.add(&r);
        }

        let extra = adw::EntryRow::new();
        extra.set_title(&t("Other options, typed"));
        extra.set_text(&self.an.borrow().extra);
        let this = self.clone();
        extra.connect_changed(move |e| {
            this.an.borrow_mut().extra = e.text().to_string();
            this.update_cmdline();
        });
        g.add(&extra);

        g
    }
}

// -------------------------------------------------------------- file e stato

impl App {
    fn refresh_files(&self) {
        let dir = self.workdir.borrow().clone();
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        files.sort();
        let mut an = self.an.borrow_mut();
        an.sel_files.retain(|f| files.contains(f));
        an.files = files;
    }

    /// Reads the speakers and the presence of `%mor` from the chosen files:
    /// those are the two facts the whole pre-flight check rests on.
    fn refresh_speakers(&self) {
        let dir = self.workdir.borrow().clone();
        let selected = self.an.borrow().selected_files();

        let mut speakers: Vec<chat::Speaker> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut langs: BTreeSet<String> = BTreeSet::new();
        let mut has_mor = false;

        for name in &selected {
            let info = chat::inspect(&dir.join(name));
            has_mor |= info.has_mor;
            langs.extend(info.languages.iter().cloned());
            for sp in info.speakers {
                if seen.insert(sp.code.clone()) {
                    speakers.push(sp);
                }
            }
        }

        let mut an = self.an.borrow_mut();
        an.sel_speakers.retain(|c| seen.contains(c));
        an.speakers = speakers;
        an.files_have_mor = has_mor;
        an.file_languages = langs.into_iter().collect();
    }

    fn on_file_toggled(&self, name: &str, active: bool) {
        {
            let mut an = self.an.borrow_mut();
            if active {
                an.sel_files.insert(name.to_string());
            } else {
                an.sel_files.remove(name);
            }
        }
        self.refresh_speakers();
        self.refresh_who_group();
        self.update_cmdline();
    }

    fn toggle_select_all(&self) {
        {
            let mut an = self.an.borrow_mut();
            if an.sel_files.is_empty() {
                an.sel_files = an.files.iter().cloned().collect();
            } else {
                an.sel_files.clear();
            }
        }
        self.refresh_speakers();
        self.rebuild_content();
    }

    fn update_files_description(&self) {
        let Some(g) = self.files_group.borrow().clone() else { return };
        let n = self.an.borrow().sel_files.len() as u32;
        g.set_description(Some(&tn("%u file selected", "%u files selected", n).replace("%u", &n.to_string())));
    }

    fn update_cmdline(&self) {
        let line = self.an.borrow().command_line();
        self.cmdline.set_text(&line);
        self.cmdline_box
            .set_visible(config::with(|c| c.show_command));
        self.update_files_description();
        self.update_run_state();
    }

    fn update_run_state(&self) {
        let warn = config::with(|c| c.preflight);
        let (pf, running, has_cmd) = {
            let an = self.an.borrow();
            (an.preflight(warn), an.running, an.cmd.is_some())
        };
        self.run_btn
            .set_sensitive(has_cmd && !pf.blocks() && !running);
        self.show_preflight(&pf);
    }

    fn show_preflight(&self, pf: &Preflight) {
        let name = self.an.borrow().cmd.map(|c| c.name).unwrap_or("");
        let (msg, fix) = match pf {
            Preflight::Ok => {
                self.banner.set_revealed(false);
                *self.banner_fix.borrow_mut() = None;
                return;
            }
            Preflight::NoFiles => (t("Choose at least one file to analyse."), None),
            Preflight::NeedsSpeaker => (
                t("%s needs to know which speaker to analyse. Pick one under “Who to analyse”.")
                    .replace("%s", name),
                None,
            ),
            Preflight::NeedsLanguage => (
                t("%s needs the language of the transcript.").replace("%s", name),
                None,
            ),
            Preflight::MissingMor => (
                t("These files have no %mor tier, which this analysis reads. Run “Morphological analysis” (mor) first, or work on the words of the main tier instead."),
                Some(t("What can I do?")),
            ),
        };
        self.banner.set_title(&msg);
        self.banner.set_button_label(fix.as_deref());
        *self.banner_fix.borrow_mut() = Some(pf.clone());
        self.banner.set_revealed(true);
    }

    fn apply_fix(&self) {
        if !matches!(*self.banner_fix.borrow(), Some(Preflight::MissingMor)) {
            return;
        }
        self.show_mor_remedies();
    }

    /// The three ways out of a missing `%mor`, with the one that suits the files'
    /// language put first.
    ///
    /// The classic MOR grammars only cover English, French, Spanish and Chinese:
    /// for other languages offering `mor` would be a dead end, and that is where
    /// Batchalign comes in.
    fn show_mor_remedies(&self) {
        const MOR_LANGS: [&str; 4] = ["eng", "fra", "spa", "zho"];
        let langs = self.an.borrow().file_languages.clone();
        let mor_covers = langs.is_empty()
            || langs
                .iter()
                .any(|l| MOR_LANGS.iter().any(|m| l.starts_with(m)));

        let dialog = adw::AlertDialog::new(
            Some(&t("This analysis needs the %mor tier")),
            Some(&t(
                "The %mor tier holds the morphological analysis of every word. Three ways forward:",
            )),
        );
        dialog.add_response("words", &t("Count words instead"));
        dialog.set_response_appearance("words", adw::ResponseAppearance::Suggested);

        if mor_covers {
            dialog.add_response("mor", &t("Create it with MOR"));
        } else {
            // Say why MOR is not an option rather than just omitting it: anyone
            // who knows CLAN would wonder where it went.
            dialog.set_body(&format!(
                "{}\n\n{}",
                t("The %mor tier holds the morphological analysis of every word. Three ways forward:"),
                t("The classic MOR grammars only cover English, French, Spanish and Chinese, so for these files they are not an option.")
                    .to_string()
            ));
        }
        dialog.add_response("batchalign", &t("Create it with Batchalign (UD)"));
        dialog.add_response("cancel", &t("Cancel"));
        dialog.set_close_response("cancel");

        let this = self.clone();
        dialog.connect_response(None, move |_, response| match response {
            "words" => {
                state::add_words_fallback(&mut this.an.borrow_mut().extra);
                this.rebuild_content();
                this.toast(&t("Added -t%mor: the analysis will count words on the main tier."));
            }
            "mor" => this.select_named("mor"),
            "batchalign" => this.select_task_named(talkbank_batchalign::Command::Morphotag),
            _ => {}
        });
        dialog.present(Some(&self.win));
    }

    /// Moves the selection to a CLAN command, updating the sidebar too: changing
    /// the page without changing the highlighted row would leave the interface
    /// saying two different things.
    fn select_named(&self, name: &str) {
        let mut child = self.cmd_list.first_child();
        while let Some(w) = child {
            if let Ok(row) = w.clone().downcast::<gtk::ListBoxRow>() {
                if row_command(&row).is_some_and(|c| c.name == name) {
                    self.cmd_list.select_row(Some(&row));
                    return;
                }
            }
            child = w.next_sibling();
        }
    }

    fn select_task_named(&self, cmd: talkbank_batchalign::Command) {
        let mut child = self.cmd_list.first_child();
        while let Some(w) = child {
            if let Ok(row) = w.clone().downcast::<gtk::ListBoxRow>() {
                if row_task(&row).is_some_and(|t| t.command == cmd) {
                    self.cmd_list.select_row(Some(&row));
                    return;
                }
            }
            child = w.next_sibling();
        }
    }

    pub fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    fn apply_font(&self) {
        let size = config::with(|c| c.font_size);
        self.css.load_from_string(&format!(
            "textview.clanout {{ font-family: monospace; font-size: {size}pt; }}"
        ));
    }
}

// ------------------------------------------------------------------ esecuzione

impl App {
    fn run(&self) {
        let (cmd, args) = {
            let an = self.an.borrow();
            if an.running {
                return;
            }
            let Some(cmd) = an.cmd else { return };
            (cmd, an.args())
        };

        {
            let mut an = self.an.borrow_mut();
            let line = an.command_line();
            an.history.push(line);
            an.running = true;
        }
        self.run_btn.set_sensitive(false);
        self.spinner.set_visible(true);
        self.status.set_text(&t("running…"));
        self.banner.set_revealed(false);

        let bin = self.bin_dir.clone();
        let cwd = self.workdir.borrow().clone();
        let name = cmd.name.to_string();
        let (tx, rx) = async_channel::bounded(1);

        gio::spawn_blocking(move || {
            let out = runner::run(&bin, &name, &args, &cwd);
            let _ = tx.send_blocking(out);
        });

        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = rx.recv().await {
                this.on_run_done(result);
            }
        });
    }

    fn on_run_done(&self, result: Result<runner::RunOutput, runner::RunError>) {
        self.an.borrow_mut().running = false;
        self.spinner.set_visible(false);
        self.run_btn.set_sensitive(true);

        let out = match result {
            Ok(o) => o,
            Err(e) => {
                self.status.set_text(&t("failed"));
                self.err_buf.set_text(&e.to_string());
                self.out_stack.set_visible_child_name("messages");
                return;
            }
        };

        self.out_buf.set_text(&out.stdout);
        self.err_buf.set_text(&out.stderr);
        self.fill_created(&out.created);
        self.status
            .set_text(&t("done in %.1f s").replace("%.1f", &format!("{:.1}", out.seconds)));

        // With "+f" the result goes to a file and the Output view stays empty:
        // take the user to where there is actually something to read.
        let page = if !out.stdout.trim().is_empty() {
            "output"
        } else if !out.created.is_empty() {
            "created"
        } else if !out.stderr.trim().is_empty() {
            "messages"
        } else {
            "output"
        };
        self.out_stack.set_visible_child_name(page);

        if !out.created.is_empty() {
            let n = out.created.len() as u32;
            self.toast(
                &tn("%u file created", "%u files created", n).replace("%u", &n.to_string()),
            );
        }

        // Rebuilding clears the banner, so the message has to come afterwards.
        self.refresh_files();
        self.rebuild_content();

        match state::interpret(&out.stdout, &out.stderr, out.exit_code) {
            Hint::None => {}
            hint => self.show_hint(&hint),
        }
    }

    fn show_hint(&self, hint: &Hint) {
        let (msg, fix) = match hint {
            Hint::MissingMor => (
                t("The files have no %mor tier. Run “Morphological analysis” (mor) on them first, or switch to counting the words of the main tier."),
                Some(t("Use words instead")),
            ),
            Hint::NeedsSpeaker => (t("Pick a speaker under “Who to analyse”."), None),
            Hint::NeedsLanguage => (t("Choose the language of the transcript."), None),
            Hint::NotChat => (
                t("This file is not a valid CHAT transcript. Run “Check the file is valid” (check) to see what is wrong."),
                None,
            ),
            Hint::Failed(code) => (
                t("The program stopped with code %d. See the Messages tab.")
                    .replace("%d", &code.to_string()),
                None,
            ),
            Hint::None => return,
        };
        self.banner.set_title(&msg);
        self.banner.set_button_label(fix.as_deref());
        *self.banner_fix.borrow_mut() = matches!(hint, Hint::MissingMor)
            .then_some(Preflight::MissingMor);
        self.banner.set_revealed(true);
    }

    fn fill_created(&self, created: &[String]) {
        while let Some(c) = self.created_list.first_child() {
            self.created_list.remove(&c);
        }
        for name in created {
            let row = adw::ActionRow::new();
            row.set_title(name);
            row.add_prefix(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row.set_activatable(true);
            unsafe { row.set_data("name", name.clone()) };
            self.created_list.append(&row);
        }
    }

    fn preview(&self, name: &str) {
        let path = self.workdir.borrow().join(name);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut text = String::from_utf8_lossy(&bytes).into_owned();
                if text.len() > (1 << 20) {
                    text.truncate(1 << 20);
                }
                self.out_buf.set_text(&text);
                self.out_stack.set_visible_child_name("output");
                self.toast(&t("Showing %s").replace("%s", name));
            }
            Err(_) => self.toast(&t("Could not open the file.")),
        }
    }

    fn show_usage(&self) {
        let Some(cmd) = self.an.borrow().cmd else { return };
        let text = runner::usage(&self.bin_dir, cmd.name)
            .unwrap_or_else(|e| e.to_string());

        let dlg = adw::Dialog::new();
        dlg.set_title(cmd.name);
        dlg.set_content_width(720);
        dlg.set_content_height(620);

        let tv = gtk::TextView::new();
        tv.set_editable(false);
        tv.set_monospace(true);
        tv.set_left_margin(12);
        tv.set_top_margin(12);
        tv.buffer().set_text(&text);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&gtk::ScrolledWindow::builder().child(&tv).build()));
        dlg.set_child(Some(&toolbar));
        dlg.present(Some(&self.win));
    }

    pub fn choose_folder(&self) {
        let dialog = gtk::FileDialog::new();
        dialog.set_title(&t("Choose the folder with your transcripts"));
        dialog.set_initial_folder(Some(&gio::File::for_path(&*self.workdir.borrow())));

        let this = self.clone();
        dialog.select_folder(Some(&self.win), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    this.set_workdir(&path);
                }
            }
        });
    }

    pub fn set_workdir(&self, path: &Path) {
        *self.workdir.borrow_mut() = path.to_path_buf();
        self.an.borrow_mut().sel_files.clear();
        config::update(|c| c.workdir = Some(path.to_path_buf()));
        config::remember_dir(path);
        self.refresh_files();
        self.refresh_file_list();
        self.refresh_speakers();
        self.rebuild_content();
    }

    pub fn history(&self) -> Vec<String> {
        self.an.borrow().history.clone()
    }

    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.win
    }

    pub fn workdir(&self) -> PathBuf {
        self.workdir.borrow().clone()
    }

    pub fn refresh_appearance(&self) {
        self.apply_font();
        apply_theme();
        self.update_cmdline();
    }

    /// Opens a file: changes folder, selects it for the analyses and shows it in
    /// the editor. This is the path taken by both the command line and the
    /// recents list.
    pub fn open_file(&self, path: &Path) {
        let Some(dir) = path.parent() else { return };
        let Some(name) = path.file_name() else { return };
        *self.workdir.borrow_mut() = dir.to_path_buf();
        config::update(|c| c.workdir = Some(dir.to_path_buf()));
        config::remember_dir(dir);
        self.refresh_files();
        self.refresh_file_list();
        self.an
            .borrow_mut()
            .sel_files
            .insert(name.to_string_lossy().into_owned());
        self.refresh_speakers();
        self.rebuild_content();
        self.editor().open(path);
        // Whoever opens a file wants to see it, not the start page.
        self.show_section("transcripts");
    }

    /// Goes to the analyses with *only* this file selected. This is what the
    /// editor's "Analyse this" button needs: whoever is looking at a transcript
    /// wants to analyse that one, not the others ticked half an hour ago.
    pub fn analyse_only(&self, path: &Path) {
        let Some(name) = path.file_name() else { return };
        {
            let mut an = self.an.borrow_mut();
            an.sel_files.clear();
            an.sel_files.insert(name.to_string_lossy().into_owned());
        }
        self.refresh_speakers();
        self.rebuild_content();
        self.show_section("analysis");
    }

    /// After a saved edit: the speakers and the requirements may have changed,
    /// and the analysis page reads them from disk.
    pub fn refresh_after_edit(&self) {
        self.refresh_files();
        self.refresh_file_list();
        self.refresh_speakers();
        self.update_cmdline();
    }

    /// Jumps to the first Batchalign task. The start page offers "Transcribe an
    /// audio file" without asking which of the nine tasks that is.
    pub fn select_first_media_task(&self) {
        // Selecting the row does the rest: the list's signal calls `select_task`.
        // Doing it by hand as well would rebuild the page twice.
        self.select_task_named(talkbank_batchalign::Command::Transcribe);
    }
}

fn is_transcript(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".cha", ".cex", ".cut", ".txt"]
        .iter()
        .any(|e| lower.ends_with(e))
}

impl App {
    pub fn show_toast(&self, toast: adw::Toast) {
        self.toasts.add_toast(toast);
    }

    /// Punto d'ingresso dell'azione `app.run`, legata a Ctrl+Invio.
    pub fn run_from_action(&self) {
        if self.run_btn.is_sensitive() {
            self.run();
        }
    }
}

// ------------------------------------------------- compiti di Batchalign

impl App {
    fn select_task(&self, task: &'static crate::batchalign::Task) {
        *self.ba_task.borrow_mut() = Some(task);
        self.an.borrow_mut().cmd = None;
        self.rebuild_task_page(task);
    }

    fn rebuild_task_page(&self, task: &'static crate::batchalign::Task) {
        self.title.set_title(&t(task.title));
        self.title.set_subtitle(&format!(
            "batchalign {} · {}",
            task.command.as_str(),
            self.workdir.borrow().display()
        ));
        self.banner.set_revealed(false);

        let this = self.clone();
        let page = crate::batchalign::page(task, move |cmd| this.run_task(cmd));
        // Files are chosen as for the analyses: we reuse the same group.
        page.add(&self.group_files());
        self.page_holder.set_child(Some(&page));
        self.update_cmdline();
    }

    fn run_task(&self, cmd: talkbank_batchalign::Command) {
        let dir = self.workdir.borrow().clone();
        let paths: Vec<String> = self
            .an
            .borrow()
            .selected_files()
            .iter()
            .map(|f| dir.join(f).display().to_string())
            .collect();
        if paths.is_empty() {
            self.toast(&t("Choose at least one file to analyse."));
            return;
        }
        if let Some(task) = *self.ba_task.borrow() {
            if crate::batchalign::media_missing(task, &paths) {
                self.toast(&t(
                    "This task needs a recording: select an audio file, or put one next to the transcript.",
                ));
                return;
            }
        }
        let lang = self.an.borrow().file_languages.first().cloned();

        self.spinner.set_visible(true);
        self.status.set_text(&t("running…"));
        let status = self.status.clone();
        let spinner = self.spinner.clone();
        let this = self.clone();

        crate::batchalign::run_task(
            cmd,
            paths,
            lang,
            move |text, _frac| status.set_text(&text),
            move |res| {
                spinner.set_visible(false);
                match res {
                    Ok(info) if crate::batchalign::succeeded(&info) => {
                        this.toast(&t("Batchalign finished."));
                        this.refresh_files();
                        this.update_cmdline();
                    }
                    Ok(info) => this.toast(&talkbank_batchalign::client::describe(&info)),
                    Err(e) => this.toast(&e.to_string()),
                }
            },
        );
    }
}
