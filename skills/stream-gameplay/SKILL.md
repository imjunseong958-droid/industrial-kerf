---
name: stream-gameplay
description: Reframe gameplay while preserving the requested game, camera, and chat regions with reviewed crops.
---

# Stream/gameplay reframing

1. Read `project.outline` and bounded `timeline.query` pages for the source,
   placement, and revision. Inspect filmstrips and selected exact frames with
   `media.probe`. Treat game content as the framing anchor unless the user
   requests another subject.
2. Identify game content, camera, and chat regions from available evidence.
   `vision.track_objects` may report `CAPABILITY_UNAVAILABLE`; use observed
   frames when tracking is unavailable and do not invent tracked motion.
3. Keep requested regions visible. If one crop cannot preserve all requested
   regions, explain the composition choice. For a layered proposal, use the
   offline preview because native playback does not composite all layers.
4. Read `tools.describe` for the required `timeline.apply` command types.
   Stage related crop, placement, and supported transform changes in one
   atomic batch. Coordinates must be finite, normalized, and within the frame.
5. Run `timeline.check` on the pending proposal and inspect `proposal.capture`
   for visibility, crop edges, and motion. Recheck proposal status before
   subsequent edits. Render the current project after acceptance as requested.

Never silently crop away requested chat or camera content. Use structured
commands and preserve the original project until the proposal is accepted.
