#!/bin/sh
# TalkBank installer.
#
#   curl -LsSf https://raw.githubusercontent.com/matteospanio/talkbank/main/install.sh | sh
#
# Builds from source. GTK 4 and libadwaita binaries do not travel well between
# distributions, so a prebuilt tarball would break more often than it helped.
#
# Environment:
#   PREFIX   install prefix          (default: ~/.local)
#   SRC      where to keep the clone (default: ~/.local/share/talkbank-src)
#   REF      branch or tag to build  (default: main)
set -eu

REPO=https://github.com/matteospanio/talkbank.git
PREFIX=${PREFIX:-$HOME/.local}
SRC=${SRC:-$HOME/.local/share/talkbank-src}
REF=${REF:-main}

say()  { printf '\033[1m==>\033[0m %s\n' "$1"; }
die()  { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# --- dependencies ----------------------------------------------------------
# Named the way the user has to install them, not the way we use them: "install
# meson" is actionable, "meson not found" is not.
missing=
for tool in git g++ meson ninja msgfmt cargo pkg-config; do
    have "$tool" || missing="$missing $tool"
done
# GTK is a build dependency of the app but not of the CLAN programs, so it gets
# checked separately and reported by package name.
if have pkg-config; then
    pkg-config --exists gtk4 || missing="$missing libgtk-4-dev"
    pkg-config --exists libadwaita-1 || missing="$missing libadwaita-1-dev"
fi

if [ -n "$missing" ]; then
    printf '\033[31merror:\033[0m missing build dependencies:%s\n\n' "$missing" >&2
    if have apt; then
        echo "  sudo apt install build-essential meson ninja-build gettext pkg-config \\" >&2
        echo "                   libgtk-4-dev libadwaita-1-dev cargo" >&2
    elif have dnf; then
        echo "  sudo dnf install gcc-c++ meson ninja-build gettext pkgconf-pkg-config \\" >&2
        echo "                   gtk4-devel libadwaita-devel cargo" >&2
    elif have pacman; then
        echo "  sudo pacman -S base-devel meson ninja gettext gtk4 libadwaita rust" >&2
    fi
    exit 1
fi

# --- source ----------------------------------------------------------------
if [ -d "$SRC/.git" ]; then
    say "Updating $SRC"
    git -C "$SRC" fetch --depth 1 origin "$REF"
    git -C "$SRC" checkout -q FETCH_HEAD
else
    say "Cloning into $SRC"
    mkdir -p "$(dirname "$SRC")"
    git clone --depth 1 --branch "$REF" "$REPO" "$SRC"
fi

# --- build -----------------------------------------------------------------
say "Building (the first run compiles chatter's dependencies; give it a few minutes)"
cd "$SRC"
# -Ddepdir points the CLAN programs at their installed language data instead of
# at this checkout, so the clone is only needed for updates.
setup="--prefix=$PREFIX -Ddepdir=$PREFIX/share/talkbank/lib"
# shellcheck disable=SC2086
meson setup build $setup --wipe >/dev/null 2>&1 || meson setup build $setup
meson compile -C build
meson install -C build

# --- report ----------------------------------------------------------------
say "Installed into $PREFIX"
echo "  Source kept in $SRC (rerun this script to update)"
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *)
        echo
        echo "  $PREFIX/bin is not on your PATH. Add it with:"
        echo
        echo "    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.profile"
        echo
        ;;
esac
echo "  Run:  talkbank"
echo "  Docs: https://github.com/matteospanio/talkbank#documentation"
