# ADR 0006: Exact-time kernel with a text-canonical project

Status: accepted (phases 1 and 2 implemented)
Date: 2026-10-04

## Context

Cut grew its agent harness from coding-agent patterns: tool discovery
(`tools.search` → `tools.describe` → `tools.call`), paginated project queries,
result handles, and procedural recipes in the system prompt. Video editing
inverts the coding problem. The timeline is small and easy to change; the hard
parts are understanding the footage and judging the result.

The domain underneath was also loose:

- Time was `f64` seconds (`FrameGrid { fps: f64 }`), which cannot represent
  23.976, drop-frame timecode, or sample-accurate audio exactly.
- Four places interpreted the project schema separately: `cut-domain` (with an
  alias and normalization layer), `cut-core` compile (re-parsing raw JSON),
  `cut-playback`, and the Qt app.
- Features entered as untyped JSON (`UpdateClip { patch }`, generic effect
  params, `extensions` on every type).
- There was no NLE edit vocabulary: no insert, overwrite, ripple, roll, slip,
  slide, or extract.

A foundation for film-scale work needs exact time, one model, real editing
semantics, and a representation people and agents can both read.

## Decision

- **Exact time.** `cut-model` measures all time in flicks (1/705,600,000 s),
  which divides every common film, video, NTSC, and audio rate. Rates are
  reduced rationals that must be flick-exact. Timecode supports drop-frame.
  Speed is a rational. Clip, source, and keyframe times are offsets;
  timecode origins are applied only when presenting text.
- **One model.** `cut-model` owns the typed project: media, sequences (nesting
  allowed), video, audio, and subtitle tracks, clips with media, sequence,
  generator, or caption content, transitions at cuts, markers, and link
  groups. `validate` checks every invariant.
- **Features are data.** A `Catalog` defines every intrinsic clip property,
  effect, transition, and generator with typed, ranged, optionally animatable
  parameters. Validation, the language, the agent reference, and later the
  inspector and render lowering read the same definitions.
- **NLE operations.** `apply(project, catalog, ops)` is atomic and uses NLE
  semantics: ripple edits act on every unlocked track, linked clips follow
  each other, and edits that would desync or cut through other material are
  refused with the clip named.
- **Text is canonical.** `cut-lang` reads and prints the `.cut` document
  deterministically, omitting defaults, so a document diff is an edit diff.
  Every write is a script of operations in the same vocabulary; `print_ops`
  turns any operation list back into script. An annotated view adds end times
  and source outs for agents, and parsing ignores the annotations.
- **One-time import.** `cut-interchange::v2` converts v1 and v2
  `project.cut.json`. It snaps times to the frame grid, turns overlapping
  clips with transitions into cuts with handles, moves embedded sound onto
  linked audio clips, and reports anything without an equivalent.

## Roadmap

Each phase is its own pull request; the old stack keeps working until phase 5.

1. Kernel: `cut-model`, `cut-lang`, and the v2 import (this ADR).
2. `cutd` on the kernel: store `.cut` documents, journal operation scripts as
   history, show proposals as text diffs plus previews, and keep a temporary
   v2 export for the existing renderer, playback, and Qt app. Persist
   allocated ids so deleted ids are never reused.
3. Agent harness and MCP: `look` (media index and frames), `edit` (script →
   proposal with automatic check and preview), and `render`, with the
   annotated document and `reference()` as context. Skills keep editorial
   guidance only.
4. Render compiler, playback, and the Qt bridge read `cut-model`.
5. Delete `cut-domain`, `cut-core` compile, the v2 export, and the old
   harness.

## Consequences

- Project files are text and diff cleanly in version control, and agents read
  the same file people do. Agents need no tool discovery, so a typical edit is
  a single call.
- Adding a feature means adding a catalog definition plus its render lowering,
  not a new command, tool, or schema.
- v2 projects need the one-time import, and some v2 features (subtitle file
  overlays, project-level effects, 3D transform values) are reported rather
  than carried over.
- Ids are never reused. A project lists the ids of deleted items as
  `retired=` on its `project` line; the allocator skips them and edits that
  claim one are refused. Undo keeps an undone item's ids retired.
- Until phase 4, `cutd` derives a v2 project for the renderer, playback, the
  Qt app, and MCP. v2 commands from those clients become kernel operations
  where one exists (trims, splits, moves, deletes, media and clip insertion,
  markers, tracks). The rest run on the v2 export and are imported back with
  the model as a hint; a command whose effect the model cannot hold is
  refused with `NO_EQUIVALENT`.
