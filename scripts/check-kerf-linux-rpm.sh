#!/usr/bin/env bash
# Install Kerf's .rpm as someone would, with dnf, and fail if what it installs does not
# run, is not among the applications, does not open kerf:// links, or does not come away
# whole. It asks of the .rpm what scripts/check-kerf-linux-deb.sh asks of the .deb.
#
# dnf installs it without its Recommends, so its Requires alone have to bring every library
# its binaries name: before anything else is installed, the loader has to find each (ldd).
# In a bare container (fedora), where package.yml runs this, that is what shows the
# Requires to be whole. Then it installs what the checks themselves need and runs
# scripts/check-kerf-linux.sh on /opt/industrial-kerf, the folder as installed; then what an
# .rpm adds to the folder: the four tools on PATH, each working through its link, the
# desktop entry, the icon at each size, GLib taking Kerf for kerf:// links and a link
# followed through it. Last, dnf remove, after which none of its files is left.
#
# It installs and removes packages: run it as root, in a container or on a machine that can
# take that.
#
#   scripts/check-kerf-linux-rpm.sh path/to/Industrial-Kerf-<version>-linux-x64.rpm
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 path/to/the.rpm" >&2; exit 2; }
[ -f "$1" ] || { echo "No .rpm at $1" >&2; exit 1; }
rpm="$(readlink -f "$1")"
[ "$(id -u)" = 0 ] || { echo "Run as root: this installs the package with dnf, and removes it" >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/kerf-linux-desktop.sh"
prefix=/opt/industrial-kerf
entry=/usr/share/applications/industrial-kerf.desktop

[ "$(rpm -qp --queryformat '%{NAME}' "$rpm" 2> /dev/null)" = industrial-kerf ] || { echo "$rpm is not the package industrial-kerf" >&2; exit 1; }
[ "$(rpm -qp --queryformat '%{ARCH}' "$rpm" 2> /dev/null)" = x86_64 ] || { echo "$rpm is not for x86_64" >&2; exit 1; }
version="$(rpm -qp --queryformat '%{VERSION}' "$rpm" 2> /dev/null)"
if rpm -q industrial-kerf > /dev/null 2>&1; then
    echo "industrial-kerf is installed here already: this check installs it and removes it" >&2
    exit 1
fi

scratch="$(mktemp -d)"
installed=false
cleanup() {
    # A check that failed halfway leaves the machine as it found it.
    if $installed; then dnf remove -y -q industrial-kerf > /dev/null 2>&1 || true; fi
    rm -rf -- "$scratch"
}
trap cleanup EXIT

# As `sudo dnf install ./the.rpm` would, but for the Recommends: what Kerf needs to start
# has to be in its Requires. The package is not signed, as none of Kerf's is.
cp "$rpm" "$scratch/industrial-kerf.rpm"
installed=true
dnf install -y -q --setopt=install_weak_deps=False --nogpgcheck "$scratch/industrial-kerf.rpm" > "$scratch/dnf.log" 2>&1 ||
    { cat "$scratch/dnf.log" >&2; echo "dnf could not install $rpm" >&2; exit 1; }
[ "$(rpm -q --queryformat '%{VERSION}' industrial-kerf)" = "$version" ] || { echo "dnf installed another version than $version" >&2; exit 1; }
[ -x "$prefix/bin/kerf-qt" ] || { echo "The package installed no $prefix/bin/kerf-qt" >&2; exit 1; }

# With only what dnf brought for it: every library a binary names is found.
unfound="$(while IFS= read -r -d '' file; do
    head -c 4 "$file" | grep -q 'ELF' || continue
    env -u LD_LIBRARY_PATH ldd "$file" 2> /dev/null | awk -v file="${file#"$prefix"/}" '/not found/ {print file ": " $1}'
done < <(find "$prefix" -type f -print0) | sort -u)"
if [ -n "$unfound" ]; then
    printf "The package's Requires bring nothing that has:\n%s\n" "$unfound" >&2
    exit 1
fi

# What the checks need, which is not Kerf's to depend on.
dnf install -y -q --setopt=install_weak_deps=False xorg-x11-server-Xvfb xorg-x11-xauth binutils procps-ng glib2 \
    desktop-file-utils which findutils diffutils > "$scratch/dnf.log" 2>&1 ||
    { cat "$scratch/dnf.log" >&2; echo "dnf could not install the check's tools" >&2; exit 1; }

# The folder, as the tar.xz's is checked.
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
    [ "$(readlink -f "$(as_user bash -c 'command -v "$1"' which "$tool")")" = "$prefix/bin/$tool" ] ||
        { echo "$tool on PATH is not $prefix/bin/$tool" >&2; exit 1; }
done
as_user kerf --version > /dev/null
# kerfd looks for ffprobe beside itself: beside the file in /opt, not the link in /usr/bin.
"$prefix/bin/ffmpeg" -v error -f lavfi -i testsrc=duration=2:size=320x240:rate=25 -c:v mpeg4 "$scratch/clip.mp4"
printf '%s\n' \
    '{"protocolVersion":1,"id":"import","operation":"import","actor":"ui","idempotencyKey":"check-import","baseRevision":0,"path":"'"$scratch/clip.mp4"'"}' \
    '{"protocolVersion":1,"id":"state","operation":"get_state"}' |
    as_user env PATH=/usr/bin:/bin kerfd --stdio --dir "$scratch/project" > "$scratch/import.json"
if ! grep -q 'rate=25 dur=00:00:02:00 video=320x240' "$scratch/import.json"; then
    echo "kerfd, started through /usr/bin, did not probe the clip with the package's ffprobe (it is 320x240, 25 fps, 2 s):" >&2
    grep -o 'rate=[^ ]* dur=[^ ]* video=[0-9x]*' "$scratch/import.json" >&2 || grep '"id":"import"' "$scratch/import.json" >&2
    exit 1
fi
as_user xvfb-run -a -s "-screen 0 1600x1200x24" \
    kerf-qt --settings-dir "$scratch/settings" --screenshot "$scratch/start.png" --quit-after-ms 3000
[ -s "$scratch/start.png" ] || { echo "The editor, started through /usr/bin, made no screenshot" >&2; exit 1; }

# Among the applications, under its own icon, and opening kerf:// links.
kerf_check_entry "$entry" "$prefix/bin/kerf-qt"
[ -f /usr/share/icons/hicolor/index.theme ] || { echo "No hicolor theme for the icon to be found in: the package requires hicolor-icon-theme" >&2; exit 1; }
kerf_check_icons /usr/share/icons/hicolor
kerf_takes_links as_user
kerf_follow_link "$prefix/bin/kerf-qt" as_user

# Removed, it leaves nothing: none of its files, its folder, nor its claim on kerf://.
rpm -ql industrial-kerf > "$scratch/files.txt"
dnf remove -y -q industrial-kerf > "$scratch/dnf.log" 2>&1 || { cat "$scratch/dnf.log" >&2; echo "dnf could not remove industrial-kerf" >&2; exit 1; }
installed=false
left="$(while IFS= read -r path; do
    if [ -L "$path" ] || [ -f "$path" ]; then echo "$path"; fi
done < "$scratch/files.txt")"
[ ! -e "$prefix" ] || left+=$'\n'"$prefix"
if [ -n "$left" ]; then
    printf 'dnf remove left:\n%s\n' "$left" >&2
    exit 1
fi
handlers="$(as_user gio mime x-scheme-handler/kerf 2>&1 || true)"
if grep -q 'industrial-kerf' <<< "$handlers"; then
    printf 'Removed, Kerf is still named for kerf:// links:\n%s\n' "$handlers" >&2
    exit 1
fi
echo "The .rpm installed $version with what dnf brought for it, ran, took kerf:// links under Kerf's own icon, and came away whole."
