# What scripts/check-kerf-linux.sh and scripts/check-kerf-linux-deb.sh both ask of Kerf's
# desktop entry, whether install-desktop-entry.sh wrote it for one user or the .deb
# installed it for all: sourced by each, and run by neither.

# The sizes of Kerf's icon a Linux package installs into the hicolor theme.
kerf_icon_sizes="16 24 32 48 64 128 256 512"

# kerf_check_entry <entry> <editor>: the entry runs that editor with the link, takes
# kerf:// links and wears Kerf's own icon.
kerf_check_entry() {
    local entry="$1" editor="$2" icon
    [ -f "$entry" ] || { echo "No desktop entry at $entry" >&2; return 1; }
    grep -qxF "Exec=\"$editor\" %u" "$entry" ||
        { echo "The desktop entry does not run $editor with the link:" >&2; cat "$entry" >&2; return 1; }
    grep -qxF 'MimeType=x-scheme-handler/kerf;' "$entry" || { echo "The desktop entry takes no kerf:// links" >&2; return 1; }
    icon="$(sed -n 's/^Icon=//p' "$entry")"
    [ "$icon" = industrial-kerf ] || { echo "The desktop entry's icon is '$icon', not Kerf's own" >&2; return 1; }
    if command -v desktop-file-validate > /dev/null; then desktop-file-validate "$entry"; fi
}

# kerf_check_icons <hicolor> [<icons>]: the entry's icon, looked up as a desktop does, by
# name, in that part of the hicolor theme, at the size asked for. Each has to be a PNG of
# that size, in a folder the system's theme lists, since no other would be looked in; and
# the same file as <icons>/industrial-kerf-<size>.png, where a folder of them is given.
kerf_check_icons() {
    local hicolor="$1" icons="${2:-}" size installed width height w1 w2 w3 w4 h1 h2 h3 h4
    for size in $kerf_icon_sizes; do
        installed="$hicolor/${size}x${size}/apps/industrial-kerf.png"
        [ -s "$installed" ] || { echo "No $size pixel icon for the desktop entry at $installed" >&2; return 1; }
        [ "$(head -c 8 "$installed" | od -An -tx1 | tr -d ' \n')" = 89504e470d0a1a0a ] ||
            { echo "$installed is not a PNG" >&2; return 1; }
        # A PNG's width and height: eight bytes, big-endian, from the sixteenth.
        read -r w1 w2 w3 w4 h1 h2 h3 h4 < <(od -An -tu1 -j16 -N8 "$installed")
        width=$((w1 * 16777216 + w2 * 65536 + w3 * 256 + w4))
        height=$((h1 * 16777216 + h2 * 65536 + h3 * 256 + h4))
        [ "$width" = "$size" ] && [ "$height" = "$size" ] ||
            { echo "$installed is ${width}x$height, not ${size}x$size" >&2; return 1; }
        if [ -n "$icons" ]; then
            cmp -s "$installed" "$icons/industrial-kerf-$size.png" ||
                { echo "$installed is not $icons/industrial-kerf-$size.png" >&2; return 1; }
        fi
        if [ -f /usr/share/icons/hicolor/index.theme ]; then
            grep -qE "^Directories=(.*,)?${size}x${size}/apps(,|\$)" /usr/share/icons/hicolor/index.theme ||
                { echo "The hicolor theme has no ${size}x${size}/apps for the icon to be found in" >&2; return 1; }
        fi
    done
}

# kerf_takes_links <command...>: GLib, through which GNOME and most desktops open a link,
# takes Kerf for kerf:// links. The command runs gio as the user in question.
kerf_takes_links() {
    local handlers
    # Read whole first: grep -q stops at its match, and with pipefail gio's broken pipe
    # would fail the check.
    handlers="$("$@" gio mime x-scheme-handler/kerf)"
    grep -q '^Default application .*: industrial-kerf\.desktop$' <<< "$handlers" ||
        { printf 'GLib does not take Kerf for kerf:// links:\n%s\n' "$handlers" >&2; return 1; }
}

# kerf_follow_link <editor> <command...>: a kerf:// link followed through GLib starts that
# editor with it. As a click on a link: gio starts the entry's command and returns, and the
# editor has to be running with the link.
kerf_follow_link() {
    local editor="$1" link='kerf://kerf-linux-check/rev1@00:00:01:00' follow
    shift
    follow='
        gio open "$1"
        for _ in $(seq 1 40); do
            for pid in $(pgrep -f -- "$1"); do
                if [ "$(readlink -f "/proc/$pid/exe")" = "$2" ]; then kill "$pid"; exit 0; fi
            done
            sleep 0.5
        done
        exit 1'
    "$@" xvfb-run -a -s "-screen 0 1280x800x24" bash -c "$follow" follow "$link" "$editor" ||
        { echo "Following $link through GLib started no $editor with it" >&2; return 1; }
}
