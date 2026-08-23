//! Preferences, in `~/.config/talkbank/settings.ini`.
//!
//! The format matches the one the C version used. We use `glib::KeyFile` for the
//! same reason: it is the same reader, so there is no dialect drift.

use gtk::glib;
use std::cell::RefCell;
use std::path::PathBuf;

const GROUP: &str = "ui";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Language {
    System,
    Italian,
    English,
}

impl Language {
    pub fn code(&self) -> &'static str {
        match self {
            Language::System => "auto",
            Language::Italian => "it",
            Language::English => "en",
        }
    }
    pub fn from_code(s: &str) -> Language {
        match s {
            "it" => Language::Italian,
            "en" => Language::English,
            _ => Language::System,
        }
    }
    pub fn index(&self) -> u32 {
        match self {
            Language::System => 0,
            Language::Italian => 1,
            Language::English => 2,
        }
    }
    pub fn from_index(i: u32) -> Language {
        match i {
            1 => Language::Italian,
            2 => Language::English,
            _ => Language::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn code(&self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
    pub fn from_code(s: &str) -> Theme {
        match s {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::System,
        }
    }
    pub fn index(&self) -> u32 {
        match self {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        }
    }
    pub fn from_index(i: u32) -> Theme {
        match i {
            1 => Theme::Light,
            2 => Theme::Dark,
            _ => Theme::System,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub language: Language,
    pub theme: Theme,
    pub font_size: i32,
    pub show_command: bool,
    pub preflight: bool,
    pub default_r6: bool,
    pub default_save: bool,
    pub workdir: Option<PathBuf>,
    pub command: Option<String>,
    /// TalkBank account email. Not a secret, and it pre-fills the field before
    /// the keyring unlocks; the password lives elsewhere.
    pub email: Option<String>,
    /// Where downloaded corpora land.
    pub download_dir: Option<PathBuf>,
    /// Whether "include audio and video" starts switched on. Off by default:
    /// recordings are two to four orders of magnitude larger than transcripts,
    /// so opting in has to be a decision, not a surprise.
    pub download_media: bool,
    /// Recently opened folders, newest first. The start page needs it: resuming
    /// work is the most frequent action on opening.
    pub recent_dirs: Vec<PathBuf>,
    /// Recently opened transcripts, newest first.
    pub recent_files: Vec<PathBuf>,
}

/// How many recent entries to keep. Past ten the list stops being a shortcut and
/// becomes a second navigation tree to read.
const MAX_RECENT: usize = 10;

/// Puts `path` at the head of the list, without duplicates, trimming the tail.
fn remember(list: &mut Vec<PathBuf>, path: &std::path::Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX_RECENT);
}

pub fn remember_dir(path: &std::path::Path) {
    update(|c| remember(&mut c.recent_dirs, path));
}

pub fn remember_file(path: &std::path::Path) {
    update(|c| remember(&mut c.recent_files, path));
}

/// The recent folders that still exist: a deleted corpus must not linger on a
/// start page that promises to reopen it.
pub fn recent_dirs() -> Vec<PathBuf> {
    with(|c| c.recent_dirs.iter().filter(|p| p.is_dir()).cloned().collect())
}

pub fn recent_files() -> Vec<PathBuf> {
    with(|c| c.recent_files.iter().filter(|p| p.is_file()).cloned().collect())
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: Language::System,
            theme: Theme::System,
            font_size: 11,
            show_command: true,
            preflight: true,
            default_r6: false,
            default_save: false,
            workdir: None,
            command: None,
            email: None,
            download_dir: None,
            download_media: false,
            recent_dirs: Vec::new(),
            recent_files: Vec::new(),
        }
    }
}

pub fn path() -> PathBuf {
    glib::user_config_dir().join("talkbank").join("settings.ini")
}

impl Config {
    pub fn load() -> Config {
        let kf = glib::KeyFile::new();
        if kf.load_from_file(path(), glib::KeyFileFlags::NONE).is_err() {
            return Config::default();
        }
        let d = Config::default();
        let s = |k: &str| kf.string(GROUP, k).ok().map(|v| v.to_string());
        let b = |k: &str, dflt: bool| kf.boolean(GROUP, k).unwrap_or(dflt);
        // One key per entry (`recent-dir-0`, `recent-dir-1`, …): a path can
        // contain the `;` KeyFile uses to separate lists, and glib-rs does not
        // expose the writer that would escape it.
        let list = |prefix: &str| {
            (0..MAX_RECENT)
                .filter_map(|i| kf.string(GROUP, &format!("{prefix}-{i}")).ok())
                .map(|v| PathBuf::from(v.as_str()))
                .collect::<Vec<_>>()
        };

        Config {
            language: s("language").map_or(d.language, |v| Language::from_code(&v)),
            theme: s("theme").map_or(d.theme, |v| Theme::from_code(&v)),
            font_size: kf.integer(GROUP, "font-size").unwrap_or(d.font_size).clamp(7, 22),
            show_command: b("show-command", d.show_command),
            preflight: b("preflight", d.preflight),
            default_r6: b("default-r6", d.default_r6),
            default_save: b("default-save", d.default_save),
            workdir: s("workdir").map(PathBuf::from).filter(|p| p.is_dir()),
            command: s("command"),
            email: s("email"),
            download_dir: s("download-dir").map(PathBuf::from),
            download_media: b("download-media", d.download_media),
            recent_dirs: {
                let mut r = list("recent-dir");
                // Anyone upgrading from an earlier version has a working
                // directory but no recents: without this they would see the
                // welcome screen despite having already done work.
                if r.is_empty() {
                    if let Some(w) = s("workdir").map(PathBuf::from).filter(|p| p.is_dir()) {
                        r.push(w);
                    }
                }
                r
            },
            recent_files: list("recent-file"),
        }
    }

    pub fn save(&self) {
        let kf = glib::KeyFile::new();
        // Re-read before writing, so we do not wipe keys a future version might
        // have added.
        let _ = kf.load_from_file(path(), glib::KeyFileFlags::NONE);

        kf.set_string(GROUP, "language", self.language.code());
        kf.set_string(GROUP, "theme", self.theme.code());
        kf.set_integer(GROUP, "font-size", self.font_size);
        kf.set_boolean(GROUP, "show-command", self.show_command);
        kf.set_boolean(GROUP, "preflight", self.preflight);
        kf.set_boolean(GROUP, "default-r6", self.default_r6);
        kf.set_boolean(GROUP, "default-save", self.default_save);
        kf.set_boolean(GROUP, "download-media", self.download_media);
        if let Some(w) = &self.workdir {
            kf.set_string(GROUP, "workdir", &w.to_string_lossy());
        }
        if let Some(c) = &self.command {
            kf.set_string(GROUP, "command", c);
        }
        if let Some(e) = &self.email {
            kf.set_string(GROUP, "email", e);
        }
        if let Some(d) = &self.download_dir {
            kf.set_string(GROUP, "download-dir", &d.to_string_lossy());
        }
        for (prefix, paths) in [
            ("recent-dir", &self.recent_dirs),
            ("recent-file", &self.recent_files),
        ] {
            for i in 0..MAX_RECENT {
                let key = format!("{prefix}-{i}");
                match paths.get(i) {
                    Some(p) => kf.set_string(GROUP, &key, &p.to_string_lossy()),
                    // Surplus keys must go, otherwise a shortened list would keep
                    // showing its old tail.
                    None => {
                        let _ = kf.remove_key(GROUP, &key);
                    }
                }
            }
        }

        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = kf.save_to_file(&p) {
            tracing::warn!("could not save the preferences to {}: {e}", p.display());
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Config> = RefCell::new(Config::default());
}

pub fn init(cfg: Config) {
    CURRENT.with(|c| *c.borrow_mut() = cfg);
}

pub fn with<T>(f: impl FnOnce(&Config) -> T) -> T {
    CURRENT.with(|c| f(&c.borrow()))
}

/// Change and save in one go: there are few preferences and saving is instant,
/// so an abrupt exit loses nothing.
pub fn update(f: impl FnOnce(&mut Config)) {
    CURRENT.with(|c| {
        let mut cfg = c.borrow_mut();
        f(&mut cfg);
        cfg.save();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_language_and_theme_codes_round_trip() {
        for l in [Language::System, Language::Italian, Language::English] {
            assert_eq!(Language::from_code(l.code()), l);
            assert_eq!(Language::from_index(l.index()), l);
        }
        for t in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_code(t.code()), t);
            assert_eq!(Theme::from_index(t.index()), t);
        }
    }

    #[test]
    fn recents_have_no_duplicates_and_the_newest_is_first() {
        let mut l = Vec::new();
        for p in ["/a", "/b", "/a", "/c"] {
            remember(&mut l, std::path::Path::new(p));
        }
        assert_eq!(l, [PathBuf::from("/c"), PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn recents_stop_at_the_ceiling() {
        let mut l = Vec::new();
        for i in 0..(MAX_RECENT + 5) {
            remember(&mut l, std::path::Path::new(&format!("/p{i}")));
        }
        assert_eq!(l.len(), MAX_RECENT);
        assert_eq!(l[0], PathBuf::from(&format!("/p{}", MAX_RECENT + 4)));
    }

    #[test]
    fn an_unknown_code_falls_back_to_the_system_setting() {
        assert_eq!(Language::from_code("klingon"), Language::System);
        assert_eq!(Theme::from_code("fuchsia"), Theme::System);
    }
}
