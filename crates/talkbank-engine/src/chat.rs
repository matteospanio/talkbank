//! Reading the header of a CHAT file.
//!
//! Three pieces of information keep the user from making a mistake before an
//! analysis: which speakers are present (from `@Participants`), whether a
//! `%mor` tier exists (without it MLU, DSS and IPSyn have nothing to count),
//! and which recording the transcript belongs to (from `@Media`).

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speaker {
    /// Code used on the speaker lines: CHI, MOT, INV…
    pub code: String,
    /// Declared role: Target_Child, Mother, Investigator…
    pub role: Option<String>,
}

/// The recording a transcript belongs to, from its `@Media` header.
///
/// The five flags are the same vocabulary the archive catalogue uses, but the
/// type is deliberately not shared: it lives in `talkbank-archive`, which this
/// crate does not depend on, and five booleans cost less than that dependency.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaRef {
    /// Name without extension. Usually the transcript's own stem, but the
    /// format does not promise it, so this is the authority.
    pub basename: String,
    /// `false` means audio.
    pub video: bool,
    /// The recording is declared but not held by the archive: do not ask.
    pub missing: bool,
    /// The media exists but the transcript is not time-aligned to it.
    pub unlinked: bool,
    /// The media exists but has no transcription attached.
    pub notrans: bool,
}

impl MediaRef {
    /// True when it is worth asking the server for this file.
    pub fn is_fetchable(&self) -> bool {
        !self.missing && !self.basename.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct FileInfo {
    pub is_chat: bool,
    pub has_mor: bool,
    pub has_gra: bool,
    pub languages: Vec<String>,
    pub speakers: Vec<Speaker>,
    /// The recording, when the file declares one.
    pub media: Option<MediaRef>,
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
        if let Some(rest) = line.strip_prefix("@Media:") {
            info.media = parse_media(rest);
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

/// `@Media:	adam01, audio` — sometimes with extra flags, as in
/// `@Media:	adam01, audio, unlinked`.
///
/// The first item is the filename without extension; the rest are the same
/// flags the catalogue uses. An unknown flag is ignored rather than rejected:
/// a header we do not fully understand still tells us which file to fetch.
fn parse_media(rest: &str) -> Option<MediaRef> {
    let mut items = rest.split(',').map(str::trim);
    let basename = items.next().unwrap_or("").to_string();
    if basename.is_empty() {
        return None;
    }
    let mut media = MediaRef {
        basename,
        ..Default::default()
    };
    for flag in items {
        match flag {
            "video" => media.video = true,
            "audio" => media.video = false,
            "missing" => media.missing = true,
            "unlinked" => media.unlinked = true,
            "notrans" => media.notrans = true,
            _ => {}
        }
    }
    Some(media)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITHOUT_MOR: &str = "@UTF8\n@Begin\n@Languages:\teng\n\
        @Participants:\tCHI Nicky Target_Child, MOT Kelly Mother\n\
        *CHI:\tsee the chalk .\n*MOT:\tyes .\n@End\n";

    const WITH_MOR: &str = "@UTF8\n@Begin\n@Languages:\teng, spa\n\
        @Participants:\tCHI Nicky Target_Child\n\
        *CHI:\tsee the chalk .\n%mor:\tv|see det:art|the n|chalk .\n\
        %gra:\t1|0|ROOT\n@End\n";

    #[test]
    fn reads_speakers_and_roles() {
        let i = inspect_bytes(WITHOUT_MOR);
        assert!(i.is_chat);
        assert!(!i.has_mor);
        assert_eq!(i.speaker_codes(), ["CHI", "MOT"]);
        assert_eq!(i.speakers[0].role.as_deref(), Some("Target_Child"));
        assert_eq!(i.speakers[1].role.as_deref(), Some("Mother"));
        assert_eq!(i.languages, ["eng"]);
    }

    #[test]
    fn recognises_dependent_tiers() {
        let i = inspect_bytes(WITH_MOR);
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
    fn the_media_header_gives_the_recording_and_its_flags() {
        let plain = inspect_bytes("@UTF8\n@Media:\tadam01, audio\n").media.unwrap();
        assert_eq!(plain.basename, "adam01");
        assert!(!plain.video && !plain.missing && !plain.unlinked);
        assert!(plain.is_fetchable());

        let video = inspect_bytes("@UTF8\n@Media:\t020408, video\n").media.unwrap();
        assert!(video.video);

        // Extra flags ride along after the kind.
        let un = inspect_bytes("@UTF8\n@Media:\te01, audio, unlinked\n").media.unwrap();
        assert!(un.unlinked && !un.missing);
        assert!(un.is_fetchable(), "unlinked means unaligned, not absent");

        // `missing` is the one case where asking the server is pointless.
        let gone = inspect_bytes("@UTF8\n@Media:\tx, audio, missing\n").media.unwrap();
        assert!(gone.missing);
        assert!(!gone.is_fetchable());
    }

    #[test]
    fn a_transcript_without_media_says_so() {
        assert!(inspect_bytes(WITHOUT_MOR).media.is_none());
        // A header with nothing after it is not a recording called "".
        assert!(inspect_bytes("@UTF8\n@Media:\t\n").media.is_none());
        assert!(inspect_bytes("@UTF8\n@Media:\t,,,\n").media.is_none());
    }

    #[test]
    fn an_unknown_media_flag_does_not_lose_the_filename() {
        // The header still tells us which file to fetch, which is the part we
        // actually need.
        let m = inspect_bytes("@UTF8\n@Media:\tadam01, audio, somethingnew\n")
            .media
            .unwrap();
        assert_eq!(m.basename, "adam01");
        assert!(m.is_fetchable());
    }

    #[test]
    fn a_nonexistent_path_does_not_panic() {
        let i = inspect(Path::new("/non/esiste/affatto.cha"));
        assert!(!i.is_chat && i.speakers.is_empty());
    }
}
