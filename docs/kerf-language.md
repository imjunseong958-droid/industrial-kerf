# The Kerf language

A Kerf project is a `.kerf` text document. Every change to it is an edit
script in the same vocabulary. People, agents, and version control all read
the same file. The full vocabulary of properties, effects, transitions, and
generators is generated from the catalog:

```bash
cargo run -p kerf-lang --example reference
```

A video editor's transitions, carried from FilmCraft, are listed there by
category: pushes, slides, whips, zooms, flashes, light leaks, glitches,
irises, page peels, and more, drawn from both clips across the cut
(`transition add from=c1 to=c2 fx=push@1 dur=12f direction=from-east`). A
`direction` names the side the incoming clip enters from; a center is
`center_x` and `center_y` in fractions of the frame; pixel sizes are canvas
pixels. Kerf's own renderer draws them.

A video editor's effects, carried from FilmCraft, are listed there by
category too: Lumetri grades, blurs, keys and track mattes, glows, light
leaks, distortions, moves that animate over the clip, burn-ins, and 360°
effects (`fx c1 add color.lumetri@2 exposure=0.5 look=teal-orange`). They
run in stack order on the clip's picture after its masks, and Kerf's own
renderer draws them. A point is `<name>_x` and `<name>_y` across the
picture; an offset is in pixels; a curve is text of `x,y` points
(`curve_luma="0,0 0.5,0.6 1,1"`); a LUT is a quoted `.cube` or `.3dl` path;
another track is its id (`fx c1 add key.trackmatte matte=V2`), read even
while it is hidden. Sizes are the clip's source pixels. Where FilmCraft's
effect replaced an EffectCraft one under the same id, it is version 2 of
that id (`channel.invert@2`), and version 1 no longer reads.

Kerf also keeps 24 of EffectCraft's effects that FilmCraft has nothing like
(grain, dust, rolling shutter, lens and upscaling repair; key and matte
clean-up; Curves, Hue/Saturation, Selective Color, Photo Filter, and the Auto
corrections; a few blurs and distortions; the audio visualizers), listed
there by id; `look` with `effect` gives one's parameters, and they are
written like any other (`fx c1 add color.curves rgb="0,0 0.5,0.6 1,1"`). A
point parameter is two numbers across the picture, such as `center_x=0.5
center_y=0.5`. A sound parameter names a sound track whose sound the effect
draws (`fx c1 add generate.audiowaveform audioLayer=A1`), and a mask
parameter one of the clip's masks (`path=k1`). Kerf's own renderer runs them
in stack order beside FilmCraft's effects. Any other id EffectCraft had is
an error that names what to use instead (`stylize.glow` is
`light.wonderglow@1` now).

`image.fill` removes what a mask outlines by filling it from the rest of the
picture. Give the mask `mode=none`, so the picture stays whole for it:
`mask c1 add k1 rect=0.4,0.4,0.6,0.6 mode=none`, then `fx c1 add image.fill
path=k1`. `method=object` matches texture within the frame, for stills and locked-off shots;
`method=smooth` blends the edges across, for skies and walls; and `method=video` takes what the
area hides from the frames around it along the motion, for anything that moves, in runs of
`reach` frames either side. An outline holds still for now, so this removes what stays in place
in the picture, as a logo or a spot does, while the shot moves behind it. Identical frames, as a
still has, reuse one fill.

Looks are the playful effects of a short-form editor written in the Kerf
language from those effects ([looks.md](looks.md)): shakes, zoom pulses,
flashes, glitches, film and VHS looks, title pop-ins, and whips and zooms
through a cut. They are listed there too and written like any effect or
transition (`fx c1 add look.shake@1 intensity=0.7 hit=12f`, `transition add
from=c1 to=c2 fx=look.whip@1 dur=12f`); a time parameter is a time in the
clip, or the id of a marker in its sequence to land on (`hit=beat1`). The document keeps the one line and the look expands when it draws;
`fx c1 flatten 1` replaces it with what it expands to. A project's own looks
are `.kerf/looks/*.kerflook` files.

A track parameter names another picture track of the sequence, whose
picture the effect reads even while the track is hidden, as a track matte
does; an effect cannot read its own track, and tracks cannot read each other
in a loop.

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
      fx color.basic@1 exposure=0.3
    clip c2 media=m1 src=14:23:02:00 at=01:00:06:12 dur=00:00:03:00
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set gain=-3
  track S1 subtitle
    clip cap1 caption="Where are we?" at=01:00:01:00 dur=00:00:02:00
  marker mk1 at=01:00:05:00 name="pickup needed" color=red
```

- `alpha=true` after `video=` marks media whose picture carries
  transparency; compositing keeps it.
- `bits=` (8, the default, 10, 12, or 16) says how deep a picture is, and drawing keeps
  all of it, so a grade that stretches it does not band. `color=` says how its code values
  become light: `rec709` (the default), `srgb`, `rec2020`, `p3d65`, `rec2100-pq`,
  `rec2100-hlg`, `linear709`, or a camera's log such as `slog3-sgamut3cine`, `logc3-awg3`,
  `logc4-awg4`, `vlog-vgamut`, `clog3-cinemagamut`, or `applelog`. HDR and log come in tone
  mapped for an SDR picture. Importing finds both from the file's tags; log footage rarely
  tags itself, so write its `color=` by hand, with `media replace`.
- `retired=` on the `project` line lists ids of deleted items. Ids are never
  reused, so an edit cannot claim one and generated ids skip them.
- One line is one thing; two-space indentation nests tracks in sequences,
  clips in tracks, and `set`/`fx` lines in clips. `#` starts a comment.
- A clip's content is exactly one of `media=`, `seq=` (a nested sequence),
  `gen=` (a generator such as `title`), or `caption=`.
- `set` holds intrinsic properties (position, scale, crop, opacity, gain, …)
  and generator parameters. `fx` lines are the effect stack, top to bottom.
- Effects, transitions, and generators are named with their version,
  `blur.gaussian@1`. Kerf always writes it; a reference without one, as in a
  hand-written script, takes this Kerf's. A version newer than this Kerf's
  is an error, and an older one it still reads is drawn by the current one
  with a warning ([looks.md](looks.md#versions)).
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
- `set path="0,0 0.2,-0.1 0.4,0" along={0f:0 24f:1:ease}` moves a clip along a smooth curve
  through those points, in the units of `x` and `y`, which it adds to. `along` is by distance
  on the canvas, so an even `along` is an even speed. `orient=true` turns the clip to face the
  way it is heading.
- `parent=` names a clip of the same sequence this one moves with: the parent's position,
  scale, and rotation apply on top of the clip's own, about the parent's pivot, at each moment,
  and parents chain. `gen=null@1` draws nothing, to rig several clips under one
  (`set c2 parent=n1`, and `parent=""` to let go). Deleting a parent lets its children go; a
  child of a clip that is split stays with its first part.
- Printing is deterministic and omits defaults. Printing with annotations
  appends comments with each clip's end and source out; parsing ignores them.

### Times

- **Positions** (`at=`, `start=`, markers) are timecode on the sequence,
  `HH:MM:SS:FF`, or `HH:MM:SS;FF` for drop-frame 29.97 and 59.94.
- **Source positions** (`src=`, `out=`) are timecode on the media, or a
  quoted phrase that a transcript of the media made earlier heard, as in
  `src="so we closed"`: an in point lands on the frame its first word starts,
  an out point past the frame its last word ends. Case, punctuation, and
  spacing do not count; a phrase said more than once is refused with the
  times it was said.
- **Durations** (`dur=`) count frames in the same `HH:MM:SS:FF` shape.
- Anywhere a time is read, `96f` and `4.5s` also work as offsets from the
  start. Seconds round to the nearest frame.
- A position in a script may also be an **anchor**: `end` (the end of the
  sequence), `c3.start` or `c3.end`, a marker's id (`beat1`, or `beat1.end`
  where a marker with a duration ends), or `c3@<media timecode>`, where
  clip `c3` plays that position of its media (or nested sequence), on the
  nearest frame, with speed and reverse taken into account. Each takes an
  optional amount, as in `at=end`, `to=c3.start-6f`, `at=beat1+2f`, or
  `split c3 at=c3@14:24:05:02`. Anchors read the project as the line runs,
  so they follow earlier lines' edits; they are written into history as the
  timecode they found. `end` always means the end of the sequence, and a
  marker id that holds `-` is read as the id, not as an amount.
- Keyframe times are relative to the clip: `{0f:0 12f:1:ease}`. What follows a value is
  how it moves toward the next keyframe: `linear` (the default), `hold`, `ease-in`,
  `ease-out`, `ease`, or a curve's two control points, `x1,y1,x2,y2`, as a graph editor
  gives them (`{0f:0:0.2,0.8,0.4,1 12f:1}`). `x` stays within 0 to 1, and `y` may overshoot
  to 5 either way, for a bounce. A clip split through a curve keeps its motion on both
  sides, as two curves.

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
- `snap=pause|cut [within=<amount>]` on `trim`, `split`, `insert`, and
  `overwrite` moves the edges the line gives (`insert` and `overwrite`:
  `src=` and `out=`) into the nearest pause of the media's sound, clear of
  the sound either side, or onto a cut of its picture, as the index made on
  import found them; within half a second unless given. A split stays inside
  its clip and a trimmed clip keeps a length. Without an index, or with
  nothing in reach, the line is refused.
- `trim … src=<source position>` puts the edge where the clip plays that
  position of its media, so a cut can land on a word's time from a
  transcript.
- `media replace m1 path="media/take2.mov" rate=24 dur=00:04:30:00
  video=3840x2160 audio=2ch@48000` swaps the file behind a media id and
  takes the same fields as `media add` (`tc=`, `alpha=`, and `reel=` too).
  Every clip that uses it keeps its timing; a file one no longer fits (too
  short for its source range or a transition's handles, or without the
  picture or sound its track needs) is refused, naming the clips.
  `media relink` changes only the path, for the same file moved.
- With several sequences, `use s2` selects the one later lines act on.
- `captions c3 on=S1 [words=<n>]` captions what clip `c3` says, timed to its
  speech, from transcripts of its media made earlier; without a transcript
  covering the clip the line is refused. It runs as the caption lines it
  becomes.
- Quoted phrases, `snap=`, and `captions` read analysis. Whoever runs the
  script says where it comes from (`kerfd` reads its media cache), and a
  line that needs what is not there is refused.

Every operation prints back as script (`print_op`, `print_ops`), so history,
UI actions, and agent edits share one readable log.

## In `kerfd`

`kerfd` keeps the project as `project.kerf`, written after every change. Edits
arrive as scripts (`apply_script`) or typed operations from the editor
(`apply_ops`, frame counts on the sequence). Each change is journaled as
the script it ran, and undo restores the project as it was. Agent edits stay pending proposals that show the
operations they run, the script as it was written (`source`, with its
anchors and `captions` lines), and a diff of the document; `get_document` returns the document,
optionally annotated. Render jobs and proposal previews hand renderers the
document with media paths resolved against the project.

Media paths are relative to the project folder. `import` and
`replace_media` read a relative path against it, and keep a file inside it
as a project-relative path with `/` separators (`media/a-1f2e3d4c.mov`), so
a shared project reads the same on every machine; a file outside it is kept
where it is. A Git LFS pointer, which stands for media not yet downloaded,
is refused. `replace_media {media, path}` probes the new file and refuses one
it cannot read or that has no length, rather than guessing; it runs as the
`media replace` line it becomes, so it is undone like any edit.
