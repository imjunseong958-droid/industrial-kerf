#!/usr/bin/env bash
# The complete source of what scripts/package-kerf-windows.ps1 packs, as one tar to
# publish beside the package: the package carries a GPL build of FFmpeg, so it has to
# be offered with the source of all of it. Kerf at this commit with its Rust crates,
# FFmpeg and each library built into it, the scripts that built those, and Qt.
#
# Runs on Linux with git, cargo, curl and docker. What to collect is pinned in
# .github/workflows/windows-package.yml, with the build the package is made from.
set -euo pipefail

: "${FFMPEG_BUILDS_TAG:?the FFmpeg-Builds release the package uses}"
: "${FFMPEG_BUILDS_COMMIT:?the commit of FFmpeg-Builds that made it}"
: "${FFMPEG_COMMIT:?the commit of FFmpeg it was built from}"
: "${QT_VERSION:?the Qt version the package uses}"

root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)"
name="Industrial-Kerf-$version-windows-x64-source"
out="$root/target/package-source"
stage="$out/$name"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
rm -rf "$out"
mkdir -p "$stage/ffmpeg-libraries" "$stage/qt"

# Kerf, and the crates its Cargo.lock names.
git -C "$root" archive --format=tar --prefix=kerf/ HEAD | tar -x -C "$stage"
(cd "$stage/kerf" && cargo vendor --locked --versioned-dirs vendor > cargo-vendor-config.toml)

# FFmpeg itself.
curl -fsSL -o "$stage/ffmpeg-$FFMPEG_COMMIT.tar.gz" "https://github.com/FFmpeg/FFmpeg/archive/$FFMPEG_COMMIT.tar.gz"

# The scripts that built it, and through them each library at the commit they pin.
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$FFMPEG_BUILDS_TAG" \
    https://github.com/BtbN/FFmpeg-Builds.git "$work/builds"
built="$(git -C "$work/builds" rev-parse HEAD)"
if [ "$built" != "$FFMPEG_BUILDS_COMMIT" ]; then
    echo "$FFMPEG_BUILDS_TAG is at $built, not $FFMPEG_BUILDS_COMMIT" >&2
    exit 1
fi
git -C "$work/builds" archive --format=tar --prefix=FFmpeg-Builds/ HEAD | tar -x -C "$stage"
# Out of GITHUB_REPOSITORY its download.sh would name a tools image of this repository.
(cd "$work/builds" && env -u GITHUB_REPOSITORY ./download.sh)
# The files, not the links to them download.sh leaves beside them.
find "$work/builds/.cache/downloads" -type f -name '*.tar.xz' -exec cp {} "$stage/ffmpeg-libraries/" \;

# Qt's modules, of the version in the package.
for module in qtbase qtsvg; do
    curl -fsSL -o "$stage/qt/$module-everywhere-src-$QT_VERSION.tar.xz" \
        "https://download.qt.io/archive/qt/${QT_VERSION%.*}/$QT_VERSION/submodules/$module-everywhere-src-$QT_VERSION.tar.xz"
done

cat > "$stage/README.txt" <<TEXT
The complete source of Industrial Kerf $version for Windows.

kerf/               Kerf at commit $(git -C "$root" rev-parse HEAD), with the crates of
                    its Cargo.lock in vendor/. To build from them, add the contents of
                    cargo-vendor-config.toml to .cargo/config.toml.
                    scripts/package-kerf-windows.ps1 builds the package.
ffmpeg-$FFMPEG_COMMIT.tar.gz
                    FFmpeg, as built into the package.
FFmpeg-Builds/      The scripts that built it ($FFMPEG_BUILDS_TAG of
                    https://github.com/BtbN/FFmpeg-Builds), for win64-gpl-shared-9.0.
ffmpeg-libraries/   Each library those scripts build, at the commit they pin.
qt/                 Qt $QT_VERSION's modules.
TEXT

tar -C "$out" -cf "$out/$name.tar" "$name"
rm -rf "$stage"
(cd "$out" && sha256sum "$name.tar" > "$name.tar.sha256")
du -h "$out/$name.tar"
