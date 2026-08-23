# AGENTS.md

Notes for AI coding agents working in this repository. Humans should read
[docs/development.md](docs/development.md) instead — this file only covers what
tends to trip an agent up.

## What this is

A Linux desktop client for TalkBank. Two languages, one build:

- `src/`, `lib/`, `compat/` — the 70 CLAN programs. **Upstream C++ from Carnegie
  Mellon, byte-exact, GPL-2.0. Do not modify these.** If something does not
  compile, fix it with a flag in `meson.build` or a shim in `compat/`, not by
  editing the sources.
- `crates/` — the Rust app. This is the part you change.

## Build and test

```sh
meson setup build && meson compile -C build   # everything
cargo test --workspace                        # the tests that matter day to day
cargo check --workspace --all-targets --offline
```

`meson compile` drives cargo through `tools/build-app.sh`, so the app binary
lands at `build/talkbank`. Working on Rust only? `cargo` directly is faster.

Two suites do **not** run by default and should not be made to:

- `tests/network.rs` is `#[ignore]`d because it talks to the live TalkBank
  service. Run it deliberately:
  `cargo test -p talkbank-archive --test network -- --ignored --nocapture`.
- `tests/conformance.rs` skips unless `$TESTCHAT` points at a clone of
  [testchat](https://github.com/TalkBank/testchat).

## Conventions

**English everywhere.** Code, comments, doc-comments, commit messages, test
names. The repository is public.

**Comments say why, not what.** Most comments here record a measured fact or a
decision that is not obvious from the code — a server that answers 200 where you
would expect 401, a libadwaita assertion, a race that lost one result in seven.
When you change such code, update the comment; when you delete the code, delete
the comment. Do not add comments that restate the line below them.

**User-facing strings go through `t()` / `tn()`** from `crate::i18n`, always as
string literals — `t(variable)` is invisible to the extractor. After adding
strings run `tools/extract-strings.py --merge` and fill in `po/it.po`. Changing an
existing msgid orphans its translation, so change one only when the text is
genuinely wrong.

**Measured numbers are load-bearing.** Baselines in the tests (301/339 good files,
23 KB per transcript, 500 probes) come from real measurements. If a test fails on
one of them, find out what changed before adjusting the number.

## Things that will bite you

**GTK widgets only from the main loop.** Network work goes through
`crate::net::Net::spawn`, which hands the result back on the UI thread. Never
touch a widget from a tokio task.

**Check `widget.root().is_some()` in async callbacks.** A page can be closed while
a request is in flight, and calling `remove()` on a disposed `AdwPreferencesGroup`
trips libadwaita assertions.

**Do not race the progress channel against the result channel.** In
`spawn_with_progress` they are read in sequence, on purpose:
`crates/talkbank-gtk/src/net.rs` has the test that pins this down.

**The CLAN programs need a pseudo-terminal.** With a non-tty stdin they ignore
their filename arguments; with a non-tty stdout they refuse `+f`. This is why
`runner.rs` looks the way it does. `tests/against_real_clan.rs` is what protects
it.

**Whether a folder is a corpus is only knowable from the server.** Never infer it
from tree depth or the presence of files — see
[docs/talkbank-api.md](docs/talkbank-api.md), where the counter-examples are
listed.

## Credentials

`.env` at the repository root holds `USERNAME` and `PASSWORD` for the network
tests. It is gitignored. Never print its contents, commit it, or copy the values
into code, tests or logs.
