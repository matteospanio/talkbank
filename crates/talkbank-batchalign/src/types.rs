//! The Batchalign3 control-plane types, taken from its `openapi.json`.
//!
//! Written out by hand rather than generated: we need twelve commands and four
//! structs, and a dependency on an OpenAPI generator would cost more than it
//! returns. In exchange, every field in here is one we actually use.

use serde::{Deserialize, Serialize};

/// The commands Batchalign3 declares released (`ReleasedCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Align,
    Transcribe,
    TranscribeS,
    Translate,
    Morphotag,
    Coref,
    Utseg,
    Benchmark,
    Opensmile,
    Compare,
    Avqi,
    Diarize,
}

impl Command {
    /// From the name used on the wire. Useful for anything that keeps the choice
    /// as a string, such as a UI row or the preferences file.
    pub fn from_str(s: &str) -> Option<Command> {
        Some(match s {
            "align" => Command::Align,
            "transcribe" => Command::Transcribe,
            "transcribe_s" => Command::TranscribeS,
            "translate" => Command::Translate,
            "morphotag" => Command::Morphotag,
            "coref" => Command::Coref,
            "utseg" => Command::Utseg,
            "benchmark" => Command::Benchmark,
            "opensmile" => Command::Opensmile,
            "compare" => Command::Compare,
            "avqi" => Command::Avqi,
            "diarize" => Command::Diarize,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Command::Align => "align",
            Command::Transcribe => "transcribe",
            Command::TranscribeS => "transcribe_s",
            Command::Translate => "translate",
            Command::Morphotag => "morphotag",
            Command::Coref => "coref",
            Command::Utseg => "utseg",
            Command::Benchmark => "benchmark",
            Command::Opensmile => "opensmile",
            Command::Compare => "compare",
            Command::Avqi => "avqi",
            Command::Diarize => "diarize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    WritebackFailed,
    /// A status we did not know about: better to show it than to pretend the job
    /// finished well.
    #[serde(other)]
    Unknown,
}

impl Status {
    pub fn is_final(self) -> bool {
        !matches!(self, Status::Queued | Status::Running)
    }
    pub fn is_success(self) -> bool {
        matches!(self, Status::Completed)
    }
}

/// Why a job failed. The server types this, so we use it rather than showing a
/// string: a momentary provider error deserves "try again", a worker crash
/// deserves a different explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    Validation,
    ParseError,
    InputMissing,
    WorkerCrash,
    WorkerTimeout,
    WorkerProtocol,
    WorkerBootstrap,
    ProviderTransient,
    ProviderTerminal,
    MemoryPressure,
    Cancelled,
    System,
    #[serde(other)]
    Unknown,
}

impl FailureCategory {
    /// True when retrying makes sense without changing anything.
    pub fn worth_retrying(self) -> bool {
        matches!(
            self,
            FailureCategory::ProviderTransient
                | FailureCategory::WorkerCrash
                | FailureCategory::WorkerTimeout
        )
    }
}

/// A job request.
///
/// We use `paths_mode`: the server reads and writes the given paths directly,
/// without the app handing it file contents. That is the right mode for a local
/// application — it avoids copying thousand-file corpora into memory, and the
/// results land where they are wanted.
#[derive(Debug, Clone, Serialize)]
pub struct Submission {
    pub command: Command,
    /// Typed command options; empty is fine for ordinary use.
    pub options: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    pub paths_mode: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub output_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_speakers: Option<u32>,
}

impl Submission {
    /// A job over local files, with the output next to the input.
    pub fn on_paths(command: Command, paths: Vec<String>, lang: Option<String>) -> Submission {
        let dir = paths
            .first()
            .and_then(|p| std::path::Path::new(p).parent())
            .map(|p| p.display().to_string());
        Submission {
            command,
            options: serde_json::json!({}),
            lang,
            paths_mode: true,
            source_paths: paths,
            output_paths: Vec::new(),
            source_dir: dir,
            num_speakers: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobInfo {
    pub job_id: String,
    pub status: Status,
    #[serde(default)]
    pub total_files: u32,
    #[serde(default)]
    pub completed_files: u32,
    #[serde(default)]
    pub current_file: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub duration_s: Option<f64>,
}

impl JobInfo {
    /// Progress between 0 and 1, when the file count is known.
    pub fn fraction(&self) -> Option<f64> {
        (self.total_files > 0)
            .then(|| self.completed_files as f64 / self.total_files as f64)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub job_slots_available: u32,
    #[serde(default)]
    pub workers_available: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_names_match_the_servers() {
        // List taken from `ReleasedCommand` in their openapi.json.
        let expected = [
            "align", "transcribe", "transcribe_s", "translate", "morphotag", "coref",
            "utseg", "benchmark", "opensmile", "compare", "avqi", "diarize",
        ];
        for (cmd, want) in [
            Command::Align, Command::Transcribe, Command::TranscribeS, Command::Translate,
            Command::Morphotag, Command::Coref, Command::Utseg, Command::Benchmark,
            Command::Opensmile, Command::Compare, Command::Avqi, Command::Diarize,
        ]
        .into_iter()
        .zip(expected)
        {
            assert_eq!(cmd.as_str(), want);
            // and the JSON serialisation must give the same shape
            assert_eq!(serde_json::to_string(&cmd).unwrap(), format!("\"{want}\""));
        }
    }

    #[test]
    fn the_name_round_trips() {
        for c in [Command::Align, Command::Morphotag, Command::TranscribeS, Command::Diarize] {
            assert_eq!(Command::from_str(c.as_str()), Some(c));
        }
        assert_eq!(Command::from_str("does-not-exist"), None);
    }

    #[test]
    fn an_unknown_status_is_not_mistaken_for_success() {
        let s: Status = serde_json::from_str("\"something_new\"").unwrap();
        assert_eq!(s, Status::Unknown);
        assert!(!s.is_success());
        assert!(s.is_final(), "an unknown status must not leave the wait hanging");
    }

    #[test]
    fn final_and_in_progress_statuses_are_distinguished() {
        assert!(!Status::Queued.is_final());
        assert!(!Status::Running.is_final());
        for s in [Status::Completed, Status::Failed, Status::Cancelled, Status::Interrupted] {
            assert!(s.is_final(), "{s:?} should be final");
        }
        assert!(Status::Completed.is_success());
        assert!(!Status::WritebackFailed.is_success());
    }

    #[test]
    fn only_some_errors_are_worth_retrying() {
        assert!(FailureCategory::ProviderTransient.worth_retrying());
        assert!(FailureCategory::WorkerCrash.worth_retrying());
        // redoing the same job with a missing file would give the same result
        assert!(!FailureCategory::InputMissing.worth_retrying());
        assert!(!FailureCategory::Validation.worth_retrying());
        assert!(!FailureCategory::Cancelled.worth_retrying());
    }

    #[test]
    fn the_request_uses_paths_mode_and_omits_empty_fields() {
        let s = Submission::on_paths(
            Command::Morphotag,
            vec!["/data/Brown/adam01.cha".into()],
            Some("eng".into()),
        );
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["command"], "morphotag");
        assert_eq!(v["paths_mode"], true);
        assert_eq!(v["lang"], "eng");
        assert_eq!(v["source_dir"], "/data/Brown");
        assert!(v.get("output_paths").is_none(), "empty fields must not be sent");
        assert!(v.get("num_speakers").is_none());
    }

    #[test]
    fn progress_is_derived_from_the_completed_files() {
        let j: JobInfo = serde_json::from_str(
            r#"{"job_id":"x","status":"running","total_files":4,"completed_files":1}"#,
        )
        .unwrap();
        assert_eq!(j.fraction(), Some(0.25));

        let empty: JobInfo =
            serde_json::from_str(r#"{"job_id":"x","status":"queued"}"#).unwrap();
        assert_eq!(empty.fraction(), None, "with no total we do not invent a percentage");
    }
}
