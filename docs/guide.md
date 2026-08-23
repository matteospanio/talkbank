# A guide to TalkBank and CLAN

This guide is about understanding *what* you are looking at and *why*, not only
where to click. It assumes you have never used CLAN.

---

## 1. What CHILDES is

CHILDES is a public archive of speech transcripts. It began as a way to study
how children learn to talk and has grown to cover aphasia, dementia,
bilingualism and more. It is part of **TalkBank**, the Carnegie Mellon project
that hosts it.

CHILDES is **one of the fifteen banks** in TalkBank. The others cover other
areas, and the app browses and downloads all of them:

| bank | what it holds | transcripts |
|---|---|---|
| CHILDES | child language | 53,431 |
| PhonBank | phonological development | 12,817 |
| SLABank | second languages | 10,631 |
| DementiaBank | dementia | 7,919 |
| CABank | conversation analysis | 6,555 |
| AphasiaBank | aphasia | 6,301 |
| HomeBank | home recordings | 2,422 |
| FluencyBank | fluency and stuttering | 2,346 |
| TBIBank | traumatic brain injury | 1,116 |
| PsychosisBank | psychosis | 979 |
| ASDBank | autism | 931 |
| BilingBank | bilingualism | 727 |
| ClassBank | classrooms | 468 |
| RHDBank | right hemisphere damage | 317 |
| SamtaleBank | conversation (Norwegian) | 92 |

**The depth is not the same everywhere.** In CHILDES and PhonBank there is a
collection above the corpus (`childes/Eng-NA/Brown`); in CABank, ClassBank,
BilingBank, FluencyBank and SamtaleBank the corpus sits directly under the bank
(`ca/ATC`). That is why the app shows a tree of folders and asks the server which
ones are downloadable, instead of guessing.

A **corpus** is the material of a single research project: usually a few children
followed for months or years. The *Brown* corpus, for example, holds the historic
recordings of Adam, Eve and Sarah — 214 transcripts in three subfolders, one per
child.

Every corpus must be **cited** when used in a publication. In the app you will
find the reference on the corpus page, with a button to copy it.

### Access

- **Catalogue and documentation**: open, nothing needed.
- **Almost all the data**: a free account is required. You register at
  [talkbank.org](https://talkbank.org/) with an email address and a password.
- **A few corpora** (GlobalTales, some video) need written authorisation, which
  you request from `macw@cmu.edu`.

In the app the account goes in **Preferences → TalkBank account**, where there is
also a **Test the connection** button. The password goes into the system keyring,
never into a configuration file.

---

## 2. The CHAT format

A transcript is a text file with a `.cha` extension. There are three kinds of
line:

```
@UTF8                                          ← headers: @ at the start
@Languages:	eng
@Participants:	CHI Nicky Target_Child, MOT Kelly Mother
@ID:	eng|sample|CHI|1;10.04|female|||Target_Child|||

*CHI:	see the chalk .                        ← speaker line: *CODE:
%mor:	v|see det:art|the n|chalk .            ← dependent tier: %name:
%gra:	1|2|SUBJ 2|0|ROOT 3|2|OBJ
```

- The **headers** (`@`) describe the recording: language, participants, the
  child's age, linked media.
- The **speaker lines** (`*CHI:`) are what was said. The three-letter code
  identifies the speaker and is declared in `@Participants`.
- The **dependent tiers** (`%`) are annotations on the line above:

  | tier   | contains                                     | used for         |
  |--------|----------------------------------------------|------------------|
  | `%mor` | part of speech and morphemes of every word   | MLU, DSS, IPSyn, profiles |
  | `%gra` | syntactic relations between words            | syntactic analyses |
  | `%pho` | phonetic transcription                       | phonological studies |
  | `%err` | error coding                                 | clinical studies |

**The most important thing to know**: the `%mor` tier is almost never present in
freshly transcribed files, and without it many analyses have nothing to count. It
is the number one cause of CLAN's baffling errors, and the app warns you before
running.

---

## 3. Using the app

One window, four sections in the sidebar.

### Start

What you see on opening: **pick up where you left off** (the last folder, with
the analysis you had chosen), your recent transcripts and folders, and four ways
to begin — open a folder, download a corpus, transcribe audio, run an analysis.

The first time round, in place of the recents, you get a short explanation of
what a CHAT transcript is and in what order to do things.

### Transcripts

The editor. Files of the working folder on the left, the text in the middle, and
the **format check** below.

The text is coloured by line type: `@` headers in bold, `*` speaker lines
highlighted, `%` annotations dimmed. You edit and save (`Save` only appears when
there is something to save); closing or switching files with unsaved changes asks
first.

The checking is done by **chatter**, with an error code, a line number and a
suggested fix. Clicking a problem jumps the cursor to it, and errors are
underlined in the text. The **Analyse this** button takes you to the analyses with
that file already selected.

### The analyses (sidebar)

The commands are listed **by goal**, not by name: "Count the words" with `freq`
underneath. The **"Start here"** group holds the six needed most often — the five
the manual names as basic (`freq`, `mlu`, `kwal`, `combo`, `gem`) plus `check`.

The flow is always the same:

1. choose the **working folder** (the folder button at the top)
2. tick the **files** to analyse
3. choose the **command** from the sidebar
4. set the **options** — the speakers appear on their own, read from
   `@Participants`
5. press **Run**, or `Ctrl+Enter`

Under the Run button there is always the corresponding **command line**, with a
button to copy it. That is not decoration: it is the same line you would type in
a terminal, so the interface teaches you the CLI instead of hiding it.

### The archive (`Ctrl+B`, or from the sidebar)

Browse all fifteen banks: bank → folders → corpus detail. The catalogue is
public, so this works without an account; one is only needed to download.

The button at the bottom of the page changes with what you are looking at:

- on a **corpus**, "Download" takes that corpus;
- on a **collection**, or any node that contains others, "Download all" takes
  **every corpus under that node**, at whatever depth. You do not have to open
  one leaf at a time;
- **inside** a corpus (`Brown/Adam`, say), "Find the corpus" tells you which
  corpus those files belong to and takes you there;
- on restricted banks (AphasiaBank, SamtaleBank, PsychosisBank and a few
  scattered corpora) it tells you who to ask for authorisation.

**How "Download all" works.** Only the server knows which folder is a corpus, one
request at a time, so the branch is surveyed first — you will see "N folders
checked" — and then a confirmation arrives saying how many corpora, how many
transcripts, roughly how much they weigh, where they will land, and what is left
out (anything needing authorisation, and anything that could not be verified).
Nothing starts before that confirmation.

Two things the confirmation says that are worth reading: the size estimate is
**approximate** (23 KB per transcript, measured over four corpora), and **media
are not included** — the zip holds transcripts only. Anything already on disk is
skipped, and if you would rather fetch it again there is "Download again".

On a very large branch the survey stops after five hundred folders and says so:
"Keep looking" resumes from where it was, without paying again for the requests
already made.

**Downloads carry on in the background**, as in a browser: you can change section,
carry on analysing, and the down-arrow button in the archive header shows how many
are left. It is a **queue**: two at a time, not forty connections at once. From
there you can cancel one job or all of them.

When the queue drains a system notification arrives — one per group, not one per
corpus — unless you are already looking at the archive, where a message at the
bottom of the window is enough. The message **reconciles the count**: if you asked
for 24 corpora and 23 arrived, it says so ("23 of 24 corpora downloaded") and
stays on screen until dismissed instead of vanishing after a few seconds. A
shortfall also goes into the log, so you can find it again even if you were not
at the screen.

If your session expires halfway, or the disk fills up, the queue **pauses**
instead of cancelling: sign back in and it resumes from where it was. A corpus
arrives whole or not at all: extraction happens in a temporary folder and is moved
into place only when the work is finished, so a folder that exists is complete,
and an interruption leaves nothing half-done.

On a corpus page you will find what it contains, the citation to use, the metadata
(languages, study type, activity, group) and the participants — the last of these
only on request, because on a large corpus that call can take half a minute.

**Download** puts the corpus in `<destination>/<bank>/<…>/<corpus>/` — the bank is
part of the path, so CHILDES's `Eng-NA` and PhonBank's `Eng-NA` do not get mixed
up. The download-finished message has an **Open** button that points the
transcripts at the folder just downloaded. It does not change the working folder
by itself: if you were analysing something else, you would find it changed under
your hands.

The **metadata filter** (language, study type, group, presence of media) needs an
index, because the archive answers one folder at a time. It is built one bank at
a time, takes a few minutes, and is saved. It needs a session: without one the
server answers the same way for every folder and the index would come back empty.

### Media and transcription (Batchalign)

Automatic transcription from audio, alignment to media, creation of `%mor` and
`%gra` for about 26 languages, segmentation, translation.

Batchalign is a **separate and optional** TalkBank tool: if it is not installed
the app says so and gives you the command to install it. CLAN works in full
without it.

---

## 4. A typical path, from the start

**I want to compute the MLU of the children in a corpus.**

1. `Ctrl+B` → choose the bank → find the corpus → **Download** → **Open**
2. Choose **"Check that the file is valid"** (`check`) and press Run. Always
   worth it: a format error skews every analysis that follows.
3. Choose **"Mean length of utterance (MLU)"**. If the warning *"These files have
   no %mor tier"* appears, press **What can I do?**:
   - *Create it with MOR* — if the language is English, French, Spanish or
     Chinese;
   - *Create it with Batchalign* — for every other language;
   - *Count words* — MLU computed in words rather than morphemes, which is fine
     for many purposes as long as you say so.
4. Under **"Who to analyse"** tick the child (usually `*CHI`).
5. Run. If you need the result in a file, turn on *"Save the result to a file
   too"*: a `.cex` will appear next to the data.

---

## 5. The most used commands

| goal | command | needs |
|---|---|---|
| Count words, type/token ratio | `freq` | — |
| Mean length of utterance | `mlu` | `%mor` |
| Mean length of turn | `mlt` | — |
| Find a word in context | `kwal` | — |
| Search for combinations (`^` = followed by) | `combo` | — |
| Check the format | `check` | — |
| Full profile of the child | `kideval` | `%mor`, language |
| Lexical diversity (D) | `vocd` | `%mor` |
| Developmental Sentence Score | `dss` | `%mor`, speaker, language |
| Create `%mor` | `mor` | grammar for the language |

The app checks the requirements **before** running: if something is missing, the
Run button stays off and a message says what is needed.

Three traps worth knowing, all exposed as switches in the options:

- **repetitions are excluded** by default (`+r6` includes them);
- **the result goes to the screen**, not to a file, unless you ask for `+f`;
- **several files are analysed separately**, unless you ask for `+u`.

---

## 6. Where to look when something does not add up

- **Messages** (the tab next to Output) holds the analysis header and the
  diagnostics: CLAN writes there what it actually analysed.
- **All the options** (the ⓘ next to the example) shows the program's own
  documentation, so it always matches the installed version.
- **Recent commands** (`Ctrl+H`) retraces what you have run.
- The complete manuals are `CLAN.pdf` (the programs) and `CHAT.pdf` (the format),
  both from [talkbank.org](https://talkbank.org/manuals/).

---

## 7. From the command line

The 70 CLAN programs are the original ones and work without the interface too:

```sh
build/freq +t*CHI 0042.cha
export PATH="$PWD/build:$PATH"     # to have them always at hand
```

The line the app shows is exactly this one, so it can be copied and pasted into a
script to repeat the same analysis over many corpora.
