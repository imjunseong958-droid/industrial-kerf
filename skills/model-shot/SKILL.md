---
name: model-shot
description: Block a shot in Kerf and have a video model film it, from a guide sequence and the steps that draw and film it to the takes a making leaves.
---

# A shot a model films

- Start from the model. `look` lists `models` with what each takes: make the
  guide a sequence of its own, in the model's shape (`size`) and no longer
  than its `seconds`, and write the `render` step `look` gives as `guide`, so
  it is drawn at the model's size and rate.
- Which kind of guide a model follows is the model's, and its `about`
  says: the picture, for a model that covers a render, or a pass of it
  (`pass=depth` on the `render` step), for one that follows depth. With a
  depth pass the guide's colors say nothing, so the prompt gives every
  color, and a backdrop out of depth is black.
- A guide is read for layout, motion, and camera, not for finish: plain
  solids that differ clearly guide better than detail. A model keeps where
  things are and how they move, and takes what each is and its color from
  the words: a white box is a dark boat until they say a white hull. One
  prompt does not say which solid is which: two boxes and "a red car and a
  white van" came out both red and white. Say it on each solid,
  `prompt="a red car"`, and a model whose `reads` has `regions` hears it over
  where the guide shows that clip; the step's prompt says the rest.
- Stack the scene with `overwrite`, a clip a track, each at `at=0f` for the
  shot's length (`insert` puts each after the last), the camera and lights
  on top.
- A thing seen from more than one side is a `gen=mesh@1`:
  `kind=box|ball|cylinder`, `width`, `height`, and `length` in frame
  heights, and a `color`. It is in depth, its middle at `x`, `y`, `z`, and
  has every side, so a camera can go round it; a body and a cabin are two,
  and with a `gen=null@1` as `parent=` move and turn as one. With no light
  it is one flat color. With `file="car.glb"` it is a glTF model, `height`
  tall, a color a part; replacing the file asks for the shot again. A part
  the file gives a picture or corner colors is drawn in them, unlit.
  `animation="Walk"` plays one of its animations, by name or number,
  looping; `seconds={0f:0 48f:1}` retimes it.
- A set rebuilt from a shot is seen through the shot's camera: `look`
  with `mesh: {"clip": "set", "camera": "shot"}` returns the lines that
  add a camera of the file, or with `from` one a model wrote. What is
  added stands in the set, before or behind its walls.
- In depth x runs right, y down, and z away from the viewer, in frame
  heights (x in frame widths); `depth` gives a flat clip sides. What is
  nearer is drawn over what is farther. Make a floor a large clip tipped
  back, below the middle (`rotate_x=-84 y=0.75 z=4 scale=16`). Put a thing
  near its place: `look` with `stands` gives the `script` that sets it
  down. What of a clip is under the floor is hidden, and a model leaves
  out what is hidden.
- Leave a far backdrop, a sky, out of depth: a plain clip on the lowest
  track draws flat behind the scene, unlit. In depth it takes longer to
  draw, and where it meets the floor is the take's horizon.
- A camera's `z` moves it forward, `pan` turns it right, and `tilt` up. A
  directional light's `x y z` is the way it shines. With a light that casts
  a `shadow`, add an `ambient` one, or what faces away is black. A guide may
  go without lights: a model lights the shot itself, and made as good a take.
- When something has to happen on a frame, a ball landing, a flash, and
  the model's `reads` has `signal`, say when with a signal: a
  `gen=signal@1` clip on a track of its own in the guide's sequence, from
  `at=0f` for the shot's length, with `value` keyed 0 to 1, and
  `signal=<clip>` on the `generate` step. The prompt says what the number is
  of. Where the moments are a sound's, music's beats or what was recorded,
  `look` with `signal` writes the keys from the clip that plays it. Hold a
  peak for two of the model's frames, or it arrives lower. A model that
  reads none says so in `notes`: leave it off the step.
- Read the guide before writing the steps, at its first, middle, and last
  frame: `look` with `shows`, `stands`, and `at`. `shows` says where each
  clip is (`box`), what hides it (`hidden`, `by`), and the `edges` it
  reaches; `stands`, what `floats`, is `sunk`, or goes `through` another.
  Mend those, and a subject with a floor in its `by`, no `box`, or a wrong
  edge. Then see the frames: nothing crossing by accident, the move plain.
- The shot is a media with two steps under it: `step <id> add render
  <sequence> {guide.mp4} …`, then `step <id> add generate model=…
  prompt="…" guide={guide.mp4}` with `seconds` and a `seed`. The media's
  line names the shot's file, with the model's rate and size; a making
  replaces them with what was made. A picture is fit inside the frame
  before its `scale`, so a shot of another shape leaves edges: the check
  says the `scale` that fills it.
- A shot cut beside another: `look` with `eye`. Before making, `guide`
  has each subject (the step's `moves=`, or `eye`'s `subjects`) either
  side of the cut, its `jump`, `size`, and `reversed`; the eye holds under
  0.1, and the guide mends it. After, where `guide` holds and `take`
  jumps, roll again.
- `look` lists the project's `cast` and `styles`: name them,
  `cast=Ada style=film`, and have the prompt say what they do, not how
  they look; `edit`'s `cast` adds one.
- Mend what `toMake`'s `cannot` and `notes` say, then make it once with `make`,
  with `why`, after the edit is accepted. Let through, it runs as a job you
  `wait` for; asked, the person makes it. A making costs minutes and money:
  never make one to see the guide, which `look` shows.
- After a making, look at the take (`media` with `sheet`), not at the guide,
  and say what the model kept and what it lost. Change one thing for the
  next take: the guide for layout and motion, the words for the look, the
  seed for a roll (`step <id> set 2 seed=8`). Any change
  to the guide, one too small to see among them, brings other scenery with
  it: judge a take against the one before for place, path, and size, and
  keep a take that is wanted by leaving its guide as it is. A take that was
  better is chosen again with its `use` line.
