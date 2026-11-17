# Industrial Kerf Qt editor

This is the native Qt6 desktop shell for Kerf. It keeps the project state in
`kerfd` through the small C ABI in `bridge.h`; the window does not maintain a
second editable copy of the timeline. Work starts as a conversation: the folders
at the left and Kerf beside them. The picture and the timeline open in a panel at
the right once there is a cut to show, or when a cut Kerf made is opened from its card; Media opens on demand. All of them are backed by the
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

On Windows, `scripts/package-kerf-windows.ps1` makes a zip that runs on its own:
the editor, `kerfd`, `kerf`, `kerf-mcp`, and the Qt, FFmpeg and C++ runtime libraries
beside them. The script's help says what it needs. It makes the same folder an
installer too, with Inno Setup (`packaging/windows/kerf.iss`): for the current user
alone, with no administrator, a Start menu entry, `kerf://` registered so project links
open the editor, and an uninstaller. Neither is signed. `scripts/check-kerf-windows-installer.ps1`
installs it, runs the installed copy, follows a `kerf://` link as Explorer would, and
uninstalls it; the zip alone does not open links.

Its FFmpeg build makes the package GPL version 3, so publish it only together with
its complete source, from `scripts/collect-kerf-windows-sources.sh`, and keep that
available for as long as the package is. The **Package** workflow makes both,
from one commit, as artifacts, and publishes neither; a `v<version>` tag publishes
both as a GitHub release, with that version's section of `CHANGELOG.md` as its notes.

On macOS, `scripts/package-kerf-macos.sh` makes `Industrial Kerf.app` the same way: the
editor, `kerfd`, `kerf`, `kerf-mcp`, `ffmpeg` and `ffprobe`, with Qt and every Homebrew
library they load copied in by `macdeployqt`. It fails if anything still loads from
Homebrew, runs the tools and the editor with nothing of Homebrew's on `PATH`, and signs
the app ad hoc. It is for Apple silicon and macOS 26 or later, by decision: Intel Macs
are not supported, and the Homebrew libraries it carries are bottled for the runner's
macOS 26. The script writes that minimum into the app's `Info.plist`
(`LSMinimumSystemVersion`, so an older macOS says so), and `check-kerf-macos.sh` fails
if any file in the app needs a newer macOS than it names, as one built on a newer
runner would (`docs/shipping.md`, Packaging). Homebrew's FFmpeg is GPL, so `scripts/collect-kerf-macos-sources.sh`
collects the source of each Homebrew formula the app has files of; the Package workflow
publishes the two together on a `v<version>` tag. Where pushing a tag is not possible, Actions ▸ Package ▸ Run
workflow on `main` with **release** checked publishes the same release and makes its
tag. A file of 2 GiB or more, which GitHub refuses (the Windows source is over 3),
goes up in parts, and the release notes say how to join them. It takes that FFmpeg from
BtbN's builds because they pin the commit of each library in them.

On Linux, `scripts/package-kerf-linux.sh` makes a folder that runs on its own, packed as a
`.tar.xz`: the same programs in `bin/`, and in `lib/` every library they load that is not
in `packaging/linux/system-libraries.txt` (Qt and its ICU, FFmpeg, the xcb utilities,
xkbcommon), found through an RPATH of `$ORIGIN/../lib` that `patchelf` sets; `bin/qt.conf`
points Qt to `plugins/`. It carries the xcb platform only, so a Wayland desktop runs it
through XWayland. `scripts/check-kerf-linux.sh` runs it with nothing on `PATH` and no
library path on a virtual X server, and fails if a binary names a library that is neither
carried nor on that list, or if Qt, its plugins or FFmpeg load from outside the folder.
A folder registers nothing of itself, so it carries `install-desktop-entry.sh`
(`packaging/linux/`): for the user alone, it writes
`~/.local/share/applications/industrial-kerf.desktop`, which runs the folder's
`bin/kerf-qt` with the link (`%u`), and makes it the default for
`x-scheme-handler/kerf` in `~/.config/mimeapps.list`, as `xdg-mime` would, keeping
the file's other lines; `--remove` takes both away. The check runs it for a user in a
scratch home, validates the entry, has GLib's `gio` name Kerf for `kerf://`, follows
a link through `gio open` and finds the editor started with it, then removes it. The
entry's icon is Kerf's own (`Icon=industrial-kerf`): the script copies the package's
`icons/` into `~/.local/share/icons/hicolor/<size>x<size>/apps/`, `--remove` takes them
out, and the check finds each size there as a PNG of that size.
`scripts/collect-kerf-linux-sources.sh` collects its source, the Ubuntu packages it carries
libraries of included. Built on Ubuntu 24.04, it needs a glibc and libstdc++ that new.

`scripts/package-kerf-linux-deb.sh` makes a `.deb` of the same folder, for Debian 13,
Ubuntu 24.04 and what comes from them: the folder in `/opt/industrial-kerf`, the desktop
entry `packaging/linux/industrial-kerf.desktop` in `/usr/share/applications` (the entry
`install-desktop-entry.sh` writes, but for where the editor is, which the script holds it
to), the icon in the system's hicolor theme, and `kerf-qt`, `kerf`, `kerfd` and `kerf-mcp`
in `/usr/bin` as links. Its Depends come from `dpkg-shlibdeps`: the packages that provide
each library the binaries name and `lib/` does not carry, so they follow
`system-libraries.txt` without being written twice. `scripts/check-kerf-linux-deb.sh`,
as root, installs it with apt, finds every library found with only what its Depends
brought, runs `check-kerf-linux.sh` on `/opt/industrial-kerf`, checks the links on `PATH`,
the entry, the icon and a `kerf://` link followed through `gio`, and removes it; the
Package workflow runs it in bare `ubuntu:24.04` and `debian:trixie` containers.

Kerf's icon is drawn once, in `resources/icon/industrial-kerf.svg`, and `tools/kerf-icon`
renders the PNG, `.ico` and `.icns` files beside it (`cargo run -p kerf-icon`), which are in
the repository; `cargo test -p kerf-icon` fails when they no longer match the SVG
(`resources/icon/README.md`). The editor's windows and taskbar entry take it from the PNGs
compiled in (`ui/app_icon.cpp`), `kerf-qt.exe` carries the `.ico` as a resource
(`resources/icon/kerf-qt.rc`) and the installer wears it, the macOS app has the `.icns` in
`Resources` under `CFBundleIconFile`, and the Linux package carries the PNGs in `icons/`.
Each package check holds its own: the icon group in `kerf-qt.exe`, the `.icns` `Info.plist`
names, and the desktop entry's icon in the hicolor theme. A release from before 0.1.2 has
none and is checked without it.

All the packages carry the user guide as `user-guide.html` (beside `kerf-qt.exe`, and in
the app's `Resources`), rendered from `docs/user-guide.md` by `tools/kerf-guide`; Help ▸
Kerf User Guide opens it, or the guide on GitHub in a build that has none. On Linux it is
`share/doc/industrial-kerf/user-guide.html`.

Both packaging scripts end by running what they made, with `scripts/check-kerf-windows.ps1`
and `scripts/check-kerf-macos.sh`, which also fail if the editor loads a library from
outside the package. After publishing a release, the Package workflow runs **Verify
release**, which downloads the release onto fresh Windows and macOS runners with no FFmpeg
or Qt (Homebrew moved out of reach) and runs the same checks; Actions ▸ Verify release ▸
Run workflow checks any tag by hand.

The positional arguments may also be supplied as `KERF_PROJECT_DIR` and
`KERFD_BIN`; without either, the editor runs the `kerfd` beside itself (the
installed app carries one), else the one on the `PATH`. Without a project, Kerf opens the one last worked on
(`KERFD_BIN=/path/to/kerfd target/qt/kerf-qt`), or with none to go back to, an
editor with no folder. For a quick visual smoke
run, add `--screenshot /tmp/kerf.png --quit-after-ms 900`; such a run opens only the
project it is given, never the last one. `--graph <media>` opens that shot's graph
for it ([A shot's graph](#a-shots-graph)). `--window-size 1600x1000` sizes the window, and `--settings-dir DIR`
keeps the run's settings in DIR (INI files) instead of the user's.

The editor and the launcher are dark only, and fill the screen edge to edge
as Final Cut Pro does. There is one graphite ramp (#050505 behind everything,
#0e0e0e around the region picker, #121212 wells, #171717 panels,
#1c1c1c cards, #262626 raised controls, menus and popovers, #303030 chosen
ones) with warm paper inks #ece6da / #ada79b / #878278. The colours that say
something are faded, printed rather than lit: one dusty blue accent (#4a6ab0)
for Kerf, focus, proposals and the default action; sage for what passed, ochre
for what wants a look and the request to fix it, a faded brick for what
failed; the playhead is a dusty violet of its own, and a clip an open proposal
changes is outlined in sage. Only the computer's amber,
which says something runs, stays bright. Colours, type
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
chat, agent, timeline, bin, toolbar, status), and each region
keeps its state in a wiring class under `ui/wiring/` that talks to the others
only through `ui/editor_bus.h` (the bridge's events, the snapshot, the
playhead and cross-region requests). `ui/` holds the shared
parts: the kit (`components`, `icons`), `stage_shell` (the
window's content and its geometry), the proposals and conversation models, the
conversation's messages (`chat_messages`) and proposal cards, the agent HUD, and
`project_link` (kerf:// links). `viewer_widget` is the
picture, `editor_theme`/`editor_style.h` the colour roles and the
style sheet generated from them, and `project_launcher` the Choose a Project
window. `main.cpp` holds only `main()`; the tests under `tests/` (one class
per region, run together by `tests/test_main.cpp`) build into
`kerf-qt-tests` instead.

Menus, shortcuts, the toolbar, the player's controls, and timeline gestures
run commands from the registry in `crates/surfaces/kerf-commands`. Each
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
and timeline edits go to `kerfd` as typed operations counted in
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

The Media band shows the project's media as a bin, and its Index lists a
media's shots. Double-click an item, or choose it and use Insert (,) or
Overwrite (.), to place the whole of it at the playhead on the picture and
sound tracks the timeline's heads target; a picture and its sound are placed
as linked clips, and Insert ripples the rest of the sequence.
Tracks follow the classic edit-suite order: picture tracks stack upward from
V1, because a later picture track plays over the ones below it, and sound
tracks run downward from A1. The ruler and track headers stay pinned while the
timeline scrolls. A header is the track's kind on a chip (picture, captions or
sound; its number, V1 or A1, is in the chip's tip), in a narrow column, so the
clips, which carry their own names, have the width. Under the pointer the
header opens over the start of its lane, with the track's name and its
toggles: the eye turns a picture track's output on and off (M mutes a sound
track, S solos it), and the padlock locks the track against edits. The
playhead's timecode and the rate are in the strip over the timeline, after
the transport. While a clip
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
pauses. Captions, the QC overlap box and safe zones lie over the picture.

## The layout

**Starting.** There is no start window: started without a project, Kerf opens
the latest recent project that is still on disk (`ProjectLauncher::lastProject`),
as a coding agent's app opens on what was being worked on. With none, it opens
an editor with no folder, which starts no `kerfd`; its new conversation offers
the folders, Open Folder… and New Project… (the Choose a Project window, for
templates, presets and clones). A folder with no `project.kerf` opened there
becomes a project, with the media at its top level in its media and the
timeline left empty.

**The regions** stay flush, with one hairline between them and no cards, radii
or margins (`ui/stage_shell.h`; `stage::rects()` is the geometry, a pure function
of the window's size, the picture's aspect and which panes are open). The toolbar
is 40 points tall. The sidebar is 320 wide until its edge is dragged (from 232 to
all but 640 of the window; a press on the edge puts the sidebar away, and a
double-click puts the width back), and gives way in a window under 1100. With the
panel closed the conversation has the rest of the window, its text in a 720 wide
column. With it open the conversation is 448 wide, giving way down to 320 only
when it would leave the panel under 600. At a 1600 × 1000 window:

| Sequence | Sidebar | Conversation | Picture | Timeline |
|---|---|---|---|---|
| Wide (aspect ≥ 1.2) | 320 × 960 at (0, 40) | 448 × 960 at (320, 40) | 832 × 420 at (768, 40) | 832 × 540 at (768, 460) |
| 9:16 | 320 × 960 at (0, 40) | 448 × 960 at (320, 40) | 832 × 614 at (768, 40) | 832 × 346 at (768, 654) |

Under a wide picture the timeline has 56% of the height, at least 260, and
whatever the picture's aspect does not use; the picture pillarboxes in black in
its slot. One pane alone has the panel: a timeline with the transport in its
strip, or a picture with the transport's block, 88 tall, under it.

A sequence narrower than 1.2 keeps the same arrangement: the picture fits inside
its slot over a timeline with 36% of the height, at least 220. The picture keeps
12 points inside its slot, with rounded corners, on the room's neutral grey.

Track heights do not follow the timeline's height, so a big edit shows at once.
Regions move to a new arrangement rather than jump to it (`ui/stage_shell.h`), and
what is on screen becomes what comes next (`ui/stage_motion.h`). The conversation,
the picture and the room are one surface with no line between them: the shell paints
the room and the picture's shadow, and the conversation and the picture's region are
clear over it, so the picture floats. The room stays neutral, whatever the picture
shows, so its colours can be judged. Kerf's footage gathers as named tiles
in the conversation. A first cut Kerf makes is a card there from the start: while Kerf
works, light runs round its picture, which becomes each clip as it is placed, and
when it is done the ring lights once and goes out. Moving over the picture runs
through its clips, and pressing it grows it into the picture as
the conversation gives way and the timeline comes in out of the room; closing the
panel puts the picture back into the card. With the stage open while Kerf works,
each tile flies to its lane while the picture becomes that clip. While a first cut is only a proposal the timeline
shows it, read-only, the picture takes its format, and its preview follows. Watching what changed, what the picture
was wipes to and fro over what it now is, with the changed clips lifted off their
lanes; and when Kerf has rendered what was asked, the picture goes back into the
project's folder and the conversation has the room again. Under Reduce Motion
everything is simply there. [`MOTION_UI.md`](MOTION_UI.md) lists each scene and what to
reuse; start there when adding one.

**The panel** opens by itself for a cut there is to show: clips on the
timeline. A first cut Kerf proposes stays in the conversation, as the picture on its
card, until that is pressed. After that, or once a pane is opened or closed by hand,
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

**The sidebar** (`ui/project_sidebar.h`, ⌥⌘S; its choice and its width are kept)
leads with New conversation and a search, then lists the recent project folders
that still exist. Each folder unfolds to its own conversations, latest first under
a guide: the open one's as the bridge lists them (`.kerf/conversations/`), another's
read from the same folder beside its project. The open folder starts unfolded and
the others folded; a folder's name or its chevron folds and unfolds it (Left and
Right from the keys), and that is kept. The first six show, with "Show N more"
for the rest. A title has its row to itself, starting under its folder's icon,
with its age in its tip: a band of light passes over it while Kerf works or makes
something for it, and a dot at its end says it is done while you looked
elsewhere, waits for your review, or failed (until you look at it). Hovering one
of the open folder's shows ⋯, its menu (Rename… (F2), Archive, Delete…), and
archived ones fold under "Archived · N". Delete asks first. A folded folder has
the light on its name while Kerf works in it. Another folder's
conversation opens its project on that conversation. Hovering a folder shows + (a
new conversation in it) and its menu, which a right-click opens too: New
Conversation, Open Project, Show in Finder, Copy Path, and Remove from Sidebar,
which takes it off the list and touches nothing on disk. The + beside Projects is
New Project… and Open Folder…. At its foot is
the account: the Kerf account when one is signed in, else what chat runs on.
It opens the menu to everything the window does not show: Settings…, Account…, the
User Guide, Media (⌘1), Import… and View (the panes, the mask tools,
safe zones, viewer zoom, the keyboard's choice and the sync's rows), and Sign Out
when signed in. Every row is a registry command.

What is left of the toolbar is the top of the room, in its colour, with no line
under it: the traffic lights' room and the sidebar's top, the sidebar's button, the
title, Export (always there; off until the program has a clip), and one button for the panel (the
picture and the timeline together; ⌥⌘P and ⌥⌘T still take each alone). While the
sidebar is away, a ⋯ button beside its button opens the account's menu. The Media
sheet's button, Follow Kerf and the two format icons are in the timeline's strip,
before its zoom. The undo, redo, split, delete and import commands have their keys
and native menu items.

**Settings** (⌘, or the sidebar's gear) opens over the window, as everything one
goes into does: a panel in the middle of the dimmed window (`ui/modal_scrim.h`,
`StageShell::setCover`), Account, Chat, Models, Appearance, General and MCP
named down its side with their glyphs; Esc, its close or a press on the dim
gives the window back. Chat is the model the
conversation runs on, a key of one's own with its fields under that choice,
and how far Kerf goes without asking, the same choice as the composer's chip.
Models lists the models a recipe's `generate` can name, read from the
person's own models file (nothing is run to learn them), and says what runs
inside Kerf; it names no model of its own. Appearance
chooses the faces text is set in (`editor_theme.h`, `setEditorFaces`): the
interface's (Inter, with Pretendard for Hangul; Pretendard; IBM Plex, with its
own Hangul and mono; or the system's), and
what is said in the conversation, which may be a serif (Noto Serif, with Nanum
Myeongjo for Hangul) at one of three sizes. They are compiled in
(`resources/fonts`), kept under `appearance/`, and put in force at once.

There is no status bar. Beside the title the toolbar says what one would, and only
when there is something to say: a passing message for its timeout, an offline or
error state until it clears, a running sync's stage or a render's progress ("Draft
export · 42%"). "Edited 2 min ago · rev 22" is the title's tooltip. An
agent that cannot take requests (no key, signed out of Kerf) says so in the
composer's placeholder, with the reason on its tooltip. The
connection, error and project.kerf bars that carry an action lie over the top of
the content while they show.

**The Media band.** Media (the bin and roles) is a band 152 tall across the
panel, between the picture and the timeline, with a close button at the end of
its strip: its tiles are one row that goes along (a wheel turned down goes along
it too), and Roles is a column at its end. The timeline gives it its top, and the
picture its foot only when the timeline would be left under 200, so it covers
neither what is said nor the picture, and what is dragged from it has only a
little way to go. Its command (⌘1, or the grid in the timeline's strip) toggles
it, opening the panel if it is closed, and Esc closes it. Selecting a clip opens
nothing by itself.

**The picture.** Masks, annotations, captions and overlays lie on the frame at
its size. While a proposal is being previewed, a chip 12 points inside the
picture's top left reads "Previewing proposal vN" with Exit preview. The agent
HUD stands in the picture's title-safe area while a preview renders.

**The timeline.** Heads are 140 wide: the target button and the name; lock, hide,
mute and solo appear on hover over that head, and sync lock is in its context
menu. Under a wide sequence the default heights fit the lanes: V1 40, V2 28, each
track above them 20, sound 24. Vertical sequences keep 36 and 32. Proposal bands
stay on the ruler.

**The conversation** is always open, 560 wide beside the picture; ⇧⌘J puts the
keys in its composer, and with no timeline open the keys start there. Under its
text the composer says what a request goes with (`ui/composer.h`): how far Kerf
may go without asking (Ask before edits, Accept edits, Auto; its chip opens the
three, and Shift-Tab in the text goes round them), the model that answers (a menu
where there is a choice), the playhead's stamp only while its time is not sent (a request goes with it while a
picture is open; the conversation's menu takes it off), and a ring
for how full Kerf's context is once a turn has said (`agent_context`). The mode is
sent with each request (`permissions`) and kept under `agent/permissions`. Its
header is the
conversation's title (double-click renames it) and a ⋯ menu with Rename, Archive,
Delete…, Agent settings…, Model and Activity. Messages and proposal cards fill
its column at every width.

Settings from before this layout (saved workspaces, the panel's width, device
previews) load without error and are ignored.

## The conversation

A conversation reads top to bottom, as a coding agent's chat does: what you
asked as a bubble at the right (its timecode, when stamped, seeks), and Kerf's
answer flush left, its Markdown as rich text. While Kerf works, "Working" with
the time so far and the step it is on, and the last steps under it on a
hairline; done, it folds to "Worked for 1m 12s · 6 steps" (`ui/chat_messages.h`).
The bridge keeps each conversation beside the project (`.kerf/conversations/<id>.json`,
ignored by Git): the request when it is asked, then Kerf's answer with its
steps, how long it worked, the proposal it staged and whether it failed or was
stopped, and sends the conversation again (`conversation`) and the list
(`conversations`). A session new to a conversation hears its earlier turns.
The editor asks for them with `conversations {action: list | open | archive |
rename | delete}` (`ui/conversation_model.h`, `ui/wiring/chat_wiring.h`).
A proposal's card sits under the answer that staged it; one staged elsewhere,
by an agent over MCP or in another conversation, waits at the end of the open
one. A new message keeps the messages already shown and makes only its own
(`whatAConversationCosts` measures it).

Links: `kerf://<project>/rev<N>@HH:MM:SS:FF` opens a project at a time.
`<project>` is the slug of the project's folder name (lower case, each
run of anything but letters and digits one `-`: "Atlas Cut" is `atlas-cut`),
whatever the document calls itself, so a link finds a project that is not
open; a name outside ASCII travels in its punycode form and is read back. On macOS the app registers the `kerf` URL scheme, so a clicked link
opens (or raises) the project's editor and seeks to the timecode; `kerf-qt
kerf://atlas/rev22@00:00:17:08` does the same from a shell. A project this
Mac has not opened is refused with a message, never guessed. Windows and
Linux start an editor for each link followed, so one started for links alone
hands them to the editor already open and ends, and the link raises that
editor as on macOS; with none open, or none answering within two seconds, it
opens them itself (`link_inbox.cpp`). The inbox is a local socket that only
the user's own processes can reach, named by an id the user's settings keep
(`links/inbox`), in the user's runtime folder on Linux. A run that captures
or quits on its own neither hands links over nor takes them.

The bridge also gives the editor (see `bridge.h`): the program meter
(`meter`, L/R peak and RMS with BS.1770 momentary and short-term loudness)
and dropped frames (`clock.dropped`), solo monitoring, effect and generator
previews with parameters, the full QC `check` (loudness, true peak, gamut,
luma, title safe, caption overlap and timing, flash and black), programme
R128 `loudness`, a render queue with ProRes 422 HQ masters, paperwork
(captions as SRT or VTT), media
information and the media index, and looks saved and applied as
`.kerflook` files. The editor itself asks for the checks (Sequence ▸ Run QC,
listed under Preflight in the Export menu) and for renders; no screen of it
shows the meter or the queue, or writes the paperwork. A project's master
output (folder, name, codec) is kept per project, keyed by its absolute path.

## Projects

A project is a folder: `project.kerf`, a `media/` folder, and a `presets/`
library. Started without one, Kerf opens the last (above). A new conversation's New
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

Beside its title the toolbar shows what a running sync is doing, an unfinished merge
or a failed sync, and the account's menu ▸ View ▸ Sync says how far the branch is ahead of and
behind its upstream, with Sync Now. After a sync, each incoming commit
(`jun · Trimmed c3`) and each change that kept yours is listed in the activity
log; errors use the error
banner. When `project.kerf` changes outside Kerf (a `git pull` in a
terminal, a text editor), a banner offers Load from Disk or Keep Editor
Version.

File ▸ Save Current as Preset… adds the program sequence to the project's
`presets/`, which travels with the repository. Right-click media in the Project bin to Replace Media… (swap the
file behind the media for everyone; its clips keep their timing), Reveal in
Finder, or Consolidate Media. Bin items show Offline when the file is
missing, Not downloaded for a Git LFS file a sync has not fetched, and
External when it lives outside the project folder.

## A shot's graph

A shot a recipe makes ([docs/shots.md](../../docs/shots.md)) opens as a graph of
how it is made: Open Graph on the media in the bin or on its clip's menu, or a
double-click on its clip. It opens over the window, as Settings does
(`ui/recipe_graph_panel.h`), and Esc, its close or a press on the dim gives the
window back.

The graph is the recipe, read by the engine and laid out (`recipe_get` answers
`recipe`, from `kerf_engine::recipe::graph`): a card for each step, a `render`
that draws a guide from a sequence or a `generate` that asks a model, and for
each thing of the project a step reads, a sequence, a picture, a signal clip,
the way a clip goes (`moves=`), one of the cast; and an edge for each reading,
ending in the shot's takes. A card has an icon and its name, a quiet line under
it, and a tag that says what it gives; a signal's card draws its curve and a
moving clip's its path. Edges are in the colour of what goes along them
(`tokens::nodes`: a picture in motion the accent's ink, a still sage, the way a
thing goes the playhead's violet, a number over time ochre), named in the
legend at the canvas's foot. What a model has said it has no use for goes to it
dashed.

The canvas is free-form (`ui/recipe_graph.h`): a card stays where it is dragged,
kept per shot in the editor's settings on this computer and never in the recipe;
a drag on the canvas moves it, two fingers pan, a wheel or a pinch zooms about
the pointer, and Fit (F, or a double-click on the canvas) takes the whole graph
in. Until a card is moved they stand in columns by how far each is from what it
reads. The arrows go from card to card.

Nodes are wired by hand. A press on a node's port, where its edges leave it,
draws an edge from it; let go over a step that could read it, the step's line
gets the word that does: a signal as `signal=`, the way a clip goes as `moves=`,
a sequence as what a guide draws, a file a step made as `{name}` under `guide=`,
`first=`, `last=` or `ref=`, asked for in a menu where more than one name would
read it, with what the model has said it has no use for marked. A step reads
only what a step before it made, so an edge cannot be drawn backwards. An edge
is let go in the node's card, under Reads.

The card at the canvas's right says and sets what the chosen node holds
(`ui/recipe_inspector.h`): a model's prompt, its seed (and another at a press),
how long and how large, each knob its line sets or its model has said it takes,
`name=value` for any other, and what it reads, each let go at a press; a guide's
sequence, what of it is drawn, its size and rate; a signal's curve, drawn anew by
dragging across it; the takes, the one the shot is ticked, another chosen at a
press. With no node chosen it says the shot: why it cannot be made where it
cannot, and what a making would note first.

A change to a step is one word of one line of the recipe (`recipe_edit {media,
was, line, key, nth, value?}`): the engine sets that word where it stands
(`kerf_engine::recipe::text`), so comments, order, spacing and line endings stay,
reads the result as a recipe before writing it, and refuses a file another
program changed since the graph was read. It is the person's own edit, written in
place as a text editor would; an agent's recipe is still kept as a version. A
signal's curve is the clip's `value` in the project, one undoable edit, as the
timeline's own drawing of it is. The head makes the shot as the bin does: Make
(Make again once it has a take) shows what would run and asks, Several makes two
to four takes with each seed one on, Stop ends a making, and its takes card wears
the stripes of what is being made.

A model whose entry in the person's models file runs `kerf-comfy` has a workflow
behind it, and its node opens into it: Open workflow in its card, or a
double-click on it (`workflow_get {media, line}` answers `workflow`, read by
`kerf_workflow::detail` from the workflow that comes with the program, the file
the entry names, or the shot's own copy). The same canvas then shows the
ComfyUI graph: a card for each node with a row for each input, a link coming in
at its row, a blank the recipe fills (`{prompt}`, `{steps}`) marked in the
accent's ink, a value the workflow sets itself plain; the legend says the three.
The node's card sets any value that is neither a link nor a blank: a text, a
number (which stays one), a yes or no. With no node chosen it says where the
workflow comes from and what fills each blank, with what the line gives for it
now. The head's arrow goes back to the shot's graph.

A value set there is kept in the shot's own copy of the workflow
(`workflow_edit {media, line, was, node, input, value}`): the first copies the
workflow as it comes into the project beside the recipe
(`<shot>.<model>.comfy.json`) with that value changed and nothing else, and adds
`workflow=<file>` to the step's line; the next changes that file. The model's
entry is untouched, so other shots are made as before, and Use it as it comes
takes the word off the line. ComfyUI is never linked or changed: the editor
reads workflow files, and `kerf-comfy` asks the server over HTTP when the shot
is made. A model any other program runs has no workflow to open, and its node
says nothing of one.

`--graph <media>` starts a run on that shot's graph, and `--graph-node <id>` with
a node chosen (`workflow:<line>` opens the workflow of the model on that line of
the recipe), for a capture (`--screenshot`).

Restore take's recipe, in the takes' card, shows for the take the shot is
when what it was made from has changed and the take kept its recipe (`kept` on
the take); it sends `recipe_edit {restore: <take>}`, which the bridge answers
by writing the recipe as that take was made from it
(`kerf_shots::takes::restore`).

Add step, in the head, sends the kind of step wanted (`recipe_edit {add:
guide|model|join}`): the bridge writes the line (`kerf_shots::graph::step`)
and answers with the graph and the line it added (`added`), whose card the
wiring opens. Delete step, at the foot of a step's card, sends its line
(`recipe_edit {line, remove: true}`). Both are changes to the recipe's text,
refused as a making would refuse what they leave.

A `join` step, which sets the parts of a shot longer than a model makes at
once end to end, is a card of its own (`Kind::Join`), with its `lap`; a
guide's card has `From` and `Seconds`, the part of its sequence it draws, and
a model that is a part says where it starts. A picture in motion drawn to a
model can be its `first`: the shot then goes on from that one's last frame.

The takes' card shows each take's picture, and a guide's card the guide as it
was drawn for the take that shows (`made` on a step's node, a file in that
take's working folder), both decoded off the paint path by a `MediaArtwork` of
the panel's own, as the bin's posters are. Hold to guide asks the bridge to
read each take beside its guide (`takes_read`, answered with a `take_read` a
take as it is read, on a thread of its own, then `takes_read`), and each row
then says its number, the closest marked. Why a shot's takes cannot be held to
anything is said in the card and not in the window's banner.

What the graph shows and sets, an agent's tools read and set through the same
crate (`kerf-shots`: `look` with `graph` and `workflow`, `edit` with
`recipeSet` and `workflowSet`; [An agent's part](../../docs/shots.md#an-agents-part)).
An agent's change is a revision of the project, in which the media names the
recipe's next version: the open graph asks for the recipe again on any
revision (`GraphWiring::onSnapshot`), so it shows the shot as the agent left
it, with the node chosen still chosen and the cards where they were put, a
version being the same shot (`anAgentsChangeShowsInTheGraphThatIsOpen`).

## Masks

The Rectangle, Ellipse, and Pen mask tools sit in the Program panel's title
bar; they draw on the selected picture
clip. Drag out a rectangle or an ellipse on the picture; with the
pen, click each corner and click the first one again to close the outline,
or press Return to leave it open as a path (Escape drops it). A new mask is
named `k1`, `k2`, … and selected. While the clip shows masks, the monitor
holds its picture: drag a handle to move a corner or point, drag inside a
mask to move it, and press Delete to remove the selected one. Masks are
drawn where the clip's picture lands, after its crop, scale, rotation, and
position; a mask with mode `none` is dashed, a path that effects such as
Stroke and Reshape read. A mask's mode, inversion, feather, opacity and
expansion are Kerf's to set. Each change is one edit (`mask-add`, `mask-set`, or
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
struck through. Kerf's turns are the conversation's (above), and Stop ends
one. A pending proposal is a card under Kerf's answer: its
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
subtitle does ("9:16 · 1080 × 1920"), and any other
size as
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
chooses Kerf, the hosted model (the default unless a key of your own
is already saved; see
[docs/cloud.md](../../docs/cloud.md)), or My own API key. Kerf shows the
account, plan, and requests left, with Sign in (a token, for now) and Sign
out; Ask Kerf stays unavailable, with the reason, until the account can make
requests. My own API key
holds two protocol slots: Anthropic Messages and OpenAI-compatible. Each slot
has an API key, Base URL, and model (empty for the service's default). The
OpenAI-compatible slot also has Chat Completions vs Responses; other services
that speak that API use it with their own Base URL. Chat sends the active
connection and its model.
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
and its geometry, the conversation and the proposals, timeline, bin,
toolbar, status, the kit, theme, launcher,
wiring guards, kerf:// links), each fed bridge events shaped as kerfd sends them,
and `MockupParityTest` checks the whole editor against the design at 1600 × 1000
in 16:9 and 9:16, with each sheet and a preview.
With `KERF_SCREENSHOT_DIR` set, it saves each state under the mockup's query state
(`169.png`, `916-media.png`, `916-preview.png`, …), and the
region galleries save theirs (`editor-<format>-dark.png`,
the launcher's pages, the conversation's states).
The pictures in them are the tests' procedural footage (`test_footage.h`:
a harbor at dawn, a close-up on a dock and a drone pass over the coast,
drawn in code with texture and grain), so no media files are needed.
The macOS CI job also runs the real bridge/media workflow in
`scripts/test-kerf-qt-bridge.py`.
