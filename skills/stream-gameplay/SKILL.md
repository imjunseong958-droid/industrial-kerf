---
name: stream-gameplay
description: Reframe streams around cam, chat, and content tracking with reviewed v2 crop and transform.
---

# Stream/gameplay reframing

Reframe a landscape stream or gameplay recording into a vertical or focused
layout. Use only the three configured tracking classes: `cam` (streamer camera
overlay), `chat` (chat overlay), and `content` (main gameplay/stream canvas).
Treat `content` as the default framing anchor when the user wants the game or
stream itself visible; add `cam` and `chat` when those overlays should stay on
screen.

If only the compact index was injected, call skill_read with `{ "name": "stream-gameplay" }`
to load this playbook.

Use existing `timeline` / `probe` / `project_edit` / `render`, and optional `track_objects` when configured.

## Workflow

1. **Inspect the project.** `timeline` first: source clip, output aspect,
   duration, and current revision.
2. **Track exactly three targets.** When configured, call `track_objects` with
   the source path. Use only `cam`, `chat`, and `content`.
3. **Review the crop plan.** Keep class-separated ROIs for `cam`, `chat`, and
   `content`; use `plan.keyframesByClass.content` / `byClass.content` for the
   main-picture ROI at the requested aspect ratio. Show proposed bounds and
   whether each target is included; reject obvious outliers. If chat must be
   preserved and one crop cannot fit cam, chat, and content together, widen the
   crop or add a separate v2 video track/clip when exposed. Never silently crop
   chat away.
4. **Apply one safe transaction.** After the plan is accepted, `project_edit`
   with one non-empty atomic `commands` batch of `update_clip` patches
   (`crop`, `transform`, `keyframes`; time relative to the containing clip).
   Include chat-preserving geometry in the same batch. A rejected batch must
   leave the project unchanged.
5. **Preview, then render.** Draft preview first; inspect cam, chat, content
   visibility, crop edges, and motion. Full render only after acceptance.

## Guardrails

- Tracking targets for this skill are only `cam`, `chat`, and `content`.
- Keep crop coordinates normalized, finite, and within `[0,1]`; clamp before
  writing to the timeline.
- Do not use shell commands or hand-edit the project around the transaction.
  If `track_objects` or v2 commands are unavailable, report the gap
  and stop before mutating.
