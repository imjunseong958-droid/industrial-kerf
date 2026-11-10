#!/usr/bin/env bash
# Install Kerf's .deb as someone would, with apt, and fail if what it installs does not
# run, is not among the applications, does not open kerf:// links, or does not come away
# whole.
#
# apt installs it without its Recommends, so its Depends alone have to bring every library
# its binaries name: before anything else is installed, the loader has to find each (ldd).
# On a system with a desktop that says little; in a bare container (debian:trixie,
# ubuntu:24.04), where package.yml and verify-release.yml run this, it is what shows the
# Depends to be whole. Then it installs what the checks themselves need (Xvfb, binutils,
# GLib's gio, desktop-file-utils, procps) and runs scripts/check-kerf-linux.sh on
# /opt/industrial-kerf: the folder as installed, with nothing on PATH. After that, what a
# .deb adds to the folder: kerf, kerfd, kerf-mcp and kerf-qt on PATH, each working through
# its link (kerfd imports a clip, which it probes right only if it finds the ffprobe beside
# the file it really is, and the editor opens its start window); the desktop entry in
# /usr/share/applications, running the installed editor with the link; Kerf's icon in the
# system's hicolor theme at each size; GLib taking Kerf for kerf:// links for a user who
# has set nothing, and a link followed through it starting the installed editor. Last,
# apt remove, after which none of its files, its folder, nor its claim on kerf:// is left.
#
# It installs and removes packages: run it as root, in a container or on a machine that can
# take that.
#
#   scripts/check-kerf-linux-deb.sh path/to/Industrial-Kerf-<version>-linux-x64.deb
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 path/to/the.deb" >&2; exit 2; }
[ -f "$1" ] || { echo "No .deb at $1" >&2; exit 1; }
deb="$(readlink -f "$1")"
[ "$(id -u)" = 0 ] || { echo "Run as root: this installs the package with apt, and removes it" >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/kerf-linux-desktop.sh"
prefix=/opt/industrial-kerf
entry=/usr/share/applications/industrial-kerf.desktop
export DEBIAN_FRONTEND=noninteractive

[ "$(dpkg-deb -f "$deb" Package)" = industrial-kerf ] || { echo "$deb is not the package industrial-kerf" >&2; exit 1; }
[ "$(dpkg-deb -f "$deb" Architecture)" = amd64 ] || { echo "$deb is not for amd64" >&2; exit 1; }
version="$(dpkg-deb -f "$deb" Version)"
if dpkg-query -W -f='${Status}' industrial-kerf 2> /dev/null | grep -q 'ok installed'; then
    echo "industrial-kerf is installed here already: this check installs it and removes it" >&2
    exit 1
fi

scratch="$(mktemp -d)"
installed=false
cleanup() {
    # A check that failed halfway leaves the machine as it found it.
    if $installed; then apt-get remove -y -q industrial-kerf > /dev/null 2>&1 || true; fi
    rm -rf -- "$scratch"
}
trap cleanup EXIT

# apt-get update succeeds with a warning when it reaches no archive, and then nothing is
# installable: what it said is kept for that case.
apt-get update -q > "$scratch/update.log" 2>&1 || { cat "$scratch/update.log" >&2; echo "apt could not read its archives" >&2; exit 1; }
# As `sudo apt install ./the.deb` would, but for the Recommends: what Kerf needs to start
# has to be in its Depends. apt reads the file as its own user where it can.
cp "$deb" "$scratch/industrial-kerf.deb"
chmod 755 "$scratch"
installed=true
apt-get install -y -q --no-install-recommends "$scratch/industrial-kerf.deb" > "$scratch/apt.log" 2>&1 ||
    { grep -E '^(W|E|Err):' "$scratch/update.log" >&2 || true; cat "$scratch/apt.log" >&2; echo "apt could not install $deb" >&2; exit 1; }
[ "$(dpkg-query -W -f='${Version}' industrial-kerf)" = "$version" ] || { echo "apt installed another version than $version" >&2; exit 1; }
[ -x "$prefix/bin/kerf-qt" ] || { echo "The package installed no $prefix/bin/kerf-qt" >&2; exit 1; }

# With only what apt brought for it: every library a binary names is found.
unfound="$(while IFS= read -r -d '' file; do
    head -c 4 "$file" | grep -q 'ELF' || continue
    env -u LD_LIBRARY_PATH ldd "$file" 2> /dev/null | awk -v file="${file#"$prefix"/}" '/not found/ {print file ": " $1}'
done < <(find "$prefix" -type f -print0) | sort -u)"
if [ -n "$unfound" ]; then
    printf "The package's Depends bring nothing that has:\n%s\n" "$unfound" >&2
    exit 1
fi

# What the checks need, which is not Kerf's to depend on.
apt-get install -y -q --no-install-recommends xvfb xauth binutils procps libglib2.0-bin desktop-file-utils \
    > "$scratch/apt.log" 2>&1 || { cat "$scratch/apt.log" >&2; echo "apt could not install the check's tools" >&2; exit 1; }

# The folder, as the tar.xz's is checked. It has no install-desktop-entry.sh, so that
# check leaves the desktop entry to this one.
"$here/check-kerf-linux.sh" "$prefix"

# As a user who has set nothing, with a home of their own.
home="$scratch/home"
mkdir -p "$home"
as_user() {
    env -u LD_LIBRARY_PATH -u LD_PRELOAD -u QT_PLUGIN_PATH -u QT_QPA_PLATFORM_PLUGIN_PATH \
        -u QML2_IMPORT_PATH -u QML_IMPORT_PATH -u QT_QPA_PLATFORM -u XDG_DATA_HOME -u XDG_CONFIG_HOME \
        -u XDG_DATA_DIRS HOME="$home" XDG_RUNTIME_DIR="$scratch" "$@"
}

# On PATH, each through its link in /usr/bin.
for tool in kerf-qt kerf kerfd kerf-mcp; do
    [ "$(as_user bash -c 'command -v "$1"' which "$tool")" = "/usr/bin/$tool" ] || { echo "$tool is not on PATH as /usr/bin/$tool" >&2; exit 1; }
    [ "$(readlink -f "/usr/bin/$tool")" = "$prefix/bin/$tool" ] || { echo "/usr/bin/$tool is not $prefix/bin/$tool" >&2; exit 1; }
done
as_user kerf --version > /dev/null
# kerfd looks for ffprobe beside itself: beside the file in /opt, not the link in /usr/bin.
"$prefix/bin/ffmpeg" -v error -f lavfi -i testsrc=duration=2:size=320x240:rate=25 -c:v mpeg4 "$scratch/clip.mp4"
printf '%s\n' \
    '{"protocolVersion":1,"id":"import","operation":"import","actor":"ui","idempotencyKey":"check-import","baseRevision":0,"path":"'"$scratch/clip.mp4"'"}' \
    '{"protocolVersion":1,"id":"state","operation":"get_state"}' |
    as_user env PATH=/usr/bin:/bin kerfd --stdio --dir "$scratch/project" > "$scratch/import.json"
if ! grep -q 'rate=25 dur=00:00:02:00 video=320x240' "$scratch/import.json"; then
    echo "kerfd, started as /usr/bin/kerfd, did not probe the clip with the package's ffprobe (it is 320x240, 25 fps, 2 s):" >&2
    grep -o 'rate=[^ ]* dur=[^ ]* video=[0-9x]*' "$scratch/import.json" >&2 || grep '"id":"import"' "$scratch/import.json" >&2
    exit 1
fi
as_user xvfb-run -a -s "-screen 0 1600x1200x24" \
    kerf-qt --settings-dir "$scratch/settings" --screenshot "$scratch/start.png" --quit-after-ms 3000
[ -s "$scratch/start.png" ] || { echo "The editor, started as /usr/bin/kerf-qt, made no screenshot" >&2; exit 1; }

# Among the applications, under its own icon, and opening kerf:// links.
kerf_check_entry "$entry" "$prefix/bin/kerf-qt"
[ -f /usr/share/icons/hicolor/index.theme ] || { echo "No hicolor theme for the icon to be found in: the package depends on hicolor-icon-theme" >&2; exit 1; }
kerf_check_icons /usr/share/icons/hicolor
kerf_takes_links as_user
kerf_follow_link "$prefix/bin/kerf-qt" as_user

# Removed, it leaves nothing: none of its files, its folder, nor its claim on kerf://.
dpkg-query -L industrial-kerf > "$scratch/files.txt"
apt-get remove -y -q industrial-kerf > "$scratch/apt.log" 2>&1 || { cat "$scratch/apt.log" >&2; echo "apt could not remove industrial-kerf" >&2; exit 1; }
installed=false
left="$(while IFS= read -r path; do
    # A folder other packages share stays; its own, and every file, has to be gone.
    if [ -L "$path" ] || [ -f "$path" ]; then echo "$path"; fi
done < "$scratch/files.txt")"
[ ! -e "$prefix" ] || left+=$'\n'"$prefix"
if [ -n "$left" ]; then
    printf 'apt remove left:\n%s\n' "$left" >&2
    exit 1
fi
handlers="$(as_user gio mime x-scheme-handler/kerf 2>&1 || true)"
if grep -q 'industrial-kerf' <<< "$handlers"; then
    printf 'Removed, Kerf is still named for kerf:// links:\n%s\n' "$handlers" >&2
    exit 1
fi
echo "The .deb installed $version with what apt brought for it, ran, took kerf:// links under Kerf's own icon, and came away whole."
