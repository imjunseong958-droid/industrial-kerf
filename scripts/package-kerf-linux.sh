#!/usr/bin/env bash
# Industrial Kerf for Linux as a folder that runs on its own, packed as
# target/package-linux/<name>.tar.xz with its sha256, and the same folder as <name>.deb,
# which apt installs (scripts/package-kerf-linux-deb.sh), and as <name>.rpm, which dnf
# does (scripts/package-kerf-linux-rpm.sh). bin/ holds the editor (kerf-editor), kerfd,
# kerf, kerf-mcp, ffmpeg and ffprobe; lib/ every library they load that a desktop Linux does
# not have of its own (FFmpeg's), found through each one's RPATH, $ORIGIN/../lib. The editor
# is one Rust binary drawn with GPUI: it carries no toolkit, and what it takes from the
# system is apps/kerf-editor/packaging/linux/system-libraries.txt (xkbcommon, libxcb, the
# Wayland client library, the Vulkan loader and the system's Vulkan driver, ALSA).
#
# Runs on Ubuntu with Rust, patchelf, dpkg-dev, Xvfb with a Vulkan driver (Mesa's lavapipe,
# mesa-vulkan-drivers), GPUI's build dependencies (libxkbcommon-dev, libxkbcommon-x11-dev,
# libwayland-dev, libxcb1-dev, libx11-xcb-dev, libfontconfig-dev, libfreetype-dev, libvulkan1),
# and a shared GPL build of FFmpeg in FFMPEG_DIR (BtbN's, pinned in package.yml). Built on
# Ubuntu 24.04, it needs a glibc at least that new.
# The FFmpeg makes the package GPL: target/package-linux/bundled-packages.txt names each
# Ubuntu package it carries a library of, for scripts/collect-kerf-linux-sources.sh.
#
#   scripts/package-kerf-linux.sh [--skip-run]
set -euo pipefail

skip_run=false
[ "${1:-}" = "--skip-run" ] && skip_run=true

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
: "${FFMPEG_DIR:?a shared GPL build of FFmpeg, with bin/ffmpeg and lib/}"
ffmpeg="$FFMPEG_DIR"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
name="Industrial-Kerf-$version-linux-x64"
out="$root/target/package-linux"
stage="$out/$name"
commit="$(git rev-parse HEAD)"
system_list="$root/apps/kerf-editor/packaging/linux/system-libraries.txt"

for tool in patchelf readelf; do
    command -v "$tool" > /dev/null || { echo "$tool is needed" >&2; exit 1; }
done
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "Commit first: the package names the commit its source comes from" >&2
    exit 1
fi
# What the package says about its licence and its source has to hold.
buildconf="$("$ffmpeg/bin/ffmpeg" -hide_banner -buildconf)"
if grep -q -- --enable-nonfree <<< "$buildconf"; then
    echo "This FFmpeg build is nonfree, and may not be given to anyone" >&2
    exit 1
fi
if ! grep -q -- --enable-gpl <<< "$buildconf" || ! grep -q -- --enable-version3 <<< "$buildconf"; then
    echo "THIRD-PARTY.txt and SOURCE.txt describe a GPL version 3 build of FFmpeg, and this is not one" >&2
    exit 1
fi

rm -rf "$out"
mkdir -p "$stage/bin" "$stage/lib"

# Optimized: the editor's live preview and its exports draw frames in kerf-bridge, which it
# links. The FFmpeg in FFMPEG_DIR is the one they all link (ffmpeg-sys-next reads it).
cargo build --release --manifest-path apps/kerf-editor/Cargo.toml
cargo build --release -p kerf-runtime --bin kerfd
cargo build --release -p kerf-cli --bin kerf
cargo build --release -p kerf-mcp --bin kerf-mcp

# The editor looks beside itself for kerfd and kerf-mcp, and each of them for ffmpeg and
# ffprobe (kerf-engine's ffmpeg.rs).
for tool in kerf-editor kerfd kerf kerf-mcp; do cp "target/release/$tool" "$stage/bin/"; done
for tool in ffmpeg ffprobe; do cp "$ffmpeg/bin/$tool" "$stage/bin/"; done

# Every library one of these loads, and one of those, that the system does not provide:
# found where the build took it from, and copied into lib/.
system_libraries="$(grep -v '^#' "$system_list" | sed '/^$/d')"
system_library() { grep -qxF "$1" <<< "$system_libraries"; }
from_system="$out/.from-system"
: > "$from_system"
missing=""
find_library() {
    local soname="$1" folder
    for folder in "$ffmpeg/lib" "$root/target/release"; do
        [ -e "$folder/$soname" ] && { echo "$folder/$soname"; return; }
    done
    # The build machine's own, by the loader's cache.
    /sbin/ldconfig -p | awk -v soname="$soname" '$1 == soname && /x86-64/ {print $NF; exit}'
}
queue=()
while IFS= read -r -d '' file; do
    queue+=("$file")
done < <(find "$stage/bin" "$stage/lib" -type f -print0)
while [ "${#queue[@]}" -gt 0 ]; do
    current="${queue[0]}"
    queue=("${queue[@]:1}")
    head -c 4 "$current" | grep -q 'ELF' || continue
    while read -r soname; do
        system_library "$soname" && continue
        [ -e "$stage/lib/$soname" ] && continue
        found="$(find_library "$soname")"
        if [ -z "$found" ]; then
            missing+="$soname (for $(basename "$current"))"$'\n'
            continue
        fi
        cp -L "$found" "$stage/lib/$soname"
        chmod 755 "$stage/lib/$soname"
        case "$found" in "$ffmpeg"/*|"$root"/*) ;; *) echo "$found" >> "$from_system" ;; esac
        queue+=("$stage/lib/$soname")
    done < <(patchelf --print-needed "$current")
done
if [ -n "$missing" ]; then
    printf 'Nothing provides:\n%s' "$missing" >&2
    exit 1
fi

# Each finds the libraries in lib/ first, whatever LD_LIBRARY_PATH says: an RPATH, which
# comes before it, and not a RUNPATH, which comes after.
while IFS= read -r -d '' file; do
    head -c 4 "$file" | grep -q 'ELF' || continue
    case "$file" in
        "$stage"/bin/*) rpath='$ORIGIN/../lib' ;;
        "$stage"/lib/*) rpath='$ORIGIN' ;;
    esac
    patchelf --force-rpath --set-rpath "$rpath" "$file"
done < <(find "$stage" -type f -print0)

# Which Ubuntu packages the libraries taken from the system came from, with their
# versions, for the source archive and THIRD-PARTY.txt.
packages="$out/bundled-packages.txt"
: > "$packages"
while read -r library; do
    real="$(readlink -f "$library")"
    package="$(dpkg -S "$real" 2>/dev/null | head -n 1 | cut -d: -f1)"
    [ -n "$package" ] || package="$(dpkg -S "$library" 2>/dev/null | head -n 1 | cut -d: -f1)"
    [ -n "$package" ] || { echo "No Ubuntu package owns $library" >&2; exit 1; }
    package="${package%%:*}"
    printf '%s %s\n' "$package" "$(dpkg-query -W -f='${Version}' "$package")" >> "$packages"
done < "$from_system"
sort -u -o "$packages" "$packages"
rm -f "$from_system"

# The terms, and where the source is.
cp LICENSE "$stage/LICENSE.txt"
cp -R licenses "$stage/licenses"
mkdir -p "$stage/fonts"
cp apps/kerf-editor/assets/fonts/*-LICENSE.txt "$stage/fonts/"
cp "$ffmpeg/LICENSE.txt" "$stage/COPYING-GPLv3.txt"
grep -q 'GNU GENERAL PUBLIC LICENSE' "$stage/COPYING-GPLv3.txt" || { echo "No GPL text in $ffmpeg/LICENSE.txt" >&2; exit 1; }
printf '%s\n' "$buildconf" > "$stage/FFmpeg-BUILD.txt"
{
    cat apps/kerf-editor/packaging/linux/THIRD-PARTY.txt
    echo
    echo "Every Ubuntu package with a library in lib/, its version, and its licence (its copyright file):"
    while read -r package package_version; do
        printf '  %s %s: /usr/share/doc/%s/copyright\n' "$package" "$package_version" "$package"
        mkdir -p "$stage/licenses/ubuntu/$package"
        if [ -f "/usr/share/doc/$package/copyright" ]; then
            cp "/usr/share/doc/$package/copyright" "$stage/licenses/ubuntu/$package/copyright"
        else
            echo "See debian/copyright in its source." > "$stage/licenses/ubuntu/$package/copyright"
        fi
    done < "$packages"
} > "$stage/THIRD-PARTY.txt"
cat > "$stage/SOURCE.txt" <<EOF
Industrial Kerf $version for Linux (x86-64), from commit $commit.
Its source: https://github.com/imjunseong958-droid/industrial-kerf

This package is under the GNU General Public License version 3 (COPYING-GPLv3.txt), and
you have the right to its complete source. That is $name-source.tar, published beside this
package wherever you got it. It holds Kerf and its Rust crates (GPUI, which draws the
editor, among them), FFmpeg, each library built into FFmpeg, the scripts that built them,
and the source of each Ubuntu package a library in lib/ comes from, if any.

Which licence covers which part is in THIRD-PARTY.txt.
EOF
cat > "$stage/README.txt" <<EOF
Industrial Kerf $version for Linux.

Run bin/kerf-editor. The folder can go anywhere; it needs no installing. It uses what
every desktop Linux has of its own (system-libraries.txt): glibc as new as Ubuntu 24.04's,
X11 or Wayland with xkbcommon, a Vulkan driver (Mesa's, or your GPU vendor's), and ALSA.

To have Kerf among your applications, and kerf:// project links open in it, run
./install-desktop-entry.sh once: it is for you alone and needs no administrator. Run it
again after moving the folder; ./install-desktop-entry.sh --remove takes it away. Its icon
is in icons/.
EOF
# The desktop entry and kerf:// links, which a folder cannot register of itself.
install -m 755 apps/kerf-editor/packaging/linux/install-desktop-entry.sh "$stage/install-desktop-entry.sh"
# Kerf's icon at each size that script installs for the entry (tools/kerf-icon makes them
# from apps/kerf-editor/assets/icon/industrial-kerf.svg).
mkdir -p "$stage/icons"
for size in 16 24 32 48 64 128 256 512; do
    cp "apps/kerf-editor/assets/icon/industrial-kerf-$size.png" "$stage/icons/"
done
cp "$system_list" "$stage/system-libraries.txt"

# With nothing on PATH and no library path, it has to find all it needs inside itself, and
# load nothing from outside but the system's own.
$skip_run || scripts/check-kerf-linux.sh "$stage"

tar -C "$out" -cJf "$out/$name.tar.xz" "$name"
(cd "$out" && sha256sum "$name.tar.xz" > "$name.tar.xz.sha256")
echo "$out/$name.tar.xz"

# The same folder as a .deb, whose Depends are what the folder takes from the system.
# Installing it takes root: scripts/check-kerf-linux-deb.sh does, in package.yml.
scripts/package-kerf-linux-deb.sh "$stage" "$out/$name.deb"

# And as an .rpm, whose Requires are the same libraries as rpm names them.
# scripts/check-kerf-linux-rpm.sh installs it, in package.yml.
scripts/package-kerf-linux-rpm.sh "$stage" "$out/$name.rpm"
