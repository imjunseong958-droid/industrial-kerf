#!/usr/bin/env bash
# The complete source of what scripts/package-kerf-linux.sh packs, as one tar to publish
# beside it: what collect-kerf-windows-sources.sh collects (Kerf at this commit with its
# Rust crates, FFmpeg and each library built into it, the scripts that built those, and
# Qt), for the Linux build of the same FFmpeg, and the source of each Ubuntu package the
# package carries a library of (target/package-linux/bundled-packages.txt), as
# `apt-get source` gives it.
#
# Runs on the Ubuntu machine that made the package, after it, with deb-src enabled in apt's
# sources, git, cargo, curl and docker. What to collect is pinned in
# .github/workflows/package.yml.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
packages="$root/target/package-linux/bundled-packages.txt"
[ -f "$packages" ] || { echo "No $packages: run scripts/package-kerf-linux.sh first" >&2; exit 1; }
extra="$(mktemp -d)"
trap 'rm -rf -- "$extra"' EXIT
mkdir -p "$extra/ubuntu-packages"
while read -r package version; do
    # The source package of each, at the version whose library is in the package.
    (cd "$extra/ubuntu-packages" && apt-get source --download-only --quiet "$package=$version")
done < "$packages"
cp "$packages" "$extra/ubuntu-packages/bundled-packages.txt"
KERF_SOURCE_PLATFORM=linux-x64 KERF_EXTRA_SOURCES="$extra" "$root/scripts/collect-kerf-windows-sources.sh"
