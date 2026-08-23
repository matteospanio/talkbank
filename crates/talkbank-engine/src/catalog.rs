//! The 70 CLAN programs described by goal, with the requirements the UI
//! checks *before* running anything.
//!
//! The texts are English and get translated at runtime. They are identical to
//! the ones in the C version, so the existing catalogues in `po/` still apply.
//!
//! They were migrated from the original C client keeping the msgids byte for
//! byte, which is why the translations kept working. This file is now the
//! source of truth: a test checks that every title and description also shows
//! up in `po/talkbank.pot`.

/// What a command needs, checked against the selected files before running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Req(u8);

impl Req {
    pub const NONE: Req = Req(0);
    /// Needs a `%mor` tier in the chosen files.
    pub const MOR: Req = Req(1 << 0);
    /// Needs at least one speaker (`+t*XXX`).
    pub const SPEAKER: Req = Req(1 << 1);
    /// Needs the language (`+l`).
    pub const LANG: Req = Req(1 << 2);
    /// The language is accepted but not required.
    pub const LANG_OPT: Req = Req(1 << 3);

    pub const fn union(self, other: Req) -> Req {
        Req(self.0 | other.0)
    }
    pub const fn has(self, other: Req) -> bool {
        self.0 & other.0 != 0
    }
    /// True if the command takes the language option, required or not.
    pub const fn takes_language(self) -> bool {
        self.0 & (Req::LANG.0 | Req::LANG_OPT.0) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Essential,
    Count,
    Profile,
    Search,
    Morph,
    Convert,
    Check,
}

impl Category {
    /// Label, translated at runtime.
    pub const fn label(self) -> &'static str {
        match self {
            Category::Essential => "Start here",
            Category::Count => "Counts and vocabulary",
            Category::Profile => "Measures and profiles",
            Category::Search => "Search",
            Category::Morph => "Morphology",
            Category::Convert => "Convert formats",
            Category::Check => "Check and clean up",
        }
    }
    pub const ALL: [Category; 7] = [
        Category::Essential,
        Category::Count,
        Category::Profile,
        Category::Search,
        Category::Morph,
        Category::Convert,
        Category::Check,
    ];
}

#[derive(Debug, Clone, Copy)]
pub struct Command {
    /// Program name: `freq`, `mlu`, …
    pub name: &'static str,
    pub cat: Category,
    /// What it does, phrased as a goal.
    pub title: &'static str,
    /// Explanation for someone who has never used CLAN.
    pub desc: &'static str,
    /// Example line, taken from the manual where possible.
    pub example: &'static str,
    pub req: Req,
    /// Subdirectory of `lib/` holding the language files, for the `+l` option.
    pub lang_dir: Option<&'static str>,
    /// Option that produces spreadsheet output, where one exists.
    pub sheet_flag: Option<&'static str>,
}

pub static COMMANDS: &[Command] = &[
    Command {
        name: "freq",
        cat: Category::Essential,
        title: "Count words",
        desc: "Counts how many times each word appears and computes the type/token ratio, the usual measure of vocabulary variety.",
        example: "freq +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: Some("+d2"),
    },
    Command {
        name: "mlu",
        cat: Category::Essential,
        title: "Mean length of utterance (MLU)",
        desc: "Average number of morphemes per utterance, the classic index of grammatical development. It counts on the %mor tier, so the file must have been run through MOR first.",
        example: "mlu +t*CHI 0042.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: Some("+d"),
    },
    Command {
        name: "kwal",
        cat: Category::Essential,
        title: "Find a word in context",
        desc: "Shows every line containing the word you searched for, together with the surrounding utterances, so you can read it in context.",
        example: "kwal +sbunny -w2 +w2 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "combo",
        cat: Category::Essential,
        title: "Search word combinations",
        desc: "Finds sequences and combinations of words. Use ^ for \"immediately followed by\", + for \"or\": +s\"kitty^kitty\" finds kitty said twice in a row.",
        example: "combo +t*MOT +s\"kitty^kitty\" 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "gem",
        cat: Category::Essential,
        title: "Extract marked sections",
        desc: "Pulls out the parts of the transcript marked with @bg and @eg headers, for example a single activity or task, so you can analyse it separately.",
        example: "gem +sbook 0012.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "check",
        cat: Category::Essential,
        title: "Check the file is valid",
        desc: "Verifies that the transcript follows the CHAT rules and reports every error with its line number. Run this before any analysis.",
        example: "check 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "freqpos",
        cat: Category::Count,
        title: "Word frequency by position",
        desc: "Counts words according to where they sit in the utterance: first, second, last. Useful for studying word order.",
        example: "freqpos +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "gemfreq",
        cat: Category::Count,
        title: "Word frequency inside each section",
        desc: "Like counting words, but reported separately for each marked section (gem).",
        example: "gemfreq +t*CHI +sbook 0012.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "phonfreq",
        cat: Category::Count,
        title: "Sound frequency",
        desc: "Counts the phonemes on the %pho tier and shows where in the word each one occurs.",
        example: "phonfreq +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "wdlen",
        cat: Category::Count,
        title: "Length histograms",
        desc: "Distribution of the length of words, utterances and turns, in characters and morphemes.",
        example: "wdlen +t*CHI 0042.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "maxwd",
        cat: Category::Count,
        title: "Longest words and utterances",
        desc: "Finds the longest words, or with +g2 the longest utterances. Used for the MLU of the five longest utterances (MLU5).",
        example: "maxwd +g2 +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "vocd",
        cat: Category::Count,
        title: "Lexical diversity (VOCD)",
        desc: "Computes the D index, a measure of vocabulary richness that, unlike the type/token ratio, does not depend on sample length.",
        example: "vocd +t*CHI 0042.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: Some("+d3"),
    },
    Command {
        name: "mortable",
        cat: Category::Count,
        title: "Table of parts of speech",
        desc: "Spreadsheet with how often each part of speech and each bound morpheme is used.",
        example: "mortable +leng +t*CHI 0042.cha",
        req: Req::MOR.union(Req::LANG),
        lang_dir: Some("mortable"),
        sheet_flag: None,
    },
    Command {
        name: "codes",
        cat: Category::Count,
        title: "Table of MC codes",
        desc: "Spreadsheet of the coded categories entered with the coder mode.",
        example: "codes 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "uniq",
        cat: Category::Count,
        title: "Repeated lines",
        desc: "Reports lines that appear more than once, handy for cleaning up word lists.",
        example: "uniq sample.frq.cex",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "mlt",
        cat: Category::Profile,
        title: "Mean length of turn (MLT)",
        desc: "Counts utterances, turns and words per speaker, and their ratios. Unlike MLU it also counts lines with unintelligible material (xxx).",
        example: "mlt +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: Some("+d"),
    },
    Command {
        name: "kideval",
        cat: Category::Profile,
        title: "Full child language profile",
        desc: "Runs a whole battery of measures at once (MLU, DSS, IPSyn, vocabulary, errors) and compares the child against a reference database. The best starting point for a clinical profile.",
        example: "kideval +t*CHI +leng *.cha",
        req: Req::MOR.union(Req::LANG_OPT),
        lang_dir: Some("kideval"),
        sheet_flag: None,
    },
    Command {
        name: "dss",
        cat: Category::Profile,
        title: "Developmental Sentence Score",
        desc: "Assigns the DSS score, which weights the grammatical structures used in 50 sentences. Needs a speaker and a language.",
        example: "dss +t*CHI +leng 0042.cha",
        req: Req::MOR.union(Req::SPEAKER.union(Req::LANG)),
        lang_dir: Some("dss"),
        sheet_flag: None,
    },
    Command {
        name: "ipsyn",
        cat: Category::Profile,
        title: "Index of Productive Syntax",
        desc: "Scores the variety of syntactic structures produced, adding a %syn tier to the file.",
        example: "ipsyn +leng +t*CHI 0042.cha",
        req: Req::MOR.union(Req::LANG),
        lang_dir: Some("ipsyn"),
        sheet_flag: None,
    },
    Command {
        name: "flucalc",
        cat: Category::Profile,
        title: "Fluency and disfluency measures",
        desc: "Counts repetitions, revisions, filled pauses and blocks, the measures used in stuttering research.",
        example: "flucalc +t*PAR *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "sugar",
        cat: Category::Profile,
        title: "SUGAR measures",
        desc: "MLU, total number of words, clauses per sentence and words per sentence, following the SUGAR protocol.",
        example: "sugar +t*CHI *.cha",
        req: Req::MOR.union(Req::SPEAKER),
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "eval",
        cat: Category::Profile,
        title: "Summary profile with comparison",
        desc: "Spreadsheet with duration, MLU, type/token ratio, error percentages, parts of speech, repetitions and revisions, compared against a reference database.",
        example: "eval +t*PAR +u *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "eval-d",
        cat: Category::Profile,
        title: "Summary profile, file by file",
        desc: "Same measures as the summary profile, but reported separately for each file.",
        example: "eval-d +t*PAR *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "timedur",
        cat: Category::Profile,
        title: "Speech duration",
        desc: "Uses the timestamps linked to the media to compute words and utterances per minute of speech.",
        example: "timedur +t*PAR +d10 *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: Some("+d10"),
    },
    Command {
        name: "chip",
        cat: Category::Profile,
        title: "Parent-child exchanges",
        desc: "Measures how much each speaker repeats, expands or changes what the other just said. Needs the two speakers given with +b (source) and +c (response).",
        example: "chip +bMOT +cCHI chip.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "keymap",
        cat: Category::Profile,
        title: "What follows what",
        desc: "Contingency table showing which code tends to follow which other code.",
        example: "keymap +b%spa 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "dist",
        cat: Category::Profile,
        title: "Average distance between items",
        desc: "How many utterances typically separate two occurrences of the same word or code.",
        example: "dist +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "cooccur",
        cat: Category::Profile,
        title: "Words occurring together",
        desc: "Lists the clusters of words that recur next to each other.",
        example: "cooccur +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "chains",
        cat: Category::Profile,
        title: "Chains of codes across turns",
        desc: "Follows a code through the conversation to see how long it stays active.",
        example: "chains +t%spa chains.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "rely",
        cat: Category::Profile,
        title: "Transcription reliability",
        desc: "Compares two transcriptions of the same recording and reports where they disagree.",
        example: "rely file1.cha file2.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "gemlist",
        cat: Category::Search,
        title: "List the sections of a file",
        desc: "Shows the structure of the gems in the transcript, to see what can be extracted.",
        example: "gemlist 0012.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "repeat",
        cat: Category::Search,
        title: "Repetitions between speakers",
        desc: "Finds where one speaker repeats what another just said.",
        example: "repeat +tCHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "retrace",
        cat: Category::Search,
        title: "Mark reformulations",
        desc: "Adds a %ret tier coding retracings and self-corrections.",
        example: "retrace +t*CHI 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "modrep",
        cat: Category::Search,
        title: "Compare target and actual pronunciation",
        desc: "Matches the words on the %mod tier against how they were actually produced on %pho.",
        example: "modrep +b*CHI +c%pho modrep.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "longtier",
        cat: Category::Search,
        title: "Join wrapped tiers",
        desc: "Puts tiers split over several lines back onto a single line.",
        example: "longtier 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "lines",
        cat: Category::Search,
        title: "Number the lines",
        desc: "Adds line numbers, useful when reporting errors to a colleague.",
        example: "lines 0042.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "mor",
        cat: Category::Morph,
        title: "Morphological analysis (creates %mor)",
        desc: "Adds the %mor tier with part of speech and morphemes for every word. This is the step that unlocks MLU, DSS, IPSyn and the profiles. Needs a MOR grammar for the language of the transcript.",
        example: "mor +leng *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "post",
        cat: Category::Morph,
        title: "Disambiguate %mor",
        desc: "Where MOR proposed several analyses for a word, POST picks the right one using the context.",
        example: "post +leng *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "megrasp",
        cat: Category::Morph,
        title: "Grammatical relations (creates %gra)",
        desc: "Adds the %gra tier with the syntactic dependencies between words: subject, object, modifier.",
        example: "megrasp *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "postmortem",
        cat: Category::Morph,
        title: "Fix leftover errors after POST",
        desc: "Corrects the analyses POST could not resolve.",
        example: "postmortem *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "trnfix",
        cat: Category::Morph,
        title: "Compare %trn with %mor",
        desc: "Reports where the manual coding on %trn disagrees with the automatic %mor.",
        example: "trnfix *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "postlist",
        cat: Category::Morph,
        title: "Inspect a POST database",
        desc: "Prints the contents of a trained POST database.",
        example: "postlist eng.db",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "posttrain",
        cat: Category::Morph,
        title: "Train a POST database",
        desc: "Builds a new disambiguation database from files already disambiguated by hand.",
        example: "posttrain +leng *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "postmodrules",
        cat: Category::Morph,
        title: "Apply POST modification rules",
        desc: "Runs the rule file that adjusts POST output.",
        example: "postmodrules +leng",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "compound",
        cat: Category::Morph,
        title: "Handle compound words",
        desc: "Prepares the compound word list POST uses.",
        example: "compound +leng",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "makemod",
        cat: Category::Morph,
        title: "Create the %mod tier",
        desc: "Derives the target pronunciation tier %mod from %pho.",
        example: "makemod +t*CHI *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "chat2elan",
        cat: Category::Convert,
        title: "CHAT to Elan",
        desc: "Converts a transcript to Elan XML, for annotating video.",
        example: "chat2elan *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "elan2chat",
        cat: Category::Convert,
        title: "Elan to CHAT",
        desc: "Imports an Elan annotation as a CHAT transcript.",
        example: "elan2chat *.eaf",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "chat2praat",
        cat: Category::Convert,
        title: "CHAT to Praat",
        desc: "Produces a Praat TextGrid for acoustic analysis.",
        example: "chat2praat *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "praat2chat",
        cat: Category::Convert,
        title: "Praat to CHAT",
        desc: "Imports a Praat TextGrid as a CHAT transcript.",
        example: "praat2chat *.textGrid",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "lena2chat",
        cat: Category::Convert,
        title: "LENA to CHAT",
        desc: "Imports the XML produced by a LENA recorder.",
        example: "lena2chat *.its",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "lipp2chat",
        cat: Category::Convert,
        title: "LIPP to CHAT",
        desc: "Imports transcripts in LIPP format.",
        example: "lipp2chat *.lipp",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "salt2chat",
        cat: Category::Convert,
        title: "SALT to CHAT",
        desc: "Imports transcripts in SALT format.",
        example: "salt2chat *.slt",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "srt2chat",
        cat: Category::Convert,
        title: "Subtitles to CHAT",
        desc: "Turns an SRT subtitle file into a transcript with time links.",
        example: "srt2chat *.srt",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "text2chat",
        cat: Category::Convert,
        title: "Plain text to CHAT",
        desc: "Turns a plain text file into a CHAT transcript, adding the required headers.",
        example: "text2chat *.txt",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "play2chat",
        cat: Category::Convert,
        title: "Datavyu to CHAT",
        desc: "Imports coding done in Datavyu.",
        example: "play2chat *.txt",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "dataclean",
        cat: Category::Check,
        title: "Normalise formatting",
        desc: "Fixes spacing and layout so the file matches the current CHAT conventions.",
        example: "dataclean *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "fixit",
        cat: Category::Check,
        title: "Split multi-utterance tiers",
        desc: "Puts each utterance on its own tier when several ended up on one line.",
        example: "fixit *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "fixbullets",
        cat: Category::Check,
        title: "Repair media time links",
        desc: "Rebuilds and realigns the bullets that link the transcript to audio or video.",
        example: "fixbullets *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "delim",
        cat: Category::Check,
        title: "Normalise end-of-utterance marks",
        desc: "Makes sure every utterance ends with a proper delimiter.",
        example: "delim *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "indent",
        cat: Category::Check,
        title: "Indent CA overlaps",
        desc: "Lays out overlapping speech the way Conversation Analysis expects.",
        example: "indent *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "quotes",
        cat: Category::Check,
        title: "Move quoted speech to its own tier",
        desc: "Takes reported speech at the end of a line and puts it on a separate tier.",
        example: "quotes *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "lowcase",
        cat: Category::Check,
        title: "Convert to lower case",
        desc: "Lower-cases every word except proper nouns and the words you list as exceptions.",
        example: "lowcase *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "chstring",
        cat: Category::Check,
        title: "Search and replace in bulk",
        desc: "Applies a list of substitutions across many files at once.",
        example: "chstring +q0changes.cut *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "flo",
        cat: Category::Check,
        title: "Create the plain text tier (%flo)",
        desc: "Adds a simplified version of each utterance, without codes or annotations.",
        example: "flo *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "ort",
        cat: Category::Check,
        title: "Create the orthography tier (%ort)",
        desc: "Adds the standard spelling of what was said.",
        example: "ort *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "script",
        cat: Category::Check,
        title: "Compare against a reference script",
        desc: "Aligns a transcript with the script the speaker was reading, for reading studies.",
        example: "script +leng *.cha",
        req: Req::MOR,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "tierorder",
        cat: Category::Check,
        title: "Reorder dependent tiers",
        desc: "Puts the dependent tiers of every utterance into a consistent order.",
        example: "tierorder *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "dates",
        cat: Category::Check,
        title: "Compute ages and dates",
        desc: "Works out the child's age from the birth date and recording date in the headers.",
        example: "dates *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
    Command {
        name: "combtier",
        cat: Category::Check,
        title: "Merge two dependent tiers",
        desc: "Combines the content of two dependent tiers into one.",
        example: "combtier +t%mor *.cha",
        req: Req::NONE,
        lang_dir: None,
        sheet_flag: None,
    },
];

pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

pub fn by_category(cat: Category) -> impl Iterator<Item = &'static Command> {
    COMMANDS.iter().filter(move |c| c.cat == cat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_seventy_commands_are_present() {
        assert_eq!(COMMANDS.len(), 70);
    }

    #[test]
    fn every_command_has_its_texts_and_a_unique_name() {
        let mut seen = std::collections::HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate name: {}", c.name);
            assert!(!c.title.is_empty(), "{} has no title", c.name);
            assert!(!c.desc.is_empty(), "{} has no description", c.name);
            assert!(
                c.example.starts_with(c.name),
                "the example for {} does not start with the command name: {}",
                c.name,
                c.example
            );
        }
    }

    #[test]
    fn whatever_needs_a_language_also_says_where_to_find_it() {
        for c in COMMANDS.iter().filter(|c| c.req.takes_language()) {
            assert!(
                c.lang_dir.is_some(),
                "{} accepts +l but does not name a language directory",
                c.name
            );
        }
    }

    #[test]
    fn the_starting_group_holds_the_manuals_basic_commands() {
        // The manual (Part 2, ch. 3.3) names KWAL, FREQ, MLU, COMBO and GEM as
        // the five basic commands; we add CHECK because it should run first.
        let essential: Vec<_> = by_category(Category::Essential).map(|c| c.name).collect();
        for want in ["freq", "mlu", "kwal", "combo", "gem", "check"] {
            assert!(essential.contains(&want), "{want} missing from the essentials");
        }
    }

    /// The generator once re-escaped literals lifted from the C source, which put
    /// backslashes on screen. This checks that the quotes in the texts are real
    /// quotes.
    #[test]
    fn the_texts_hold_no_half_finished_escapes() {
        for c in COMMANDS {
            for (field, text) in [("title", c.title), ("description", c.desc)] {
                assert!(
                    !text.contains('\\'),
                    "the {} of {} contains a backslash: {text}",
                    field,
                    c.name
                );
            }
        }
        // and the case that revealed it must have real quotes
        assert!(find("combo").unwrap().desc.contains("\"immediately followed by\""));
    }

    #[test]
    fn the_secondary_fields_survive_generation() {
        let sheet: Vec<_> = COMMANDS
            .iter()
            .filter_map(|c| c.sheet_flag.map(|f| (c.name, f)))
            .collect();
        assert_eq!(
            sheet,
            [("freq", "+d2"), ("mlu", "+d"), ("vocd", "+d3"), ("mlt", "+d"), ("timedur", "+d10")]
        );
        let langs: Vec<_> = COMMANDS
            .iter()
            .filter_map(|c| c.lang_dir.map(|d| (c.name, d)))
            .collect();
        assert_eq!(
            langs,
            [("mortable", "mortable"), ("kideval", "kideval"), ("dss", "dss"), ("ipsyn", "ipsyn")]
        );
    }

    #[test]
    fn mlu_needs_mor_and_freq_does_not() {
        assert!(find("mlu").unwrap().req.has(Req::MOR));
        assert!(!find("freq").unwrap().req.has(Req::MOR));
        // dss is the only entry with three requirements at once
        let dss = find("dss").unwrap();
        assert!(dss.req.has(Req::MOR) && dss.req.has(Req::SPEAKER) && dss.req.has(Req::LANG));
    }
}
