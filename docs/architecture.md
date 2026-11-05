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
| 1 | `render` | Pictures and how they are drawn on | `kerf-raster`, `kerf-draw` |
| 2 | `edit` | What a timeline does beyond its model: interchange, sharing | `kerf-interchange`, `kerf-collab` |
| 2 | `effects` | The effects carried from FilmCraft and EffectCraft, all areas together | `kerf-fx`, `kerf-raster-fx` |
| 2 | `image` | The little of a photo editor video needs | `kerf-image` |
| 2 | `color`, `motion`, `vfx` | Reserved: each gets its folder with its first crate | |
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

Measured on an Apple Silicon Mac with `examples/frame_bench.rs` in
`kerf-compose`, on 4K H.264 test footage, decode included, median frame:

| Scene | 1080p | 4K |
|---|---|---|
| Placed, nothing else | 61 fps | 15 fps |
| One Lumetri | 32 fps | 8 fps |
| Gaussian blur, radius 40 | 38 fps | 9 fps |
| One glow | 14 fps | 3 fps |
| Four layers | 31 fps | 6 fps |

Drawing is on the CPU (`rayon`, `f32` RGBA), and a frame's work is thrown away
when it is shown. 1080p with one light effect plays. 4K with any effect does
not, and neither does a stack of effects at any size. Three limits matter, in
this order:

1. **No GPU.** EffectCraft's compositor (`gpu`, 33.7k lines including its WGSL)
   left with its renderer. It compiled against EffectCraft's project model, so
   it comes back piece by piece rather than whole. `FxOp` in `gpufx.rs` already
   separates reading an effect's numbers from running them, which is the seam a
   kernel needs.
2. **Nothing is kept between frames.** A clip with no animation is drawn again
   every frame. EffectCraft's layer cache (`render/src/cache.rs`, `disk_cache.rs`)
   keyed a layer's pixels by a hash of exactly what they depend on.
3. **Eight-bit input.** Footage decodes to 8-bit sRGB RGBA (`footage.rs`,
   `Image::from_rgba8`). The working picture is float, but 10-bit, log, and HDR
   footage has lost its range before anything sees it. No grade can bring that
   back, so the color area depends on fixing it.

The plan, in order:

- **E1. `crates/render/kerf-gpu`.** One wgpu device, a texture pool, upload and
  readback, compositing, then `FxOp` kernels family by family. The CPU stays the
  reference: a kernel is tested against it, and an effect without one runs on the
  CPU, with the frame moved between them only where the stack changes sides.
- **E2. Results kept.** A layer cache in memory, then on disk, keyed by content.
- **E3. Deep input.** Decode at the source's depth, with its primaries and
  transfer, into a scene-linear working space. On Apple, hand decoded surfaces to
  the GPU without a copy. That needs `unsafe`, which the workspace forbids, so it
  gets one small crate with the exception written down.

## The areas

Code that left is still in history. `5c54866` has all of EffectCraft's crates as
they were before they were taken out; `91ae7f7` has inpainting, scopes, optical
flow, cut detection, ICC, and color spaces; `173914c` has the camera solver and
face tracking; `73a43cc` has the paint, expression, audio, and 3D-channel effects.
Carrying back means rewriting into Kerf's terms and wiring through the catalog:
code nothing calls was cut once already and should not come back as such.

### Edit

Has: ripple, roll, slip, slide, transitions, nested sequences, captions from
transcripts, snapping to pauses and cuts, roles, proxies, interchange, QC.

To add, in order: speed that changes over a clip (a clip's speed is one constant
ratio today), with optical-flow frames between; multicam; sound beyond gain and
fades, with effects and buses. A timeline's own tools stay in `kerf-model` and
`kerf-lang`.

### Color

Has: Lumetri, curves, levels, Hue/Saturation, Selective Color, CDL, LUTs, Auto
corrections.

To add: **scopes first**, carried from EffectCraft's `scopes.rs`, and given to the
agent as numbers through `look` so that it can grade by measuring, which a person
cannot do. Then color management (needs E3), the Lumetri presets, and layered
grades as stacked effects.

### Motion

Has: keyframes (hold, linear, ease), transforms, titles, generators, 15 looks.

The rule: **a composition is a sequence.** Nesting already works, so there is no
second document type, and an agent learns one model. To add, in order:

1. Bezier keyframes and motion paths, from EffectCraft's `keyframe`. The curve
   comes first since everything else animates on it.
2. Parenting, then 3D layers, cameras, and lights (EffectCraft's `render/three_d`).
3. Shape clips, from EffectCraft's `path` (booleans, offsets, trim, variable
   strokes).
4. Text animators and variable fonts.
5. Particles (EffectCraft's `fx_particles`, once E1 is in).

Expressions stay out. They exist so that a person need not write hundreds of
keyframes, and an agent writes them gladly; a look covers the reusable case.

### VFX

Has: keys, track mattes, blurs, glows, distortions, warp stabilizer, restoration,
360°.

To add, in order: optical flow; a point and planar tracker whose data clips and
masks can follow (EffectCraft's `track`); roto with the swappable model interface
of its `segment`, for a Magnetic Mask or Object Matte of Kerf's own; `image.fill`
over time, by flow (`fill_sequence`), so that it holds on a moving shot; the camera
solver.

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
  decoded IOSurfaces as wgpu textures for E3; a fact sheet for footage (faces, the
  subject, on-screen text, camera motion) beside what `kerfd` indexes now; and
  checks for clipped text, contrast, and safe areas.
