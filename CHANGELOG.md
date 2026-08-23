# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Audio and video downloads.** The archive serves recordings from a separate
  host and never puts them in the corpus zip, so they are now fetched as their
  own transfers: a switch on the corpus page, a tick box in the "Download all"
  confirmation, and **Get the media** for a corpus already on disk. Files land
  next to their transcripts, where CLAN and Batchalign look for them.
- The size is estimated by sampling the corpus rather than from a constant.
  Audio runs 0.5-68 MB per file and video 203-963 MB, and between corpora the
  spread is twenty-five fold, so a general figure would be worthless. The
  estimate is shown before anything is committed to.
- Transcripts now take priority over recordings in the download queue: on a large
  branch all the text arrives first, and the media follow.
- `@Media` is read from each transcript, and is the authority on which file to
  fetch.
- A new preference, "Include audio and video by default".

## [0.1.1] - 2026-08-23

### Fixed

- **The 70 CLAN programs no longer go into `bin/`.** Three of them are named
  after standard Unix commands — `uniq` (coreutils), `script` (util-linux) and
  `gem` (RubyGems) — so installing them onto the PATH shadowed those commands.
  The visible symptom was CLAN's `uniq` banner printed on every new shell,
  because a shell startup file called `uniq`. They now live in
  `<prefix>/libexec/talkbank`, off the PATH; the app finds them by itself, and
  the README explains how to add them to a shell deliberately. `install.sh`
  cleans up the stray copies an earlier install left in `bin/`.
- **The desktop entry now works from a `~/.local` install.** Its `Exec` line
  carries the absolute path to the binary — `~/.local/bin` is not on the PATH of
  every desktop session — and the installer refreshes the desktop database and
  the icon cache, so the launcher actually appears.

### Added

- `uninstall.sh`, runnable the same way as the installer. It removes the
  installed files and the source clone, keeps your settings and your downloaded
  corpora, and takes `PURGE=1` to drop the settings and cache too.

## [0.1.0] - 2026-08-23

First public release.

### Added

- **Transcript editor** for CHAT files, with per-line colouring and live format
  checking through [chatter](https://github.com/TalkBank/chatter). Nothing like
  it existed on Linux: CLAN's own editor is macOS and Windows only.
- **Analyses**: all 70 CLAN programs, listed by goal rather than by name, with
  their requirements (`%mor` tier, speaker, language) checked *before* running.
  The equivalent command line is always shown and can be copied.
- **TalkBank archive**: browse all fifteen banks, search, filter by metadata,
  and download. "Download all" plans a whole branch, shows what it is about to
  fetch, and downloads through a background queue that survives changing section.
- **Batchalign3 integration** (optional) for automatic transcription, media
  alignment, and `%mor`/`%gra` in roughly 26 languages.
- Italian and English translations.

### Notes

- Licensed GPL-2.0, matching the upstream CLAN sources it ships.
- Requires a free TalkBank account to download data; browsing needs nothing.

[Unreleased]: https://github.com/matteospanio/talkbank/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/matteospanio/talkbank/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/matteospanio/talkbank/releases/tag/v0.1.0
