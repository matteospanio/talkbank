//! Reading the header of a CHAT file.
//!
//! Two pieces of information keep the user from making a mistake before an
//! analysis: which speakers are present (from `@Participants`) and whether a
//! `%mor` tier exists (without it MLU, DSS and IPSyn have nothing to count).

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speaker {
    /// Code used on the speaker lines: CHI, MOT, INV…
    pub code: String,
    /// Ruolo dichiarato: Target_Child, Mother, Investigator…
    pub role: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FileInfo {
    pub is_chat: bool,
    pub has_mor: bool,
    pub has_gra: bool,
    pub languages: Vec<String>,
    pub speakers: Vec<Speaker>,
}

impl FileInfo {
    pub fn speaker_codes(&self) -> Vec<&str> {
        self.speakers.iter().map(|s| s.code.as_str()).collect()
    }
}

/// Reads a CHAT file. An unreadable or non-CHAT file gives back an empty struct
/// rather than an error: the UI has to show something either way, and "this is
/// not a transcript" is information, not a failure.
pub fn inspect(path: &Path) -> FileInfo {
    match std::fs::read(path) {
        Ok(bytes) => inspect_bytes(&String::from_utf8_lossy(&bytes)),
        Err(_) => FileInfo::default(),
    }
}

pub fn inspect_bytes(text: &str) -> FileInfo {
    let mut info = FileInfo::default();

    for (i, line) in text.lines().enumerate() {
        if i < 3 && (line.starts_with("@UTF8") || line.starts_with("@Begin")) {
            info.is_chat = true;
        }
        if line.starts_with("%mor:") {
            info.has_mor = true;
        }
        if line.starts_with("%gra:") {
            info.has_gra = true;
        }
        if let Some(rest) = line.strip_prefix("@Languages:") {
            info.languages = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(rest) = line.strip_prefix("@Participants:") {
            info.speakers = parse_participants(rest);
        }
    }
    info
}

/// `@Participants:	CHI Nicky Target_Child, MOT Kelly Mother`
/// We keep the role rather than the name: the role is what tells the user who
/// the target child is, and names are often pseudonyms or missing entirely.
fn parse_participants(rest: &str) -> Vec<Speaker> {
    rest.split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() {
                return None;
            }
            let mut parts = item.split_whitespace();
            let code = parts.next()?.to_string();
            // `last()` on an already advanced iterator: if only the code was
            // there it stays None, which is what we want.
            let role = parts.last().map(str::to_string);
            Some(Speaker { code, role })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENZA_MOR: &str = "@UTF8\n@Begin\n@Languages:\teng\n\
        @Participants:\tCHI Nicky Target_Child, MOT Kelly Mother\n\
        *CHI:\tsee the chalk .\n*MOT:\tyes .\n@End\n";

    const CON_MOR: &str = "@UTF8\n@Begin\n@Languages:\teng, spa\n\
        @Participants:\tCHI Nicky Target_Child\n\
        *CHI:\tsee the chalk .\n%mor:\tv|see det:art|the n|chalk .\n\
        %gra:\t1|0|ROOT\n@End\n";

    #[test]
    fn reads_speakers_and_roles() {
        let i = inspect_bytes(SENZA_MOR);
        assert!(i.is_chat);
        assert!(!i.has_mor);
        assert_eq!(i.speaker_codes(), ["CHI", "MOT"]);
        assert_eq!(i.speakers[0].role.as_deref(), Some("Target_Child"));
        assert_eq!(i.speakers[1].role.as_deref(), Some("Mother"));
        assert_eq!(i.languages, ["eng"]);
    }

    #[test]
    fn recognises_dependent_tiers() {
        let i = inspect_bytes(CON_MOR);
        assert!(i.has_mor, "the %mor tier was not recognised");
        assert!(i.has_gra);
        assert_eq!(i.languages, ["eng", "spa"]);
        assert_eq!(i.speaker_codes(), ["CHI"]);
    }

    #[test]
    fn a_plain_text_file_does_not_become_a_transcript() {
        let i = inspect_bytes("just some text\nand another line\n");
        assert!(!i.is_chat);
        assert!(i.speakers.is_empty());
    }

    #[test]
    fn a_participant_with_no_role_does_not_break_the_read() {
        let i = inspect_bytes("@UTF8\n@Begin\n@Participants:\tCHI\n@End\n");
        assert_eq!(i.speaker_codes(), ["CHI"]);
        // With no second field there is no role: better no subtitle than
        // repeating the code and faking information that is not there.
        assert_eq!(i.speakers[0].role, None);
    }

    #[test]
    fn a_nonexistent_path_does_not_panic() {
        let i = inspect(Path::new("/non/esiste/affatto.cha"));
        assert!(!i.is_chat && i.speakers.is_empty());
    }
}
