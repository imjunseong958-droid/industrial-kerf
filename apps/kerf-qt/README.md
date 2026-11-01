# Industrial Kerf Qt editor

This is the native Qt6 desktop shell for Kerf. It keeps the project state in
`kerfd` through the small C ABI in `bridge.h`; the window does not maintain a
second editable copy of the timeline. Its docked panels — Project,
Source, Program, Timeline, and the Inspector — and the Kerf Assistant (the
chat, in the Program monitor's stage) are all backed by the bridge event
stream.

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
`KERFD_BIN`. Without a project, Kerf opens the project launcher instead
(`KERFD_BIN=/path/to/kerfd target/qt/kerf-qt`). For a quick visual smoke
run, add `--screenshot /tmp/kerf.png --quit-after-ms 900`; without a project
it captures the launcher.

The editor and the launcher follow the system's light or dark appearance and
switch live when it changes; `KERF_COLOR_SCHEME=light` or `dark` picks one
for the run. Both draw one palette: antique cream paper in light (#f4eee2
panels on a #e6dece desk) and warm espresso in dark (#1c1713 panels), with
warm near-black or cream text, never pure black or white. Kerf has its own
accent and does not follow the system's: an aubergine ink (#6a3471) on cream
and its mauve (#c39acb, carrying dark ink) on espresso, for focus, the
playhead, each default action and Kerf's own work (proposals, Ask Kerf).
Chosen tools and modes are a quiet wash with no outline, groups and cards
sit on their panel without frames, and the type follows one scale (11, 12,
13, 14, 16 and 20px). The viewers' stage follows the appearance; the
picture card is warm black in both. On macOS the editor's toolbar is its title bar, as the
launcher's sidebar and title are: the window runs up under a transparent
title bar, the window buttons sit at the toolbar's leading end, and the
project's name is the window's title, centred on the window. Pop-up buttons
and menus open as macOS's do (rounded, a check by the current choice, the
accent under the pointer, no command symbols), buttons and pop-up buttons
take focus from Tab alone, shown as a soft accent ring, and a field being
typed in always shows that ring.

Sources live in the `kerf` namespace, one class per header/source pair:
`main_window` (the window, which builds and connects the panels),
`project_launcher` (the Choose a Project window), `workspace` (the docked
panels and workspaces), `project_panel` (the bin), `presets_panel` (the
project's preset library), `source_monitor_panel`, `effect_controls_panel`,
`timeline_widget`, `viewer_widget` (the viewer, its mask drawing, and the
focus-region picker),
`command_registry` (the editor's side of the command registry), and
`media_artwork` (background thumbnails and waveforms). `editor_common`
holds shared constants and helpers (among them `useEditorStyle()`, the
application style: Fusion with macOS's focus, pop-up rows and menu shapes),
`editor_theme` the appearance (the
`EditorTheme` colour roles every widget draws with, light and dark, Kerf's
accent, and the system appearance detection the launcher shares),
`editor_style.h` the stylesheet generated from those roles, and
`main.cpp` only `main()`. `editor_test.cpp` (with `test_footage.cpp`) builds into `kerf-qt-tests`
instead of `main.cpp`.

Menus, shortcuts, the toolbar, the player's controls, and timeline gestures
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
and timeline and Inspector edits go to `kerfd` as typed operations counted in
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
rest of the sequence. The Inspector (see below) shows and edits the
selected clip.
Tracks follow the classic edit-suite order: picture tracks stack upward from
V1, because a later picture track plays over the ones below it, and sound
tracks run downward from A1. The ruler and track headers stay pinned while the
timeline scrolls. Each header has two toggles after the track number: the
eye turns a picture track's output on and off (M mutes a sound track), and
the padlock locks the track against edits. While a clip
moves or trims, a readout shows the new edge timecode and its offset.

Tracks start dense (28 pt picture tracks, 22 pt sound tracks), for edits with
many layers: eight fit the timeline at 1440×900, six at 1280×800. The
Timeline's tab row says how many tracks and how long ("9 tracks ·
00:00:18:00") after its name, and on the right has a zoom (− slider +) and
the track heights; ⌘-click (or ⇧-click) a clip to add it to the selection,
which the status bar's left end counts ("2 clips selected · 00:00:11:00").
The track heights set every track's height at
once: Mini (one 18 pt line per track: its number, toggles, name, and clip
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
and Delete and Shift+Delete delete or ripple-delete it (Kerf keymap). The
Program monitor's transport lies on the picture, as a video player's
controls do: a gradient along the picture's foot, a progress bar across it
(thicker under the pointer, its played part in Kerf's purple, the time under
the pointer shown over it; click or drag it to seek), and a row with play or
pause, a frame back, a frame on, and the time over the program length. On a
portrait picture they take a short-video player's arrangement: the bar along
the foot, the buttons at the top left, the time small over the bar. They
show while paused and while the pointer is over the picture, and fade out
2.5 seconds after it stops while playing; the keys never depend on them.
As on a video player, a click on the picture plays or pauses (the bar
seeks), and moving the pointer across the picture skims it from start to
end, as the bar maps it; the stage around the picture does neither. The
caption preview rises over the controls while they show, as a player's
captions do. Nothing sits under the picture. The Editing
workspace reads as a Short does on YouTube's desktop page: the Program
monitor takes the top row past the browsers, and in its stage the picture
card (12-point corners), an action rail and the Kerf Assistant (the chat)
form one group centred in the stage, the chat panel level with the card (or
the phone or laptop it shows in) and exactly as tall, as a Short's comments
open beside it: where the stage has the room the card is made tall enough
for a chat of 352 points (its header, a couple of comments, the chips and
the composer), the chat narrowing for it. A 9:16 or 1:1 card fills the
stage's height; a 16:9 one takes the height, or the width the rail and chat
leave it. With the chat closed the card is centred alone, the rail hanging
on its right. In a narrow stage the chat narrows first, so the picture keeps
four fifths of its width, then the card shrinks, and past that the chat lies
over the stage's right side.

A portrait (9:16) picture shows in a phone, as a simulator shows an app: a
generic phone drawn in code, graphite in either appearance, its screen
taller than the picture (9:18.5) with the picture across it and black above
and below, a status bar, and a camera island or punch-hole. The phone's body
takes the card's place in the group, the chat level with it. The rail's
Phone button and Window ▸ Viewer choose Phone · Island (the default), Phone ·
Punch-Hole or Off (the plain card); its Safe button outlines where a
vertical-video app's interface usually covers the picture (a top bar, a
column of actions, a caption band); while they show, the player's controls
(all but the bar) wait for the pointer to come over the picture. Both are
preferences of this Mac. Every format shares one layout: at 1440 × 900 the
stage row is about 575 points over a 290-point timeline, so the timeline does
not move when the format changes.

A wide (16:9) picture shows in a laptop, the phone's companion: a generic
laptop drawn in code in the same graphite, its lid's screen 16:10 inside a
thin even bezel (a little thicker over it, with a camera dot) so the picture
sits across it with thin black bars above and below, a hinge line, and a
wider, thin base (108% of the lid) with a lighter top face and a shallow
thumb-cut. The player's controls lie inside the screen. The laptop's body (lid
top to base bottom, as wide as the base, so the stage's margin and the gap
before the rail are measured from its widest part) takes the card's place in
the group, the chat level with it and as tall; at 1440 × 900 the picture keeps at least 90% of the plain card's width
(the bezel and base come out of the chat's width). The rail's Laptop button
and Window ▸ Viewer choose Laptop (the default) or Off; it is a preference of
this Mac. A square picture stays on the plain card: in a phone it would be
under half as wide (measured at 1440 × 900: 499 points wide on the card, 242
across the phone's screen). A card, a laptop and a phone all keep the same room
over them as under them, 16 points; only a phone in a stage too short for a
240-point screen at 16 takes as little less as it needs (12 at 1440 × 900).
The phone's and the laptop's frames are drawn once for
their size, appearance and pixel ratio and then only blitted, so playback
costs no more for them.

The Source monitor is a tab behind the Program monitor; double-clicking media
brings it forward. The chat stays with the Program monitor: while the
Source monitor covers it, it counts as closed, so the bar over the timeline
stands in for it and ⇧⌘J brings it back into sight. Left of them,
the Project bin, the Inspector, Effects, and Presets share a column of
about 300 points. The Timeline runs the window's full width under both, for
its many tracks; the stage row takes the height (575 over a 290 timeline as
arranged, for every picture shape). Panels have text tabs (the chosen one
underlined), no title strips; the window's title has the project's state
under it ("Edited · 9:16 · 24 fps"). The Project bin is "Search media" over
two columns of tiles, each with its name, duration and shape.

The chat opens and closes like the comments button beside a Short: the
rail's speech bubble (with the number of messages under it), the bubble at
the end of the toolbar, Window ▸ Kerf Assistant, ⇧⌘J, or the close button in
the chat's header; opening it brings the Program monitor forward. Drag the
chat's leading edge to resize it (300 to 420 points; 360 at first). Its width
and whether it is open are preferences of this Mac (QSettings), never part of
`project.kerf`. Layouts saved before this arrangement (layout versions 4 and
5) give way to it for the Editing workspace; workspaces of your own saved
then keep their arrangement, without the old Assistant dock. An empty
timeline starts without tracks. Hover over the
empty row in the left track header to reveal a plus; click it to add a video
or audio track. Double-click a clip to open its media in the Source monitor
at the clip's in point.

## Projects

A project is a folder: `project.kerf`, a `media/` folder, and a `presets/`
library. Started without one, Kerf opens the launcher, like an edit suite's
Choose a Project window. Like the editor, it follows the system's light or
dark appearance in the editor's palette and accent, and switches when it changes
(`KERF_COLOR_SCHEME=light` or `dark` picks one). New Project offers Empty
16:9, Vertical 9:16, and Square 1:1 templates; Details sets the name,
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

The status bar shows the branch and how far it is ahead of and behind its
upstream (`⎇ main ↑2 ↓1`), what a running sync is doing, live sync, an
unfinished merge, or a failed sync; click it to sync now. After a sync,
each incoming commit (`jun · Trimmed c3`) and each change that kept yours
is listed in the activity log and the chip's tooltip; errors use the error
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
it and allows twice that on CI's shared runners).

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
clip. Drag out a rectangle or an ellipse on the Program monitor; with the
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
struck through. The Kerf Assistant starts open beside the picture and is laid
out as a video's comments panel: a header with its title, the number of
messages, ⋯ (Agent Settings…, Activity) and a close button; every turn is a
comment row (an avatar, the name and time, the text, and quiet Copy and Ask
again actions), with no bubbles; the steps Kerf's tools took fold under its
comment behind "▸ 2 steps", as replies do; and while Kerf works three dots
and its latest step follow the latest comment. The comments scroll under the
header as a comments panel's do: cut cleanly at its edge, with a hairline
under it while the list is scrolled away from its top. A sliver is never left
at either edge, and under the top one a line of text, or a text button's
words (Copy, Ask again, "2 steps"), shows only with its capitals' tops (a name
cut through them would read as another word). A file name in a comment wraps
as one word, never at its hyphen, and selected text copies as written. A
pending proposal is pinned at the top of the list ("Pinned by Kerf") and scrolls away with it;
when one arrives the list shows it from its head (following the latest again
once you scroll or send). It lists its changes in words, a row each with a
glyph and a detail ("Trim fisher-close.mov — end −1.5 s", "Remove
drone-pass.mov — 0:11–0:18"), up to three, under a summary that counts them
("Trim, remove, add a title · 3 changes"). The amount or time is never cut:
in a narrow panel (as beside the laptop) a row first drops the clip's file
extension ("Trim fisher-close"), then puts the detail on a second line under
it, with the full wording in its tooltip. Then come Reject and Accept pills
(⌘Return; Escape with the timeline focused rejects) and a hairline; its
script and diff are behind "Script" (as tall as their lines, up to ten).
While its head or its pills are scrolled out of view, a compact bar under the
header keeps its verbs (or only "3 changes" where they would not fit
whole), Reject and Accept in reach. The up to three next steps Kerf ends each turn with
("Stronger glow", "Wipe to the next shot") are one row of chips over the
composer, scrolling sideways rather than wrapping, so the conversation
keeps its height; pressing one asks for it about the same selection.
The composer is the panel's footer: your avatar and one line over an
underline that turns Kerf's purple in focus, growing with the draft, with the
model, Cancel and Send (Stop while Kerf works) shown while it is in use.
Return sends and Shift+Return inserts a new line. Before the first message
Kerf's welcome offers two requests to try (plain rows after an arrow, wrapping
to two lines), which fill the composer. While
the chat is closed,
a bar over the timeline stands in for it, as a Short's comment count does:
it says what Kerf is working on, offers Stop, Accept, Reject and the next
steps, and Details opens the chat again.

Use the project-format picker in the toolbar to switch between 16:9
(1920 × 1080), 9:16 (1080 × 1920), 1:1 (1080 × 1080) and 4:5 (1080 × 1350). It
names the format as the window's subtitle and the Inspector's Frame size do
("1:1 · 1080 × 1080"), and any other size as "Custom · 1280 × 720". The viewer fits either canvas
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
application menu), also reachable from the chat's ⋯ menu. Settings → AI
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
long timeline fit, scrolling through many tracks, complete proposal review,
the sync chip and Collaborate requests, the diverged banner, the Presets
panel, media badges, and the launcher's validation (test builds never start
launcher tasks), and the launcher's and the editor's looks in both
appearances; with `KERF_SCREENSHOT_DIR` set, `launcherRendersBothAppearances`
saves each page as `<page>-<light|dark>.png`, and
`editorRendersBothAppearances` saves the editor (`editor-`), Settings
(`settings-`), the preset bindings dialog (`dialog-`), the diverged banner
with an agent proposal and conversation (`states-`), and the Inspector
with an effect's parameters (`inspector-`), each `<light|dark>.png`;
`inspectorRendersEveryState` saves the Inspector for a video clip (its Scale
row under the pointer), its effects, an audio clip, a title, an adjustment
clip, a caption, nothing selected and three clips (`inspector-video-`,
`-video-effects-`, `-audio-`, `-title-`, `-adjustment-`, `-caption-`,
`-nothing-`, `-multi-`), and the whole editor with a clip clicked
(`editor-9x16-inspector-`, `editor-16x9-inspector-`);
`editorShowsEveryFormatBesideItsConversation` saves the whole editor with
the chat open beside a 16:9 (in its laptop), a 9:16 (in its phone) and a 1:1
picture (`editor-16x9-`, `editor-9x16-`, `editor-1x1-`), the 16:9 one with
the laptop off (`editor-16x9-off-`), the 9:16 one in the other phone, with
safe zones and on the plain card (`editor-9x16-punchhole-`, `-safezones-`,
`-card-`), and both with the chat closed (`editor-16x9-closed-`,
`editor-9x16-closed-`); `conversationRendersItsStates` saves the chat's
welcome, a long streaming conversation, a pinned proposal and, scrolled away
under a longer conversation, its compact bar (`editor-9x16-welcome-`,
`-streaming-`, `-proposal-`, `-proposal-scrolled-`);
`timelineFollowsTheAppearance` saves the timeline (`timeline-`,
`timeline-drag-`) and `paintedPanelsFollowTheAppearance` the Program monitor
(`viewer-`, `viewer-empty-`), the region picker (`region-`), the Source
monitor (`source-`), the bin (`bin-`) and the Effects panel (`effects-`).
The pictures in them are the tests' procedural footage (`test_footage.h`:
a harbor at dawn, a close-up on a dock and a drone pass over the coast,
drawn in code with texture and grain), so no media files are needed.
The macOS CI job also runs the real bridge/media workflow in
`scripts/test-kerf-qt-bridge.py`.
