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
# driver, GLib's PCRE) is the system's affair. Last, its install-desktop-entry.sh, for a
# user in a scratch home: the entry runs this folder's editor, takes kerf:// links, and, where
# GLib's gio is there, a link followed through it starts the editor with the link; and the
# entry's icon is Kerf's own, found in the user's hicolor theme at each size.
#
# package-kerf-linux.sh runs this on the folder it makes, and
# .github/workflows/verify-release.yml on a published release, unpacked on a fresh runner.
#
#   scripts/check-kerf-linux.sh path/to/Industrial-Kerf-<version>-linux-x64
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 path/to/the/unpacked/package" >&2; exit 2; }
# What is asked of a desktop entry, here and of the .deb's (check-kerf-linux-deb.sh).
. "$(dirname "$0")/kerf-linux-desktop.sh"
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

# The desktop entry and kerf:// links, for a user whose home is in the scratch folder:
# install-desktop-entry.sh writes an entry that runs this folder's editor with the link and
# makes it the default for x-scheme-handler/kerf; GLib, through which GNOME and most
# desktops open a link, finds it so and follows a link through it; and --remove takes both
# away. Its icon is Kerf's own, from the package's icons/. A package from before 0.1.2 has
# no such script, and no icon.
if [ -e "$package/install-desktop-entry.sh" ]; then
    home="$scratch/home"
    entry="$home/.local/share/applications/industrial-kerf.desktop"
    mimeapps="$home/.config/mimeapps.list"
    as_user() {
        env -u LD_LIBRARY_PATH -u LD_PRELOAD -u QT_PLUGIN_PATH -u QT_QPA_PLATFORM_PLUGIN_PATH \
            -u QML2_IMPORT_PATH -u QML_IMPORT_PATH -u QT_QPA_PLATFORM -u XDG_DATA_HOME -u XDG_CONFIG_HOME \
            HOME="$home" XDG_RUNTIME_DIR="$scratch" "$@"
    }
    as_user "$package/install-desktop-entry.sh" > /dev/null
    kerf_check_entry "$entry" "$bin/kerf-qt"
    grep -qxF 'x-scheme-handler/kerf=industrial-kerf.desktop' "$mimeapps" ||
        { echo "mimeapps.list does not send kerf:// links to Kerf" >&2; exit 1; }
    kerf_check_icons "$home/.local/share/icons/hicolor" "$package/icons"
    if command -v gio > /dev/null; then
        kerf_takes_links as_user
        kerf_follow_link "$bin/kerf-qt" as_user
    fi
    as_user "$package/install-desktop-entry.sh" --remove > /dev/null
    [ ! -e "$entry" ] || { echo "--remove left $entry" >&2; exit 1; }
    left="$(find "$home/.local/share" -name 'industrial-kerf*' 2> /dev/null)"
    [ -z "$left" ] || { printf -- "--remove left Kerf's icons:\n%s\n" "$left" >&2; exit 1; }
    if grep -q 'industrial-kerf' "$mimeapps"; then
        echo "--remove left Kerf in mimeapps.list:" >&2
        cat "$mimeapps" >&2
        exit 1
    fi
    echo "The desktop entry runs this folder's editor under Kerf's own icon, takes kerf:// links, and comes away cleanly."
fi
echo "The package ran on its own; it loaded $(grep -cF "$package/" <<< "$loaded") libraries of its own, the rest the system's."
