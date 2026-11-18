# The car on a road

The scene `docs/architecture.md` measures guides with (What directs a model,
Measured): a road tipped back under a sky out of depth, a car of two solids
driving across (`s2`), and the same car parked with the camera going round it
(`s3`). How each media is made is in `project.kerf` itself, as the steps under
its line: one kind of guide drawn of the scene, and the model named `shot`
asked for the shot. Nothing beside the document says it, so the scene and
what is made of it are read, edited, and undone together:

```text
media depth path="shots/depth.mp4" rate=16 dur=65f video=832x480
  render s2 {guide.mp4} pass=depth size=832x480 rate=16
  generate model=shot prompt="a red sports car drives along a wet city street at dusk, cinematic, photorealistic" guide={guide.mp4} seconds=4 seed=7
```

- `color`: the guide as its picture, in flat colors.
- `depth`: the guide as how near each pixel is.
- `ids`: the guide as which clip each pixel is of.
- `orbit`: the camera goes 75 degrees round the car, parked, as depth.

With a model set up (`docs/shots.md`, A model on a ComfyUI server), from a
copy of this folder:

```bash
kerf load --yes
kerf make depth --yes
```

A take is kept in `shots/<media>.takes/`, with the guide it was made from in
its `.work` folder, and the steps it was made from beside the guide. An agent
makes one with `make`, and changes a word for the next with an edit:
`step depth set 2 seed=8`.

Two more read a picture back (`docs/shots.md`, A picture read back), with the
models `depth` and `cameras` set up as `kerf-comfy`'s workflows of
those names:

- `readback`: the depth guide of the car driving across, the take `shot`
  makes of it (`out={take.mp4}`), and that take's depth as a model reads it,
  which is the media: three steps, each reading what the one before made.
  The guide's own depth is `guide.mp4` in the take's `.work` folder, to hold
  beside it.
  `look` with `{"follows": "readback", "sequence": "s2"}` says how far the
  take is from the guide over the pixels of each clip.
- `views`: the camera going round the parked car, with where a model
  puts the camera on each frame (`cameras.txt` in the `.work` folder) and
  the depth it agrees on across the frames, which is the media.

What `look` with `stands` says of it (2026-11-18), which is left as it is,
its takes being of this guide: in `s2` and in `s3` the body is sunk 0.032
of the frame's height in the road with half of its feet, on every frame.
Its foot is level at 0.95, and the road leans 6 degrees, 0.960 under the
body's front and 0.918 under its back, so the road hides its far side from
the depth pass. `set body y=0.808` would have it rest on its far side with
0.042 open under its front, and the cabin, which stands on the body, would
then be 0.032 in it until it is moved up as well. Nothing goes through
anything, and the sky is out of depth.

`project.kerf` was written again on 2026-11-13 from what
`docs/architecture.md` says of the scene: the one the first measurements were
made on was never in the repository, a `project.kerf` being a file Git is told
to leave out, and this one is added in spite of that. So its car and its
camera are near what was measured then, and not the same.
