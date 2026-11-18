# Native editing presets

A Kerf preset is a single `.kerfpreset` text file containing **media slots and
Kerf editing code**. It is exported directly from the accepted native timeline.
The importer accepts this native contract; it does not infer an edit from a
rendered video or translate another editor's template format.

## In the editor

1. Edit the sequence normally, then choose **File → Export Editing Preset…**.
2. Share the resulting `.kerfpreset` file, or keep it in the project's
   preset library, `presets/`, which a shared project shares with every
   sync (see below).
3. In another project, import the replacement footage into the Project bin.
4. Choose **File → Import Editing Preset…** and open the shared file.
5. Assign media to each slot, choose its source start, and optionally replace
   title/caption text. Each slot shows the track and time of the clips that
   use it, and lists only imported media with the streams and length it
   needs; the start snaps to a whole frame of that media. The dialog also
   shows the preset's editing code.
6. Choose **Create proposal**, review the timeline, and accept the proposal.

A media Kerf or a model makes, with the steps under its line
([shots.md](shots.md#steps)), travels with its steps and is
made again where the preset is applied; it takes no binding. See
[Media a preset makes](#media-a-preset-makes).

Export does not change the project. Applying replaces the program sequence;
the existing project stays intact until acceptance, apart from the looks the
preset brings (see below), and acceptance can be undone. Locked destination tracks must be unlocked first. A sequence that is
itself used by another sequence cannot be replaced this way.

Kerf timing, gaps, tracks, transitions, constant speeds (including reverse),
transforms, keyframes, effect stacks, generators, captions, markers, linked
clips, and nested sequences come from the model. There is no model request
or editorial guess in export or application.

## The project's preset library

A project's `presets/` folder is its preset library. In a
[shared project](collaboration.md) it is part of the repository, so every
collaborator has the same presets after a sync. Kerf lists presets from
`presets/` first, then `.kerf/presets`, then the project folder.

- **Window → Presets** shows the library: each preset's name and slots and,
  in a repository, who changed it last and when. **Apply** opens the binding
  dialog described above.
- **Save Current as Preset…** exports the program sequence to
  `presets/<name>.kerfpreset`.
- **From Preset** in the project launcher (New Project from Preset) makes a
  new project from a preset: pick the preset, choose a file and a start for
  each slot, and Kerf copies the preset into the new project's `presets/`
  and the files into its `media/`, imports them, and applies the preset as
  the project's first edit.

Applying binds each slot to a media id of the project. In a shared project
a media id names one file in the repository, so a slot plays the same file
for every collaborator, and **Replace Media…** changes that file for
everyone. See [shared presets and media slots](collaboration.md#shared-presets-and-media-slots).

## File contract, version 1

The file uses the [Kerf language](kerf-language.md), starting with `kerf 1`.
Its first sequence is the pattern's entry point. Only sequences reachable
from it, or drawn by the steps of a media the preset makes, may be included.
The project name is the preset name.

```text
kerf 1
project name="Two-shot product"
media slot1 path="slot:slot1" rate=30 dur=00:00:02:00 video=1920x1080
media slot2 path="slot:slot2" rate=30 dur=00:00:03:00 video=1920x1080
sequence s1 name="Two-shot product" rate=30 size=1080x1920
  track V1 video
    clip c1 media=slot1 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00
      set scale={0f:1 30f:1.1:ease}
    clip c2 media=slot2 src=00:00:00:00 at=00:00:02:00 dur=00:00:03:00
  track V2 video
    clip title1 gen=title@1 at=00:00:00:00 dur=00:00:02:00
      set text="Your product"
```

- Every media declaration is a slot, its path exactly `slot:<id>`, or a
  media the preset makes, `made:<id>` and its extension, with its steps
  under it ([Media a preset makes](#media-a-preset-makes)).
  No original media paths, source timecode origins, reels, or retired IDs
  are exported. Actual video/audio files are not included.
- A cut gets its own slot, even when several cuts use the same original
  file, unless the steps of a media the preset makes read that file: then
  it is one slot for all of them. Linked picture and sound from that file share a slot, preserving
  their source offsets and synchronization.
- `video=` and `audio=` specify the required streams; `dur=` specifies the
  minimum source length **including transition handles**. Replacement
  resolution and frame rate may differ. The preset keeps its own canvas,
  timeline rate, transforms, and cropping; it does not choose a new framing.
- Source positions are offsets within the slot's selected material.
  Export subtracts the earliest required source position, retaining exact
  relative positions and room before/after a transition. A binding's
  `from` is where that material begins in the replacement file.
- Caption clips and `title` generators expose text overrides by their
  template clip ID. Their timing and styling stay fixed. Speech is not
  retranscribed when media changes.
- Music and other audio are slots too; each must be assigned explicitly.
  Fonts referenced by the edit must be available on the receiving machine.
- Unknown syntax/features, external media paths, missing/unknown bindings,
  insufficient source material, incompatible streams, invalid nesting,
  and unknown text overrides are rejected. Files are limited to 1 MiB.
- Instantiation allocates fresh project IDs, including link/transition IDs,
  and emits an ordinary atomic edit script for `kerfd`. Other sequences and
  existing media remain in the project. A repeated application may allocate
  different IDs; for the same preset, bindings and target state the emitted
  edit is deterministic.
- So a clip the preset calls `sa` is not `sa` in the project: it may be `p9`.
  `texts` and `bindings` are given by the preset's own IDs, since that is
  what inspecting the preset shows; everything after is by the project's.
  Applying a preset through `edit` answers with `ids`, each of the preset's
  IDs with the one it became (`{"sa": "p9", "t1": "p12"}`), so the clips,
  markers and transitions it placed can be named in the edits that follow.
  A slot is listed with the media it was bound to, and the preset's first
  sequence with the target sequence, which keeps its own ID.

## Media a preset makes

A media of the pattern that has steps under its line is one the preset
makes, not a slot. Its path is `made:<id>` and its file's extension, and its
steps come with it. Two patterns, as a preset writes them:

```text
kerf 1
project name="Moved"
media slot1 path="slot:slot1" rate=16 dur=81f video=832x480
media man path="made:man.mp4" rate=16 dur=81f video=832x480
  generate model=matte prompt="man" guide=slot:slot1
media gone path="made:gone.mp4" rate=16 dur=81f video=832x480
  generate model=matte prompt="man" guide=slot:slot1 out={man.mp4}
  generate model=plate prompt="an empty dirt path" guide=slot:slot1 guide_mask={man.mp4} seed=7 reach=72
sequence s2 name="Moved" rate=16 size=832x480
  track V1 video
    clip plate media=gone src=0f at=0f dur=81f
  track V2 video
    clip mover media=slot1 src=0f at=0f dur=81f
      set x=0.25 scale=1.5
      fx key.trackmatte@1 matte=V3 composite=matte-luma
  track V3 video enabled=false
    clip moves media=man src=0f at=0f dur=81f
      set x=0.25 scale=1.5
```

```text
kerf 1
project name="Car"
media depth path="made:depth.mp4" rate=16 dur=65f video=832x480
  render s2 {guide.mp4} pass=depth size=832x480 rate=16
  generate model=shot prompt="a red sports car on a wet street at dusk" guide={guide.mp4} moves=body seconds=4 seed=7
sequence s1 name="Car" rate=16 size=832x480
  track V1 video
    clip shot media=depth src=0f at=0f dur=65f
sequence s2 rate=24 size=1664x960
  track V1 video
    clip road gen=solid@1 at=0f dur=96f
      set color=#404448 three_d=true rotate_x=-84 y=0.75 z=4 scale=16
  track V2 video
    clip body gen=shape@1 at=0f dur=96f
      set kind=rect width=0.9 height=0.22 fill=#c02020 three_d=true depth=0.4 x={0f:-0.45 96f:0.45} y=0.84 z=2
```

- **Export.** A clip that plays a media with steps makes that media one the
  preset makes, once however many clips play it. A media with steps that a
  step reads by its path travels the same way, and is named
  `made:<id>.<ext>` in the steps that read it.
- **A step reads footage by its slot.** A file a step names that is the
  path of footage of the project (a media with no steps) becomes that
  footage's slot, `guide=slot:slot1`. Footage a making reads is **one slot
  for every cut that plays it**, its source range the union of theirs and of
  the clips that play the media made of it: the making is of that one file.
  Footage a step reads and no clip plays is a slot as long as the file. A
  file of the project that is not footage, a reference picture or a copy of
  a workflow, is kept as it is written; the making where the preset is
  applied says when it is not there.
- **The sequences its steps draw travel with it.** A sequence a `render`
  step draws, and one that holds a clip a step names (`matte=`, `moves=`,
  `signal=`), comes with the preset with what it nests, though the first
  sequence does not nest it. A sequence reached neither from the first nor
  from a step is still refused.
- **A recipe file does not travel.** A clip that plays a media whose steps
  are still in a recipe file (`recipe=`) is refused at export: carry them
  into the document first (`kerf carry <media>`,
  [shots.md](shots.md#carrying-a-recipe-into-the-document)).
- **Applying adds each as a new media** of the project, by an ID of the
  project's own and a file in `shots/` with its extension, `shots/p12.mp4`,
  so its takes are kept in `shots/p12.takes/`. The edit adds the media,
  then the sequences, then each step with a `step <id> add …` line, all in
  the one proposal. In the steps, a sequence and a clip are named by the IDs
  they were given, a slot by the path of the media bound to it
  (`guide=footage/run.mp4`), and a made media by its new path. A `{name}`,
  `cast=`, `style=`, the model and every other word stay as they are. The
  answer's `ids` lists the made media, and the sequences and clips that
  came with them, with the rest, so `step p12 set 2 seed=9` can follow.
- **A matte is over its own frames.** A made media whose steps read one
  slot's file and draw no sequence, a matte or a plate, is of that footage
  frame for frame: it takes the bound media's `rate`, `dur` and `video` on
  its line, and its clips start from the binding's `from` as the slot's
  clips do. A made media drawn from a sequence keeps the pattern's line.
- **Nothing is made by applying.** The media are made with `make` (or
  `kerf make`), as any made media is, once the proposal is accepted.
- A made media no clip plays and no step reads is refused, as an unused slot
  is; so are steps out of order, steps under a slot, a `made:` path that is
  not its own ID, and steps that name a slot, a made media, a sequence or a
  clip the preset has not.

Inspecting a preset lists the made media apart from the slots (`made`):
each with its steps as the document writes them, the slot it is of frame for
frame (`of`), and the clips that play it. A slot lists the made media whose
steps read it (`readBy`). No binding is asked for a made media; one given is
refused.

## Looks and beats

A preset carries the [looks](looks.md) its pattern uses that Kerf does not
bundle, so it draws the same where they are not installed. Each follows the
sequences as a `look` line at the top level, with the rest of its
`.kerflook` file indented under it:

```text
kerf 1
project name="Punch"
media slot1 path="slot:slot1" rate=30 dur=00:00:02:00 video=1920x1080
sequence s1 name="Punch" rate=30 size=1080x1920
  track V1 video
    clip c1 media=slot1 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00
      fx look.studio.punch@1 hit=beat1
  marker beat1 at=00:00:01:00

look look.studio.punch version=1 name="Punch" category="Motion" on=clip
  doc "A zoom punch at a hit."
  param hit time default=0f
  set scale={$hit:1 $hit+3f:1.15 $hit+9f:1:ease}
```

- A carried look must be one the pattern uses. A look that Kerf or the
  receiving project already has by that id and version must be the same
  look, or the preset is rejected.
- Applying copies the looks the project lacks into
  `.kerf/looks/<id>@<version>.kerflook`, so the edit keeps drawing after the
  preset file is gone. That copy is the one change made before the proposal
  is accepted.

A look's time can name a marker (`hit=beat1` above), so the look lands
wherever the marker is. A binding marked as music (`beats`) moves every
marker over a clip that plays it to the nearest sound onset of the new
media, from its media index:

- Markers land on whole frames, each after the one before it.
- A marker with no onset left in its clip stays where it is, as do markers
  over other clips.
- Cuts do not move.

So the punches and flashes of a pattern hit the new music's beats.

Other tools can create presets by producing this contract and passing Kerf's
normal language/model validation. Presets contain declarative editing data,
not executable JavaScript, HTML, shell commands, or Python.

## CLI and assistant access

The native menu is the primary workflow. The same implementation is available
without a provider API key through the CLI:

```bash
kerf --dir ./project preset save product.kerfpreset --name "Product"
kerf --dir ./project preset list
kerf --dir ./project preset show product.kerfpreset
kerf --dir ./project preset apply product.kerfpreset \
  --bind slot1=m1@0s --bind slot2=m2@1s --text 'title1=New product'
kerf --dir ./project preset apply product.kerfpreset \
  --bind slot1=m1 --bind slot2=song --beats slot2
```

Saving to `presets/product.kerfpreset` puts the preset in the project's
library. Bindings use already-imported media IDs from the target project. Optional
`--sequence` selects the saved/target sequence. `apply` stages a proposal;
accept it in the native editor. CLI/assistant save writes a new file inside
the project and never overwrites an existing file. The native export dialog
can save to a chosen location using the normal save-file overwrite prompt.

The assistant and external MCP clients keep the three-tool interface:

```json
{"tool":"look","arguments":{"presets":true}}
{"tool":"look","arguments":{"preset":"product.kerfpreset"}}
{"tool":"edit","arguments":{"savePreset":"product.kerfpreset","name":"Product"}}
{"tool":"edit","arguments":{"preset":"product.kerfpreset","bindings":{"slot1":{"media":"m1","from":"0s"},"slot2":{"media":"m2","from":"1s"}},"texts":{"title1":"New product"}}}
{"tool":"edit","arguments":{"preset":"product.kerfpreset","bindings":{"slot1":{"media":"m1"},"slot2":{"media":"song","beats":true}}}}
```

`beats` is available from the CLI and the assistant; the editor's dialog
does not offer it yet.

Listing covers `presets/`, then `.kerf/presets` and the project root. Inspect/apply can read a
shared `.kerfpreset` at an explicitly supplied path. Inspecting lists the
looks the preset carries (`looks`) and, for each
slot, the clips that use it and the project's imported media that can fill it
(`fits`), each with the latest start frame (`latestFrame`, usable as
`"from": "<n>f"`), and the media the preset makes (`made`), which take no
binding. Application uses the same
proposal diff, checks, and before/after preview as other edits, and adds
`ids` (see above).
