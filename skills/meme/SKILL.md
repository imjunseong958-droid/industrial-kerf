---
name: meme
description: Beat-aware meme edits via split, speed, crop, and optional v2 transform keyframes.
---

# Meme edits

Use short, reversible timeline operations to make the beat legible without
stacking effects that fight each other.

If only the compact index was injected, call skill_read with `{ "name": "meme" }`
to load this playbook.

Use existing `timeline` / `probe` / `project_edit` / `render`.

## Workflow

1. **Inspect timing.** `timeline` for clip IDs; `probe` for beat boundaries and
   optional `frames`.
2. **Build ramps.** `project_edit` with legacy `split` at reviewed beat
   boundaries, then set speed on each resulting clip (`update_clip` patch or
   legacy `set_speed`: build-up ~1.5-2x, punchline ~0.5x, return to 1x). Keep
   split and speed changes in one atomic batch when clip IDs are known.
3. **Punch in.** For a static punch-in, split around the moment and apply a
   normalized crop to the middle clip in the same atomic batch. For a short
   eased move, use v2 `transform` / `keyframes` on `update_clip` when exposed.
4. **Preview.** `render` with `{draft:true}`; inspect punchline, framing, and
   return to neutral speed before the full render.

## Guardrails

- Keep punch-ins to roughly 0.2-0.4 seconds and at most two effects on one
  beat (for example, crop plus speed).
- End every ramp or reframing at a neutral state unless the user asks for a
  deliberately held effect.
- Use only structured timeline operations. If the configured tools cannot
  represent a requested effect, report the limitation rather than fabricating
  an operation.
