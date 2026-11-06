# One editor, separate areas

Kerf is one editor that an agent can drive from cutting through grading,
motion, and effects: the job Final Cut Pro and Motion, or Premiere and After
Effects, split between two apps. Resolve (Cut, Edit, Fusion, Color, Fairlight,
Deliver, and since 21 Photo) showed that one project can hold all of it. An
agent makes the split less useful still, since it does not care which app a
tool is in.

So the product is one, and the code is not. Each area can be rewritten without
touching the others, and this document says where the lines are and what is
still to come.

## Where things live

`crates/<folder>/<crate>`. The folder is the rule: a crate may depend on its
own folder and on lower layers, and `tools/kerf-layers` fails the build's tests
otherwise. Nothing else has to be kept in step with it.

| Layer | Folder | What | Crates |
|---|---|---|---|
| 0 | `foundation` | The document, its language, and arithmetic everything shares | `kerf-model`, `kerf-lang`, `kerf-geom`, `kerf-color`, `kerf-text` |
| 1 | `render` | Pictures and how they are drawn on | `kerf-raster`, `kerf-draw`, `kerf-gpu` |
| 2 | `edit` | What a timeline does beyond its model: interchange, sharing | `kerf-interchange`, `kerf-collab` |
| 2 | `effects` | The effects carried from FilmCraft and EffectCraft, all areas together | `kerf-fx`, `kerf-raster-fx` |
| 2 | `image` | The little of a photo editor video needs | `kerf-image` |
| 2 | `color` | Measuring a grade; the grading effects themselves are in `effects` for now | `kerf-scopes` |
| 2 | `vfx` | Following what moves in a picture and cutting it out; keys and mattes are in `effects` for now | `kerf-track`, `kerf-segment` |
| 2 | `motion` | Reserved: it gets its folder with its first crate | |
| 3 | `engine` | Sequences into frames and files | `kerf-core`, `kerf-compose`, `kerf-playback`, `kerf-engine` |
| 4 | `surfaces` | What people and agents touch | `kerf-runtime`, `kerf-agent`, `kerf-mcp`, `kerf-skills`, `kerf-commands`, `kerf-cli`, `kerf-qt-bridge`, `kerf-cloud`, `kerf-fetcher` |

The areas in layer 2 may not depend on each other. What two of them need moves
down to `render` or `foundation`. Only `engine` and `surfaces` see them all.

`kerf-model` holds the timeline's operations and stays in `foundation`, since
every layer reads the document. `kerf-fx` is sorted by where its code came from,
not by what it does; it drains into `color`, `vfx`, and `motion` as those areas
are worked on, and nothing has to move before then. Every definition already
says which area it belongs to (`FxDef::area`), so people and agents see the areas
now: `look` takes `effects="glow" area="vfx"`.

### Adding an effect

1. Implement `kerf_draw::effect::PictureEffect` in the area's crate and list it
   in the crate's `EFFECTS`.
2. Describe it in the area's table in `kerf-model` (`image_effects.tsv`).
3. For a new area crate, add its `EFFECTS` to `kerf-compose/src/domains.rs`.

The compositor, the parser, the language reference, and `look` need no change. A
test fails if the catalog and the code disagree. `image.fill` is the worked
example.

## The engine

A frame is drawn on the GPU when every part of it has a kernel, and on the CPU
when any part does not (`kerf-compose/src/draw/gpu.rs`). The decision is for the
whole frame and is made before anything is decoded. Footage goes up as the 8 bytes
a pixel it decoded to, everything stays on the GPU, and only the finished bytes come
back: moving a 4K float picture across for each effect would cost what the effect
saves. `kerf_draw` stays the reference. Kernels are tested against it per value
(`kerf-gpu`, `kerf-fx/src/filmcraft/gpu_tests.rs`) and per frame on real footage
(`kerf-compose/tests/gpu.rs`). The kernels build on a background thread, and the
CPU draws until they are ready. `KERF_GPU=0` turns all of this off.

Measured on an Apple Silicon Mac with `examples/frame_bench.rs` in
`kerf-compose`, on 4K H.264 test footage, decode included, median frame:

| Scene | 1080p CPU | 1080p GPU | 4K CPU | 4K GPU |
|---|---|---|---|---|
| Placed, nothing else | 59 fps | 87 fps | 18 fps | 25 fps |
| One Lumetri | 29 fps | 78 fps | 9 fps | 24 fps |
| Gaussian blur, radius 40 | 33 fps | 42 fps | 8 fps | 15 fps |
| Four layers | 31 fps | 53 fps | 9 fps | 15 fps |
| One glow | 13 fps | 15 fps | 3 fps | 5 fps |
| Ultra Key | 25 fps | 80 fps | 8 fps | 32 fps |
| Radial Zoom Blur, amount 30 | 5 fps | 33 fps | 0.7 fps | 6 fps |

A 96-frame 1080p render with Lumetri, a masked and blended layer, and a title took
3.3 s against 7.4 s, 46 dB from the CPU's after encoding.

**What the GPU draws:** media, nested sequences, solids, titles, captions; masks,
crops, letterboxing, placing, opacity, wipes and dissolves, dips, all 27 blend modes,
adjustment layers; `color.basic`, `blur.gaussian`, the 29 kinds of operation in
`FxOp` with Lumetri among them, and Wonder Glow, Glint, RGB Split, Wave Warp, and
Noise, Color, Luma, and Ultra Keys, Logo Cutout, Extract, and Radial Zoom Blur. So 13 of the 15 bundled
looks stay on the GPU, including the transitions that are looks (Whip, Flash Cut), since a look
transition is layers rather than a mix.
**What sends a frame to the CPU:** the transitions carried from FilmCraft, every other
effect (other lights, stylize, Ultra Key's Choke and Soften, and the kept EffectCraft ones, which is
what Film and Zoom Through need), and a Lumetri using a LUT or curves. In HDR, so does an effect that
needs the whole picture brought under 1, which goes by a peak only the CPU reads; effects that bring
each pixel under 1 (`catalog::headroom`) stay on the GPU. Glow gains least
(13 to 15 fps at 1080p, 3 to 5 at 4K): its 18 blur passes are the work either way.

Three limits remain, in this order:

1. **More of the frame on the GPU.** Each effect without a kernel costs its whole
   frame the GPU. The 72 carried transitions are procedural maths with no kernels (FilmCraft
   has one, Push, in `crates/gpu/src/transition.wgsl`), and they last only as long as a
   cut. EffectCraft's WGSL (`effectcraft/crates/gpu` at `5c54866`) has kernels for many of them.
2. **Kept frames.** Playback, previews, and `look` keep frames by a hash of what they depend on
   (`Drawing::frame_key`), in memory (`frames.rs`) and on disk (`disk.rs`, after EffectCraft's
   `render/src/disk_cache.rs`), so a frame comes back from any compositor, an edit, which makes a new
   one, only redraws the frames it touches, and a clip moved along the timeline keeps its frames.
   Renders use the disk too, so rendering again after a small edit draws only what it changed, and
   frames last between sessions. A frame reading another track, a sound, or a file is never kept.
   Files are LZ4, whole or not at all, and go by the code that draws them (`build.rs`), so a new Kerf
   never reads an older one's pixels. Up to 10 GB (`KERF_FRAME_CACHE_GB`) in the user's cache folder
   (`KERF_FRAME_CACHE`, or `off`). A title's fonts are not in the key, as in memory.
   A 96-frame 720p render with Lumetri and a glow took 4.53 s, 0.63 s the second time, and 1.70 s
   after an edit to half of it.
3. **Depth and color.** Footage decodes at its depth, with its transfer and gamut, into the float
   working picture (`bits=` and `color=` on a media line, found on import): `InputTransform`
   on the CPU, `input.wgsl` on the GPU. A ProRes master takes 16-bit frames, so it keeps that
   depth. A sequence with `color=rec2100-pq` or `rec2100-hlg` keeps BT.709 working primaries,
   with what lies past them as values below 0 and above 1, and `HdrOutput` (`output.wgsl`) turns
   them into the master's signal or tone maps them for monitors. Lumetri grades in the sequence's
   own signal. Other effects written for 0 to 1 get the picture brought under 1 and put back,
   each pixel or the whole picture as `catalog::headroom` says. They still lose wide colors, and three looks still clip. ProRes and HEVC Main 10 carry HDR; H.264 is
   8-bit, and scopes read the 8-bit SDR frame. An HDR10 file (`hevc_hdr`) says what it was mastered on
   (P3 D65 at 1000 nits unless told otherwise) and its MaxCLL and MaxFALL, measured from the frames
   before encoding, which disk-kept frames make cheap; HLG carries its tags without them. On Apple, handing decoded surfaces to the
   GPU without a copy needs `unsafe`, which the workspace forbids, so that gets one small
   crate with the exception written down. Scaling footage is another cost: 4K footage in a
   1080p sequence spends more converting (12 ms) than 4K shown whole (5 ms).

## The areas

FilmCraft upstream, at the commit Kerf carried (`0f99661`), still holds work worth
reading before writing it again: `crates/scopes`, `crates/audio-dsp`, and in
`crates/render` the optical flow, multicam, color management, and sound mixing.

Code that left is still in history. `5c54866` has all of EffectCraft's crates as
they were before they were taken out; `91ae7f7` has inpainting, scopes, optical
flow, cut detection, ICC, and color spaces; `173914c` has the camera solver and
face tracking; `73a43cc` has the paint, expression, audio, and 3D-channel effects.
Carrying back means rewriting into Kerf's terms and wiring through the catalog:
code nothing calls was cut once already and should not come back as such.

### Edit

Has: ripple, roll, slip, slide, transitions, nested sequences, captions from
transcripts, snapping to pauses and cuts, roles, proxies, interchange, QC.

Slow motion can make frames: `retime=blend` or `retime=flow`, on the CPU. Speed changes over a clip
(`kerf_model::Speed`, `speed={0f:1 24f:1/4}`): the model knows one curve, source played by clip time,
and everything from trims to handles reads it. FFmpeg's `atempo` cannot follow a ramp (timed tempo
commands drifted by up to 0.39 s when measured), so `kerf_compose::ramps` makes the sound with
waveform-similarity overlap-add into a WAV in the temp folder, in sync within 10 ms by construction
(80 s of stereo in 142 ms in a release build),
and the graph and the live mixer play that as plain media. Native playback hands a ramp's picture to
the compositor. To add, in order: multicam; sound beyond gain and fades, with effects and buses. A
timeline's own tools stay in `kerf-model` and `kerf-lang`.

### Color

Has: Lumetri, curves, levels, Hue/Saturation, Selective Color, CDL, LUTs, Auto
corrections.

`crates/color/kerf-scopes` reads a frame as a colorist reads scopes, and `look` gives the
agent the numbers (`scopes: true`), so that it can grade by measuring: levels, clipping,
tonal zones, a histogram, the cast, saturation, the skin tone line, and the densest colors.
Comparing `proposal` with the project shows what a grade did. FilmCraft's `crates/scopes`
also has the waveform, parade, and vectorscope grids a panel in the editor would draw.

To add: the Lumetri presets (cut in `6eafef8`), and
layered grades as stacked effects.

### Motion

Has: keyframes (hold, linear, easings, Bezier curves), transforms, titles, generators,
15 looks.

The rule: **a composition is a sequence.** Nesting already works, so there is no
second document type, and an agent learns one model. To add, in order:

1. A graph editor in the panel, for the Bezier keyframes (`kerf_model::Bezier`) and motion paths
   (`path=`, `along=`, `orient=`) that are in the language.
2. 3D layers, cameras, and lights (EffectCraft's `render/three_d`), on the parenting that is
   in (`parent=`, `gen=null@1`).
3. Shape clips, from EffectCraft's `path` (booleans, offsets, trim, variable
   strokes).
4. Text animators and variable fonts.
5. Particles (EffectCraft's `fx_particles`).

Expressions stay out. They exist so that a person need not write hundreds of
keyframes, and an agent writes them gladly; a look covers the reusable case.

### VFX

Has: keys, track mattes, blurs, glows, distortions, warp stabilizer, restoration,
360°.

`crates/vfx/kerf-track` follows a point of the picture (pyramidal Lucas-Kanade with a
forward-backward check), and `look` gives the result as keyframes (`track`), so that what
follows is plain text in the document, and a null and `parent=` do the rest. Given four corners it
follows a flat surface: a homography from the first frame, fitted by RANSAC to the corners of detail
inside it, so what passes in front is left out and an error in one frame does not add up. `look`
gives it as a `distort.cornerpin@2` line and as keyframes for a moving mask. On a surface sliding and
tilting for 48 frames at 320x180, a corner is at worst 0.57 pixels off, 1.75 with a bar crossing it.

Mask outlines animate (`MaskShape::Moving`), and `image.fill method=video` with one removes what
moves: each frame's hole comes from the outline at that moment.

Given points on an object, `look` cuts it out and follows it (`kerf_track::roto`, a graph cut carried
forward by optical flow, in the same `track`), and what it finds is a moving mask of 40 corners, not a
matte kept beside the document. A trained model is optional and swappable (`kerf_segment::MaskModel`):
it only sharpens the cut's prior, and one that disagrees with the flow is left out for that frame.
MobileSAM runs in plain Rust; its weights are never bundled, and `look` names the file to fetch. On a
disk over a still textured background for 24 frames at 200x150, the outline overlaps the truth by at
least 0.994; camouflaged so colour says nothing, 0.005 alone and 0.966 with a model standing in.

To add: soft edges, which a polygon cannot hold (`feather=` approximates them); the camera solver. Optical flow is in
`kerf-raster` now, which `image.fill` uses on a moving shot and `retime=flow` uses for slow motion.

### Image

Kept small on purpose. Video editors have always carried a little of Photoshop,
and that little is what belongs here. A still is a one-frame clip and a selection
is a mask, so this area adds no document type.

Has: `image.fill`. To add: clone and heal as strokes in the document, then a mesh
warp for liquify work.

Stays out: a brush engine, pixel layers as their own documents, RAW development,
and print color. When a job needs those, it is a job for a photo editor.

## The agent

The agent still gets four tools and a document it can read. Each area adds
vocabulary to the language and numbers to `look`, and no new tool: scopes as
measurements, tracks as data, and check findings for what a viewer would notice.

## What others do, and what to take

- **Resolve 21** added a Photo page, and Fusion effects in the Edit page's
  keyframe and curve editors: where one model pays off.
- **Final Cut Pro and Motion** keep two apps but share Magnetic Mask, and added
  Auto Mask. On-device matting is now expected, not special.
- **After Effects 26** has parametric meshes, depth of field in its 3D renderer,
  variable-font animation, and a faster mask tracker with Object Matte.
- **Jumper and the Resolve and Premiere MCP servers** bolt agents onto editors
  that were not built for them. Kerf's document, proposals, and Git merging are
  what they cannot have, and each area has to keep that: every feature is text in
  `project.kerf`.
- **[raea](https://github.com/shinyobjectz/raea)** (reviewed at `925bc03`) renders
  React and TypeScript compositions with wgpu and Bevy. It is a composition engine,
  with no timeline document or proposals, so it is not a base for Kerf. **Its
  license, FSL-1.1-ALv2, forbids a competing commercial product, and Kerf Cloud is a
  paid tier, so none of its code comes here;** each version turns Apache-2.0 two
  years after release. These ideas are worth doing our own way: one kernel source
  run on the CPU and compiled to WGSL, which would end writing every effect twice;
  decoded IOSurfaces as wgpu textures, to skip the copy; a fact sheet for footage (faces, the
  subject, on-screen text, camera motion) beside what `kerfd` indexes now; and
  checks for clipped text, contrast, and safe areas.
