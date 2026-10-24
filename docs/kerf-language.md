# The Kerf language

A Kerf project is a `.kerf` text document. Every change to it is an edit
script in the same vocabulary. People, agents, and version control all read
the same file. The full vocabulary of properties, effects, transitions, and
generators is generated from the catalog:

```bash
cargo run -p kerf-lang --example reference
```

Raster effects and transitions (from EffectCraft) are listed there by id;
`look` with `effect` gives one's parameters, and they are written like any
other (`fx c1 add stylize.glow`, `transition add from=c1 to=c2
fx=transition.radialwipe dur=12f`). A point parameter is two numbers across
the picture, such as `center_x=0.5 center_y=0.5`. A file parameter (a LUT,
an OpenColorIO config) is a quoted path, relative to the project, such as
`fx c1 add utility.applylut lut="looks/warm.cube"`. A track parameter names
another picture track of the sequence, whose picture the effect reads even
while the track is hidden, as a matte or a displacement map does
(`fx c1 add channel.setmatte takeMatteFromLayer=V2`); an effect cannot read
its own track, and tracks cannot read each other in a loop. Raster effects
run on the clip's own picture before it is placed, so they apply before the
clip's other effects; a raster transition's progress follows the
transition.

## Documents

```text
kerf 1
project name="Short Film"

media m1 path="footage/A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000 reel="A001"

sequence s1 name="Main Edit" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set opacity=0.9 scale={0f:1 48f:1.1:ease}
      mask k1 ellipse=0.5,0.45,0.35,0.4 feather=40
      fx color.basic exposure=0.3
    clip c2 media=m1 src=14:23:02:00 at=01:00:06:12 dur=00:00:03:00
    transition t1 from=c1 to=c2 fx=dissolve dur=00:00:00:12
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set gain=-3
  track S1 subtitle
    clip cap1 caption="Where are we?" at=01:00:01:00 dur=00:00:02:00
  marker mk1 at=01:00:05:00 name="pickup needed" color=red
```

- `alpha=true` after `video=` marks media whose picture carries
  transparency; compositing keeps it.
- `retired=` on the `project` line lists ids of deleted items. Ids are never
  reused, so an edit cannot claim one and generated ids skip them.
- One line is one thing; two-space indentation nests tracks in sequences,
  clips in tracks, and `set`/`fx` lines in clips. `#` starts a comment.
- A clip's content is exactly one of `media=`, `seq=` (a nested sequence),
  `gen=` (a generator such as `title`), or `caption=`.
- `set` holds intrinsic properties (position, scale, crop, opacity, gain, …)
  and generator parameters. `fx` lines are the effect stack, top to bottom.
- `mask` lines are a picture clip's masks, combined in order. The outline is
  one of `rect=left,top,right,bottom`, `ellipse=x,y,radius across,radius
  down`, or `points="x,y x,y …"` (with `closed=false` for an open path), in
  fractions of the clip's picture, 0,0 at its top left. `mode=` is `add` (the
  default), `subtract`, `intersect`, `lighten`, `darken`, `difference`, or
  `none`, which cuts nothing and leaves a path for effects; `inverted=true`
  flips it. `feather=` and `expansion=` are in picture pixels and `opacity=`
  runs 0 to 1; all three animate. A crop still applies over the masks.
  Effects name a mask by its name (`fx generate.stroke path=k1`); names are
  unique within the clip.
- Clips sharing a `link=` group, such as a picture and its sound, edit
  together.
- Printing is deterministic and omits defaults. Printing with annotations
  appends comments with each clip's end and source out; parsing ignores them.

### Times

- **Positions** (`at=`, `start=`, markers) are timecode on the sequence,
  `HH:MM:SS:FF`, or `HH:MM:SS;FF` for drop-frame 29.97 and 59.94.
- **Source positions** (`src=`, `out=`) are timecode on the media.
- **Durations** (`dur=`) count frames in the same `HH:MM:SS:FF` shape.
- Anywhere a time is read, `96f` and `4.5s` also work as offsets from the
  start. Seconds round to the nearest frame.
- A position in a script may also be an **anchor**: `end` (the end of the
  sequence), `c3.start`, or `c3.end`, each with an optional amount, as in
  `at=end`, `at=c3.end`, or `to=c3.start-6f`. Anchors read the project as
  the line runs, so they follow earlier lines' edits.
- Keyframe times are relative to the clip: `{0f:0 12f:1:ease}`.

Internally every time is an exact integer of flicks (1/705,600,000 s), so
23.976, 29.97 DF, and 48 kHz audio never drift.

## Scripts

```text
insert media=m1 src=14:22:50:00 dur=00:00:04:00 at=01:00:06:12 on=V1,A1 as=c9,a9
trim c9 out -6f
roll c1 +12f
set c9 opacity={0f:0 12f:1}
fx c9 add color.basic exposure=0.3
mask c9 add k1 rect=0.1,0.1,0.9,0.9 feather=20
transition add from=c1 to=c9 fx=dissolve dur=12f
```

- Lines run in order and atomically: if any line fails, nothing changes, and
  the error names the line and column.
- Later lines see what earlier lines did, so `as=` names let a script refer to
  the clips it creates.
- `insert` opens time on every unlocked track and `overwrite` replaces what
  is underneath. `on=V1,A1` places a picture and its sound as linked clips;
  `out=` gives a source out point instead of `dur=`.
- `ripple-delete`, `extract`, and ripple trims keep every unlocked track in
  sync, and refuse when they would cut through another track's clip. Lock a
  track to leave it out.
- `trim`, `roll`, `slide`, and `move` accept either an amount (`+12f`) or an
  absolute `to=`/`at=` position.
- `trim … src=<source position>` puts the edge where the clip plays that
  position of its media, so a cut can land on a word's time from a
  transcript.
- With several sequences, `use s2` selects the one later lines act on.

Every operation prints back as script (`print_op`, `print_ops`), so history,
UI actions, and agent edits share one readable log.

## In `kerfd`

`kerfd` keeps the project as `project.kerf`, written after every change. Edits
arrive as scripts (`apply_script`) or typed operations from the editor
(`apply_ops`, frame counts on the sequence). Each change is journaled as
the script it ran, and undo restores the project as it was. Agent edits stay pending proposals that show the
script and a diff of the document; `get_document` returns the document,
optionally annotated. Render jobs and proposal previews hand renderers the
document with media paths resolved against the project.
