# The car on a road

The scene `docs/architecture.md` measures guides with (What directs a model,
Measured): a road tipped back under a sky out of depth, a car of two solids
driving across (`s2`), and the same car parked with the camera going round it
(`s3`). Each media's recipe draws one kind of guide of it and asks the model
named `shot` for the shot.

With a model set up (`docs/shots.md`, A model on a ComfyUI server), from a
copy of this folder:

```bash
kerf load --yes
kerf make depth --yes
```

A take is kept in `shots/<media>.takes/`, with the guide it was made from in
its `.work` folder.

Two more read a picture back (`docs/shots.md`, A picture read back), with the
models `depth` and `cameras` set up as `kerf-comfy`'s workflows of
those names:

- `readback`: the depth guide of the car driving across, the take `shot`
  makes of it, and that take's depth as a model reads it, which is the media.
  The guide's own depth is `guide.mp4` in the take's `.work` folder, to hold
  beside it.
  `look` with `{"follows": "readback", "sequence": "s2"}` says how far the
  take is from the guide over the pixels of each clip.
- `views`: the camera going round the parked car, with where a model
  puts the camera on each frame (`cameras.txt` in the `.work` folder) and
  the depth it agrees on across the frames, which is the media.

`project.kerf` was written again on 2026-11-13 from what
`docs/architecture.md` says of the scene: the one the first measurements were
made on was never in the repository, a `project.kerf` being a file Git is told
to leave out, and this one is added in spite of that. So its car and its
camera are near what was measured then, and not the same.
