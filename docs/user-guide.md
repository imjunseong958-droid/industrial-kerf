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
review links in it; Settings ▸ Apps uninstalls it. Or unpack the zip anywhere and
run `kerf-qt.exe`, which does not open links. For a Mac with Apple silicon, unpack
the zip and move Industrial Kerf to Applications. For Debian 13, Ubuntu 24.04 and what
comes from them, `sudo apt install ./Industrial-Kerf-…-linux-x64.deb` in the folder you
downloaded it to installs Kerf with what it needs from the system, adds it to your
applications, opens `kerf://` review links in it, and puts `kerf`, `kerfd` and `kerf-mcp`
on your `PATH`; `sudo apt remove industrial-kerf` takes it away. For Fedora,
`sudo dnf install ./Industrial-Kerf-…-linux-x64.rpm` does the same, and
`sudo dnf remove industrial-kerf` takes it away. On any other Linux
(x86-64, as new as Ubuntu 24.04), or to keep it in a folder of your own,
unpack the `.tar.xz` anywhere and run `bin/kerf-qt`; it carries what it needs
beyond what every desktop has, which `system-libraries.txt` beside it lists. Run
`./install-desktop-entry.sh` in the folder once to have Kerf among your applications and
open `kerf://` review links in it, for you alone; run it again after moving the folder,
and `--remove` takes it away. Releases are not
signed yet, so the first run asks: on Windows, More info ▸ Run anyway; on a Mac,
right-click the app ▸ Open. Help ▸ About
Kerf says which version you have. Once a day Kerf asks GitHub whether a newer one
is out and offers its page; Settings (⌘,) ▸ General turns that off.

## Start a project

A project is a folder: `project.kerf` (the edit, as text), `media/`, and
`presets/`.

- **From the start window.** Opened without a project, Kerf shows your recent
  folders and one question. Pick a folder, say what to make ("a 30-second cut of
  the best moments"), and send. A folder with no `project.kerf` becomes a project,
  with the media in it imported. **Open without asking** opens the folder as it is.
- **New Project…** (File ▸ New Project…, ⌘N) offers Empty 16:9 and Vertical 9:16,
  a name, a location, a frame rate, and whether to share it with Git or GitHub.
  **From Preset** fills a shared preset's slots with your files. **Clone from
  GitHub** copies a project someone shared.
- **Open Project…** (⌘⇧O) opens any folder; **Open Recent** lists the latest.

Import media with File ▸ Import Media… (⌘O).

## The window

- **Conversation** on the left: you and Kerf, and every comment on the project.
- **Picture** and **timeline** at the right, once there is a cut to show.
  ⌥⌘P and ⌥⌘T open and close them.
- A first cut Kerf makes shows in the conversation as a card with its picture.
  While Kerf works, light runs around the picture as each clip is placed; it
  lights once when the cut is done. Move over the picture to run through its clips; click it to open it
  with the timeline. Closing them puts it back in the card.
- **Media** (⌘1) opens over the conversation: the project's media, with Import.
- The format picker in the timeline's strip switches the sequence between 16:9
  and 9:16 (⌥1, ⌥2).
- The **sidebar** (⌥⌘S) has New conversation, your project folders and their
  conversations, and at its foot your account, which opens Settings and
  everything else.

## Ask Kerf

Ask in the conversation, or about one clip: select it and press ⌘J (Clip ▸ Ask
Kerf…), Option-double-click it, or right-click ▸ Ask Kerf…. With a clip, choose
**This clip** (only it changes; clips after it follow) or **Whole sequence**.
Return sends; Shift+Return adds a line.

While Kerf works, its steps fold under its reply, a blue cursor on the timeline
shows where it is, and **Stop** ends the turn. With **Follow Kerf** on (⌥⌘F, the
eye in the timeline's strip) the picture follows it.

Kerf answers with a **proposal**, a card in its thread:

- The timeline shows it before anything changes: added or changed clips are
  outlined where they would land, removed ones struck through.
- **Preview** plays it; **Watch what changed** plays only the stretches it
  touches.
- **Accept** (⌘Return) applies it; **Reject** (Escape, with the timeline
  focused) drops it. **Versions** appear once Kerf has revised it.

A proposal that has gone stale, because the edit changed under it, cannot be
accepted: ask Kerf to stage it again.

### Connecting Kerf

Settings (⌘, ; or the account at the sidebar's foot ▸ Settings…) ▸ AI connection
chooses how Kerf thinks, and ▸ Account signs in to Kerf Cloud:

- **Kerf Cloud**, the hosted model: sign in, and the account, plan and requests
  left show there. The sign-in is kept in your system's keychain, as keys are.
- **My own API key**: an Anthropic or OpenAI-compatible key, base URL, and model.
  Keys are kept in your system's keychain (macOS Keychain, Windows Credential
  Manager, or the Secret Service on Linux), or in Kerf's settings where there is
  none.
- **Claude Code**: Kerf runs your installed Claude Code for each turn, on your
  own account.

## Edit by hand

- **Timeline.** Drag clips to move them and drag their ends to trim. The Edit
  menu has Split at Playhead, Delete and Ripple Delete; the timeline also has a
  Razor and a Ripple Edit tool. Edit ▸ Undo
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
media with a recipe: a small text file in the project with the steps that make
it. Ask Kerf for the shot and it writes the recipe and adds the media; Kerf
never runs one itself. Right-click the media in the Media sheet and choose Make
from Recipe…, read the steps it shows, and say yes; or run `kerf make <media>`
in a terminal.

Each making is kept as a take, and the newest is the one in your cut. `kerf
takes <media>` lists them and `kerf takes <media> --use 1` goes back to one,
and you can ask Kerf to do the same. Trim, move, grade and title a made shot
as you like: none of that asks for it again. Kerf says when a shot does want
making again, and for what: its recipe, a file the recipe reads, or the
sequence that guides it changed.

Kerf comes with no model. To have a recipe ask one, name the program that
reaches it in `~/.kerf/models.json`; [shots.md](shots.md) has the format and
what the program is handed.

## Review with others

Every thread in the conversation is a comment kept beside the project
(`review/comments.jsonl`), so notes travel with it and never get in the way of an
edit.

- Add a note at the playhead, draw on the frame, @mention someone, and resolve a
  thread when it is done. Filter by open threads, proposals, author or tag.
- **Blocking notes** hold delivery back: the toolbar's Export with
  Master and an agent's full render both refuse while one is open, and say which
  note and where. A Draft still exports.
- **Copy link** gives a `kerf://` link to a version and a timecode. On macOS,
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

## Check and export

- **Sequence ▸ Run QC** runs the checks a delivery needs: loudness, true peak,
  gamut, title safe, caption overlap and timing, flash, and black. What it finds
  is listed under Preflight in the toolbar's Export menu, where Ask Kerf on a
  failed row asks Kerf to fix it. A blocking finding becomes a note in the
  conversation, resolved once a later check no longer finds it.
- **Export** in the toolbar renders a Draft (H.264) or a Master (ProRes 422 HQ
  with 24-bit sound; ⌘E). A master goes to `deliveries/` in the project.
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
