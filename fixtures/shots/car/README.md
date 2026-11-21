# The car on a road

A scene to make guides of: a road tipped back under a sky out of depth, a car of two solids
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

With a model set up, from a copy of this folder:

```bash
kerf load --yes
kerf make depth --yes
```

A take is kept in `shots/<media>.takes/`, with the guide it was made from in
its `.work` folder, and the steps it was made from beside the guide. An agent
makes one with `make`, and changes a word for the next with an edit:
`step depth set 2 seed=8`.

Two more read a picture back, with the
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

`project.kerf` is a file Git is told to leave out; this one is added in spite of that,
so the scene the tests read is in the repository.
