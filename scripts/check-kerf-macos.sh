#!/usr/bin/env bash
# Run Industrial Kerf.app as someone who unzipped it would, and fail if it needs
# anything it does not carry, or a newer macOS than its Info.plist says it runs on.
#
# With nothing of Homebrew's on PATH, runs ffmpeg -version, ffprobe -version,
# kerf --version and kerfd --endpoint from the app, and opens the editor's start
# window and takes a picture of it. kerfd imports a clip the app's ffmpeg made, which it
# probes right only with the app's ffprobe: a Mac without Homebrew has none on PATH.
# Each runs with DYLD_PRINT_LIBRARIES, so dyld
# names every library it loads: each has to be inside the app or macOS's own
# (/System, /usr/lib, /Library/Apple), wherever Homebrew is. A Mac that builds Kerf
# has Homebrew, where an @rpath or a fallback path can find what the app left out;
# on a Mac without it, a library left out stops dyld instead.
#
# package-kerf-macos.sh runs this on the app it makes, and
# .github/workflows/verify-release.yml on a published release, unzipped on a fresh
# runner with Homebrew moved away.
#
#   scripts/check-kerf-macos.sh "path/to/Industrial Kerf.app"
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 path/to/Industrial Kerf.app" >&2; exit 2; }
app="$(cd "$1" && pwd)"
# dyld names a library by the path it was found at, which may or may not go through
# a symbolic link such as /var: either is the app's.
app_real="$(cd "$1" && pwd -P)"
macos="$app/Contents/MacOS"
[ -x "$macos/kerf-qt" ] || { echo "No Contents/MacOS/kerf-qt in $app" >&2; exit 1; }

scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
clean_path="/usr/bin:/bin:/usr/sbin:/sbin"
loaded="$scratch/loaded.txt"
: > "$loaded"

# Runs a program from the app with a clean PATH and no dyld or Qt plugin settings of the
# caller's (a build machine's Qt may name its own plugins),
# keeping what dyld says it loaded and passing the program's own output through.
run() {
    local log="$scratch/dyld.log"
    local status=0
    env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH -u DYLD_FRAMEWORK_PATH \
        -u DYLD_FALLBACK_FRAMEWORK_PATH -u DYLD_INSERT_LIBRARIES \
        -u QT_PLUGIN_PATH -u QT_QPA_PLATFORM_PLUGIN_PATH -u QML2_IMPORT_PATH -u QML_IMPORT_PATH \
        PATH="$clean_path" DYLD_PRINT_LIBRARIES=1 "$@" 2> "$log" || status=$?
    # dyld's lines are "dyld[pid]: <uuid> /path"; the rest is the program's.
    grep '^dyld\[' "$log" | grep -o '/.*' >> "$loaded" || true
    grep -v '^dyld\[' "$log" >&2 || true
    if [ "$status" -ne 0 ]; then
        echo "$(basename "$1") did not run from the app (exit status $status)" >&2
        return 1
    fi
}

# Nothing in the app may need a newer macOS than its Info.plist says it runs on
# (LSMinimumSystemVersion, which package-kerf-macos.sh sets): dyld refuses such a library
# on the older macOS, and the app fails to start without saying why. Read from each
# Mach-O file's LC_BUILD_VERSION, before anything runs. An app from before 0.1.2 names
# no minimum.
version_number() { awk -F. '{printf "%d\n", $1 * 10000 + $2 * 100 + $3}' <<< "$1"; }
minimum="$(plutil -extract LSMinimumSystemVersion raw -o - "$app/Contents/Info.plist" 2> /dev/null || true)"
if [ -n "$minimum" ]; then
    newer=""
    while IFS= read -r -d '' file; do
        [[ "$(file -b "$file")" == *Mach-O* ]] || continue
        # awk reads to the end: one that stopped at the first minos would break otool's
        # pipe, and with pipefail the check.
        needs="$(otool -l "$file" | awk '/LC_BUILD_VERSION/ {seen = 1} seen && $1 == "minos" && !found {print $2; found = 1}')"
        [ -n "$needs" ] || continue
        if [ "$(version_number "$needs")" -gt "$(version_number "$minimum")" ]; then
            newer+="${file#"$app"/}: macOS $needs"$'\n'
        fi
    done < <(find "$app/Contents" -type f -print0)
    if [ -n "$newer" ]; then
        printf 'The app says it runs on macOS %s, but these need a newer one:\n%s' "$minimum" "$newer" >&2
        exit 1
    fi
    echo "Nothing in the app needs a newer macOS than the $minimum its Info.plist names."
fi

run "$macos/ffmpeg" -version > /dev/null
run "$macos/ffprobe" -version > /dev/null
run "$macos/kerf" --version > /dev/null
run "$macos/kerfd" --dir "$scratch/kerfd" --endpoint > "$scratch/endpoint"
[ -s "$scratch/endpoint" ] || { echo "kerfd said no endpoint" >&2; exit 1; }
# kerfd looks for ffmpeg and ffprobe beside itself: without them it still imports, at a
# made-up size, rate and length.
run "$macos/ffmpeg" -v error -f lavfi -i testsrc=duration=2:size=320x240:rate=25 -c:v mpeg4 "$scratch/clip.mp4"
printf '%s\n' \
    '{"protocolVersion":1,"id":"import","operation":"import","actor":"ui","idempotencyKey":"check-import","baseRevision":0,"path":"'"$scratch/clip.mp4"'"}' \
    '{"protocolVersion":1,"id":"state","operation":"get_state"}' |
    run "$macos/kerfd" --stdio --dir "$scratch/project" > "$scratch/import.json"
if ! grep -q 'rate=25 dur=00:00:02:00 video=320x240' "$scratch/import.json"; then
    echo "kerfd did not probe the clip with the app's ffprobe (it is 320x240, 25 fps, 2 s):" >&2
    grep -o 'rate=[^ ]* dur=[^ ]* video=[0-9x]*' "$scratch/import.json" >&2 ||
        grep '"id":"import"' "$scratch/import.json" >&2
    exit 1
fi
# On the Mac's own window system, as someone opening the app gets: the app carries
# only that platform plugin.
run "$macos/kerf-qt" --settings-dir "$scratch/settings" \
    --screenshot "$scratch/start.png" --quit-after-ms 3000
[ -s "$scratch/start.png" ] || { echo "kerf-qt ran and made no screenshot" >&2; exit 1; }

# The platform plugin is loaded once the window is made: without it in the list,
# dyld did not say what it loaded, and the list proves nothing.
grep -q '/PlugIns/platforms/libqcocoa.dylib$' "$loaded" || {
    echo "dyld named no platform plugin for the editor: DYLD_PRINT_LIBRARIES had no effect" >&2
    exit 1
}
outside="$(sort -u "$loaded" | grep -vF -e "$app/" -e "$app_real/" |
    grep -vE '^(/System/|/usr/lib/|/Library/Apple/)' || true)"
if [ -n "$outside" ]; then
    printf 'The app loaded libraries from outside itself:\n%s\n' "$outside" >&2
    exit 1
fi
echo "The app ran on its own; it loaded $(sort -u "$loaded" | grep -cF -e "$app/" -e "$app_real/") libraries of its own, the rest macOS's."
