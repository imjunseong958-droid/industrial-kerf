#!/usr/bin/env bash
# Start the editor, close its window as a window manager does for the title bar's button,
# and fail unless the editor then quits: exit 0, with no panic in what it printed. A run
# that quits by --quit-after-ms never goes this way, where GPUI's X11 client is still
# closing the window when the editor's own code runs (a quit made from there panicked,
# exit 101).
#
# The ask is the ICCCM's: a WM_PROTOCOLS client message that carries WM_DELETE_WINDOW,
# sent to each window of the editor's that is shown and takes it, by a few lines of C
# built here against libxcb, which GPUI's build needs already. Nothing more is installed
# for it: Ubuntu 24.04's xdotool has no windowquit and its windowclose destroys the
# window unasked, and wmctrl -c needs a window manager.
#
# The editor is asked every 0.2 s, from when its window is shown until it ends, not once:
# an ask sent while it is starting waits for it, and by hand, with no window manager, as
# many as 22 went by before one took. It has $limit seconds from its start to end. If it
# has not by then, it is stopped and this fails: the close never took. It is started with
# a --quit-after-ms well past that, so that it ends whatever becomes of this script and
# opens no project but the one given here; a run that lasted until then is out of time
# long before, and fails.
#
# On an X server, with the editor and what it is to open:
#
#   xvfb-run -a -s "-screen 0 1600x1000x24 +extension Composite" \
#       scripts/smoke-editor-close.sh target/debug/kerf-editor [project]
set -euo pipefail

[ $# -ge 1 ] || { echo "usage: $0 path/to/kerf-editor [project]" >&2; exit 2; }
[ -n "${DISPLAY:-}" ] || { echo "No DISPLAY: run this under xvfb-run" >&2; exit 2; }

limit=30
scratch="$(mktemp -d)"
editor=""
trap '[ -z "$editor" ] || kill "$editor" 2> /dev/null || true; rm -rf -- "$scratch"' EXIT

# ask-to-close <pid>: exit 0 if a window of that process was asked, 3 if it shows none.
cc -o "$scratch/ask-to-close" -x c - -lxcb << 'C'
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <xcb/xcb.h>

static xcb_connection_t *c;
static xcb_atom_t wm_protocols, wm_delete_window, net_wm_pid;

static xcb_atom_t atom(const char *name) {
    xcb_intern_atom_reply_t *r =
        xcb_intern_atom_reply(c, xcb_intern_atom(c, 0, strlen(name), name), NULL);
    xcb_atom_t a = r ? r->atom : XCB_ATOM_NONE;
    free(r);
    return a;
}

/* Whether the window's property, a list of 32-bit values, has this one. */
static int has(xcb_window_t window, xcb_atom_t property, uint32_t value) {
    xcb_get_property_reply_t *r = xcb_get_property_reply(
        c, xcb_get_property(c, 0, window, property, XCB_GET_PROPERTY_TYPE_ANY, 0, 64), NULL);
    int found = 0;
    if (r && r->format == 32) {
        uint32_t *values = xcb_get_property_value(r);
        for (int i = 0; i < xcb_get_property_value_length(r) / 4; i++)
            found |= values[i] == value;
    }
    free(r);
    return found;
}

/* Whether the window is on the screen: a window manager has no button for one that is not. */
static int shown(xcb_window_t window) {
    xcb_get_window_attributes_reply_t *r =
        xcb_get_window_attributes_reply(c, xcb_get_window_attributes(c, window), NULL);
    int viewable = r && r->map_state == XCB_MAP_STATE_VIEWABLE;
    free(r);
    return viewable;
}

/* Asks the window, if it is the process's, takes the ask and is shown; then the windows in it. */
static int ask(xcb_window_t window, uint32_t pid) {
    int asked = 0;
    if (has(window, net_wm_pid, pid) && has(window, wm_protocols, wm_delete_window) &&
        shown(window)) {
        xcb_client_message_event_t e = {0};
        e.response_type = XCB_CLIENT_MESSAGE;
        e.format = 32;
        e.window = window;
        e.type = wm_protocols;
        e.data.data32[0] = wm_delete_window;
        e.data.data32[1] = XCB_CURRENT_TIME;
        xcb_send_event(c, 0, window, XCB_EVENT_MASK_NO_EVENT, (const char *)&e);
        asked = 1;
    }
    xcb_query_tree_reply_t *tree = xcb_query_tree_reply(c, xcb_query_tree(c, window), NULL);
    if (tree) {
        xcb_window_t *inside = xcb_query_tree_children(tree);
        for (int i = 0; i < xcb_query_tree_children_length(tree); i++)
            asked += ask(inside[i], pid);
        free(tree);
    }
    return asked;
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    c = xcb_connect(NULL, NULL);
    if (xcb_connection_has_error(c)) return 1;
    wm_protocols = atom("WM_PROTOCOLS");
    wm_delete_window = atom("WM_DELETE_WINDOW");
    net_wm_pid = atom("_NET_WM_PID");
    uint32_t pid = (uint32_t)strtoul(argv[1], NULL, 10);
    int asked = 0;
    xcb_screen_iterator_t screen = xcb_setup_roots_iterator(xcb_get_setup(c));
    for (; screen.rem; xcb_screen_next(&screen))
        asked += ask(screen.data->root, pid);
    xcb_flush(c);
    xcb_disconnect(c);
    return asked ? 0 : 3;
}
C

# On X11 even where the caller's desktop is Wayland.
env -u WAYLAND_DISPLAY "$@" --quit-after-ms $(((limit + 90) * 1000)) > "$scratch/editor.log" 2>&1 &
editor=$!
SECONDS=0
asks=0
while kill -0 "$editor" 2> /dev/null && [ "$SECONDS" -lt "$limit" ]; do
    if "$scratch/ask-to-close" "$editor"; then asks=$((asks + 1)); fi
    sleep 0.2
done
# Out of time, whether it is still running or ended only now.
if [ "$SECONDS" -ge "$limit" ]; then
    cat "$scratch/editor.log"
    if [ "$asks" -eq 0 ]; then
        echo "The editor showed no window to close in the $limit s after its start" >&2
    else
        echo "The editor had not ended $limit s after its start (asks sent to its window: $asks)" >&2
    fi
    exit 1
fi
status=0
wait "$editor" || status=$?
editor=""
cat "$scratch/editor.log"
if [ "$asks" -eq 0 ]; then
    echo "The editor ended (exit $status) before it had a window to close" >&2
    exit 1
fi
if [ "$status" -ne 0 ]; then
    echo "The editor's window was asked to close and the editor ended with exit $status, not 0" >&2
    exit 1
fi
if grep -q 'panicked at' "$scratch/editor.log"; then
    echo "The editor's window was asked to close and the editor panicked on the way to its exit 0" >&2
    exit 1
fi
echo "The editor quit, exit 0, $SECONDS s after its start (asks sent to its window: $asks)"
