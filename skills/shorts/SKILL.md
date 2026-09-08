---
name: shorts
description: Build a reviewed vertical short from observed source ranges, with framing, captions, and an offline preview.
---

# Vertical shorts

Create a focused 15–60 second cut unless the user specifies another duration.

1. Read `project.outline` and a bounded `timeline.query`. Follow pagination or
   expand a node with `nodePath` when more detail is needed.
2. Inspect source evidence with `media.probe`, using `action: "observe"` and
   an explicit `opts.start` plus `opts.duration`. Use a filmstrip first;
   request exact frames or local transcription only for the selected range.
3. Choose the strongest hook and source ranges. Use `workflow.roughcut` with
   those selections and a rationale for each. The result is a pending proposal,
   not an applied edit.
4. For crop or timing refinements, read `tools.describe` with
   `{"name":"timeline.apply","commandType":"update_clip"}`. Stage related
   changes as one atomic batch. Crop coordinates are normalized and must stay
   within the source frame. Preserve words needed to understand the hook.
5. Use existing subtitles or observed transcript cues. Read the `add_overlay`
   command contract before staging captions. Do not invent speech. Existing
   local media may be staged with `media.asset`; direct fetching and history
   changes are not tools of the built-in agent.
6. Run `timeline.check` on the pending proposal and use `proposal.capture`
   to inspect its offline before/after preview. This is required when live
   playback cannot represent layering, audio mixing, or the requested effect.
   After acceptance, render the current project with `render.preview` or
   `render.start` as requested.

Keep the proposal pending for user review. Recheck revision and proposal
status before further editing; an archived tool result is historical evidence.
Retrieve abbreviated results with `session.read` in the built-in agent or
`results.read` when the MCP server returns an evidence handle.
