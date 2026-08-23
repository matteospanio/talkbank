# TalkBank

A desktop client for [TalkBank](https://talkbank.org/) on Linux: open and correct
CHAT transcripts, analyse them with the original CLAN programs, and browse and
download the corpora of all fifteen TalkBank banks.

[![License: GPL v2](https://img.shields.io/badge/license-GPL--2.0-blue.svg)](LICENSE)

One window, four sections: **Start** (resume your work), **Transcripts** (editor
with format checking), **Analyses** (the 70 CLAN programs plus the Batchalign
tasks), **Archive** (browse and download TalkBank — transcripts, and the audio
and video too when you ask for them).

> **Not affiliated with Carnegie Mellon University.** CLAN and the TalkBank
> archive are the work of Brian MacWhinney and the TalkBank project; this is an
> independent Linux front-end for them.

## Install

```sh
curl -LsSf https://raw.githubusercontent.com/matteospanio/talkbank/main/install.sh | sh
```

The script checks for the build dependencies, clones the repository, builds it,
installs into `~/.local` and registers the launcher, so **TalkBank** appears in
your applications menu. It builds from source on purpose: GTK 4 and libadwaita
binaries do not travel well between distributions.

To install elsewhere, or from a checkout you already have:

```sh
meson setup build --prefix="$HOME/.local"
meson compile -C build
meson install -C build
```

### Uninstall

```sh
curl -LsSf https://raw.githubusercontent.com/matteospanio/talkbank/main/uninstall.sh | sh
```

It removes the installed files and the source clone, and leaves your settings and
your downloaded corpora alone. Add `PURGE=1` to drop the settings and cache too.
From a checkout, `ninja -C build uninstall` does the same job.

Requirements: `g++`, `meson`, `ninja`, `gettext`, `cargo`, `libgtk-4-dev`,
`libadwaita-1-dev`. On Debian and Ubuntu:

```sh
sudo apt install build-essential meson ninja-build gettext \
                 libgtk-4-dev libadwaita-1-dev cargo
```

## Use

```sh
talkbank                      # reopens the last folder and analysis
talkbank /path/to/corpus      # opens a folder
talkbank transcript.cha       # opens a file and selects it
```

Shortcuts: `Ctrl+Enter` runs an analysis, `Ctrl+B` opens the archive, `Ctrl+,`
preferences, `Ctrl+H` recent commands.

### The CLAN programs

The 70 CLAN programs are installed in `~/.local/libexec/talkbank`, **deliberately
off your PATH**: three of them are named after standard Unix commands — `uniq`
(coreutils), `script` (util-linux) and `gem` (RubyGems) — and putting them on the
PATH would shadow those for everything you run. The app finds them by itself.

To use them from a shell, **append** that directory so the system commands keep
winning:

```sh
echo 'export PATH="$PATH:$HOME/.local/libexec/talkbank"' >> ~/.profile
freq +t*CHI 0042.cha
```

### An account

The catalogue and the documentation are open. Almost all the *data* needs a free
account, which you create at [talkbank.org](https://talkbank.org/) and enter under
**Preferences → TalkBank account**. The password goes into the system keyring,
never into a configuration file.

## Documentation

- **[User guide](docs/guide.md)** — what CHILDES and the CHAT format are, what the
  analyses do, how the archive is organised, and a worked example from start to
  finish. Start here if you have never used CLAN.
- **[Development](docs/development.md)** — building, testing, translating, and the
  design choices behind the code.
- **[Notes on the TalkBank service](docs/talkbank-api.md)** — what we measured
  about the live API, and why the archive code is shaped the way it is.
- **[Changelog](CHANGELOG.md)**

The complete CLAN and CHAT manuals are `CLAN.pdf` and `CHAT.pdf`, downloadable
from [talkbank.org/manuals](https://talkbank.org/manuals/).

## What it is made of

The analyses are run by the **real CLAN binaries** — byte-exact, because they
*are* CLAN. The CHAT format is read by [chatter](https://github.com/TalkBank/chatter),
TalkBank's own authority on the format. Automatic transcription and morphosyntax
go through [Batchalign3](https://github.com/TalkBank/batchalign2), which is
optional: everything else works without it.

The editor is the part that did not exist on Linux before: CLAN's own editor is
macOS and Windows only, and `chatter-desktop` validates but does not edit.

## Licence

GPL-2.0. The CLAN sources under `src/` and `lib/` are
Copyright 1990-2026 Brian MacWhinney, distributed under the GNU General Public
License version 2 — see [LICENSE](LICENSE).
