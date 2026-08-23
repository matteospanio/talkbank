//! The state of the analysis screen, and the logic that follows from it.
//!
//! No GTK in here: building the arguments, the pre-flight checks and reading
//! CLAN's messages are pure logic, and so testable without an interface. The
//! wording is chosen by the drawing code, keeping translations where the rest of
//! the interface lives.

use std::collections::BTreeSet;
use std::path::PathBuf;

use talkbank_engine::catalog::{Command, Req};
use talkbank_engine::chat::Speaker;

/// Dependent tiers offered as shortcuts. These are the most used ones; the rest
/// go in the free-text options field.
pub const TIERS: &[&str] = &["%mor", "%gra", "%pho", "%mod", "%err", "%spa"];

#[derive(Debug, Default)]
pub struct Analysis {
    pub cmd: Option<&'static Command>,
    pub files: Vec<String>,
    pub sel_files: BTreeSet<String>,
    pub speakers: Vec<Speaker>,
    pub sel_speakers: BTreeSet<String>,
    pub sel_tiers: BTreeSet<String>,
    /// True when at least one selected file has a `%mor` tier.
    pub files_have_mor: bool,
    /// Languages declared in the chosen files (`@Languages`), to suggest `+l`.
    pub file_languages: Vec<String>,
    pub word: String,
    pub extra: String,
    pub lang: Option<String>,
    pub opt_case: bool,
    pub opt_merge: bool,
    pub opt_recursive: bool,
    pub opt_repetitions: bool,
    pub opt_save: bool,
    pub opt_sheet: bool,
    pub running: bool,
    pub history: Vec<String>,
}

/// A problem found before running. The interface supplies the wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preflight {
    Ok,
    /// No file chosen: blocking.
    NoFiles,
    /// The command needs a speaker: blocking.
    NeedsSpeaker,
    /// The command needs the language: blocking.
    NeedsLanguage,
    /// The `%mor` tier is missing: warns and offers a remedy, but does not block.
    MissingMor,
}

impl Preflight {
    /// When it blocks, the Run button stays disabled.
    pub fn blocks(&self) -> bool {
        !matches!(self, Preflight::Ok | Preflight::MissingMor)
    }
}

/// Interpretation of CLAN's messages after a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    None,
    MissingMor,
    NeedsSpeaker,
    NeedsLanguage,
    NotChat,
    Failed(i32),
}

impl Analysis {
    pub fn selected_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| self.sel_files.contains(*f))
            .cloned()
            .collect()
    }

    /// The options, in the order they appear on the command line.
    pub fn flags(&self) -> Vec<String> {
        let mut v = Vec::new();
        for s in &self.sel_speakers {
            v.push(format!("+t*{s}"));
        }
        for t in &self.sel_tiers {
            v.push(format!("+t{t}"));
        }
        if let Some(l) = self.lang.as_deref().filter(|l| !l.is_empty()) {
            v.push(format!("+l{l}"));
        }
        let word = self.word.trim();
        if !word.is_empty() {
            v.push(format!("+s{word}"));
        }
        if self.opt_case {
            v.push("+k".into());
        }
        if self.opt_merge {
            v.push("+u".into());
        }
        if self.opt_recursive {
            v.push("+re".into());
        }
        if self.opt_repetitions {
            v.push("+r6".into());
        }
        if self.opt_save {
            v.push("+f".into());
        }
        if self.opt_sheet {
            if let Some(flag) = self.cmd.and_then(|c| c.sheet_flag) {
                v.push(flag.into());
            }
        }
        v.extend(tokenize(&self.extra));
        v
    }

    /// The full argument list: options, then the filenames.
    pub fn args(&self) -> Vec<String> {
        let mut v = self.flags();
        v.extend(self.selected_files());
        v
    }

    /// The line as it would be typed in a terminal. It is for display, not for
    /// running: execution goes through `args()`, with no shell in between.
    pub fn command_line(&self) -> String {
        let Some(cmd) = self.cmd else {
            return String::new();
        };
        let mut out = String::from(cmd.name);
        for a in self.args() {
            out.push(' ');
            out.push_str(&shell_quote(&a));
        }
        out
    }

    pub fn preflight(&self, warn_enabled: bool) -> Preflight {
        let Some(cmd) = self.cmd else {
            return Preflight::Ok;
        };
        if self.sel_files.is_empty() {
            return Preflight::NoFiles;
        }
        if cmd.req.has(Req::SPEAKER) && self.sel_speakers.is_empty() {
            return Preflight::NeedsSpeaker;
        }
        if cmd.req.has(Req::LANG) && self.lang.as_deref().unwrap_or("").is_empty() {
            return Preflight::NeedsLanguage;
        }
        // A warning, not a block: running anyway can still be what you want.
        if warn_enabled
            && cmd.req.has(Req::MOR)
            && !self.files_have_mor
            && !self.extra.contains("-t%mor")
        {
            return Preflight::MissingMor;
        }
        Preflight::Ok
    }
}

/// Adds `-t%mor` to the free-text options: it is the remedy CLAN itself suggests
/// when the tier is missing, and it counts the words of the main tier instead.
pub fn add_words_fallback(extra: &mut String) {
    if extra.contains("-t%mor") {
        return;
    }
    if !extra.trim().is_empty() {
        extra.push(' ');
    }
    extra.push_str("-t%mor");
}

/// Reads CLAN's output and traces it back to an understandable cause.
///
/// The messages are the programs' real ones, collected by trying them one by
/// one: CLAN says "TIER \"%MOR\" ... HASN'T BEEN FOUND", which means nothing to
/// a beginner.
pub fn interpret(stdout: &str, stderr: &str, exit_code: i32) -> Hint {
    for text in [stdout, stderr] {
        if text.contains("HASN'T BEEN FOUND IN THE INPUT DATA") || text.contains("ADD -t%mor") {
            return Hint::MissingMor;
        }
        if text.contains("Please specify at least one speaker") {
            return Hint::NeedsSpeaker;
        }
        if text.contains("rules file name with") || text.contains("language script file name") {
            return Hint::NeedsLanguage;
        }
        if text.contains("can NOT be run on a non-CHAT") {
            return Hint::NotChat;
        }
    }
    if exit_code != 0 {
        return Hint::Failed(exit_code);
    }
    Hint::None
}

/// Splits hand-typed options while respecting quotes, so `+s"in the tree"` stays
/// a single argument.
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Quotes only when needed. `+t*CHI` has to be quoted: a shell would expand the
/// asterisk, and in zsh an expansion with no matches is an error.
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".into();
    }
    if !s.contains(|c: char| {
        c.is_whitespace() || "\"'*?$<>|&;()[]{}!~`\\".contains(c)
    }) {
        return s.to_string();
    }
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Directory of language files for a command's `+l` option.
pub fn languages_for(lib_dir: &PathBuf, cmd: &Command) -> Vec<String> {
    let Some(sub) = cmd.lang_dir else {
        return Vec::new();
    };
    let mut v: Vec<String> = std::fs::read_dir(lib_dir.join(sub))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".cut").map(str::to_string)
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use talkbank_engine::catalog;

    fn base() -> Analysis {
        Analysis {
            cmd: catalog::find("freq"),
            files: vec!["a.cha".into(), "b.cha".into()],
            sel_files: ["a.cha".to_string()].into_iter().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_command_line_puts_the_options_before_the_files() {
        let mut a = base();
        a.sel_speakers.insert("CHI".into());
        a.opt_case = true;
        assert_eq!(a.command_line(), r#"freq "+t*CHI" +k a.cha"#);
    }

    #[test]
    fn with_no_files_selected_the_line_shows_only_the_command() {
        let mut a = base();
        a.sel_files.clear();
        assert_eq!(a.command_line(), "freq");
    }

    #[test]
    fn free_text_options_respect_quotes() {
        assert_eq!(tokenize(r#"+d2 +s"in the tree" +o"#),
                   vec!["+d2", "+sin the tree", "+o"]);
        assert_eq!(tokenize("   "), Vec::<String>::new());
    }

    #[test]
    fn quoting_happens_only_when_needed() {
        assert_eq!(shell_quote("+k"), "+k");
        assert_eq!(shell_quote("a.cha"), "a.cha");
        // the shell would expand the asterisk: it has to be quoted
        assert_eq!(shell_quote("+t*CHI"), r#""+t*CHI""#);
        assert_eq!(shell_quote("two words"), r#""two words""#);
    }

    #[test]
    fn the_preflight_check_blocks_only_when_it_should() {
        let mut a = base();
        a.sel_files.clear();
        assert_eq!(a.preflight(true), Preflight::NoFiles);
        assert!(a.preflight(true).blocks());

        let mut a = base();
        a.cmd = catalog::find("dss"); // needs a speaker, a language and %mor
        assert_eq!(a.preflight(true), Preflight::NeedsSpeaker);
        a.sel_speakers.insert("CHI".into());
        assert_eq!(a.preflight(true), Preflight::NeedsLanguage);
        a.lang = Some("eng".into());
        // now only the %mor warning is left, and that does not block
        assert_eq!(a.preflight(true), Preflight::MissingMor);
        assert!(!a.preflight(true).blocks());
    }

    #[test]
    fn the_mor_warning_goes_away_once_the_user_switches_to_words() {
        let mut a = base();
        a.cmd = catalog::find("mlu");
        assert_eq!(a.preflight(true), Preflight::MissingMor);
        add_words_fallback(&mut a.extra);
        assert_eq!(a.extra, "-t%mor");
        assert_eq!(a.preflight(true), Preflight::Ok);
        // and it does not pile up if applied twice
        add_words_fallback(&mut a.extra);
        assert_eq!(a.extra, "-t%mor");
    }

    #[test]
    fn the_mor_warning_can_be_turned_off_in_the_preferences() {
        let mut a = base();
        a.cmd = catalog::find("mlu");
        assert_eq!(a.preflight(false), Preflight::Ok);
    }

    #[test]
    fn with_the_mor_tier_present_there_is_no_warning() {
        let mut a = base();
        a.cmd = catalog::find("mlu");
        a.files_have_mor = true;
        assert_eq!(a.preflight(true), Preflight::Ok);
    }

    #[test]
    fn clans_real_messages_are_recognised() {
        assert_eq!(
            interpret("TIER \"%MOR\", ASSOCIATED WITH A SELECTED SPEAKER,\nHASN'T BEEN FOUND IN THE INPUT DATA!", "", 0),
            Hint::MissingMor
        );
        assert_eq!(
            interpret("", "Please specify at least one speaker tier name with \"+t\" option.", 0),
            Hint::NeedsSpeaker
        );
        assert_eq!(
            interpret("", "Please specify ipsyn rules file name with \"+l\" option.", 0),
            Hint::NeedsLanguage
        );
        assert_eq!(interpret("all good", "", 0), Hint::None);
        assert_eq!(interpret("", "", 3), Hint::Failed(3));
    }

    #[test]
    fn the_spreadsheet_flag_depends_on_the_command() {
        let mut a = base(); // freq -> +d2
        a.opt_sheet = true;
        assert!(a.flags().contains(&"+d2".to_string()));

        a.cmd = catalog::find("kwal"); // has no spreadsheet format
        assert!(!a.flags().iter().any(|f| f.starts_with("+d")));
    }
}
