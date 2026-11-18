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
// request is about, in sequence frames), provider (anthropic|openai|
// claude-code: the installed Claude Code, signed in on its own, with this
// project's kerf-mcp as its only tools; no key),apiMode,apiKey,baseUrl,
// model,mcpEnabled,mcpConfigVersion,cloud? (use Kerf Cloud's hosted model),
// conversation? (the one the turn goes on in; none starts one), at? (the
// playhead it was asked at, s; see Conversations below)}, abort, play, pause, seek {position,scrub},
// volume {value}, solo {tracks: [trackId]} (hear only those audio tracks,
// [] hears all; monitoring only, the project never changes, and it holds
// across edits and a reconnect to kerfd until changed, so sending it again
// is harmless; no event answers it). Agent
// settings are request-scoped so switching providers never mutates the
// project or leaks credentials into its state.
// Edit ops beyond moves and trims: set {clip, props} and the generator
// parameters, fx-add {clip, fx, params?, index?} (at the end of the stack
// unless index), fx-set {clip, index, enabled?, params}, mask-add/mask-set
// {clip, name, params} take a static value or keyframes {keys: [{at: frame
// (whole frames from the clip's first frame), value, interp?: linear|hold|
// ease|ease-in|ease-out}]}, which the timeline sends back as it got them
// (each key's frame); keys on pan and on sound-effect parameters are
// refused (playback and renders follow keyframes on gain only).
// insert / overwrite {media, sourceIn (media frames) | generator: {fx
// (title, solid, adjustment), params?}, at, frames, tracks, ids?,
// sequence?, mode?: insert|overwrite (in place of the op's own)} place
// media or a generated picture on each of tracks; insert pushes what
// follows, overwrite covers it. add-track {kind: video|audio|subtitle,
// id?, name?, sequence?} adds an empty track after the others of its kind.
// set-sequence {width?, height? (together), name?, rate?: {num, den} |
// "29.97" | 25, start? (timecode text, ';' before the frames for drop
// frame), sequence?} changes what it names; a new rate only while the
// sequence has no clips or markers, else the edit is refused with "Frame
// rate can change only while the sequence is empty".
// set-marker {marker, at?: frame, frames?, name?, note?, color?} moves,
// renames or recolours a marker ("" clears a field). set-track {track,
// sequence?, enabled?, locked?, muted?, role?, syncLocked?} ("" role gives
// back the kind's default; ripple edits move only sync-locked tracks).
// Effect previews: effect_previews {clip, at, effects: [fxId | {fx, params?,
// key?}], width, key?} answers one effect_preview per entry that has a
// picture, width wide (at most 640, even). An id shows the effect added
// after the clip's own at its defaults; {fx, params} shows those static
// parameters (numbers kept in range, unknown names ignored), in place of
// the clip's own effect of that id when it has one, so a look before and
// after a change can be compared. A generator id (title, solid) draws the
// generator alone on black on a canvas shaped like the clip's sequence (a
// title with no text says "Title"); clip may then be "". Effects run on
// the decoded picture of a clip of footage, in the order of the stack;
// on a clip with no file of its own (a title, a nested sequence) and for
// generators, Kerf's compositor draws the picture. Sound effects, looks
// and the adjustment generator have no preview. A newer request stops an
// older one.
// Conversations (.kerf/conversations/<id>.json beside the project, never
// in the document or undo): conversations {action: list} answers
// conversations; {action: open, id} answers conversation; {action: archive,
// id, archived? (true when absent)}, {action: rename, id, title} and
// {action: delete, id} change one and answer conversation (but delete) and
// conversations. A refusal is error {message, command: conversation}. An
// agent turn keeps the request in its conversation before the agent runs,
// then Kerf's answer (its steps, how long it worked, the proposal it
// staged, whether it failed or was stopped), sending conversation and
// conversations each time. A session new to a conversation hears its
// earlier turns. The agent's stream events are unchanged.
// QC: check {sequence?} runs Kerf's full QC pass on the project kerfd has
// (the document checks, plus the sound measured to EBU R128 and every frame
// measured for gamut, luma levels, flashing and black, on the sequence or
// else the first one; about as long as a draft render): check {status:
// running, progress (0..1), sequence?} as it goes, then check {status:
// done, sequence?,
// valid (nothing blocks), findings: [{severity: error|warning|info, code,
// entity, message, sequence?, at?, duration?, timecode?, rect?: {x, y, w,
// h} (0..1 of the frame), value?, limit?, unit?}], summary:
// {passed, total, blocking, rows: [{code, name, passed, value, limit?, at?,
// count, blocking}]}} or check {status: failed, message} (also
// when it is cancelled, which closing the bridge does). With
// a sequence, findings on other sequences are left out and the rows
// counted again.
// One check runs at a time; another meanwhile is error {command: check}.
// Proposals: several can wait at once (at most 8, newest first in the
// snapshot's proposals); accepting one makes the others stale (they then
// need a revise). An earlier version's proposalId accepts that
// version.
// Rendering: preview {proposalId?, start?, duration?} renders the program,
// or what that pending proposal would make of it, over start..start+duration
// seconds when given (both or neither, at most 120 s), and plays it in place
// of the program; preview_stop stops every preview sent before it, the one
// rendering and one not begun alike, each of which answers preview {status:
// stopped} (with none to stop, the stop answers so at once); preview_exit
// stops them too and plays the program again (preview {status: exited}, then
// viewer_reset and the program's own preview status when a preview was
// showing). One viewer preview renders at a time; it is not an output and
// never shows in the render queue.
// Render queue: export {quality: preview|full|review|master, codec?:
// h264|hevc|prores_hq (a master is ProRes 422 HQ in QuickTime with 24-bit
// PCM unless codec says otherwise; a review copy is full-size H.264 at
// 12 Mb/s; a preview is a draft from proxies), sequence? (the first when
// absent), output? (a folder, ending in / or existing, or a file whose
// extension the codec takes: .mov for ProRes, else .mp4; relative ones are
// read against the project and must stay inside it; an absolute folder
// must exist), name? (the job's name, and the file's in a folder)} queues
// the render with kerfd at once: export {status: queued, jobId, name, path
// (the file it will write; a folder takes the next free name, "Master
// 2.mov"), progress: 0, quality, codec, sequence?}. A refusal is error
// {message, code: INVALID_RENDER_QUALITY|INVALID_RENDER_CODEC|
// INVALID_RENDER_OUTPUT|INVALID_RENDER_NAME|RENDER_OUTPUT_BUSY|
// RENDER_OUTPUT_EXISTS|..., command: export}, and nothing is queued. Jobs
// render one at a time, oldest first, from the document as it was when
// queued: export {status: running, jobId, name, path, progress (0..1)},
// then completed (path, progress 1), failed (message; also error {command:
// export, jobId}) or cancelled. export_cancel {jobId} takes a queued job
// out, or stops the one rendering (export {status: cancelled}); a finished
// job stays as it is. render_queue {} answers render_queue {jobs: [{jobId,
// name (or what it renders: Master, Review copy, Full render, Preview),
// quality, codec, sequence, status: queued|running|completed|failed|
// cancelled, progress, outputPath, error?}]} (oldest first: kerfd's queued,
// running and newest finished jobs, from any client; kerfd keeps the newest
// 50 finished), which is also sent
// after every change of status. Closing the bridge stops the job rendering
// and cancels the queued ones.
// Captions: export_captions {format: srt|vtt, path?, sequence? (the
// program when absent), track? (every enabled subtitle track when absent)}.
// The file goes to path (a file, or a folder for the default name; relative
// to the project), else beside the newest finished master (<master>.srt|vtt),
// else in deliveries/ named after the project; it is written whole (through
// a temporary file) and replaces an earlier file of that name. The answer is
// export {status: completed, kind: captions, format, path} or export
// {status: failed, kind, format, message} with error {message, command:
// export_captions, format}.
// Analysis: loudness {sequence?} measures the programme (or that sequence)
// to EBU R128 off the GUI: loudness {status: running, progress, revision,
// sequence?}, then loudness {status: ready, integrated (LUFS), lra (LU),
// truePeak: [l, r] (dBTP), truePeakMax, shortTermMax, momentaryMax,
// shortTermMaxAt, momentaryMaxAt (s), series: [[t, momentary, shortTerm]]
// every 0.1 s, duration, target: -23, limit: -1, revision (the document
// revision measured), sequence?} with silence as null, or loudness {status:
// failed, message, revision?, sequence?}. A measurement is kept for its
// revision, so asking again before an edit answers ready at once; a
// request while one runs is taken when it ends. media_info {media?} answers
// media_info {media: {mediaId: {codec, profile?, pixFmt, bitDepth,
// colorSpace?, transfer?, primaries?, colorRange?, camera?, lens?, reel?,
// timecode?, audioCodec?, audioSampleFmt?, audioBitDepth?, audioChannels?,
// audioSampleRate?}}, errors?: {mediaId: message}} (fields the file does
// not say are left out) for every media of the project, or only the one
// named; it is also sent on its own for media new to the editor (on
// opening, after an import, a replace or a merge), naming only those, so
// the editor merges each answer into what it has. The one
// media asked for that cannot be read is error {command: media_info,
// media, message}. It is never written into the document. media_index
// {media} answers media_index {media, duration, cuts: [s], ranges:
// [{start, end}] (the shots the cuts make), silences: [[start, end]],
// loudest: [s] (in time order), peaks: [{at, level (dBFS)}], onsets: [s],
// tempo: {bpm, first, confidence} or null}, seconds from the media's first
// frame, from the project's cache when kerfd's import made it, else made
// now; a refusal is error {command: media_index, media, message}.
// Effects presets: look_save {clip, name, replace?} keeps the clip's
// enabled picture effects (settings and keyframes) as a look in
// presets/looks/<name>.kerflook (refusing an existing file unless replace):
// look_saved {path (relative to the project), id (look.<name>), version,
// effects (how many), skipped: [what a look cannot carry: looks, sound
// effects, parameters naming a mask or a track]}. look_apply {clip, path}
// keeps the look in the project's .kerf/looks/<id>@<version>.kerflook (a
// look that differs from one of that id already there becomes its next
// version) and adds it to the clip as one effect: the state, then
// look_applied {path, clip, fx, version}. A refusal is error {message,
// command: look_save|look_apply}.
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
// into media/ the same way. media_make {media, confirmed?} makes a take of
// a media that names a recipe (recipe= on its media line; a source says its
// recipe), or whose steps the document holds, written under its media line
// with no recipe file (a source says steps, how many; the editor offers it
// as Make… and Make Several where a recipe's is Make from Recipe…). Without
// confirmed it runs nothing and answers media_make_plan
// {media, recipe, command, steps, notes, take, file}: the recipe's file (the
// empty string for steps in the document), its
// steps as they would be run (command holds them a line each), what its
// model has said it does not take (notes, none when the recipe asks only
// what it takes), and the number and file of the take they would write, for
// a person to read. A recipe that
// could not run, such as one asking a model that is not set up, is an error
// {message} instead. With rolls: N (2 to 16) it is N takes, one after
// another, each with the recipe's seeds one on from the last: the plan says
// rolls and the first take, each take answers a media_making of its own as it
// begins, and media_made names the last, which the media then is; a take
// that fails leaves the ones before it, the media being the last made.
// With confirmed: true, which only a person's yes to that
// plan may send, it runs the recipe off the GUI thread, answers media_made
// {media, take} and then the state of the replace that has the media be
// that take, or error {message, command: "media_make", media} with the media
// as it was. While it runs it answers media_making {media, take, line}: first
// with an empty line, to say that it began, then with each line its steps
// say as they say it (a rendering's share done, a model's program's words).
// media_make_cancel {media} stops a making under way: its step is asked to
// end, so that a model's program can take its job back, and is ended if it
// has not within a few seconds; it answers media_make_cancelled {media},
// with no take and the media as it was. Closing the bridge stops every
// making the same way.
// A making another program runs on the project, `kerf make` or an agent's
// own kerf-mcp, answers the same media_making, media_made,
// media_make_cancelled and error as it goes, though nothing asked the bridge
// for it, and media_make_cancel {media} stops it too. Closing the bridge
// leaves it running.
// Cast: cast_get answers cast {entries, toMake, saved: false}: the project's
// cast.kerfcast as entries {kind: "cast"|"style", name, words, refs}, in the
// file's order, and the media whose shots want making as things stand, each
// {media, why}. cast_set {entries} writes those as the file, in place of the
// one there (its comments are not kept), and answers the same with saved:
// true; entries that would not read as a cast file are an error {message,
// command: "cast"} and the file is as it was.
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
// Events: state {snapshot}, connection {status,message}, error {message,
// code?, command?, id?} (code, command and id name a refused command),
// agent {kind: delta|status|tool|done|...,text}, agent_busy {busy},
// agent_state {state: running|awaiting_review|aborted|completed|failed,
// message?}, agent_next {steps: [text]} (next steps to offer as buttons),
// clock {position, duration, dropped (frames the presenter skipped as late
// since the program was opened or moved)}, playback {playing},
// caption {text},
// meter {peak: [l, r], rms: [l, r] (dBFS, at least -120), momentary,
// shortTerm (BS.1770 LUFS, null until 400 ms / 3 s were heard), at (the
// program time the reading reaches)} about 20 times a second while
// playing, and once with every value null when playback stops (a pause
// or the end of the program),
// conversations {items: [{id, title, created, updated, archived, count}]}
// (newest first), conversation {id, title, created, updated, archived,
// items: [{id, kind: you|kerf, text, at, timecode?, steps?, seconds?,
// error?, stopped?, proposal?}]},
// viewer_reset,
// preview {status: running|ready|live|required|unavailable|failed|stopped|
// exited, progress?, message?, proposalId? (the proposal a rendered preview
// shows)}, export (see Render queue and Captions above: {status: queued|
// running|completed|failed|cancelled, jobId, name, path, progress, quality,
// codec, sequence?} for renders, {status, kind: captions, format,
// path|message} for captions), render_queue {jobs}, loudness,
// media_info, media_index, look_saved, look_applied (see Analysis and
// Effects presets above),
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
// proposal, and its earlier versions made on this revision), revisionTime?
// (ISO 8601 of the current revision)}. timeline.sequences lists every sequence: [{id, name, width,
// height, rate, fps, dropFrame, duration}]; timeline.tracks[] carry role
// (the track's role or its kind's default), roleAssigned and syncLocked;
// each keyframe carries frame (clip-relative, as edits send it back) beside
// at (seconds), value and interp.
// effect_preview {clip, fx, key? (the entry's key, else the request's),
// width, height, rgba (base64)},
// check (see QC above),
// media_status {missing: [mediaId], pointers: [mediaId] (Git LFS files not
// downloaded), external: [mediaId] (outside the project folder, or ignored
// by Git there)},
// preset_library {directory, presets: [{path (relative to the project),
// name, slots: [{id, seconds, video, audio, clips}], texts: {clipId: text},
// author?, updated? (ISO 8601, from the last commit that touched it),
// error?}], looks: [{path, name, id, version, effects, author?, updated?}
// or {path, name, error}] (the effects presets in presets/looks/)},
// preset_saved {path},
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
// An HDR sequence's frame as its signal: 16-bit little-endian RGBA in BT.2020, on curve 1 (PQ) or 2 (HLG).
struct KerfSignal { const uint8_t *pixels; size_t length; uint32_t width; uint32_t height; double time; uint32_t curve; };
bool kerf_bridge_signal(void *bridge, KerfSignal *signal);
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
// Everything the catalog offers, the same shape plus domain: video|audio:
// picture effects and transitions, generators (kind: generator, such as
// title and solid) and sound effects (domain: audio). Free with
// kerf_bridge_string_free.
char *kerf_fx_catalog_all(void);
// The properties a clip has by its track's type, which that list leaves out:
// {video: [def], audio: [def]}, as a timeline's properties carries them. Free
// with kerf_bridge_string_free.
char *kerf_clip_properties(void);
// {name, version, license, repository}: what Help > About Kerf shows. Free
// with kerf_bridge_string_free.
char *kerf_about(void);
// Whether a newer Kerf is released on GitHub: {current, latest?, newer,
// url?} or {error} (KERF_UPDATE_CHECK=off asks nothing). Blocks on the
// network for up to 10 s: call it off the GUI thread. Free with
// kerf_bridge_string_free.
char *kerf_update_check(void);
// Report every panic in the editor's process to the crash folder, as kerfd
// and the CLI do, and every crash of its native code too: a bad memory
// access, or an abort (signals on macOS and Linux; an unhandled-exception
// filter and SIGABRT on Windows). Call once, before any other thread starts.
// Kerf's tools over MCP on stdin and stdout, as kerf-mcp serves them, when the
// arguments (without the program's name) begin with --serve-mcp --dir <project>:
// returns the status to exit with, or -1 when they do not and the editor starts.
int kerf_serve_mcp(int count, const char *const *arguments);
void kerf_crash_install(void);
// Report `message` now as what ends the editor, as Qt's fatal messages do
// before Qt aborts; the abort then adds no second report. Nothing is written
// before kerf_crash_install.
void kerf_crash_fatal(const char *message);
// The crash folder's reports: {folder (null when there is nowhere to keep
// them), reports: [{path, component (editor|kerfd|kerf), atMs, message}]},
// newest first. Free with kerf_bridge_string_free.
char *kerf_crash_reports(void);
// The system keychain: {op: get|set|delete, name, value?} -> {ok: true,
// value? (null when get finds none)} or {error} when it cannot be used
// (KERF_SECRETS=off turns it off). Free with kerf_bridge_string_free.
char *kerf_secret(const char *json);
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
