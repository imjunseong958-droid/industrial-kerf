# ADR 0007: An editor first, with AI on the selection

Status: accepted (phases 1–4 implemented)
Date: 2026-10-09

## Context

The Qt editor is a viewer and a timeline beside a permanent agent column
(Chat, Inspector, Settings). Its menus and shortcuts are hard-coded in
`main_window.cpp`, and it has no docking, workspaces, bins, source monitor,
or tool palette. The kernel underneath is already an NLE: exact time,
insert, overwrite, ripple, roll, slip, and slide (ADR 0006).

To be used as a professional editor, Cut must work as one with no AI at
all, and keep the habits editors bring from Premiere Pro, Final Cut Pro,
and Resolve. The AI is a feature that acts on what is selected, not the
centre of the window.

FilmCraft (storytold/filmcraft, MIT OR Apache-2.0) is the reference: a
clean-room Premiere workflow in Rust whose every action is a command in one
registry shared by menus, shortcuts, the CLI, and its MCP server. We take
its patterns and data, not its egui UI or its own codecs; Cut stays Rust
with a Qt presentation layer and FFmpeg.

## Decision

- **One command registry** (`cut-commands`). Each command has a stable id,
  label, menu and group, shortcuts per keymap, a parameter description, and
  an enabled predicate that names why it is unavailable. Engine commands
  resolve, in the current context, to the bridge request that carries them
  out; frontend commands (dialogs, playback, view) are carried out by the
  editor. Menus, shortcuts, the toolbar, and timeline gestures all go
  through it, and later the AI does too.
- **Keymaps are data**: Cut, Premiere Pro, and Final Cut Pro layouts over the
  same commands. A test refuses a keymap that binds one key twice.
- **Docked workspaces** (phase 2): Qt Advanced Docking System, the window
  split into panel classes, saved layouts. The agent column goes away; the
  inspector becomes Effect Controls and the agent settings move to
  Preferences.
- **Standard NLE interaction** (phase 3): bins, a source monitor with in and
  out marks, insert and overwrite, editing tools, snapping, and track lock
  and sync-lock toggles.
- **AI on the selection** (phase 4): a shortcut opens a prompt anchored to the
  selected clip, range, or playhead. The agent edits within that scope;
  other clips follow by the kernel's ripple and sync-lock rules rather than
  by the model. The proposal shows on the timeline and is accepted or
  rejected in place. Plain double-click keeps its NLE meaning (open in the
  source monitor).
- **Hosted model as the paid tier** (phase 5): the editor is free; the
  subscription is a hosted provider with server-side entitlement, since a
  client-side gate in an open-source app is not a gate.

## Phase 1

`crates/cut-commands` holds the registry. The Qt bridge exposes it through
three synchronous calls — `cut_commands_catalog(keymap)`,
`cut_commands_states(context)`, and `cut_commands_run({command, params,
context})` — that need no running bridge. The editor builds one `QAction`
per command (`apps/cut-qt/command_registry.*`), sends its context (connection,
revision, history, selection, playhead, preview state, track counts) to
enable them, and sends the requests engine commands resolve to. Given
parameters take the place of the context, so an agent can run
`clip.delete {"clip": "c3"}` without a selection.

The inspector's property edits and the focus-region crop still send typed
operations directly.

## Phase 2

The window is a dock manager (Qt Advanced Docking System 5.1.1, fetched at
configure time) holding four panels: Program (viewer and transport),
Timeline, Effect Controls (the former inspector, its own class), and Cut
Assistant (chat, activity, and proposal review). `Workspace` arranges the
built-in Editing layout — Program over the Timeline, with Effect Controls
and the Assistant tabbed beside the Program — and keeps named workspaces
in settings: each has a saved layout (Editing's is built in) and the changes
made to it, which persist across launches until it is reset. Panel
visibility, workspaces, and Settings are registry commands in a new Window
menu and in Edit. The provider and MCP settings moved from the agent column
into the Settings dialog. Until phase 4 the Assistant stays a dockable
panel so the agent remains reachable.

## Phase 3

Two panels join the workspace: a Project bin of the media and a Source
monitor that scrubs one piece of media frame by frame (decoded in the
background, latest request first) with in and out marks. The Editing
workspace follows the edit-suite arrangement — Source and Program over the
Project bin and the Timeline.

New registry commands carry the standard interaction:

- `source.insert` (`,`) and `source.overwrite` (`.`) resolve the marked
  range, the playhead, and the target tracks (the first unlocked track of
  each kind the media has) to the kernel's insert and overwrite, which link
  a picture and its sound.
- `source.markIn` (I), `source.markOut` (O), `source.clearMarks`, and frame
  steps are scoped to the source monitor; a key may mean different things
  in different focused panels, and the keymap test refuses only real
  collisions.
- Six tools (`tool.selection` V, `tool.razor` C, `tool.ripple` B,
  `tool.roll` N, `tool.slip` Y, `tool.slide` U) decide what a timeline drag
  becomes: a move, split, ripple trim, roll of the cut, slip, or slide, each
  its own kernel operation.
- `sequence.snap` snaps drags to clip edges, the playhead, and the start.
- `track.set` backs the track header toggles for output, mute, and lock.

Sync lock is not in the kernel yet; ripple edits already act on every
unlocked track. Drag and drop from the bin to the timeline and source
playback (J, K, L) are left for later.

## Phase 4

`ai.ask` (⌘J, Option-double-click, or a clip's context menu) opens a
prompt over the selected clip or the playhead. Its request carries a focus
— the scope (`clip`, `range`, or `sequence`), the clip, and its span in
sequence frames — and `cut-agent` states that focus ahead of the person's
words (`focused_request`), so every adapter sends the same anchored request.
The clips around a focused clip move by the kernel's ripple rules, not by
the model.

For a pending proposal the bridge applies its script to the current
document (`cut_runtime::proposal_timeline`) and adds the resulting timeline
to the state event. The timeline draws it over the current one — outlines
for added and changed clips, strikes for removed ones — and a bar above the
timeline carries the agent's progress and the review: `proposal.accept`
(⌘Return), `proposal.reject` (Escape), and `ai.stop` are registry commands
that resolve to the bridge's accept, reject, and abort. The Cut Assistant
panel stays as the conversation history and starts closed. Plain
double-click keeps its edit-suite meaning: open the clip in the source
monitor.
