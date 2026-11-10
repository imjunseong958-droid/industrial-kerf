#!/bin/sh
# Adds Industrial Kerf to the desktop's applications and makes it the program that opens
# kerf:// review links, for this user alone and with no administrator: it writes
# $XDG_DATA_HOME/applications/industrial-kerf.desktop, which runs this folder's bin/kerf-qt,
# names it the default for x-scheme-handler/kerf in $XDG_CONFIG_HOME/mimeapps.list, as
# xdg-mime would, and copies this folder's icons/ to
# $XDG_DATA_HOME/icons/hicolor/<size>x<size>/apps/industrial-kerf.png, where the entry's
# Icon=industrial-kerf finds them. Run it again after moving the folder; --remove takes all
# three away.
#
#   ./install-desktop-entry.sh [--remove]
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
applications="$data/applications"
entry="$applications/industrial-kerf.desktop"
mimeapps="$config/mimeapps.list"
scheme="x-scheme-handler/kerf"
# The user's part of the hicolor theme, which every icon theme falls back to.
hicolor="$data/icons/hicolor"
icon_sizes="16 24 32 48 64 128 256 512"

# mimeapps.list with the default for kerf:// set to $1, or taken out when $1 is empty; the
# rest of the file as it was. Only a default that is Kerf's is taken out.
set_default() {
    mkdir -p "$config"
    [ -f "$mimeapps" ] || : > "$mimeapps"
    awk -v key="$scheme" -v value="$1" '
        function put() { if (value != "" && !done) { print key "=" value; done = 1 } }
        /^\[/ {
            if (section == "[Default Applications]") put()
            section = $0
            print
            next
        }
        section == "[Default Applications]" && index($0, key "=") == 1 {
            if (value != "") put()
            else if ($0 != key "=industrial-kerf.desktop") print
            next
        }
        { print }
        END {
            if (section == "[Default Applications]") put()
            if (value != "" && !done) {
                if (NR > 0) print ""
                print "[Default Applications]"
                print key "=" value
            }
        }
    ' "$mimeapps" > "$mimeapps.kerf-new"
    mv "$mimeapps.kerf-new" "$mimeapps"
}

refresh() {
    if command -v update-desktop-database > /dev/null 2>&1; then
        update-desktop-database "$applications" 2> /dev/null || true
    fi
    # A desktop that keeps a cache of the theme reads it again when the folder is newer.
    if [ -d "$hicolor" ]; then
        touch "$hicolor"
        if command -v gtk-update-icon-cache > /dev/null 2>&1; then
            gtk-update-icon-cache -q -t -f "$hicolor" 2> /dev/null || true
        fi
    fi
}

# Kerf's icons out of the theme, and the folders that held nothing else.
remove_icons() {
    for size in $icon_sizes; do
        rm -f "$hicolor/${size}x${size}/apps/industrial-kerf.png"
        rmdir "$hicolor/${size}x${size}/apps" "$hicolor/${size}x${size}" 2> /dev/null || true
    done
    # The cache this script wrote names nothing once the icons are gone.
    if [ -d "$hicolor" ] && [ -z "$(find "$hicolor" -mindepth 1 ! -name icon-theme.cache | head -n 1)" ]; then
        rm -f "$hicolor/icon-theme.cache"
    fi
    rmdir "$hicolor" "$data/icons" 2> /dev/null || true
}

if [ "${1:-}" = "--remove" ]; then
    rm -f "$entry"
    [ -f "$mimeapps" ] && set_default ""
    remove_icons
    refresh
    echo "Industrial Kerf is no longer among this user's applications, nor opens kerf:// links."
    exit 0
fi
if [ -n "${1:-}" ]; then
    echo "usage: $0 [--remove]" >&2
    exit 2
fi

editor="$here/bin/kerf-qt"
[ -x "$editor" ] || { echo "No editor at $editor: run this from the unpacked folder" >&2; exit 1; }
# The desktop entry quotes the path; these would need escapes desktops do not all read alike.
case "$editor" in
    *[\"\`\$\\%]* | *"
"*)
        echo "Move the folder to a path without \" \` \$ \\ % or a line break in it, then run this again" >&2
        exit 1
        ;;
esac

for size in $icon_sizes; do
    [ -f "$here/icons/industrial-kerf-$size.png" ] ||
        { echo "No icons/industrial-kerf-$size.png in $here: run this from the unpacked folder" >&2; exit 1; }
done
for size in $icon_sizes; do
    mkdir -p "$hicolor/${size}x${size}/apps"
    cp "$here/icons/industrial-kerf-$size.png" "$hicolor/${size}x${size}/apps/industrial-kerf.png.kerf-new"
    mv "$hicolor/${size}x${size}/apps/industrial-kerf.png.kerf-new" "$hicolor/${size}x${size}/apps/industrial-kerf.png"
done

mkdir -p "$applications"
cat > "$entry.kerf-new" <<EOF
[Desktop Entry]
Type=Application
Version=1.5
Name=Industrial Kerf
GenericName=Video Editor
Comment=Edit video, with an agent editing beside you
Exec="$editor" %u
TryExec=$editor
Icon=industrial-kerf
Terminal=false
Categories=AudioVideo;Video;AudioVideoEditing;
MimeType=$scheme;
StartupWMClass=kerf-qt
EOF
mv "$entry.kerf-new" "$entry"
set_default "industrial-kerf.desktop"
refresh
echo "Industrial Kerf is among this user's applications, and opens kerf:// links ($entry)."
