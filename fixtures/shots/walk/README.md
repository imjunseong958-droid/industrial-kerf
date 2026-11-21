# A walk in layers

A shot to make a matte of: a man walking
toward the camera along a dirt path, 81 frames of 832x480 at 16 a second.
The footage is not in the repository. It is from Netflix's *Chimera* (2014),
Creative Commons Attribution 4.0 International (`licenses/README.md`), cut
from the 960x540 file of the Open Content bucket, into a copy of this
folder:

```bash
curl -fLo chimera.mp4 https://s3.amazonaws.com/download.opencontent.netflix.com/AV1/Chimera/Chimera-2397fps-AV1-10bit-960x540-1658kbps.mp4
mkdir -p footage
ffmpeg -ss 11 -i chimera.mp4 -vf "fps=16,scale=854:480:flags=area,crop=832:480,format=yuv420p" -frames:v 81 -an -c:v libx264 -crf 14 footage/walk.mp4
```

With four models set up as `kerf-comfy`'s workflows, `matte` as `matte`, `shown` as
`matte-shown`, `into` as `shot-into`, and `plate` as `plate`:

```bash
kerf load --yes
kerf make man --yes
kerf make layers --yes
```

How each media is made is in `project.kerf` itself, as the steps under its
line, each making one thing of the footage. A file is named from the
project's folder, and a later step reads what an earlier one made by its name
in braces:

```text
media gone path="shots/gone.mp4" rate=16 dur=81f video=832x480
  generate model=matte prompt="man" guide=footage/walk.mp4 out={man.mp4}
  generate model=plate prompt="an empty dirt path below a dry grassy hillside, bushes and trees, daylight" guide=footage/walk.mp4 guide_mask={man.mp4} seed=7 reach=72
```

- `man`: the man as a matte, white on black, by his name.
- `shown`: what `man, tree` finds, each thing in a color of its own with
  its number on it, which is the number `which=` keeps it by.
- `shirt`: his shirt's matte, and the footage with that part made anew.
- `gone`: the man taken out, and his shadow with him: the path and the
  hillside that were behind him, by the model made for that. `reach` takes
  his shadow with him, which the model takes out and his matte does not
  cover.
- `layers`: sequence `s1` as Kerf draws it, and no model: the footage, a
  title over it, and the footage again over that, shown through the man's
  matte (`key.trackmatte`, the matte on a track that is turned off). The
  title stands behind him. Make `man` first: it is the matte's take that
  the sequence plays.
- `moved`: sequence `s2` as Kerf draws it, and no model: `gone` on the
  lowest track, and over it the man through his matte, further along the
  path and half as large again, with the path whole where he was. Make
  `man` and `gone` first.

Neither sequence has a clip in depth, so `look` with `stands` has nothing
to measure of them and says so: every clip that draws is named as out of
depth.
