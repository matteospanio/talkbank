#!/bin/sh
# TalkBank uninstaller.
#
#   curl -LsSf https://raw.githubusercontent.com/matteospanio/talkbank/main/uninstall.sh | sh
#
# Removes the files install.sh put in place. Your own data — downloaded corpora
# and transcripts — is never touched: it lives wherever you chose to save it.
#
# Environment:
#   PREFIX   where it was installed  (default: ~/.local)
#   SRC      where the clone is kept (default: ~/.local/share/talkbank-src)
#   PURGE    set to 1 to also drop settings, cache and the saved password
set -eu

PREFIX=${PREFIX:-$HOME/.local}
SRC=${SRC:-$HOME/.local/share/talkbank-src}
PURGE=${PURGE:-0}

say()  { printf '\033[1m==>\033[0m %s\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }

removed=0

# --- what meson installed --------------------------------------------------
# meson keeps a log of every file it installed, so when the build directory is
# still there we remove exactly that list and nothing else.
if [ -f "$SRC/build/meson-logs/install-log.txt" ]; then
    say "Removing the installed files (from meson's install log)"
    ninja -C "$SRC/build" uninstall >/dev/null 2>&1 || meson uninstall -C "$SRC/build" >/dev/null 2>&1 || true
    removed=1
fi

# Fall back to the known layout: the build directory may be gone, or the install
# may predate the log. Only paths this project owns are touched.
say "Checking $PREFIX for anything left behind"
rm -f  "$PREFIX/bin/talkbank"
rm -rf "$PREFIX/libexec/talkbank"
rm -rf "$PREFIX/share/talkbank"
rm -rf "$PREFIX/share/doc/talkbank"
rm -f  "$PREFIX/share/applications/org.talkbank.TalkBank.desktop"
rm -f  "$PREFIX/share/icons/hicolor/128x128/apps/org.talkbank.TalkBank.png"
rm -f  "$PREFIX/share/locale/it/LC_MESSAGES/talkbank.mo"
rm -f  "$PREFIX/share/locale/en/LC_MESSAGES/talkbank.mo"

# Releases before 0.1.1 put the 70 CLAN programs straight into bin/, where three
# of them (uniq, script, gem) shadowed standard commands. Clean those up, but
# only when the file really is ours: a name match alone is not enough to delete
# something out of the user's bin directory.
if [ -d "$SRC" ] && [ -d "$SRC/build" ]; then
    for f in "$SRC"/build/*; do
        name=${f##*/}
        case "$name" in
            *.*|talkbank) continue ;;
        esac
        [ -f "$f" ] && [ -x "$f" ] || continue
        target="$PREFIX/bin/$name"
        [ -f "$target" ] || continue
        if cmp -s "$f" "$target"; then
            rm -f "$target"
            removed=1
        fi
    done
fi

# --- source clone ----------------------------------------------------------
if [ -d "$SRC" ]; then
    say "Removing the source clone $SRC"
    rm -rf "$SRC"
fi

# --- personal data (opt-in) ------------------------------------------------
if [ "$PURGE" = "1" ]; then
    say "Removing settings and cache"
    rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/talkbank"
    rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/talkbank"
    echo "  The TalkBank password, if saved, is in your system keyring under"
    echo "  'org.talkbank.TalkBank' — remove it with your keyring manager."
else
    echo "  Settings and cache kept. Re-run with PURGE=1 to remove them too."
fi

# --- refresh the desktop ---------------------------------------------------
have update-desktop-database && update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
have gtk-update-icon-cache && gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true

say "Done"
if [ "$removed" = "0" ]; then
    echo "  Nothing looked installed under $PREFIX — if you used a different"
    echo "  prefix, re-run with PREFIX=/your/prefix"
fi
