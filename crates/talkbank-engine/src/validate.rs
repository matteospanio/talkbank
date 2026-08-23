//! CHAT validation through `chatter`, the authority on the format.
//!
//! Why CLAN's own CHECK is not enough: chatter runs two independent parsers
//! that check each other, reports the exact position and a typed error code.
//! CHECK says something is wrong; chatter says *where* and *why*.
//!
//! Two things learned from reading their `validate` command, worth keeping:
//!  * cross-tier alignment (`%mor`, `%gra`, `%pho`, `%wor`) is on **by
//!    default** — without it dozens of valid files get rejected, and vice versa;
//!  * use the variant that *collects* errors instead of stopping at the first,
//!    and that knows the file path, because some checks compare the filename
//!    against the `@ID` header.
//!
//! Their documentation also warns: "match on `code`, never on the message
//! text". That is why the code is a field of its own here.
//!
//! The chatter book says that "any release may change which files validate":
//! the dependency is pinned to a tag and all use of their API goes through this
//! file, so an upgrade touches one place only.

use std::path::Path;

use talkbank_model::{ErrorCollector, ParseValidateOptions, Severity, SourceLocation};

/// 1-based line number from a byte offset.
///
/// `SourceLocation` has a `line` field, but in practice it almost always comes
/// back empty, while the span is always there. Since "where" is half the value
/// of a diagnostic, we derive it — using their helper, not a copy of our own,
/// so we see the same lines their tool sees.
fn line_of(source: &str, offset: usize) -> usize {
    SourceLocation::calculate_line_column(offset, source).0
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Typed code, e.g. `E316`. This is what to match on.
    pub code: String,
    pub message: String,
    /// 1-based line number, when chatter manages to locate it.
    pub line: Option<usize>,
    /// Hint on how to fix it, when there is one.
    pub suggestion: Option<String>,
    pub is_error: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Validation {
    /// True when there is no diagnostic of severity "error". Warnings do not
    /// make a file invalid: it stays analysable.
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub utterance_count: usize,
    pub speakers: Vec<String>,
}

impl Validation {
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error)
    }
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| !d.is_error)
    }
}

/// Validates a CHAT transcript. `path` is needed by the checks that compare the
/// filename against the header; for in-memory text any plausible `.cha` name
/// will do.
pub fn validate_at(path: &Path, source: &str) -> Validation {
    let sink = ErrorCollector::new();
    let options = ParseValidateOptions::default().with_alignment();

    let parsed = talkbank_transform::parse_and_validate_streaming_for_path(
        path, source, options, &sink,
    );

    let mut diagnostics: Vec<Diagnostic> = sink
        .into_vec()
        .into_iter()
        .map(|e| Diagnostic {
            code: format!("{}", e.code),
            message: e.message.clone(),
            line: e
                .location
                .line
                .or_else(|| Some(line_of(source, e.location.span.start as usize))),
            suggestion: e.suggestion.clone(),
            is_error: matches!(e.severity, Severity::Error),
        })
        .collect();

    match parsed {
        Ok(file) => {
            let ok = !diagnostics.iter().any(|d| d.is_error);
            Validation {
                ok,
                diagnostics,
                utterance_count: file.utterance_count(),
                speakers: file
                    .unique_utterance_speakers()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            }
        }
        Err(err) => {
            // The parser gave up entirely: this never shows up in the collector.
            diagnostics.push(Diagnostic {
                code: "PARSE".into(),
                message: err.to_string(),
                line: None,
                suggestion: None,
                is_error: true,
            });
            Validation {
                ok: false,
                diagnostics,
                ..Default::default()
            }
        }
    }
}

/// Convenience for text with no file behind it.
pub fn validate(source: &str) -> Validation {
    validate_at(Path::new("transcript.cha"), source)
}

pub fn is_valid_at(path: &Path, source: &str) -> bool {
    validate_at(path, source).ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUONO: &str = "@UTF8\n@Begin\n@Languages:\teng\n@Participants:\tCHI Child\n\
        @ID:\teng|corpus|CHI|||||Child|||\n*CHI:\thello .\n@End\n";

    #[test]
    fn un_file_corretto_passa() {
        let v = validate(BUONO);
        assert!(v.ok, "chatter rejected a valid file: {:?}", v.diagnostics);
        assert_eq!(v.utterance_count, 1);
        assert_eq!(v.speakers, ["CHI"]);
    }

    #[test]
    fn line_number_from_offset() {
        let s = "uno\ndue\ntre\n";
        assert_eq!(line_of(s, 0), 1);
        assert_eq!(line_of(s, 4), 2);
        assert_eq!(line_of(s, 8), 3);
        // past the end it must stay inside the file instead of panicking
        assert!(line_of(s, 9999) >= 3);
    }

    #[test]
    fn the_diagnostic_points_at_the_line() {
        // the error is on the fifth line: that is where the user has to look
        let v = validate(
            "@UTF8\n@Begin\n@Languages:\teng\n@Participants:\tCHI Child\n\
             @ID:\teng|corpus|CHI|||||Child|||\n*CHI:\thello\n@End\n",
        );
        let d = v.errors().next().expect("nessun errore");
        assert!(d.line.is_some(), "diagnostic with no line number: {d:?}");
    }

    #[test]
    fn a_broken_file_is_rejected_with_a_code_and_a_message() {
        // missing utterance delimiter, the classic beginner mistake
        let v = validate(
            "@UTF8\n@Begin\n@Languages:\teng\n@Participants:\tCHI Child\n\
             @ID:\teng|corpus|CHI|||||Child|||\n*CHI:\thello\n@End\n",
        );
        assert!(!v.ok, "an utterance with no delimiter should have been rejected");
        let first = v.errors().next().expect("no error reported");
        assert!(!first.code.is_empty(), "diagnostic with no code");
        assert!(!first.message.is_empty(), "diagnostic with no message");
    }
}
