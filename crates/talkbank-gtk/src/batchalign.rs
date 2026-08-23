//! The "Media and transcription" section, which goes through Batchalign3.
//!
//! Batchalign is optional. When it is absent, this section **explains itself**
//! rather than failing: it says what the tool does, why it might help, and how to
//! install it. That is the state seen on a freshly set-up machine, so it is the
//! one polished first.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use talkbank_batchalign::server::{self, Availability};
use talkbank_batchalign::types::{Command, Submission};
use talkbank_batchalign::{client, Client, Status};

use crate::i18n::t;
use crate::net::net;

/// The released commands, presented by goal the way the CLAN ones are.
///
/// We do not show all twelve: `benchmark`, `compare` and `coref` are for people
/// developing Batchalign, not for people analysing transcripts.
pub struct Task {
    pub command: Command,
    pub title: &'static str,
    pub desc: &'static str,
    /// True when it works on the audio rather than on the transcript.
    pub needs_media: bool,
}

pub static TASKS: &[Task] = &[
    Task {
        command: Command::Transcribe,
        title: "Transcribe audio into a CHAT file",
        desc: "Turns a recording into a transcript, with speakers separated and utterances \
               segmented. Runs locally with Whisper; no account or key is needed.",
        needs_media: true,
    },
    Task {
        command: Command::Align,
        title: "Link the transcript to the recording",
        desc: "Aligns an existing transcript with its audio, so every utterance carries the \
               time when it was said and you can play it back.",
        needs_media: true,
    },
    Task {
        command: Command::Morphotag,
        title: "Create the %mor and %gra tiers",
        desc: "Adds morphology and grammatical relations using Universal Dependencies. \
               Covers around 26 languages, where the classic MOR grammars only cover \
               English, French, Spanish and Chinese.",
        needs_media: false,
    },
    Task {
        command: Command::Utseg,
        title: "Split into utterances",
        desc: "Restores utterance boundaries and punctuation in a transcript that has none.",
        needs_media: false,
    },
    Task {
        command: Command::Diarize,
        title: "Work out who is speaking",
        desc: "Separates the voices in a recording and assigns each utterance to a speaker.",
        needs_media: true,
    },
    Task {
        command: Command::Translate,
        title: "Translate the transcript",
        desc: "Adds a translation tier to each utterance.",
        needs_media: false,
    },
];

pub fn find_task(command: Command) -> Option<&'static Task> {
    TASKS.iter().find(|t| t.command == command)
}

pub struct State {
    server: RefCell<Option<server::Server>>,
    port: u16,
}

thread_local! {
    static STATE: Rc<State> = Rc::new(State {
        server: RefCell::new(None),
        port: server::DEFAULT_PORT,
    });
}

pub fn state() -> Rc<State> {
    STATE.with(Rc::clone)
}

impl State {
    /// Shuts the server down if we started it. Call it on exit: an orphan
    /// process holds the port and the memory.
    pub fn shutdown(&self) {
        if let Some(mut s) = self.server.borrow_mut().take() {
            s.shutdown();
        }
    }

    pub fn started_by_us(&self) -> bool {
        self.server.borrow().is_some()
    }

    fn adopt(&self, s: server::Server) {
        *self.server.borrow_mut() = Some(s);
    }
}

/// The page shown when a Batchalign task is chosen.
///
/// It always builds the descriptive part, then decides what to offer based on
/// what is present on the machine.
pub fn page(task: &'static Task, on_run: impl Fn(Command) + 'static) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let g = adw::PreferencesGroup::new();
    g.set_title(&t("What it does"));
    g.set_description(Some(&t(task.desc)));
    page.add(&g);

    let status_group = adw::PreferencesGroup::new();
    status_group.set_title(&t("Batchalign"));
    let checking = adw::ActionRow::new();
    checking.set_title(&t("Checking whether Batchalign is available…"));
    checking.add_prefix(&adw::Spinner::new());
    status_group.add(&checking);
    page.add(&status_group);

    let on_run = Rc::new(on_run);
    let sg = status_group.clone();
    let row = checking.clone();
    let cmd = task.command;
    net().spawn(
        async move {
            let http = reqwest::Client::new();
            server::availability(&http, server::DEFAULT_PORT).await
        },
        move |avail| {
            sg.remove(&row);
            match avail {
                Availability::NotInstalled => sg.add(&not_installed_row()),
                Availability::Installed(path) => {
                    sg.add(&installed_row(&path));
                    sg.add(&run_row(cmd, on_run.clone(), t("Start Batchalign and run")));
                }
                Availability::Running(port) => {
                    let r = adw::ActionRow::new();
                    r.set_title(&t("Batchalign is running"));
                    r.set_subtitle(&format!("127.0.0.1:{port}"));
                    r.add_prefix(&gtk::Image::from_icon_name("emblem-ok-symbolic"));
                    sg.add(&r);
                    sg.add(&run_row(cmd, on_run.clone(), t("Run")));
                }
            }
        },
    );
    page
}

fn not_installed_row() -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(&t("Batchalign is not installed"));
    r.set_subtitle(&t(
        "It is a separate TalkBank tool. This app works fully without it: you only need it for \
         speech recognition, media alignment, and morphology in languages the classic MOR \
         grammars do not cover.",
    ));
    r.add_prefix(&gtk::Image::from_icon_name("dialog-information-symbolic"));

    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_valign(gtk::Align::Center);
    copy.set_tooltip_text(Some(&t("Copy the installation command")));
    copy.connect_clicked(|b| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(server::INSTALL_COMMAND);
        }
        b.set_tooltip_text(Some(&t("Copied")));
    });
    r.add_suffix(&copy);
    r
}

fn installed_row(path: &std::path::Path) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(&t("Batchalign is installed but not running"));
    r.set_subtitle(&format!(
        "{}\n{}",
        path.display(),
        t("It will be started when you run a task, and stopped when you close TalkBank. \
           The first run downloads several gigabytes of models.")
    ));
    r.add_prefix(&gtk::Image::from_icon_name("dialog-information-symbolic"));
    r
}

fn run_row(cmd: Command, on_run: Rc<impl Fn(Command) + 'static>, label: String) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(&label);
    let b = gtk::Button::with_label(&t("Run"));
    b.add_css_class("suggested-action");
    b.set_valign(gtk::Align::Center);
    b.connect_clicked(move |_| on_run(cmd));
    r.add_suffix(&b);
    r
}

/// Starts the server if needed, then submits the job and follows it to the end.
pub fn run_task(
    cmd: Command,
    paths: Vec<String>,
    lang: Option<String>,
    mut on_progress: impl FnMut(String, Option<f64>) + 'static,
    on_done: impl FnOnce(Result<talkbank_batchalign::JobInfo, client::Error>) + 'static,
) {
    let st = state();
    let port = st.port;

    // If it is not listening and the binary is there, we start it. Never when the
    // app opens: only when the user actually asks for a job.
    if !st.started_by_us() {
        if let Some(bin) = server::find_binary() {
            match server::Server::spawn(&bin, port) {
                Ok(s) => st.adopt(s),
                Err(e) => tracing::warn!("could not start Batchalign: {e}"),
            }
        }
    }

    let (tx, rx) = async_channel::bounded::<(String, Option<f64>)>(32);
    net().spawn(
        async move {
            let http = reqwest::Client::new();
            server::wait_until_healthy(&http, port, std::time::Duration::from_secs(180))
                .await
                .map_err(client::Error::Unreachable)?;

            let client = Client::at_port(port);
            let job = client.submit(&Submission::on_paths(cmd, paths, lang)).await?;
            let id = job.job_id.clone();
            client
                .follow(&id, |info| {
                    let _ = tx.try_send((client::describe(info), info.fraction()));
                    true
                })
                .await
        },
        move |res| on_done(res),
    );

    glib::spawn_future_local(async move {
        while let Ok((text, frac)) = rx.recv().await {
            on_progress(text, frac);
        }
    });
}

/// Tasks that work on audio need a media file.
///
/// Worth checking up front: otherwise Batchalign accepts the job, loads the
/// models — gigabytes on the first run — and only fails at the end with
/// `input_missing`.
pub fn media_missing(task: &Task, paths: &[String]) -> bool {
    if !task.needs_media {
        return false;
    }
    !paths.iter().any(|p| {
        let path = std::path::Path::new(p);
        if is_media(path) {
            return true;
        }
        // or a media file with the same name next to the transcript
        ["wav", "mp3", "mp4", "mov", "m4a", "flac"]
            .iter()
            .any(|ext| path.with_extension(ext).is_file())
    })
}

fn is_media(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "wav" | "mp3" | "mp4" | "mov" | "m4a" | "flac" | "aif" | "aiff"
            )
        })
        .unwrap_or(false)
}

/// True when the status says the job succeeded.
pub fn succeeded(info: &talkbank_batchalign::JobInfo) -> bool {
    info.status == Status::Completed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i_compiti_mostrati_sono_quelli_utili_a_chi_analizza() {
        assert_eq!(TASKS.len(), 6);
        // benchmark, compare e coref servono a chi sviluppa Batchalign
        for escluso in [Command::Benchmark, Command::Compare, Command::Coref] {
            assert!(find_task(escluso).is_none(), "{escluso:?} non va mostrato");
        }
        assert!(find_task(Command::Morphotag).is_some());
    }

    #[test]
    fn every_task_has_a_title_and_an_explanation() {
        for task in TASKS {
            assert!(!task.title.is_empty());
            assert!(
                task.desc.len() > 40,
                "\"{}\" has an explanation too short for someone new to the tool",
                task.title
            );
        }
    }

    #[test]
    fn a_warning_is_raised_when_the_media_is_missing() {
        let dir = tempdir::TempDir::new("talkbank-ba").unwrap();
        let cha = dir.path().join("a.cha");
        std::fs::write(&cha, "").unwrap();
        let text_only = vec![cha.display().to_string()];

        let transcribe = find_task(Command::Transcribe).unwrap();
        assert!(media_missing(transcribe, &text_only), "with no audio, warn");

        // with the media next to it, same name, all is well
        std::fs::write(dir.path().join("a.wav"), "").unwrap();
        assert!(!media_missing(transcribe, &text_only));

        // and a task that works on text must never complain
        let morpho = find_task(Command::Morphotag).unwrap();
        assert!(!media_missing(morpho, &text_only));
    }

    #[test]
    fn an_audio_file_chosen_directly_is_enough() {
        let transcribe = find_task(Command::Transcribe).unwrap();
        assert!(!media_missing(transcribe, &["/x/recording.mp3".to_string()]));
        assert!(media_missing(transcribe, &["/x/notes.txt".to_string()]));
    }

    #[test]
    fn tasks_that_require_media_are_marked() {
        assert!(find_task(Command::Transcribe).unwrap().needs_media);
        assert!(find_task(Command::Diarize).unwrap().needs_media);
        // morphosyntax works on text, no audio needed
        assert!(!find_task(Command::Morphotag).unwrap().needs_media);
    }
}
