# A walk in layers

The shot `docs/shots-measured.md` (12) measures a matte with: a man walking
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

With four models set up as `kerf-comfy`'s workflows (`docs/shots.md`, A
thing in footage, by its name), `matte` as `matte`, `shown` as
`matte-shown`, `into` as `shot-into`, and `plate` as `plate`:

```bash
kerf load --yes
kerf make man --yes
kerf make layers --yes
```

Each media's recipe makes one thing of the footage:

- `man`: the man as a matte, white on black, by his name.
- `shown`: what `man, tree` finds, each thing in a color of its own with
  its number on it, which is the number `which=` keeps it by.
- `shirt`: his shirt's matte, and the footage with that part made anew.
- `gone`: the man taken out, and his shadow with him: the path and the
  hillside that were behind him, by the model made for that.
- `layers`: sequence `s1` as Kerf draws it, and no model: the footage, a
  title over it, and the footage again over that, shown through the man's
  matte (`key.trackmatte`, the matte on a track that is turned off). The
  title stands behind him. Make `man` first: it is the matte's take that
  the sequence plays.
- `moved`: sequence `s2` as Kerf draws it, and no model: `gone` on the
  lowest track, and over it the man through his matte, further along the
  path and half as large again, with the path whole where he was. Make
  `man` and `gone` first.
