---
name: meme
description: Make beat-aware meme edits with observed timing, canonical splits, speed changes, and reviewed crops.
---

# Meme edits

1. Read `timeline.query` for clip IDs and current placement. Inspect bounded
   source ranges with `media.probe` frames or `listen` when local transcription
   is configured. Beat detection is not an advertised probe action; do not
   claim measured beats without evidence.
2. Read `tools.describe` for `timeline.apply` with `commandType: "split_clip"`
   and then `commandType: "update_clip"`. Split at reviewed timeline frames,
   assign explicit new clip IDs, and stage speed changes in one atomic batch.
   A faster setup and slower punchline are options, not mandatory presets.
3. For a punch-in, split around the moment and use a normalized crop on the
   middle clip. Read the current command contract before adding transforms
   or keyframes. Return to the original framing unless the user wants it held.
4. Check the pending proposal with `timeline.check` and inspect
   `proposal.capture`. Use offline evidence for effects unsupported by native
   playback. Keep the edit pending until reviewed; after acceptance, use
   `render.preview` or `render.start` for the current project.

Prefer one or two clear changes per beat. Follow paginated queries and read
archived evidence when necessary; never repeat a mutation to recover output.
