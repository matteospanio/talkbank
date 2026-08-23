//! The start page.
//!
//! On opening, the question is not "which analysis" but "what was I working on".
//! First you resume, then you choose: yesterday's work sits here, along with the
//! four routes that lead to everything else.
//!
//! Anyone opening the app for the first time has nothing to resume: in that case
//! the page explains in four lines what a corpus is and where to begin, instead
//! of showing an empty list.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::glib;

use crate::config;
use crate::i18n::{t, tn};
use crate::window::App;

pub fn page(app: &App) -> gtk::Widget {
    let head = adw::HeaderBar::new();
    head.set_title_widget(Some(&adw::WindowTitle::new(&t("Home"), "")));
    head.pack_end(&app.menu_button());

    let page = adw::PreferencesPage::new();
    let recent_dirs = config::recent_dirs();
    let recent_files = config::recent_files();

    if recent_dirs.is_empty() && recent_files.is_empty() {
        page.add(&first_time_group(app));
    } else {
        page.add(&resume_group(app, &recent_dirs));
    }
    page.add(&actions_group(app));
    if !recent_files.is_empty() {
        page.add(&files_group(app, &recent_files));
    }
    if recent_dirs.len() > 1 {
        page.add(&dirs_group(app, &recent_dirs[1..]));
    }
    page.add(&learn_group(app));

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&head);
    tv.set_content(Some(&page));
    tv.upcast()
}

/// Resume: a single large row, with the folder and the last analysis.
fn resume_group(app: &App, dirs: &[PathBuf]) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("Pick up where you left off"));

    let dir = &dirs[0];
    let row = adw::ActionRow::new();
    row.set_title(&folder_title(dir));
    let files = count_transcripts(dir);
    let last = config::with(|c| c.command.clone()).unwrap_or_default();
    let mut bits = vec![tn("%u transcript", "%u transcripts", files as u32)
        .replace("%u", &files.to_string())];
    if !last.is_empty() {
        bits.push(t("last analysis: %s").replace("%s", &last));
    }
    row.set_subtitle(&bits.join(" · "));
    row.add_prefix(&gtk::Image::from_icon_name("folder-open-symbolic"));

    let open = gtk::Button::with_label(&t("Resume"));
    open.add_css_class("suggested-action");
    open.set_valign(gtk::Align::Center);
    let a = app.clone();
    let d = dir.clone();
    open.connect_clicked(move |_| {
        a.set_workdir(&d);
        a.show_section("transcripts");
    });
    row.add_suffix(&open);
    row.set_activatable_widget(Some(&open));
    g.add(&row);
    g
}

/// First run: four lines on what all this is, because an empty recents list
/// teaches nothing.
fn first_time_group(app: &App) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("Welcome"));
    g.set_description(Some(&t(
        "This app works on CHAT transcripts: plain-text recordings of talk, the format used by \
         CHILDES and the other TalkBank collections. You need a folder of them to start.",
    )));

    let row = adw::ActionRow::new();
    row.set_title(&t("Get a corpus from TalkBank"));
    row.set_subtitle(&t("Thousands of transcripts, free with an account. This is the usual way to start."));
    row.add_prefix(&gtk::Image::from_icon_name("folder-download-symbolic"));
    let btn = gtk::Button::with_label(&t("Browse the archive"));
    btn.add_css_class("suggested-action");
    btn.set_valign(gtk::Align::Center);
    let a = app.clone();
    btn.connect_clicked(move |_| a.show_section("archive"));
    row.add_suffix(&btn);
    row.set_activatable_widget(Some(&btn));
    g.add(&row);
    g
}

/// The four routes. They are verbs, not program names: someone arriving for the
/// first time knows what they want to do, not what the tool is called.
fn actions_group(app: &App) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("What would you like to do?"));

    let voci: [(&str, String, String, &str); 4] = [
        (
            "folder-open-symbolic",
            t("Open a folder of transcripts"),
            t("Read, edit and check the files you already have"),
            "open-folder",
        ),
        (
            "folder-download-symbolic",
            t("Download a corpus"),
            t("Browse TalkBank: CHILDES, PhonBank, AphasiaBank and twelve more"),
            "archive",
        ),
        (
            "audio-input-microphone-symbolic",
            t("Transcribe audio or video"),
            t("Automatic transcription and alignment with Batchalign"),
            "media",
        ),
        (
            "view-list-symbolic",
            t("Run an analysis"),
            t("Word counts, MLU, searches: the 70 CLAN programs"),
            "analysis",
        ),
    ];

    for (icon, title, subtitle, action) in voci {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&subtitle);
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_activatable(true);
        let a = app.clone();
        row.connect_activated(move |_| match action {
            "open-folder" => a.choose_folder(),
            "archive" => a.show_section("archive"),
            "media" => {
                a.show_section("analysis");
                a.select_first_media_task();
            }
            _ => a.show_section("analysis"),
        });
        g.add(&row);
    }
    g
}

fn files_group(app: &App, files: &[PathBuf]) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("Recent transcripts"));
    for f in files.iter().take(5) {
        let row = adw::ActionRow::new();
        row.set_title(&f.file_name().unwrap_or_default().to_string_lossy());
        row.set_subtitle(&shorten(f.parent().unwrap_or(f)));
        row.add_prefix(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_activatable(true);
        let a = app.clone();
        let p = f.clone();
        row.connect_activated(move |_| {
            a.open_file(&p);
            a.show_section("transcripts");
        });
        g.add(&row);
    }
    g
}

fn dirs_group(app: &App, dirs: &[PathBuf]) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("Recent folders"));
    for d in dirs.iter().take(5) {
        let row = adw::ActionRow::new();
        row.set_title(&folder_title(d));
        row.set_subtitle(&shorten(d));
        row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_activatable(true);
        let a = app.clone();
        let p = d.clone();
        row.connect_activated(move |_| {
            a.set_workdir(&p);
            a.show_section("transcripts");
        });
        g.add(&row);
    }
    g
}

/// The bare minimum needed to understand what you are looking at, collapsible so
/// that it does not take up the page for anyone who already knows.
fn learn_group(app: &App) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    let exp = adw::ExpanderRow::new();
    exp.set_title(&t("New to CHAT transcripts?"));
    exp.set_subtitle(&t("What the files contain, and the order to do things in"));

    for (title, body) in [
        (
            t("A transcript is a text file"),
            t("Lines starting with @ describe the recording, lines starting with * are what \
               someone said, lines starting with % are annotations added on top."),
        ),
        (
            t("The %mor tier is what most analyses need"),
            t("It marks the part of speech of every word. Freshly transcribed files almost \
               never have it; MOR or Batchalign can add it."),
        ),
        (
            t("The usual order"),
            t("Get the files → check the format → add %mor if it is missing → run the analysis."),
        ),
    ] {
        let r = adw::ActionRow::new();
        r.set_title(&title);
        r.set_subtitle(&body);
        r.set_subtitle_lines(0);
        exp.add_row(&r);
    }

    let guide = adw::ActionRow::new();
    guide.set_title(&t("Open the full guide"));
    guide.add_suffix(&gtk::Image::from_icon_name("external-link-symbolic"));
    guide.set_activatable(true);
    let win = app.window().clone();
    guide.connect_activated(move |_| {
        // The guide sits next to the executable or one level up (where it lands
        // when working from `build/`); if it is missing we open the website,
        // which says the same things and is always reachable. The current
        // directory will not do: the app gets launched from anywhere.
        let uri = guide_uri().unwrap_or_else(|| "https://talkbank.org/".into());
        gtk::UriLauncher::new(&uri).launch(Some(&win), gtk::gio::Cancellable::NONE, |_| {});
    });
    exp.add_row(&guide);

    g.add(&exp);
    g
}

// ------------------------------------------------------------------ helpers

/// The path to the guide, if we find it near the executable.
///
/// Two layouts: the build directory (`build/talkbank` next to the checkout) and
/// an installation (`<prefix>/bin/talkbank`, guide under `share/doc`).
fn guide_uri() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let up = dir.parent()?.to_path_buf();
    [
        dir.join("docs/guide.md"),
        up.join("docs/guide.md"),
        up.join("share/doc/talkbank/guide.md"),
    ]
    .into_iter()
    .find(|p| p.is_file())
        .and_then(|p| glib::filename_to_uri(&p, None).ok())
        .map(|u| u.to_string())
}

fn folder_title(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// Shortens a path by replacing the home directory with `~`: the row stays
/// readable without losing what is needed to recognise the folder.
pub fn shorten(p: &Path) -> String {
    let home = glib::home_dir();
    match p.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn count_transcripts(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "cha" || x == "cex")
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_inside_the_home_directory_is_shortened() {
        let home = glib::home_dir();
        assert_eq!(shorten(&home.join("a/b")), "~/a/b");
        assert_eq!(shorten(Path::new("/opt/data")), "/opt/data");
    }

    #[test]
    fn a_folders_title_is_its_name() {
        assert_eq!(folder_title(Path::new("/a/b/Brown")), "Brown");
        assert_eq!(folder_title(Path::new("/")), "/");
    }

    #[test]
    fn only_transcripts_are_counted() {
        let d = tempdir::TempDir::new("talkbank-home").unwrap();
        for n in ["a.cha", "b.cha", "c.cex", "note.txt", "audio.wav"] {
            std::fs::write(d.path().join(n), "").unwrap();
        }
        assert_eq!(count_transcripts(d.path()), 3);
    }
}
