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
| 2 | `motion` | Motion graphics drawn from numbers | `kerf-motion` |
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

**What the GPU draws:** media, nested sequences, solids, titles, captions; clips in depth with their
lights, shadows, and solids; masks,
crops, letterboxing, placing, opacity, wipes and dissolves, dips, all 27 blend modes,
adjustment layers; `color.basic`, `blur.gaussian`, the 29 kinds of operation in
`FxOp` with Lumetri among them, and Wonder Glow, Glint, RGB Split, Wave Warp, and
Noise, Color, Luma, and Ultra Keys, Logo Cutout, Extract, and Radial Zoom Blur. So 13 of the 15 playful looks and all
18 grades stay on the GPU, including the transitions that are looks (Whip, Flash Cut), since a look
transition is layers rather than a mix.
**What sends a frame to the CPU:** a camera with an `aperture`, the transitions carried from FilmCraft, every other
effect (other lights, stylize, Ultra Key's Choke and Soften, and the kept EffectCraft ones, which is
what Film and Zoom Through need), and a Lumetri using a LUT, curves, sharpening, or an HSL secondary, which none of the 18 grades does. In HDR, so does an effect that
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
   own signal. Other effects written for 0 to 1 get the picture brought between 0 and 1 and put back,
   each pixel or the whole picture as `catalog::headroom` says: a color past BT.709 goes toward the gray of its own brightness until it fits and comes back around
   the brightness it has after the effect, on the CPU and the GPU, and the whole picture is lifted as well as scaled, which a filter over its pixels undoes exactly. `time.echo` adds up what it is given as it is, below 0 and above 1, since adding and weighing need nothing brought under 1; only its Screen operator stops at SDR white. Light from `light.echoglow` and `light.glint` adds to a color past BT.709 and leaves how far it stands past the rest, white light by the same in each channel. What changes a pixel's channels apart (`stylize.posterize`, `color.autocolor`, an exposure that saturates) keeps the gamut but moves the hue of what lies far past it. ProRes and HEVC Main 10 carry HDR; H.264 is
   8-bit. `look` with `scopes` reads an HDR sequence by its PQ or HLG signal, with what that comes to in cd/m² and how much of it BT.709 cannot hold; so does the Finish tray, whose bridge draws that signal small again on a thread of its own while the scopes are in sight, since playback hands over only the 8-bit frame a monitor gets, and marks where SDR white falls. An HDR10 file (`hevc_hdr`) says what it was mastered on
   (P3 D65 at 1000 nits unless told otherwise) and its MaxCLL and MaxFALL, measured from the frames
   before encoding, which disk-kept frames make cheap; HLG carries its tags without them. Holding an effect costs GPU passes, measured with `examples/frame_bench.rs` on 4K ProRes in PQ on an M1 with 8 GB, ms a frame, decode included:

   | Scene | 1080p SDR | PQ | HLG | 4K SDR | PQ | HLG |
   |---|---|---|---|---|---|---|
   | Placed, nothing else | 17.7 | 18.3 | 18.3 | 55.6 | 51.3 | 51.9 |
   | `color.basic`, each pixel held | 19.2 | 24.1 | 24.4 | 34.8 | 51.9 | 51.3 |
   | `blur.unsharp`, Gamut held | 28.2 | 32.7 | 31.9 | 72.5 | 93.8 | 87.6 |
   | `light.wonderglow`, Gamut held | 62.2 | 72.1 | 65.7 | 207.9 | 230.5 | 221.8 |

   So an HDR frame costs nothing by being HDR, a held `color.basic` adds 5 ms at 1080p and 17 at 4K, and a Gamut effect 4 to 10 and 15 to 23. Handing VideoToolbox surfaces to the GPU without a copy was measured and left (`examples/upload_bench.rs`, an M1 with 8 GB). A 4K H.264 frame takes 4.5 ms to decode in software, 1.4 to convert, and 4.7 to upload, 10.6 in all, against 14.5 ms just to decode it on a surface; 1080p takes 3.0 against 5.3, and 4K HEVC 14.2 against 14.6. Only 10-bit ProRes gains, 24 to 27 against 14.8, and more than that gain was its 16-bit RGBA. A frame for the GPU at its full size now goes up in the planes it decoded to where that comes out right, and a kernel (`planes.wgsl`) makes it RGB. At 8 bits it goes by libswscale's own whole-number arithmetic and so to the same bytes, for 4:2:0 and 4:2:2 on 64-bit ARM at the sizes its fast path takes; deeper footage comes within 3 of 255. Other formats and sizes, alpha, other machines' 8-bit footage, and footage scaled on decode still go up as RGBA. With a grade on it, on an M1, 4K ProRes 10-bit goes from 46 ms a frame to 34, past 24 fps, 4K H.264 from 34 to 27, and 1080p from 9.1 to 7.4. So there is no crate with an exception to the workspace's `forbid`. Footage scaled on decode still converts on the CPU, and a GPU scale from its planes was built and measured and left out (`KERF_BENCH_SIZE` in `examples/upload_bench.rs`, 4K H.264 on an M1 with 8 GB, ms a frame, converting and upload and kernels and bytes back). To 1920x1080, 7.3 through libswscale (9.4 in the compositor, whose copy and threads add to its converting) against 12.4 from planes, in two passes of libswscale's bicubic with its own coefficients; to 1728x972, 7.5 against 11.2; to 3456x1944, a clip at 0.9 in a 4K sequence, 21 (24 in the compositor) against 19. So it loses in the 1080p sequence it was meant for, where such a frame takes 14 ms in all, 70 fps, and gains 2 to 5 of 39 ms only near full size. Nor is it the same picture: a mean of 1.66 of 255 from libswscale's, 1.7% of pixels off by more than 2, and 9 at worst, which `tests/gpu.rs` would refuse, so it could only be used while playing, under frame keys of its own. What it would save is CPU: libswscale spends 19 ms of core time a frame across its threads to take 3 to 5.

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
the compositor. A multicam is a sequence with a camera on each track, and a clip of it shows one
(`angle=V2`), since a composition is a sequence and this needs no second document type; `look` with
`sync` lines recordings up by the rise of their loudness (`kerf_engine::sync`). Sound goes through
clip effects and then buses (`kerf_model::Bus`), mixed by the render's graph and, at the top of a
program, by the live mixer's own stream for each bus, so a compressor hears the same sum in both. A
timeline's own tools stay in `kerf-model` and `kerf-lang`.

### Color

Has: Lumetri, curves, levels, Hue/Saturation, Selective Color, CDL, LUTs, Auto
corrections.

`crates/color/kerf-scopes` reads a frame as a colorist reads scopes, and `look` gives the
agent the numbers (`scopes: true`), so that it can grade by measuring: levels, clipping,
tonal zones, a histogram, the cast, saturation, the skin tone line, and the densest colors. In an HDR
sequence they are of its signal, with `hdr` giving peak and mean cd/m², the share over SDR white, and the share past BT.709.
Comparing `proposal` with the project shows what a grade did. A person reads the same frame in the
Finish tray: a waveform, a vectorscope, and an RGB parade (`apps/kerf-qt/ui/scopes.h`), in an HDR sequence of its signal.

The 18 grades (`look.grade.*`, `crates/foundation/kerf-model/looks`) are FilmCraft's Lumetri presets
as looks, each behind an `amount`, so 0 leaves the picture alone and `fx c1 flatten 1` opens one up for
hand tuning. Their tints became color wheels, since a number can scale and a color cannot; Cross
Processed's crossed curves did too, and Legal Range is `color.levels`. Wheels and named looks draw in
a second pass of Lumetri's GPU kernel, as the CPU's reference does them after the rest, so Sepia at 4K
takes 39 ms a frame where the CPU took 236.

Grades stack as plain `fx` lines, with no new words. Each runs on what the line above drew, which is
byte for byte what a grade gives on a clip of a sequence holding the other. `amount` animates because a
number a look's lines only scale stays a straight line through its keys, so the keys land as they are on
each value it reaches (`Look::animates`); a number inside the look's own keyframes does not, and is refused.
A Lumetri or Levels with all its numbers at rest does not run (`FxOp::leaves_alone`), on either the CPU or the
GPU, since the way to display encoding and back can move a byte, so a grade at 0 is not there. Three grades
take 19 ms a frame at 1080p and 56 ms at 4K (128 and 471 on the CPU). Three at 0 take 12 and 27 ms: the same
bytes as a clip with none, which at 4K has a shortcut of its own (`plain_frame`, 6 ms), since `untouched`
reads the document and not the frame.

### Motion

Has: keyframes (hold, linear, easings, Bezier curves) with a graph editor in the editor (`apps/kerf-qt/ui/graph_editor.h`), transforms, titles with reveals by character or
word, set in variable fonts at any axis (`wght`, `wdth`, `slnt`, `opsz`, `ital`, which animate, in `glyf` or CFF2 outlines, with line height left at the font's own since none of those axes changes it by more than 0.7% in any of 50 variable faces tried), shapes whose outlines combine and offset (`fx shape.outline@1`, `offset`, on a sweep carried from
EffectCraft's `path`) with trimmed and tapered strokes, and particles worked out from their birth (`kerf-motion`, so any frame
can be drawn first), generators, 15 playful looks, and clips in depth with cameras and lights (`draw/three_d.rs`): each pixel's ray meets the clip's plane, so
perspective is true and lighting is per pixel, on the GPU where there is one (see below). Where two adjacent clips in depth cross, their run goes down pixel by pixel, nearest last, with the pixels on the line shared; a run where none cross
takes the same path as ever, and a crossing costs 3 ms more at 1080p and 5 ms more at 4K than the same two apart. A light with a `shadow` sends a ray from each pixel back to itself through the other clips,
reading each one's alpha where it passes, which is exact where a shadow map would be a sampled copy. With a `size` the light is a disc or an angle, and 32 rays go across it on one fixed spiral, each reading as much of a clip's alpha as its share of the light covers (from a summed table, or from just its rectangle on an opaque clip), so the same pixel gets the same rays on every frame and the edge has no steps. Where the four rays around the light say a clip cannot matter, or hides it all, no ray goes: on the CPU at 1080p a hard shadow takes 50 ms and a soft one 72 to 140, at 4K 200 and 290 to 570. A camera with an `aperture` blurs each clip alone by its plane's distance from `focus` at each pixel (`draw/focus.rs`,
carried from EffectCraft's progressive blur with a round iris), so a sharp clip keeps its edge over a soft one.
A clip with a `depth` is a solid with no mesh: each pixel's ray goes between the planes of its front and its back, half a picture's pixel at a time, reading the picture's alpha, and takes the color of whatever more of it it reaches, lit by the plane's normal at the front and by the slope of the alpha on a side. So a title's sides are its letters', its holes stay holes, and its edges are as soft as a pixel. What is empty of alpha is passed over by a summed table without being read. Lights fall only on the way a solid's surface faces, and its shadow is what five planes through it cast between them. On the CPU a 1080p title takes 25 ms for 16 flat, and 4K 108 for 70; with a soft shadow on a wall, 200 and 763. An environment light (`kind=environment image=V3`) is a track's picture wrapped around the scene as a panorama and reduced to how much light reaches each way a surface can face, a table of 32 by 16 made for each frame, which is all that a surface with no gloss shows of it, and such a frame is not kept, since it reads another track.

On the GPU all of that but the camera's blur is one kernel (`kerf-gpu/src/depth.wgsl`) doing the same arithmetic in 32 bits, with the compositor's 64-bit numbers measured from the eye so that nothing is lost by it. A kernel cannot take a varying number of textures, so what stands in a light's way goes into buffers first: its alpha for a hard shadow, its summed table for a soft one, neither for a solid color, and a track around the scene into its table of light. Clips that cross go down one below another in a tall texture and are put in order at each pixel (`by_depth.wgsl`). Held to the CPU by `tests/depth_gpu.rs` and `tests/gpu.rs`: on flat clips, crossings, hard and soft shadows, solids, and the environment light, 960x540, the mean is at most 0.001 of 255, and at most 0.013% of pixels are off by more than 2, those on a solid's sides facing straight up or down, where a table of light has no one way round; the test allows 0.05 and 0.05%. A camera's `aperture` was not carried over, so its frames stay on the CPU.

Median frame with `examples/motion_bench.rs` on an M1 with 8 GB, in ms, the bytes read back included:

| Scene | 1080p CPU | 1080p GPU | 4K CPU | 4K GPU |
|---|---|---|---|---|
| Two apart | 26 | 10 | 102 | 38 |
| Two crossing | 28 | 12 | 110 | 44 |
| Lit | 36 | 12 | 142 | 41 |
| Hard shadow | 51 | 12 | 204 | 42 |
| Soft shadow | 73 | 13 | 292 | 48 |
| Soft shadow from a point | 116 | 16 | 469 | 56 |
| Soft shadow all over | 139 | 16 | 563 | 58 |
| Soft shadow of a title | 98 | 25 | 393 | 82 |
| Solid square | 20 | 7 | 79 | 26 |
| Solid title | 23 | 18 | 95 | 60 |
| Flat title | 14 | 13 | 57 | 48 |
| Solid title with a soft shadow | 176 | 33 | 698 | 130 |

A title gains least, since its glyphs are still drawn on the CPU and go up as floats. So depth plays past 24 fps at 1080p in every row here; at 4K a hard shadow is about 24 fps, a soft one 17 to 21, and a solid title with a soft shadow 8.

A solid's `bevel` cuts its edges back at each face at 45 degrees, with no mesh either: at each step a ray asks how far inside the picture's outline it is, against how far a bevel takes the outline in at that depth, from a distance field made by jump flooding (`draw/bevel.rs`, `distance.wgsl`, the same passes on both). So a title's bevel follows its letters and leaves its holes, and is lit as a surface facing out of the outline and toward its face. Lights fall on it as on any surface, and its shadow is the same five planes through the solid, those within a bevel of a face with their outline taken in, each its own alpha and table. On an M1, a 1080p title with a bevel takes 36 ms on the GPU and 170 on the CPU, and 4K 171 and 894; with a soft shadow on a wall, 67 and 438, and 267 and 2021. Bevels are flat cuts, not rounded, and a clip scaled unevenly has them by the mean of its two scales. A bevel holds to the CPU as the rest does (`tests/depth_gpu.rs`).

The rule: **a composition is a sequence.** Nesting already works, so there is no
second document type, and an agent learns one model.

The picture shows and edits the curve of a selected clip's `path=`, through its parents
(`ViewerWidget::setMotionPath`, `parentMotion`; both held to the compositor by a table of values), adding a
point with a double-click and removing one with Delete. The graph editor zooms and pans both axes.

On a clip in depth the curve and the masks are drawn through what the camera does to a plane, which one projective
map says exactly, so `picturePlacement` and `pathPlacement` return it and the viewer paints through it and inverts it to drag
(`a_clip_in_depth_lands_where_the_table_says` holds it to `Scene::at` and `Plane::of`). A mask lies on the clip's plane; a path lies square to the world at the
clip's `z`, as a clip turns about its middle. What is keyed goes by the moment on the picture, so the rest of the curve says where
the clip would be if those stayed. A plane edge on or wholly behind the camera shows nothing, and in a drag what goes past a plane's horizon, or further than `path=` can say, stays where it last was. A path it faces along (`orient`) from an anchor off the middle of a turned clip leaves the plane by a different amount at each point, which no one map says, so its curve is drawn place by place from where the clip's own placement puts its anchor (`sampledPath`), and is shown and not edited.

To add:

- A camera's `aperture` on the GPU: its frames are the last in depth on the CPU.
- `three_d`, `z`, `rotate_x`, `rotate_y`, `depth`, and `bevel` in the Inspector, whose Transform rows are a fixed list: they are in the catalog and the language, and a generator's own parameters (a light's `shadow` and `size` among them) are all that it takes from the catalog as they come.
- EffectCraft's `render/three_d/adv` at `5c54866` is a triangle rasterizer with materials, and the rest of it does not fit rays on planes: models need meshes, a depth buffer, and a texture for each surface, and an environment's reflections need gloss, which no clip has.

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
matte kept beside the document. How soft its edge is at each corner is plain text on the same line, a third number on the corner, measured
across the edge in light: what lies well inside and well outside are the two colours mixed, and the
fall-off that draws what lies between says how wide it is and where its middle is (`roto/soft.rs`). A
subject out of focus on one side, or moving under an open shutter, comes within 0.052 of the true alpha on
average over its edge's pixels, where the best single `feather=` on the same outline is off by 0.08 to 0.09, in 14 ms a frame at 960x540. Hair comes out as a band that is soft on
average, not as strands, which is all a soft edge along an outline can say. A subject going further in a frame than two of its bands (a band being a fortieth of the frame's width) is found again first, by matching its own pixels across the frame at a few sizes (`moved_by`), and the flow then says only what changed of it, in a band up to three times as wide the further it went; below that the flow keeps up alone and nothing changes. At 640x360, a disk 120 across going back and forth over 24 frames at 64 pixels a frame overlapped the truth by 0.17 on average, lost after 5 frames, and now by 0.996, 0.995 at worst; at 128, 0.04 and 0.996; colors closer to the background's, 0.16 and 0.04 and now 0.98. At 2 bands and below it holds what it had, and the worst single frame at 4 bands of 200x150 is 0.938 where it was 0.979, on a mean of 0.993. Looking costs 8 of 48 ms a frame at 640x360 on every frame. A trained model is optional and swappable (`kerf_segment::MaskModel`):
it only sharpens the cut's prior, and one that disagrees with the flow is left out for that frame.
MobileSAM runs in plain Rust; its weights are never bundled, and `look` names the file to fetch. On a
disk over a still textured background for 24 frames at 200x150, the outline overlaps the truth by at
least 0.994; camouflaged so colour says nothing, 0.005 alone and 0.966 with a model standing in.

Given `camera`, `look` solves where the camera went over a shot (`kerf_track::camera`, EffectCraft's
camera tracker in Kerf's terms): points followed through it, a start from the essential matrix or a
flat scene's homography, the other frames found from what is placed, and bundle adjustment over it
all, with the focal length searched for. What it finds is `gen=camera@1` keyframes and a list of
places to set a clip's `x`, `y`, and `z` to, not a solve kept beside the document. Its first frame is
the 2D picture at the shot's own lens, with z=0 at the middle depth of what shows. It says plainly what
it cannot solve: a picture that does not move, a camera that only turns, a picture no one camera
explains. A camera that never turns fits any lens, which it says too, and takes the lens it is told.
On a camera wandering a room for 240 frames at 960x540, the way is 0.15% of how far it went off, the
turn 0.15 degrees at worst, and the field of view 0.2 degrees, with 0.13 pixels of reprojection error,
in 31 ms a frame after the frames are drawn (28 of it following points). A clip pinned under a 55
degree lens for 36 frames at 480x270 stays within 0.72 pixels of its spot.

A lens that changes during a shot is solved too, and goes in as `fov` keyframes on `gen=camera@1`. How much the picture grew from frame to frame, read from how far apart what it shows is, starts each frame's lens, and each key frame's focal length then goes wherever fits, against what lies at other depths: a zoom grows all of a picture alike, and a move along its line grows what is near more. It stands only where it leaves under three quarters of one lens's error, so a shot one lens explains comes out as it did. A camera that holds its lens, in a room filmed over 120 frames at 960x540, has the same 0.099 pixels, way off by 0.097%, turned off by 0.136 degrees, and a field of view off by 0.26; asking costs a second solve wherever the picture grows or shrinks by 3%, which on that shot takes 2.2 s where one lens took 0.83. Over the same way:

| Lens | Solve | Reprojection error | Way | Turn at worst | Field of view at worst |
|---|---|---|---|---|---|
| 1.2x | One lens | 0.37 px | 25.6% | 9.6 deg | 19.1 deg |
| 1.2x | Zooming | 0.14 px | 0.54% | 0.17 deg | 0.46 deg |
| 2x | One lens | 0.45 px | 68.1% | 17.8 deg | 42.1 deg |
| 2x | Zooming | 0.17 px | 0.43% | 0.24 deg | 1.25 deg |

A zoom at a flat wall square to the camera and a push in at one are the same picture, and so is a tripod turning under a zoom if a move can also be made to fit it. There `look` says so (`zoomOrPush`) and writes the move with one lens; and a camera that plainly only turns under a zoom is refused, as one that only turns is. A clip pinned under a zoom of 1.6x for 36 frames at 480x270 stays within 1.2 pixels of its spot.

Optical flow is in
`kerf-raster` now, which `image.fill` uses on a moving shot and `retime=flow` uses for slow motion.

### Image

Kept small on purpose. Video editors have always carried a little of Photoshop,
and that little is what belongs here. A still is a one-frame clip and a selection
is a mask, so this area adds no document type.

Has: `image.fill`; `image.clone` and `image.heal`, a stroke in the document each, painted from
elsewhere in the frame or from another frame, and from a place heal finds itself when not told one; and
`image.warp`, a mesh dragged by moves in small steps so that it does not fold, for liquify work.

Stays out: a brush engine, pixel layers as their own documents, RAW development,
and print color. When a job needs those, it is a job for a photo editor.

## The agent

The agent still gets four tools and a document it can read. Each area adds
vocabulary to the language and numbers to `look`, and no new tool: scopes as
measurements, tracks as data, and check findings for what a viewer would notice.

`look` with `contrast` reads each title and caption against what is behind it, from three frames at each
of three moments it has come in by: as drawn, without it, and its glyphs alone, white on nothing, so that a
title the colour of its background is seen not to show. Each patch about half a line across goes by the
strongest of its ink, so a caption's outline counts and a title half over a highlight is caught, and
it is flagged below WCAG's 3:1 over a fifth of it. It draws frames, so it is asked for and not part of the fast check; the full QC has it as a row, Text contrast, after the eight of the Finish tray's mockup, in the last tenth of its progress, and it adds 0.36 s a clip at 1080p, for at most 40 clips spread over the sequence. The same frames say what the frame cuts off: Clipped text, the row after it, is a text whose box goes past an edge with its glyphs reaching that edge, so a box wider than its ink says nothing and text moved wholly off the frame has nothing left to cut; `look` gives those as `clipped`. A crop or a mask is taken as meant.

`look` with `media` and `facts` gives a fact sheet beside the index: what the camera does in each shot, from the one move, turn, and zoom that
most of the picture agrees on between small frames four times a second (`kerf_track::shot`, so a subject crossing the frame is left out), and R128 loudness.
60 seconds of 1080p takes 7 s the first time, and it is kept with the media's other derivatives. It reads a push in as a zoom and a move sideways as a pan, and only `track` with `camera` tells those apart. It says in `notMeasured` what it
leaves out, since silence would read as a finding: faces and the subject, which take a trained model, and on-screen text, which without one cannot be found reliably.

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
  run on the CPU and compiled to WGSL, which would end writing every effect twice. Decoded IOSurfaces as wgpu textures were measured, and cost more than they save here; uploading decoded planes got more than that (see Depth and color). The fact sheet and the contrast check are in `look`.
