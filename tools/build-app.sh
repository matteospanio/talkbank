#!/bin/sh
# Build the Rust app and drop the binary where meson expects it.
#
# meson cannot see cargo's dependency graph, so the target is always stale and
# cargo itself decides whether anything needs rebuilding.
#
#   $1  source root      $2  output path      $3  cargo profile (debug|release)
set -eu
root=$1
out=$2
profile=$3

if [ "$profile" = "release" ]; then
    cargo build --release --manifest-path "$root/Cargo.toml" -p talkbank-gtk
else
    cargo build --manifest-path "$root/Cargo.toml" -p talkbank-gtk
fi
cp -f "$root/target/$profile/talkbank" "$out"
