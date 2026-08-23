//! The transcript editor.
//!
//! This was the missing piece: CLAN can *analyse* files, `chatter` can *say
//! whether they are valid*, but on Linux neither lets you open one and fix it.
//! Without this, "working on CHAT files" means keeping some general-purpose text
//! editor open next to the app — and a general-purpose editor knows nothing
//! about the format.
//!
//! Three choices govern this module:
//!
//!  * **Colouring is per line, not per grammar.** In CHAT the first character
//!    decides everything: `@` header, `*` speaker, `%` annotation. A full
//!    format-aware highlighter would be a second parser to keep in step with
//!    `chatter`; this one cannot diverge.
//!  * **Validation is `chatter`'s**, the same one the analysis preview uses: two
//!    different verdicts on the same file would be worse than none. It runs on a
//!    delay after the last keystroke, not on every key.
//!  * **Nothing is lost silently.** Switching files with unsaved changes asks
//!    first, and saving goes through a temporary file.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::config;
use crate::i18n::{t, tn};
use crate::window::App;

/// How long to wait after the last keystroke before re-validating. Long enough
/// not to re-parse mid-word, short enough to feel immediate.
const VALIDATE_DELAY_MS: u32 = 600;

pub struct Inner {
    app: App,
    view: gtk::TextView,
    buffer: gtk::TextBuffer,
    title: adw::WindowTitle,
    save_btn: gtk::Button,
    problems: gtk::ListBox,
    problems_stack: adw::ViewStack,
    /// Blank sheet or text. It lives here rather than in `widget` because a file
    /// can be opened (from the command line, or from the recents) before the
    /// l'impaginazione esista.
    text_stack: adw::ViewStack,
    summary: gtk::Label,
    path: RefCell<Option<PathBuf>>,
    dirty: Cell<bool>,
    /// True while loading: programmatic changes must not count as user edits nor
    /// restart validation.
    loading: Cell<bool>,
    pending: RefCell<Option<glib::SourceId>>,
    /// Built with the header, and so after `Inner`: kept here because
    /// `load` deve poterlo mostrare.
    analyse_btn: RefCell<Option<gtk::Button>>,
}

#[derive(Clone)]
pub struct Editor(Rc<Inner>);

impl std::ops::Deref for Editor {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

pub fn new(app: &App) -> Editor {
    let buffer = gtk::TextBuffer::new(None);
    let view = gtk::TextView::with_buffer(&buffer);
    view.set_monospace(true);
    view.set_left_margin(14);
    view.set_right_margin(14);
    view.set_top_margin(10);
    view.set_bottom_margin(10);
    view.set_wrap_mode(gtk::WrapMode::WordChar);

    let e = Editor(Rc::new(Inner {
        app: app.clone(),
        view,
        buffer,
        title: adw::WindowTitle::new(&t("Transcripts"), ""),
        save_btn: gtk::Button::with_label(&t("Save")),
        problems: gtk::ListBox::new(),
        problems_stack: adw::ViewStack::new(),
        text_stack: adw::ViewStack::new(),
        summary: gtk::Label::new(None),
        path: RefCell::new(None),
        dirty: Cell::new(false),
        loading: Cell::new(false),
        pending: RefCell::new(None),
        analyse_btn: RefCell::new(None),
    }));
    e.install_tags();
    e.build_stacks();

    let this = e.clone();
    e.buffer.connect_changed(move |_| {
        if this.loading.get() {
            return;
        }
        this.mark_dirty(true);
        this.schedule_check();
    });

    let this = e.clone();
    e.save_btn.connect_clicked(move |_| {
        this.save();
    });
    e.save_btn.add_css_class("suggested-action");
    // With no file open there is nothing to save and nothing to analyse: the two
    // buttons appear with the file rather than sitting there greyed out.
    e.save_btn.set_visible(false);
    e
}

impl Editor {
    /// Fills the two stacks. Kept apart from `widget` because order matters: a
    /// file can be opened before the layout exists, and the stack has to know
    /// its pages by then.
    fn build_stacks(&self) {
        // No file open: say what to do, instead of showing a blank sheet.
        let empty = adw::StatusPage::new();
        empty.set_icon_name(Some("text-x-generic-symbolic"));
        empty.set_title(&t("No transcript open"));
        empty.set_description(Some(&t(
            "Pick one from the list on the left, or choose another folder.",
        )));
        self.text_stack.add_named(&empty, Some("empty"));
        self.text_stack.add_named(
            &gtk::ScrolledWindow::builder().child(&self.view).build(),
            Some("text"),
        );
        self.text_stack.set_visible_child_name("empty");
        self.text_stack.set_vexpand(true);

        self.problems.set_selection_mode(gtk::SelectionMode::None);
        self.problems.add_css_class("boxed-list");
        self.problems.set_margin_start(12);
        self.problems.set_margin_end(12);
        self.problems.set_margin_top(12);
        self.problems.set_margin_bottom(12);
        self.problems.set_valign(gtk::Align::Start);

        let clean = adw::StatusPage::new();
        clean.set_icon_name(Some("object-select-symbolic"));
        clean.set_title(&t("The format is correct"));
        self.problems_stack.add_named(&clean, Some("clean"));
        self.problems_stack.add_named(
            &gtk::ScrolledWindow::builder().child(&self.problems).build(),
            Some("list"),
        );
        self.problems_stack.set_visible_child_name("clean");
        self.problems_stack.set_vexpand(true);
    }

    // -------------------------------------------------------------- aspetto

    /// The colour tags. Defined once and reused: creating them on every recolour
    /// would fill the buffer's tag table with duplicates.
    fn install_tags(&self) {
        let tags = self.buffer.tag_table();
        let mk = |name: &str, f: &dyn Fn(&gtk::TextTag)| {
            let tag = gtk::TextTag::builder().name(name).build();
            f(&tag);
            tags.add(&tag);
        };
        // The header is structure: bold, like a title.
        mk("header", &|t| {
            t.set_weight(700);
        });
        // The speaker line is what you actually read: it keeps the text colour.
        mk("speaker", &|t| {
            t.set_weight(600);
        });
        // Annotations are scaffolding: dimmed, so they do not bury the speech.
        mk("tier", &|t| {
            t.set_foreground(Some("#8b8b8b"));
        });
        mk("problem", &|t| {
            t.set_underline(gtk::pango::Underline::Error);
        });
    }

    /// Recolours the whole buffer. In CHAT the line type is read off the first
    /// character, so a single pass over the lines is enough.
    fn recolour(&self) {
        let b = &self.buffer;
        let (start, end) = b.bounds();
        for name in ["header", "speaker", "tier"] {
            b.remove_tag_by_name(name, &start, &end);
        }
        let text = b.text(&start, &end, false).to_string();
        for (i, line) in text.lines().enumerate() {
            let tag = match line.chars().next() {
                Some('@') => "header",
                Some('*') => "speaker",
                Some('%') => "tier",
                _ => continue,
            };
            let Some(ls) = b.iter_at_line(i as i32) else { continue };
            let mut le = ls;
            if !le.ends_line() {
                le.forward_to_line_end();
            }
            b.apply_tag_by_name(tag, &ls, &le);
        }
    }

    // ------------------------------------------------------------- apertura

    /// Opens a file. If one has unsaved changes, it asks first.
    pub fn open(&self, path: &Path) {
        if self.dirty.get() {
            self.ask_then(path.to_path_buf());
            return;
        }
        self.load(path);
    }

    fn load(&self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                // Some historic transcripts are not UTF-8. Reading the bytes and
                // converting with replacement beats refusing them: they can be
                // seen, and saving stays blocked until the user decides.
                match std::fs::read(path) {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(_) => {
                        self.app.toast(&t("Could not open %s: %e")
                            .replace("%s", &path.display().to_string())
                            .replace("%e", &e.to_string()));
                        return;
                    }
                }
            }
        };

        self.loading.set(true);
        self.buffer.set_text(&text);
        self.loading.set(false);
        *self.path.borrow_mut() = Some(path.to_path_buf());
        self.text_stack.set_visible_child_name("text");
        self.mark_dirty(false);
        self.recolour();

        self.title.set_title(&path.file_name().unwrap_or_default().to_string_lossy());
        self.title.set_subtitle(&crate::home::shorten(path.parent().unwrap_or(path)));
        config::remember_file(path);
        if let Some(b) = self.analyse_btn.borrow().as_ref() {
            b.set_visible(true);
        }
        self.check_now();
    }

    fn ask_then(&self, next: PathBuf) {
        let dlg = adw::AlertDialog::new(
            Some(&t("Save the changes?")),
            Some(&t("The current transcript has unsaved changes.")),
        );
        dlg.add_response("cancel", &t("Cancel"));
        dlg.add_response("discard", &t("Discard"));
        dlg.add_response("save", &t("Save"));
        dlg.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dlg.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dlg.set_default_response(Some("save"));
        dlg.set_close_response("cancel");

        let this = self.clone();
        dlg.choose(Some(&self.app.window().clone()), gtk::gio::Cancellable::NONE, move |resp| {
            match resp.as_str() {
                "save" => {
                    if this.save() {
                        this.load(&next);
                    }
                }
                "discard" => {
                    this.mark_dirty(false);
                    this.load(&next);
                }
                _ => {}
            }
        });
    }

    // ---------------------------------------------------------- salvataggio

    /// Saves. Returns `false` when it could not.
    pub fn save(&self) -> bool {
        let Some(path) = self.path.borrow().clone() else {
            return false;
        };
        let (s, e) = self.buffer.bounds();
        let text = self.buffer.text(&s, &e, true).to_string();

        // Write beside it and rename: an interruption halfway leaves the file as
        // it was, not half written.
        let tmp = path.with_extension("cha.tmp");
        if let Err(err) = std::fs::write(&tmp, text.as_bytes()) {
            self.app.toast(&t("Could not save: %e").replace("%e", &err.to_string()));
            return false;
        }
        if let Err(err) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            self.app.toast(&t("Could not save: %e").replace("%e", &err.to_string()));
            return false;
        }
        self.mark_dirty(false);
        self.app.toast(&t("Saved."));
        // The rest of the app looks at the files on disk: after a save the
        // speakers and the requirements may have changed.
        self.app.refresh_after_edit();
        true
    }

    fn mark_dirty(&self, dirty: bool) {
        self.dirty.set(dirty);
        self.save_btn.set_visible(dirty);
        if let Some(p) = self.path.borrow().as_ref() {
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            self.title
                .set_title(&if dirty { format!("• {name}") } else { name });
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    /// Asks what to do with the changes and then closes. The caller has already
    /// stopped the close: here it is resumed, or cancelled.
    pub fn confirm_then_close(&self) {
        let dlg = adw::AlertDialog::new(
            Some(&t("Save before closing?")),
            Some(&t("The current transcript has unsaved changes.")),
        );
        dlg.add_response("cancel", &t("Cancel"));
        dlg.add_response("discard", &t("Close without saving"));
        dlg.add_response("save", &t("Save and close"));
        dlg.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dlg.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dlg.set_default_response(Some("save"));
        dlg.set_close_response("cancel");

        let this = self.clone();
        dlg.choose(Some(&self.app.window().clone()), gtk::gio::Cancellable::NONE, move |resp| {
            match resp.as_str() {
                "save" if !this.save() => {}
                "cancel" => {}
                _ => {
                    // The close handler looks at `dirty`: clearing it makes it
                    // passare senza richiedere di nuovo.
                    this.mark_dirty(false);
                    this.app.window().close();
                }
            }
        });
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.path.borrow().clone()
    }

    // ------------------------------------------------------------ verifica

    fn schedule_check(&self) {
        if let Some(id) = self.pending.borrow_mut().take() {
            id.remove();
        }
        let this = self.clone();
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(VALIDATE_DELAY_MS as u64),
            move || {
                *this.pending.borrow_mut() = None;
                this.check_now();
            },
        );
        *self.pending.borrow_mut() = Some(id);
    }

    fn check_now(&self) {
        let Some(path) = self.path.borrow().clone() else { return };
        let (s, e) = self.buffer.bounds();
        let text = self.buffer.text(&s, &e, true).to_string();
        let v = talkbank_engine::validate::validate_at(&path, &text);

        while let Some(row) = self.problems.first_child() {
            self.problems.remove(&row);
        }
        // Clear the previous underlines before adding new ones: otherwise a
        // corrected error would stay marked.
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag_by_name("problem", &start, &end);

        let errors = v.errors().count();
        let warnings = v.warnings().count();
        self.summary.set_label(&if errors == 0 && warnings == 0 {
            t("No problems · %u utterances").replace("%u", &v.utterance_count.to_string())
        } else {
            let mut bits = Vec::new();
            if errors > 0 {
                bits.push(tn("%u error", "%u errors", errors as u32)
                    .replace("%u", &errors.to_string()));
            }
            if warnings > 0 {
                bits.push(tn("%u warning", "%u warnings", warnings as u32)
                    .replace("%u", &warnings.to_string()));
            }
            bits.join(" · ")
        });

        if v.diagnostics.is_empty() {
            self.problems_stack.set_visible_child_name("clean");
            self.recolour();
            return;
        }
        self.problems_stack.set_visible_child_name("list");

        // Errors first: those are the ones that block the analyses.
        let mut diags = v.diagnostics.clone();
        diags.sort_by_key(|d| (!d.is_error, d.line.unwrap_or(usize::MAX)));

        for d in diags.iter().take(200) {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&d.message));
            row.set_title_lines(0);
            let mut sub = vec![d.code.clone()];
            if let Some(l) = d.line {
                sub.push(t("line %u").replace("%u", &l.to_string()));
            }
            if let Some(s) = &d.suggestion {
                sub.push(s.clone());
            }
            row.set_subtitle(&glib::markup_escape_text(&sub.join(" · ")));
            row.set_subtitle_lines(0);
            row.add_prefix(&gtk::Image::from_icon_name(if d.is_error {
                "dialog-error-symbolic"
            } else {
                "dialog-warning-symbolic"
            }));
            if let Some(line) = d.line {
                row.set_activatable(true);
                let this = self.clone();
                row.connect_activated(move |_| this.goto_line(line));
                if d.is_error {
                    self.underline(line);
                }
            }
            self.problems.append(&row);
        }
        self.recolour();
    }

    fn underline(&self, line: usize) {
        let Some(ls) = self.buffer.iter_at_line(line as i32 - 1) else { return };
        let mut le = ls;
        if !le.ends_line() {
            le.forward_to_line_end();
        }
        self.buffer.apply_tag_by_name("problem", &ls, &le);
    }

    fn goto_line(&self, line: usize) {
        let Some(iter) = self.buffer.iter_at_line(line as i32 - 1) else { return };
        self.buffer.place_cursor(&iter);
        self.view
            .scroll_to_iter(&mut iter.clone(), 0.1, true, 0.0, 0.3);
        self.view.grab_focus();
    }

    // ------------------------------------------------------------- widget

    pub fn widget(&self) -> gtk::Widget {
        let head = adw::HeaderBar::new();
        head.set_title_widget(Some(&self.title));
        head.pack_end(&self.app.menu_button());
        head.pack_end(&self.save_btn);

        let folder = gtk::Button::from_icon_name("folder-open-symbolic");
        folder.set_tooltip_text(Some(&t("Choose the working folder")));
        let a = self.app.clone();
        folder.connect_clicked(move |_| a.choose_folder());
        head.pack_start(&folder);

        let analizza = gtk::Button::with_label(&t("Analyse this"));
        analizza.set_tooltip_text(Some(&t("Go to the analyses with this file selected")));
        // A file may already be open: if so, the button starts out visible.
        analizza.set_visible(self.path.borrow().is_some());
        let this = self.clone();
        analizza.connect_clicked(move |_| {
            if let Some(p) = this.path() {
                this.app.analyse_only(&p);
            }
        });
        head.pack_start(&analizza);
        *self.analyse_btn.borrow_mut() = Some(analizza);

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("toolbar");
        let lbl = gtk::Label::new(Some(&t("Format check")));
        lbl.add_css_class("heading");
        bar.append(&lbl);
        self.summary.add_css_class("dim-label");
        self.summary.add_css_class("caption");
        bar.append(&self.summary);

        let problems_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        problems_box.append(&bar);
        self.problems_stack.set_vexpand(true);
        problems_box.append(&self.problems_stack);

        let paned = gtk::Paned::new(gtk::Orientation::Vertical);
        paned.set_start_child(Some(&self.text_stack));
        paned.set_end_child(Some(&problems_box));
        paned.set_position(520);
        paned.set_resize_start_child(true);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);

        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&head);
        tv.set_content(Some(&paned));
        tv.upcast()
    }
}
