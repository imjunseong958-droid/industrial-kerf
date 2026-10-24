# Industrial Kerf Qt editor

This is the native Qt6 desktop shell for Kerf. It keeps the project state in
`kerfd` through the small C ABI in `bridge.h`; the window does not maintain a
second editable copy of the timeline. Its docked panels — Project,
Source, Program, Timeline, Effect Controls, and Kerf Assistant — are all
backed by the bridge event stream.

Build the Rust bridge first, then configure and build the app from the
repository root:

```sh
cargo build -p kerf-qt-bridge
cmake -S apps/kerf-qt -B target/qt
cmake --build target/qt
target/qt/kerf-qt /path/to/project /path/to/kerfd
```

The positional arguments may also be supplied as `KERF_PROJECT_DIR` and
`KERFD_BIN`. For a quick visual smoke run, add
`--screenshot /tmp/kerf.png --quit-after-ms 900`.

Sources live in the `kerf` namespace, one class per header/source pair:
`main_window` (the window, which builds and connects the panels),
`workspace` (the docked panels and workspaces), `project_panel` (the bin),
`source_monitor_panel`, `effect_controls_panel`,
`timeline_widget`, `viewer_widget` (the viewer, its mask drawing, and the
focus-region picker),
`command_registry` (the editor's side of the command registry), and
`media_artwork` (background thumbnails and waveforms). `editor_common`
holds shared constants and helpers, `editor_style.h` the stylesheet, and
`main.cpp` only `main()`. `editor_test.cpp` builds into `kerf-qt-tests`
instead of `main.cpp`.

Menus, shortcuts, the toolbar, the transport buttons, and timeline gestures
run commands from the registry in `crates/kerf-commands`. Each
command is one `QAction` whose label, shortcuts, and enabled state come
from the registry; an unavailable command says why in its tooltip and the
status bar. Edit → Keyboard Shortcuts switches between the Kerf, Premiere
Pro, and Final Cut Pro keymaps, and the choice is remembered.

On macOS the default bridge path is
`target/debug/libkerf_qt_bridge.dylib`; on Linux pass
`-DKERF_BRIDGE_LIBRARY=$PWD/target/debug/libkerf_qt_bridge.so` when configuring.
The project directory is created by the runtime when it does not already
exist. Importing media, timeline edits, undo/redo, proposal approval, preview
seeking, and export are sent as bounded asynchronous bridge commands. The
timeline draws the `timeline` view of the model that each snapshot carries,
and timeline and Effect Controls edits go to `kerfd` as typed operations counted in
frames of the sequence.

The timeline supports click-to-seek, clip selection, drag-to-move, edge
trimming, split-at-playhead, and Delete. The tool palette beside the track
headers picks what a click or drag on a clip does, with the usual keys while
the timeline has focus: Selection (V) moves and trims, Razor (C) splits
where you click, Ripple Edit (B) trims and closes or opens the gap, Rolling
Edit (N) moves a cut between two clips, Slip (Y) shows other media in the
same span, and Slide (U) moves a clip between its neighbours. With Snap on
(Sequence → Snap, Shift+S), dragged clips and edges snap to clip edges, the
playhead, and the start. A drop lands where the drag's preview showed it.

The Project panel shows the project's media as a bin; double-click an item
to open it in the Source monitor. There, scrub the bar under the picture or
step with Left and Right, and mark the range with I and O (Clear In and Out
is Ctrl+Shift+X). Insert (,) and Overwrite (.) place the marked range at
the playhead on the first unlocked picture and sound tracks the media needs;
a picture and its sound are placed as linked clips, and Insert ripples the
rest of the sequence. Effect Controls edits a clip's timing,
the catalog properties of its track type, and a generator's parameters, from
the definitions each snapshot carries; animated properties are read-only
there. Its values commit on focus loss.
Tracks follow the classic edit-suite order: picture tracks stack upward from
V1, because a later picture track plays over the ones below it, and sound
tracks run downward from A1. The ruler and track headers stay pinned while the
timeline scrolls. Each header has two toggles after the track number: the
eye turns a picture track's output on and off (M mutes a sound track), and
the padlock locks the track against edits. While a clip
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
Panels dock with the Qt Advanced Docking System (LGPL-2.1, fetched at a
pinned release when configuring and linked as a shared library). Drag a
panel's tab to dock it elsewhere, tab it with another panel, or float it;
drag the dividers to resize. Window lists the panels to show or hide and
the workspaces: Editing is built in, and Save as New Workspace adds your
own. A workspace keeps the changes made to it, across launches, until
Reset to Saved Layout (Alt+Shift+0) returns it to its saved layout; Save
Changes to This Workspace makes the current arrangement that layout. The
timeline fits the window on its own. All tracks remain vertically
scrollable when their combined height exceeds the timeline panel.
Import media from File. Moving the pointer across the timeline
scrubs the viewer and shows the time beside the pointer. Play and pause are
Space; with the timeline focused, Home and End jump to the start and end,
Left and Right step one frame, S splits the selected clip at the playhead,
and Delete and Shift+Delete delete or ripple-delete it (Kerf keymap). The transport under the viewer has the same
controls between the playhead timecode and the program length. The Editing
workspace puts the Source monitor (with Effect Controls behind it) beside
the Program monitor, over the Project bin (with Effects behind it) and the
Timeline. An empty
timeline starts without tracks. Hover over the
empty row in the left track header to reveal a plus; click it to add a video
or audio track. Double-click a clip to open its media in the Source monitor
at the clip's in point.

## Effects

The Effects panel (Window → Effects) shows the catalog's picture effects as
cards, narrowed by category and by words ("glow", "zoom", "retro"), each
card the selected media clip's picture at the playhead through that effect.
Double-click a card, or press Return, to add the effect to the selected clip;
the edit is an ordinary one, undone like any other. Effect Controls lists the
clip's effects in order: the checkbox turns one off, ✕ removes it, and its
parameters edit like the clip's properties, resetting to the default when
set back to it.

## Masks

The Rectangle, Ellipse, and Pen mask tools sit in the Program panel's title
bar and beside Masks in Effect Controls; they draw on the selected picture
clip. Drag out a rectangle or an ellipse on the Program monitor; with the
pen, click each corner and click the first one again to close the outline,
or press Return to leave it open as a path (Escape drops it). A new mask is
named `k1`, `k2`, … and selected. While the clip shows masks, the monitor
holds its picture: drag a handle to move a corner or point, drag inside a
mask to move it, and press Delete to remove the selected one. Masks are
drawn where the clip's picture lands, after its crop, scale, rotation, and
position; a mask with mode `none` is dashed, a path that effects such as
Stroke and Reshape read. Effect Controls lists the masks with their mode,
inversion, feather, opacity, and expansion, and an effect's mask parameters
pick from them. Each change is one edit (`mask-add`, `mask-set`, or
`mask-remove`), undone like any other; a masked clip previews through the
compositor.

## Asking Kerf

File → Export Editing Preset… saves the accepted edit as a portable
`.kerfpreset` containing media slots and Kerf editing code. File → Import
Editing Preset… opens a native dialog to assign media from the Project bin,
set source starts, change title/caption text, and inspect the code. Create
proposal opens the normal timeline review before acceptance. See the
[preset contract](../../docs/presets.md).

Ask Kerf (Clip → Ask Kerf…, ⌘J), Option-double-click a clip, or right-click
it and choose Ask Kerf… A prompt opens over the selected clip — or over the
timeline at the playhead when nothing is selected — naming what it is about.
Return sends; Shift+Return adds a line. With a clip, choose This clip (Kerf
changes only that clip, and the clips around it follow by ripple) or Whole
sequence (centred on the clip). The request reaches the agent with that
focus, in frames of the sequence.

While Kerf works, a bar over the timeline says what it is working on and
offers Stop. Its proposal then shows on the timeline before anything
changes: clips it adds or changes are outlined where they would land, and
clips it removes are struck through. The bar offers Accept (⌘Return),
Reject (Escape with the timeline focused), and Details, which opens the
Kerf Assistant panel with the conversation and the proposed script. The Kerf
Assistant panel starts closed; open it from Window to keep chatting there,
where Return sends and Shift+Return inserts a new line. Kerf ends each turn
with up to three next steps, shown as buttons on the bar and over the
composer ("Stronger glow", "Wipe to the next shot"); pressing one asks for it
about the same selection.

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

Agent settings are in Settings (Edit → Settings…, ⌘, ; on macOS in the
application menu), also reachable from the Kerf Assistant panel. Settings → AI
chooses Kerf Cloud, the hosted model (the default unless a key of your own
is already saved; see
[docs/cloud.md](../../docs/cloud.md)), or My own API key. Kerf Cloud shows the
account, plan, and requests left, with Sign in (a token, for now) and Sign
out; Ask Kerf stays unavailable, with the reason, until the account can make
requests. The status bar shows the connection in use. Advanced
holds two protocol slots for your own key: Anthropic Messages and OpenAI-compatible. Each slot
has an API key, Base URL, and model. The OpenAI-compatible slot also has Chat
Completions vs Responses. Gemini and other compatible hosts use that slot and
its Base URL. Chat sends the active Settings connection plus the model combo.
Environment variables remain available as fallbacks.

Settings separates the two MCP directions. “Use MCP tools in Kerf chat” makes
the embedded agent call local stdio MCP servers from `~/.kerf/mcp.json` or
`.kerf/mcp.json`; it can import an existing project or home Cursor config and
save a validated project config. “Use Kerf from Cursor / Claude” generates the
opposite-direction `kerf-mcp` configuration for an external client. Enabling
one direction does not enable the other.

Run the Qt behavior tests after configuring with `-DKERF_QT_BUILD_TESTS=ON`:

```sh
cmake --build target/qt --parallel 2
ctest --test-dir target/qt --output-on-failure
```

The tests cover disconnected controls, reconnection state, error deduplication,
long timeline fit, scrolling through many tracks, and complete proposal review.
The macOS CI job also runs the real bridge/media workflow in
`scripts/test-kerf-qt-bridge.py`.
