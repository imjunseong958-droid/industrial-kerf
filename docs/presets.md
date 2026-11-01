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
from it may be included. The project name is the preset name.

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

- Every media declaration is a slot: its path must be exactly `slot:<id>`.
  No original media paths, source timecode origins, reels, or retired IDs
  are exported. Actual video/audio files are not included.
- A cut gets its own slot, even when several cuts use the same original
  file. Linked picture and sound from that file share a slot, preserving
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
`"from": "<n>f"`). Application uses the same
proposal diff, checks, and before/after preview as other edits.
