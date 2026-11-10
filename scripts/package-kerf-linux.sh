#!/usr/bin/env bash
# Industrial Kerf for Linux as a folder that runs on its own, packed as
# target/package-linux/<name>.tar.xz with its sha256, and the same folder as <name>.deb,
# which apt installs (scripts/package-kerf-linux-deb.sh). bin/ holds the editor, kerfd, kerf,
# kerf-mcp, ffmpeg and ffprobe; lib/ every library they load that a desktop Linux does not
# have of its own (Qt and its ICU, FFmpeg, the xcb utilities, xkbcommon), found through each
# one's RPATH, $ORIGIN/../lib; plugins/ the Qt plugins, which bin/qt.conf points Qt to. What
# it takes from the system is apps/kerf-qt/packaging/linux/system-libraries.txt.
#
# Runs on Ubuntu with Rust, CMake, patchelf, dpkg-dev and Xvfb, the Qt in QT_ROOT_DIR (as
# install-qt-action sets it) and a shared GPL build of FFmpeg in FFMPEG_DIR (BtbN's, pinned in
# package.yml). Built on Ubuntu 24.04, it needs a glibc and a libstdc++ at least that new.
# The FFmpeg makes the package GPL: target/package-linux/bundled-packages.txt names each
# Ubuntu package it carries a library of, for scripts/collect-kerf-linux-sources.sh.
#
#   scripts/package-kerf-linux.sh [--skip-run]
set -euo pipefail

skip_run=false
[ "${1:-}" = "--skip-run" ] && skip_run=true

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
: "${QT_ROOT_DIR:?the Qt to build with and carry (install-qt-action sets it)}"
: "${FFMPEG_DIR:?a shared GPL build of FFmpeg, with bin/ffmpeg and lib/}"
qt="$QT_ROOT_DIR"
ffmpeg="$FFMPEG_DIR"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
name="Industrial-Kerf-$version-linux-x64"
out="$root/target/package-linux"
stage="$out/$name"
commit="$(git rev-parse HEAD)"
system_list="$root/apps/kerf-qt/packaging/linux/system-libraries.txt"

for tool in patchelf readelf cmake; do
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
mkdir -p "$stage/bin" "$stage/lib" "$stage/plugins" "$stage/share/doc/industrial-kerf"

# Optimized: the editor's live preview and its exports draw frames in the bridge.
cargo build --release -p kerf-qt-bridge -p kerf-runtime --lib --bin kerfd
cargo build --release -p kerf-cli --bin kerf
cargo build --release -p kerf-mcp --bin kerf-mcp
cargo build --release -p kerf-guide
bridge="$root/target/release/libkerf_qt_bridge.so"
# The bridge links FFmpeg, which the linker has to find to link the editor to it.
cmake -S apps/kerf-qt -B target/qt-linux -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_PREFIX_PATH="$qt" -DKERF_BRIDGE_LIBRARY="$bridge" \
    -DCMAKE_EXE_LINKER_FLAGS="-Wl,-rpath-link,$ffmpeg/lib"
cmake --build target/qt-linux --target kerf-qt --parallel

# The editor looks beside itself for kerfd and kerf-mcp, and each of them for ffmpeg and
# ffprobe (kerf-engine's ffmpeg.rs).
cp target/qt-linux/kerf-qt "$stage/bin/"
for tool in kerfd kerf kerf-mcp; do cp "target/release/$tool" "$stage/bin/"; done
for tool in ffmpeg ffprobe; do cp "$ffmpeg/bin/$tool" "$stage/bin/"; done
cp "$bridge" "$stage/lib/"
# Cargo gives the bridge no soname, so the editor names it by the path it was linked from.
patchelf --set-soname libkerf_qt_bridge.so "$stage/lib/libkerf_qt_bridge.so"
while read -r needed; do
    case "$needed" in */*) patchelf --replace-needed "$needed" "$(basename "$needed")" "$stage/bin/kerf-qt" ;; esac
done < <(patchelf --print-needed "$stage/bin/kerf-qt")

# Qt's plugins for an X11 desktop (Wayland's run it through XWayland): the xcb platform
# and its GL integrations, compose keys and ibus, the portal's file dialogs, the image
# formats Qt reads, TLS, and network reachability. The editor draws its icons in code and
# uses no SVG.
for plugin in platforms/libqxcb.so xcbglintegrations platforminputcontexts/libcomposeplatforminputcontextplugin.so \
    platforminputcontexts/libibusplatforminputcontextplugin.so platformthemes/libqxdgdesktopportal.so \
    imageformats/libqgif.so imageformats/libqico.so imageformats/libqjpeg.so tls networkinformation; do
    source="$qt/plugins/$plugin"
    [ -e "$source" ] || { echo "Qt has no $plugin" >&2; exit 1; }
    mkdir -p "$stage/plugins/$(dirname "$plugin")"
    cp -R "$source" "$stage/plugins/$(dirname "$plugin")/"
done
cat > "$stage/bin/qt.conf" <<'EOF'
[Paths]
Prefix = ..
Libraries = lib
Plugins = plugins
EOF

# Every library one of these loads, and one of those, that the system does not provide:
# found where the build took it from, and copied into lib/.
system_libraries="$(grep -v '^#' "$system_list" | sed '/^$/d')"
system_library() { grep -qxF "$1" <<< "$system_libraries"; }
from_system="$out/.from-system"
: > "$from_system"
missing=""
find_library() {
    local soname="$1" folder
    for folder in "$qt/lib" "$ffmpeg/lib" "$root/target/release"; do
        [ -e "$folder/$soname" ] && { echo "$folder/$soname"; return; }
    done
    # The build machine's own, by the loader's cache.
    /sbin/ldconfig -p | awk -v soname="$soname" '$1 == soname && /x86-64/ {print $NF; exit}'
}
queue=()
while IFS= read -r -d '' file; do
    queue+=("$file")
done < <(find "$stage/bin" "$stage/lib" "$stage/plugins" -type f -print0)
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
        case "$found" in "$qt"/*|"$ffmpeg"/*|"$root"/*) ;; *) echo "$found" >> "$from_system" ;; esac
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
        "$stage"/plugins/*/*) rpath='$ORIGIN/../../lib' ;;
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

# The terms, the user guide, and where the source is.
cp LICENSE "$stage/LICENSE.txt"
cp -R licenses "$stage/licenses"
mkdir -p "$stage/fonts"
cp apps/kerf-qt/resources/fonts/*-LICENSE.txt "$stage/fonts/"
cp "$ffmpeg/LICENSE.txt" "$stage/COPYING-GPLv3.txt"
grep -q 'GNU GENERAL PUBLIC LICENSE' "$stage/COPYING-GPLv3.txt" || { echo "No GPL text in $ffmpeg/LICENSE.txt" >&2; exit 1; }
printf '%s\n' "$buildconf" > "$stage/FFmpeg-BUILD.txt"
# Help > Kerf User Guide opens it from bin/../share/doc/industrial-kerf (user_guide.cpp).
target/release/kerf-guide docs/user-guide.md "$stage/share/doc/industrial-kerf/user-guide.html" \
    --version "$version" --link-base "https://github.com/imjunseong958-droid/industrial-kerf/blob/$commit/docs/"
qt_version="$("$qt/bin/qtpaths6" --qt-query QT_VERSION 2>/dev/null || basename "$(dirname "$qt")")"
{
    cat apps/kerf-qt/packaging/linux/THIRD-PARTY.txt
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
package wherever you got it. It holds Kerf and its Rust crates, FFmpeg, each library built
into FFmpeg, the scripts that built them, Qt $qt_version, and the source of each Ubuntu
package a library in lib/ comes from.

Which licence covers which part is in THIRD-PARTY.txt.
EOF
cat > "$stage/README.txt" <<EOF
Industrial Kerf $version for Linux.

Run bin/kerf-qt. The folder can go anywhere; it needs no installing. It uses what every
desktop Linux has of its own (system-libraries.txt): glibc and libstdc++ as new as Ubuntu
24.04's, X11 (or XWayland), OpenGL, fontconfig, D-Bus, GLib and ALSA.

To have Kerf among your applications, and kerf:// review links open in it, run
./install-desktop-entry.sh once: it is for you alone and needs no administrator. Run it
again after moving the folder; ./install-desktop-entry.sh --remove takes it away. Its icon
is in icons/.
EOF
# The desktop entry and kerf:// links, which a folder cannot register of itself.
install -m 755 apps/kerf-qt/packaging/linux/install-desktop-entry.sh "$stage/install-desktop-entry.sh"
# Kerf's icon at each size that script installs for the entry (tools/kerf-icon makes them
# from apps/kerf-qt/resources/icon/industrial-kerf.svg).
mkdir -p "$stage/icons"
for size in 16 24 32 48 64 128 256 512; do
    cp "apps/kerf-qt/resources/icon/industrial-kerf-$size.png" "$stage/icons/"
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
