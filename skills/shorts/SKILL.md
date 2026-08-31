---
name: shorts
description: 9:16 vertical shorts with probe, timeline, project_edit, fetch, and render.
---

# Vertical shorts

Turn a longer source into a focused 15-60 second vertical cut. Keep every
decision in the validated v2 project so it can be previewed and revised.

If only the compact index was injected, call skill_read with `{ "name": "shorts" }`
to load this playbook.

Use existing `timeline` / `probe` / `project_edit` / `render` (and `fetch` for URLs).

## Workflow

1. **Inspect first.** `timeline` for source and clip IDs; `probe` for
   `info`, `silence`, and `frames` when framing is uncertain.
2. **Tighten the cut.** Turn reviewed silence ranges into one non-empty atomic
   `project_edit` batch. Legacy `remove_ranges` is fine. Keep words needed
   for the hook.
3. **Choose the hook.** Put the strongest claim, question, or visual beat in
   the first three seconds. Do not open on dead air or an unreviewed crop.
4. **Frame for 9:16.** Prefer v2 `update_clip` with a normalized `crop`
   patch; legacy `set_crop` in the same atomic batch is OK. Use output
   dimensions or transform keyframes when the host exposes them; otherwise
   state the limitation instead of inventing an unsupported effect. When
   `track_objects` is available, call it and use the returned `content` ROI
   as the primary subject; include `cam`/`chat` only when the user wants those
   overlays kept. If tracking is unavailable, use `probe` frames for crop
   decisions as above.
5. **Use the supplied subtitles.** For a YouTube source, `fetch` returns
   `subsPath` (normally `.vtt`). Pass that path through `project_edit` with
   `add_subtitles`. Use inline `add_captions` only for cues the user supplied
   or the project already contains.
6. **Preview before delivery.** `render` with `{draft:true}` and inspect the
   result. Run the full render only after crop, hook, timing, and subtitles
   are acceptable.

## Guardrails

- Keep the final cut at or below 60 seconds unless the user asks otherwise.
- Keep crop rectangles normalized, finite, and inside the frame.
- Use only structured timeline operations. If an effect is not represented by
  an available command or legacy op, explain the limitation and ask before
  changing the workflow.

Checklist: hook ≤ 3s, reviewed 9:16 framing, concise timing, subtitle source
identified, draft preview inspected, full render approved.
