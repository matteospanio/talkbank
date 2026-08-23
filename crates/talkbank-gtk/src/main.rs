//! TalkBank — desktop client for CHAT transcripts and the TalkBank archive.

mod archive;
mod batchalign;
mod config;
mod downloads;
mod editor;
mod home;
mod i18n;
mod net;
mod preferences;
mod state;
mod window;

use std::path::PathBuf;

use adw::prelude::*;
use gtk::{gio, glib};

use i18n::t;
use window::App;

const APP_ID: &str = "org.talkbank.TalkBank";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,talkbank=info,talkbank_engine=info,talkbank_archive=info".into()),
        )
        .init();

    // The language has to be picked before gettext is initialised, and gettext
    // before any widget is built: labels are translated as they are constructed.
    let cfg = config::Config::load();
    i18n::init(cfg.language.code());
    config::init(cfg);

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_activate(|app| {
        start(app, Vec::new());
    });
    app.connect_open(|app, files, _| {
        start(app, files.iter().filter_map(|f| f.path()).collect());
    });

    let code = app.run();
    // If we started the Batchalign server ourselves, shut it down: an orphan
    // process holds the port and the memory until the next reboot.
    batchalign::state().shutdown();
    code
}

fn start(app: &adw::Application, paths: Vec<PathBuf>) {
    register_icon();
    // A folder we open; files we open and select. That is what the .desktop file
    // needs, since it passes %f.
    let dir = paths.iter().find(|p| p.is_dir()).cloned();
    let win = App::build(app, dir);
    for f in paths.iter().filter(|p| p.is_file()) {
        win.open_file(f);
    }
    install_actions(app, &win);
}

fn install_actions(app: &adw::Application, win: &App) {
    let add = |name: &str, cb: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| cb());
        app.add_action(&action);
    };

    let w = win.clone();
    add("preferences", Box::new(move || preferences::show(&w)));
    let w = win.clone();
    add("archive", Box::new(move || w.show_section("archive")));
    let w = win.clone();
    add("about", Box::new(move || show_about(&w)));
    let w = win.clone();
    add("history", Box::new(move || show_history(&w)));
    // In the original Commands window an analysis starts with Enter: we keep the
    // habit, with Ctrl+Enter so it does not clash with text entries.
    for (action, accel) in [
        ("app.preferences", "<Control>comma"),
        ("app.history", "<Control>h"),
        ("app.archive", "<Control>b"),
        ("app.run", "<Control>Return"),
    ] {
        app.set_accels_for_action(action, &[accel]);
    }
    let w = win.clone();
    add("run", Box::new(move || w.run_from_action()));

    // Activated by the download-finished notification, which carries the path:
    // without the parameter we would not know which folder to open.
    let w = win.clone();
    let open = gio::SimpleAction::new("open-downloaded", Some(glib::VariantTy::STRING));
    open.connect_activate(move |_, param| {
        if let Some(p) = param.and_then(|v| v.str()) {
            w.open_downloaded(std::path::Path::new(p));
        }
    });
    app.add_action(&open);
}

/// Registers the app icon.
///
/// It belongs here rather than in `main`: the icon theme only exists once GTK
/// has opened a display, and forcing `gtk::init()` before the application is the
/// kind of shortcut you pay for with double starts.
fn register_icon() {
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(icons_dir());
    }
    gtk::Window::set_default_icon_name(APP_ID);
}

fn icons_dir() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for rel in ["../../data/icons", "../../../data/icons", "data/icons"] {
                let p = parent.join(rel);
                if p.is_dir() {
                    return p;
                }
            }
        }
    }
    std::path::PathBuf::from("data/icons")
}

fn show_about(win: &App) {
    let d = adw::AboutDialog::new();
    d.set_application_name("TalkBank");
    d.set_application_icon(APP_ID);
    d.set_version(env!("CARGO_PKG_VERSION"));
    d.set_developer_name("Brian MacWhinney · TalkBank");
    d.set_comments(&t(
        "Graphical front-end for the CLAN programs (Computerized Language ANalysis), part of the CHILDES/TalkBank project.",
    ));
    d.set_website("https://talkbank.org/childes/");
    d.set_issue_url("https://github.com/matteospanio/talkbank/issues");
    d.present(Some(win.window()));
}

fn show_history(win: &App) {
    let dlg = adw::Dialog::new();
    dlg.set_title(&t("Recent commands"));
    dlg.set_content_width(620);
    dlg.set_content_height(420);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    let history = win.history();
    if history.is_empty() {
        group.set_description(Some(&t("Nothing has been run yet in this session.")));
    }
    for line in history.iter().rev() {
        let row = adw::ActionRow::new();
        row.set_title(line);
        row.set_use_markup(false);
        row.set_subtitle_selectable(true);
        row.add_css_class("monospace");
        group.add(&row);
    }
    page.add(&group);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&page));
    dlg.set_child(Some(&tv));
    dlg.present(Some(win.window()));
}
