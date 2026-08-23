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

# Releases before 0.1.1 installed the 70 CLAN programs straight into bin/, where
# `uniq`, `script` and `gem` shadowed the system commands of the same name — with
# the visible effect of a CLAN banner every time a shell started. Upgrading has
# to clean those up, or the damage outlives the fix.
old=0
for f in build/*; do
    name=${f##*/}
    case "$name" in
        *.*|talkbank) continue ;;
    esac
    [ -f "$f" ] && [ -x "$f" ] || continue
    [ -f "$PREFIX/bin/$name" ] || continue
    if cmp -s "$f" "$PREFIX/bin/$name"; then
        rm -f "$PREFIX/bin/$name"
        old=$((old + 1))
    fi
done
[ "$old" -gt 0 ] && say "Removed $old CLAN programs left in $PREFIX/bin by an earlier version"

# --- desktop integration ---------------------------------------------------
# meson does this too, but only when installing into a system prefix; for a
# ~/.local install the user's own databases are the ones that need refreshing.
have update-desktop-database && update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
have gtk-update-icon-cache && gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true

# --- report ----------------------------------------------------------------
say "Installed into $PREFIX"
echo "  Source kept in $SRC (rerun this script to update)"
echo
echo "  The app is in your applications menu as \"TalkBank\"."
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *)
        echo
        echo "  $PREFIX/bin is not on your PATH, so the \`talkbank\` command will not"
        echo "  be found from a shell. Add it with:"
        echo
        echo "    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.profile"
        ;;
esac
cat <<EOF

  Run:       talkbank
  Uninstall: curl -LsSf https://raw.githubusercontent.com/matteospanio/talkbank/main/uninstall.sh | sh
  Docs:      https://github.com/matteospanio/talkbank#documentation

  The 70 CLAN programs are in $PREFIX/libexec/talkbank, deliberately off your
  PATH: three of them (uniq, script, gem) share a name with a system command.
  The app finds them by itself. To use them from a shell, append — never
  prepend — that directory:

    echo 'export PATH="\$PATH:$PREFIX/libexec/talkbank"' >> ~/.profile
EOF
