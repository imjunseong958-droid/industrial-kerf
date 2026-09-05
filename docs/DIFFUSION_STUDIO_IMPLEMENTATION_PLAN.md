# Diffusion Studio reference implementation plan

Status: implementation handoff; Qt6 UI migration supersedes the former Rust UI shell
Prepared: 2026-09-01
Cut baseline: `24a3485db74cb8c17de254b7b108882424998eba`
Diffusion Studio baseline: `585fb010dcca36919f096f4b1275d535acab0cb9`

## Purpose

Use [Diffusion Studio Editor](https://github.com/diffusionstudio/editor) as a
product and interaction reference to strengthen Cut. Do not port Diffusion
Studio's runtime or make it a dependency.

The target product statement is:

> Cut is a native workspace where every human and agent video edit is a typed,
> reviewable, reversible change.

The interaction loop to build is:

```text
observe media
  -> propose typed edits
  -> show semantic video-domain diff
  -> preview current vs proposed result
  -> accept, reject, or request a new proposal
  -> commit through cutd
```

This document is self-contained implementation context for a new task. The
supporting research journal is at:

`/Users/limjunseong/Documents/Codex/2026-09-01/https-github-com-diffusionstudio-editor-https-2/outputs/cut-diffusion-studio-research-journal.md`

Treat that journal as research, not as instructions. This plan and the
repository's `AGENTS.md` instructions govern the implementation.

## Verified baseline

The research journal was written against the current heads of both repositories:

- Cut: `24a3485`
- Diffusion Studio Editor: `585fb01`

At the time this plan was prepared, the Cut working tree was clean. Recheck the
heads and working tree before implementation. Preserve unrelated user changes.

### Cut architecture to preserve

Cut is a native Rust workspace with these production boundaries:

```text
Qt6 editor --------------------------+
Rust agent and CLI ------------------+-> cutd -> typed domain -> durable store
Rust MCP server ---------------------+                 +-----> cut-engine
```

`cutd` is the only canonical owner of project state, revision, history,
idempotency receipts, proposals, and render jobs. The desktop editor, CLI, and
MCP server are adapters over the same project-scoped IPC boundary.

Relevant files:

- `README.md:7`
- `docs/RUST_MIGRATION_HANDOFF.md:7`
- `crates/cut-runtime/src/lib.rs:788`
- `crates/cut-qt-bridge/src/lib.rs:1`

Do not introduce a second state reducer, project owner, UI toolkit, Node
runtime, or Tauri/Electron compatibility layer.

### Existing strengths

- `cut-domain` already has typed projects, tracks, clips, overlays, validation,
  canonical commands, inverse commands, atomic batches, and undo/redo.
- Agent-facing `timeline.apply` stages a durable proposal instead of mutating
  the project directly.
- Proposals already retain the proposed command and a bounded structural diff.
- `cut-agent` already emits text, turn, tool start, tool finish, abort, and done
  events through `run_with_events`.
- `cut-engine` already provides media info, silence detection, scene detection,
  frame extraction, offline rendering, and optional vision.
- `cut-playback` already supports worker-backed native playback, seeking, audio,
  captions, and bounded frame delivery.

Relevant files:

- `crates/cut-domain/src/lib.rs:485`
- `crates/cut-domain/src/lib.rs:605`
- `crates/cut-mcp/src/lib.rs:761`
- `crates/cut-runtime/src/lib.rs:355`
- `crates/cut-agent/src/lib.rs:1206`
- `crates/cut-engine/src/probe.rs:187`

### Current product gaps

- The timeline shows track names and clip counts, not a time axis or clip
  blocks. It has no zoom, waveform, thumbnails, drag, trim, split, or snapping.
- Selection is track-centric.
- The inspector edits only the first clip's volume on the selected track.
- The media bin is a read-only path list.
- The proposal UI reduces a real structural diff to a change count.
- The desktop agent calls `run`, so streaming and tool events never reach the UI.
- Live playback supports a smaller feature set than offline export. Advanced
  transform, effects, keyframes, transitions, and layered composition can render
  differently in the viewer and export.
- Render cancellation updates job state but does not reliably stop the running
  FFmpeg child.

Relevant files:

- `apps/cut-qt/main.cpp:98`
- `crates/cut-qt-bridge/src/lib.rs:1`
- `crates/cut-playback/src/timeline.rs:214`
- `crates/cut-playback/src/decoder.rs:140`
- `crates/cut-core/src/compile.rs:1836`
- `crates/cut-mcp/src/lib.rs:652`

## What to borrow from Diffusion Studio

Borrow behavior and product contracts, then implement them natively in Cut:

1. Agent media inspection that rises from cheap observation to expensive
   reasoning: probe, waveform/filmstrip, transcript/listen, timestamp grabs,
   structural check, and rendered capture.
2. First-class editable results. Agent output must remain clips, tracks,
   overlays, captions, effects, and commands a human can continue editing.
3. A custom immediate-mode timeline with separate passes for ruler, clips,
   playhead, waveform, thumbnails, selection, and proposal overlays.
4. Stable node identity and semantic edits rather than UI-specific mutations.
5. Separation of authored project state from computed presentation/render state.
6. A long-term goal that live preview and offline export consume the same
   normalized evaluation semantics.
7. Project-local, version-matched agent documentation.

Useful upstream references:

- Repository overview and CLI:
  <https://github.com/diffusionstudio/editor/blob/585fb010dcca36919f096f4b1275d535acab0cb9/README.md>
- Timeline surface:
  <https://github.com/diffusionstudio/editor/blob/585fb010dcca36919f096f4b1275d535acab0cb9/apps/web/src/engine/timeline/surface.ts>
- Timeline drag and trim behavior:
  <https://github.com/diffusionstudio/editor/blob/585fb010dcca36919f096f4b1275d535acab0cb9/apps/web/src/engine/timeline/drag.ts>
- Snapping behavior:
  <https://github.com/diffusionstudio/editor/blob/585fb010dcca36919f096f4b1275d535acab0cb9/apps/web/src/engine/timeline/snapping.ts>
- Authored vs computed world state:
  <https://github.com/diffusionstudio/editor/blob/585fb010dcca36919f096f4b1275d535acab0cb9/packages/runtime/src/world/store.ts>
- Source write-back complexity, for understanding only:
  <https://github.com/diffusionstudio/editor/blob/585fb010dcca36919f096f4b1275d535acab0cb9/apps/desktop/src/edit.ts>

## Explicit non-goals

Do not:

- make TSX or any code file Cut's canonical project document;
- port JSX write-back;
- embed Electron, Solid, Vite, Koota, or a browser runtime;
- execute arbitrary npm project code;
- copy Diffusion Studio source files, UI styling, logo, or brand assets;
- create a generic plugin ABI before concrete use cases require it;
- implement partial proposal acceptance in the first version;
- put ripple edit, multi-select drag, keyframe editing, scene boards, and recipes
  into the first timeline milestone;
- expose an editor control for a feature that cannot be previewed accurately.

Diffusion Studio's code is MPL-2.0 and its desktop brand assets are excluded
from that license. Reimplement behavior in new Rust files. If any upstream code
is ever copied, stop and perform a license review first.

License:
<https://github.com/diffusionstudio/editor/blob/585fb010dcca36919f096f4b1275d535acab0cb9/LICENSE>

## Cross-cutting decisions

### Canonical state

Every accepted edit must reach `cutd` as one typed command or one atomic batch.
The UI may own transient selection, drag previews, viewport state, and bounded
agent run events. It must not own a competing project snapshot or reducer.

### Time representation

Do not migrate the v2 project schema from `f64` seconds in this work. Add a
small `FrameGrid` or equivalent helper that deterministically converts between
project seconds and project frame indices. Every editor-generated placement,
trim, split, seek snap, and proposal overlay must quantize through this helper
before a command is sent.

Only consider a frame-native project schema in a separately designed v3
migration if focused drift and interchange tests prove it necessary.

### Proposal review

Keep the current single-pending-proposal rule. A request for changes should:

1. reject the current proposal through the revisioned, idempotent boundary;
2. start a new agent run containing the user's feedback and a compact summary
   of the rejected proposal.

Do not partially apply a batch. Atomic review is easier to reason about and
matches the current durable receipt model.

### Derived artifacts

Thumbnails, waveforms, filmstrips, transcripts, and captures are regenerable
cache artifacts, not canonical project data. Store them under a content-addressed
path below `.cut/cache/derived/`. Key them from source identity and normalized
options. Return paths and compact metadata by default. Include base64 only when
a multimodal tool result explicitly requests it.

### UI performance

Use visible-time and visible-row culling. Do not repaint an idle timeline
continuously. Do not add pulse, shimmer, blur, or spinner animations. Establish
a benchmark fixture with at least 1,000 clips and measure paint and hit-testing
before setting a hard release budget.

## Implementation sequence

Each phase should be a reviewable PR. Do not combine all phases into one diff.

### Phase 0: architecture record, capability matrix, and static mocks

Goal: lock product and rendering contracts before changing the UI.

Work:

1. Add a short architecture decision record that incorporates the invariants in
   this plan.
2. Create a machine-readable or typed live/offline capability matrix covering:
   placement, in/out, speed, volume, captions, crop, transform, opacity, layered
   video, audio mixing, effects, keyframes, transitions, overlays, and titles.
3. Add focused tests proving the matrix matches current playback/compiler
   behavior.
4. Build three distinct static HTML mocks:
   - review-first;
   - timeline-first;
   - split before/after preview.

Stop after publishing the mocks and wait for Lim to choose one. Do not edit the
real components before that choice.

Likely code locations:

- `docs/architecture/`
- `crates/cut-core/src/`
- `crates/cut-playback/src/`
- a non-production mock directory agreed with Lim

Acceptance:

- unsupported live features are discoverable without attempting playback;
- the matrix is typed, not a string map scattered through the UI;
- the three mocks are visually distinct and obey the project design constraints;
- no production UI changes land in this phase.

### Phase 1: semantic proposal impacts

Goal: convert proposal review from JSON leaf paths into video-domain meaning.

Add a typed `ProposalImpact` model. The exact names may follow existing crate
style, but it must be able to represent at least:

- track added/removed/reordered;
- clip added/removed;
- clip moved between tracks or times;
- clip trimmed;
- clip property changed;
- overlay added/removed/changed;
- project setting changed;
- an explicit fallback for a bounded unclassified change.

Derive impacts inside the canonical runtime while staging a proposal. The
runtime already has the typed command, before project, and candidate project at
that point. Do not make the desktop infer semantics from JSON paths.

Retain the existing bounded raw `ProposalChange` list for diagnostics and wire
compatibility. Extend, do not silently replace, the proposal checkpoint format.
Legacy checkpoints without semantic impacts must still load; compute impacts
when possible or expose a typed unavailable state.

Add an explicit stale state when the proposal base revision is no longer the
current project revision. Never silently rebase a proposal.

Likely files:

- `crates/cut-domain/src/lib.rs`
- `crates/cut-runtime/src/lib.rs`
- `fixtures/domain/parity.json`
- `fixtures/protocol/conformance.json`
- runtime unit and restart tests

Acceptance:

- move, trim, add, remove, and property changes produce stable typed impacts;
- restart preserves the pending proposal and impacts;
- old checkpoint fixtures remain readable;
- diff truncation remains bounded;
- accept/reject replay and revision-conflict tests remain green.

### Phase 2: desktop agent event transport

Goal: turn the agent rail into a live work log without changing project
ownership.

Work:

1. Replace the desktop-only `AgentFinished(Result<String, String>)` event with a
   typed event capable of carrying bounded `AgentEvent` updates and terminal
   failure.
2. Call `AgentSession::run_with_events` in the native worker.
3. Add an agent run view model with an enum state such as idle, running,
   awaiting review, completed, failed, and aborted.
4. Show text streaming, tool start/finish, relevant source/clip/track targets,
   artifact paths, and compact errors.
5. Associate the completed run with the proposal it staged.

Do not retain raw frame base64, unbounded tool JSON, provider payloads, or
credentials in UI state or logs. Add a compact typed tool result summary rather
than passing the current arbitrary `detail: String` through unchanged.

Likely files:

- `crates/cut-agent/src/lib.rs`
- `crates/cut-qt-bridge/src/lib.rs`
- `apps/cut-qt/main.cpp`

Acceptance:

- events appear while the agent is running;
- the UI stays responsive and can repaint on event arrival;
- abort and provider/tool failures are distinct states;
- event history is bounded;
- the final durable proposal is still read from `cutd`, not from agent UI state.

### Phase 3: read-only custom timeline and proposal ghosts

Goal: provide a real time-based surface before enabling direct manipulation.

Replace the summary timeline projection with typed presentation structures for:

- track rows and track state;
- clip identity and timeline/source ranges;
- source label and media type;
- playhead and selection;
- timeline viewport, zoom, and scroll;
- thumbnail and waveform cache keys;
- semantic proposal impacts.

Use Qt's `QPainter` with separate logical passes for background/grid, ruler,
track rows, clip bodies, media decoration, selection, proposal overlay, and
playhead. Cull clips outside the visible time and row range.

Proposal visualization:

- added clip: translucent ghost at the proposed range;
- removed clip: removal tint/strike on the existing clip;
- moved clip: outline at the original location and ghost at the destination;
- trimmed clip: compare original and proposed edges;
- changed property: badge or inspector diff, not a misleading geometry change.

Add AccessKit labels for tracks, clips, time ranges, proposal status, and the
playhead. Keyboard selection and focus must remain usable.

Likely files:

- `apps/cut-qt/main.cpp`
- new small timeline geometry/render modules only if the existing file becomes
  unwieldy;
- `apps/cut-qt/main.cpp`
- `crates/cut-qt-bridge/src/lib.rs`
- the centralized Qt stylesheet and timeline palette in `apps/cut-qt/main.cpp`

Acceptance:

- ruler, clips, selection, playhead, zoom, and horizontal scroll work;
- proposal impacts appear at the correct track and frame;
- a 1,000-clip fixture uses visible-range culling;
- the idle app does not repaint continuously;
- no direct edit command is emitted in this phase.

### Phase 4: basic NLE editing commands and interactions

Goal: expose only the editing operations Cut can preview and export reliably.

Add explicit semantic commands where the existing low-level `UpdateClip` and
`Batch` would obscure intent. Prefer a minimal set:

- place a clip at a timeline position and optionally another track;
- trim a clip at a typed leading/trailing edge;
- split a clip at a timeline frame.

Keep `UpdateClip` for wire compatibility. The command parser may accept legacy
shapes, but the reducer and new UI should use the semantic variants.

Interaction order:

1. clip selection;
2. playhead seek;
3. single-clip move;
4. leading/trailing trim;
5. split at playhead;
6. snapping to project start, playhead, and non-active clip edges;
7. keyboard nudge on the project frame grid.

During pointer movement, the UI owns only a transient drag preview. On release,
send one command or one atomic batch with the displayed base revision. A
revision conflict cancels the local preview and refreshes from `cutd` with an
explicit user-visible message.

Do not add ripple, overwrite sequence settlement, or multi-selection in this
phase.

Likely files:

- `crates/cut-domain/src/lib.rs`
- `crates/cut-qt-bridge/src/lib.rs`
- `apps/cut-qt/main.cpp`
- domain golden fixture and app/runtime tests

Acceptance:

- every completed gesture is exactly one undo step;
- trim cannot exceed source bounds or produce a nonpositive duration;
- split creates stable, explicit IDs and preserves source mapping;
- all editor-generated time values are frame-grid quantized;
- two-client conflicts never lose or silently merge an edit;
- undo/redo, restart, Korean input, keyboard focus, and AccessKit gates remain
  green.

### Phase 5: local media observation and derivative cache

Goal: give both the built-in agent and MCP clients cheap, reusable media senses.

Extend the existing `media.probe` action model rather than multiplying MCP tools
without need. Preserve existing `info`, `silence`, `scenes`, and `frames`
behavior and add typed action schemas for:

- exact timestamp frame grabs;
- filmstrip/contact sheet;
- waveform envelope and optional image.

Add separate project-level read-only capabilities for:

- `timeline.check`;
- `timeline.capture`.

Initial `timeline.check` findings:

- missing/offline source;
- zero or negative range;
- source-range overflow;
- black canvas gaps in video coverage;
- disabled, opacity-zero, or out-of-range never-visible clips;
- compiler-rejected feature;
- known live/offline capability mismatch.

Return typed findings containing code, severity, entity path/ID, optional time
range, message, and optional safe suggested action. Do not auto-fix.

Use existing FFmpeg/ffprobe sidecars. Do not add a media decoding crate merely
for waveform or contact sheet generation.

Likely files:

- `crates/cut-engine/src/probe.rs`
- one small derivative cache module in `cut-engine` if needed;
- `crates/cut-core/src/` for structural checks;
- `crates/cut-mcp/src/lib.rs`
- `crates/cut-cli/src/main.rs`
- MCP/protocol fixtures and focused engine tests

Acceptance:

- repeated requests with the same source and normalized options hit cache;
- source replacement invalidates derivatives;
- paths and metadata are returned without base64 by default;
- check findings are deterministic and stable enough for agents;
- capture uses the same offline compiler semantics as export.

### Phase 6: transcription and selected-range listening

Goal: add expensive media reasoning only after local observation is stable.

Recommended initial transcription backend: an optional local `whisper.cpp`
sidecar that emits word-level timestamps. If the binary is unavailable, expose
a clear capability-unavailable response. Do not silently upload media.

Before implementation, write and approve a short backend decision covering:

- local vs provider processing;
- supported formats and range limits;
- word timestamp contract;
- cancellation and progress;
- cache identity;
- privacy and log redaction;
- packaging cost on each target platform.

`listen` should operate on an explicit bounded time range and consume compact
derived observations where possible. Provider upload must be explicit in config
and UI.

Acceptance:

- transcription output uses a typed word/cue schema;
- the source fingerprint and backend/model version are in the cache key;
- cancellation stops the sidecar;
- provider mode never activates implicitly;
- no audio bytes or transcript content enter canonical project state unless the
  user explicitly creates captions from them.

### Phase 7: rendered-preview fallback and render cancellation

Goal: prevent the native viewer from lying about an edit.

Short-term behavior:

1. Analyze the current project or selected range with the capability matrix.
2. If native playback is exact, use native playback.
3. If native playback is incomplete, show the unsupported features and offer or
   automatically use a bounded draft render for that range.
4. Reuse the capture path for proposal before/after contact sheets.

Add per-job cancellation rather than the current process-wide interrupt flag.
The executor must retain a job-specific cancellation token and kill the exact
FFmpeg child. The durable runtime remains the owner of job state; execution
workers report progress and terminal state through the existing runtime
boundary.

Longer term, introduce a normalized evaluation plan in `cut-core` that both
`cut-playback` and `cut-engine` consume. Do not introduce an ECS. The purpose is
shared timing, layer, transition, effect, and visibility semantics, not a new
state owner.

Likely files:

- `crates/cut-core/src/`
- `crates/cut-playback/src/timeline.rs`
- `crates/cut-playback/src/decoder.rs`
- `crates/cut-playback/src/audio.rs`
- `crates/cut-engine/src/render.rs`
- `crates/cut-runtime/src/lib.rs`
- `crates/cut-mcp/src/lib.rs`
- playback release gates and engine compile tests

Acceptance:

- the UI never presents an incomplete native preview as exact;
- supported placement, trim, speed, volume, and caption cases match export;
- unsupported cases use an explicit rendered fallback;
- cancelling a render terminates the correct child and survives retries;
- restart reconciliation and durable job receipts remain correct.

### Phase 8: inspector, media bin, and product polish

Goal: expose the domain features that now have reliable preview behavior.

Add controls in this order:

1. selected clip timing and source range;
2. speed and volume;
3. crop;
4. position, scale, rotation, and opacity;
5. supported color effect parameters;
6. supported keyframe list and editing;
7. transitions with exact supported types.

The capability matrix determines which controls are enabled. Do not use a
generic JSON editor for arbitrary effect or overlay fields.

Upgrade the media bin with import, thumbnail, duration, dimensions, audio state,
and waveform availability. Add command palette and SVG icons only as required by
the approved design. Keep the layout flat, dark, information-dense, and free of
decorative card/pill chrome.

Acceptance:

- each inspector edit is a typed, undoable command;
- disabled capabilities explain why they are unavailable;
- media import still commits atomically through `cutd`;
- preview/export parity tests cover every newly exposed property.

### Phase 9: generated agent docs and outline projection

Goal: make the current Cut contract discoverable without creating a second
source of truth.

Generate machine-owned files under:

```text
.cut/docs/generated/protocol.md
.cut/docs/generated/tools.json
.cut/docs/generated/skills.md
.cut/docs/generated/project-summary.md
```

Reserve `.cut/docs/conventions.md` for user-owned editorial guidance. Never
overwrite an existing root `AGENTS.md`. Offer an explicit opt-in helper if a
pointer from `AGENTS.md` is useful.

Add a read-only Outline projection for scene/sequence, track, clip, caption,
effect, and marker hierarchy. If editing is later enabled, support only an
explicit subset and compile it to typed command batches. The outline document
must never become canonical state.

Acceptance:

- generated/user-owned boundaries are unmistakable;
- docs include protocol/app version and regeneration metadata;
- regeneration is idempotent;
- outline identity maps exactly to domain IDs;
- no JSON or text document can bypass `cutd` mutations.

## Deferred experiments

Do not start these until the proposal review, real timeline, observation tools,
and preview contract are proven in production use:

- scene board for multiple sequences/aspect ratios/languages;
- reusable scene recipes;
- exposed recipe parameters;
- generated asset declarations;
- multi-select and sequence overwrite editing;
- ripple edit;
- third-party plugin ABI.

When recipes are eventually explored, begin with three concrete repeated
compositions, such as a lower third, podcast layout, and product CTA. Do not
design a general system from hypothetical requirements.

## Suggested PR order

1. Architecture record and live/offline capability matrix.
2. Semantic proposal impacts and checkpoint compatibility.
3. Desktop agent event transport and bounded run log.
4. Three static UI mocks; stop for Lim's selection.
5. Theme foundation and read-only custom timeline.
6. Proposal review surface and timeline ghosts.
7. Place, trim, split, and snapping commands/interactions.
8. Derivative cache, exact grabs, filmstrip, and waveform.
9. Timeline check and capture.
10. Backend decision, transcription, and selected-range listening.
11. Rendered-preview fallback and real per-job cancellation.
12. Advanced inspector and media bin.
13. Generated docs and read-only outline.

If a PR reveals a live/offline mismatch in a property about to be exposed, stop
that UI work and complete the capability or fallback path first.

## Verification strategy

Follow the repository's compiler-first workflow. After each edit, pay for the
smallest command that proves the next fact.

Typical sequence:

1. `cargo check -p <changed-crate>`
2. one focused unit or integration test for changed behavior
3. check reverse-dependent crates only for a public API/protocol change
4. run the relevant clippy command if CI covers the changed crate
5. run wider workspace tests only at a cross-crate milestone boundary

Expected focused gates by area:

- domain commands and semantic impacts: `cargo test -p cut-domain`
- durable proposal/job behavior: `cargo test -p cut-runtime`
- MCP schemas and routing: `cargo test -p cut-mcp`
- media probe/cache/render: `cargo test -p cut-engine`
- Qt bridge and command surface: `cargo check -p cut-qt-bridge -p cut-cli`
- live parity and real media: `cargo test -p cut-playback --all-targets -- --nocapture`
- binary surface and restart: `python3 scripts/test-cut-qt-bridge.py`

Do not run `cargo clean`. Do not use a workspace-wide check to hunt a local
compiler error. `cargo fmt --all --check` is not currently a clean repository
gate because of pre-existing `cut-engine` formatting; do not rewrite unrelated
engine files to hide that distinction.

### Required test cases

Domain and runtime:

- place, trim, split inverse commands;
- frame-grid quantization at non-integer frame rates;
- proposal semantic impacts for add/remove/move/trim/property change;
- legacy proposal checkpoint load;
- stale proposal and two-client conflict;
- accept/reject idempotent retry after restart.

Timeline UI:

- time-to-pixel and pixel-to-time round trip;
- hit testing at clip boundaries;
- visible-range culling;
- ghost geometry for move/trim/add/remove;
- drag cancellation on revision conflict;
- keyboard focus and AccessKit labels.

Observation:

- exact frame extraction order;
- cache hit and source invalidation;
- waveform bucket determinism;
- black gap and never-visible findings;
- compiler capability mismatch finding;
- bounded result sizes.

Preview/render:

- native/export parity for supported edits;
- fallback selection for unsupported edits;
- before/after proposal capture;
- per-job cancellation kills only the target process;
- restart reconciles abandoned work without corrupting project state.

## Completion criteria

The Diffusion Studio reference work is complete when:

- an agent can inspect footage with bounded, cached observation tools;
- its tool activity is visible while it works;
- its proposal is shown as typed video-domain changes rather than a count;
- proposed move, trim, add, and removal changes appear as timeline ghosts;
- a user can place, trim, split, snap, undo, and redo through `cutd`;
- the viewer identifies or avoids every known live/export mismatch;
- unsupported projects use a rendered preview fallback;
- project-local generated docs describe the installed protocol and tools;
- Cut still has one canonical state owner, one native UI toolkit, and no Node or
  Electron production surface.

The first implementation target is Phase 1 plus Phase 2: semantic proposal
impacts and desktop agent event transport. They unlock the review surface while
touching the smallest amount of rendering and interaction code.
