#!/bin/sh
# Bump the project version in the places that carry one, and open a new
# CHANGELOG section.
#
#   tools/bump-version.sh 0.2.0
#
# It does not commit or tag: it prints the commands, so you can look at the diff
# first.
set -eu

new=${1:-}
case "$new" in
    '')             echo "usage: $0 <major.minor.patch>" >&2; exit 2 ;;
    *.*.*)          ;;
    *)              echo "error: '$new' is not a semantic version" >&2; exit 2 ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# A dirty tree would make the diff unreadable and the bump hard to undo.
if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree is not clean" >&2
    exit 1
fi

old=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$old" ] || { echo "error: could not read the current version" >&2; exit 1; }
[ "$old" != "$new" ] || { echo "error: already at $new" >&2; exit 1; }

echo "$old -> $new"

# Cargo.toml: the workspace version, which every crate inherits.
sed -i "0,/^version = \"$old\"$/s//version = \"$new\"/" Cargo.toml

# meson.build: the project() version.
sed -i "s/^  version: '$old',$/  version: '$new',/" meson.build

# po headers, so the catalogues do not claim an old release.
sed -i "s/Project-Id-Version: talkbank $old/Project-Id-Version: talkbank $new/" po/*.po po/*.pot

# CHANGELOG: turn Unreleased into the new version and open a fresh Unreleased.
today=$(date +%Y-%m-%d)
python3 - "$new" "$old" "$today" <<'PY'
import pathlib, re, sys
new, old, today = sys.argv[1:4]
base = "https://github.com/matteospanio/talkbank"
p = pathlib.Path("CHANGELOG.md")
s = p.read_text()
if "## [Unreleased]" not in s:
    sys.exit("error: CHANGELOG.md has no '## [Unreleased]' section")
s = s.replace("## [Unreleased]\n", f"## [Unreleased]\n\n## [{new}] - {today}\n", 1)
# Point Unreleased at the new tag and add the new release link above the old
# ones; the existing links stay untouched.
s = re.sub(r"^\[Unreleased\]: .*$",
           f"[Unreleased]: {base}/compare/v{new}...HEAD\n"
           f"[{new}]: {base}/compare/v{old}...v{new}",
           s, count=1, flags=re.M)
p.write_text(s)
PY

# Keep Cargo.lock in step, so the bump is one self-contained commit. `--workspace`
# re-records only the workspace members' versions; it does not touch dependencies.
cargo update --workspace --offline \
    || echo "warning: could not refresh Cargo.lock; run 'cargo update --workspace' yourself" >&2

grep -q "version = \"$new\"" Cargo.toml || { echo "error: Cargo.toml not updated" >&2; exit 1; }
grep -q "version: '$new'" meson.build   || { echo "error: meson.build not updated" >&2; exit 1; }

cat <<EOF

Updated: Cargo.toml, Cargo.lock, meson.build, po/*, CHANGELOG.md

Review the CHANGELOG entry, then:

  git commit -am "Release v$new"
  git tag -a "v$new" -m "v$new"
  git push --follow-tags
EOF
