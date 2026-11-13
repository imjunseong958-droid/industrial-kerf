---
name: model-shot
description: Block a shot in Kerf and have a video model film it, from a guide sequence and a recipe to a making a person approves and the takes it leaves.
---

# A shot a model films

- Start from the model. `look` lists `models` with what each takes: make the
  guide a sequence of its own, in the model's shape (`size`) and no longer
  than its `seconds`, and write the `render` line `look` gives as `guide`, so
  the guide is drawn at the model's size and rate and not the sequence's.
- A guide is read for layout, motion, and camera, not for finish. Flat colors
  and plain solids that differ clearly from each other guide better than
  detail; what the picture should look like goes in the prompt. A model keeps
  where things are and how they move, and takes their color from the prompt
  alone: a white box is a dark boat until the prompt says a white hull. What
  kind of thing it is comes from the prompt and its outline: one box for a
  car was a van, and a low body with a cabin on it was a car.
- Stack the scene with `overwrite`, a clip a track, each at `at=0f` for the
  length of the shot. `insert` opens time, and puts each clip after the last.
  The camera and the lights go on the top tracks.
- In depth x runs right, y down, and z away from the viewer, in frame
  heights (x in frame widths); `depth` gives a clip sides. What is nearer
  at a place in the picture is drawn over what is farther there. Make a
  floor a large clip tipped back, below the middle (`rotate_x=-84 y=0.75
  z=4 scale=16`). Tipped so, it is 0.1 lower for each frame height nearer:
  1.0 where z is 1.6. A clip stands on it with its foot there. What of a
  clip is under the floor is hidden, and a model leaves out what is hidden.
- Leave a far backdrop, a sky, out of depth: a plain clip on the lowest
  track draws flat behind the scene, unlit. In depth it takes longer to
  draw, and where it meets the floor is the take's horizon.
- A camera's `z` moves it forward, `pan` turns it right, and `tilt` up. A
  directional light's `x y z` is the way it shines. With a light that casts
  a `shadow`, add an `ambient` one, or what faces away is black. A guide may
  go without lights: a model lights the shot itself, and made as good a take.
- When something has to happen on a frame, a ball landing, a flash, smoke
  building, and the model's `reads` has `signal`, say when with a signal: a
  `gen=signal@1` clip on a track of its own in the guide's sequence, from
  `at=0f` for the shot's length, with `value` keyed 0 to 1, and
  `signal=<clip>` on the `generate` line. The prompt says what the number is
  of. Hold a peak for two of the model's frames, or it arrives lower. A
  model that does not read one is said to have no use for it in `notes`:
  leave it off the line.
- Read the guide before writing the recipe, at its first, middle, and last
  frame: `look` with `shows` and `at` says of each clip where it is (`box`),
  what hides it (`hidden`, `by`), and which edges of the frame it reaches.
  Mend a subject with a floor in its `by`, one with no `box`, which has left
  the frame, and one at an edge it should not be at. Then look at the frames
  themselves: nothing crossing by accident, the move plain.
- The recipe is the `render` line and a `generate` line: the prompt,
  `guide={guide.mp4}`, `seconds`, and a `seed`, so that a take can be made
  again. Write it with the edit, in `recipes`, beside the script that adds
  the media line; the media's line names the file the shot will be, with the
  model's rate and size, and a making replaces them with what was made. A
  picture is fit inside the frame before its `scale`, so a shot of another
  shape leaves edges: the check says the `scale` that fills it.
- Mend what `toMake`'s `cannot` and `notes` say, then ask for the making
  once with `make`, with `why`. Asked, the person makes it; let through, it
  runs as a job you `wait` for. A making costs minutes and money: never ask
  for one to see the guide, which `look` shows.
- After a making, look at the take (`media` with `sheet`), not at the guide,
  and say what the model kept and what it lost. Change one thing for the
  next take: the guide for layout and motion, the prompt for the look, the
  seed for another roll of the same. Any change to the guide, one too small
  to see among them, brings other scenery with it: judge a take against the
  one before for place, path, and size, and keep a take that is wanted by
  leaving its guide as it is. A take that was better is chosen again with
  its `use` line.
