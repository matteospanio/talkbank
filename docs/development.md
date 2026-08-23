# Development

How the project is built and how it is put together. For what the analyses *do*,
see the [user guide](guide.md); for what we measured about the TalkBank service,
see [talkbank-api.md](talkbank-api.md).

## Building

Meson is the single entry point. It builds the 70 CLAN programs, the translation
catalogues, and — by driving cargo — the Rust desktop app.

```sh
meson setup build
meson compile -C build       # everything: CLAN programs, translations, the app
meson install -C build       # into /usr/local by default
```

Requirements: `g++`, `meson`, `ninja`, `gettext`, `cargo`, `libgtk-4-dev`,
`libadwaita-1-dev`. The first build fetches and compiles chatter's dependency
tree: give it a few minutes, after which it stays cached.

`meson setup build -Dapp=disabled` skips the Rust app and builds only the CLAN
programs — useful when you only want the command-line tools.

`-Ddepdir=<path>` sets where the CLAN programs look for `lib/` at run time. It is
compiled in, and they have no way to look it up themselves. Left empty it points
at the source tree, which is what makes `build/freq` work with no setup;
packagers and `install.sh` set it to `<prefix>/share/talkbank/lib` so the
binaries keep working without the checkout.

### Where things land

```
<prefix>/bin/talkbank                 the app, and the only thing on the PATH
<prefix>/libexec/talkbank/            the 70 CLAN programs
<prefix>/share/talkbank/lib/          the language data they read (DEPDIR)
<prefix>/share/locale/*/LC_MESSAGES/  the compiled catalogues
<prefix>/share/applications/          the desktop entry
<prefix>/share/doc/talkbank/          the guide the start page links to
```

**The CLAN programs must not go into `bin/`.** Three of them — `uniq`, `script`
and `gem` — are named after standard Unix commands, and others (`indent`,
`repeat`, `post`, `dist`, `lines`, `check`) collide on some systems. An early
release did install them into `~/.local/bin`, where CLAN's `uniq` shadowed
coreutils' and printed its banner every time a shell startup file called `uniq`.
`find_bin_dir()` in `talkbank-engine` knows the private layout; the PATH stays
last in its search order, as a courtesy to anyone who added the directory
themselves.

### Running from the build directory

```sh
./build/talkbank                    # reopens the last folder and analysis
./build/talkbank /path/to/corpus    # opens a folder
./build/talkbank transcript.cha     # opens a file and selects it
```

The app looks for the CLAN programs in `$CLAN_BIN`, then next to itself, then on
the `PATH` — so from `build/` it finds them without any setup. Translations work
the same way, through `$TALKBANK_LOCALE` or `build/locale/`.

Shortcuts: `Ctrl+Enter` runs, `Ctrl+B` opens the archive, `Ctrl+,` preferences,
`Ctrl+H` recent commands.

### Tests

```sh
cargo test --workspace       # offline checks
cargo test -p talkbank-archive --test network -- --ignored --nocapture
```

The network suite is `#[ignore]`d so a plain `cargo test` never touches the
network. Most of it works without credentials, because the catalogue and the
metadata are public; the rest reads `.env` (`USERNAME`, `PASSWORD`) and skips if
it is missing.

The conformance suite checks our validation wrapper against
[`testchat`](https://github.com/TalkBank/testchat); point `$TESTCHAT` at a clone,
or keep one in `~/testchat`, otherwise those tests skip.

### Translations

Italian and English. `xgettext` cannot read Rust — lifetimes such as `'static`
look to it like unterminated character literals — so extraction goes through
`tools/extract-strings.py`:

```sh
tools/extract-strings.py            # list the new strings
tools/extract-strings.py --merge    # append them to po/it.po and po/en.po
```

To add a language: add its code to `languages` in `meson.build`, copy `po/en.po`
to `po/<code>.po`, and translate.

### Releasing

```sh
tools/bump-version.sh 0.2.0    # updates Cargo.toml, meson.build, CHANGELOG.md
```

It refuses to run on a dirty tree, moves the CHANGELOG's `Unreleased` section
under the new version, and prints the commit and tag commands for you to run.

## Layout

```
Cargo.toml              Rust workspace
crates/
  talkbank-engine/      running the CLAN programs, reading and validating CHAT
  talkbank-archive/     TalkBank archive: catalogue, metadata, access, downloads
    batch.rs            planning the download of a whole branch
  talkbank-batchalign/  client for the local Batchalign3 server
  talkbank-gtk/         the interface (binary: talkbank)
    window.rs           the shell: section sidebar and page stack
    home.rs             start page: resume, recents, what would you like to do
    editor.rs           CHAT editor with colouring and format checking
    archive.rs          the TalkBank section
    downloads.rs        download queue, pausing and notifications
meson.build             the 70 CLAN programs, translations, and the cargo target
src/ lib/ compat/       upstream CLAN sources, unchanged
po/                     translations (Italian, English)
tools/                  string extractor, version bump, meson→cargo shim
data/                   .desktop file and icons
docs/                   this documentation
```

## Choices worth knowing about

**The CLAN binaries remain the analysis engine.** They are byte-exact because
they *are* CLAN. `chatter-clan` exists, reimplementing 34 analyses in Rust, but it
is declared dormant and its own book says no command reaches parity with CLAN. It
is also the direction the maintainers chose: `send2clan` exists precisely to hand
files to the real CLAN.

**The editor is ours, the validation is chatter's.** On Linux there was no CHAT
editor: `chatter-desktop` only validates, and CLAN's own editor is macOS and
Windows only. The colouring here is per line — in CHAT the first character decides
the type — while the verdict on the format comes entirely from `chatter`, so there
are never two verdicts on the same file.

**The CHAT format is read by `chatter`**, their authority on the format, pinned as
a git dependency at tag `v0.12.0`. Their book warns that "any release may change
which files validate", so all our use of their API goes through
`crates/talkbank-engine/src/validate.rs`: an upgrade touches one file.

**Batchalign3 is driven through its local HTTP server**, not by linking its
crates: that is the contract their own desktop app uses, it is described by a
versioned `openapi.json`, and it keeps their ML build out of ours. It stays
optional.

**The CLAN programs run on a pseudo-terminal.** This is not an affectation: with a
non-tty stdin they ignore the filenames on the command line and read standard
input instead, and with a non-tty stdout they refuse `+f`.

## Compiling the CLAN programs

- `-std=gnu++98 -fpermissive` — the sources do not compile under g++ 15's default
  standard;
- `compat/termio.h` — glibc removed `<termio.h>`; it is remapped onto
  `<termios.h>` here;
- `-DDEPDIR='"…/lib/"'` — location of the `lib` directory, as `src/0README.TXT`
  asks, without editing `common.h`;
- `dist` is a name meson reserves: it is built as `clan-dist` and a copy under the
  right name is placed next to it.
