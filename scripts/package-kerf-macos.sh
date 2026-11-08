#!/usr/bin/env bash
# Industrial Kerf for macOS as an app that runs on its own: the editor, kerfd, kerf,
# kerf-mcp, ffmpeg and ffprobe, with Qt, FFmpeg and every Homebrew library they need
# copied into the bundle, zipped as target/package-macos/<name>.zip with its sha256.
#
# Runs on a Mac with Homebrew's ffmpeg, qt, pkg-config and cmake, as CI's macOS job
# installs them. Homebrew's FFmpeg is GPL, so the zip is too: it says so, and
# target/package-macos/bundled-formulae.txt names each Homebrew formula whose files
# went in, for scripts/collect-kerf-macos-sources.sh to fetch the source of.
#
# The app is signed ad hoc, not with a Developer ID: macOS asks before it first opens
# (right-click > Open, or System Settings > Privacy & Security > Open Anyway).
#
#   scripts/package-kerf-macos.sh [--skip-run]
set -euo pipefail

skip_run=false
[ "${1:-}" = "--skip-run" ] && skip_run=true

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
arch="$(uname -m)"
name="Industrial-Kerf-$version-macos-$arch"
out="$root/target/package-macos"
app="$out/Industrial Kerf.app"
macos="$app/Contents/MacOS"
frameworks="$app/Contents/Frameworks"
brew_prefix="$(brew --prefix)"
qt_prefix="$(brew --prefix qt)"
ffmpeg_prefix="$(brew --prefix ffmpeg)"
commit="$(git rev-parse HEAD)"

rm -rf "$out"
mkdir -p "$out"

# The bridge and the tools optimized, as the editor draws frames inside the bridge.
cargo build --release -p kerf-qt-bridge -p kerf-runtime --lib --bin kerfd
cargo build --release -p kerf-cli --bin kerf
cargo build --release -p kerf-mcp --bin kerf-mcp
bridge="$root/target/release/libkerf_qt_bridge.dylib"
cmake -S apps/kerf-qt -B target/qt-release -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_PREFIX_PATH="$qt_prefix" -DKERF_BRIDGE_LIBRARY="$bridge"
cmake --build target/qt-release --target kerf-qt --parallel

cp -R target/qt-release/kerf-qt.app "$app"
mkdir -p "$frameworks"

# The bridge in Frameworks, found through the editor's rpath, as the local install does.
editor="$macos/kerf-qt"
bridge_ref="$(otool -L "$editor" | awk '/libkerf_qt_bridge/ {print $1; exit}')"
cp -fL "$bridge" "$frameworks/libkerf_qt_bridge.dylib"
chmod 755 "$frameworks/libkerf_qt_bridge.dylib"
install_name_tool -id @rpath/libkerf_qt_bridge.dylib "$frameworks/libkerf_qt_bridge.dylib"
install_name_tool -change "$bridge_ref" @rpath/libkerf_qt_bridge.dylib "$editor"
for rpath in $(otool -l "$editor" | awk '/LC_RPATH/ {getline; getline; print $2}'); do
    case "$rpath" in "$root"/*) install_name_tool -delete_rpath "$rpath" "$editor" ;; esac
done
install_name_tool -add_rpath @executable_path/../Frameworks "$editor"

# The editor looks beside itself for kerfd and kerf-mcp, and kerfd for ffmpeg and ffprobe.
tools=(kerfd kerf kerf-mcp)
for tool in "${tools[@]}"; do
    cp -fL "target/release/$tool" "$macos/$tool"
    chmod 755 "$macos/$tool"
done
for tool in ffmpeg ffprobe; do
    cp -fL "$ffmpeg_prefix/bin/$tool" "$macos/$tool"
    chmod 755 "$macos/$tool"
done

# Qt's frameworks and plugins, and every library from Homebrew that any of these
# executables or the bridge loads, copied in with their references rewritten.
executables=()
for tool in "${tools[@]}" ffmpeg ffprobe; do executables+=("-executable=$macos/$tool"); done
"$qt_prefix/bin/macdeployqt" "$app" -always-overwrite -no-strip \
    "-executable=$frameworks/libkerf_qt_bridge.dylib" "${executables[@]}"

# What still points outside the bundle would fail on a Mac without Homebrew.
outside=""
while IFS= read -r -d '' file; do
    if file -b "$file" | grep -q 'Mach-O'; then
        refs="$(otool -L "$file" | tail -n +2 | awk '{print $1}' | grep -E "^($brew_prefix|/usr/local|/opt/homebrew)/" || true)"
        [ -n "$refs" ] && outside+="$file:"$'\n'"$refs"$'\n'
    fi
done < <(find "$app" -type f -print0)
if [ -n "$outside" ]; then
    printf 'These still load libraries from outside the app:\n%s' "$outside" >&2
    exit 1
fi

# Which Homebrew formulae the bundle carries files of, with their versions and
# licences, for the source archive and THIRD-PARTY.txt. macdeployqt names a copied
# library as it was named in the Cellar; resolving each original from the build tree
# finds its formula.
formulae="$out/bundled-formulae.txt"
: > "$formulae.all"
# Walk the dependency tree of the originals (not the rewritten copies).
seen="$out/.seen"
: > "$seen"
queue=("target/qt-release/kerf-qt.app/Contents/MacOS/kerf-qt" "$bridge" "target/release/kerfd"
    "target/release/kerf" "target/release/kerf-mcp" "$ffmpeg_prefix/bin/ffmpeg" "$ffmpeg_prefix/bin/ffprobe")
for plugin in $(find "$app/Contents/PlugIns" -name '*.dylib' 2>/dev/null); do
    queue+=("$qt_prefix/share/qt/plugins/${plugin#"$app/Contents/PlugIns/"}")
done
while [ "${#queue[@]}" -gt 0 ]; do
    current="${queue[0]}"
    queue=("${queue[@]:1}")
    [ -e "$current" ] || continue
    real="$(cd "$(dirname "$current")" && pwd -P)/$(basename "$current")"
    grep -qxF "$real" "$seen" && continue
    echo "$real" >> "$seen"
    case "$real" in
        "$brew_prefix"/Cellar/*)
            echo "${real#"$brew_prefix"/Cellar/}" | cut -d/ -f1-2 >> "$formulae.all"
            ;;
    esac
    while IFS= read -r ref; do
        case "$ref" in
            @rpath/*)
                # Homebrew's Qt frameworks, the only @rpath references outside the build.
                candidate="$qt_prefix/lib/${ref#@rpath/}"
                [ -e "$candidate" ] && queue+=("$candidate")
                ;;
            "$brew_prefix"/*|/opt/homebrew/*|/usr/local/*) queue+=("$ref") ;;
        esac
    done < <(otool -L "$real" 2>/dev/null | tail -n +2 | awk '{print $1}')
done
sort -u "$formulae.all" | tr '/' ' ' > "$formulae"
rm -f "$formulae.all" "$seen"
if ! grep -q '^ffmpeg ' "$formulae" || ! grep -q '^qt' "$formulae"; then
    echo "bundled-formulae.txt names no ffmpeg or Qt formula:" >&2
    cat "$formulae" >&2
    exit 1
fi

# The terms, and where the source is.
cp LICENSE "$app/Contents/Resources/LICENSE.txt"
cp -R licenses "$app/Contents/Resources/licenses"
mkdir -p "$app/Contents/Resources/fonts"
cp apps/kerf-qt/resources/fonts/*-LICENSE.txt "$app/Contents/Resources/fonts/"
gpl="$(brew --prefix ffmpeg)/share/doc/ffmpeg/COPYING.GPLv3"
[ -f "$gpl" ] || gpl="$(find "$(brew --cellar ffmpeg)" -name 'COPYING.GPLv3' | head -n 1)"
if [ -f "$gpl" ]; then
    cp "$gpl" "$app/Contents/Resources/COPYING-GPLv3.txt"
else
    curl -fsSL --retry 5 -o "$app/Contents/Resources/COPYING-GPLv3.txt" https://www.gnu.org/licenses/gpl-3.0.txt
fi
"$macos/ffmpeg" -hide_banner -buildconf > "$app/Contents/Resources/FFmpeg-BUILD.txt"
{
    cat apps/kerf-qt/packaging/macos/THIRD-PARTY.txt
    echo
    echo "Every Homebrew formula with files in this app, its version, and its licence:"
    while read -r formula formula_version; do
        license="$(brew info --json=v2 "$formula" | python3 -c 'import json,sys; f=json.load(sys.stdin)["formulae"][0]; print(f.get("license") or "see its source")')"
        printf '  %s %s: %s\n' "$formula" "$formula_version" "$license"
    done < "$formulae"
} > "$app/Contents/Resources/THIRD-PARTY.txt"
cat > "$app/Contents/Resources/SOURCE.txt" <<EOF
Industrial Kerf $version for macOS ($arch), from commit $commit.
Its source: https://github.com/imjunseong958-droid/industrial-kerf

This app is under the GNU General Public License version 3 (COPYING-GPLv3.txt), and
you have the right to its complete source. That is $name-source.tar, published beside
this app wherever you got it. It holds Kerf and its Rust crates, and the source of
each Homebrew formula whose files are in the app, FFmpeg and Qt among them, as
Homebrew builds them.

Which licence covers which part is in THIRD-PARTY.txt.
EOF

# Signed again inside out, ad hoc: macOS refuses code whose signature no longer
# matches what install_name_tool changed.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

if ! $skip_run; then
    scratch="$(mktemp -d)"
    trap 'rm -rf -- "$scratch"' EXIT
    # With nothing of Homebrew's on PATH, it has to find all it needs inside itself.
    clean_path="/usr/bin:/bin:/usr/sbin:/sbin"
    for check in "ffmpeg -version" "ffprobe -version" "kerf --version"; do
        set -- $check
        PATH="$clean_path" "$macos/$1" "$2" > /dev/null
    done
    endpoint="$(PATH="$clean_path" "$macos/kerfd" --dir "$scratch" --endpoint)"
    [ -n "$endpoint" ] || { echo "kerfd did not run from the app" >&2; exit 1; }
    PATH="$clean_path" QT_QPA_PLATFORM=offscreen "$editor" --settings-dir "$scratch/settings" \
        --screenshot "$scratch/start.png" --quit-after-ms 3000
    [ -s "$scratch/start.png" ] || { echo "kerf-qt ran and made no screenshot" >&2; exit 1; }
fi

(cd "$out" && ditto -c -k --sequesterRsrc --keepParent "Industrial Kerf.app" "$name.zip")
(cd "$out" && shasum -a 256 "$name.zip" > "$name.zip.sha256")
echo "$out/$name.zip"
