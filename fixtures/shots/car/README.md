# The car on a road

The scene `docs/architecture.md` measures guides with (What directs a model,
Measured): a road tipped back under a sky out of depth, a car of two solids
driving across (`s2`), and the same car parked with the camera going round it
(`s3`). Each media's recipe draws one kind of guide of it and asks the model
named `vace` for the shot.

With a model set up (`docs/shots.md`, A model on a ComfyUI server), from a
copy of this folder:

```bash
kerf load --yes
kerf make depth --yes
```

A take is kept in `shots/<media>.takes/`, with the guide it was made from in
its `.work` folder.
