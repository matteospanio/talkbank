# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/matteospanio/talkbank/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/matteospanio/talkbank/releases/tag/v0.1.0
