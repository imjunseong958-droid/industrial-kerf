# Scenes of solids

Nine scenes blocked from Kerf's own solids for a model to film, a film of six
shots cut together from four more, and a media for each other thing a step
does. They were written by an agent with `kerf-mcp`'s tools on 2026-11-19, to
see how many kinds of guide a sequence makes and what a model makes of each.
`docs/architecture.md` (What directs a model, 3, "What came of a snowman")
has what was found on them and mended. How each media is made is in
`project.kerf` itself, as the steps under its line.

| Sequence | What is in it | What it tries |
|---|---|---|
| `snowman` | Three balls, a hat of two cylinders, a nose and arms of boxes, two trees | A camera going 70 degrees round, by keys of `x`, `z`, and `pan` |
| `rocket` | `models/rocket.glb` on a pad beside a tower | A file's mesh that rises out of the frame, the camera tilting after it |
| `solar` | A sun, four planets, a ring | Places that are expressions of `t`, each planet on a circle |
| `robot` | A robot of nine boxes and cylinders in an alley of six walls | Parts swung by expressions, parented to a null that bobs; each part has the walk's `z` keys itself, as a parent carried nothing of depth when the scene was made |
| `city` | Twelve towers and a spire | A camera that flies between them, rises, tilts, and rolls |
| `bowling` | A ball, six pins that tip, a lane | A signal that is 1 on the strike, and where the ball goes (`moves=ball`) |
| `donut` | `models/donut.glb` on a plate, a cup, a jar | A file's animation (`animation="Spin"`), and a still made first for the shot to look like |
| `saucer` | `models/saucer.glb`, a barn under a box turned on its edge, a silo, a cow of six boxes, a beam | A thing that flies in and stops, and a clip whose `opacity` comes up |
| `lighthouse` | Cylinders, a ball, boxes, a sea | A crane: the camera rises 2.5 frame heights while going 50 degrees round |
| `gaze`, `ready`, `voyage`, `landed` | The same robot and rocket | The film's shots that the scenes above do not block |
| `film` | Six shots, three titles, two sounds | The cut: `f1` to `f6` on one track, three dissolves, a fade |

With seven models set up as `kerf-comfy`'s workflows (`docs/shots.md`, A
model on a ComfyUI server; `kerf-comfy --describe` prints them all), `shot`,
`still`, `depth`, `cameras`, and `matte` by their own names, `directed` as
`shot-directed`, and `led` as `shot-led`, from a copy of this folder:

```bash
kerf load --yes
kerf make snowman_shot --yes
kerf shots
```

- `<scene>_shot`: the scene's depth pass, and `shot` asked for the shot.
  `bowling_shot` asks `directed` with the signal and the ball's way beside
  the guide, and `donut_shot` asks `still` first and hands the picture on as
  `ref=`.
- `f1` to `f6`: the film's shots, each naming who is in it and how it looks
  from `cast.kerfcast` (`cast=Bolt cast=rocket style=pulp`).
- `pass_picture`, `pass_depth`, `pass_relative`, `pass_ids`, `pass_normals`,
  `pass_matte`: the saucer's scene as each thing a `render` step draws, with
  no model asked. Each is drawn in about four seconds on four cores.
- `city_long`: a longer shot in two parts of 41 frames, the second going on
  from the first's last frame, joined (`docs/shots.md`, A longer shot).
- `orbit_views`: the snowman's guide as its picture, the camera a model
  finds of it (`cameras.txt` in the take's working folder), and its depth.
- `ufo_led`: the saucer's way alone, handed to a model as a layout.
- `snow_read` and `robot_matte` read a take, so make `snowman_shot` and
  `robot_shot` first: the snowman's take as its depth, and the robot in its
  take as a matte.

## What was made of them

Each was made once, seed 7, by Wan 2.1 VACE 1.3B on a ComfyUI server: 81
frames of 832x480. A making took 102 to 142 s; a depth read back 10 s, a
camera's path 6 s, a matte 13 s. The model is a small one kept for tests, so
how its takes look decides nothing (`docs/architecture.md`, the fifth rule).

| Shot | What its take kept |
|---|---|
| `snowman_shot` | Place and size, and the turn: the nose points at the camera first and to the left by the last frame. The trees as clipped hedges, in a forest the model added |
| `rocket_shot` | The rocket, the tower, the lift-off, and the tilt after it |
| `solar_shot` | The sun, a blue planet, a ringed one on their paths; the red one large at the frame's edge, the smallest hardly there |
| `robot_shot` | The robot, part for part, walking up the alley |
| `city_shot` | The towers and the flight between them; dark |
| `saucer_shot` | All of it: the barn, the silo, the cow, the saucer coming in, the beam from the frame it shows on. `look` with `placed` has the saucer where it was asked for, `overlap` 0.75 |
| `lighthouse_shot` | The lighthouse, the house, the cliff, and the crane |
| `bowling_shot` | The ball's way down the lane, and the pins down from the strike on |
| `donut_shot` | The donut as the still has it, on its plate |

The snowman's take, held to its guide by `look` with `held`: the frame's
order of near and far 0.989, and `held` 0.28, its hat followed on 19 of 81
frames.

The film, `film`, is 27 s. `look` with `eye` reads its five cuts between
shots by the guides and by the takes, and the two agree: where the guides
have the subject jump 0.24 of the frame's height at the first cut, the takes
have 0.25.

## What `stands` says of them

The parts of each thing reach a little way into the part under them, as a
ball set on a ball has to. So `look` with `stands` says the snowman's belly
is sunk 0.07 in its base, its head 0.04 in its belly, and its brim 0.025 in
its head, by its middle alone; the lighthouse's cap 0.12 in its lantern; the
barn's roof in the barn. They are meant to be, and are left
(`what_the_scenes_of_solids_stand_on_is_what_was_found_of_them`). A body on
legs it is wider than, the robot's and the cow's, is said to float over the
floor: its corners and its middle are over nothing, which is what is read of
it.

## The files

`models/rocket.glb`, `donut.glb`, and `saucer.glb` were written for this by
a script, of 468, 1,600, and 456 triangles: a profile turned about an
upright axis, fins, a ring, each part in a color of its own, and one
animation, `Spin`, a turn in four seconds. The saucer's dome is open
underneath, so its mesh does not close.

The film's two sounds are not in the repository. They are made into
`media/`:

```bash
ffmpeg -f lavfi -i "sine=f=110:sample_rate=48000:duration=28" -f lavfi -i "sine=f=164.81:sample_rate=48000:duration=28" \
  -f lavfi -i "sine=f=220:sample_rate=48000:duration=28" -f lavfi -i "sine=f=277.18:sample_rate=48000:duration=28" \
  -filter_complex "[0]volume=0.5[a];[1]volume=0.35,tremolo=f=0.23:d=0.6[b];[2]volume=0.25,tremolo=f=0.31:d=0.7[c];[3]volume=0.12,tremolo=f=0.17:d=0.8[d];[a][b][c][d]amix=inputs=4:normalize=0,lowpass=f=900,aecho=0.6:0.5:180|340:0.3|0.2,volume=0.5,afade=t=in:d=2,afade=t=out:st=25:d=3,pan=stereo|c0=c0|c1=c0" \
  -ar 48000 media/pad.wav
ffmpeg -f lavfi -i "anoisesrc=color=brown:sample_rate=48000:duration=6.5:amplitude=0.9" \
  -filter_complex "lowpass=f=260,highpass=f=30,volume=2.2,afade=t=in:d=1.2,afade=t=out:st=4:d=2.5,pan=stereo|c0=c0|c1=c0" \
  -ar 48000 media/rumble.wav
```
