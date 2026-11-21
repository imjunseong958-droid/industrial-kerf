//! The editor's bridge to Kerf: JSON commands in, JSON events out, over kerfd and Rust
//! workers.
//!
//! kerf-editor opens a project with [`Bridge::open`], sends it commands with [`Bridge::send`],
//! and takes its events with [`Bridge::poll`] and its pictures with [`Bridge::frame`] and
//! [`Bridge::signal`]. Every call returns at once and never waits on project I/O: canonical
//! edits stay in kerfd, and runtime, media, agent, render and collaboration work run on Rust
//! workers. Media frames travel through a latest-frame slot rather than JSON or an
//! accumulating queue. Dropping the bridge closes it: the workers are told to stop, and
//! nothing waits for them. All text is UTF-8.
//!
//! This page is the protocol's reference: the commands [`Bridge::send`] takes and the events
//! [`Bridge::poll`] gives, then the functions that need no bridge. A command or an event is a
//! JSON object whose `type` names it, such as `{"type": "seek", "position": 2.5}`. Below,
//! `seek {position, scrub?}` stands for that object, a `?` marking a field that may be left
//! out and `a|b` the values a field takes.
//!
//! # Commands
//!
//! [`Bridge::send`] returns `false` when a command is refused: it does not read as one, it
//! cannot be taken as it is (a seek before 0, an agent turn while one runs), or its bounded
//! queue is full. Show an error then; never silently drop edits.
//!
//! ## Project, edits and playback
//!
//! - `refresh`; `reconnect`.
//! - `import {path, mediaOnly?}`; `edit {ops, revision}`; `undo {revision}`; `redo
//!   {revision}`; `accept {proposalId, revision}`; `reject {proposalId, revision}`; `forward
//!   {proposalId, revision}` (see Proposals).
//! - `abort` stops the agent's turn, and every version running.
//! - `play`; `pause`; `seek {position, scrub?}` (with `scrub`, the pointer is still moving:
//!   the nearest picture shows at once, and a later seek without it is exact); `volume
//!   {value}`; `solo {tracks: [trackId]}` hears only those audio tracks (`[]` hears all). Solo
//!   is monitoring only: the project never changes, and it holds across edits and a reconnect
//!   to kerfd until changed, so sending it again is harmless. No event answers it.
//!
//! Edit ops beyond moves and trims:
//!
//! - `set {clip, props}` and the generator parameters; `fx-add {clip, fx, params?, index?}`
//!   (at the end of the stack unless `index`); `fx-set {clip, index, enabled?, params}`;
//!   `mask-add` and `mask-set {clip, name, params}`. A parameter takes a static value or
//!   keyframes `{keys: [{at: frame, value, interp?: linear|hold|ease|ease-in|ease-out}]}`, `at`
//!   in whole frames from the clip's first frame, which the timeline sends back as it got
//!   them (each key's frame). Keys on pan and on sound-effect parameters are refused
//!   (playback and renders follow keyframes on gain only).
//! - `insert` and `overwrite {media, sourceIn (media frames) | generator: {fx (title, solid,
//!   adjustment), params?}, at, frames, tracks, ids?, sequence?, mode?: insert|overwrite (in
//!   place of the op's own)}` place media or a generated picture on each of `tracks`; insert
//!   pushes what follows, overwrite covers it.
//! - `add-track {kind: video|audio|subtitle, id?, name?, sequence?}` adds an empty track
//!   after the others of its kind.
//! - `set-sequence {width?, height? (together), name?, rate?: {num, den} | "29.97" | 25,
//!   start? (timecode text, ';' before the frames for drop frame), sequence?}` changes what
//!   it names. A new rate is taken only while the sequence has no clips or markers; else the
//!   edit is refused with "Frame rate can change only while the sequence is empty".
//! - `set-marker {marker, at?: frame, frames?, name?, note?, color?}` moves, renames or
//!   recolours a marker (`""` clears a field).
//! - `set-track {track, sequence?, enabled?, locked?, muted?, role?, syncLocked?}` (`""` role
//!   gives back the kind's default; ripple edits move only sync-locked tracks).
//!
//! ## Agent
//!
//! `agent {text, focus?, provider?, apiMode?, apiKey?, baseUrl?, model?, mcpEnabled?,
//! mcpConfigVersion?, cloud?, conversation?, at?, permissions?}`:
//!
//! - `focus: {scope: clip|range|sequence, clip?, fromFrame, toFrame}` is what the request is
//!   about, in sequence frames.
//! - `provider: anthropic|openai|claude-code`. `claude-code` is the installed Claude Code,
//!   signed in on its own, with this project's kerf-mcp as its only tools, and needs no key.
//!   `cloud` uses Kerf Cloud's hosted model.
//! - `conversation` is the one the turn goes on in (none starts one), and `at` the playhead
//!   it was asked at, in seconds (see Conversations).
//! - `permissions: ask|accept-edits|auto` is what Kerf may do without asking, as chosen in
//!   the editor; none is the person's own file's say. A name that is none of them asks.
//!
//! Agent settings are request-scoped, so switching providers never changes the project or
//! leaks credentials into its state. One turn runs at a time: an agent command while one
//! runs, or with empty text, is refused.
//!
//! `agent_versions {text, count?, ...settings}` has several agents (3 when `count` is left
//! out, at most 3) make the same request at once, each staging its own proposal in a
//! conversation of its own: `agent_version {key: A|B|C, state: running|done|failed|stopped,
//! title?, conversation?, status?, proposalId?, message?}` follows each.
//!
//! `models` answers `models {file, exists, models, error?}`: the person's models (from their
//! own models file) that a recipe's `generate` can name.
//!
//! ## Conversations
//!
//! Conversations are kept in `.kerf/conversations/<id>.json` beside the project, never in
//! the document or undo. `conversations {action: list}` answers `conversations`; `{action:
//! open, id}` answers `conversation`; `{action: archive, id, archived? (true when absent)}`,
//! `{action: rename, id, title}` and `{action: delete, id}` change one and answer
//! `conversation` (but delete) and `conversations`. A refusal is `error {message, command:
//! conversation}`.
//!
//! An agent turn keeps the request in its conversation before the agent runs, then Kerf's
//! answer (its steps, how long it worked, the proposal it staged, whether it failed or was
//! stopped), sending `conversation` and `conversations` each time. A session new to a
//! conversation hears its earlier turns. The agent's stream events are unchanged.
//!
//! ## Effect previews
//!
//! `effect_previews {clip, at, effects: [fxId | {fx, params?, key?}], width, key?}` answers
//! one `effect_preview` per entry that has a picture, `width` wide (at most 640, even).
//!
//! - An id shows the effect added after the clip's own, at its defaults.
//! - `{fx, params}` shows those static parameters (numbers kept in range, unknown names
//!   ignored), in place of the clip's own effect of that id when it has one, so a look
//!   before and after a change can be compared.
//! - A generator id (title, solid) draws the generator alone on black on a canvas shaped
//!   like the clip's sequence (a title with no text says "Title"); `clip` may then be `""`.
//!
//! Effects run on the decoded picture of a clip of footage, in the order of the stack; on a
//! clip with no file of its own (a title, a nested sequence) and for generators, Kerf's
//! compositor draws the picture. Sound effects, looks and the adjustment generator have no
//! preview. A newer request stops an older one.
//!
//! ## QC
//!
//! `check {sequence?}` runs Kerf's full QC pass on the project kerfd has: the document
//! checks, plus the sound measured to EBU R128 and every frame measured for gamut, luma
//! levels, flashing and black, on the sequence or else the first one (about as long as a
//! draft render). It answers `check {status: running, progress (0..1), sequence?}` as it
//! goes, then one of:
//!
//! - `check {status: done, sequence?, valid (nothing blocks), findings, summary}`, with
//!   `findings: [{severity: error|warning|info, code, entity, message, sequence?, at?,
//!   duration?, timecode?, rect?: {x, y, w, h} (0..1 of the frame), value?, limit?, unit?}]`
//!   and `summary: {passed, total, blocking, rows: [{code, name, passed, value, limit?, at?,
//!   count, blocking}]}`. With a sequence, findings on other sequences are left out and the
//!   rows counted again.
//! - `check {status: failed, message}`, also when it is cancelled, which closing the bridge
//!   does.
//!
//! One check runs at a time; another meanwhile is `error {command: check}`.
//!
//! ## Proposals
//!
//! Several can wait at once (at most 8, newest first in the snapshot's `proposals`);
//! accepting one makes the others stale. A stale one is not accepted: `forward {proposalId,
//! revision}` brings it onto the project as it is, its operations run again with each place
//! they name moved with the clips that held it, as its next version (a new `proposalId`,
//! freshness current) to be reviewed again. It answers with `error {command: forward, code,
//! message}`, the proposal left as it was, where a clip a line names or a place was on has
//! been cut, moved or removed since: the message names the clip, and the proposal needs a
//! revise. An earlier version's `proposalId` accepts that version.
//!
//! ## Viewer previews
//!
//! `preview {proposalId?, start?, duration?}` renders the program, or what that pending
//! proposal would make of it, over `start..start + duration` seconds when given (both or
//! neither, at most 120 s), and plays it in place of the program. `preview_stop` stops every
//! preview sent before it, the one rendering and one not begun alike, each of which answers
//! `preview {status: stopped}` (with none to stop, the stop answers so at once).
//! `preview_exit` stops them too and plays the program again (`preview {status: exited}`,
//! then `viewer_reset` and the program's own preview status when a preview was showing).
//! One viewer preview renders at a time; it is not an output and never shows in the render
//! queue.
//!
//! ## Render queue
//!
//! `export {quality, codec?, sequence?, output?, name?}` queues a render with kerfd at once:
//!
//! - `quality: preview|full|review|master` and `codec?: h264|hevc|prores_hq`. A master is
//!   ProRes 422 HQ in QuickTime with 24-bit PCM unless `codec` says otherwise; a review copy
//!   is full-size H.264 at 12 Mb/s; a preview is a draft from proxies.
//! - `sequence` is the first when absent.
//! - `output` is a folder (ending in `/`, or existing) or a file whose extension the codec
//!   takes: `.mov` for ProRes, else `.mp4`. Relative ones are read against the project and
//!   must stay inside it; an absolute folder must exist.
//! - `name` is the job's name, and the file's in a folder.
//!
//! It answers `export {status: queued, jobId, name, path, progress: 0, quality, codec,
//! sequence?}`, `path` being the file it will write (a folder takes the next free name,
//! "Master 2.mov"). A refusal is `error {message, code, command: export}` with `code` one of
//! `INVALID_RENDER_QUALITY`, `INVALID_RENDER_CODEC`, `INVALID_RENDER_OUTPUT`,
//! `INVALID_RENDER_NAME`, `RENDER_OUTPUT_BUSY`, `RENDER_OUTPUT_EXISTS` and others, and
//! nothing is queued.
//!
//! Jobs render one at a time, oldest first, from the document as it was when queued: `export
//! {status: running, jobId, name, path, progress (0..1)}`, then `completed` (`path`,
//! `progress` 1), `failed` (`message`; also `error {command: export, jobId}`) or `cancelled`.
//! `export_cancel {jobId}` takes a queued job out, or stops the one rendering (`export
//! {status: cancelled}`); a finished job stays as it is.
//!
//! `render_queue` answers `render_queue {jobs: [{jobId, name, quality, codec, sequence,
//! status: queued|running|completed|failed|cancelled, progress, outputPath, error?}]}`, `name`
//! being the job's or what it renders (Master, Review copy, Full render, Preview). The jobs
//! are kerfd's queued, running and newest finished ones (it keeps the newest 50 finished),
//! from any client, oldest first. It is also sent after every change of status. Closing the
//! bridge stops the job rendering and cancels the queued ones.
//!
//! ## Captions
//!
//! `export_captions {format: srt|vtt, path?, sequence? (the program when absent), track?
//! (every enabled subtitle track when absent)}`. The file goes to `path` (a file, or a folder
//! for the default name; relative to the project), else beside the newest finished master
//! (`<master>.srt|vtt`), else in `deliveries/` named after the project. It is written whole
//! (through a temporary file) and replaces an earlier file of that name. The answer is
//! `export {status: completed, kind: captions, format, path}`, or `export {status: failed,
//! kind, format, message}` with `error {message, command: export_captions, format}`.
//!
//! ## Analysis
//!
//! `loudness {sequence?}` measures the programme (or that sequence) to EBU R128 off the UI
//! thread: `loudness {status: running, progress, revision, sequence?}`, then `loudness
//! {status: ready, integrated (LUFS), lra (LU), truePeak: [l, r] (dBTP), truePeakMax,
//! shortTermMax, momentaryMax, shortTermMaxAt, momentaryMaxAt (s), series: [[t, momentary,
//! shortTerm]] every 0.1 s, duration, target: -23, limit: -1, revision (the document
//! revision measured), sequence?}` with silence as null, or `loudness {status: failed,
//! message, revision?, sequence?}`. A measurement is kept for its revision, so asking again
//! before an edit answers ready at once; a request while one runs is taken when it ends.
//!
//! `media_info {media?}` answers `media_info {media: {mediaId: {codec, profile?, pixFmt,
//! bitDepth, colorSpace?, transfer?, primaries?, colorRange?, camera?, lens?, reel?,
//! timecode?, audioCodec?, audioSampleFmt?, audioBitDepth?, audioChannels?,
//! audioSampleRate?}}, errors?: {mediaId: message}}` (fields the file does not say are left
//! out) for every media of the project, or only the one named. It is also sent on its own
//! for media new to the editor (on opening, after an import, a replace or a merge), naming
//! only those, so the editor merges each answer into what it has. The one media asked for
//! that cannot be read is `error {command: media_info, media, message}`. It is never written
//! into the document.
//!
//! `media_index {media}` answers `media_index {media, duration, cuts: [s], ranges: [{start,
//! end}] (the shots the cuts make), silences: [[start, end]], loudest: [s] (in time order),
//! peaks: [{at, level (dBFS)}], onsets: [s], tempo: {bpm, first, confidence} or null}`, in
//! seconds from the media's first frame, from the project's cache when kerfd's import made
//! it, else made now. A refusal is `error {command: media_index, media, message}`.
//!
//! `scope_signal {time, sequence?}` draws an HDR sequence's frame at `time` as the signal its
//! master gets, small, for the Finish tray's scopes to take with [`Bridge::signal`]. Only the
//! newest request waits for one that runs. `scope_signal {status}` says when it starts or
//! stops being a sequence with one; an SDR sequence draws nothing.
//!
//! ## Effects presets (looks)
//!
//! `look_save {clip, name, replace?}` keeps the clip's enabled picture effects (settings and
//! keyframes) as a look in `presets/looks/<name>.kerflook`, refusing an existing file unless
//! `replace`: `look_saved {path (relative to the project), id (look.<name>), version, effects
//! (how many), skipped: [what a look cannot carry: looks, sound effects, parameters naming a
//! mask or a track]}`.
//!
//! `look_apply {clip, path}` keeps the look in the project's
//! `.kerf/looks/<id>@<version>.kerflook` (a look that differs from one of that id already
//! there becomes its next version) and adds it to the clip as one effect: the state, then
//! `look_applied {path, clip, fx, version}`. A refusal is `error {message, command:
//! look_save|look_apply}`.
//!
//! ## Kerf Cloud
//!
//! `cloud_sign_in {token}` keeps a token and reads its account, `cloud_sign_out` forgets it,
//! and `cloud_refresh` reads the account again; each is answered by a `cloud` event.
//!
//! ## Presets
//!
//! - `preset_export {path}` answers `preset_export {path, document}`: the program as a
//!   `.kerfpreset` document for the editor to write.
//! - `preset_inspect {path}` answers `preset_inspect {path, preset}`.
//! - `preset_apply {path, bindings, texts, revision}` stages the preset as a proposal and
//!   answers `preset_applied {path, proposal}`.
//! - `preset_library` lists the project's preset library (`presets/`, then `.kerf/presets`
//!   and the project folder).
//! - `preset_save {name, replace?}` saves the program sequence as
//!   `presets/<name>.kerfpreset` (the name cleaned up as project folders are: `/`, `\` and
//!   `:` become `-`, surrounding dots and spaces trimmed), refusing an existing file, by
//!   path, unless `replace`: `preset_saved {path}`.
//!
//! ## Media
//!
//! `import` copies the file into the project's `media/` folder first when the project has
//! one (named `<stem>-<8 hex of its SHA-256>.<ext>`, so every collaborator's copy of one file
//! has one name), and stores the path relative to the project. With `mediaOnly` it is added
//! to the project's media without being cut into the program.
//!
//! `media_replace {media, path}` swaps the file behind a media id (every clip that uses it
//! keeps its timing; a file too short or without the streams the clips need is refused,
//! naming them), copying it into `media/` the same way.
//!
//! ### Making
//!
//! `media_make {media, confirmed?, rolls?}` makes a take of a media from the steps the
//! document holds under its media line (a source says `steps`, how many; the editor offers
//! it as Make… and Make Several).
//!
//! - Without `confirmed` it runs nothing and answers `media_make_plan {media, recipe,
//!   command, steps, notes, take, file}`: `recipe` is the empty string (no file says how a
//!   media is made), `steps` are as they would be run (`command` holds them a line each),
//!   `notes` what its model has said it does not take (none when the steps ask only what it
//!   takes), and the number and file of the take they would write, for a person to read.
//!   Steps that could not run, such as ones asking a model that is not set up, are an `error
//!   {message}` instead; so is a media whose steps are still in the recipe file it names,
//!   with how to carry them (`media_carry`).
//! - With `rolls: N` (2 to 16) it is N takes, one after another, each with the steps' seeds
//!   one on from the last: the plan says `rolls` and the first take, each take answers a
//!   `media_making` of its own as it begins, and `media_made` names the last, which the media
//!   then is. A take that fails leaves the ones before it, the media being the last made.
//! - With `confirmed: true`, which only a person's yes to that plan may send, it runs the
//!   steps off the UI thread and answers `media_made {media, take}` and then the state of the
//!   replace that has the media be that take, or `error {message, command: "media_make",
//!   media}` with the media as it was. While it runs it answers `media_making {media, take,
//!   line}`: first with an empty `line`, to say that it began, then with each line its steps
//!   say as they say it (a rendering's share done, a model's program's words).
//!
//! `media_make_cancel {media}` stops a making under way: its step is asked to end, so that a
//! model's program can take its job back, and is ended if it has not within a few seconds.
//! It answers `media_make_cancelled {media}`, with no take and the media as it was. Closing
//! the bridge stops every making the same way.
//!
//! A making another program runs on the project, `kerf make` or an agent's own kerf-mcp,
//! answers the same `media_making`, `media_made`, `media_make_cancelled` and `error` as it
//! goes, though nothing asked the bridge for it, and `media_make_cancel {media}` stops it
//! too. Closing the bridge leaves it running.
//!
//! ### Carrying a recipe into the document
//!
//! `media_carry {media, confirmed?}` carries the steps of a media that names a recipe file
//! (`recipe=` on its media line and no steps under it; a source says `recipe` and no steps,
//! and the editor offers Carry Steps into the Document…) into the document.
//!
//! - Without `confirmed` it writes nothing and answers `media_carry_plan {media, recipe,
//!   script, notes}`: the recipe file as the document names it, the lines of the edit (the
//!   media's line without `recipe=`, then a step line for each step, its files named from
//!   the project's folder), and what does not come with them, a line each.
//! - With `confirmed: true`, which only a person's yes to that plan may send, kerfd applies
//!   the lines as one edit of the person's own, which undo takes back (the state follows),
//!   the takes made from the recipe are held to the steps it became, and it answers
//!   `media_carried {media, recipe, held}`: how many takes are held to the steps now. The
//!   recipe file is left where it is.
//!
//! A media that cannot be carried (a recipe with a run line, a media whose steps are in the
//! document already) is `error {message, command: "media_carry", media}`, with the document
//! as it was.
//!
//! ### Recipes, shots and workflows
//!
//! - `recipe_get {media}` answers `recipe`: how a media is made, as a graph of its steps and
//!   what they read. Nothing is run.
//! - `recipe_edit {media, was, text?, line?, key?, nth?, value?, add?, remove?, restore?}`
//!   changes a media's steps, where they still read as the editor read them (`was`): the
//!   `nth` word of name `key` on `line` (from 1) set to `value`, or taken off the line with
//!   no value; or the whole `text`; or a step added by its kind (`add: guide|model|join`),
//!   the step on `line` taken out (`remove`), or the steps a take was made from restored
//!   (`restore: take`). It is applied as one edit, then `recipe` as they now read.
//! - `signal_from_sound {clip, of}` sets a signal clip's keys from the onsets of the sound
//!   clip `of` (of its sequence) plays: one edit of the project, as drawing its curve is.
//! - `shot_picture {clip, of: moves|matte}` draws the picture a person points on to read a
//!   clip from footage: `shot_picture {clip, of, path (a PNG), width, height, revision}`.
//! - `shot_follow {clip, of: moves|matte, points: [[x, y]], outside?: [[x, y]]}` has the clip
//!   follow the one point pointed at (`moves`), or cuts out of it what the points are on and
//!   not what the `outside` ones are on (`matte`): one edit of the project, then
//!   `shot_followed {clip, of, frames, keyframes, lostAt, says}`.
//! - `takes_read {media}` holds a media's takes to its guide by where its subjects are: a
//!   `take_read` for each as it is read, then `takes_read`.
//! - `workflow_get {media, line}` answers `workflow`, every node and input of the workflow a
//!   `generate` step's model is run with, where its program is kerf-comfy. What the server
//!   takes at each input (`taken: true`, each input's `choices` or `min`, `max` and `step`,
//!   or `takenError` with why it could not be asked) comes in a second `workflow` once the
//!   model's program has said it. That one is always the step's workflow as last sent: an
//!   edit sent meanwhile is not undone by it, and none comes when the last one has it.
//! - `workflow_edit {media, line, was, node, input, value?, link?: [node, output]}` sets a
//!   value of that workflow in the shot's own copy of it beside its recipe: `workflow`, then
//!   `recipe`.
//!
//! A refusal of any of these is an `error {message, command, ...}` naming what it was about.
//!
//! ### Cast
//!
//! `cast_get` answers `cast {entries, toMake, saved: false}`: the project's `cast.kerfcast`
//! as `entries` `{kind: "cast"|"style", name, words, refs}`, in the file's order, and the
//! media whose shots want making as things stand, each `{media, why}`. `cast_set {entries}`
//! writes those as the file, in place of the one there (its comments are not kept), and
//! answers the same with `saved: true`; entries that would not read as a cast file are an
//! `error {message, command: "cast"}` and the file is as it was.
//!
//! ## Projection
//!
//! `projection_import` loads a `project.kerf` changed outside Kerf (a git pull in a
//! terminal, a text editor) as one revision; `projection_repair` writes the editor's project
//! over it.
//!
//! ## Collaboration over Git
//!
//! All asynchronous, and answered by collab events:
//!
//! - `collab_status`.
//! - `collab_sync {message?}` commits local edits, pulls and merges the upstream branch, and
//!   pushes.
//! - `collab_live {enabled, interval?}` syncs every `interval` seconds (default 10), and a few
//!   seconds after each edit, while enabled.
//! - `collab_commit {message}`.
//! - `collab_branch {name, create}` commits local edits, then switches (or creates and
//!   switches).
//! - `collab_pull_request {title, body?}`.
//! - `collab_publish {remote?, github?: {name?, private}}` makes the project a repository if
//!   it is not one, adds the remote or creates the GitHub repository with gh, then pushes.
//!   With neither, it only makes the project a repository on this computer, and an existing
//!   repository without a remote is refused.
//! - `collab_consolidate` copies media from outside the project into `media/` and relinks it.
//!
//! # Events
//!
//! [`Bridge::poll`] gives one event at a time, and `None` when none is waiting.
//!
//! - `state {snapshot}` (see The state snapshot); `connection {status, message}`.
//! - `error {message, code?, command?, id?}`: `code`, `command` and `id` name a refused
//!   command.
//! - `agent {kind: delta|status|tool|done|..., text}`; `agent_busy {busy}`; `agent_state
//!   {state: running|awaiting_review|aborted|completed|failed, message?}`; `agent_next
//!   {steps: [{label, request}]}` (next steps to offer as buttons: the words on each and
//!   what pressing it asks); `agent_focus {tool, times, clips}`
//!   (where a tool call is working, for the timeline and picture to follow); `agent_context
//!   {used, limit, unit, conversation}` (how full the agent's context is); `agent_ask {media,
//!   rolls, take, text, conversation}` (a making the person is asked to run); `agent_version`
//!   (see Agent).
//! - `clock {position, duration, dropped}`, `dropped` being the frames the presenter skipped
//!   as late since the program was opened or moved; `playback {playing}`; `caption {text}`.
//! - `meter {peak: [l, r], rms: [l, r] (dBFS, at least -120), momentary, shortTerm (BS.1770
//!   LUFS, null until 400 ms / 3 s were heard), at (the program time the reading reaches)}`
//!   about 20 times a second while playing, and once with every value null when playback
//!   stops (a pause or the end of the program).
//! - `conversations {items: [{id, title, created, updated, archived, count}]}` (newest
//!   first); `conversation {id, title, created, updated, archived, items: [{id, kind:
//!   you|kerf, text, at, timecode?, steps?, seconds?, error?, stopped?, proposal?}]}`.
//! - `viewer_reset`.
//! - `preview {status: running|ready|live|required|unavailable|failed|stopped|exited,
//!   progress?, message?, proposalId? (the proposal a rendered preview shows)}`.
//! - `export` (see Render queue and Captions: `{status: queued|running|completed|failed|
//!   cancelled, jobId, name, path, progress, quality, codec, sequence?}` for renders, `{status,
//!   kind: captions, format, path|message}` for captions); `render_queue {jobs}`.
//! - `loudness`, `media_info`, `media_index`, `scope_signal` (see Analysis); `look_saved`,
//!   `look_applied` (see Effects presets).
//! - `cloud {access: ready|unavailable|signedOut|unreachable|notIncluded|outOfCredits,
//!   account?, message?}`.
//! - `effect_preview {clip, fx, key? (the entry's key, else the request's), width, height,
//!   rgba (base64)}`.
//! - `check` (see QC).
//! - `media_status {missing: [mediaId], pointers: [mediaId] (Git LFS files not downloaded),
//!   external: [mediaId] (outside the project folder, or ignored by Git there)}`.
//! - `preset_library {directory, presets: [{path (relative to the project), name, slots:
//!   [{id, seconds, video, audio, clips}], texts: {clipId: text}, author?, updated? (ISO 8601,
//!   from the last commit that touched it), error?}], looks: [{path, name, id, version,
//!   effects, author?, updated?} or {path, name, error}] (the effects presets in
//!   presets/looks/)}`; `preset_saved {path}`.
//! - `collab {status: {repository, enclosing?, branch, upstream, remote, ahead, behind,
//!   changed, conflicted, branches, lfs, gh (installed and signed in), live, busy, user:
//!   {name, email} or null (who commits here are by, also outside a repository: "You"), head
//!   (the commit checked out, or null)}}`.
//! - `collab_sync {stage: committing|fetching|merging|pushing|downloading|done|failed,
//!   message?, report?: {committed?, incoming: [{commit, author, subject}], fastForward,
//!   pushed, conflicts: [{path, what, kept}], renamed: [[from, to]], summary: [line],
//!   branch?}}`; `collab_pull_request {url, created}`.
//! - The answers named with their commands above: `models`, `cast`, `recipe`, `workflow`,
//!   `shot_picture`, `shot_followed`, `take_read`, `takes_read`, `media_make_plan`,
//!   `media_making`, `media_made`, `media_make_cancelled`, `media_carry_plan`,
//!   `media_carried`, `preset_export`, `preset_inspect`, `preset_applied`.
//!
//! ## The state snapshot
//!
//! `{document, timeline, revision, canUndo, canRedo, duration, sourceVersion,
//! projectionStatus, importWarnings?, pendingProposal, proposalTimeline, proposals,
//! proposalTimelines, revisionTime?}`:
//!
//! - `pendingProposal` is the first open proposal, else the newest settled one: `{status:
//!   pending|accepted|rejected, proposalId, baseRevision, script?, source?, documentDiff?,
//!   freshness?: stale, appliedRevision?}`; `proposalTimeline` is its timeline.
//! - `proposals: [{proposalId, status: pending|accepted|rejected, freshness: current|stale,
//!   baseRevision, appliedRevision?, script, source, documentDiff, author: {kind:
//!   kerf|mcp|person|qc, name, id?, role?}, version, supersedes (the proposalId this version
//!   replaced, or null), versions: [{version, proposalId, baseRevision, script, source?,
//!   documentDiff}] (oldest first, the current one included), staged? (ISO 8601)}]`.
//! - `proposalTimelines: {proposalId: timeline}`: every open proposal's, and its earlier
//!   versions' made on this revision.
//! - `revisionTime` is the ISO 8601 time of the current revision.
//! - `timeline.sequences` lists every sequence: `[{id, name, width, height, rate, fps,
//!   dropFrame, duration}]`; `timeline.tracks[]` carry `role` (the track's role or its kind's
//!   default), `roleAssigned` and `syncLocked`; each keyframe carries `frame`
//!   (clip-relative, as edits send it back) beside `at` (seconds), `value` and `interp`.
//!
//! # Pictures
//!
//! [`Bridge::frame`] takes the latest playback frame only ([`Frame`]: RGBA8). [`Bridge::signal`]
//! takes the frame `scope_signal` drew ([`Signal`]: 16-bit little-endian RGBA in BT.2020, on
//! curve 1 (PQ) or 2 (HLG)). Neither copies the pixels.
//!
//! # Without a bridge
//!
//! These are synchronous, and can be called before a bridge exists:
//!
//! - The command registry (kerf-commands), pure: [`commands_catalog`], [`commands_states`],
//!   [`commands_run`].
//! - The catalog: [`fx_catalog`], [`fx_catalog_all`], [`clip_properties`].
//! - [`about`]; [`update_check`], which asks the network: call it off the UI thread.
//! - [`ffmpeg_bin`]: the FFmpeg kerfd runs, for the editor's own pictures (found once, by
//!   running the candidates: call it off the UI thread the first time).
//! - Crash reports: [`crash_install`], [`crash_fatal`], [`crash_reports`].
//! - The system keychain: [`secret`].
//! - The project launcher, local: [`project_inspect`], [`preset_inspect`] (which probes files
//!   with ffprobe and may take a moment).
//! - Serving Kerf's tools over MCP from the editor's own binary: [`serve_mcp`].
//!
//! ## Project tasks
//!
//! Project tasks may take a while: [`Task::start`] runs one on a thread of its own, and
//! [`Task::status`] is polled from the UI thread. Dropping a task leaves one still running to
//! finish on its own. The request is one of:
//!
//! - `{type: "create", name, location, template: standard|vertical|square, rate:
//!   "23.976"|"24"|"25"|"29.97"|"30"|"60", git, remote?, github?: {name?, private}, kerfd?,
//!   preset?: {path, bindings: {slotId: {file, from? (a time such as "2s" or "48f")}}, texts?:
//!   {clipId: text}}}` makes the folder and, with a preset, copies it into `presets/`, copies
//!   each file into `media/`, imports them with kerfd and applies the preset with those
//!   bindings as the project's first edit.
//! - `{type: "clone", url, location}`.
//! - `{type: "tools"}` -> `{git, gitLfs, gh, ghUser}`.
//!
//! The status is `{status: "running", message?}`, `{status: "done", path?, remote?,
//! warnings?, ...}` or `{status: "failed", error}`.

// No unsafe code in the library; the crate's empty `[lints]` table is for a Windows test.
#![forbid(unsafe_code)]

mod analysis;
mod claude_code;
mod cloud_session;
mod collab;
mod conversations;
mod looks;
mod media;
mod meter;
mod paperwork;
mod presets;
mod preview_turns;
mod previews;
mod qc;
mod queue;
mod recipes;
mod secrets;
mod sessions;
mod steps;
mod takes;
mod tasks;
#[cfg(test)]
mod tests_support;
mod tracks;
mod updates;
mod workflows;

pub use claude_code::SERVE_MCP;
pub use tasks::Task;

use kerf_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, AgentTool, OpenAiApiMode, ProviderName,
    ReasoningEffort,
};
use kerf_mcp::inbound;
use kerf_mcp::permissions::{Mode, Permissions};
use kerf_mcp::KerfMcpRouter;
use kerf_model::{
    AudioInfo, Clip, Content, Id, Media, Project, Ratio, Sequence, Time, Track, TrackKind,
    VideoInfo,
};
use kerf_playback::{PlaybackConfig, PlaybackEvent, PlaybackFrame, PlaybackSession, PlaybackState};
use kerf_runtime::{RuntimeClient, StateSnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct AgentSettings {
    #[serde(default)]
    provider: Option<ChatProvider>,
    #[serde(rename = "apiMode", default)]
    api_mode: Option<OpenAiApiMode>,
    #[serde(rename = "apiKey", default)]
    api_key: Option<String>,
    #[serde(rename = "baseUrl", default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "mcpEnabled", default)]
    mcp_enabled: bool,
    #[serde(rename = "mcpConfigVersion", default)]
    mcp_config_version: u64,
    /// Use Kerf Cloud, the hosted model, instead of a key of one's own.
    #[serde(default)]
    cloud: bool,
}

/// Who answers in Kerf chat: a model API with a key, or the person's own
/// Claude Code, signed in to their Claude account.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ChatProvider {
    Anthropic,
    Openai,
    ClaudeCode,
}

impl ChatProvider {
    fn api(self) -> Option<ProviderName> {
        match self {
            Self::Anthropic => Some(ProviderName::Anthropic),
            Self::Openai => Some(ProviderName::Openai),
            Self::ClaudeCode => None,
        }
    }
}

/// The agent behind a chat: Kerf's own loop on a model API, with the router
/// its tools run on, or Claude Code.
enum Agent {
    Native(Box<AgentSession>, KerfMcpRouter),
    ClaudeCode(claude_code::ClaudeCodeSession),
}

impl Agent {
    fn abort_handle(&self) -> Arc<AtomicBool> {
        match self {
            Self::Native(session, _) => session.abort_handle(),
            Self::ClaudeCode(session) => session.abort_handle(),
        }
    }

    /// What its context is counted in: the model's tokens on Claude Code,
    /// which says how many a request read, or Kerf's own measure of its
    /// input allowance.
    fn context_unit(&self) -> &'static str {
        match self {
            Self::Native(..) => "units",
            Self::ClaudeCode(_) => "tokens",
        }
    }

    /// What the next turn may do without asking: the mode the person chose
    /// in the editor, or their own file's with none. The session goes on, so
    /// a change of mode keeps what the agent knows of the conversation.
    fn set_permissions(&mut self, mode: Option<Mode>) {
        match self {
            Self::Native(_, router) => router.set_permissions(mode.map(chosen_permissions)),
            Self::ClaudeCode(session) => session.set_permissions(mode),
        }
    }

    fn run_with_events(&mut self, text: &str, emit: impl FnMut(AgentEvent)) -> Result<(), String> {
        match self {
            Self::Native(session, _) => session
                .run_with_events(text, emit)
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Self::ClaudeCode(session) => session.run_with_events(text, emit),
        }
    }
}

/// The person's permissions with the mode they chose in the editor in place
/// of their file's: how many makings run on their own stays the file's say.
fn chosen_permissions(mode: Mode) -> Permissions {
    Permissions {
        mode,
        ..Permissions::load().unwrap_or_default()
    }
}

/// The mode a request names: `ask`, `accept-edits` or `auto`. A name that is
/// none of them asks, never lets more through.
fn permission_mode(name: Option<&str>) -> Option<Mode> {
    name.map(|name| Mode::named(name).unwrap_or(Mode::Ask))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Refresh,
    Reconnect,
    /// Render a preview of the program, or of what the pending proposal
    /// `proposalId` would make of it, over `start`..`start + duration` when
    /// given, and play it in place of the program.
    Preview {
        #[serde(rename = "proposalId", default)]
        proposal_id: Option<String>,
        #[serde(default)]
        start: Option<f64>,
        #[serde(default)]
        duration: Option<f64>,
    },
    /// Stop the preview render running: `preview{status:"stopped"}`.
    PreviewStop,
    /// Stop any preview render and play the program again:
    /// `preview{status:"exited"}`, then `viewer_reset`.
    PreviewExit,
    Import {
        path: String,
        /// Add it to the project's media without cutting it into the program.
        #[serde(default, rename = "mediaOnly")]
        media_only: bool,
    },
    /// Typed edits on the model, applied atomically.
    Edit {
        ops: Vec<kerf_runtime::EditOp>,
        revision: u64,
    },
    Undo {
        revision: u64,
    },
    Redo {
        revision: u64,
    },
    Accept {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    Reject {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    /// Bring a stale proposal onto the project as it is, as its next
    /// version.
    Forward {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    Agent {
        text: String,
        /// The clip, range, or playhead the request is about.
        #[serde(default)]
        focus: Option<kerf_agent::Focus>,
        /// The conversation the turn goes on in; none starts a new one.
        #[serde(default)]
        conversation: Option<String>,
        /// Where the playhead was when it was asked, in seconds.
        #[serde(default)]
        at: Option<f64>,
        /// What Kerf may do without asking, as chosen in the editor: `ask`,
        /// `accept-edits` or `auto`. None is the person's own file's say.
        #[serde(default)]
        permissions: Option<String>,
        #[serde(flatten)]
        settings: AgentSettings,
    },
    /// The same request made by several agents at once, each staging its own
    /// proposal in its own thread: `agent_version` events follow each one.
    AgentVersions {
        text: String,
        #[serde(default = "default_versions")]
        count: usize,
        #[serde(flatten)]
        settings: AgentSettings,
    },
    Abort,
    Play,
    Pause,
    Seek {
        position: f64,
        /// Pointer is still moving. The playback worker shows the nearest
        /// picture immediately; a later seek without this flag is exact.
        #[serde(default)]
        scrub: bool,
    },
    Volume {
        value: f32,
    },
    /// Queue a render: `export {status: queued, jobId, …}`, then its
    /// turn on the render queue's thread.
    Export(queue::ExportRequest),
    /// Take a queued render out of the queue, or stop the one rendering.
    ExportCancel {
        #[serde(rename = "jobId")]
        job_id: String,
    },
    /// The render jobs kerfd keeps: `render_queue`.
    RenderQueue,
    /// The captions as SRT or WebVTT.
    ExportCaptions(paperwork::CaptionsRequest),
    /// The programme measured to EBU R128: `loudness`.
    Loudness {
        #[serde(default)]
        sequence: Option<String>,
    },
    /// An HDR sequence's frame for the Finish tray's scopes: `scope_signal`.
    ScopeSignal {
        time: f64,
        #[serde(default)]
        sequence: Option<String>,
    },
    /// What the media files say of themselves: `media_info`.
    MediaInfo {
        #[serde(default)]
        media: Option<String>,
    },
    /// Where a media file cuts, pauses and is loudest: `media_index`.
    MediaIndex {
        media: String,
    },
    /// Keep a clip's picture effects as an effects preset: `look_saved`.
    LookSave {
        clip: String,
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// Put an effects preset on a clip, as one edit.
    LookApply {
        clip: String,
        path: String,
    },
    PresetExport {
        path: String,
    },
    PresetInspect {
        path: String,
    },
    PresetApply {
        path: String,
        bindings: Value,
        texts: Value,
        revision: u64,
    },
    /// List the project's preset library: `preset_library`.
    PresetLibrary,
    /// Save the program as `presets/<name>.kerfpreset`: `preset_saved`.
    PresetSave {
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// Swap the file behind a media id, keeping every clip's timing. A
    /// shared project copies the file into `media/` first.
    MediaReplace {
        media: String,
        path: String,
    },
    /// Make a media's file from its steps: `media_make_plan` and nothing
    /// run unless `confirmed`, which a person's yes to that plan sends.
    MediaMake {
        media: String,
        #[serde(default)]
        confirmed: bool,
        /// How many takes to make, each with its seeds one on from the last: one where it
        /// is not said.
        #[serde(default)]
        rolls: Option<u32>,
    },
    /// Read the project's cast file: `cast`.
    CastGet,
    /// Write `entries` as the project's cast file, in place of the one there: `cast`, with
    /// the shots that want making for it.
    CastSet {
        entries: Vec<media::CastEntry>,
    },
    /// Carry the steps of a media that names a recipe file into the document, under its
    /// line: `media_carry_plan` and nothing written unless `confirmed`, which a person's yes
    /// to that plan sends; then one edit and `media_carried`.
    MediaCarry {
        media: String,
        #[serde(default)]
        confirmed: bool,
    },
    /// Stop the making of a media that is running: `media_make_cancelled`.
    MediaMakeCancel {
        media: String,
    },
    /// How a media is made, as a graph of its steps and what they read:
    /// `recipe`. Nothing is run.
    RecipeGet {
        media: String,
    },
    /// Change a media's steps, where they still read as the editor read them:
    /// one name on one line, or the whole text, as `step` lines applied as one
    /// edit. `recipe` then, as they now read.
    RecipeEdit(recipes::Edit),
    /// Set a signal clip's keys from the onsets of the sound another clip of
    /// its sequence plays ([`kerf_shots::signals::from_sound`]): one edit of
    /// the project, as drawing its curve is.
    SignalFromSound {
        clip: String,
        of: String,
    },
    /// Draw the picture a person points on to read a clip from footage
    /// ([`kerf_shots::tracks`]): its sequence on the clip's first frame, as a
    /// `shot_picture` naming the file.
    ShotPicture {
        clip: String,
        of: tracks::Of,
    },
    /// Have a clip follow the point pointed at (`moves`), or cut out of it
    /// what the points are on (`matte`): one edit of the project, then
    /// `shot_followed`, or `error`.
    ShotFollow(tracks::Follow),
    /// Hold a media's takes to its guide by where its subjects are: a
    /// `take_read` for each as it is read, then `takes_read`, or `error`.
    TakesRead {
        media: String,
    },
    /// The workflow a `generate` step's model is run with, where its program
    /// is kerf-comfy: `workflow`, every node and input of it.
    WorkflowGet {
        media: String,
        line: usize,
    },
    /// Set a value of that workflow, in the shot's own copy of it beside its
    /// recipe: `workflow`, then `recipe`.
    WorkflowEdit(workflows::Edit),
    /// The person's models a recipe's `generate` can name, from their own
    /// models file: `models {file, exists, models, error?}`.
    Models,
    /// Load a `project.kerf` changed outside Kerf as one revision.
    ProjectionImport,
    /// Write the editor's project over a `project.kerf` changed outside Kerf.
    ProjectionRepair,
    // Collaboration over Git, run by the collaboration worker.
    CollabStatus,
    CollabSync {
        #[serde(default)]
        message: Option<String>,
    },
    CollabLive {
        enabled: bool,
        /// Seconds between syncs.
        #[serde(default)]
        interval: Option<f64>,
    },
    CollabCommit {
        #[serde(default)]
        message: String,
    },
    CollabBranch {
        name: String,
        #[serde(default)]
        create: bool,
    },
    CollabPullRequest {
        title: String,
        #[serde(default)]
        body: Option<String>,
    },
    CollabPublish {
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        github: Option<kerf_collab::scaffold::GitHubRepository>,
    },
    CollabConsolidate,
    /// Republish the state with the viewer's media reopened: files changed
    /// on disk under an unchanged document. Sent by the bridge's own
    /// workers only.
    #[serde(skip)]
    Reload,
    /// How `effects` would look on `clip`, on its picture at sequence time
    /// `at`, or a generator alone on black: an `effect_preview` event per
    /// entry that has a picture, `width` pixels wide. A newer request
    /// supersedes it.
    EffectPreviews(previews::Request),
    /// The conversations kept beside the project: list, open, archive,
    /// rename or delete one (`conversations`, `conversation`).
    Conversations(conversations::Request),
    /// Kerf's QC pass over a sequence: `check`, whose failing rows the
    /// conversation offers as things to ask Kerf.
    Check {
        #[serde(default)]
        sequence: Option<String>,
    },
    /// Hear only these audio tracks; empty hears them all. Monitoring
    /// only: the project never changes.
    Solo {
        #[serde(default)]
        tracks: Vec<String>,
    },
    /// Keep a Kerf Cloud token and read its account.
    CloudSignIn {
        token: String,
    },
    CloudSignOut,
    CloudRefresh,
}

/// What the viewer plays: the project itself, or a rendered preview of it,
/// the file that preview plays, and the proposal it previews. Both have
/// their media paths resolved.
#[derive(Clone)]
enum MediaProject {
    Native(Project),
    Rendered(Project, PathBuf, Option<String>),
}

/// One render the runtime worker hands to an export thread.
struct RenderRequest {
    quality: String,
    /// Played in place of the program once rendered.
    preview: bool,
    proposal: Option<String>,
    range: Option<kerf_runtime::RenderRange>,
    sequence: Option<String>,
}

impl RenderRequest {
    /// A `preview` or `export` event about this render.
    fn event(&self, status: &str) -> Value {
        let mut event = json!({
            "type": if self.preview { "preview" } else { "export" },
            "status": status,
        });
        if let Some(proposal) = &self.proposal {
            event["proposalId"] = json!(proposal);
        }
        event
    }
}

struct PendingSeek {
    position: f64,
    exact: bool,
}

struct Shared {
    closed: AtomicBool,
    revision: AtomicU64,
    agent_busy: AtomicBool,
    /// A viewer preview is rendering; exports go through `renders`.
    preview_busy: AtomicBool,
    abort: Mutex<Option<Arc<AtomicBool>>>,
    /// The agents making versions of a request, while they run.
    version_aborts: Mutex<Vec<Arc<AtomicBool>>>,
    frame: Mutex<Option<PlaybackFrame>>,
    seek: Mutex<Option<PendingSeek>>,
    project: Mutex<Option<MediaProject>>,
    /// The project as edited, its media paths resolved.
    model: Mutex<Option<Project>>,
    /// The latest effect preview request; older ones stop.
    previews: AtomicU64,
    /// The project directory, which relative media paths are resolved
    /// against.
    directory: PathBuf,
    /// The media status last emitted.
    media_status: Mutex<media::MediaStatus>,
    /// Copies into `media/` still running; live sync waits for them.
    copies: AtomicUsize,
    /// The cancel marker of the preview render running, so closing the
    /// bridge stops it.
    preview_cancel: Mutex<Option<PathBuf>>,
    /// The order previews and their stops were sent in, which says which
    /// previews `preview_stop` and `preview_exit` asked to stop.
    preview_turns: preview_turns::PreviewTurns,
    /// Preview renders nothing shows any more whose files could not be
    /// removed yet: on Windows a playback worker still shutting down keeps
    /// its file open. The media worker tries them again.
    stale_previews: Mutex<Vec<PathBuf>>,
    /// The preview file the media worker plays, so closing the bridge can
    /// remove it even when the process exits before that worker stops.
    shown_preview: Mutex<Option<PathBuf>>,
    /// A QC check is running.
    check_busy: AtomicBool,
    /// Cancels the QC check running, so closing the bridge stops it.
    check_cancel: Mutex<Option<kerf_engine::CancellationToken>>,
    /// The makings running, by the media each makes: what cancels it.
    makings: Mutex<std::collections::BTreeMap<String, kerf_engine::CancellationToken>>,
    /// The makings another program runs on this project that are shown, by the media each
    /// makes: the take and the line last shown of it.
    others_makings: Mutex<std::collections::BTreeMap<String, (u32, String)>>,
    /// The render queue the export thread works through.
    renders: queue::Queue,
    /// Loudness, media info and media index work.
    analysis: analysis::Analysis,
    /// The workflow last said of each step a graph has open.
    workflows: workflows::Said,
}

/// One project open in the editor: commands in through [`Bridge::send`], events out through
/// [`Bridge::poll`], and pictures through [`Bridge::frame`] and [`Bridge::signal`]. Every
/// call returns at once; kerfd, playback, the agent and the other workers run on threads of
/// their own. Dropping it closes the project without waiting for them.
pub struct Bridge {
    /// The project folder, where its conversations are kept.
    directory: PathBuf,
    events_tx: SyncSender<Value>,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    collab: SyncSender<Command>,
    prompts: SyncSender<AgentRequest>,
    events: Receiver<Value>,
    shared: Arc<Shared>,
}

struct AgentRequest {
    text: String,
    settings: AgentSettings,
    /// More than none: this many agents make their own version of the request.
    versions: usize,
    /// The person's own words, the conversation the turn goes on in (none
    /// starts one), and the playhead when it was asked: every turn of one
    /// agent is kept; one of several versions is not.
    turn: Option<(String, Option<String>, Option<f64>)>,
    /// What this turn may do without asking; none is the person's file's say.
    permissions: Option<Mode>,
}

fn emit(tx: &SyncSender<Value>, value: Value) {
    let _ = tx.send(value);
}

fn error(tx: &SyncSender<Value>, value: impl std::fmt::Display) {
    emit(tx, json!({"type":"error", "message":value.to_string()}));
}

fn key() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    format!(
        "editor-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn publish(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &SyncSender<Value>,
    previous: &mut Option<Value>,
) {
    publish_snapshot(state, shared, tx, previous, false);
}

fn publish_force(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &SyncSender<Value>,
    previous: &mut Option<Value>,
) {
    publish_snapshot(state, shared, tx, previous, true);
}

fn publish_snapshot(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &SyncSender<Value>,
    previous: &mut Option<Value>,
    force: bool,
) {
    let snapshot = serde_json::to_value(&state).unwrap_or_default();
    if !force && previous.as_ref() == Some(&snapshot) {
        return;
    }

    // Proposal-only refreshes must not reopen decoders or interrupt playback.
    let project_changed = previous
        .as_ref()
        .and_then(|snapshot| snapshot.get("document"))
        .and_then(Value::as_str)
        != Some(state.document.as_str());
    if project_changed {
        // The document may use looks another process added to the project.
        let _ = kerf_model::look::refresh_project_looks(&shared.directory);
        match kerf_runtime::parse_document(&state.document) {
            Ok(mut project) => {
                media::publish_status(shared, tx, &project);
                kerf_runtime::resolve_media_paths(&mut project, &shared.directory);
                // What new media are, for the bin and the inspector.
                analysis::media_changed(shared, tx, &project);
                if let Ok(mut model) = shared.model.lock() {
                    *model = Some(project.clone());
                }
                if let Ok(mut slot) = shared.project.lock() {
                    shared.revision.fetch_add(1, Ordering::SeqCst);
                    install_project(shared, &mut slot, MediaProject::Native(project));
                }
            }
            Err(err) => error(tx, err.message),
        }
    }
    *previous = Some(snapshot.clone());
    emit(
        tx,
        json!({"type":"state", "snapshot":with_proposal_timeline(snapshot, &state)}),
    );
}

/// Each open proposal's timeline beside the current one, so the editor can
/// show proposals on the timeline before they are accepted:
/// `proposalTimelines` by proposal id (an open proposal's earlier versions
/// too, while they were made on this revision), and `proposalTimeline` for
/// the pending proposal. The list fields a snapshot leaves out while empty
/// are filled in, so the editor always reads the same shape.
fn with_proposal_timeline(mut snapshot: Value, state: &StateSnapshot) -> Value {
    if let Some(kerf_runtime::PendingProposal::Pending { script, .. }) = &state.pending_proposal {
        if let Ok(timeline) = kerf_runtime::proposal_timeline(&state.document, script) {
            snapshot["proposalTimeline"] = timeline;
        }
    }
    let mut timelines = serde_json::Map::new();
    for entry in state
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
    {
        let earlier = entry.versions.iter().filter(|version| {
            version.proposal_id != entry.proposal_id && version.base_revision == state.revision
        });
        let scripts = std::iter::once((&entry.proposal_id, &entry.script))
            .chain(earlier.map(|version| (&version.proposal_id, &version.script)));
        for (id, script) in scripts {
            if let Ok(timeline) = kerf_runtime::proposal_timeline(&state.document, script) {
                timelines.insert(id.clone(), timeline);
            }
        }
    }
    snapshot["proposalTimelines"] = Value::Object(timelines);
    if snapshot.get("proposals").is_none() {
        snapshot["proposals"] = json!([]);
    }
    snapshot
}

fn connection_event(tx: &SyncSender<Value>, status: &str, message: Option<&str>) {
    let mut event = json!({"type":"connection", "status":status});
    if let Some(message) = message {
        event["message"] = Value::String(message.to_owned());
    }
    emit(tx, event);
}

fn connect_runtime(
    directory: &std::path::Path,
    binary: &std::path::Path,
    tx: &SyncSender<Value>,
    shared: &Arc<Shared>,
    previous: &mut Option<Value>,
) -> Option<RuntimeClient> {
    connection_event(tx, "connecting", None);
    let bootstrap = match RuntimeClient::connect_or_spawn(directory, Some(binary)) {
        Ok(client) => client,
        Err(error) => {
            connection_event(tx, "disconnected", Some(&error.message));
            return None;
        }
    };
    let state = match bootstrap.get_state() {
        Ok(state) => state,
        Err(error) => {
            connection_event(tx, "disconnected", Some(&error.message));
            return None;
        }
    };
    // Do not retain the binary on the steady-state client: a daemon that
    // disappears must be reported as disconnected until the user reconnects.
    let client = match RuntimeClient::new(directory) {
        Ok(client) => client,
        Err(error) => {
            connection_event(tx, "disconnected", Some(&error.message));
            return None;
        }
    };
    // Rebuild the viewer after a reconnect even when the canonical snapshot
    // did not change while the transport was down.
    *previous = None;
    publish(state, shared, tx, previous);
    connection_event(tx, "ready", None);
    Some(client)
}

fn runtime_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<Command>,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    tx: SyncSender<Value>,
    shared: Arc<Shared>,
) {
    let mut previous = None;
    let mut client = connect_runtime(&directory, &binary, &tx, &shared, &mut previous);
    // Hold kerfd for as long as the editor is open; it exits on its own
    // once the editor and every other client are gone.
    let _lease = RuntimeClient::new(&directory)
        .ok()
        .map(|lease| lease.hold());
    while !shared.closed.load(Ordering::Relaxed) {
        // What `kerf make` or an agent's own kerf-mcp is making of this project shows as the
        // editor's own makings do.
        media::follow_others_makings(&shared, std::process::id(), &tx);
        let command = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(active_client) = client.as_ref() {
                    match active_client.get_state() {
                        Ok(state) => publish(state, &shared, &tx, &mut previous),
                        Err(error) if error.is_transport() => {
                            let _ = media.try_send(Command::Pause);
                            connection_event(&tx, "disconnected", Some(&error.message));
                            client = None;
                        }
                        Err(_) => {}
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        // A preview's place among those sent, which a stop goes by.
        let preview_turn =
            matches!(command, Command::Preview { .. }).then(|| shared.preview_turns.take());
        if matches!(command, Command::Reconnect) {
            if client.is_some() {
                let _ = media.try_send(Command::Pause);
            }
            client = connect_runtime(&directory, &binary, &tx, &shared, &mut previous);
            continue;
        }
        let Some(active_client) = client.clone() else {
            error(&tx, "Runtime is disconnected; reconnect to continue");
            if let (Some(turn), Command::Preview { proposal_id, .. }) = (preview_turn, command) {
                preview_unstarted(&shared, &tx, turn, proposal_id);
            }
            continue;
        };
        let operation = key();
        let force_refresh = matches!(&command, Command::Refresh);
        // Render queue, paperwork files and saved looks answer with their
        // own events, never a state.
        let side = match command {
            Command::Export(request) => {
                let renders: Arc<dyn queue::Renders> = Arc::new(active_client.clone());
                Err(("export", queue::submit(&shared, renders, &request, &tx)))
            }
            Command::ExportCancel { job_id } => Err((
                "export_cancel",
                queue::cancel(&shared, &active_client, &job_id, &tx),
            )),
            Command::RenderQueue => {
                Err(("render_queue", queue::publish(&shared, &active_client, &tx)))
            }
            Command::ExportCaptions(request) => Err((
                "export_captions",
                paperwork::export_captions(&active_client, &directory, &request, &tx),
            )),
            Command::LookSave {
                clip,
                name,
                replace,
            } => Err((
                "look_save",
                looks::save_from(&active_client, &directory, &clip, &name, replace, &tx),
            )),
            command => Ok(command),
        };
        let command = match side {
            Ok(command) => command,
            Err((name, result)) => {
                match result {
                    Ok(()) => {}
                    Err(error) if error.is_transport() => {
                        let _ = media.try_send(Command::Pause);
                        connection_event(&tx, "disconnected", Some(&error.message));
                        client = None;
                    }
                    Err(error) => {
                        queue::refused(&tx, name, &error);
                    }
                }
                continue;
            }
        };
        // What follows a command's state, and the command a refusal names.
        let mut then: Option<Value> = None;
        let mut refusal: Option<&'static str> = None;
        let result = match command {
            Command::Refresh => active_client.get_state(),
            Command::Reload => {
                previous = None;
                active_client.get_state()
            }
            // A shared project copies the file into media/ on another
            // thread, then imports the copy.
            Command::Import { path, media_only }
                if media::needs_copy(&directory, Path::new(&path)) =>
            {
                media::copy_then(&shared, &path, &commands, &tx, media::probe, move |path| {
                    Command::Import { path, media_only }
                });
                continue;
            }
            Command::Import { path, media_only } => active_client.get_state().and_then(|state| {
                if media_only {
                    active_client.add_media_request("ui", &operation, &path, state.revision)
                } else {
                    active_client.import_media_request("ui", &operation, &path, state.revision)
                }
                .map(|(state, _, _)| state)
            }),
            Command::MediaReplace { media, path }
                if media::needs_copy(&directory, Path::new(&path)) =>
            {
                media::copy_then(&shared, &path, &commands, &tx, media::probe, move |path| {
                    Command::MediaReplace { media, path }
                });
                continue;
            }
            Command::MediaMake {
                media,
                confirmed,
                rolls,
            } => {
                let rolls = rolls.unwrap_or(1).clamp(1, 16);
                media::make(
                    &shared,
                    &active_client,
                    &media,
                    confirmed,
                    rolls,
                    &commands,
                    &tx,
                );
                continue;
            }
            Command::CastGet => {
                media::cast(&shared, &active_client, None, &tx);
                continue;
            }
            Command::CastSet { entries } => {
                media::cast(&shared, &active_client, Some(entries), &tx);
                continue;
            }
            Command::MediaCarry { media, confirmed } => {
                recipes::carry(&shared, &active_client, &media, confirmed, &tx);
                continue;
            }
            Command::RecipeGet { media } => {
                recipes::get(&shared, &active_client, &media, &tx);
                continue;
            }
            Command::RecipeEdit(edit) => {
                recipes::edit(&shared, &active_client, &edit, &tx);
                continue;
            }
            Command::WorkflowGet { media, line } => {
                workflows::get(&shared, &active_client, &media, line, &tx);
                continue;
            }
            Command::ShotPicture { clip, of } => {
                tracks::picture(&shared, &active_client, clip, of, &tx);
                continue;
            }
            Command::ShotFollow(asked) => {
                tracks::follow(&shared, &active_client, asked, &tx);
                continue;
            }
            Command::TakesRead { media } => {
                takes::read(&shared, &active_client, media, &tx);
                continue;
            }
            Command::WorkflowEdit(edit) => {
                workflows::edit(&shared, &active_client, &edit, &tx);
                continue;
            }
            Command::SignalFromSound { clip, of } => {
                active_client
                    .get_document(false)
                    .and_then(|(revision, text)| {
                        let project = kerf_runtime::parse_document(&text)?;
                        let read = kerf_shots::signals::from_sound(
                            &project,
                            &shared.directory,
                            &of,
                            &clip,
                            None,
                        )
                        .map_err(|why| kerf_runtime::RuntimeError::new("SIGNAL_FAILED", why))?;
                        active_client.apply_script_request(
                            "ui",
                            &operation,
                            &read.script(&clip),
                            revision,
                        )
                    })
            }
            Command::MediaReplace { media, path } => active_client.get_state().and_then(|state| {
                active_client.replace_media_request("ui", &operation, &media, &path, state.revision)
            }),
            Command::ProjectionImport => active_client.get_state().and_then(|state| {
                active_client.import_projection_request("ui", &operation, state.revision)
            }),
            Command::ProjectionRepair => active_client.repair_projection(),
            Command::LookApply { clip, path } => {
                let opened = active_client
                    .get_document(false)
                    .and_then(|(revision, text)| {
                        kerf_runtime::parse_document(&text).map(|project| (revision, project))
                    });
                match opened {
                    Ok((revision, project)) => {
                        match looks::apply(&directory, &project, &clip, &path) {
                            Ok((ops, event)) => {
                                then = Some(event);
                                refusal = Some("look_apply");
                                active_client.apply_ops_request("ui", &operation, ops, revision)
                            }
                            Err(message) => {
                                looks::refused(&tx, "look_apply", &message);
                                continue;
                            }
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Command::PresetSave { name, replace } => {
                let saved = active_client
                    .get_document(false)
                    .and_then(|(_, text)| kerf_runtime::parse_document(&text))
                    .map_err(|error| error.message)
                    .and_then(|project| presets::export_program(&project, Some(&name)))
                    .and_then(|document| presets::save(&directory, &name, &document, replace));
                match saved {
                    Ok(path) => emit(&tx, json!({"type":"preset_saved", "path":path})),
                    Err(message) => error(&tx, message),
                }
                continue;
            }
            Command::Edit { ops, revision } => {
                active_client.apply_ops_request("ui", &operation, ops, revision)
            }
            Command::Undo { revision } => active_client.undo_request("ui", &operation, revision),
            Command::Redo { revision } => active_client.redo_request("ui", &operation, revision),
            Command::Accept {
                proposal_id,
                revision,
            } => active_client.accept_proposal(&proposal_id, "ui", &operation, revision),
            Command::Reject {
                proposal_id,
                revision,
            } => active_client.reject_proposal_request(&proposal_id, "ui", &operation, revision),
            Command::Forward {
                proposal_id,
                revision,
            } => {
                // Its refusal is said as one, with the clip that stopped it.
                refusal = Some("forward");
                active_client.forward_proposal(&operation, revision, &proposal_id)
            }
            Command::PresetExport { path } => {
                match active_client
                    .get_document(false)
                    .and_then(|(_, text)| kerf_runtime::parse_document(&text))
                {
                    Ok(project) => match presets::export_program(&project, None) {
                        Ok(document) => emit(
                            &tx,
                            json!({"type":"preset_export", "path":path, "document":document}),
                        ),
                        Err(message) => error(&tx, message),
                    },
                    Err(err) => error(&tx, err.message),
                }
                continue;
            }
            Command::PresetInspect { path } => {
                match KerfMcpRouter::new(&directory, Some(&binary)) {
                    Ok(router) => {
                        let response = router.call("look", json!({"preset":path}));
                        if response["ok"] == true {
                            emit(
                                &tx,
                                json!({"type":"preset_inspect", "path":path, "preset":response["result"]}),
                            );
                        } else {
                            error(
                                &tx,
                                response["error"]["message"]
                                    .as_str()
                                    .unwrap_or("cannot read preset"),
                            );
                        }
                    }
                    Err(message) => error(&tx, message),
                }
                continue;
            }
            Command::PresetApply {
                path,
                bindings,
                texts,
                revision,
            } => {
                match active_client.get_state() {
                    Ok(state) if state.revision == revision => {
                        match KerfMcpRouter::new(&directory, Some(&binary)) {
                            Ok(router) => {
                                let response = router.call(
                                    "edit",
                                    json!({"preset":path,"bindings":bindings,"texts":texts}),
                                );
                                if response["ok"] == true {
                                    emit(
                                        &tx,
                                        json!({"type":"preset_applied", "path":path, "proposal":response["result"]["proposal"]}),
                                    );
                                } else {
                                    error(
                                        &tx,
                                        response["error"]["message"]
                                            .as_str()
                                            .unwrap_or("cannot apply preset"),
                                    );
                                }
                            }
                            Err(message) => error(&tx, message),
                        }
                    }
                    Ok(_) => error(
                        &tx,
                        "The project changed while choosing preset media. Apply the preset again.",
                    ),
                    Err(err) => error(&tx, err.message),
                }
                // Publish the same proposal state used by the native review UI.
                active_client.get_state()
            }
            Command::Preview {
                proposal_id,
                start,
                duration,
            } => {
                let turn = preview_turn.expect("taken with the command");
                let range = match (start, duration) {
                    (None, None) => Ok(None),
                    (Some(start), Some(duration)) => {
                        kerf_runtime::RenderRange::new(start, duration)
                            .map(Some)
                            .map_err(|err| err.message)
                    }
                    _ => Err("A preview range needs both start and duration".to_owned()),
                };
                let range = match range {
                    Ok(range) => range,
                    Err(message) => {
                        error(&tx, message);
                        preview_unstarted(&shared, &tx, turn, proposal_id);
                        continue;
                    }
                };
                spawn_render(
                    RenderRequest {
                        quality: "preview".into(),
                        preview: true,
                        proposal: proposal_id,
                        range,
                        sequence: None,
                    },
                    turn,
                    &active_client,
                    &tx,
                    &shared,
                );
                continue;
            }
            _ => continue,
        };
        match result {
            Ok(state) => {
                if force_refresh {
                    publish_force(state, &shared, &tx, &mut previous);
                } else {
                    publish(state, &shared, &tx, &mut previous);
                }
                if let Some(event) = then.take() {
                    emit(&tx, event);
                }
            }
            Err(error) if error.is_transport() => {
                let _ = media.try_send(Command::Pause);
                connection_event(&tx, "disconnected", Some(&error.message));
                client = None;
            }
            Err(runtime_error) => {
                match refusal {
                    Some(name) => {
                        queue::refused(&tx, name, &runtime_error);
                    }
                    None => error(&tx, runtime_error.message),
                }
                if let Ok(state) = active_client.get_state() {
                    publish_force(state, &shared, &tx, &mut previous);
                } else {
                    let _ = media.try_send(Command::Pause);
                    connection_event(&tx, "disconnected", Some("runtime became unavailable"));
                    client = None;
                }
            }
        }
    }
}

/// A preview that ends before any render of it began: refused, or stopped
/// on its way here. One asked to stop says so, which answers the stop.
fn preview_unstarted(shared: &Shared, tx: &SyncSender<Value>, turn: u64, proposal: Option<String>) {
    if shared.preview_turns.over(turn) {
        let mut event = json!({"type":"preview","status":"stopped"});
        if let Some(proposal) = proposal {
            event["proposalId"] = json!(proposal);
        }
        emit(tx, event);
    }
}

/// Run the preview `request`, the one of `turn`, on its own thread, one at
/// a time. It plays once rendered, unless the program was edited, the
/// preview stopped, or the bridge closed meanwhile. A stop or an exit sent
/// after it and here before it keeps it from starting.
fn spawn_render(
    request: RenderRequest,
    turn: u64,
    client: &RuntimeClient,
    tx: &SyncSender<Value>,
    shared: &Arc<Shared>,
) {
    if shared.preview_turns.stopped(turn) {
        preview_unstarted(shared, tx, turn, request.proposal);
        return;
    }
    if shared.preview_busy.swap(true, Ordering::SeqCst) {
        error(tx, "A preview is already rendering");
        preview_unstarted(shared, tx, turn, request.proposal);
        return;
    }
    let client = client.clone();
    let tx = tx.clone();
    let shared = shared.clone();
    thread::spawn(move || {
        let revision = shared.revision.load(Ordering::SeqCst);
        let result = export(&client, &request, turn, &tx, &shared);
        let rendered = result.as_ref().ok().and_then(|(output, document)| {
            let program = kerf_runtime::parse_document(document).ok()?;
            let project = rendered_preview(&program, output, request.range.as_ref())?;
            Some((project, PathBuf::from(output)))
        });
        // Whether it stopped is settled with the viewer's slot held, which
        // `preview_exit` takes after asking for the stop: an exit sent before
        // this keeps the preview out, and one sent after finds it waiting
        // there and puts the program back.
        let stopped = {
            let slot = shared.project.lock();
            let stopped = shared.preview_turns.over(turn);
            // A preview of a program edited since, one stopped, or one for a
            // closed bridge, is never shown.
            let shown = match (&rendered, slot) {
                (Some((project, file)), Ok(mut slot))
                    if !stopped
                        && !shared.closed.load(Ordering::SeqCst)
                        && shared.revision.load(Ordering::SeqCst) == revision =>
                {
                    let project = MediaProject::Rendered(
                        project.clone(),
                        file.clone(),
                        request.proposal.clone(),
                    );
                    install_project(&shared, &mut slot, project);
                    true
                }
                _ => false,
            };
            if !shown {
                if let Ok((output, _)) = &result {
                    discard_preview(&shared, PathBuf::from(output));
                }
            }
            stopped
        };
        match result {
            Ok(_) => {}
            Err(err) if stopped && err.code == "CANCELLED" => {}
            Err(err) => {
                error(&tx, err.message);
                let mut event = request.event("required");
                event["message"] =
                    json!("Preview render failed. Retry after fixing the reported error.");
                emit(&tx, event);
            }
        }
        // The answer to the stop, however the render ended.
        if stopped {
            emit(&tx, request.event("stopped"));
        }
        shared.preview_busy.store(false, Ordering::SeqCst);
    });
}

/// Render the program. Returns the output path and the document rendered.
/// Closing the bridge or stopping the preview cancels a preview, and a
/// preview that fails leaves no file behind.
fn export(
    client: &RuntimeClient,
    request: &RenderRequest,
    turn: u64,
    tx: &SyncSender<Value>,
    shared: &Shared,
) -> Result<(String, String), kerf_runtime::RuntimeError> {
    let preview = request.preview;
    let (job, document, output) = client.start_render_range(
        &request.quality,
        request.range.clone(),
        request.proposal.as_deref(),
        request.sequence.as_deref(),
    )?;
    let cancel = kerf_runtime::render_cancel_path(client.directory(), &job.job_id);
    if preview {
        // The viewer's render, not an output.
        shared.renders.mark_viewer(&job.job_id);
        if let Ok(mut running) = shared.preview_cancel.lock() {
            *running = Some(cancel.clone());
        }
        // The bridge may have closed, or the preview stopped, before it
        // could see this render.
        if shared.closed.load(Ordering::SeqCst) || shared.preview_turns.stopped(turn) {
            cancel_preview(shared);
        }
    }
    let mut running = request.event("running");
    running["progress"] = json!(0.0);
    emit(tx, running.clone());
    let result = kerf_runtime::run_render_job(
        client,
        &job,
        &document,
        &output,
        request.quality == "preview",
        |progress| {
            // Progress is replaceable; never stall rendering behind a busy GUI.
            running["progress"] = json!(progress);
            let _ = tx.try_send(running.clone());
        },
    );
    if preview {
        if let Ok(mut running) = shared.preview_cancel.lock() {
            *running = None;
            // A cancel that came as the render finished outlives the job.
            let _ = std::fs::remove_file(&cancel);
        }
    }
    match result {
        Ok(()) => {
            let path = output.to_string_lossy().into_owned();
            if !preview {
                let mut event = request.event("completed");
                event["path"] = json!(path);
                event["progress"] = json!(1.0);
                emit(tx, event);
            }
            Ok((path, document))
        }
        Err(err) => {
            if preview {
                discard_preview(shared, output);
            }
            if !(err.code == "CANCELLED" && shared.preview_turns.stopped(turn)) {
                emit(tx, request.event("failed"));
            }
            Err(err)
        }
    }
}

/// `preview_stop`: stop every preview sent before it, the one rendering
/// and one still on its way to its render alike. Each says `stopped` when
/// it is over; with none to stop the answer comes at once.
///
/// Which previews it stops is settled where the stop is sent, as that is
/// what orders it among them; the rest runs on a thread of its own, since
/// the events queue may be full while the editor is sending.
fn stop_preview(shared: &Arc<Shared>, tx: &SyncSender<Value>) {
    let answered = shared.preview_turns.stop();
    let shared = shared.clone();
    let tx = tx.clone();
    thread::spawn(move || stopped_preview(&shared, answered, &tx));
}

/// What follows a stop: the render running is told, and a stop that no
/// preview answers for (`answered`) answers itself.
fn stopped_preview(shared: &Shared, answered: bool, tx: &SyncSender<Value>) {
    cancel_preview(shared);
    if !answered {
        emit(tx, json!({"type":"preview","status":"stopped"}));
    }
}

/// `preview_exit`: stop every preview sent before it, as `preview_stop`
/// does, and play the program again in place of a rendered preview, which
/// the media worker announces with `viewer_reset`.
fn exit_preview(shared: &Arc<Shared>, tx: &SyncSender<Value>) {
    shared.preview_turns.stop();
    let shared = shared.clone();
    let tx = tx.clone();
    thread::spawn(move || exited_preview(&shared, &tx));
}

/// What follows an exit, once its previews are asked to stop.
fn exited_preview(shared: &Shared, tx: &SyncSender<Value>) {
    cancel_preview(shared);
    let model = shared.model.lock().ok().and_then(|model| model.clone());
    if let (Some(model), Ok(mut slot)) = (model, shared.project.lock()) {
        // A preview that was not stopped in time is waiting in the slot or
        // showing: both are looked at with the slot held, as a preview moves
        // from one to the other and into the slot with it held.
        let showing = shared
            .shown_preview
            .lock()
            .is_ok_and(|shown| shown.is_some());
        let waiting = matches!(*slot, Some(MediaProject::Rendered(..)));
        if showing || waiting {
            // A preview finishing now is of the program as it was.
            shared.revision.fetch_add(1, Ordering::SeqCst);
            install_project(shared, &mut slot, MediaProject::Native(model));
        }
    }
    emit(tx, json!({"type":"preview","status":"exited"}));
}

/// Stop the preview render running, if there is one.
fn cancel_preview(shared: &Shared) {
    if let Ok(running) = shared.preview_cancel.lock() {
        if let Some(marker) = &*running {
            let _ = std::fs::write(marker, b"");
        }
    }
}

/// Put `project` in the media slot. A rendered preview it replaces there was
/// never opened, so its file goes now.
fn install_project(shared: &Shared, slot: &mut Option<MediaProject>, project: MediaProject) {
    if let Some(MediaProject::Rendered(_, file, _)) = slot.replace(project) {
        discard_preview(shared, file);
    }
}

/// Remove the preview the media worker plays and one still waiting for it:
/// the bridge has closed, so nothing will show either.
fn discard_previews(shared: &Shared) {
    let shown = shared
        .shown_preview
        .lock()
        .ok()
        .and_then(|mut shown| shown.take());
    if let Some(file) = shown {
        discard_preview(shared, file);
    }
    let waiting = shared.project.lock().ok().and_then(|mut slot| {
        if matches!(*slot, Some(MediaProject::Rendered(..))) {
            slot.take()
        } else {
            None
        }
    });
    if let Some(MediaProject::Rendered(_, file, _)) = waiting {
        discard_preview(shared, file);
    }
}

/// Remove a preview render nothing shows any more, or keep it for the media
/// worker to try again while it is still open. Only previews come here;
/// exports are the person's.
fn discard_preview(shared: &Shared, file: PathBuf) {
    if !remove_preview(&file) {
        if let Ok(mut stale) = shared.stale_previews.lock() {
            stale.push(file);
        }
    }
}

/// Try the previews not removed yet again; `true` once none are left.
fn sweep_previews(shared: &Shared) -> bool {
    shared.stale_previews.lock().map_or(true, |mut stale| {
        stale.retain(|file| !remove_preview(file));
        stale.is_empty()
    })
}

/// Remove a preview file; `false` while something still holds it open.
fn remove_preview(file: &Path) -> bool {
    match std::fs::remove_file(file) {
        Ok(()) => true,
        Err(err) => err.kind() == std::io::ErrorKind::NotFound,
    }
}

/// A project that plays a rendered preview of `program` in its place: all
/// of it, or the `range` rendered, at its own place on the program.
fn rendered_preview(
    program: &Project,
    output: &str,
    range: Option<&kerf_runtime::RenderRange>,
) -> Option<Project> {
    let sequence = program.sequences.first()?;
    let (start, duration) = match range {
        Some(range) => (
            Time::from_seconds(range.start)?,
            Time::from_seconds(range.duration)?,
        ),
        None => (Time::ZERO, sequence.duration()),
    };
    let id = |text: &str| Id::new(text).ok();
    let media = id("preview")?;
    let clip = |name: &str| {
        Some(Clip::new(
            id(name)?,
            Content::Media {
                media: media.clone(),
                source_in: Time::ZERO,
                speed: Ratio::ONE.into(),
            },
            start,
            duration,
        ))
    };
    let mut picture = Track::new(id("V1")?, TrackKind::Video);
    picture.clips.push(clip("preview-picture")?);
    let mut sound = Track::new(id("A1")?, TrackKind::Audio);
    sound.clips.push(clip("preview-sound")?);
    Some(Project {
        name: program.name.clone(),
        media: vec![Media {
            id: media,
            path: output.to_owned(),
            rate: sequence.rate,
            drop_frame: false,
            start: Time::ZERO,
            duration,
            video: Some(VideoInfo {
                width: sequence.width,
                height: sequence.height,
                alpha: false,
                ..Default::default()
            }),
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 48_000,
            }),
            reel: None,
            recipe: None,
            steps: Vec::new(),
            sync: None,
        }],
        sequences: vec![Sequence {
            id: id("program")?,
            name: String::new(),
            tracks: vec![picture, sound],
            markers: Vec::new(),
            start: Time::ZERO,
            ..sequence.clone()
        }],
        retired: Default::default(),
    })
}

fn dimension(value: f64) -> u32 {
    value.round().clamp(2.0, u32::MAX as f64) as u32
}

/// The viewer frame: the sequence canvas, with its longest edge at most
/// 1920 pixels.
fn preview_frame_size(project: &Project) -> (u32, u32) {
    let (width, height) = project.sequences.first().map_or((1920, 1080), |sequence| {
        (sequence.width.max(2), sequence.height.max(2))
    });
    let scale = (1920.0 / f64::from(width.max(height))).min(1.0);
    (
        dimension(f64::from(width) * scale),
        dimension(f64::from(height) * scale),
    )
}

/// With `KERF_PLAYBACK_STATS` set, a line on stderr every 5 seconds of playing: frames shown and
/// dropped, and the size they came at, for measuring playback in the real editor.
struct PlaybackStats {
    on: bool,
    since: Instant,
    shown: u64,
    dropped: u64,
    dropped_before: u64,
    size: (u32, u32),
}

impl PlaybackStats {
    fn new() -> Self {
        Self {
            on: std::env::var_os("KERF_PLAYBACK_STATS").is_some(),
            since: Instant::now(),
            shown: 0,
            dropped: 0,
            dropped_before: 0,
            size: (0, 0),
        }
    }

    fn frame(&mut self, width: u32, height: u32) {
        self.shown += 1;
        self.size = (width, height);
    }

    fn clock(&mut self, dropped: u64) {
        self.dropped = dropped;
        if !self.on || self.since.elapsed() < Duration::from_secs(5) {
            return;
        }
        let spent = self.since.elapsed().as_secs_f64();
        eprintln!(
            "playback: {:.1} shown/s, {:.1} dropped/s, {}x{}",
            self.shown as f64 / spent,
            dropped.saturating_sub(self.dropped_before) as f64 / spent,
            self.size.0,
            self.size.1
        );
        self.shown = 0;
        self.dropped_before = dropped;
        self.since = Instant::now();
    }
}

fn media_worker(rx: Receiver<Command>, tx: SyncSender<Value>, shared: Arc<Shared>) {
    let mut stats = PlaybackStats::new();
    let mut session: Option<PlaybackSession> = None;
    let mut position = 0.0_f64;
    let mut playing = false;
    let mut volume = 1.0_f32;
    let mut swept = Instant::now();
    // The tracks heard alone, kept across sessions; the program's own
    // sessions only, as a rendered preview has none of its tracks.
    let mut solo: Vec<Id> = Vec::new();
    let mut native = false;
    let mut meter = meter::Meter::default();
    while !shared.closed.load(Ordering::Relaxed) {
        for command in rx.try_iter() {
            let handle = session.as_ref().map(PlaybackSession::handle);
            let result = match command {
                Command::Solo { tracks } => {
                    solo = tracks
                        .iter()
                        .filter_map(|track| Id::new(track).ok())
                        .collect();
                    handle
                        .filter(|_| native)
                        .map(|handle| handle.set_solo(solo.clone()))
                }
                Command::Play => {
                    playing = true;
                    handle.map(|handle| handle.play())
                }
                Command::Pause => {
                    playing = false;
                    handle.map(|handle| handle.pause())
                }
                Command::Volume { value } => {
                    volume = value;
                    handle.map(|handle| handle.set_volume(value))
                }
                _ => None,
            };
            if let Some(Err(err)) = result {
                error(&tx, err);
            }
        }
        // What waits in the slot becomes what is shown with the slot held,
        // so `preview_exit` finds a preview in one place or the other.
        let taken = shared.project.lock().ok().and_then(|mut slot| {
            // The proposal a rendered preview shows.
            let (project, file, shown_proposal) = match slot.take()? {
                MediaProject::Native(project) => (project, None, None),
                MediaProject::Rendered(project, file, proposal) => (project, Some(file), proposal),
            };
            let rendered = file.is_some();
            let old = shared
                .shown_preview
                .lock()
                .ok()
                .and_then(|mut shown| std::mem::replace(&mut *shown, file));
            Some((project, rendered, shown_proposal, old))
        });
        if let Some((project, rendered, shown_proposal, old)) = taken {
            native = !rendered;
            // Close the old session before the preview it played goes.
            drop(session.take());
            if let Some(old) = old {
                discard_preview(&shared, old);
            }
            if let Ok(mut frame) = shared.frame.lock() {
                *frame = None;
            }
            emit(&tx, json!({"type":"viewer_reset"}));
            emit(&tx, json!({"type":"caption","text":""}));
            let plan = kerf_playback::preview_plan(&project);
            let message = plan
                .reasons
                .iter()
                .map(|reason| reason.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            let composited = !rendered && plan.composited();
            if !rendered && !plan.native_exact() && !composited {
                session = None;
                playing = false;
                emit(&tx, json!({"type":"playback","playing":false}));
                emit(
                    &tx,
                    json!({"type":"preview","status":if plan.unavailable() { "unavailable" } else { "required" }, "message":message}),
                );
                thread::sleep(Duration::from_millis(4));
                continue;
            }
            // The compositor draws a program cuts cannot show, live; what it
            // or an export does not match yet comes along as a note.
            emit(
                &tx,
                if composited {
                    json!({"type":"preview","status":"live","message":message})
                } else if let Some(proposal) = &shown_proposal {
                    json!({"type":"preview","status":"ready","proposalId":proposal})
                } else {
                    json!({"type":"preview","status":"ready"})
                },
            );
            let size = preview_frame_size(&project);
            // A new worker prevents old-project frames from resurfacing after an edit.
            session = match PlaybackSession::open(
                project,
                PlaybackConfig::default()
                    .with_frame_size(size.0, size.1)
                    .with_composited(composited),
            ) {
                Ok(session) => {
                    let _ = session.handle().set_volume(volume);
                    if native && !solo.is_empty() {
                        let _ = session.handle().set_solo(solo.clone());
                    }
                    let _ = session.handle().seek(position);
                    if playing {
                        let _ = session.handle().play();
                    }
                    Some(session)
                }
                Err(err) => {
                    error(&tx, err);
                    None
                }
            };
        }
        if let Some(session) = &session {
            let handle = session.handle();
            let seek = shared.seek.lock().ok().and_then(|mut slot| slot.take());
            if let Some(seek) = seek {
                // Where the viewer was asked to be, before the session says
                // so itself: an edit that replaces this session first opens
                // the next one here, not where the last frame was.
                position = seek.position;
                // Keep the picture that is already on screen. The UI drops a
                // frame only when a newer one is closer to the pointer, so
                // clearing this slot just freezes the viewer during a scrub.
                let result = if seek.exact {
                    handle.seek(seek.position)
                } else {
                    handle.scrub(seek.position)
                };
                if let Err(err) = result {
                    error(&tx, err);
                }
            }
            for event in session.drain_events() {
                match event {
                    PlaybackEvent::Frame(frame) => {
                        stats.frame(frame.width, frame.height);
                        position = frame.timeline_time;
                        let caption = frame
                            .captions
                            .iter()
                            .map(|cue| cue.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let _ = tx.try_send(json!({"type":"caption","text":caption}));
                        if let Ok(mut slot) = shared.frame.lock() {
                            *slot = Some(frame);
                        }
                    }
                    PlaybackEvent::Clock(clock) => {
                        stats.clock(clock.dropped);
                        position = clock.position;
                        let _ = tx.try_send(json!({
                            "type": "clock",
                            "position": clock.position,
                            "duration": clock.duration,
                            "dropped": clock.dropped,
                        }));
                    }
                    PlaybackEvent::StateChanged(state) => {
                        playing = state == PlaybackState::Playing;
                        emit(&tx, json!({"type":"playback","playing":playing}));
                    }
                    PlaybackEvent::Error(err) => error(&tx, err),
                    PlaybackEvent::Ended => emit(&tx, json!({"type":"playback","playing":false})),
                    PlaybackEvent::Opened { duration, .. } => {
                        position = position.min(duration);
                        emit(
                            &tx,
                            json!({"type":"clock","position":position,"duration":duration,"dropped":0}),
                        );
                    }
                }
            }
        }
        meter.tick(
            Instant::now(),
            playing && session.is_some(),
            || session.as_ref().and_then(PlaybackSession::levels),
            &tx,
        );
        if swept.elapsed() >= Duration::from_millis(250) {
            sweep_previews(&shared);
            swept = Instant::now();
        }
        thread::sleep(if session.is_some() {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(4)
        });
    }
    drop(session);
    discard_previews(&shared);
    // A playback worker that is shutting down may hold its file open a
    // little longer.
    for _ in 0..40 {
        if sweep_previews(&shared) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn new_agent(
    shared: &Arc<Shared>,
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &SyncSender<Value>,
) -> Result<Agent, String> {
    if !settings.cloud && settings.provider == Some(ChatProvider::ClaudeCode) {
        return claude_code::ClaudeCodeSession::new(directory, binary, settings.model.clone())
            .map(Agent::ClaudeCode);
    }
    native_agent(shared, directory, binary, settings, tx)
        .map(|(session, router)| Agent::Native(Box::new(session), router))
}

fn native_agent(
    shared: &Arc<Shared>,
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &SyncSender<Value>,
) -> Result<(AgentSession, KerfMcpRouter), String> {
    // Kerf Cloud speaks the Anthropic Messages protocol with the account token.
    let cloud = if settings.cloud {
        let store = kerf_cloud::Store::default_path()
            .map(|path| cloud_session::load(&path, &secrets::SystemKeychain::new()))
            .unwrap_or_default();
        Some(kerf_cloud::model_credentials(&store).map_err(|error| error.to_string())?)
    } else {
        None
    };
    let provider = if cloud.is_some() {
        ProviderName::Anthropic
    } else if let Some(provider) = settings.provider.and_then(ChatProvider::api) {
        provider
    } else {
        match std::env::var("KERF_PROVIDER").ok().as_deref() {
            Some(name) => {
                ProviderName::parse(name).ok_or("KERF_PROVIDER must be openai or anthropic")?
            }
            None if settings
                .api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty()) =>
            {
                // A key entered in the UI without an explicit provider defaults to
                // OpenAI, matching the existing environment fallback.
                ProviderName::Openai
            }
            None if std::env::var_os("OPENAI_API_KEY").is_some() => ProviderName::Openai,
            _ => ProviderName::Anthropic,
        }
    };
    let key_name = if provider == ProviderName::Openai {
        "OPENAI_API_KEY"
    } else {
        "ANTHROPIC_API_KEY"
    };
    let api_key = cloud
        .as_ref()
        .map(|(_, token)| token.clone())
        .or_else(|| settings.api_key.clone())
        .filter(|key| !key.trim().is_empty())
        .or_else(|| std::env::var(key_name).ok())
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("Add an API key in Settings ▸ Chat or set {key_name}"))?;
    let api_mode = match settings.api_mode {
        Some(mode) => mode,
        None => match std::env::var("OPENAI_API_MODE") {
            Ok(name) => {
                OpenAiApiMode::parse(&name).ok_or("OPENAI_API_MODE must be chat or responses")?
            }
            Err(_) => OpenAiApiMode::Chat,
        },
    };
    let reasoning_effort = std::env::var("OPENAI_REASONING_EFFORT")
        .ok()
        .map(|value| ReasoningEffort::parse(&value).ok_or("Invalid OPENAI_REASONING_EFFORT"))
        .transpose()?;
    // The editor's own lease covers it too, but an agent session holds
    // kerfd for as long as it lives wherever it runs.
    let router = KerfMcpRouter::new(directory, Some(binary))?.held();
    // What it makes on its own, where its person lets it, shows as their own making does.
    router.watch_makings(media::agent_makings(shared.clone(), tx.clone()));
    let mut tools = router.agent_tools();
    inbound::add_agent_tools(&mut tools, directory, settings.mcp_enabled, |line| {
        // Connection errors may contain command arguments or server responses.
        // Never forward raw names, paths, or error details to chat activity.
        emit(
            tx,
            json!({"type":"agent","kind":"status","text":mcp_connection_status(&line)}),
        );
    })
    .map_err(|_| {
        "Could not load MCP configuration. Check ~/.kerf/mcp.json and project .kerf/mcp.json."
            .to_owned()
    })?;
    let next = tx.clone();
    tools.insert(
        "suggest".to_owned(),
        AgentTool::new(
            "suggest",
            SUGGEST_DOC,
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["steps"],
                "properties": {"steps": {
                    "type": "array", "minItems": 1, "maxItems": MAX_NEXT_STEPS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["label", "request"],
                        "properties": {
                            "label": {"type": "string", "maxLength": 40, "description": "The button: a few words, such as \"Stronger glow\"."},
                            "request": {"type": "string", "description": "What the person asks by pressing it, in their words."}
                        }
                    }
                }}
            }),
            move |input| {
                let steps: Vec<Value> = input["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|step| {
                        let label = step["label"].as_str()?.trim();
                        let request = step["request"].as_str()?.trim();
                        (!label.is_empty() && !request.is_empty()).then(|| {
                            json!({"label": label.chars().take(40).collect::<String>(), "request": request})
                        })
                    })
                    .take(MAX_NEXT_STEPS)
                    .collect();
                if steps.is_empty() {
                    return Err("give one to three steps, each with a label and a request".into());
                }
                emit(&next, json!({"type": "agent_next", "steps": steps}));
                Ok(json!({"shown": steps.len()}))
            },
        ),
    );
    let session = AgentSession::new(AgentSessionOptions {
        provider,
        api_mode,
        reasoning_effort,
        api_key: Some(api_key),
        model: settings
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .or_else(|| std::env::var("KERF_MODEL").ok()),
        base_url: cloud
            .map(|(url, _)| url)
            .or_else(|| settings.base_url.clone())
            .filter(|url| !url.trim().is_empty())
            .or_else(|| {
                std::env::var(if provider == ProviderName::Openai {
                    "OPENAI_BASE_URL"
                } else {
                    "ANTHROPIC_BASE_URL"
                })
                .ok()
            }),
        system_prompt: Some(format!(
            "{}\n\n{EDITOR_GUIDE}",
            router.agent_system_prompt()
        )),
        tools,
        context: Some(router.agent_context()),
        ..Default::default()
    });
    Ok((session, router))
}

/// The models event: where the person's models file is, whether it is
/// there, and each model it names as `look` lists them. Nothing is run.
fn models_event() -> Value {
    use kerf_engine::recipe::model::Models;
    let file = Models::default_file();
    models_value(Models::read(&file), &file)
}

fn models_value(
    read: kerf_engine::error::Result<kerf_engine::recipe::model::Models>,
    file: &Path,
) -> Value {
    let mut event = json!({
        "type": "models",
        "file": file.display().to_string(),
        "exists": file.is_file(),
        "models": [],
    });
    match read {
        Ok(models) => event["models"] = json!(kerf_mcp::harness::models_listed(&models)),
        // A file that does not read: said, and what to mend.
        Err(error) => event["error"] = json!(error.to_string()),
    }
    event
}

/// The most next steps the editor shows at once.
const MAX_NEXT_STEPS: usize = 3;

const SUGGEST_DOC: &str = "Offer the person one to three next steps as buttons in the editor. \
Pressing one sends its request to you as if they had typed it.";

/// What the built-in agent does in the editor beyond the shared guide.
const EDITOR_GUIDE: &str = "# In the editor\n\n\
The person works in Kerf's editor and answers with buttons more easily than \
by typing. End each turn by calling `suggest` with one to three next steps \
they are likely to want: a variation on what you just did (another effect or \
transition for that moment, stronger or subtler), the next part of the edit, \
or checking the result. Labels are a few words; requests are what the person \
would say. Do not repeat a step they just took.\n\n\
The timeline shows your proposal as it grows, so they watch you work. Build a \
longer edit in steps: stage the first part with `edit`, check it, then `extend` \
the proposal with the next part, rather than one large script at the end.\n\n\
Asked for a vertical or square cut, set the format first in the same proposal \
(`sequence set s1 size=1080x1920`) and frame footage of another shape to fill it; \
the picture takes the proposal's format as it grows.";

fn default_versions() -> usize {
    3
}

/// The most versions of a request made at once.
const MAX_VERSIONS: usize = 3;
const VERSION_KEYS: [&str; MAX_VERSIONS] = ["A", "B", "C"];
const VERSION_ANGLES: [&str; MAX_VERSIONS] = [
    "Stay closest to what was asked, in its most direct form.",
    "Take a bolder approach: a different opening, structure, or pace.",
    "Take the simplest approach: shorter, with fewer cuts and effects.",
];

/// What one agent is asked to make: its version of the person's request.
fn version_prompt(text: &str, index: usize, count: usize) -> String {
    format!(
        "{text}\n\nMake version {key} of {count}; other agents are making the others at the same \
         time, to compare. {angle} Do not ask questions. Stage it as one proposal with `edit` \
         and do not change any other proposal. End with one line saying what makes this version \
         different.",
        key = VERSION_KEYS[index],
        angle = VERSION_ANGLES[index],
    )
}

/// Where a tool call is working, for the timeline and picture to follow.
fn focus_event(name: &str, targets: &kerf_agent::AgentTargetSummary) -> Option<Value> {
    (!targets.times.is_empty() || !targets.clips.is_empty()).then(|| {
        json!({
            "type": "agent_focus",
            "tool": name,
            "times": targets.times,
            "clips": targets.clips,
        })
    })
}

/// Whether a finished tool call changed what the timeline shows.
fn staged_proposal(summary: &kerf_agent::ToolResultSummary) -> bool {
    summary.proposal.is_some() || summary.status.as_deref() == Some("awaiting_review")
}

/// Carry out a Kerf Cloud command, if any, then report the account:
/// `{type: "cloud", access, account?, message?}`.
fn cloud_status(tx: &SyncSender<Value>, command: Option<Command>) {
    let Some(path) = kerf_cloud::Store::default_path() else {
        emit(tx, json!({"type":"cloud","access":"unavailable"}));
        return;
    };
    // The token in the system's keychain, else in the file (cloud_session.rs).
    let keychain = secrets::SystemKeychain::new();
    let mut message = None;
    let kept = match command {
        Some(Command::CloudSignIn { token }) => {
            cloud_session::keep_token(&path, &keychain, Some(token.trim()))
        }
        Some(Command::CloudSignOut) => cloud_session::keep_token(&path, &keychain, None),
        _ => Ok(()),
    };
    if let Err(error) = kept {
        message = Some(error.to_string());
    }
    let store = cloud_session::load(&path, &keychain);
    let event = match (store.service_url(), store.token()) {
        (None, _) => json!({"type":"cloud","access":"unavailable"}),
        (Some(_), None) => json!({"type":"cloud","access":"signedOut"}),
        (Some(url), Some(token)) => match kerf_cloud::fetch_account(&url, token) {
            Ok(account) => json!({
                "type":"cloud",
                "access": kerf_cloud::Access::of(&account),
                "account": account,
            }),
            Err(kerf_cloud::CloudError::Unauthorized) => {
                let _ = cloud_session::keep_token(&path, &keychain, None);
                message = Some(kerf_cloud::CloudError::Unauthorized.to_string());
                json!({"type":"cloud","access":"signedOut"})
            }
            // Keep the session; the service may be briefly unreachable.
            Err(error) => {
                message = Some(error.to_string());
                json!({"type":"cloud","access":"unreachable"})
            }
        },
    };
    let mut event = event;
    if let Some(message) = message {
        event["message"] = json!(message);
    }
    emit(tx, event);
}

fn mcp_connection_status(line: &str) -> &'static str {
    if line.starts_with("mcp: connected ") {
        "MCP server connected"
    } else if line == inbound::TRACKED_PROJECT_CONFIG {
        "Skipped the project's .kerf/mcp.json: Git tracks it, so it came with the repository \
         rather than from you. Untrack it (git rm --cached .kerf/mcp.json) to use its servers."
    } else {
        "MCP server unavailable; skipped. Check its local command and configuration."
    }
}

/// Make `request.versions` versions of a request, each agent on its own thread.
fn run_versions(
    directory: &std::path::Path,
    binary: &std::path::Path,
    request: &AgentRequest,
    tx: &SyncSender<Value>,
    commands: &SyncSender<Command>,
    shared: &Arc<Shared>,
) {
    let count = request.versions.min(MAX_VERSIONS);
    thread::scope(|scope| {
        for index in 0..count {
            scope.spawn(move || {
                run_version(
                    directory, binary, request, index, count, tx, commands, shared,
                )
            });
        }
    });
}

/// One agent's version: its request and reply go in a thread of their own,
/// and `agent_version` events say how it is going.
#[allow(clippy::too_many_arguments)]
fn run_version(
    directory: &std::path::Path,
    binary: &std::path::Path,
    request: &AgentRequest,
    index: usize,
    count: usize,
    tx: &SyncSender<Value>,
    commands: &SyncSender<Command>,
    shared: &Arc<Shared>,
) {
    let key = VERSION_KEYS[index];
    let title = format!("Version {key}: {}", request.text.trim());
    emit(
        tx,
        json!({"type":"agent_version","key":key,"state":"running","title":title}),
    );
    let finish = |state: &str, proposal: Option<&str>, message: Option<&str>| {
        emit(
            tx,
            json!({
                "type": "agent_version", "key": key, "state": state,
                "proposalId": proposal, "message": message,
            }),
        );
    };
    // Each version is a conversation of its own.
    let store = conversations::Store::of(directory);
    let mut turn = match conversations::Turn::begin(&store, None, &title, None, tx) {
        Ok(turn) => turn,
        Err(err) => {
            finish("failed", None, Some(&err));
            return;
        }
    };
    emit(
        tx,
        json!({"type":"agent_version","key":key,"state":"running","conversation":turn.id()}),
    );
    let mut proposal: Option<String> = None;
    let mut stopped = false;
    let result = (|| {
        let mut session = new_agent(shared, directory, binary, &request.settings, tx)?;
        if let Ok(mut aborts) = shared.version_aborts.lock() {
            aborts.push(session.abort_handle());
        }
        session.run_with_events(
            &version_prompt(&request.text, index, count),
            |event| match event {
                AgentEvent::ToolCallStarted { name, .. } => emit(
                    tx,
                    json!({"type":"agent_version","key":key,"state":"running","status":name}),
                ),
                AgentEvent::ToolCallFinished {
                    name, ok, summary, ..
                } => {
                    turn.step(&format!(
                        "{name} · {}{}",
                        if ok { "done" } else { "failed" },
                        summary
                            .status
                            .as_ref()
                            .map(|status| format!(" · {status}"))
                            .unwrap_or_default(),
                    ));
                    if staged_proposal(&summary) {
                        if let Some(id) = &summary.proposal {
                            proposal = Some(id.clone());
                        }
                        let _ = commands.try_send(Command::Refresh);
                    }
                }
                AgentEvent::Done { text } => turn.done(&text),
                AgentEvent::Aborted => {
                    stopped = true;
                    turn.stopped();
                }
                AgentEvent::TextDelta(_)
                | AgentEvent::TurnStarted { .. }
                | AgentEvent::Context { .. } => {}
            },
        )?;
        Ok::<_, String>(())
    })();
    let outcome = match &result {
        Ok(()) => conversations::Outcome::Done(String::new()),
        Err(err) => conversations::Outcome::Failed(err.clone()),
    };
    if let Err(err) = turn.finish(&store, outcome, proposal.clone(), None, tx) {
        error(tx, format!("Kerf's reply was not kept: {err}"));
    }
    match result {
        Err(err) => finish("failed", proposal.as_deref(), Some(&err)),
        Ok(()) if stopped => finish("stopped", proposal.as_deref(), None),
        Ok(()) => finish("done", proposal.as_deref(), None),
    }
}

fn agent_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<AgentRequest>,
    commands: SyncSender<Command>,
    tx: SyncSender<Value>,
    shared: Arc<Shared>,
) {
    // A session for each conversation, kept beside it as each turn ends, so
    // going back to one goes on with what its agent knew (sessions.rs).
    let mut sessions = sessions::Sessions::of(&directory);
    // A turn whose request could not be kept still has a session of its own.
    let mut loose: Option<(AgentSettings, Agent)> = None;
    let store = conversations::Store::of(&directory);
    while let Ok(request) = rx.recv() {
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        emit(&tx, json!({"type":"agent_busy","busy":true}));
        emit(&tx, json!({"type":"agent_state","state":"running"}));
        if request.versions > 0 {
            run_versions(&directory, &binary, &request, &tx, &commands, &shared);
            if let Ok(mut aborts) = shared.version_aborts.lock() {
                aborts.clear();
            }
            emit(&tx, json!({"type":"agent_state","state":"completed"}));
            shared.agent_busy.store(false, Ordering::SeqCst);
            emit(&tx, json!({"type":"agent_busy","busy":false}));
            let _ = commands.send(Command::Refresh);
            continue;
        }
        // The turn in its conversation: the person's request first, and the
        // open proposals, to find the one this turn stages.
        let mut kept = request
            .turn
            .as_ref()
            .and_then(|(prompt, conversation, at)| {
                let client = RuntimeClient::new(&directory).ok()?;
                let before = client
                    .get_state()
                    .map(|state| conversations::open_proposals(&state))
                    .unwrap_or_default();
                match conversations::Turn::begin(&store, conversation.as_deref(), prompt, *at, &tx)
                {
                    Ok(turn) => Some((client, turn, before)),
                    Err(err) => {
                        error(&tx, format!("The request was not kept: {err}"));
                        None
                    }
                }
            });
        let conversation = kept.as_ref().map(|(_, turn, _)| turn.id().to_owned());
        let earlier = kept.as_ref().and_then(|(_, turn, _)| turn.earlier());
        let mut turn_events = TurnEvents {
            tx: &tx,
            commands: &commands,
            conversation: conversation.clone(),
            turn: kept.as_mut().map(|(_, turn, _)| turn),
            running: std::collections::HashMap::new(),
            heard: false,
        };
        let open = || new_agent(&shared, &directory, &binary, &request.settings, &tx);
        let mut result = (|| {
            let (current, opened) = match &conversation {
                Some(id) => sessions.get(id, &request.settings, open)?,
                None => {
                    let fresh = loose
                        .as_ref()
                        .is_none_or(|(settings, _)| settings != &request.settings);
                    if fresh {
                        loose = Some((request.settings.clone(), open()?));
                    }
                    let (_, agent) = loose.as_mut().ok_or("Agent session unavailable")?;
                    let opened = if fresh {
                        sessions::Opened::New
                    } else {
                        sessions::Opened::Continued
                    };
                    (agent, opened)
                }
            };
            run_turn(
                current,
                opened,
                &request,
                earlier.as_deref(),
                &shared,
                &mut turn_events,
            )
            .map_err(|err| {
                if opened == sessions::Opened::Restored {
                    format!("{RESTORED_FAILED}{err}")
                } else {
                    err
                }
            })
        })();
        // A kept session that no longer goes on (Claude Code forgot it, or its
        // transcript would not run): the conversation starts its agent afresh,
        // hearing what was said, once, if nothing of the turn was heard yet.
        if let (Err(err), Some(id)) = (&result, &conversation) {
            if err.starts_with(RESTORED_FAILED) && !turn_events.heard {
                sessions.forget(id);
                result = sessions
                    .get(id, &request.settings, open)
                    .and_then(|(current, opened)| {
                        run_turn(
                            current,
                            opened,
                            &request,
                            earlier.as_deref(),
                            &shared,
                            &mut turn_events,
                        )
                    });
            }
        }
        let result = result.map_err(|err| {
            err.strip_prefix(RESTORED_FAILED)
                .map(str::to_owned)
                .unwrap_or(err)
        });
        drop(turn_events);
        let outcome = match &result {
            Ok(()) => conversations::Outcome::Done(String::new()),
            Err(err) => conversations::Outcome::Failed(err.clone()),
        };
        if let Err(err) = result {
            emit(
                &tx,
                json!({"type":"agent_state","state":"failed","message":err}),
            );
            // Named as the turn's, so the editor clears it when the next turn starts.
            emit(&tx, json!({"type":"error","command":"agent","message":err}));
        }
        // What the session knows, kept for the next launch.
        let claude_session = conversation.as_deref().and_then(|id| sessions.kept(id));
        if let Some((client, turn, before)) = kept {
            let proposal = client
                .get_state()
                .ok()
                .and_then(|state| conversations::staged_proposal(&before, &state));
            if let Err(err) = turn.finish(&store, outcome, proposal, claude_session, &tx) {
                error(&tx, format!("Kerf's reply was not kept: {err}"));
            }
        }
        if let Ok(mut abort) = shared.abort.lock() {
            *abort = None;
        }
        shared.agent_busy.store(false, Ordering::SeqCst);
        emit(&tx, json!({"type":"agent_busy","busy":false}));
        let _ = commands.send(Command::Refresh);
    }
}

/// How a turn on a session gone on from what was kept fails, so that it is
/// tried once more on a fresh one.
const RESTORED_FAILED: &str = "\u{0}restored: ";

/// What a turn's events go to: the editor, the conversation that keeps the
/// turn, and the tool calls under way, by id.
struct TurnEvents<'a> {
    tx: &'a SyncSender<Value>,
    commands: &'a SyncSender<Command>,
    conversation: Option<String>,
    turn: Option<&'a mut conversations::Turn>,
    running: std::collections::HashMap<String, kerf_agent::AgentTargetSummary>,
    /// Whether anything of the turn has been heard yet.
    heard: bool,
}

impl TurnEvents<'_> {
    fn hear(&mut self, event: AgentEvent, unit: &str) {
        let tx = self.tx;
        if !matches!(event, AgentEvent::Context { .. }) {
            self.heard = true;
        }
        match event {
            // How full Kerf's context is, for the composer's meter.
            AgentEvent::Context { used, limit } => emit(
                tx,
                json!({
                    "type": "agent_context", "used": used, "limit": limit,
                    "unit": unit, "conversation": self.conversation,
                }),
            ),
            AgentEvent::TextDelta(text) => {
                emit(tx, json!({"type":"agent","kind":"delta","text":text}));
            }
            AgentEvent::TurnStarted { .. } => emit(
                tx,
                json!({"type":"agent","kind":"status","text":"Planning next step…"}),
            ),
            // A tool call is one row of the work log: said as it starts, and
            // again in its place once it ends.
            AgentEvent::ToolCallStarted { id, name, targets } => {
                if let Some(focus) = focus_event(&name, &targets) {
                    emit(tx, focus);
                }
                let text = steps::running(&name, &targets);
                self.running.insert(id.clone(), targets);
                emit(
                    tx,
                    json!({"type":"agent","kind":"tool","id":id,"state":"running","text":text}),
                );
            }
            AgentEvent::ToolCallFinished {
                id,
                name,
                ok,
                summary,
            } => {
                if summary.status.as_deref() == Some("awaiting_review") {
                    emit(tx, json!({"type":"agent_state","state":"awaiting_review"}));
                }
                // The proposal shows on the timeline as it grows.
                if staged_proposal(&summary) {
                    let _ = self.commands.try_send(Command::Refresh);
                }
                let targets = self.running.remove(&id).unwrap_or_default();
                let text = steps::finished(&name, &targets, ok, &summary);
                // The cut itself was rendered: what Kerf was asked to make is made.
                let rendered = ok && steps::tool_name(&name) == "render" && summary.deliverable;
                emit(
                    tx,
                    json!({
                        "type": "agent", "kind": "tool", "id": id,
                        "state": if ok { "done" } else { "failed" },
                        "text": text, "rendered": rendered,
                    }),
                );
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.step(&text);
                }
                // A making the person is asked to run: a card with Make.
                if let (Some("asked"), Some(media), Some(ask)) =
                    (summary.status.as_deref(), &summary.media, &summary.ask)
                {
                    let ask = conversations::Ask {
                        media: media.clone(),
                        rolls: summary.rolls.filter(|rolls| *rolls > 1),
                        take: summary.take,
                        text: ask.clone(),
                    };
                    emit(
                        tx,
                        json!({
                            "type": "agent_ask", "media": ask.media, "rolls": ask.rolls,
                            "take": ask.take, "text": ask.text,
                            "conversation": self.conversation,
                        }),
                    );
                    if let Some(turn) = self.turn.as_deref_mut() {
                        turn.ask(ask);
                    }
                }
            }
            AgentEvent::Done { text } => {
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.done(&text);
                }
                emit(tx, json!({"type":"agent","kind":"done","text":text}));
            }
            AgentEvent::Aborted => {
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.stopped();
                }
                emit(tx, json!({"type":"agent_state","state":"aborted"}));
                emit(tx, json!({"type":"agent","kind":"status","text":"Stopped"}));
            }
        }
    }
}

/// One turn on `agent`. A session that has nothing of the conversation hears
/// what was said in it first.
fn run_turn(
    agent: &mut Agent,
    opened: sessions::Opened,
    request: &AgentRequest,
    earlier: Option<&str>,
    shared: &Shared,
    events: &mut TurnEvents<'_>,
) -> Result<(), String> {
    let text = match (opened, earlier) {
        (sessions::Opened::New, Some(earlier)) => format!("{earlier}\nNow: {}", request.text),
        _ => request.text.clone(),
    };
    agent.set_permissions(request.permissions);
    if let Ok(mut abort) = shared.abort.lock() {
        *abort = Some(agent.abort_handle());
    }
    let unit = agent.context_unit();
    agent.run_with_events(&text, |event| events.hear(event, unit))?;
    emit(events.tx, json!({"type":"agent_state","state":"completed"}));
    Ok(())
}

impl Bridge {
    /// Open the project in `project_directory` and start the workers behind it, without
    /// waiting for project I/O: kerfd (`kerfd_binary`, started when none runs for the
    /// project yet) is reached on a worker, which says how that went with `connection` and
    /// `state` events. Never blocks.
    pub fn open(project_directory: &Path, kerfd_binary: &Path) -> Bridge {
        let directory = project_directory.to_path_buf();
        let project_folder = directory.clone();
        let binary = kerfd_binary.to_path_buf();
        // The project's own looks, for its previews; kerfd reports any that do
        // not read.
        let _ = kerf_model::look::install_project_looks(&directory);
        let (commands, rx) = mpsc::sync_channel(32);
        let (media, media_rx) = mpsc::sync_channel(32);
        let (prompts, prompt_rx) = mpsc::sync_channel(1);
        let (collab, collab_rx) = mpsc::sync_channel(16);
        let (emit, events) = mpsc::sync_channel(256);
        let shared = Arc::new(Shared {
            directory: directory.clone(),
            media_status: Mutex::new(media::MediaStatus::default()),
            copies: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            agent_busy: AtomicBool::new(false),
            preview_busy: AtomicBool::new(false),
            abort: Mutex::new(None),
            version_aborts: Mutex::new(Vec::new()),
            frame: Mutex::new(None),
            seek: Mutex::new(None),
            project: Mutex::new(None),
            model: Mutex::new(None),
            previews: AtomicU64::new(0),
            preview_cancel: Mutex::new(None),
            preview_turns: Default::default(),
            stale_previews: Mutex::new(Vec::new()),
            shown_preview: Mutex::new(None),
            check_busy: AtomicBool::new(false),
            check_cancel: Mutex::new(None),
            makings: Mutex::new(std::collections::BTreeMap::new()),
            others_makings: Mutex::new(std::collections::BTreeMap::new()),
            renders: queue::Queue::default(),
            analysis: analysis::Analysis::default(),
            workflows: Default::default(),
        });
        {
            let directory = directory.clone();
            let binary = binary.clone();
            let commands = commands.clone();
            let media = media.clone();
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || {
                runtime_worker(directory, binary, rx, commands, media, tx, shared)
            });
        }
        {
            let directory = directory.clone();
            let binary = binary.clone();
            let runtime = commands.clone();
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || collab::run(directory, binary, collab_rx, runtime, tx, shared));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || media_worker(media_rx, tx, shared));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || queue::worker(shared, tx));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            let commands = commands.clone();
            thread::spawn(move || agent_worker(directory, binary, prompt_rx, commands, tx, shared));
        }
        {
            let tx = emit.clone();
            thread::spawn(move || cloud_status(&tx, None));
        }
        Bridge {
            directory: project_folder,
            events_tx: emit,
            commands,
            media,
            collab,
            prompts,
            events,
            shared,
        }
    }

    /// Queue a command (see the crate docs for every command). `false` means it was refused:
    /// it does not read as a command, cannot be taken as it is, or its bounded queue is full.
    /// Show an error then; an edit is never dropped silently. Never blocks.
    pub fn send(&mut self, command: &Value) -> bool {
        let Ok(command) = Command::deserialize(command) else {
            return false;
        };
        match command {
            Command::Seek { position, scrub } => {
                if !position.is_finite() || position < 0.0 {
                    return false;
                }
                // The worker holds this only to take the seek out, and a seek
                // refused for that moment would be lost.
                if let Ok(mut slot) = self.shared.seek.lock() {
                    *slot = Some(PendingSeek {
                        position,
                        exact: !scrub,
                    });
                    true
                } else {
                    false
                }
            }
            Command::Play | Command::Pause | Command::Volume { .. } | Command::Solo { .. } => {
                self.media.try_send(command).is_ok()
            }
            Command::Abort => {
                if let Ok(aborts) = self.shared.version_aborts.lock() {
                    for abort in aborts.iter() {
                        abort.store(true, Ordering::SeqCst);
                    }
                }
                if let Ok(abort) = self.shared.abort.try_lock() {
                    if let Some(abort) = &*abort {
                        abort.store(true, Ordering::SeqCst);
                    }
                    true
                } else {
                    false
                }
            }
            Command::EffectPreviews(request) => {
                let generation = self.shared.previews.fetch_add(1, Ordering::SeqCst) + 1;
                let shared = self.shared.clone();
                let tx = self.events_tx.clone();
                thread::spawn(move || {
                    if let Err(message) = previews::run(&shared, generation, &request, &tx) {
                        error(&tx, message);
                    }
                });
                true
            }
            Command::Check { sequence } => {
                qc::spawn(self.shared.clone(), self.events_tx.clone(), sequence);
                true
            }
            // Not queued behind the runtime worker: a making is stopped when it is asked.
            Command::MediaMakeCancel { media } => {
                media::cancel_make(&self.shared, &media, &self.events_tx);
                true
            }
            Command::Models => {
                emit(&self.events_tx, models_event());
                true
            }
            Command::Loudness { sequence } => {
                analysis::loudness(self.shared.clone(), self.events_tx.clone(), sequence);
                true
            }
            Command::ScopeSignal { time, sequence } => {
                analysis::scope_signal(self.shared.clone(), self.events_tx.clone(), time, sequence);
                true
            }
            Command::MediaInfo { media } => {
                analysis::media_info(self.shared.clone(), self.events_tx.clone(), media);
                true
            }
            Command::MediaIndex { media } => {
                analysis::media_index(self.shared.clone(), self.events_tx.clone(), media);
                true
            }
            Command::CloudSignIn { .. } | Command::CloudSignOut | Command::CloudRefresh => {
                let tx = self.events_tx.clone();
                thread::spawn(move || cloud_status(&tx, Some(command)));
                true
            }
            // A preview is counted as it is sent, and a stop or an exit acts
            // here rather than behind the runtime worker's queue: it stops the
            // previews sent before it, wherever they have got to.
            Command::Preview { .. } => {
                let commands = &self.commands;
                self.shared
                    .preview_turns
                    .send(|| commands.try_send(command).is_ok())
            }
            Command::PreviewStop => {
                stop_preview(&self.shared, &self.events_tx);
                true
            }
            Command::PreviewExit => {
                exit_preview(&self.shared, &self.events_tx);
                true
            }
            Command::PresetLibrary => {
                let directory = self.shared.directory.clone();
                let tx = self.events_tx.clone();
                thread::spawn(move || emit(&tx, presets::library(&directory)));
                true
            }
            Command::CollabStatus
            | Command::CollabSync { .. }
            | Command::CollabLive { .. }
            | Command::CollabCommit { .. }
            | Command::CollabBranch { .. }
            | Command::CollabPullRequest { .. }
            | Command::CollabPublish { .. }
            | Command::CollabConsolidate => self.collab.try_send(command).is_ok(),
            Command::Agent {
                text,
                focus,
                conversation,
                at,
                permissions,
                settings,
            } => {
                if text.trim().is_empty() || self.shared.agent_busy.swap(true, Ordering::SeqCst) {
                    return false;
                }
                let turn = Some((text.clone(), conversation, at));
                let text = match &focus {
                    Some(focus) => kerf_agent::focused_request(&text, focus),
                    None => text,
                };
                if self
                    .prompts
                    .try_send(AgentRequest {
                        text,
                        settings,
                        versions: 0,
                        turn,
                        permissions: permission_mode(permissions.as_deref()),
                    })
                    .is_err()
                {
                    self.shared.agent_busy.store(false, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            }
            Command::AgentVersions {
                text,
                count,
                settings,
            } => {
                if text.trim().is_empty()
                    || count == 0
                    || self.shared.agent_busy.swap(true, Ordering::SeqCst)
                {
                    return false;
                }
                if self
                    .prompts
                    .try_send(AgentRequest {
                        text,
                        settings,
                        versions: count,
                        turn: None,
                        permissions: None,
                    })
                    .is_err()
                {
                    self.shared.agent_busy.store(false, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            }
            Command::Conversations(request) => {
                conversations::command(
                    &conversations::Store::of(&self.directory),
                    request,
                    &self.events_tx,
                );
                true
            }
            _ => self.commands.try_send(command).is_ok(),
        }
    }

    /// The next event (see the crate docs for every event), or `None` when none is waiting.
    /// Never blocks.
    pub fn poll(&mut self) -> Option<Value> {
        self.events.try_recv().ok()
    }

    /// The newest playback frame not taken yet, without copying its pixels; `None` when no
    /// newer one has come. Only the latest frame is kept: one not taken before the next
    /// arrives is never seen.
    pub fn frame(&mut self) -> Option<Frame> {
        let frame = self
            .shared
            .frame
            .try_lock()
            .ok()
            .and_then(|mut frame| frame.take())?;
        frame.has_valid_pixels().then_some(Frame {
            pixels: frame.pixels,
            width: frame.width,
            height: frame.height,
            time: frame.timeline_time,
        })
    }

    /// The newest signal frame `scope_signal` drew and that was not taken yet, for the
    /// Finish tray's scopes, without copying its pixels; `None` when there is no newer one.
    pub fn signal(&mut self) -> Option<Signal> {
        let frame = self
            .shared
            .analysis
            .signal_frame
            .try_lock()
            .ok()
            .and_then(|mut frame| frame.take())?;
        Some(Signal {
            pixels: frame.pixels,
            width: frame.width,
            height: frame.height,
            time: frame.time,
            curve: frame.curve,
        })
    }

    /// The project folder the bridge was opened on.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Bridge {
    /// Close the bridge and signal the workers to shut down, without blocking the editor.
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        // Nobody would see a preview still rendering, or the one playing. The
        // process may exit before the media worker stops, so the files go now;
        // one a playback worker still holds open is left for the media worker.
        cancel_preview(&self.shared);
        discard_previews(&self.shared);
        qc::cancel(&self.shared);
        // A making nobody would see the end of: its program is asked to end.
        media::cancel_makings(&self.shared);
        self.shared.renders.close(&self.shared.directory);
        self.shared.analysis.close();
        if let Ok(abort) = self.shared.abort.lock() {
            if let Some(abort) = &*abort {
                abort.store(true, Ordering::SeqCst);
            }
        };
        if let Ok(aborts) = self.shared.version_aborts.lock() {
            for abort in aborts.iter() {
                abort.store(true, Ordering::SeqCst);
            }
        };
    }
}

/// A playback frame: RGBA8, `width * height * 4` bytes.
#[derive(Clone)]
pub struct Frame {
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    /// Where it is on the program, in seconds.
    pub time: f64,
}

/// An HDR sequence's frame as its signal, for the Finish tray's scopes: 16-bit
/// little-endian RGBA in BT.2020, `width * height * 8` bytes.
#[derive(Clone)]
pub struct Signal {
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    /// Where it is on its sequence, in seconds.
    pub time: f64,
    /// 1 for PQ, 2 for HLG.
    pub curve: u32,
}

// ---- about ---------------------------------------------------------------------

/// Where Kerf's source and releases are.
pub const REPOSITORY: &str = "https://github.com/imjunseong958-droid/industrial-kerf";

/// `{name, version, license, repository}`: what Help > About Kerf shows.
pub fn about() -> Value {
    json!({
        "name": "Industrial Kerf",
        "version": env!("CARGO_PKG_VERSION"),
        "license": env!("CARGO_PKG_LICENSE"),
        "repository": REPOSITORY,
    })
}

/// Whether a newer Kerf is released on GitHub: `{current, latest?, newer, url?}` or `{error}`
/// (`KERF_UPDATE_CHECK=off` asks nothing). It asks the network and may block for up to
/// 10 s, so call it off the UI thread.
pub fn update_check() -> Value {
    updates::check()
}

// ---- FFmpeg -----------------------------------------------------------------------

/// The FFmpeg Kerf runs, as kerfd finds it (`kerf_engine::ffmpeg::resolve_ffmpeg_bin`): the
/// first that runs of `KERF_FFMPEG`, the one beside the program (where each package puts it,
/// the macOS app's Contents/MacOS among them), Homebrew's ffmpeg-full, then `ffmpeg` on the
/// PATH; `ffmpeg` when none runs, to say so as it fails. Found once, by running each with
/// `-version`, so call it off the UI thread the first time.
pub fn ffmpeg_bin() -> &'static str {
    kerf_engine::ffmpeg::resolve_ffmpeg_bin()
}

// ---- Kerf's tools, served by the editor ------------------------------------------

/// Serve Kerf's tools over MCP on stdin and stdout, as `kerf-mcp` does, when `args` (the
/// program's, without its name) begin with [`SERVE_MCP`]: `kerf-editor --serve-mcp --dir
/// <project>`. Claude Code is given the editor to run this way where no `kerf-mcp` is found.
/// Returns the status to exit with, or `None` when the arguments ask for no such thing and
/// the editor should start.
pub fn serve_mcp(args: &[String]) -> Option<i32> {
    let directory = served_directory(args)?;
    kerf_engine::sidecar::forward_termination_signals();
    let kerfd = std::env::var_os("KERFD_BIN").map(PathBuf::from);
    Some(
        match directory.and_then(|directory| kerf_mcp::serve_stdio(&directory, kerfd.as_deref())) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("kerf-editor {SERVE_MCP}: {error}");
                1
            }
        },
    )
}

/// The project `--serve-mcp --dir <project>` names; `None` when the arguments do not ask the
/// editor to serve.
fn served_directory(arguments: &[String]) -> Option<Result<PathBuf, String>> {
    let (first, rest) = arguments.split_first()?;
    if first != SERVE_MCP {
        return None;
    }
    Some(match rest {
        [option, directory] if option == "--dir" => Ok(directory.into()),
        _ => Err("expected --dir <project-directory>".into()),
    })
}

// ---- crash reports ---------------------------------------------------------------

/// Report every panic in the editor's process to the crash folder, as kerfd and the CLI do,
/// and every crash of its native code too: a bad memory access, or an abort (signals on
/// macOS and Linux; an unhandled-exception filter and `SIGABRT` on Windows). Call once,
/// before any other thread starts.
pub fn crash_install() {
    kerf_runtime::crash::install("editor");
}

/// Report `message` now as what ends the editor, before it aborts; the abort then adds no
/// second report. Nothing is written before [`crash_install`].
pub fn crash_fatal(message: &str) {
    let _ = kerf_runtime::crash::fatal(message);
}

/// The crash folder's reports: `{folder, reports: [{path, component (editor|kerfd|kerf),
/// atMs, message}]}`, newest first; `folder` is null when there is nowhere to keep them.
pub fn crash_reports() -> Value {
    crash_reports_json(kerf_runtime::crash::folder())
}

fn crash_reports_json(folder: Option<PathBuf>) -> Value {
    let reports: Vec<Value> = folder
        .as_deref()
        .map(kerf_runtime::crash::reports)
        .unwrap_or_default()
        .into_iter()
        .map(|report| {
            json!({
                "path": report.path,
                "component": report.component,
                // Milliseconds fit an f64 exactly until the year 287,396.
                "atMs": report.at_ms as f64,
                "message": report.message,
            })
        })
        .collect();
    json!({"folder": folder, "reports": reports})
}

/// The system keychain: `{op: get|set|delete, name, value?}` -> `{ok: true, value?}` (`value`
/// null when `get` finds none) or `{error}` when it cannot be used (`KERF_SECRETS=off` turns
/// it off).
pub fn secret(request: &Value) -> Value {
    secrets::request(request)
}

// ---- project launcher ------------------------------------------------------------

/// What the launcher shows of a project folder, before any bridge exists; synchronous and
/// local: `{path}` -> `{path, exists, project (has project.kerf), name, repository, branch,
/// remote, modified (ISO 8601 of project.kerf), presets (how many)}`.
pub fn project_inspect(request: &Value) -> Value {
    tasks::inspect_project(request)
}

/// A preset file's slots, and how files would fill them: `{path, files?: [path]}` -> `{name,
/// slots: [{id, seconds, video, audio, clips}], texts: {clipId: text}, files: [{path,
/// seconds, video, audio, fits: [slotId], error?}]}` or `{error}`. It probes the files with
/// ffprobe, so it may take a moment.
pub fn preset_inspect(request: &Value) -> Value {
    tasks::inspect_preset(request)
}

// ---- command registry ----------------------------------------------------------

fn command_context(value: &Value) -> kerf_commands::Context {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// The command registry (kerf-commands), with shortcuts in the keymap `keymap` names (such
/// as `premiere`; an unknown one is the default): `{keymap, keymaps, menus, commands}`.
/// Synchronous and pure, as the other registry functions are, so it can be called before a
/// bridge exists.
pub fn commands_catalog(keymap: &str) -> Value {
    kerf_commands::catalog(keymap)
}

/// `{commandId: reason or null}`: why each command is unavailable in the editor's JSON
/// `context`, or null when it is available.
pub fn commands_states(context: &Value) -> Value {
    kerf_commands::states(&command_context(context))
}

/// Resolve `{command, params, context}` to `{request}`, the command for [`Bridge::send`]
/// that carries it out, or `{error}`.
pub fn commands_run(invocation: &Value) -> Value {
    let id = invocation["command"].as_str().unwrap_or_default();
    let context = command_context(&invocation["context"]);
    match kerf_commands::run(id, &context, &invocation["params"]) {
        Ok(request) => json!({"request": request}),
        Err(error) => json!({"error": error}),
    }
}

/// The picture effects and transitions the editor offers, as a JSON array of `{id, kind,
/// category, name, doc, engine, params}`.
pub fn fx_catalog() -> Value {
    kerf_runtime::fx_catalog_json()
}

/// Everything the catalog offers, as a JSON array of `{id, kind, domain, category, name,
/// doc, engine, params}`: picture effects and transitions as [`fx_catalog`] lists them, plus
/// generators (`kind: "generator"`, such as titles and solids) and sound effects (`domain:
/// "audio"`).
pub fn fx_catalog_all() -> Value {
    kerf_runtime::fx_catalog_all_json()
}

/// The properties a clip has by its track's type, which [`fx_catalog_all`] does not list:
/// `{video: [...], audio: [...]}`, as a timeline's `properties` carries them.
pub fn clip_properties() -> Value {
    kerf_runtime::properties_json()
}

#[cfg(test)]
mod tests {
    use super::{
        cancel_preview, discard_preview, discard_previews, media_worker, publish, publish_force,
        queue, rendered_preview, sweep_previews, AgentSettings, Command, MediaProject, PendingSeek,
        Shared,
    };
    use kerf_runtime::{EditOp, PendingProposal, ProposalEntry, ProposalFreshness, StateSnapshot};
    use serde_json::json;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn the_models_file_is_listed_for_settings() {
        use kerf_engine::recipe::model::Models;
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("models.json");
        let missing = super::models_value(Models::read(&file), &file);
        assert_eq!(missing["exists"], false);
        assert_eq!(missing["models"], json!([]));
        std::fs::write(
            &file,
            r#"{"wan": {"run": ["kerf-comfy", "{request}"],
                "takes": {"size": [832, 480], "rate": 16, "seconds": 5, "reads": ["guide", "ref"]}}}"#,
        )
        .unwrap();
        let listed = super::models_value(Models::read(&file), &file);
        assert_eq!(listed["exists"], true);
        assert_eq!(listed["models"][0]["name"], "wan");
        assert_eq!(listed["models"][0]["size"], "832x480");
        assert!(listed["models"][0]["guide"]
            .as_str()
            .unwrap()
            .starts_with("render <sequence> {guide.mp4} size=832x480"));
        std::fs::write(&file, "{ not json").unwrap();
        assert!(super::models_value(Models::read(&file), &file)["error"].is_string());
    }

    #[test]
    fn about_has_the_workspaces_version_and_license() {
        assert_eq!(super::about()["version"], json!(env!("CARGO_PKG_VERSION")));
        assert_eq!(super::about()["license"], json!("MIT"));
    }

    #[test]
    fn crash_reports_reach_the_editor_newest_first() {
        let dir = tempfile::tempdir().expect("a folder");
        for (at, message) in [(5u128, "older"), (9, "newer")] {
            let text = kerf_runtime::crash::report_text("kerfd", at, "main", message, "here", "");
            std::fs::write(dir.path().join(format!("kerfd-{at}-1.txt")), text).expect("written");
        }
        let listed = super::crash_reports_json(Some(dir.path().to_owned()));
        assert_eq!(listed["folder"], json!(dir.path()));
        let messages: Vec<&str> = listed["reports"]
            .as_array()
            .expect("reports")
            .iter()
            .map(|report| report["message"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(messages, ["newer", "older"]);
        assert_eq!(listed["reports"][0]["atMs"], json!(9.0));
        assert_eq!(
            super::crash_reports_json(None),
            json!({"folder": null, "reports": []})
        );
    }

    fn shared() -> Shared {
        crate::tests_support::shared(&std::env::temp_dir())
    }

    const DOCUMENT: &str = "kerf 1\nproject name=\"Bridge\"\n\
        sequence s1 rate=24 size=1920x1080 audio=48000\n";

    fn project(width: u32, height: u32) -> kerf_model::Project {
        kerf_runtime::parse_document(&format!(
            "kerf 1\nproject name=\"Bridge\"\n\
             sequence s1 rate=24 size={width}x{height} audio=48000\n"
        ))
        .unwrap()
    }

    pub(crate) fn snapshot(
        document: &str,
        pending_proposal: Option<PendingProposal>,
    ) -> StateSnapshot {
        StateSnapshot {
            document: document.to_owned(),
            timeline: json!({}),
            revision: 1,
            can_undo: true,
            can_redo: false,
            duration: 0.0,
            pending_proposal,
            source_version: 3,
            projection_status: "current".into(),
            import_warnings: Vec::new(),
            proposals: Vec::new(),
            review_revision: 0,
            revision_time: None,
        }
    }

    #[test]
    fn proposal_only_snapshot_is_published_without_restarting_media() {
        let (tx, rx) = mpsc::sync_channel(4);
        let shared = Arc::new(shared());
        let project = DOCUMENT;
        let mut previous = None;

        publish(snapshot(project, None), &shared, &tx, &mut previous);
        let first = rx.try_recv().expect("initial snapshot should publish");
        assert_eq!(first["type"], "state");
        let media_revision = shared.revision.load(std::sync::atomic::Ordering::SeqCst);
        publish_force(snapshot(project, None), &shared, &tx, &mut previous);
        assert_eq!(
            rx.try_recv().expect("forced refresh should publish")["type"],
            "state"
        );
        assert_eq!(
            shared.revision.load(std::sync::atomic::Ordering::SeqCst),
            media_revision,
            "forced refresh must not restart unchanged media"
        );

        let proposal = PendingProposal::Pending {
            proposal_id: "proposal-1".into(),
            base_revision: 1,
            script: "track add T1 video\n".into(),
            source: String::new(),
            document_diff: "+  track T1 video\n".into(),
            freshness: ProposalFreshness::Current,
        };
        let proposal_for_repeat = proposal.clone();
        publish(
            snapshot(project, Some(proposal)),
            &shared,
            &tx,
            &mut previous,
        );
        let proposal_event = rx.try_recv().expect("proposal snapshot should publish");
        assert_eq!(
            proposal_event["snapshot"]["pendingProposal"]["proposalId"],
            "proposal-1"
        );
        assert_eq!(
            shared.revision.load(std::sync::atomic::Ordering::SeqCst),
            media_revision,
            "proposal-only refresh must keep the media generation stable"
        );
        publish(
            snapshot(DOCUMENT, Some(proposal_for_repeat)),
            &shared,
            &tx,
            &mut previous,
        );
        assert!(
            rx.try_recv().is_err(),
            "unchanged snapshots must be coalesced"
        );
    }

    #[test]
    fn command_requests_are_bridge_commands() {
        let context = json!({
            "connected": true, "revision": 4, "canUndo": true, "playheadFrame": 30, "agentBusy": true,
            "selection": {"clip": "c1", "track": "video", "startFrame": 0, "durationFrames": 48},
        });
        for (command, params) in [
            ("file.import", json!({"path": "/tmp/a.mov"})),
            ("file.export", json!({"quality": "full"})),
            ("file.exportPreset", json!({"path":"/tmp/style.kerfpreset"})),
            (
                "file.applyPreset",
                json!({"path":"/tmp/style.kerfpreset", "bindings":{}, "texts":{}}),
            ),
            ("file.refresh", json!(null)),
            ("edit.undo", json!(null)),
            ("clip.split", json!(null)),
            ("clip.rippleDelete", json!(null)),
            ("sequence.addVideoTrack", json!(null)),
            (
                "clip.trim",
                json!({"clip": "c1", "edge": "in", "frames": 2}),
            ),
            ("proposal.reject", json!({"proposal": "p1"})),
            ("proposal.forward", json!({"proposal": "p1"})),
            ("ai.stop", json!(null)),
        ] {
            let result = super::commands_run(
                &json!({"command": command, "params": params, "context": context}),
            );
            let request = result["request"].clone();
            assert!(
                serde_json::from_value::<Command>(request).is_ok(),
                "{command} resolved to {result}"
            );
        }
        let refused =
            super::commands_run(&json!({"command": "clip.delete", "context": {"connected": true}}));
        assert_eq!(refused["error"], "Select a clip first");
    }

    #[test]
    fn cloud_commands_and_settings_are_read() {
        for command in [
            json!({"type": "cloud_sign_in", "token": "tok"}),
            json!({"type": "cloud_sign_out"}),
            json!({"type": "cloud_refresh"}),
        ] {
            assert!(
                serde_json::from_value::<Command>(command.clone()).is_ok(),
                "{command}"
            );
        }
        let Command::Agent { settings, .. } =
            serde_json::from_value(json!({"type": "agent", "text": "x", "cloud": true})).unwrap()
        else {
            panic!("expected agent command");
        };
        assert!(settings.cloud);
    }

    #[test]
    fn a_turn_names_the_mode_it_runs_in() {
        use super::{permission_mode, Mode};
        let Command::Agent {
            permissions,
            settings,
            ..
        } = serde_json::from_value(
            json!({"type": "agent", "text": "x", "permissions": "accept-edits", "cloud": true}),
        )
        .unwrap()
        else {
            panic!("expected agent command");
        };
        // The mode is the turn's, not the connection's: a change of mode keeps the session.
        assert!(settings.cloud);
        assert_eq!(
            permission_mode(permissions.as_deref()),
            Some(Mode::AcceptEdits)
        );
        assert_eq!(permission_mode(Some("auto")), Some(Mode::Auto));
        // None named leaves the person's own file its say; a wrong name asks.
        assert_eq!(permission_mode(None), None);
        assert_eq!(permission_mode(Some("everything")), Some(Mode::Ask));
    }

    #[test]
    fn versions_are_asked_for_three_by_default_and_differ() {
        use crate::{version_prompt, MAX_VERSIONS};
        let Command::AgentVersions { text, count, .. } =
            serde_json::from_value(json!({"type": "agent_versions", "text": "Open stronger"}))
                .unwrap()
        else {
            panic!("expected versions");
        };
        assert_eq!((text.as_str(), count), ("Open stronger", 3));
        let prompts: Vec<String> = (0..MAX_VERSIONS)
            .map(|index| version_prompt("Open stronger", index, MAX_VERSIONS))
            .collect();
        assert!(prompts[0].starts_with("Open stronger"));
        assert!(prompts[1].contains("version B of 3"));
        assert_ne!(prompts[0], prompts[2]);
    }

    #[test]
    fn a_tool_call_says_where_it_works() {
        use crate::focus_event;
        let targets = kerf_agent::AgentTargetSummary {
            times: vec!["4.5s".into()],
            ..Default::default()
        };
        let focus = focus_event("look", &targets).unwrap();
        assert_eq!(focus["type"], "agent_focus");
        assert_eq!(focus["times"], json!(["4.5s"]));
        assert!(focus_event("look", &Default::default()).is_none());
    }

    #[test]
    fn agent_requests_carry_their_focus() {
        let command: Command = serde_json::from_value(json!({
            "type": "agent", "text": "Tighten this",
            "focus": {"scope": "clip", "clip": "c3", "fromFrame": 24, "toFrame": 72},
        }))
        .expect("focused agent command");
        let Command::Agent { focus, .. } = command else {
            panic!("expected agent command");
        };
        let focus = focus.expect("focus");
        assert!(kerf_agent::focused_request("Tighten this", &focus)
            .starts_with("Focus: clip c3 (24f–72f)."));
    }

    /// `Bridge::send` reads a command from a JSON value where the C ABI read it from text:
    /// the same text reads as the same command, or is refused, either way.
    #[test]
    fn a_command_reads_the_same_from_a_value_as_from_its_text() {
        let commands = [
            r#"{"type":"seek","position":1}"#,
            r#"{"type":"seek","position":0.5,"scrub":true}"#,
            r#"{"type":"volume","value":0.8}"#,
            r#"{"type":"solo","tracks":["A1"]}"#,
            r#"{"type":"edit","revision":4,"ops":[{"op":"move","clip":"c1","frames":12},
                {"op":"set","clip":"c1","props":{"crop_l":0.25,"opacity":null}}]}"#,
            r#"{"type":"agent","text":"Trim it","provider":"claude-code","model":"m",
                "focus":{"scope":"clip","clip":"c1","fromFrame":0,"toFrame":24},
                "conversation":"k1","at":2.5,"permissions":"auto","unknown":1}"#,
            r#"{"type":"agent_versions","text":"Open stronger","cloud":true}"#,
            r#"{"type":"export","quality":"master","codec":"prores_hq","name":"Final"}"#,
            r#"{"type":"media_make","media":"m1","confirmed":true,"rolls":3}"#,
            r#"{"type":"shot_follow","clip":"c1","of":"matte","points":[[0.5,0.5]]}"#,
            r#"{"type":"workflow_edit","media":"m1","line":2,"was":"","node":"3",
                "input":"seed","link":["4",0]}"#,
            r#"{"type":"conversations","action":"rename","id":"k1","title":"Open"}"#,
            r#"{"type":"collab_live","enabled":true,"interval":5}"#,
            r#"{"type":"preview","proposalId":"p1","start":1,"duration":2}"#,
        ];
        let refused = [
            r#"{"type":"no_such_command"}"#,
            r#"{"type":"seek"}"#,
            r#"{"type":"edit","ops":"not a list","revision":0}"#,
            r#"{"type":"volume","value":"loud"}"#,
            r#""refresh""#,
            "[]",
        ];
        for (reads, text) in commands
            .iter()
            .map(|text| (true, text))
            .chain(refused.iter().map(|text| (false, text)))
        {
            let from_text = serde_json::from_str::<Command>(text);
            let value: serde_json::Value = serde_json::from_str(text).unwrap();
            let from_value = <Command as serde::Deserialize>::deserialize(&value);
            assert_eq!(
                from_text
                    .as_ref()
                    .map(|command| format!("{command:?}"))
                    .ok(),
                from_value
                    .as_ref()
                    .map(|command| format!("{command:?}"))
                    .ok(),
                "{text}"
            );
            assert_eq!(from_value.is_ok(), reads, "{text}");
        }
    }

    #[test]
    fn edit_command_carries_typed_operations() {
        let command: Command = serde_json::from_value(json!({
            "type": "edit",
            "revision": 4,
            "ops": [
                {"op": "move", "clip": "c1", "frames": 12},
                {"op": "trim", "clip": "c1", "edge": "out", "frames": -6},
                {"op": "set", "clip": "c1", "props": {"crop_l": 0.25, "opacity": null}}
            ]
        }))
        .expect("edit command should deserialize");
        let Command::Edit { ops, revision } = command else {
            panic!("expected an edit command");
        };
        assert_eq!((ops.len(), revision), (3, 4));
    }

    #[test]
    fn mask_edits_from_the_viewer_are_typed_operations() {
        let command: Command = serde_json::from_value(json!({
            "type": "edit",
            "revision": 2,
            "ops": [
                {"op": "mask-add", "clip": "c1", "name": "k1", "rect": [0.25, 0.25, 0.75, 0.75]},
                {"op": "mask-set", "clip": "c1", "name": "k1",
                 "points": [[0.1, 0.1], [0.9, 0.1], [0.5, 0.9]], "params": {"feather": null}},
                {"op": "mask-remove", "clip": "c1", "name": "k1"}
            ]
        }))
        .expect("mask edits should deserialize");
        let Command::Edit { ops, .. } = command else {
            panic!("expected an edit command");
        };
        let [EditOp::MaskAdd { name, outline, .. }, EditOp::MaskSet {
            outline: set,
            params,
            ..
        }, EditOp::MaskRemove { .. }] = ops.as_slice()
        else {
            panic!("expected mask edits, got {ops:?}");
        };
        assert_eq!(name.as_deref(), Some("k1"));
        assert_eq!(outline.rect, Some([0.25, 0.25, 0.75, 0.75]));
        assert_eq!(set.points.as_ref().map(Vec::len), Some(3));
        assert_eq!(params.get("feather"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn agent_command_accepts_per_request_provider_settings() {
        let command: Command = serde_json::from_value(json!({
            "type": "agent",
            "text": "Trim the opening",
            "provider": "openai",
            "apiMode": "responses",
            "apiKey": "test-key",
            "baseUrl": "http://localhost:9000/v1",
            "model": "local-model",
            "mcpEnabled": true,
            "mcpConfigVersion": 7
        }))
        .expect("agent command should deserialize");
        let Command::Agent { text, settings, .. } = command else {
            panic!("expected agent command");
        };
        assert_eq!(text, "Trim the opening");
        assert_eq!(
            settings,
            AgentSettings {
                provider: Some(crate::ChatProvider::Openai),
                api_mode: Some(kerf_agent::OpenAiApiMode::Responses),
                api_key: Some("test-key".into()),
                base_url: Some("http://localhost:9000/v1".into()),
                model: Some("local-model".into()),
                mcp_enabled: true,
                mcp_config_version: 7,
                cloud: false,
            }
        );
    }

    #[test]
    fn mcp_connection_activity_omits_untrusted_details() {
        assert_eq!(
            super::mcp_connection_status("mcp: connected secret (2 tools)"),
            "MCP server connected"
        );
        let status = super::mcp_connection_status(
            "warning: mcp server 'secret' unavailable (token=secret), skipping",
        );
        assert!(status.contains("unavailable"));
        assert!(!status.contains("secret"));
        let skipped = super::mcp_connection_status(kerf_mcp::inbound::TRACKED_PROJECT_CONFIG);
        assert!(
            skipped.contains(".kerf/mcp.json") && skipped.contains("Git tracks it"),
            "{skipped}"
        );
    }

    #[test]
    fn a_rendered_preview_plays_in_place_of_the_program() {
        let program = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=60f\n\
             \x20     set crop_l=0.25\n",
        )
        .unwrap();
        // The compositor draws a crop live; a rendered preview still plays
        // exactly.
        assert_eq!(
            kerf_playback::preview_plan(&program).mode,
            kerf_core::preview::PreviewMode::Composited
        );
        let rendered = rendered_preview(&program, "/renders/preview.mp4", None).unwrap();
        assert!(kerf_model::validate::errors(&rendered, kerf_model::Catalog::builtin()).is_empty());
        assert_eq!(
            rendered.sequences[0].duration(),
            program.sequences[0].duration()
        );
        let timeline = kerf_playback::PlaybackTimeline::from_project(&rendered).unwrap();
        assert_eq!(
            timeline.video_at(1.0).unwrap().source.path.to_str(),
            Some("/renders/preview.mp4")
        );
        assert_eq!(timeline.audio_at(1.0).len(), 1);
    }

    /// A rendered preview of a one-cut program, playing `file`.
    fn rendered(file: &Path) -> MediaProject {
        let program = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=60f\n",
        )
        .unwrap();
        let project = rendered_preview(&program, file.to_str().unwrap(), None).unwrap();
        MediaProject::Rendered(project, file.to_owned(), None)
    }

    #[test]
    fn an_edit_removes_a_preview_that_never_played() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let file = directory.path().join("render-1.mp4");
        std::fs::write(&file, b"preview").unwrap();
        *shared.project.lock().unwrap() = Some(rendered(&file));

        let (tx, _rx) = mpsc::sync_channel(8);
        publish(snapshot(DOCUMENT, None), &shared, &tx, &mut None);
        assert!(matches!(
            *shared.project.lock().unwrap(),
            Some(MediaProject::Native(_))
        ));
        assert!(!file.exists(), "the superseded preview must be removed");
    }

    #[test]
    fn a_preview_still_open_is_removed_later() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        // A directory stands in for a file something still holds open:
        // removing it as a file fails.
        let held = directory.path().join("render-held.mp4");
        std::fs::create_dir(&held).unwrap();
        discard_preview(&shared, held.clone());
        assert!(!sweep_previews(&shared));
        assert_eq!(shared.stale_previews.lock().unwrap().len(), 1);

        std::fs::remove_dir(&held).unwrap();
        assert!(sweep_previews(&shared));
        assert!(shared.stale_previews.lock().unwrap().is_empty());
    }

    #[test]
    fn closing_removes_the_previews_without_waiting_for_the_media_worker() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        let playing = directory.path().join("render-1.mp4");
        let waiting = directory.path().join("render-2.mp4");
        std::fs::write(&playing, b"preview").unwrap();
        std::fs::write(&waiting, b"preview").unwrap();
        *shared.shown_preview.lock().unwrap() = Some(playing.clone());
        *shared.project.lock().unwrap() = Some(rendered(&waiting));

        discard_previews(&shared);
        assert!(!playing.exists() && !waiting.exists());
        assert!(shared.shown_preview.lock().unwrap().is_none());
        assert!(shared.project.lock().unwrap().is_none());
    }

    #[test]
    fn closing_cancels_only_a_running_preview() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        let marker = kerf_runtime::render_cancel_path(directory.path(), "job-1");
        cancel_preview(&shared);
        assert!(!marker.exists(), "no preview is running");

        *shared.preview_cancel.lock().unwrap() = Some(marker.clone());
        cancel_preview(&shared);
        assert!(marker.exists());
    }

    #[test]
    fn the_media_worker_removes_previews_it_stops_playing() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let first = directory.path().join("render-1.mp4");
        let second = directory.path().join("render-2.mp4");
        std::fs::write(&first, b"preview").unwrap();
        std::fs::write(&second, b"preview").unwrap();
        *shared.project.lock().unwrap() = Some(rendered(&first));

        let (_commands, rx) = mpsc::sync_channel(4);
        let (tx, events) = mpsc::sync_channel(4);
        drop(events);
        let worker = {
            let shared = shared.clone();
            std::thread::spawn(move || media_worker(rx, tx, shared))
        };
        let taken = || shared.project.lock().unwrap().is_none().then_some(());
        assert!(crate::tests_support::wait(Duration::from_secs(5), taken).is_some());
        assert!(first.exists(), "the preview playing stays");

        *shared.project.lock().unwrap() = Some(rendered(&second));
        let removed = || (!first.exists()).then_some(());
        assert!(crate::tests_support::wait(Duration::from_secs(5), removed).is_some());
        assert!(crate::tests_support::wait(Duration::from_secs(5), taken).is_some());
        assert!(second.exists());

        shared.closed.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        assert!(!second.exists(), "closing removes the preview it played");
    }

    /// A program the compositor draws with no file to read: a solid, two
    /// seconds long.
    fn solid(color: &str) -> MediaProject {
        MediaProject::Native(
            kerf_runtime::parse_document(&format!(
                "kerf 1\nproject name=\"Bridge\"\n\
                 sequence s1 rate=24 size=160x90 audio=48000\n\
                 \x20 track V1 video\n\
                 \x20   clip g1 gen=solid@1 at=0f dur=48f\n\
                 \x20     set color={color}\n"
            ))
            .unwrap(),
        )
    }

    #[test]
    fn a_seek_is_kept_when_an_edit_replaces_the_session_it_went_to() {
        use crate::tests_support::wait;
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        *shared.project.lock().unwrap() = Some(solid("#336699"));
        let (_commands, rx) = mpsc::sync_channel(4);
        let (tx, events) = mpsc::sync_channel(4);
        drop(events);
        let worker = {
            let shared = shared.clone();
            std::thread::spawn(move || media_worker(rx, tx, shared))
        };
        // The frame showing, when it is of `time` and as red as `red` says.
        let shown = |time: f64, red: std::ops::Range<u8>| {
            let frame = shared.frame.lock().unwrap();
            frame
                .as_ref()
                .filter(|frame| (frame.timeline_time - time).abs() < 0.02)
                .filter(|frame| red.contains(&frame.pixels[0]))
                .map(|_| ())
        };
        assert!(wait(Duration::from_secs(20), || shown(0.0, 0..100)).is_some());

        // The worker stops here to take the seek, past where it takes a
        // project, so the edit lands after the seek went to the session the
        // edit replaces.
        let mut seek = shared.seek.lock().unwrap();
        *seek = Some(PendingSeek {
            position: 1.0,
            exact: true,
        });
        std::thread::sleep(Duration::from_millis(100));
        *shared.project.lock().unwrap() = Some(solid("#996633"));
        drop(seek);
        assert!(
            wait(Duration::from_secs(20), || shown(1.0, 100..255)).is_some(),
            "the edited program opens where the viewer was asked to be"
        );

        shared.closed.store(true, Ordering::SeqCst);
        worker.join().unwrap();
    }

    #[test]
    fn preview_and_export_commands_carry_their_new_fields() {
        let Command::Preview {
            proposal_id,
            start,
            duration,
        } = serde_json::from_value(json!({
            "type": "preview", "proposalId": "proposal-2", "start": 12.5, "duration": 4.0
        }))
        .unwrap()
        else {
            panic!("expected preview");
        };
        assert_eq!(proposal_id.as_deref(), Some("proposal-2"));
        assert_eq!((start, duration), (Some(12.5), Some(4.0)));
        // The plain preview the command registry sends still reads.
        assert!(matches!(
            serde_json::from_value(json!({"type": "preview"})).unwrap(),
            Command::Preview {
                proposal_id: None,
                start: None,
                duration: None
            }
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "preview_stop"})).unwrap(),
            Command::PreviewStop
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "preview_exit"})).unwrap(),
            Command::PreviewExit
        ));
        let Command::Export(export) = serde_json::from_value(json!({
            "type": "export", "quality": "full", "sequence": "vertical"
        }))
        .unwrap() else {
            panic!("expected export");
        };
        assert_eq!(export.quality, "full");
        assert_eq!(export.sequence.as_deref(), Some("vertical"));
        assert!(matches!(
            serde_json::from_value(json!({"type": "export", "quality": "preview"})).unwrap(),
            Command::Export(queue::ExportRequest { sequence: None, .. })
        ));
    }

    #[test]
    fn render_events_name_the_proposal_they_preview() {
        let request = super::RenderRequest {
            quality: "preview".into(),
            preview: true,
            proposal: Some("proposal-7".into()),
            range: None,
            sequence: None,
        };
        assert_eq!(
            request.event("running"),
            json!({"type": "preview", "status": "running", "proposalId": "proposal-7"})
        );
        let export = super::RenderRequest {
            quality: "full".into(),
            preview: false,
            proposal: None,
            range: None,
            sequence: Some("s2".into()),
        };
        assert_eq!(
            export.event("completed"),
            json!({"type": "export", "status": "completed"})
        );
    }

    #[test]
    fn every_open_proposal_carries_its_timeline() {
        let entry = |id: &str, script: &str, status: &str| ProposalEntry {
            proposal_id: id.into(),
            status: status.into(),
            freshness: ProposalFreshness::Current,
            base_revision: 1,
            applied_revision: None,
            script: script.into(),
            source: String::new(),
            document_diff: String::new(),
            author: kerf_runtime::Author::kerf(),
            version: 1,
            supersedes: None,
            versions: vec![kerf_runtime::ProposalVersion {
                version: 1,
                proposal_id: id.into(),
                base_revision: 1,
                script: script.into(),
                source: String::new(),
                document_diff: String::new(),
            }],
            staged: None,
        };
        let mut revised = entry("p2", "track add T2 video\n", "pending");
        revised.version = 2;
        revised.versions.insert(
            0,
            kerf_runtime::ProposalVersion {
                version: 1,
                proposal_id: "p2-v1".into(),
                base_revision: 1,
                script: "track add T5 video\n".into(),
                source: String::new(),
                document_diff: String::new(),
            },
        );
        revised.versions[1].version = 2;
        let mut state = snapshot(DOCUMENT, None);
        state.proposals = vec![
            revised,
            entry("p1", "track add T1 video\n", "pending"),
            entry("p0", "track add T0 video\n", "accepted"),
        ];
        let value = super::with_proposal_timeline(serde_json::to_value(&state).unwrap(), &state);
        let timelines = value["proposalTimelines"].as_object().unwrap();
        let mut ids: Vec<&str> = timelines.keys().map(String::as_str).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            ["p1", "p2", "p2-v1"],
            "open proposals and their versions only"
        );
        let has_track = |id: &str, track: &str| {
            timelines[id]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["id"] == track)
        };
        assert!(has_track("p2", "T2") && has_track("p2-v1", "T5") && has_track("p1", "T1"));
        assert_eq!(value["proposals"].as_array().unwrap().len(), 3);

        // A snapshot without proposals still has the field.
        let empty = snapshot(DOCUMENT, None);
        let value = super::with_proposal_timeline(serde_json::to_value(&empty).unwrap(), &empty);
        assert_eq!(value["proposals"], json!([]));
        assert!(value.get("reviewRevision").is_none());
        assert_eq!(value["proposalTimelines"], json!({}));
    }

    #[test]
    fn a_range_preview_plays_at_its_place_on_the_program() {
        let program = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=120f\n",
        )
        .unwrap();
        let range = kerf_runtime::RenderRange::new(2.0, 1.5).unwrap();
        let rendered = rendered_preview(&program, "/renders/range.mp4", Some(&range)).unwrap();
        let clip = &rendered.sequences[0].tracks[0].clips[0];
        assert_eq!(clip.start.seconds(), 2.0);
        assert_eq!(clip.duration.seconds(), 1.5);
        let timeline = kerf_playback::PlaybackTimeline::from_project(&rendered).unwrap();
        assert!(timeline.video_at(1.0).is_none());
        assert!(timeline.video_at(2.5).is_some());
    }

    /// The next event, from a thread that may not have run yet.
    fn next(rx: &mpsc::Receiver<serde_json::Value>) -> serde_json::Value {
        rx.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    fn preview_request(proposal: &str) -> super::RenderRequest {
        super::RenderRequest {
            quality: "preview".into(),
            preview: true,
            proposal: Some(proposal.into()),
            range: None,
            sequence: None,
        }
    }

    #[test]
    fn stopping_with_no_preview_running_answers_at_once() {
        let shared = Arc::new(shared());
        let (tx, rx) = mpsc::sync_channel(4);
        super::stop_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));

        // With one running, the render answers once it has stopped.
        let directory = tempfile::tempdir().unwrap();
        let marker = kerf_runtime::render_cancel_path(directory.path(), "job-1");
        *shared.preview_cancel.lock().unwrap() = Some(marker.clone());
        assert!(shared.preview_turns.send(|| true));
        let turn = shared.preview_turns.take();
        super::stop_preview(&shared, &tx);
        crate::tests_support::wait(Duration::from_secs(10), || marker.exists().then_some(()))
            .expect("the render was told to stop");
        assert!(shared.preview_turns.stopped(turn));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        // It is over, stopped, and the next stop has nothing to wait for.
        assert!(shared.preview_turns.over(turn));
        super::stop_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));
    }

    /// The editor sends `preview` and then `preview_stop`; the stop acts
    /// where it is sent and the preview waits for the runtime worker, so the
    /// stop is first. The preview must not start, and answers the stop.
    #[test]
    fn a_stop_that_gets_ahead_of_its_preview_keeps_it_from_starting() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let client = kerf_runtime::RuntimeClient::new(directory.path()).unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        assert!(shared.preview_turns.send(|| true));
        super::stop_preview(&shared, &tx);
        let turn = shared.preview_turns.take();
        super::spawn_render(preview_request("proposal-7"), turn, &client, &tx, &shared);
        assert_eq!(
            next(&rx),
            json!({"type": "preview", "status": "stopped", "proposalId": "proposal-7"})
        );
        assert!(
            !shared.preview_busy.load(Ordering::SeqCst),
            "no render began"
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "one stop, one answer"
        );

        // The same with an exit, which also answers for itself.
        assert!(shared.preview_turns.send(|| true));
        super::exit_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "exited"}));
        let turn = shared.preview_turns.take();
        super::spawn_render(preview_request("proposal-8"), turn, &client, &tx, &shared);
        assert_eq!(
            next(&rx),
            json!({"type": "preview", "status": "stopped", "proposalId": "proposal-8"})
        );
        assert!(
            !shared.preview_busy.load(Ordering::SeqCst),
            "no render began"
        );

        // A preview the runtime worker refuses answers the stop too.
        assert!(shared.preview_turns.send(|| true));
        super::stop_preview(&shared, &tx);
        let turn = shared.preview_turns.take();
        super::preview_unstarted(&shared, &tx, turn, None);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn exiting_a_preview_plays_the_program_again() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        let (tx, rx) = mpsc::sync_channel(4);
        // Nothing to leave: the program keeps playing as it is.
        super::exited_preview(&shared, &tx);
        assert_eq!(
            rx.try_recv().unwrap(),
            json!({"type": "preview", "status": "exited"})
        );
        assert!(shared.project.lock().unwrap().is_none());

        let file = directory.path().join("render-1.mp4");
        std::fs::write(&file, b"preview").unwrap();
        *shared.model.lock().unwrap() = Some(project(1920, 1080));
        *shared.project.lock().unwrap() = Some(rendered(&file));
        let generation = shared.revision.load(Ordering::SeqCst);
        super::exited_preview(&shared, &tx);
        assert!(matches!(
            *shared.project.lock().unwrap(),
            Some(MediaProject::Native(_))
        ));
        assert!(!file.exists(), "the preview never played, so it goes");
        assert!(shared.revision.load(Ordering::SeqCst) > generation);
        assert_eq!(rx.try_recv().unwrap()["status"], "exited");
    }
}
