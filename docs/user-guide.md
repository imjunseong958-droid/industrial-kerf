# Editing with Kerf

This guide is for someone making a video with Kerf. The READMEs are for people
building Kerf itself. Help ▸ Kerf User Guide opens this page.

Kerf is a video editor you can talk to. You ask for an edit in words, Kerf
proposes it on the timeline, and nothing changes until you accept it. Everything
you do by hand is an ordinary edit too, and both are undone the same way.

## Get Kerf

Download the latest release from
[GitHub](https://github.com/imjunseong958-droid/industrial-kerf/releases): for
Windows, unpack the zip anywhere and run `kerf-qt.exe`; for a Mac with Apple
silicon, unpack it and move Industrial Kerf to Applications. Releases are not
signed yet, so the first run asks: on Windows, More info ▸ Run anyway; on a Mac,
right-click the app ▸ Open. Help ▸ About
Kerf says which version you have. Once a day Kerf asks GitHub whether a newer one
is out and offers its page; Edit ▸ Settings… ▸ Updates turns that off.

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
- **Picture** and **timeline** at the right, once there is something to show.
  ⌥⌘P and ⌥⌘T open and close them.
- **Sheets** open over the conversation from the Window menu: Media (⌘1),
  Source Monitor, Inspector (⌘4), Effects and titles (⌘5), and Presets.
- **Trays** under the toolbar: **Finish** (checks and fixes before delivery) and
  **Deliver** (masters and paperwork).
- The toolbar's format picker switches the sequence between 16:9 and 9:16.

## Ask Kerf

Ask in the conversation, or about one clip: select it and press ⌘J (Clip ▸ Ask
Kerf…), Option-double-click it, or right-click ▸ Ask Kerf…. With a clip, choose
**This clip** (only it changes; clips after it follow) or **Whole sequence**.
Return sends; Shift+Return adds a line.

While Kerf works, its steps fold under its reply, a blue cursor on the timeline
shows where it is, and **Stop** ends the turn. With **Follow Kerf** on (⌥⌘F, the
toolbar's eye) the picture follows it.

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

Edit ▸ Settings… (⌘, ; on macOS in the Kerf menu) ▸ AI chooses how Kerf thinks:

- **Kerf Cloud**, the hosted model: sign in, and the account, plan and requests
  left show there.
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
- **Source Monitor.** Open a clip from Media, mark in and out, then Insert or
  Overwrite at the playhead.
- **Inspector** (⌘4). Click a clip and the Inspector shows its controls: its
  picture's position, scale and crop, its sound, and its effects in order. Esc,
  or a click on an empty lane, goes back.
- **Effects and titles** (⌘5). Cards show the selected clip through each effect.
  Double-click a card or press Return to add it, or drag it onto the Inspector to
  put it where it lands. **Titles** has a card for each generator.
- **Masks.** The Rectangle, Ellipse and Pen tools in the picture's title bar draw
  on the selected clip. Drag a handle to reshape, drag inside to move, Delete to
  remove.
- **Keyboard shortcuts.** Edit ▸ Keyboard Shortcuts chooses a layout.

## Review with others

Every thread in the conversation is a comment kept beside the project
(`review/comments.jsonl`), so notes travel with it and never get in the way of an
edit.

- Add a note at the playhead, draw on the frame, @mention someone, and resolve a
  thread when it is done. Filter by open threads, proposals, author or tag.
- **Blocking notes** hold delivery back: Deliver, the toolbar's Export with
  Master, and an agent's full render all refuse while one is open, and say which
  note and where. A Draft still exports.
- **Copy link** gives a `kerf://` link to a version and a timecode. On macOS a
  clicked link opens the project there.

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
**Keep Editor Version**.

## Finish and deliver

- **Finish** runs the checks a delivery needs: loudness, true peak, gamut, title
  safe, caption overlap and timing, flash, and black. A blocking finding becomes a
  note, resolved once a later check no longer finds it.
- **Deliver** renders a master (ProRes 422 HQ with 24-bit sound) and its
  paperwork: a comments CSV, markers, a CMX3600 EDL, and SRT or VTT captions. It
  keeps its folders and choices per project.
- **Presets.** File ▸ Save Current as Preset… keeps the program as a preset
  others can fill with their own media (Window ▸ Presets).

## When something goes wrong

- **Kerf stopped unexpectedly.** Each crash of Kerf's project service (kerfd),
  the editor, or the `kerf` command leaves a report on this computer; nothing is
  sent anywhere. The next time Kerf opens it says so, and Help ▸ Show Crash
  Reports opens the folder. Attaching a report to a bug report shows where it
  happened.
- **Reconnect.** If the connection to the project drops, the banner offers
  Reconnect.
- **Offline media.** Media shows Offline when its file is missing, and Not
  downloaded when a sync has not fetched it yet. Right-click ▸ Replace Media…
  points it at another file; its clips keep their timing.
