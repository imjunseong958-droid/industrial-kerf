#!/usr/bin/env bash
# Industrial Kerf for macOS as an app that runs on its own: the editor (kerf-editor, one
# Rust binary drawn with GPUI and Metal), kerfd, kerf, kerf-mcp, ffmpeg and ffprobe, with
# FFmpeg and every Homebrew library they need copied into the bundle, zipped as
# target/package-macos/<name>.zip with its sha256.
#
# Runs on a Mac with Xcode (GPUI compiles its Metal shaders with it), Rust, and Homebrew's
# ffmpeg and pkg-config, as CI's macOS job installs them. Homebrew's FFmpeg is GPL, so the zip is too: it says so, and
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
ffmpeg_prefix="$(brew --prefix ffmpeg)"
commit="$(git rev-parse HEAD)"

rm -rf "$out"
mkdir -p "$out"

# Optimized, as the editor draws frames in kerf-bridge, which it links.
cargo build --release --manifest-path apps/kerf-editor/Cargo.toml
cargo build --release -p kerf-runtime --bin kerfd
cargo build --release -p kerf-cli --bin kerf
cargo build --release -p kerf-mcp --bin kerf-mcp
scripts/bundle-kerf-macos.sh target/release/kerf-editor "$app"
mkdir -p "$frameworks"

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

# Every library from Homebrew that one of these loads, and one of those, copied into
# Frameworks and named there by @rpath, which each executable finds through its rpath,
# @executable_path/../Frameworks. Each copy is followed from where it came from, since
# Homebrew names a library's own dependencies by Homebrew's paths, or by @loader_path or
# @rpath relative to it. What is not Homebrew's is macOS's own.
copies=()
origins=()
for executable in kerf-editor "${tools[@]}"; do
    copies+=("$macos/$executable")
    origins+=("$root/target/release/$executable")
done
for tool in ffmpeg ffprobe; do
    copies+=("$macos/$tool")
    origins+=("$ffmpeg_prefix/bin/$tool")
done
while [ "${#copies[@]}" -gt 0 ]; do
    copy="${copies[0]}"
    origin="$(cd "$(dirname "${origins[0]}")" && pwd -P)/$(basename "${origins[0]}")"
    copies=("${copies[@]:1}")
    origins=("${origins[@]:1}")
    own="$(otool -D "$copy" 2>/dev/null | tail -n +2)"
    while IFS= read -r ref; do
        library="$(basename "$ref")"
        source=""
        case "$ref" in
            /System/*|/usr/lib/*) continue ;;
            "$brew_prefix"/*|/opt/homebrew/*|/usr/local/*) source="$ref" ;;
            @loader_path/*) source="$(dirname "$origin")/${ref#@loader_path/}" ;;
            @rpath/*)
                for dir in "$(dirname "$origin")" "$brew_prefix/lib"; do
                    if [ -e "$dir/${ref#@rpath/}" ]; then source="$dir/${ref#@rpath/}"; break; fi
                done
                ;;
            *) continue ;;
        esac
        [ -n "$source" ] && [ -e "$source" ] || { echo "$(basename "$copy") loads $ref, which is nowhere" >&2; exit 1; }
        if [ ! -e "$frameworks/$library" ]; then
            cp -fL "$source" "$frameworks/$library"
            chmod 755 "$frameworks/$library"
            install_name_tool -id "@rpath/$library" "$frameworks/$library"
            copies+=("$frameworks/$library")
            origins+=("$source")
        fi
        [ "$ref" = "@rpath/$library" ] || install_name_tool -change "$ref" "@rpath/$library" "$copy"
    done < <(otool -L "$copy" | tail -n +2 | awk '{print $1}' | grep -vxF "${own:-//}")
done
for executable in kerf-editor "${tools[@]}" ffmpeg ffprobe; do
    install_name_tool -add_rpath @executable_path/../Frameworks "$macos/$executable"
done

# Homebrew's libraries keep rpaths into the Cellar. For an @rpath reference dyld tries the rpaths of the library and of whatever loaded
# it, so each of these could find a library outside the app before the app's own,
# on a Mac that has one there. Only rpaths that lead inside the app stay.
while IFS= read -r -d '' file; do
    file -b "$file" | grep -q 'Mach-O' || continue
    dir="$(dirname "$file")"
    otool -l "$file" | awk '/cmd LC_RPATH/ {getline; getline; print $2}' | while IFS= read -r rpath; do
        case "$rpath" in
            @loader_path*) leads_to="$dir${rpath#@loader_path}" ;;
            @executable_path*) leads_to="$macos${rpath#@executable_path}" ;;
            *) leads_to="$rpath" ;;
        esac
        leads_to="$(python3 -c 'import os, sys; print(os.path.normpath(sys.argv[1]))' "$leads_to")"
        case "$leads_to/" in
            "$app"/*) ;;
            *) install_name_tool -delete_rpath "$rpath" "$file" ;;
        esac
    done
done < <(find "$app" -type f -print0)

# What still points outside the bundle would fail on a Mac without Homebrew.
outside=""
while IFS= read -r -d '' file; do
    if file -b "$file" | grep -q 'Mach-O'; then
        # A library's own id (otool -D) is not a reference to load.
        own="$(otool -D "$file" 2>/dev/null | tail -n +2)"
        refs="$(otool -L "$file" | tail -n +2 | awk '{print $1}' | grep -vxF "${own:-//}" |
            grep -E "^($brew_prefix|/usr/local|/opt/homebrew)/" || true)"
        [ -n "$refs" ] && outside+="$file:"$'\n'"$refs"$'\n'
    fi
done < <(find "$app" -type f -print0)
if [ -n "$outside" ]; then
    printf 'These still load libraries from outside the app:\n%s' "$outside" >&2
    exit 1
fi

# Which Homebrew formulae the bundle carries files of, with their versions and
# licences, for the source archive and THIRD-PARTY.txt: resolving each original from the
# build tree finds its formula.
formulae="$out/bundled-formulae.txt"
: > "$formulae.all"
# Walk the dependency tree of the originals (not the rewritten copies).
seen="$out/.seen"
: > "$seen"
queue=("target/release/kerf-editor" "target/release/kerfd" "target/release/kerf"
    "target/release/kerf-mcp" "$ffmpeg_prefix/bin/ffmpeg" "$ffmpeg_prefix/bin/ffprobe")
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
                # Homebrew's own, beside the library or in its lib folder.
                for dir in "$(dirname "$real")" "$brew_prefix/lib"; do
                    if [ -e "$dir/${ref#@rpath/}" ]; then
                        queue+=("$dir/${ref#@rpath/}")
                        break
                    fi
                done
                ;;
            @loader_path/*) queue+=("$(dirname "$real")/${ref#@loader_path/}") ;;
            "$brew_prefix"/*|/opt/homebrew/*|/usr/local/*) queue+=("$ref") ;;
        esac
    done < <(otool -L "$real" 2>/dev/null | tail -n +2 | awk '{print $1}')
done
sort -u "$formulae.all" | tr '/' ' ' > "$formulae"
rm -f "$formulae.all" "$seen"
if ! grep -q '^ffmpeg ' "$formulae"; then
    echo "bundled-formulae.txt names no ffmpeg formula:" >&2
    cat "$formulae" >&2
    exit 1
fi

# The terms, and where the source is.
cp LICENSE "$app/Contents/Resources/LICENSE.txt"
cp -R licenses "$app/Contents/Resources/licenses"
# The fonts' licences are in Resources/fonts already (bundle-kerf-macos.sh).
gpl="$(brew --prefix ffmpeg)/share/doc/ffmpeg/COPYING.GPLv3"
[ -f "$gpl" ] || gpl="$(find "$(brew --cellar ffmpeg)" -name 'COPYING.GPLv3' | head -n 1)"
if [ -f "$gpl" ]; then
    cp "$gpl" "$app/Contents/Resources/COPYING-GPLv3.txt"
else
    curl -fsSL --retry 5 -o "$app/Contents/Resources/COPYING-GPLv3.txt" https://www.gnu.org/licenses/gpl-3.0.txt
fi
"$macos/ffmpeg" -hide_banner -buildconf > "$app/Contents/Resources/FFmpeg-BUILD.txt"
{
    cat apps/kerf-editor/packaging/macos/THIRD-PARTY.txt
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
this app wherever you got it. It holds Kerf and its Rust crates (GPUI, which draws
the editor, among them), and the source of each Homebrew formula whose files are in
the app, FFmpeg among them, as Homebrew builds them.

Which licence covers which part is in THIRD-PARTY.txt.
EOF

# The oldest macOS the app runs on, so an older one says so rather than the app failing
# to start. It is 26 because the editor, FFmpeg and the Homebrew libraries the app
# carries come bottled for the runner's macOS 26; going lower would take an older
# runner or building those libraries from source, which is not worth it. Only the package says it: an
# editor built on a developer's older Mac still opens there. The runner's Homebrew bottles
# its libraries for the runner's own macOS, so a newer runner needs a newer macOS, and
# check-kerf-macos.sh then fails rather than let the app need more than it says.
minimum_macos="26.0"
plutil -replace LSMinimumSystemVersion -string "$minimum_macos" "$app/Contents/Info.plist"

# Signed again inside out, ad hoc: macOS refuses code whose signature no longer
# matches what install_name_tool changed.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

# With nothing of Homebrew's on PATH it has to find all it needs inside itself, and
# load nothing from outside, which dyld says.
$skip_run || scripts/check-kerf-macos.sh "$app"

(cd "$out" && ditto -c -k --sequesterRsrc --keepParent "Industrial Kerf.app" "$name.zip")
(cd "$out" && shasum -a 256 "$name.zip" > "$name.zip.sha256")
echo "$out/$name.zip"
