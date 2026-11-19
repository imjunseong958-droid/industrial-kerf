# Editing with Kerf

This guide is for someone making a video with Kerf. The READMEs are for people
building Kerf itself. Help ▸ Kerf User Guide opens the copy of this guide that
comes with Kerf, so it needs no connection.

Kerf is a video editor you can talk to. You ask for an edit in words, Kerf
proposes it on the timeline, and nothing changes until you accept it. Everything
you do by hand is an ordinary edit too, and both are undone the same way.

## Get Kerf

Kerf runs on:

- a **Mac with Apple silicon** (M1 or later), with macOS 26 or later. Intel Macs are
  not supported.
- **Windows 10 or 11, x64.** Windows 11 on Arm can run it through Windows' own
  emulation of x64 programs; that has not been tried.
- **Linux, x86-64**, with X11 or XWayland and a system as new as Ubuntu 24.04
  (Ubuntu 24.04 or later, Debian 13 or later).

Download the latest release from
[GitHub](https://github.com/imjunseong958-droid/industrial-kerf/releases). For
Windows, run the installer (`…-windows-x64-setup.exe`): it installs Kerf for you
alone, with no administrator, adds it to the Start menu, and opens `kerf://`
project links in it; Settings ▸ Apps uninstalls it. Or unpack the zip anywhere and
run `kerf-qt.exe`, which does not open links. For a Mac with Apple silicon, unpack
the zip and move Industrial Kerf to Applications. For Debian 13, Ubuntu 24.04 and what
comes from them, `sudo apt install ./Industrial-Kerf-…-linux-x64.deb` in the folder you
downloaded it to installs Kerf with what it needs from the system, adds it to your
applications, opens `kerf://` project links in it, and puts `kerf`, `kerfd` and `kerf-mcp`
on your `PATH`; `sudo apt remove industrial-kerf` takes it away. For Fedora,
`sudo dnf install ./Industrial-Kerf-…-linux-x64.rpm` does the same, and
`sudo dnf remove industrial-kerf` takes it away. On any other Linux
(x86-64, as new as Ubuntu 24.04), or to keep it in a folder of your own,
unpack the `.tar.xz` anywhere and run `bin/kerf-qt`; it carries what it needs
beyond what every desktop has, which `system-libraries.txt` beside it lists. Run
`./install-desktop-entry.sh` in the folder once to have Kerf among your applications and
open `kerf://` project links in it, for you alone; run it again after moving the folder,
and `--remove` takes it away. Releases are not
signed yet, so the first run asks: on Windows, More info ▸ Run anyway; on a Mac,
right-click the app ▸ Open. Help ▸ About
Kerf says which version you have. Once a day Kerf asks GitHub whether a newer one
is out and offers its page; Settings (⌘,) ▸ General turns that off.

## Start a project

A project is a folder: `project.kerf` (the edit, as text), `media/`, and
`presets/`.

- **Open Kerf.** It opens on the project you last worked on. A new conversation
  lists your recent folders, with **Open Folder…** and **New Project…** under
  them; the first time, with no project to go back to, that is what you see.
  A folder with no `project.kerf` becomes a project, with the media in it
  imported.
- **New Project…** (File ▸ New Project…, ⌘N) offers Empty 16:9 and Vertical 9:16,
  a name, a location, a frame rate, and whether to share it with Git or GitHub.
  **From Preset** fills a shared preset's slots with your files. **Clone from
  GitHub** copies a project someone shared.
- **Open Project…** (⌘⇧O) opens any folder; **Open Recent** lists the latest.

Import media with File ▸ Import Media… (⌘O).

## The window

- **Conversation** on the left: you and Kerf, one conversation at a time, read
  top to bottom.
- **Picture** and **timeline** at the right, once there is a cut to show.
  ⌥⌘P and ⌥⌘T open and close them.
- A first cut Kerf makes shows in the conversation as a card with its picture.
  While Kerf works, light runs around the picture as each clip is placed; it
  lights once when the cut is done. Move over the picture to run through its clips; click it to open it
  with the timeline. Closing them puts it back in the card.
- **Media** (⌘1) opens between the conversation and the picture: the project's
  media, with Import. The conversation makes room for it.
- The format picker in the timeline's strip switches the sequence between 16:9
  and 9:16 (⌥1, ⌥2).
- **Settings ▸ Appearance** chooses the fonts: the interface's (Inter,
  Pretendard, IBM Plex, or your system's) and the conversation's, which can be a serif
  and larger or smaller. They change as you choose.
- The **sidebar** (⌥⌘S) has New conversation, a search, your project folders,
  each with its own conversations under it, and at its foot your account, which
  opens Settings and everything else. Drag its edge to make it wider or
  narrower; a double-click on the edge puts it back.

## Ask Kerf

Ask in the conversation, or about one clip: select it and press ⌘J (Clip ▸ Ask
Kerf…), Option-double-click it, or right-click ▸ Ask Kerf…. With a clip, choose
**This clip** (only it changes; clips after it follow) or **Whole sequence**.
Return sends; Shift+Return adds a line. A request goes with the playhead's
time while a picture is open; its timecode in your message goes back there. To
send one without it, untick **Send playhead time** in the conversation's menu
(its name in the toolbar); a struck clock under the composer says it is off,
and pressing it puts the time back.
Beside it the composer names the model that answers, with a menu where there
is a choice, and a ring fills as the conversation takes up Kerf's context;
when it is full Kerf puts older steps away and goes on.

While Kerf works, **Working** under your request shows how long and the step it
is on, light runs round the picture, a blue cursor on the timeline shows where
it is, and **Stop** ends the turn. Once done it folds to "Worked for 1m 12s · 6
steps", which opens to every step. With **Follow Kerf** on (⌥⌘F, the eye in the
timeline's strip) the picture follows it. A turn that failed or was stopped
says so, with **Ask again**.

Kerf answers with a **proposal**, a card under its answer:

- The timeline shows it before anything changes: added or changed clips are
  outlined where they would land, removed ones struck through.
- **Preview** plays it; **Watch what changed** plays only the stretches it
  touches.
- **Accept** (⌘Return) applies it; **Reject** (Escape, with the timeline
  focused) drops it. **Versions** appear once Kerf has revised it.

A proposal that has gone stale, because the edit changed under it, cannot be
accepted: ask Kerf to stage it again.

### How far Kerf goes without asking

The chip at the composer's start chooses, and Shift+Tab in the composer goes
round the three. Settings ▸ AI connection has the same choice.

- **Ask before edits**: every edit is a proposal you accept or reject, as
  above. This is how Kerf starts.
- **Accept edits**: edits go into the sequence as Kerf makes them. Each is one
  step that Undo (⌘Z) takes back. A shot Kerf wants made is still yours to run.
- **Auto**: edits go in, and Kerf also makes the shots it asks for: five, then
  it asks again, unless `~/.kerf/agent.json` gives another number (`makes`).
  Making a shot can take time and, with a hosted model, money.

The choice is kept for every project. It holds for what you ask in the editor;
an agent that reaches Kerf over MCP from another program follows
`~/.kerf/agent.json`.

### Connecting Kerf

Settings (⌘, ; or the account at the sidebar's foot ▸ Settings…) ▸ AI connection
chooses how Kerf thinks, and ▸ Account signs in to your Kerf account:

- **Kerf**, the hosted model: sign in, and the account, plan and requests
  left show there. The sign-in is kept in your system's keychain, as keys are.
- **My own API key**: an Anthropic or OpenAI-compatible key, base URL, and model.
  Keys are kept in your system's keychain (macOS Keychain, Windows Credential
  Manager, or the Secret Service on Linux), or in Kerf's settings where there is
  none.
- **Claude Code**: Kerf runs your installed Claude Code for each turn, on your
  own account.

## Edit by hand

- **Timeline.** Drag clips to move them and drag their ends to trim. The Edit
  menu has Split at Playhead, Delete and Ripple Delete, and Extract, which takes
  the clip's time out of every track where Ripple Delete would cut a clip on
  another; the timeline also has a Razor and a Ripple Edit tool. Edit ▸ Undo
  and Redo cover every edit, Kerf's included.
- **Media into the timeline.** Double-click media in the Media sheet, or choose
  it and use Clip ▸ Insert or Overwrite: the whole of it goes in at the
  playhead, on the tracks the timeline's heads target. Trim it there.
- **Everything finer** (a clip's position, scale and crop, its sound, its
  effects, titles) is Kerf's to do: ask in the conversation and it proposes the
  change for you to accept.
- **Masks.** The Rectangle, Ellipse and Pen tools in the picture's title bar draw
  on the selected clip. Drag a handle to reshape, drag inside to move, Delete to
  remove.
- **Keyboard shortcuts.** Edit ▸ Keyboard Shortcuts chooses a layout.

## Shots a renderer or a model makes

A shot Kerf does not draw itself, a 3D render or a video model's, comes in as
media with the steps that make it written under it in the project. Ask Kerf
for the shot and it adds the media and writes its steps; whether it then
makes the shot without asking you is yours to say
([shots.md](shots.md#an-agents-part)), and by default it asks. Right-click
the media in the Media sheet and choose Make…, read the steps it shows, and
say yes; or run `kerf make <media>` in a terminal.

To see how a shot is made and change it, right-click the media and choose Open
Graph, or double-click its clip on the timeline. Its steps open as a graph:
the guide drawn from your sequence, the models, and what they are handed, wired
together, ending in the shot's takes. Choose a node to set its prompt, its seed
or a knob, or to draw a signal's curve, and drag from a node's port to a model
to hand it over; what you change is an edit of the project, which undo takes
back, and Make again makes a new take from it. A model that runs on a ComfyUI server opens further:
double-click it to see the workflow it is run with, node by node, and set any
value in it. Kerf keeps your change in a copy of the workflow in the project,
for that shot alone; nothing on the server changes
([shots.md](shots.md#a-workflow-of-the-shots-own)).

Each making is kept as a take, and the newest is the one in your cut. `kerf
takes <media>` lists them and `kerf takes <media> --use 1` goes back to one,
and you can ask Kerf to do the same. Trim, move, grade and title a made shot
as you like: none of that asks for it again. Kerf says when a shot does want
making again, and for what: its steps, a file they read, or the
sequence that guides it changed.

To keep the same people, places, and look from shot to shot, say them once:
right-click in the Media sheet and choose Cast and Styles…, and give each a
name, a few words, and pictures of it. A shot's steps name them, and when you
change one, saving says which shots want making again
([the cast](shots.md#the-cast)). For several takes of a shot to choose among,
choose Make Several on the media, or run `kerf make <media>
--rolls 3`; pick one under Takes. `kerf shots` lists every such shot with
its take and whether it wants making.

Kerf comes with no model. To have a shot ask one, name the program that
reaches it in `~/.kerf/models.json`; [shots.md](shots.md) has the format and
what the program is handed.

A project made before the steps were written in the project keeps them in
recipe files (`.kerfrecipe`). It opens as it did, and a shot is made again
once its steps are in the project: right-click the media and choose Carry
Steps into the Document…, or run `kerf carry <media>`
([shots.md](shots.md#carrying-a-recipe-into-the-document)).

## Conversations

Every conversation is kept beside the project (`.kerf/conversations/`), with
what Kerf did in each turn, and Kerf hears the earlier turns when you go on in
one.

- **New conversation** starts another; a conversation is titled by its first
  request, and double-clicking the title renames it.
- Hover a conversation in the sidebar to **archive** or **delete** it; its menu
  (right-click) also renames. Archived ones fold under **Archived**, and come
  back when you write in them. Deleting asks first and cannot be undone.
- Every folder in the sidebar unfolds to its conversations, the latest first:
  click its name or its arrow. The first six show, with **Show more** for the
  rest. A conversation in another folder opens that project on it.
- Hover a folder for **+**, a new conversation in it, and **⋯**, its menu:
  Open Project, Show in Finder, Copy Path, and Remove from Sidebar, which only
  takes it off the list.
- A proposal staged elsewhere, by an agent over MCP or in another conversation,
  waits at the end of the open one, to accept or reject.
- A `kerf://` link to a version and a timecode (`kerf://sea/rev22@00:00:17:08`)
  opens the project at that time. On macOS,
  on Windows with the installer, and on Linux with the `.deb` or once
  `install-desktop-entry.sh` has run, a clicked link opens the project there, in the Kerf you already have open
  if there is one.

## Share through Git

The Collaborate menu works on a project shared through Git and GitHub:

- **Sync Now** commits your edits, pulls, merges, and pushes. `project.kerf`
  merges clip by clip; where both sides changed the same thing, yours is kept and
  the sync says so.
- **Live Sync** syncs every few seconds while you work.
- **Commit…**, **New Branch…**, **Switch Branch**, **Open Pull Request…**,
  **Publish to GitHub…**, and **Consolidate Media**, which copies media from
  outside the project into `media/` so collaborators have it.

When `project.kerf` changes outside Kerf, a banner offers **Load from Disk** or
**Keep Editor Version**. From a terminal, `kerf load` is Load from Disk, and
`kerf proposals`, `kerf accept` and `kerf reject` answer what an agent
proposed, with no editor open.

## A brand

When what you make is for one name, say its colors, fonts, and words once, in
`brand.kerfbrand` in the project's folder:

```text
kerf-brand 1
color ember #c2410c
color ink #1f2937
font heading "Fraunces"
font body "Inter"
tone "warm and plain, never selling"
say "handmade" "small batch"
avoid "cheap" "discount"
logo media/logo.png
```

A line is `color <name> #rrggbb`, `font <what it is for> "<family>"`, `tone
"<how it speaks>"`, `say` and `avoid` with words in quotes, or `logo` with a
picture of the project; each is there only if you want it. Kerf reads it with
the project whenever you ask for a title or captions, and sets them in those
fonts and colors and in that voice. A check then says, as information that
fails nothing, where a title is set in a font or a color the brand has not,
and where a title or a caption says a word it avoids, as a word of its own:
"cheaper" is not "cheap". A title that sets no font or color of its own is
left alone, and nothing is changed for you.

## Check and export

- **Sequence ▸ Run QC** runs the checks a delivery needs: loudness, true peak,
  gamut, title safe, caption overlap and timing, flash, black, and a picture
  and its sound more than a frame out of step. What it finds
  is listed under Preflight in the toolbar's Export menu, where Ask Kerf on a
  failed row asks Kerf to fix it; the conversation offers each failed row as
  something to ask.
- **Export** in the toolbar renders a Draft (H.264) or a Master (ProRes 422 HQ
  with 24-bit sound; ⌘E). Both go to `deliveries/` in the project, as
  `<project>_master` and `<project>_draft`; a draft made again is numbered.
- **Presets.** File ▸ Save Current as Preset… keeps the program as a preset
  others can fill with their own media (File ▸ Import Editing Preset…).

## When something goes wrong

- **Kerf stopped unexpectedly.** Each crash of Kerf's project service (kerfd),
  the editor, or the `kerf` command leaves a report on this computer, whether it
  was in Kerf's own code, in Qt, or in FFmpeg; nothing is sent anywhere. The next time Kerf opens it says so, and Help ▸ Show Crash
  Reports opens the folder. Attaching a report to a bug report shows where it
  happened.
- **Reconnect.** If the connection to the project drops, the banner offers
  Reconnect.
- **Offline media.** Media shows Offline when its file is missing, and Not
  downloaded when a sync has not fetched it yet. Right-click ▸ Replace Media…
  points it at another file; its clips keep their timing.
