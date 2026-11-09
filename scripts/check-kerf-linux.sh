#!/usr/bin/env bash
# Run an unpacked Linux package of Kerf as someone who unpacked it would, and fail if it
# needs anything it does not carry beyond what it takes from the system on purpose
# (system-libraries.txt).
#
# Each library a binary in the package names has to be in its lib/ or in that list, and be
# found there by the loader with no LD_LIBRARY_PATH (ldd). Then, with nothing on PATH and
# no library or Qt plugin path, it runs ffmpeg -version, ffprobe -version, kerf --version
# and kerfd --endpoint, has kerfd import a clip the package's ffmpeg made (which kerfd
# probes right only with the package's ffprobe), and opens the editor's start window on a
# virtual X server and takes a picture of it. Each runs with LD_DEBUG=libs, so the loader
# names every library it loads: none of what the package carries, Qt's plugins, Qt, ICU or
# FFmpeg may come from anywhere else. What the system's own libraries load in turn (Mesa's
# driver, GLib's PCRE) is the system's affair.
#
# package-kerf-linux.sh runs this on the folder it makes, and
# .github/workflows/verify-release.yml on a published release, unpacked on a fresh runner.
#
#   scripts/check-kerf-linux.sh path/to/Industrial-Kerf-<version>-linux-x64
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 path/to/the/unpacked/package" >&2; exit 2; }
package="$(cd "$1" && pwd -P)"
bin="$package/bin"
[ -x "$bin/kerf-qt" ] || { echo "No bin/kerf-qt in $package" >&2; exit 1; }
command -v xvfb-run > /dev/null || { echo "xvfb-run is needed (apt install xvfb)" >&2; exit 1; }
system_libraries="$(grep -v '^#' "$package/system-libraries.txt" | sed '/^$/d')"
[ -n "$system_libraries" ] || { echo "No system-libraries.txt in $package" >&2; exit 1; }

scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/ld"

# Runs a program from the package with nothing on PATH and none of the caller's library or
# Qt plugin paths, keeping what the loader says it loaded, per process, in $scratch/ld.
run() {
    env -u LD_LIBRARY_PATH -u LD_PRELOAD -u QT_PLUGIN_PATH -u QT_QPA_PLATFORM_PLUGIN_PATH \
        -u QML2_IMPORT_PATH -u QML_IMPORT_PATH -u QT_QPA_PLATFORM \
        PATH=/nonexistent LD_DEBUG=libs LD_DEBUG_OUTPUT="$scratch/ld/trace" "$@"
}

carried="$(find "$package/lib" -maxdepth 1 -type f -printf '%f\n' | sort -u)"
system_library() { grep -qxF "$1" <<< "$system_libraries"; }

# Every library a binary in the package names: carried, or the system's on purpose; and
# found, by the loader with no library path of the caller's, where it should be.
wrong=""
while IFS= read -r -d '' file; do
    head -c 4 "$file" | grep -q 'ELF' || continue
    needed="$(readelf -d "$file" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')"
    resolved="$(env -u LD_LIBRARY_PATH ldd "$file" 2>&1 || true)"
    for soname in $needed; do
        # ldd names the loader by its path alone; it is the system's.
        case "$soname" in ld-linux*) continue ;; esac
        where="$(awk -v soname="$soname" '$1 == soname {print $3; exit}' <<< "$resolved")"
        case "$where" in /*) where="$(readlink -f "$where")" ;; esac
        if grep -qxF "$soname" <<< "$carried"; then
            [ "$where" = "$package/lib/$soname" ] || wrong+="${file#"$package"/}: $soname from ${where:-nowhere}, not lib/"$'\n'
        elif system_library "$soname"; then
            case "$where" in /*) ;; *) wrong+="${file#"$package"/}: the system has no $soname"$'\n' ;; esac
        else
            wrong+="${file#"$package"/}: $soname, neither carried nor the system's"$'\n'
        fi
    done
done < <(find "$package" -type f -print0)
if [ -n "$wrong" ]; then
    printf 'The package does not hold together:\n%s' "$wrong" >&2
    exit 1
fi

run "$bin/ffmpeg" -version > /dev/null
run "$bin/ffprobe" -version > /dev/null
run "$bin/kerf" --version > /dev/null
run "$bin/kerfd" --dir "$scratch/kerfd" --endpoint > "$scratch/endpoint"
[ -s "$scratch/endpoint" ] || { echo "kerfd said no endpoint" >&2; exit 1; }
# kerfd looks for ffmpeg and ffprobe beside itself: without them it still imports, at a
# made-up size, rate and length.
run "$bin/ffmpeg" -v error -f lavfi -i testsrc=duration=2:size=320x240:rate=25 -c:v mpeg4 "$scratch/clip.mp4"
printf '%s\n' \
    '{"protocolVersion":1,"id":"import","operation":"import","actor":"ui","idempotencyKey":"check-import","baseRevision":0,"path":"'"$scratch/clip.mp4"'"}' \
    '{"protocolVersion":1,"id":"state","operation":"get_state"}' |
    run "$bin/kerfd" --stdio --dir "$scratch/project" > "$scratch/import.json"
if ! grep -q 'rate=25 dur=00:00:02:00 video=320x240' "$scratch/import.json"; then
    echo "kerfd did not probe the clip with the package's ffprobe (it is 320x240, 25 fps, 2 s):" >&2
    grep -o 'rate=[^ ]* dur=[^ ]* video=[0-9x]*' "$scratch/import.json" >&2 ||
        grep '"id":"import"' "$scratch/import.json" >&2
    exit 1
fi
# On an X server, as a desktop gives it: xvfb-run gives the editor DISPLAY and XAUTHORITY.
xvfb-run -a -s "-screen 0 1600x1200x24" \
    env -u LD_LIBRARY_PATH -u LD_PRELOAD -u QT_PLUGIN_PATH -u QT_QPA_PLATFORM_PLUGIN_PATH \
    -u QML2_IMPORT_PATH -u QML_IMPORT_PATH -u QT_QPA_PLATFORM \
    PATH=/nonexistent XDG_RUNTIME_DIR="$scratch" LD_DEBUG=libs LD_DEBUG_OUTPUT="$scratch/ld/trace" \
    "$bin/kerf-qt" --settings-dir "$scratch/settings" --screenshot "$scratch/start.png" --quit-after-ms 3000
[ -s "$scratch/start.png" ] || { echo "kerf-qt ran and made no screenshot" >&2; exit 1; }

# What each process loaded: a "calling init" line for each library.
loaded="$(cat "$scratch"/ld/trace.* | sed -n 's/^ *[0-9]*:[[:space:]]*calling init: //p' | sort -u)"
grep -q '/plugins/platforms/libqxcb.so$' <<< "$loaded" || {
    echo "The loader named no xcb platform plugin for the editor: LD_DEBUG had no effect" >&2
    exit 1
}
# What the package carries, or any Qt, Qt plugin, ICU or FFmpeg, loaded from outside it: a
# library it left out, or one of the machine's own Qt found before its own.
outside="$(grep -vF "$package/" <<< "$loaded" | while IFS= read -r library; do
    name="$(basename "$library")"
    if grep -qxF "$name" <<< "$carried"; then echo "$library"; continue; fi
    case "$name" in libQt6*|libicu*|libav*|libsw*|libpostproc*|libkerf*) echo "$library"; continue ;; esac
    case "$library" in */plugins/*|*/qt6/*) echo "$library" ;; esac
done)"
if [ -n "$outside" ]; then
    printf 'The package loaded from outside it what it carries or should:\n%s\n' "$outside" >&2
    exit 1
fi
echo "The package ran on its own; it loaded $(grep -cF "$package/" <<< "$loaded") libraries of its own, the rest the system's."
