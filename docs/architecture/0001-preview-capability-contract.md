# ADR 0001: Native preview capability contract

Status: accepted for Phase 0
Date: 2026-09-01

## Context

Cut has one canonical project owner, `cutd`, but its native playback path and
offline FFmpeg compiler do not evaluate every authored property equally. An
editor that exposes an unsupported property without identifying the mismatch
can show a preview that differs from export.

## Decision

- `cutd` remains the only owner of project state, revision, history,
  idempotency receipts, proposals, and render jobs.
- Accepted edits reach `cutd` as one typed command or atomic batch. UI state is
  limited to selection, viewport, drag previews, and bounded run events.
- Project time remains `f64` seconds in schema v2. Future editor gestures must
  quantize through one frame-grid helper before sending a command.
- `cut-core::capability` is the typed product contract for native playback and
  offline compilation. `Exact` is the only level safe to present as an exact
  native preview; `Limited` and `Unsupported` require an explanation and later
  rendered fallback.
- Computed playback/render plans remain projections of authored state, never a
  second reducer or project document.
- Thumbnails, waveforms, transcripts, filmstrips, and captures are regenerable
  content-addressed artifacts under `.cut/cache/derived/`, not canonical data.
- Proposal review remains atomic with one pending proposal. Requesting changes
  rejects it before starting a replacement run.
- The native Rust stack remains the production surface. This work introduces
  no Node runtime, browser shell, second UI toolkit, or upstream runtime code.
- Timeline rendering must cull by visible time and row and stay idle without
  continuous repainting.

## Consequences

Unsupported live features are discoverable before playback. The initial matrix
is deliberately conservative: it documents current behavior rather than
promising future parity. Adding an editor control requires updating the typed
matrix and focused behavior tests first. Longer term, native playback and
offline export should consume shared normalized evaluation semantics.
