# Industrial Kerf Qt editor

This is the native Qt6 desktop shell for Kerf. It keeps the project state in
`kerfd` through the small C ABI in `bridge.h`; the window does not maintain a
second editable copy of the timeline. Work starts as a conversation: the folders
at the left and Kerf beside them. The picture and the timeline open in a panel at
the right once there is something to show; Media, Effects and titles, the
Inspector, Finish and Deliver open on demand. All of them are backed by the
bridge event stream.

Build the Rust bridge first, then configure and build the app from the
repository root:

```sh
cargo build -p kerf-qt-bridge
cmake -S apps/kerf-qt -B target/qt
cmake --build target/qt
target/qt/kerf-qt /path/to/project /path/to/kerfd
```

On macOS the build is an application bundle, `target/qt/kerf-qt.app`
(identifier `com.industrial-kerf.editor`); run
`target/qt/kerf-qt.app/Contents/MacOS/kerf-qt` where these examples run
`target/qt/kerf-qt`.

The positional arguments may also be supplied as `KERF_PROJECT_DIR` and
`KERFD_BIN`; without either, the editor runs the `kerfd` beside itself (the
installed app carries one), else the one on the `PATH`. Without a project, Kerf opens the start window instead
(`KERFD_BIN=/path/to/kerfd target/qt/kerf-qt`). For a quick visual smoke
run, add `--screenshot /tmp/kerf.png --quit-after-ms 900`; without a project
it captures the start window. `--window-size 1600x1000` sizes the window,
`--workspace Finish` opens it with that tray open (Finish, Deliver), and `--settings-dir DIR`
keeps the run's settings in DIR (INI files) instead of the user's.

The editor and the launcher are dark only, and fill the screen edge to edge
as Final Cut Pro does. There is one graphite ramp (#050505 behind everything,
#0e0e0e around the region picker, #121212 wells, #171717 panels,
#1c1c1c cards and menus, #262626 raised controls, #303030 chosen ones) with
ink #ededed / #a1a1a1 / #8a8a8a, and one blue accent (#3366e6) for Kerf, focus,
proposals and the default action; the playhead is its own red. Colours, type
(Inter and JetBrains Mono at 400 and 500, with tabular figures), radii,
spacing, control heights and motion (120/160/200/220 ms) come only from
`design_tokens.h` and the kit in `ui/components.h` and `ui/icons.h` (icons
are the mockup's SVG path data drawn as `QPainterPath`, without QtSvg).
The fonts are compiled into the app from `resources/fonts/` (Inter 4.1 and
JetBrains Mono 2.304, both SIL OFL 1.1), so an install needs none on the
system; their licences ship beside it, in the bundle's `Resources/fonts`.
Menus, tooltips and popovers are flat cards with the `--pop` shadow
(0 8 24, 45% black) round them; tooltips name the control and its shortcut.
There is no light appearance and no `KERF_COLOR_SCHEME`. On macOS the
editor's toolbar is its title bar: the window runs up under a transparent
title bar, the window buttons sit at the toolbar's leading end, and the
project's name is centred on the window with its state under it
("Edited 2m ago · rev 22").

Sources live in the `kerf` namespace. `main_window.cpp` builds the window;
its members are split by region into `main_window_<region>.cpp` (stage,
review, inspector, timeline, bin, source, toolbar, status), and each region
keeps its state in a wiring class under `ui/wiring/` that talks to the others
only through `ui/editor_bus.h` (the bridge's events, the snapshot, the
playhead, the tray open and cross-region requests). `ui/` holds the shared
parts: the kit (`components`, `icons`, `scope_math`), `stage_shell` (the
window's content and its geometry), the review model and the comments panel's
rows and proposal cards, the agent HUD, the Inspector column, the Finish and
Deliver trays, and `review_link` (kerf:// links). `viewer_widget` is the
picture, `editor_theme`/`editor_style.h` the colour roles and the
style sheet generated from them, and `project_launcher` the Choose a Project
window. `main.cpp` holds only `main()`; the tests under `tests/` (one class
per region, run together by `tests/test_main.cpp`) build into
`kerf-qt-tests` instead.

Menus, shortcuts, the toolbar, the player's controls, and timeline gestures
run commands from the registry in `crates/kerf-commands`. Each
command is one `QAction` whose label, shortcuts, and enabled state come
from the registry; an unavailable command says why in its tooltip and the
toolbar's subtitle. Edit → Keyboard Shortcuts switches between the Kerf, Premiere
Pro, and Final Cut Pro keymaps, and the choice is remembered.

On macOS the default bridge path is
`target/debug/libkerf_qt_bridge.dylib`; on Linux pass
`-DKERF_BRIDGE_LIBRARY=$PWD/target/debug/libkerf_qt_bridge.so` when configuring.
The project directory is created by the runtime when it does not already
exist. Importing media, timeline edits, undo/redo, proposal approval, preview
seeking, and export are sent as bounded asynchronous bridge commands. The
timeline draws the `timeline` view of the model that each snapshot carries,
and timeline and Inspector edits go to `kerfd` as typed operations counted in
frames of the sequence.

The timeline supports click-to-seek, clip selection, drag-to-move, edge
trimming, split-at-playhead, and Delete. The tools in the timeline's bar
pick what a click or drag on a clip does, with the usual keys while
the timeline has focus: Selection (V) moves and trims, Razor (C) splits
where you click, Ripple Edit (B) trims and closes or opens the gap, Rolling
Edit (N) moves a cut between two clips, Slip (Y) shows other media in the
same span, and Slide (U) moves a clip between its neighbours. With Snap on
(Sequence → Snap, Shift+S), dragged clips and edges snap to clip edges, the
playhead, and the start. A drop lands where the drag's preview showed it.

The Project panel shows the project's media as a bin; double-click an item
to open it in the Inspector sheet's Source page. There, scrub the bar under the
picture or step with Left and Right, and mark the range with I and O (Clear In
and Out is Ctrl+Shift+X); ⇧F goes to its clip in the timeline, and the Media
sheet's Index lists its shots. Insert (,) and Overwrite (.) place the marked range at
the playhead on the first unlocked picture and sound tracks the media needs;
a picture and its sound are placed as linked clips, and Insert ripples the
rest of the sequence. The Inspector (see below) shows and edits the
selected clip.
Tracks follow the classic edit-suite order: picture tracks stack upward from
V1, because a later picture track plays over the ones below it, and sound
tracks run downward from A1. The ruler and track headers stay pinned while the
timeline scrolls. Each header has two toggles after the track number: the
eye turns a picture track's output on and off (M mutes a sound track), and
the padlock locks the track against edits. While a clip
moves or trims, a readout shows the new edge timecode and its offset.

Tracks start dense, for edits with many layers: under a wide picture V1 is
28 pt, V2 24 pt, and every other track 20 pt. The timeline's strip holds the transport, the Selection and
Razor tools, Snap and a zoom (− slider +). The sequence starts fitted to the
lanes, and stays fitted as it and the window change; the fit has a place on the
slider, and − stops there on its way back: at a long sequence's fit, the
slider's low end. ⌘-click (or ⇧-click) a clip to add
it to the selection. The track heights in a head's context menu set every
track's height at once: Mini (one 18 pt line per track: its number, toggles, name, and clip
names), Medium (the defaults), and Large; Fit shares the panel height across
every track. Option-Up and Option-Down (with the timeline focused), or
Option-scroll over the track headers, step every track taller or shorter from
its own height, so tracks sized apart (tall picture over short sound, one tall
hero layer) keep their proportions and a step taller never shrinks one, and
keep the track you are looking at in place; a trackpad's momentum after the
fingers lift steps nothing. Drag the line under a track header
(the pointer turns into a resize cursor over it) to resize that track;
Option-drag it to resize every picture track, or every sound track, at once.
Right-click a header for the Mini, Small, Medium, Large, and Jumbo heights,
for one track or all of them; View → Fit tracks to panel and View → Reset
track heights match Fit and Medium. Below 40 pt a header is one line; clips
drop their waveform below 30 pt and their filmstrip below 34 pt, and keep
their names at every height. Heights are remembered per project in the
editor's settings on this Mac: they are a view setting, never saved in
`project.kerf`, which collaborators share.
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
All tracks remain vertically scrollable when their combined height exceeds the
timeline's lanes, and the ruler's scale fits the sequence to the lanes by
default.
Import media from File. Moving the pointer across the timeline
scrubs the viewer and shows the time beside the pointer. Play and pause are
Space; with the timeline focused, Home and End jump to the start and end,
Left and Right step one frame, S splits the selected clip at the playhead,
and Delete and Shift+Delete delete or ripple-delete it (Kerf keymap). The
transport is in the timeline's strip (under the picture for a vertical sequence): go to start, previous frame, Play (Space),
next frame and go to end; the marks (I, O, Clear Marks ⇧⌘X) and Loop (⌘L) are keys and menu items,
and the ruler shows the marks. A click on the picture plays or
pauses. Captions, the QC overlap box, safe zones and the comments' drawings
lie over the picture.

## The layout

**The start window.** With no project, Kerf asks one question beside the recent
folders. Sending a request opens the chosen folder's editor with the request on
its way to Kerf. A folder with no `project.kerf` becomes a project, with the media
at its top level in its media and the timeline left empty. Open without asking opens the folder as it
is, and New Project… is the Choose a Project window, for templates, presets and
clones. A request Kerf cannot take yet (no connection set up) waits in the
composer.

**The regions** stay flush, with one hairline between them and no cards, radii
or margins (`ui/stage_shell.h`; `stage::rects()` is the geometry, a pure function
of the window's size, the picture's aspect and which panes are open). The toolbar
is 40 points tall. The sidebar is 240 wide and gives way in a window under 1100.
With the panel closed the conversation has the rest of the window, its text in a
720 wide column. With it open the conversation is 400 wide, giving way down to 320
only when it would leave the panel under 600. At a 1600 × 1000 window:

| Sequence | Sidebar | Conversation | Picture | Timeline |
|---|---|---|---|---|
| Wide (aspect ≥ 1.2) | 240 × 960 at (0, 40) | 400 × 960 at (240, 40) | 960 × 420 at (640, 40) | 960 × 540 at (640, 460) |
| 9:16 | 240 × 960 at (0, 40) | 400 × 960 at (240, 40) | 360 × 800 at (640, 40) | 600 × 960 at (1000, 40) |

Under a wide picture the timeline has 56% of the height, at least 260, and
whatever the picture's aspect does not use; the picture pillarboxes in black in
its slot. A sequence narrower than 1.2 has two columns in the panel: the picture
over a 160 tall block with the playhead's timecode and the transport, and the
timeline at the content's full height. The timeline keeps 600 of width while the
picture's column can give it up, down to 240; the picture then letterboxes in
black. One pane alone has the panel: a timeline with the transport in its strip,
or a picture with the transport's block under it (88 tall under a wide picture).
Track heights do not follow the timeline's height, so a big edit shows at once.
Switching format lays out again at once, with no animation.

**The panel** opens by itself with the first thing to show: a clip on the
timeline, or a proposal. After that, or once a pane is opened or closed by hand,
it stays as it is left. Clicking a timecode or previewing a proposal in the
conversation shows the picture, and Media and Effects show the panel, so a drag
has somewhere to land. The toolbar's two pane buttons, ⌥⌘P and ⌥⌘T, open and close
the picture and the timeline.

**Watching Kerf work.** While Kerf works it refreshes the timeline after each
`edit`, so its proposal grows there as it is built, and a blue Kerf cursor on the
timeline shows where it is: what a tool looks at, or the newest change. With Follow
Kerf on (⌥⌘F, the toolbar's eye; its choice is kept) the picture goes there too,
unless something is playing.

**Watching what changed** (`ui/change_reel.h`). A proposal card's Watch what changed
plays only the stretches its changes touch, two seconds either side, one after
another, with a strip over the picture: where you are, previous and next, Before
and After, and Exit. After plays the proposal's rendered preview, and Before the
program as it is.

**Versions** (`ui/compare_view.h`). With the composer's branch button on, Send has
three agents make their own version of the request at once. Each stages its own
proposal in a thread of its own, and the versions lie over the timeline side by
side, leaving the picture free: Watch plays what one changes, and Use this accepts
it and rejects the others.

**The sidebar** (`ui/project_sidebar.h`, ⌥⌘S; its choice is kept) lists the recent
project folders that still exist, the current one unfolded to New conversation and
its conversations, the review log's threads, titled by their first line. A
conversation with a pending proposal says Review, a settled one Done, and the
folder says Working while Kerf is. Choosing another folder opens it as File ▸ Open
does.

The toolbar holds the traffic lights' room, one ≡ button, the sidebar's button,
the centred title and subtitle, the panes' two buttons, the two format icons and
the Export split. ≡ opens Media (⌘1), Effects and titles (⌘5), Inspector (⌘4), the
three panes, Scopes, loudness, QC, Deliver, Import…,
Sequence settings…, Settings… and View: the mask tools, annotations, safe zones
(off by default), viewer zoom, the keyboard's choice and the sync's rows. Every
row is a registry command. The undo, redo, split, delete and import commands have
their keys and native menu items.

There is no status bar. The toolbar's subtitle says what one would: a passing
message for its timeout, an offline or error state until it clears, a running
sync's stage or a render's progress ("Draft export · 42%"), and otherwise
"Edited 2 min ago · rev 22" ("· Viewing as Mina" while a guest's view is on). An
agent that cannot take requests (no key, Kerf Cloud signed out) says so in the
composer's placeholder, with the reason on its tooltip. The
connection, error and project.kerf bars that carry an action lie over the top of
the content while they show.

**Sheets.** Media (the bin and roles, 272 wide), Effects and titles (the browser,
304) and the Inspector (its pages, Source among them, 304) are overlays over the
conversation, at its panel side, its top and of its height, with a left hairline
and a close button at the end of their strip. One shows at a time; the
same command toggles and Esc closes; asking Kerf (⇧⌘J, Ask Kerf on a row) closes
one or a tray that lies over the composer. Selecting a clip opens nothing by itself. A
sheet never covers the timeline, so a drag from Media or Effects reaches it.

**Trays.** Finish (⌥⇧3: scopes, loudness and QC) and Deliver (⌥⇧4: masters,
preflight, loudness and paperwork) are overlays 344 tall across the content beside
the sidebar, at the bottom; the picture and the conversation stay where they are
underneath. Esc returns to the base state (⌥⇧2). The colour wheels are in the
Inspector sheet while Finish is open, and the Inspector's Deliver pane while
Deliver is.

**The picture.** Masks, annotations, captions and overlays lie on the frame at
its size. While a proposal is being previewed, a chip 12 points inside the
picture's top left reads "Previewing proposal vN" with Exit preview. The agent
HUD stands in the picture's title-safe area while a preview renders.

**The timeline.** Heads are 140 wide: the target button and the name; lock, hide,
mute and solo appear on hover over that head, and sync lock is in its context
menu. Under a wide sequence the default heights fit the lanes: V1 40, V2 28, each
track above them 20, sound 24. Vertical sequences keep 36 and 32. Comment marks
and proposal bands stay on the ruler.

**The conversation** is always open; ⇧⌘J puts the keys in its composer, and with
no timeline open the keys start there. With no picture open its composer has no
timecode stamp and no Draw on frame. Its header is "Conversation" and a ⋯ menu with search, sort and the All, Open and Proposals
filter. Rows and proposal cards fill the panel's width at every width.

Settings from before this layout (saved workspaces, the panel's width, device
previews) load without error and are ignored.

## Comments and review

The comments panel is the agent chat: every thread is a comment in the
project's review log (`review/comments.jsonl`, kept by kerfd beside
`project.kerf`, append-only and merged as a Git union), so a note never makes
a pending proposal stale and never enters undo. Threads come from people,
from Kerf (your turn and Kerf's reply are both kept as comments), from agents
connected over MCP (their `note` tool and their proposals, with versions)
and from QC (blocking findings of a check, resolved when a later check no
longer finds them). Filters (All / Open / Proposals, From, tags, search),
sort, resolve, drawings on the frame, @mentions and timecodes work as in the
design; the pinned proposal card previews, accepts or rejects the version it
shows. Copy link gives a `kerf://<project>/rev<N>@HH:MM:SS:FF` review link;
Deliver's paperwork gives `kerf://<project>/rev<N>` with the branch, head and
remote. `<project>` is the slug of the project's folder name (lower case, each
run of anything but letters and digits one `-`: "Atlas Cut" is `atlas-cut`),
whatever the document calls itself, so a link finds a project that is not
open; a name outside ASCII travels in its punycode form and is read back. On macOS the app registers the `kerf` URL scheme, so a clicked link
opens (or raises) the project's editor and seeks to the timecode; `kerf-qt
kerf://atlas/rev22@00:00:17:08` does the same from a shell. A project this
Mac has not opened is refused with a message, never guessed.

The bridge also gives the editor (see `bridge.h`): the program meter
(`meter`, L/R peak and RMS with BS.1770 momentary and short-term loudness)
and dropped frames (`clock.dropped`), solo monitoring, effect and generator
previews with parameters, the full QC `check` (loudness, true peak, gamut,
luma, title safe, caption overlap and timing, flash and black), programme
R128 `loudness`, a render queue with ProRes 422 HQ masters, paperwork
(comments CSV, markers, CMX3600 EDL with locators, SRT/VTT), media
information and the media index, and looks saved and applied as
`.kerflook` files. Deliver keeps its output folders and paperwork choices per
project, keyed by the project's absolute path.

## Projects

A project is a folder: `project.kerf`, a `media/` folder, and a `presets/`
library. Started without one, Kerf opens the start window (above). Its New
Project… and File → New Project… open the launcher, like an edit suite's
Choose a Project window, in the editor's dark graphite and blue accent.
New Project offers Empty
16:9 and Vertical 9:16 templates; Details sets the name,
location, frame rate, and sharing: Share with Git makes the folder a
repository, and Create a private GitHub repository (offered once the GitHub
CLI is signed in, naming the account) or a remote URL publishes it. Recent lists the projects opened
lately with their branch and last edit. From Preset starts a project from a
shared preset: pick one from your projects' `presets/` folders or any
`.kerfpreset`, then choose a file for each media slot; each slot says what
it needs, and Choose waits until every file fits. Clone from GitHub copies a
shared project by its Git address. Open an Existing Project… opens any
folder. Choose explains in the footer why it is unavailable, shows progress
while the folder is made or cloned, and reports what went wrong.

File → New Project… (⌘N) opens the launcher, Open Project… (⌘⇧O) a folder,
and Open Recent a recent project; each opens in a new window and closes the
current one. Import Media keeps ⌘O.

## Collaborating

Collaborate syncs a project shared through Git with collaborators, using
Git and GitHub as they are: Sync Now commits your edits, pulls and merges
the upstream branch, and pushes. `project.kerf` merges by clips and tracks;
where both sides changed the same thing, your version is kept and the sync
says so. Live Sync syncs every few seconds and shortly after each edit; it
is remembered per project. Commit…, New Branch…, Switch Branch, Open Pull
Request…, Publish to GitHub…, and Consolidate Media (which copies media
from outside the project into `media/` so collaborators get it) complete
the menu; each unavailable item says why.

The toolbar's subtitle shows what a running sync is doing, an unfinished merge
or a failed sync, and ≡ ▸ View ▸ Sync says how far the branch is ahead of and
behind its upstream, with Sync Now. After a sync, each incoming commit
(`jun · Trimmed c3`) and each change that kept yours is listed in the activity
log; errors use the error
banner. When `project.kerf` changes outside Kerf (a `git pull` in a
terminal, a text editor), a banner offers Load from Disk or Keep Editor
Version.

The Presets panel (Window → Presets, behind the Project bin) lists the
project's preset library, which travels with the repository: each preset's
slots, author, and last change. Apply chooses this project's media for its
slots and stages it as a proposal; Save Current as Preset… (also in File)
adds the program sequence to `presets/`. The list refreshes after each save
and sync. Right-click media in the Project bin to Replace Media… (swap the
file behind the media for everyone; its clips keep their timing), Reveal in
Finder, or Consolidate Media. Bin items show Offline when the file is
missing, Not downloaded for a Git LFS file a sync has not fetched, and
External when it lives outside the project folder.

## Inspector

Click a clip and the column beside the picture turns into that clip: the
Inspector comes to the front of the Project / Inspector / Effects / Presets
column (Window ▸ Inspector, ⌘4) and the chat beside the picture is never
touched. It remembers the tab it came from and goes back when the selection
clears (Esc, or a click on an empty lane), unless you chose the Inspector
yourself; a clip selected by an edit (an insert, a drop, a proposal, undo)
raises nothing, and a closed Inspector is never reopened by a click. While
you have Effects or Presets in front, a click on another clip leaves the
browser there (it follows the selection), so a look can be applied clip by
clip; only the Inspector's own Add effect brings Effects up for the next
click to replace, and after an effect is added there a deselect still goes
back to the tab from before the Inspector came up (Project). Window ▸
Inspector Follows Selection (⌥⌘4) turns this off. Raising never moves
keyboard focus. These are preferences of this Mac (QSettings
`editor/inspectorFollowsSelection`, `editor/inspector/…`); layouts saved with
Effect Controls (layout version 6) restore with the Inspector in its place, and
`window.effectControls` still names the command.

Its header says what the clip is: a thumbnail, its name (double-click to
rename it, `set {name}`), its kind in the clip's timeline colour (Video,
Sequence, Audio, Title, Solid, Adjustment, Caption) with its track, and Start,
End and Duration as timecode fields (drop-frame where the sequence is). Under
it are the handful of things you change, with no header — Scale (with Fit /
Fill), Position, Rotation, Opacity, Blend and Speed for a picture; Volume, Pan
and Speed for sound; Text, Font and size, Colour, Align and Place for a title
— then sections folded by default whose headers say what changed in them
("Bottom 60 px"), in whole words or not at all — the folded Effects header
names the effects that are on as far as they fit, then " +N" and " · 1 off",
or just "1 on · 1 off": Crop, Anchor & flip, the linked clip's Audio, Masks,
Effects, and Source (its file and format, and Source In and Out, which slip
the clip). Values read in px, %, °, dB and timecode; a value at its default
is drawn quieter than a changed one, and a fill means it can be changed.
Drag a value or its label to scrub it (⇧ ×10, ⌥ ÷10; one edit on release),
click to type (`+10`, `*2`, `50%`, `-3dB`, `7:00`, `+12`), and step with ↑↓
while typing (a held key or a quick run of presses is one edit; Esc takes back
a run not yet sent). A click or a drag in the
Inspector takes no keyboard focus, and Return or Esc after typing (or Esc
anywhere in it) hands the keys back to the timeline, so Space and J/K/L keep
working while you nudge values. Every row shares one grid (76 pt labels, 72 pt
value fields, 84 pt sliders at the 300-point column, controls ending at one
edge); a click on a slider changes nothing, and a drag moves the knob by as
much as the pointer moves, wherever it was grabbed (⌥ ÷10). The
default catches the knob only on the one pixel nearest it, so every value next
to it can still be dragged to; a double-click on the knob resets it. A slider's
default is a 1-point tick; under the knob its ends show above and below it, and
nothing is drawn on the knob, whose edge stands out against the groove and
panel once the value differs from its default (quieter at it). A switched-off
effect's values stay in the secondary ink, readable and editable;
↺ on hover, or a double-click on the label, resets a row, and a section's ↺
resets it all. Each effect has a switch, its name, a second line listing what
changed while folded, five parameters before "N more" when that hides two or
more (six show whole; grouped under quiet sub-headers when the catalog groups
them), and ⋯ for Reset Parameters, Ask
Kerf and Remove. Effect names read in Title Case and row labels in sentence
case, on one 12-point line: a first word the effect's name already says goes
("Threshold" under Edge Glow), and a label still too long ends in an ellipsis,
whole in its tooltip. Animated values show their value at the playhead, read-only.
With several clips it shows what they share ("Mixed" where they differ; a drag
moves each alike, in one edit; End is the span's end, and a new End trims only
the clip ending there, so no gaps open inside the span); with nothing selected, the sequence's format,
tracks and markers, with only its frame size to change. ⋯ ▸ Ask Kerf opens the
chat about the clip, and the composer names the selected clip. Selecting
another clip of the same shape (kind, effect stack, masks) only sets values:
the rows are not rebuilt, and open sections and the scroll position stay. From
the click to the painted values takes 7.9 ms (median, a clip with ten effects
open, measured unoptimized and offscreen on an M-series Mac; the test prints
it and allows four times that, as GitHub's shared macOS runner takes 17-21 ms).

## Effects

The Effects panel (Window → Effects) shows the catalog's picture effects as
cards, narrowed by category and by words ("glow", "zoom", "retro"), each
card the selected media clip's picture at the playhead through that effect.
Double-click a card, or press Return, to add the effect to the selected clip;
the edit is an ordinary one, undone like any other. Dragging a card onto the
Inspector inserts the effect where it lands. The Inspector lists the clip's
effects in order (see below).

## Masks

The Rectangle, Ellipse, and Pen mask tools sit in the Program panel's title
bar and in the Inspector's Masks header; they draw on the selected picture
clip. Drag out a rectangle or an ellipse on the picture; with the
pen, click each corner and click the first one again to close the outline,
or press Return to leave it open as a path (Escape drops it). A new mask is
named `k1`, `k2`, … and selected. While the clip shows masks, the monitor
holds its picture: drag a handle to move a corner or point, drag inside a
mask to move it, and press Delete to remove the selected one. Masks are
drawn where the clip's picture lands, after its crop, scale, rotation, and
position; a mask with mode `none` is dashed, a path that effects such as
Stroke and Reshape read. The Inspector lists the masks with their mode,
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

Kerf's proposal shows on the timeline before anything changes: clips it adds
or changes are outlined where they would land, and clips it removes are
struck through. Kerf's turns are threads of the Comments panel (above): your
request is a comment, Kerf's answer a reply, the steps its tools took fold
under it behind "2 steps", and while it works its drafting row shows the
latest step with Stop. A pending proposal is a card in its thread (pinned
first, the pin bar keeping it in reach while it is scrolled away): its
changes in words, Preview, Script (the script and the document diff, in
colour), Reject and Accept (⌘Return; Escape with the timeline focused
rejects), and Versions once Kerf has revised it. Kerf's next steps become
the welcome's requests to try and the composer's suggestions. The model each
request names is the active connection's (Settings); with your own keys the
More menu's Model chooses among its models, and Agent settings… names the one
in use. While the panel is closed, a bar over the timeline stands in for it:
it says what Kerf is working on (with Stop) or names the pinned proposal
(with Reject and Accept), and Details opens the panel again.

Use the project-format picker in the toolbar to switch between 16:9
(1920 × 1080) and 9:16 (1080 × 1920). It names the format as the window's
subtitle and the Inspector's Frame size do ("9:16 · 1080 × 1920"), and any other
size, set with Frame size's or Sequence settings' Custom, as
"Custom · 1280 × 720". The viewer fits either canvas
inside the available panel without cropping and keeps preview captions inside
the video safe area.

The connection banner reports connection progress and offers Reconnect after
a failure. The toolbar's subtitle keeps the connection state and project revision
in view and shows runtime messages. Editing controls become available only after a successful project
snapshot. Changes made through CLI or MCP are checked every 500 ms while idle;
unchanged snapshots do not redraw the UI or reopen playback.

Proposal review shows semantic clip changes, time ranges, and all supplied
before/after field values in a scrollable view. Stale proposals and proposals
whose field diff was truncated cannot be accepted; ask the agent to restage
or split them into smaller changes.

Agent settings are in Settings (Edit → Settings…, ⌘, ; on macOS in the
application menu), also reachable from the chat's ⋯ menu. Settings → AI
chooses Kerf Cloud, the hosted model (the default unless a key of your own
is already saved; see
[docs/cloud.md](../../docs/cloud.md)), or My own API key. Kerf Cloud shows the
account, plan, and requests left, with Sign in (a token, for now) and Sign
out; Ask Kerf stays unavailable, with the reason, until the account can make
requests. Advanced
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

The tests cover every region (one class per region: the picture, the stage shell
and its geometry, comments and the review model, Inspector, timeline, bin and
browser, Source, toolbar, status, Finish, Deliver, the kit, theme, launcher,
wiring guards, kerf:// links), each fed bridge events shaped as kerfd sends them,
and `MockupParityTest` checks the whole editor against the design at 1600 × 1000
in 16:9 and 9:16, with each sheet and tray and a preview.
With `KERF_SCREENSHOT_DIR` set, it saves each state under the mockup's query state
(`169.png`, `916-inspector.png`, `169-finish.png`, `916-preview.png`, …), and the
region galleries save theirs (`editor-<format>-dark.png`,
`inspector-<pane>.png`, the launcher's pages, the comments panel's states).
The pictures in them are the tests' procedural footage (`test_footage.h`:
a harbor at dawn, a close-up on a dock and a drone pass over the coast,
drawn in code with texture and grain), so no media files are needed.
The macOS CI job also runs the real bridge/media workflow in
`scripts/test-kerf-qt-bridge.py`.
