#pragma once
#include <stdint.h>
#include <stddef.h>

// Calls are nonblocking and confined to the Qt GUI thread. Runtime, media and
// agent work run on Rust workers. All strings are UTF-8.
extern "C" {
void *kerf_bridge_new(const char *project_directory, const char *kerfd_binary);
void kerf_bridge_free(void *bridge);
// JSON commands: refresh, reconnect, import {path}, edit {ops,revision},
// undo/redo {revision}, accept/reject {proposalId,revision}, agent {text,
// focus?: {scope: clip|range|sequence, clip?, fromFrame, toFrame} (what the
// request is about, in sequence frames), provider,apiMode,apiKey,baseUrl,
// model,mcpEnabled,mcpConfigVersion,cloud? (use Kerf Cloud's hosted model)},
// abort, play, pause, seek {position,scrub}, volume {value},
// effect_previews {clip,at,effects,width}. Agent
// settings are request-scoped so switching providers never mutates the
// project or leaks credentials into its state.
// Proposals: several can wait at once (at most 8, newest first in the
// snapshot's proposals); accepting one makes the others stale (they then
// need a revise), and resolves the comments linked to it (resolvedRevision
// = the revision it made). An earlier version's proposalId accepts that
// version.
// Rendering: preview {proposalId?, start?, duration?} renders the program,
// or what that pending proposal would make of it, over start..start+duration
// seconds when given (both or neither, at most 120 s), and plays it in place
// of the program; preview_stop stops the preview render running
// (preview {status: stopped}); preview_exit stops it and plays the program
// again (preview {status: exited}, then viewer_reset and the program's own
// preview status). export {quality: preview|full, sequence?} renders a
// sequence (the first when absent). One render runs at a time.
// Kerf Cloud: cloud_sign_in {token} keeps a token and reads its account,
// cloud_sign_out forgets it, cloud_refresh reads the account again; each is
// answered by a cloud event.
// Presets: preset_export {path} answers preset_export {path, document} (the
// program as a .kerfpreset document for the GUI to write); preset_inspect
// {path} answers preset_inspect {path, preset}; preset_apply {path,
// bindings, texts, revision} stages the preset as a proposal and answers
// preset_applied {path, proposal}.
// Media: import copies the file into the project's media/ folder first when
// the project has one (named <stem>-<8 hex of its SHA-256>.<ext>, so every
// collaborator's copy of one file has one name), and stores the path
// relative to the project. media_replace {media, path} swaps the file behind
// a media id (every clip that uses it keeps its timing; a file too short or
// without the streams the clips need is refused, naming them), copying it
// into media/ the same way.
// Projection: projection_import loads a project.kerf changed outside Kerf
// (a git pull in a terminal, a text editor) as one revision;
// projection_repair writes the editor's project over it.
// Presets: preset_library lists the project's preset library (presets/,
// then .kerf/presets and the project folder); preset_save {name, replace?}
// saves the program sequence as presets/<name>.kerfpreset (the name cleaned
// up as project folders are: '/', '\\' and ':' become '-', surrounding dots
// and spaces trimmed), refusing an existing file, by path, unless replace.
// Collaboration over Git, all asynchronous and answered by collab events:
// collab_status; collab_sync {message?} commits local edits, pulls and
// merges the upstream branch, and pushes; collab_live {enabled, interval?}
// syncs every interval seconds (default 10), and a few seconds after each
// edit, while enabled; collab_commit {message}; collab_branch {name,
// create} commits local edits, then switches (or creates and switches);
// collab_pull_request {title, body?}; collab_publish {remote?, github?:
// {name?, private}} makes the project a repository if it is not one, adds
// the remote or creates the GitHub repository with gh, then pushes (with
// neither, it only makes the project a repository on this computer, and an
// existing repository without a remote is refused);
// collab_consolidate copies media from outside the project into media/ and
// relinks it.
// false means the bounded queue is full; show an error, never silently drop edits.
bool kerf_bridge_send(void *bridge, const char *json);
// One JSON event, owned until kerf_bridge_string_free; nullptr when idle.
// Events: state {snapshot}, connection {status,message}, error {message},
// agent {kind: delta|status|tool|done|...,text}, agent_busy {busy},
// agent_state {state: running|awaiting_review|aborted|completed|failed,
// message?}, agent_next {steps: [text]} (next steps to offer as buttons),
// clock {position,duration}, playback {playing}, caption {text},
// viewer_reset,
// preview {status: running|ready|live|required|unavailable|failed|stopped|
// exited, progress?, message?, proposalId? (the proposal a rendered preview
// shows)}, export {status: running|completed|failed, path?, progress?},
// cloud {access: ready|unavailable|signedOut|unreachable|notIncluded|
// outOfCredits, account?, message?},
// The state snapshot: {document, timeline, revision, canUndo, canRedo,
// duration, sourceVersion, projectionStatus, importWarnings?,
// pendingProposal (the first open proposal, else the newest settled one:
// {status: pending|accepted|rejected, proposalId, baseRevision, script?,
// source?, documentDiff?, freshness?: stale, appliedRevision?}),
// proposalTimeline (the pending proposal's timeline), proposals: [{proposalId,
// status: pending|accepted|rejected, freshness: current|stale, baseRevision,
// appliedRevision?, script, source, documentDiff, author: {kind: kerf|mcp|
// person|qc, name, id?, role?}, version, supersedes (the proposalId this
// version replaced, or null), versions: [{version, proposalId, baseRevision,
// script, source?, documentDiff}] (oldest first, the current one included),
// staged? (ISO 8601)}], proposalTimelines {proposalId: timeline} (every open
// proposal, and its earlier versions made on this revision), reviewRevision
// (changes whenever review/comments.jsonl does, here, from another client
// such as an MCP agent, or on disk), revisionTime? (ISO 8601 of the current
// revision)}. timeline.sequences lists every sequence: [{id, name, width,
// height, rate, fps, dropFrame, duration}].
// effect_preview {clip,fx,width,height,rgba (base64)},
// media_status {missing: [mediaId], pointers: [mediaId] (Git LFS files not
// downloaded), external: [mediaId] (outside the project folder, or ignored
// by Git there)},
// preset_library {directory, presets: [{path (relative to the project),
// name, slots: [{id, seconds, video, audio, clips}], texts: {clipId: text},
// author?, updated? (ISO 8601, from the last commit that touched it),
// error?}]}, preset_saved {path},
// collab {status: {repository, enclosing?, branch, upstream, remote, ahead,
// behind, changed, conflicted, branches, lfs, gh (installed and signed in),
// live, busy, user: {name, email} or null (who commits here are by, also
// outside a repository: "You"), head (the commit checked out, or null)}},
// collab_sync {stage: committing|fetching|merging|pushing|downloading|done|
// failed, message?, report?: {committed?, incoming: [{commit, author,
// subject}], fastForward, pushed, conflicts: [{path, what, kept}], renamed:
// [[from, to]], summary: [line], branch?}},
// collab_pull_request {url, created}.
char *kerf_bridge_poll(void *bridge);
void kerf_bridge_string_free(char *value);
struct KerfFrame { const uint8_t *pixels; size_t length; uint32_t width; uint32_t height; double time; };
// Latest RGBA frame only. Bytes live until the next frame poll or bridge free.
bool kerf_bridge_frame(void *bridge, KerfFrame *frame);
// The command registry (kerf-commands). Synchronous and pure, so it can be
// called before a bridge exists. Results are owned JSON strings, freed with
// kerf_bridge_string_free.
// The registry with shortcuts in a keymap id: {keymap, keymaps, menus, commands}.
char *kerf_commands_catalog(const char *keymap);
// {commandId: reason or null} for a JSON editor context.
char *kerf_commands_states(const char *context);
// {command, params, context} -> {request} (a kerf_bridge_send command) or {error}.
char *kerf_commands_run(const char *invocation);
// The picture effects and transitions offered, as a JSON array of
// {id, kind, category, name, doc, engine, params}. Free with kerf_bridge_string_free.
char *kerf_fx_catalog(void);
// The project launcher, before any bridge exists. Synchronous and local.
// {path} -> {path, exists, project (has project.kerf), name, repository,
// branch, remote, modified (ISO 8601 of project.kerf), presets: count}.
char *kerf_project_inspect(const char *json);
// A preset file's slots, and how files would fill them: {path, files?:
// [path]} -> {name, slots: [{id, seconds, video, audio, clips}], texts:
// {clipId: text}, files: [{path, seconds, video, audio, fits: [slotId],
// error?}]} or {error}. Probes files with ffprobe; may take a moment.
char *kerf_preset_inspect(const char *json);
// Project tasks that may take a while, polled from the GUI thread:
// {type: "create", name, location, template: standard|vertical|square,
// rate: "23.976"|"24"|"25"|"29.97"|"30"|"60", git, remote?, github?:
// {name?, private}, kerfd?, preset?: {path, bindings: {slotId: {file,
// from? (a time such as "2s" or "48f")}}, texts?: {clipId: text}}} makes
// the folder and, with a preset, copies it into presets/, copies each file
// into media/, imports them with kerfd and applies the preset with those
// bindings as the project's first edit; {type: "clone", url, location};
// {type: "tools"} -> {git, gitLfs, gh, ghUser}. Poll returns {status:
// "running", message?}, {status: "done", path?, remote?, warnings?, ...} or
// {status: "failed", error}; each call returns an owned string.
void *kerf_task_start(const char *json);
char *kerf_task_poll(void *task);
void kerf_task_free(void *task);
}
