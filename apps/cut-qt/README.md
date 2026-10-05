# Cut Qt editor

This is the native Qt6 desktop shell for Cut. It keeps the project state in
`cutd` through the small C ABI in `bridge.h`; the window does not maintain a
second editable copy of the timeline. The viewer, timeline, inspector, and agent rail are all
backed by the bridge event stream.

Build the Rust bridge first, then configure and build the app from the
repository root:

```sh
cargo build -p cut-qt-bridge
cmake -S apps/cut-qt -B target/qt
cmake --build target/qt
target/qt/cut-qt /path/to/project /path/to/cutd
```

The positional arguments may also be supplied as `CUT_PROJECT_DIR` and
`CUTD_BIN`. For a quick visual smoke run, add
`--screenshot /tmp/cut.png --quit-after-ms 900`.

On macOS the default bridge path is
`target/debug/libcut_qt_bridge.dylib`; on Linux pass
`-DCUT_BRIDGE_LIBRARY=$PWD/target/debug/libcut_qt_bridge.so` when configuring.
The project directory is created by the runtime when it does not already
exist. Importing media, timeline edits, undo/redo, proposal approval, preview
seeking, and export are sent as bounded asynchronous bridge commands. The
timeline draws the `timeline` view of the model that each snapshot carries,
and timeline and inspector edits go to `cutd` as typed operations counted in
frames of the sequence.

The timeline supports click-to-seek, clip selection, drag-to-move, edge
trimming, split-at-playhead, and Delete. Inspector values commit on focus loss.
Tracks follow the classic edit-suite order: picture tracks stack upward from
V1, because a later picture track plays over the ones below it, and sound
tracks run downward from A1. The ruler and track headers stay pinned while the
timeline scrolls. Headers show LOCK, M, or OFF when the project marks a track
locked, muted, or disabled; these are indicators, not toggles. While a clip
moves or trims, a readout shows the new edge timecode and its offset.

Drag the bottom edge of a track header to resize that track. Right-click a
header for the Mini, Small, Medium, Large, and Jumbo heights, for one track or
all of them. View → Fit tracks to panel shares the panel height across every
track; View → Reset track heights restores the defaults. Heights are a view
setting and are not saved in the project.
Choose Edit → Focus region to draw a crop box on the selected video clip and
apply it to a time range. Reopen it on the focused segment to adjust the box.
The picker loads the exact source frame at the playhead. A fixed box ends at a
detected scene cut. Turn on Follow a moving subject only when the subject moves;
it starts from the drawn box, samples twice per second, and stops when the match
is unreliable. Select a new box after a scene cut or tracking loss. Tracking is
limited to two minute ranges; split longer ranges into shorter selections.
For an agent crop proposal, use Adjust box in the proposal review to replace
the suggested rectangle before accepting it as one edit.
The offline preview shows the crop; native playback requires a rendered preview
for this edit.
Drag the panel dividers to resize the workspace, and use View to hide either
panel. The timeline fits the window on its own. All tracks remain vertically
scrollable when their combined height exceeds the timeline panel.
Import media from File. Moving the pointer across the timeline
scrubs the viewer and shows the time beside the pointer. Play and pause are
Space; with the timeline focused, Home and End jump to the start and end, and
Left and Right step one frame. The transport under the viewer has the same
controls between the playhead timecode and the program length. The assistant
fills a narrow column on the right from top to bottom. An empty timeline starts without tracks. Hover over the
empty row in the left track header to reveal a plus; click it to add a video
or audio track. In Chat, Return sends
the message and Shift+Return inserts a new line.

Use the project-format picker in the toolbar to switch between standard
1920×1080 (16:9) and vertical 1080×1920 (9:16). The viewer fits either canvas
inside the available panel without cropping and keeps preview captions inside
the video safe area.

The connection banner reports connection progress and offers Reconnect after
a failure. The status bar keeps the connection state and project revision in
view and shows runtime messages on the left. Editing controls become available only after a successful project
snapshot. Changes made through CLI or MCP are checked every 500 ms while idle;
unchanged snapshots do not redraw the UI or reopen playback.

Proposal review shows semantic clip changes, time ranges, and all supplied
before/after field values in a scrollable view. Stale proposals and proposals
whose field diff was truncated cannot be accepted; ask the agent to restage
or split them into smaller changes.

The Agent panel includes a Chat view and an Agent settings view. Settings
holds two protocol slots: Anthropic Messages and OpenAI-compatible. Each slot
has an API key, Base URL, and model. The OpenAI-compatible slot also has Chat
Completions vs Responses. Gemini and other compatible hosts use that slot and
its Base URL. Chat sends the active Settings connection plus the model combo.
Environment variables remain available as fallbacks.

Settings separates the two MCP directions. “Use MCP tools in Cut chat” makes
the embedded agent call local stdio MCP servers from `~/.cut/mcp.json` or
`.cut/mcp.json`; it can import an existing project or home Cursor config and
save a validated project config. “Use Cut from Cursor / Claude” generates the
opposite-direction `cut-mcp` configuration for an external client. Enabling
one direction does not enable the other.

Run the Qt behavior tests after configuring with `-DCUT_QT_BUILD_TESTS=ON`:

```sh
cmake --build target/qt --parallel 2
ctest --test-dir target/qt --output-on-failure
```

The tests cover disconnected controls, reconnection state, error deduplication,
long timeline fit, scrolling through many tracks, and complete proposal review.
The macOS CI job also runs the real bridge/media workflow in
`scripts/test-cut-qt-bridge.py`.
