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

## What Kerf draws, and what it leaves to others

A video model now draws pixels no compositor will: texture, light, and the
look of a thing. So Kerf does not compete for them. Three jobs make a film
this way, and each has its own owner:

| Job | Who does it | What it has to be |
|---|---|---|
| The picture itself: texture, light, realism, style | A model, outside Kerf | Whatever the model is; Kerf draws none of it |
| Where things are and how the camera moves: the guide a model follows | A guide renderer: a sequence of the project, or a 3D renderer such as vork or Blender | Fast and readable by a model. Not beautiful |
| When things happen, and everything exact: cuts, sync, titles and logos, color, sound, delivery | Kerf's compositor | Deterministic, and playing in real time |

What joins them is the fourth thing, and it is Kerf's alone: the document
knows how each shot was made. A shot's recipe says what guided it, what was
asked, and of which model; each making is a take, and the take says what it
was made from ([shots.md](shots.md)). No model has that and no renderer does,
and a text document with proposals and Git merging holds it naturally.

A model is told three things, and the document holds each as something it
already has:

| What a model is told | By | Which in the document is |
|---|---|---|
| Where things are, and how the camera moves | A guide | Clips in depth, a camera, and lights, in a sequence |
| When something happens, and how much | A signal | A number with keyframes, on a clip that draws nothing ([shots.md](shots.md#signals)) |
| What it all looks like | A prompt and references | Words and files, in the recipe |

So it is one document, with one space and one time, and not one engine. The
camera a shot's footage was solved to, the camera its guide is drawn from,
and the plane a title stands on over the take are the same numbers, and a
signal's frames are the cut's. What draws from them stays apart: a
compositor for what has to be exact, a guide renderer for what has to be
fast, and a model for the picture.

Five rules follow, and the rest of this document should be read by them:

- **The compositor assembles; it does not stylize.** An effect that makes a
  look or an image a model would make better has no claim on a kernel, and
  what Kerf's own model already does (keyframes, clips in depth, shapes) is
  not done a second time as an effect. What an editor needs exact stays:
  color correction, keys and mattes, tracking, restoration, retouching, text.
- **Kerf draws a guide, not a finished picture, and builds no model.** What
  a guide shows is what the shot has, so a guide holds real shapes: clips in
  depth, cameras, lights, and meshes (`gen=mesh`: a solid, or a glTF file's;
  What directs a model). What it leaves out is what a model draws over it: materials, light
  that bounces, hair, water. What bends or breaks, a character or a cloth,
  is a renderer's, reached by a recipe's `run`. This rule said until
  2026-11-13 that Kerf builds no mesh renderer; the fifth rule is why it
  changed.
- **The speed that counts is from an intent to an accepted shot.** That is
  how many makings a shot takes, times how long a model takes, which no
  kernel shortens. What Kerf can do is make a guide show at once, say which
  edits are free and which cost a making, and play what came back without a
  stutter.
- **What grows is what the document can say to a model, not what Kerf
  draws.** A model that follows a new kind of direction gets a word a
  `generate` line hands it, written as keyframes where it changes over time,
  and its program turns that into whatever the model reads. A guide is made
  to say more (which clip a pixel is of, how far it is, where it went) before
  it is made to look better. Models change by the season; what directed a
  shot should still read when they have.
- **The model assumed is a good one.** Kerf is designed for a model that
  keeps what a guide shows, its places, paths, shapes, and camera, and draws
  the picture over it well: a 3D render covered by a model is how such shots
  are made. The model on hand for a test may be small and poor. What it
  makes shows that the way from the document to a take works, and decides
  nothing of the design: no effect is kept and no geometry left out because
  a small model drew badly.

### What these rules cut

38 effects went by the first rule, all of them among those with no kernel, and
none of them reached by a look, a grade, a skill, or a preset:

- *A look or an image a model makes better* (26): Brush Strokes, Color Emboss,
  Emboss, Find Edges, Roughen Edges, Motion Tile, Replicate, and Strobe Light;
  Alpha Glow, Echo Glow, Edge Glow, Lighting Effects, and Volumetric Rays; Lens
  Flare, Lightning, and Cell Pattern; Bulge, Spherize, Twirl, and Turbulent
  Displace; Long Shadow; and the 360° Chromatic Aberrations, Color Gradients,
  Digital Glitch, Fractal Noise, and Glow.
- *What a clip's own properties do* (12): Move, Grow, Shrink, Spin, Wiggle, and
  Camera Shake (keyframes, an expression, or a look); 3D Rotate and Basic 3D
  (a clip in depth); Bevel Alpha (`bevel=`); Circle and Ellipse (`gen=shape`);
  and Clone (clips side by side).

A document that names one is refused with what to use instead
(`kerf-model/src/retired.tsv`), as one naming an EffectCraft effect Kerf never
carried is. With them went nine FilmCraft definitions that no Kerf id had ever
reached (its Legacy bin's Gaussian and Directional Blur, Magnify, Mosaic,
Noise and Twirl, and Block Dissolve and the two wipes), about 3,700 lines in
`kerf-fx` and `kerf-raster-fx`.

What stayed among the effects that had no kernel then is what an editor needs
exact, whoever draws the shot: color correction (Curves, Hue/Saturation, Selective
Color, the Auto corrections), keys and mattes, what tracking writes (Corner
Pin, Warp Stabilizer, Rolling Shutter Repair), lens correction, restoration
(Remove Grain, Dust & Scratches, Median, Reduce Interlace Flicker), retouching
(`image.*`), burn-ins and text, gradients and grids to put behind titles, the
audio visualizers, Mosaic for what must not be seen, and time effects. They get
kernels in the order an editor needs them: Corner Pin, which tracking writes,
has one, and so have Drop Shadow, Rounded Crop, Spacer, and Mosaic, and
Curves, Hue/Saturation, Selective Color, Photo Filter, Unmult, the Cineon
Converter, Lens Distortion, Magnify, and the gradients, the checkerboard, and
the grid (The engine).

The six 360° effects that are left (projection, rotation, placing, and
seam-safe blur, denoise, and sharpen) stay: they are how 360° footage is
assembled, not how it is made to look, and Kerf keeps editing it.

### What is looked at again

The first cut went by the effects that had no kernel. A second goes by the
fifth rule, and asks one thing of every effect and look that is left: is it
done to a made shot, the same every time, or is it how a shot looks? The
first stays. The second is a model's, by its prompt and its references, or a
grade's.

What stays, and none of it because a model is weak:

- *What makes shots agree.* A model gives each shot a look of its own, and a
  cut wants them the same: color correction and grades, grain and a vignette
  laid over a whole cut as a finish, lens correction, stabilizing,
  restoration.
- *What has to be exact.* Text and logos, and what is done to them in depth;
  keys, mattes, masks, and what tracking writes, by which exact things are
  laid on a made shot.
- *What reads a picture.* Tracking, cutting out, solving a camera, scopes. A
  take is held to its guide by them (What directs a model, Next).
- *What is free at the edit.* A change to a look a model made costs a making.
  Cuts, transitions, retiming, and sound cost nothing and play at once, which
  the third rule counts.
- How 360° footage is assembled, and the little of a photo editor.

What went, of the 104 effects and generators that were left, is 15, and
with them four of the 33 looks and grades:

- *A look and nothing else* (8): Glint, Light Leaks, RGB Split, Wonder Glow,
  Wave Warp, Mirror, Posterize, and Echo. A model's, by its prompt and its
  references.
- *How a lens saw the shot* (5): Bokeh Blur, Focus Blur, Compound Blur, and
  Channel Blur, which are depth of field, tilt-shift, and fringing, and so a
  model's; what an editor still softens exactly, a plate behind a title or
  what a mask outlines, `blur.gaussian` does, and a camera's `focus` does on
  clips in depth. Camera Blur was `blur.gaussian` by another name.
- *What a clip or a shape does* (2): Offset (keyframes on `x` and `y`) and
  Write-on (a `gen=shape` clip with keys on `trim_end`).
- *The looks made of them*: `look.vhs`, `look.glitch`, `look.dreamy`, and
  `look.sparkle`. `look.film` lost its leaks and is version 2: grain and
  darkened corners, a finish.

What was in doubt and stayed, each for one of the reasons above:

- Every color effect, Color Pass, Leave Color, Tint, and Black & White among
  them: what they do is a grade, and a grade is how shots are made to agree.
- Noise beside Add Grain, and Vignette: finishes laid over a whole cut.
- Invert (a matte, a scanned negative), Magnify (a callout), Stroke and Edge
  Feather (cutouts), the flips, and Paint Bucket (a logo's flat color):
  exact things done to a made shot.
- Directional and Radial Zoom Blur: the whip, the shake, and the zoom through
  a cut are made of them, and those are free at the edit.
- Posterize Time, which is retiming; Transform, whose shutter blur a clip's
  own keyframes do not draw; Bilateral Blur, Median, Sharpen, and Unsharp
  Mask, which are retouching and restoration.
- The looks that move a clip or a title or make a cut (shake, flash,
  flash-cut, pop-in, slow-push, whip, wobble, zoom-pulse, zoom-through),
  `look.noir`, which is a grade, and the 18 grades.

So 89 effects are left, and 29 looks and grades. It was done as the first
cut was: a document that names one of these is refused with what to use
instead (`retired.tsv`), which for a look is the prompt and references of
the shot's recipe. With the effects went their kernels (five of
`vfx/kernels.wgsl` and three kinds of `FxOp`), about 2,300 lines in
`kerf-fx`.

Not decided here: the CPU's code as a second way to draw every frame (What
others do, vork).

## Where things live

`crates/<folder>/<crate>`. The folder is the rule: a crate may depend on its
own folder and on lower layers, and `tools/kerf-layers` fails the build's tests
otherwise. Nothing else has to be kept in step with it.

| Layer | Folder | What | Crates |
|---|---|---|---|
| 0 | `foundation` | The document, its language, and arithmetic everything shares | `kerf-model`, `kerf-lang`, `kerf-geom`, `kerf-color`, `kerf-text` |
| 1 | `render` | Pictures and how they are drawn on | `kerf-raster`, `kerf-draw`, `kerf-gpu`, `kerf-kernel` |
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

### Adding a generator

1. Implement `kerf_draw::source::PictureSource` in the area's crate and list it
   in the crate's `SOURCES`; `Reading` gives its parameters at the moment drawn.
2. Describe it in the catalog, as a generator.
3. For a new area crate, add its `SOURCES` to `kerf-compose/src/domains.rs`.

A document then says `clip g1 gen=<id>@1`, and the clip takes masks, effects,
placing, and depth as any other does. `shape` and `particles` are the worked
examples (`kerf-motion/src/sources.rs`). A solid and a title are still the
compositor's own, since it fills a solid on the GPU and plans a title's text
when it reads the document, and an adjustment layer, a null, a camera, and a
light draw nothing of their own. A test fails if the catalog has a generator
with no code.

### Media another program makes

A renderer or a model outside Kerf is not an effect or a generator: it makes a
file, and the file is media. The media line names its recipe (`recipe=`), a
text file with the steps that make the file, and `kerf make` runs them
(`kerf-engine/src/recipe.rs`) as supervised sidecars, as FFmpeg is run. So a
new renderer needs no code here: a recipe that runs it.

A recipe's steps are `run`, a program; `render`, a sequence of the project
drawn by the compositor, which is how Kerf's own clips in depth are a guide,
at the sequence's size and rate or at the ones the step names, with no sound;
and `generate`, a model asked for the file. `generate` says what is wanted in
words any model takes (a prompt, a guide, first and last frames, references,
signals, length, size, seed), and Kerf writes that as a request, a JSON file, for the
program the person's own `~/.kerf/models.json` names for that model
(`recipe/model.rs`). Kerf knows no model by name and ships none: with none set
up, a recipe that asks one is refused with how one is set up. A model with a
different way of being asked needs a different program, and nothing here.

What a recipe makes is kept as media, not as a cache, since a model does not
give the same shot twice. Each making is a take, in a folder beside the recipe
(`recipe/takes.rs`), and the media's line names the one it is, so choosing a
take is an edit: undone, proposed, and merged as any other. A take records a
hash of what it was made from, the recipe, the files it reads, the
sequences it renders (the sequence, what it nests, and the media lines it
plays, printed as a document and hashed), and the signals it hands a model
(the numbers alone, `recipe/signal.rs`), and a media wants making when one
of those has changed, and says which. Nothing else about the project is in
the record: an edit that trims, moves, grades, or titles a made shot is free,
and only a change to what guided it or what was asked costs a making.

The editor makes one from the bin's menu, and shows every step and asks
first: the bridge answers a plain `media_make` with the plan and runs nothing,
and runs only one sent confirmed, which a person's yes to that plan sends.
While it runs the bridge passes on each line its steps say (`media_making`),
and `media_make_cancel` stops it. A making says what it is doing in the
project's folder as it goes (`.kerf/cache/makings`, `recipe/told.rs`), so the
editor shows and can stop one that `kerf make` or an agent's own `kerf-mcp`
runs as it does its own. Stopping any sidecar asks before it ends it
(`sidecar.rs`): its process group gets SIGTERM and five seconds, and what is
left then is killed, so a model's program has time to take back the job it
queued, as `kerf-comfy` does. Windows has no such signal for a job object, and
ends it outright.

An agent closes the loop itself, as far as the person lets it. `look` tells
it which media want making (`toMake`: a file that is not there, or something
its take was made from changed, by name, or a recipe that cannot run, with
why), a media's takes, and the models set up. What would refuse a making is
read as a person's plan is, with nothing run, and said with the media
(`cannot`), and so is what the recipe asks that its model has said it does not
take (`notes`, which a person sees as the plan's notes). It writes the recipe
with its edit (`recipes`), as part of the proposal: the script carries the
recipe as comments for the person to read, and a recipe is never written
over: another text is kept as the next version beside it
(`open@2.kerfrecipe`, `recipe::version`), and the proposal moves the
media to it, so undo has it made from the one before, and the versions share
their takes' folder. It asks for the making with `make`
(`kerf-mcp/src/making.rs`), which answers with the plan. What follows is the
person's, in their own `~/.kerf/agent.json`, outside any project as the
models file is (`kerf-mcp/src/permissions.rs`), modelled on a coding agent's
permission modes: `ask`, the default, leaves the plan in the review thread for
`kerf make` or Make from Recipe…, which make it and answer the asking;
`accept-edits` accepts an agent's edits as they are made; `auto` also runs
the makings it asks for, up to a number a session, in the agent's own process
as a job it can wait for and stop as Stop Making stops one, and has the media
be the new take. What is let through is still a revision undo takes back and
a take kept beside the others. What a model takes (the size
and the rate a guide for it is drawn at, the longest shot, whether it follows
a guide) is said beside its program in the person's models file, under
`takes`, and `look` lists it with the model. Nothing is run to learn it: a
model's program can write its own entry for the person to keep, as
`kerf-comfy --describe` does from its workflow, and no tool of an agent's
starts one.

Passes of Kerf's own sequences are drawn: which clip each pixel is of, and
how near it is (`render … pass=ids|depth`, `kerf_compose::Pass`). Not here
yet: normals, which a 3D renderer gives today; a way to choose a
take in the editor's own window, where `kerf takes` and the conversation do
it; and a model reached without a program, such as Kerf Cloud hosting one.
What comes of these, and in what order, is under [What directs a
model](#what-directs-a-model).

One model's program is in the repository, and the engine knows nothing of it:
`tools/kerf-comfy`, for a model on a ComfyUI server
([shots.md](shots.md#a-model-on-a-comfyui-server)). With it a recipe was made
from end to end on 2026-11-11: a guide of Kerf's own clips in depth (a solid
boat leaving, a sea and a pier tipped flat, a sun, a camera pushing in and
turning), 96 frames at 1920x1080, then `generate` with Wan 2.1 VACE 1.3B, which
gave 65 frames at 832x480 and 16 a second. On four cores with no GPU the guide
took 87 s to draw, and the model, on its server, 79 s: there the guide was
over half of a first making. What that showed of the guide:

- *It was drawn larger and oftener than the model reads it.* A `render` step
  named no size and no rate, so the guide was the sequence's, and the model's
  program brought it down. A step now names them
  (`render s2 {guide.mp4} size=832x480 rate=16`) and the compositor draws the
  frames at that size, one for each frame of that rate
  (`render.rs`, `RenderOptions::size` and `rate`); a picture of another shape
  fills the size with what is over cut off evenly, as the model's program fits
  one. A like guide measured since on four cores with no GPU, 96 frames at
  1920x1080, took 54.5 s as it is and 7.8 s at 832x480 and 16 a second.
- *Drawn again, it costs little.* A second making with the guide unchanged had
  its frames from the disk (Kept frames) and the guide in a few seconds. A
  `run` step, such as a 3D renderer's, has nothing of the kind and runs whole
  for every take.
- *It carried sound it had not got.* The guide's file had a silent stereo
  track, mixed and encoded with it. What a `render` step draws now has no
  sound track, and its sound is not compiled.
- *From a terminal it was written once.* `project.kerf` edited by hand once
  kerfd has kept its state is not what `kerf make` reads, and `kerf` had no
  command that loads it, as the editor does, nor one that accepts what an
  agent proposed. `kerf load` now takes the file as the project, through the
  import kerfd already had for the editor's Load from Disk, and `kerf
  proposals`, `kerf accept` and `kerf reject` answer a proposal
  ([shots.md](shots.md#from-a-terminal-alone)). `kerf make` and `kerf takes`
  refuse a project whose file is not what kerfd has, before a making is paid
  for and its take then cannot be written to the document.

The shot kept the guide's layout, the boat's path, and the camera's move. It
also kept the guide's flat shapes, a cabin that is a box staying one; whether
depth would guide better than color was not settled then, and for this model
is now (What directs a model, Measured).

The same shot was then written by an agent with `kerf-mcp`'s tools and file
tools alone, from an empty project to a second take, with a stand-in for the
model that answers with its guide. Where it stopped, and what was done:

- *It stacked the scene with `insert`*, which opens time, and got nine clips
  one after another. The reference says so of `insert`; nothing said that a
  scene is stacked. The `model-shot` skill does.
- *Its recipe's mistakes showed only when a person ran it*: a file a step
  makes named without its braces, a sequence the project has not, a model
  that is not set up. `look` and the `edit` that names the recipe now say
  what would refuse the making, and what the model does not take.
- *The line it was told chooses a take was refused*, a path alone not being
  a media line. Each take now carries its whole line.
- *A media not made yet read as a file system error.* It says that it is not
  made, and who makes it.
- *A floor drew over the boat on it.* Clips in depth that did not cross drew
  in the order of how far their middles are, and a floor's middle can be
  nearer than what stands on its far half. Clips that cross were already
  drawn by the pixel, the nearest last (`three_d::composite_by_depth`). So
  are two now whose middles may have them the wrong way round: neither is
  all on the far side of the other's plane (`Plane::surely_behind`). A
  floor whose middle was nearer than a car on it hid the whole car and left
  its shadow, and the model drew a small flat car where the shadow was.
  Drawn by the pixel a frame of that guide costs what it did, 69 ms at
  832x480 on four cores, and a scene whose middles had it right is the same
  to the pixel. On a GPU it was not measured.
- *A shadow fell on the sky.* A backdrop in depth takes the shadow of what
  stands before it, and no clip can refuse one. The skill leaves a backdrop
  out of depth. A take made of a guide with such a shadow was none the
  worse for it (below), so no clip is given a way to refuse one.

A new project's empty first sequence made every proposal's check not valid,
for no fault of the edit, and most of all here, where the guide is a sequence
of its own beside it. A sequence with nothing on it is now said as
information (`sequence.empty`) and blocks nothing: rendering it is still
refused where that is asked for, and the full check fails the rows it cannot
measure of it.

How fine a guide has to be was then asked of the model's takes and not of
the guide's frames, on 2026-11-12: one scene, one prompt and one seed, with
one thing of the guide changed at a time, each made by Wan 2.1 VACE 1.3B as
65 frames at 832x480. The same guide, prompt and seed gave the same take to
the pixel, in two projects an hour apart, so what differs between two takes
here is the guide's doing. A making took 88 to 97 s, 83 to 91 of them the
server's, whatever the guide; the guide's drawing is the time beside it.

| The guide | Drawn in | What its take kept |
|---|---|---|
| As the skill lays it out: a sky out of depth, a far floor, a body with a cabin crossing it, a light with a shadow | 3.5 s | the car in its place, on its path, at its size |
| One box for the car | | place, path and size; a van, not a car |
| The floor drawn over the car, its shadow left: the fault mended above | 3.4 s | the path alone; a small flat car where the shadow was |
| A floor truly higher than the car's foot | 3.6 s | place and path; a car as tall as what showed of it |
| A backdrop in depth, the car's shadow on it | 6.1 s | place, path and size; the horizon where the backdrop meets the floor |
| The same, the light casting no shadow | 2.4 s | the same, to the tree beside the car |
| The light's `x` 0.288 for 0.5, its share of the frame's height for its width | 3.7 s | place, path and size |
| No lights | 2.0 s | place, path, size and kind |
| Drawn at 416x240 | 1.1 s | place, path, size and kind |

What was decided of it:

- *A guide is drawn right, and no finer.* The one thing that changed what
  the model made was a guide that showed the wrong thing: the floor over
  the car. That is mended. A model draws what a guide shows of a thing, and
  leaves out what it hides.
- *No plainer render for guides.* With no lights a guide was drawn 1.5 s
  sooner, and at half the size 2.4 s sooner, of a making of 90, where the
  server alone differs by 8 s from one making to the next. Neither take was
  the worse, so a guide may go without lights; nothing is built to draw one
  plainer.
- *No clip refuses a shadow, and `x` stays in frame widths.* A shadow on a
  backdrop and a light turned by the frame's shape each made other scenery
  and the same car. A backdrop is still left out of depth: in depth it took
  6.1 s to draw for 3.5, and where it meets the floor is the horizon.
- *A thing wants its outline, and that is all.* One box was a van, and a
  body with a cabin a car, on the same path. Flat colours did as well as
  lit ones.
- *A guide that is touched makes another take.* Two guides 41 dB apart,
  which the eye does not tell, gave takes 23 dB apart: the same car, another
  road and field. A take is judged for place, path and size, and one that
  is wanted is kept by leaving its guide as it is.

This is one scene, one seed and a small model. What a larger one keeps of a
guide is not known from it. By the fifth rule at the top, what was decided
here of how fine a guide has to be is of that model, and is no reason to
leave shapes out of a guide (What directs a model, 4).

Left as it is: what `kerf-comfy --describe` lists under `more`
includes the server's file names, which a person may want out of what an
agent reads. Where the person is asked for a making, the agent looks again
to learn that it was made.

### What directs a model

Decided on 2026-11-13, after Adobe showed a generated clip timed by a signal
(What others do): whether everything, effects and motion and the
guide, should become one 3D engine, since an agent and not a person now
writes the scene. It should not, and the 3D guide stays and grows.

- *The curve was not 3D.* It is a number over time, and no scene holds it.
  What holds a guide's space, a signal's time, and a prompt's words together
  is the document, which is why the table at the top has three rows and one
  document.
- *An agent writes a scene gladly and cannot see that it is wrong.* Of the
  faults in the shot an agent blocked (above), those in the scene itself
  were ones it had no way to see: a floor over a car, a shadow on a sky,
  clips laid end to end that should have stood together. Writing in three
  dimensions stopped being hard; checking did not.
- *What makes a 3D engine large is what a model now draws.* Materials,
  light that bounces, smoke, water: a renderer of Kerf's own that drew them
  would be large where it adds nothing. Shapes are not among them. A model
  that covers a render keeps the render's shapes, so those a guide has to
  have (4).
- *Color, keys, sound, retiming, and text are not in space.* They are done
  to a frame or along time, and they are what has to be exact. Nothing is
  gained by placing them in a scene.

What is built for it, in this order, each measured before the next:

1. **Signals**, which are in: `gen=signal@1`, and `signal=` on a `generate`
   line (`recipe/signal.rs`). Held by tests from the clip to the numbers a
   workflow's node is given. `look` with `signal` writes one's keys from the
   onsets of a sound on the timeline (`kerf-mcp/src/signal.rs`), which is how
   a signal is taken from music where Adobe's is. *Not measured against a model:* none in reach
   reads one, and the workflow that comes with `kerf-comfy` has no blank for
   one. The first thing to learn is whether a take's event lands on the
   signal's frames, with a workflow whose strength or mask takes a number a
   frame.
2. **Passes of Kerf's own sequences**, which are in: which clip each pixel
   is of, and how near it is (`render … pass=ids|depth`). What ties one
   frame to the next has been found to guide better than depth alone
   ([Diffusion as Shader](https://arxiv.org/abs/2501.03847)), and an ids
   pass does: a clip keeps its color as it moves. A pass is the picture's
   own drawing with one thing changed: a clip's picture, once its masks and
   effects have run and before it is placed, is filled with its color or
   with a grey for how near each of its pixels is, which its plane says
   exactly (`Drawing::as_pass`, `three_d::shade_by_depth`), and the scene is
   seen with no lights and no focus. So what is in front is decided as it is
   in the picture, by the pixel where clips cross. It is drawn on the CPU,
   with no kernels: a guide is small. Depth is one over the distance, scaled
   by the scene's own numbers and by nothing in the frame, so it holds still;
   a model trained on depth stretched to each frame's own nearest and
   farthest may want it stretched, which is not done. A pass has reached a
   model as its guide and come back as a take (Measured, below); a line
   cannot yet hand a model a picture and a pass at once.
3. **What `look` says of a scene in depth**, which is in: `look` with
   `shows` and `at` (`kerf_engine::shows`). It draws the frame's ids pass,
   and then each clip alone with the others made clear, so that what is
   parented to them still moves with them; what the two differ by is what
   hides a clip, and which clip. It says where each clip is, the share of it
   hidden and by what, the edges of the frame it reaches, and what has left
   the frame. The floor over a car that a guide once had reads as the car
   more than half hidden, by the floor
   (`a_floor_over_what_stands_on_it_is_said`). It is one small frame for
   each clip that draws, at most 32 of them, and reads to about a hundredth
   of the frame. A clip that is partly clear is read as not showing, and
   said to be.

4. **Geometry in the guide**, which is in (Next, 2). By the
   fifth rule what a guide shows is what the shot has: a model covers a
   render and keeps its shapes, so a car that is two boxes is a shot of two
   boxes. A clip with depth is its outline pushed straight back, one profile
   of a thing, and a thing has more than one. No measurement is waited for:
   this was to be decided by what a small model made of a camera going
   round, and the fifth rule says such a take decides nothing. `gen=mesh@1`
   is a box, a ball, or a cylinder, or with `file=` the mesh of a glTF file
   (`kerf-compose/src/draw/three_d/mesh.rs`, `gltf.rs`):
   its triangles are cut where the camera stops seeing, put on the screen,
   and scanned, the nearest kept at each of four places a pixel, which gives
   what a plane's rays give, a picture and how far ahead each pixel is. So
   it goes down among clips in depth by the pixel, as two planes that cross
   do, and a pass is the same drawing with its color changed: the clip's
   color with a hard edge for ids, a grey from each pixel's own distance for
   depth. `look` with `shows` reads it with no change, since it reads the
   ids pass. It is lit by the scene's lights by the way each place faces,
   and takes the shadows clips in depth cast on it. Of a file it reads what
   is rigid: the triangles of every mesh of its scene, each where its node
   puts it at rest, and each material's base color, made as tall as the clip
   says in the file's own proportions. Textures, skins, morph targets, and
   animations are not read. *Not done:* it casts no
   shadow, takes no effect and no mask (the compositor refuses them), a
   transition's wipe is a fade on it, and the GPU does not draw it, so a
   frame with one is the CPU's. It was not handed to a model.
5. **A making that streams**, last. Models that answer in under a second
   are reported, on data-center cards and at small sizes for now. A recipe makes
   files in steps, and a draft that is not kept wants a way of its own: the
   guide drawn on the GPU and handed on as it is drawn, with a take still
   the only thing the document names.

#### Measured

On 2026-11-13, with Wan 2.1 VACE 1.3B on a ComfyUI server, as on the 11th
and 12th: one scene, a road tipped back under a sky out of depth, a car of
two solids (a body and a cabin, `depth=0.3`) driving across, a camera
pushing in and turning ten degrees; one prompt, "a red sports car drives
along a wet city street at dusk"; 65 frames at 832x480. A making took 78 to
89 s. What changed was what the `render` step drew for the model to follow.

| The guide | Seed | What its take kept |
|---|---|---|
| The picture, in flat colors | 7 | Place and path. The guide's red boxes as red boxes, inside a dark van the model drew around them |
| The picture, in flat colors | 11 | Place, path and size. A red car at first, boxes again by the last frame |
| `pass=depth` | 7 | Place, path and size. A car, a red sedan with wheels and glass, its color from the prompt; a skyline where the road ends |
| `pass=depth` | 11 | The same: a car on its path, in every frame |
| `pass=ids` | 7 | The place and path of a faint outline with tail lights; the picture nearly black |
| `pass=depth`, the camera going 75 degrees round the car, parked | 7 | The car in its place, seen from its side and then from behind as the camera went round. The scenery did not turn with it |

What it shows, and what it does not. The model is a small one kept for
tests, so by the fifth rule how its takes look decides nothing of the
design.

- *The way works.* A pass drawn from the document reached a model as its
  guide through a recipe, and came back as a take, each time it was asked.
- *Which kind of guide a model follows is the model's.* This one made more
  of depth than of the picture, and nothing of ids. A model that covers a
  render is handed the picture. So it is said with the model, in what its
  entry says it is `about`, and is no rule of the skill's.
- *With the camera going round, the car turned and the scenery did not.*
  Written down, and nothing is decided by it.
- *Signals are still not measured.* This model has no input that takes a
  number a frame, and it is the only one on that server.

This is one scene, two seeds and a small model, as before.

#### Next

What comes after these, in the order it would be built, by the five rules
at the top. The scene the measurements above were made on is kept, with its
recipes, in `fixtures/shots/car`.

1. **The second cut**, which is done: 15 effects and four looks went (What
   is looked at again). What is left is what the rest is built beside.
2. **Geometry in the guide.** A clip whose picture is a mesh: read from a
   glTF file, or one of a few solids, a box, a ball, a cylinder. It is placed,
   keyed, and parented as any clip in depth is, seen by the sequence's
   camera, and in the ids and depth passes and in `look` with `shows` as any
   clip is, since those go by a clip and not by how it is drawn. It is drawn
   in one flat color a part, with the scene's lights, which is all a model
   covering it wants of it. Rigid only: what bends or breaks stays a
   renderer's through `run`, or footage. To draw it, the planes' rays do not
   do: it wants triangles and a depth for each pixel. EffectCraft's
   `render/three_d/adv` at `5c54866` is such a rasterizer and is the place
   to start from, carried into Kerf's terms as the rest was, and without its
   materials. The first of it is the solids, which need no file and are
   enough to say whether the passes and `shows` hold for a mesh: they are
   in, and the passes and `shows` hold (`kerf-compose/tests/mesh.rs`,
   `kerf-engine/tests/shows.rs`). A glTF file is read into the same
   triangles, a color a part. A recipe counts such a file among what a shot
   was made from only where its `needs` line names it; and no file a
   modeling program wrote was tried, only ones written by the tests.

   Where the shapes come from is not Kerf's to build. ComfyUI's 3D (What
   others do) is fine in what it holds, not in how it draws: models that
   make a mesh from a picture or from words, texture it, rig it, and
   animate it. A recipe reaches them as it reaches any model, with a step
   whose file is a mesh, `generate model=<maker> ref=car.png out={car.glb}`,
   and the clip is of that file. So a guide is as fine as its meshes are,
   and Kerf's drawing of them stays plain. Two things follow for what is
   drawn, now that the solids and a glTF's triangles are in: a mesh's own colors, its
   texture drawn unlit, since they are what a thing is and not how it is
   lit; and then a glTF's own animation, bones and all, since a character
   that comes rigged and moving is the one thing in a guide that rigid
   shapes cannot be. Splats, for a set rebuilt from photographs, wait for
   those.
3. **Footage into the scene, and a picture read back.** What people do in
   ComfyUI with a filmed shot is one loop, and Kerf wants the whole of it:
   the shot is made into a scene, its set as a mesh or as splats and its
   camera as a path; things are put into that scene, or the camera is moved
   in it; the scene is drawn; and a model covers what was drawn. Kerf has
   the last two. The first is a model's work, reached by a recipe whose
   file is the scene (`generate model=<rebuilder> guide=shot.mp4
   out={set.glb}`), and the second is the document's own, once it holds a
   mesh (2): the rebuilt set is a mesh clip with the shot as its colors, the
   camera a `gen=camera@1` clip keyed from the file or from Kerf's own solve
   (`look` with `track`), and what is added stands in the same space, behind
   and before the set's own surfaces, in the passes as in the picture. So
   this waits only on 2, and on a glTF's camera and animation being read
   with its mesh.

   A pass goes from
   the document to a picture. The other way, from a picture to its depth,
   its normals, the color of its surfaces without their light, is what
   inverse rendering does (DiffusionRenderer, UniVidX, and in ComfyUI itself
   MoGe and Marigold), which people call unrendering. In Kerf it is no engine
   code: a model's program whose file is a pass, as any model's is a shot,
   `generate model=<reader> guide=<footage> out={depth.mp4}`, with the next
   step's model following that file. Three uses, in the order they pay:
   - *Footage as a guide.* A filmed shot's depth has a model make it again
     in another look, with where everything is kept. This is the third place
     a guide comes from ([shots.md](shots.md#guides)), which so far is handed
     to a model as its picture.
   - *A take read back.* Its depth beside its guide's depth pass says how
     well it followed, where 4 below says only where the subject went.
   - *Light.* Normals and surface color of footage are what Kerf's own
     lights would need to fall on it. Not before the first two are measured:
     it would have the compositor draw a look, which the first rule is
     against.
   What it takes is a workflow for `kerf-comfy` and nothing else: load the
   guide, `MoGeInference`, `MoGeRender` with `depth`, save. The server the
   measurements were made with, ComfyUI 0.45, has had the weights since
   2026-11-13: `moge_3_vitl_fp16.safetensors` and
   `depth_anything_3_base.safetensors`, in `models/geometry_estimation/`.
   `LoadMoGeModel` and `LoadDA3Model` read only files packed for ComfyUI,
   which is why their lists were empty while the models' own files were
   there. What was found on it, by the person who keeps the server, on an
   MI250, and not run from Kerf:

   | Model | Setting | A picture takes | |
   |---|---|---|---|
   | MoGe 3, 1500x1000 | `refine_steps=3`, as it comes | 1.0 s, 5.2 s the first time | |
   | | `refine_steps=1` | 0.4 s | depth 0.11% from the default's, at the median |
   | | `refine_steps=0` | 0.1 s | 0.22% |
   | Depth Anything 3, one picture at a time (`mono`) | as it comes | 0.6 s | |
   | Depth Anything 3, a camera's path from many frames (`multiview`) | as it comes, or `bf16` | fails, on a dtype | |
   | | `weight_dtype=fp32` on `LoadDA3Model` | 1.0 s, with `cam_dec` | the one to use |
   | | `pose_method=ray_pose` | 3.0 s | |

   So MoGe is used as it comes, with `refine_steps=1` only for a very long
   shot, and **a workflow that has Depth Anything 3 find a camera's path
   sets `weight_dtype=fp32`**. Through the server's API a MoGe picture took
   2.0 s with the model's loading, and 0.5 s at `refine_steps=0`. MoGe's own
   package took 100 s a picture there, in its last refining step, and is not
   what ComfyUI's nodes run.

   The workflows are in the repository since 2026-11-13, four that come
   with `kerf-comfy` ([shots.md](shots.md#a-picture-read-back)):
   `depth`, `set`, `depth-whole`, and `cameras`. Each says
   `"of": "guide"`: what it makes is of its guide frame for frame, at the
   guide's own rate and size, where a model that makes a shot has a rate
   and a size of its own. That, and fetching the file whose kind the line
   asks for, is all `kerf-comfy` learned for them; the engine learned
   nothing. Run from recipes of `fixtures/shots/car` on that server, each on
   65 frames of 832x480, from the line being read to the file being there,
   with the guide sent up and the model loaded:

   | Recipe line | Took |
   |---|---|
   | `generate model=depth guide={take.mp4}` | 64 to 68 s |
   | the same with `refine_steps=1` | 30 s, its greys 0.3% from the default's |
   | `generate model=set guide=… out={set.glb}`, one frame | 3 s, a mesh of 97,000 corners with the frame as its texture |
   | `generate model=depth-whole guide=…` | 10 to 11 s |
   | `generate model=cameras guide=… out={cameras.txt}` | 10 to 11 s |
   | for scale, `generate model=vace …`, the take itself | 94 s |

   *A take read back* (`shots/readback.kerfrecipe`): the depth pass of the
   car driving across, the take Wan VACE made of it with seed 7, and that
   take's depth as MoGe read it. Held together frame by frame, over the
   pixels the guide has a depth for, by a script kept out of the
   repository:

   | The take's depth, read by | Same order of near and far as the guide (rank correlation, 1 is all) | Sky the guide has no depth for, read as none |
   |---|---|---|
   | MoGe 3 | 0.993, never under 0.990 | all of it |
   | MoGe 3, `refine_steps=1` | 0.993 | all of it |
   | Depth Anything 3, a picture at a time | 0.996 | 4%: it gives a sky a distance |
   | MoGe 3, the take played backward, which is the wrong take | 0.966 | all of it |
   | MoGe 3, the guide of the other scene | 0.865, and -0.42 at its worst frame | all of it |

   What it shows, and what it does not. The way works: a take's depth is a
   file a recipe makes, the same size and frames as its guide's pass, and
   the two can be held together. The number is of the whole frame, and
   most of this frame is a road that is where it was whichever way the car
   went, so the wrong take scores 0.966 where the right one scores 0.993:
   it says that the ground and the sky held, and little of where the
   subject is. That wants reading by the clip, which the ids pass says the
   pixels of, and is 4 below. A depth pass and a model's reading also differ
   in kind where they agree in order: Kerf's goes by the scene's own numbers
   and holds still, and MoGe's stretches each frame to its own nearest and
   farthest. It is one take of one scene, by a small model.

   *A camera found* (`shots/views.kerfrecipe`): the camera going 75 degrees
   round the parked car, drawn as its picture in flat colors, with
   `cameras`. It put the camera turning 7 degrees the wrong way over
   the first quarter and then 43 the right way, where the guide's turns 74
   one way throughout, with a lens of 37 degrees down the frame where the
   guide's is 40. A sky and a road in two flat colors give it little to
   hold, and footage was not tried: no filmed shot is in the repository.
   What it wrote is a file and nothing reads it yet.

   *Not done:* the camera path into a `gen=camera@1` clip; a set's own
   camera and lens, which `set`'s file does not hold; a rebuilt set
   drawn in a sequence with something added to it; footage.
4. **A take held to its guide by numbers.** Every row of the tables above
   was judged by eye: place, path, size. The document has both halves of a
   measure. `look` with `shows` says where a clip is on each frame of the
   guide, and `track` follows a thing through a picture from points on it
   (`kerf_track::roto`). So: follow the subject through the take from the
   middle of its box in the guide's first frame, and say how far its box
   in the take is from its box in the guide, frame by frame, as shares of
   the frame. It needs no model and no server, and is tried first on the
   takes `fixtures/shots/car` makes. It is what lets an agent say of a take
   what a person says of it, and choose between two.
5. **More than one guide on a line.** A `generate` line hands a model one
   guide. A model that reads a picture and its depth, or depth and ids,
   wants each named. No model in reach reads two, and Wan VACE takes one
   control video, so this waits for one that does.
6. **How a point moves, as a direction of its own.** ComfyUI has nodes for
   Wan-Move, which follows paths of points given as coordinates
   (`WanMoveTracksFromCoords`). A clip's place on each frame is what `shows`
   reads and what the document says outright, so its path is there to hand
   over, as a signal's numbers are. The weights were not on the server. It
   would be the fourth row of the table at the top: where, when, what, and
   how each thing goes.
7. **A signal measured.** Still wanting a model that takes a number a frame.
8. **In the editor**: somewhere to draw a signal's curve under the picture,
   and to choose a take.

Every effect the compositor runs on the CPU comes through that one interface,
whichever crate it is in and wherever its code came from: `kerf-fx` lists the
color correction and Gaussian blur every Kerf has had, the EffectCraft effects
Kerf keeps, and FilmCraft's (`kerf-fx/src/picture.rs`), beside `kerf-image`'s.
An effect gets its parameters, where and when it runs
(`kerf_draw::effect::EffectRun`), and, on a clip, a host to ask for the clip's
other frames, another track, or the sequence's sound (`EffectHost`). The
compositor asks no effect which kind it is. The editor's effect previews run
the same code on one decoded picture (`kerf_compose::effects_on_picture`), in
the order of the stack.

Still to come, each a place that runs effects its own way and so asks the
catalog what draws one (`Engine`):

- **The GPU's plan** (`kerf-compose/src/draw/gpu.rs`) names the effects with
  kernels by id. A kernel belongs on the same interface as the CPU's code.
- **Playback of plain cuts** (`kerf-playback/src/timeline.rs`) runs EffectCraft
  effects itself on the frames it decodes.

`Engine` goes when the last of these does. How an effect meets an HDR picture
(`filmcraft::catalog::headroom`) and whether it leaves a picture alone are
still tables by id beside the interface.

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

Measured on an M1 with 8 GB with `examples/frame_bench.rs` in
`kerf-compose`, on 4K H.264 test footage, decode included, median frame. The bench gives no number until
a scene's footage has decoded and its frames differ. The machine warms over a long run, which reads up to
15% slower on the CPU by its end, so a difference smaller than that says little:

| Scene | 1080p CPU | 1080p GPU | 4K CPU | 4K GPU |
|---|---|---|---|---|
| Placed, nothing else | 55 fps | 92 fps | 15 fps | 30 fps |
| One Lumetri | 29 fps | 82 fps | 8 fps | 38 fps |
| Gaussian blur, radius 40 | 35 fps | 49 fps | 9 fps | 17 fps |
| Four layers | 28 fps | 53 fps | 8 fps | 14 fps |
| One glow | 13 fps | 20 fps | 3 fps | 5 fps |
| Ultra Key | 22 fps | 85 fps | 7 fps | 38 fps |
| Radial Zoom Blur, amount 30 | 4 fps | 34 fps | 0.5 fps | 6 fps |

A 96-frame 1080p render with Lumetri, a masked and blended layer, and a title took
3.3 s against 7.4 s, 46 dB from the CPU's after encoding.

**What the GPU draws:** media, nested sequences, solids, titles, captions; clips in depth with their
lights, shadows, and solids; masks,
crops, letterboxing, placing, opacity, wipes and dissolves, dips, 71 of the 72 transitions carried from FilmCraft (`kerf-fx/src/transitions/kernels.wgsl`), all 27 blend modes,
adjustment layers; `color.basic`, `blur.gaussian`, the 26 kinds of operation in
`FxOp` with Lumetri among them, and Add Grain, and
Noise, Color, Luma, and Ultra Keys with their mattes cleaned up, Logo Cutout, Extract, Radial Zoom Blur, Corner Pin, Drop Shadow, Rounded Crop, Spacer, Mosaic, Curves, Hue/Saturation, Selective Color, Photo Filter, Unmult, the Cineon Converter, Lens Distortion, Magnify, Gradient, Ramp, 4-Color Gradient, Checkerboard, and Grid, and a Lumetri's curves, LUTs, HSL Secondary, and sharpening. So all 11 playful looks and all
18 grades stay on the GPU, including the transitions that are looks (Whip, Flash Cut, Zoom Through), since a look
transition is layers rather than a mix.
**What sends a frame to the CPU:** Morph Cut, whose optical flow would need both clips read back; every other
effect (other lights, stylize, and the other kept EffectCraft ones); a Lumetri with an HSL Secondary's Denoise, a median at each pixel, or a LUT with a 1D shaper; Add Grain with a Softness, whose planes are blurred, or showing its matte; a Corner Pin with its corners in a line, whose map has no numbers to give a kernel; Curves with a point outside 0 to 1; and in HDR, a Lumetri using a LUT, curves, sharpening, or an HSL Secondary, none of which any grade does. In HDR, so does an effect that
needs the whole picture brought under 1, which goes by a peak only the CPU reads; effects that bring
each pixel under 1 (`catalog::headroom`) stay on the GPU. Glow gains least
(13 to 20 fps at 1080p, 3 to 5 at 4K): its 18 blur passes are the work either way.

### Playing

A sequence plays at the size it can keep up with, not at the sizes in the tables above (`kerf-playback/src/drawn.rs`). Three threads draw the next frames, starting at 1280 wide at most, and every 12 frames the size goes down a step when frames take over 90% of their time and up a step when the next size up would take under 60% of it, by its area. What the GPU can draw never shrinks below 640x360, under which `Accel::Auto` gives a frame to the CPU. With `examples/playback_bench.rs` in `kerf-playback`, or in the editor with `KERF_PLAYBACK_STATS` set.

That floor is what real time turned on. The scene above, on three minutes of 4K H.264 at 40 Mb/s in a 1080p sequence, once held the editor at 500 to 700% of a core and still dropped a fifth of its frames, at 384x216: the first frames came while the kernels were building, the size shrank to a fifth of the canvas, and there the CPU drew every frame with all its cores, never quickly enough to grow back. And three decoders went through every frame of the file, since a thread asking for a frame just behind one already read sent another decoder after it, and the frame showing was asked for again and again. On an M1 with 8 GB, in the editor, over three minutes at 1389x868 with the frame cache off:

| | Before | Now |
|---|---|---|
| Shown | About 20 of 24 a second, at 384x216 to 625x352, by the bench, the editor having no count then | 24.0 of 24 at 1000x562, none dropped after the first seconds |
| CPU | 497% of a core, 354 to 687 | 216% |
| Footprint | 3.1 GB rising to 4.2 GB, with the machine swapping | 2.3 to 3.0 GB |

One frame at a time, in ms, on 4K H.264 at 40 Mb/s, decoding included:

| Scene at | GPU | CPU |
|---|---|---|
| 1920x1080 | 129 | 323 |
| 1280x720 | 68 | 150 |
| 1000x562 | 49 | 98 |
| 640x360 | 26 | 43 |
| 384x216 | 23 | 21 |

Three threads make 49 ms 24 fps, with some to spare, and 68 ms 14 to 17: frames on the GPU go one after another. The CPU's 21 ms at 384x216 takes all its cores to do. Of the 216%, a profile of the playback alone (160%) has about half in libavcodec and a third in libswscale scaling 4K down, and the editor's painting adds the rest. A 4K H.264 frame costs 20 ms of a core to decode at 20 Mb/s and 44 at 40, which all cores together turn into the 3.6 to 10 ms these tables go by. Decoding on VideoToolbox would take most of that off the CPU, about a core here, and `ffmpeg-next` has no way to ask for it without unsafe code, which the workspace forbids.

Memory went the same way. A decoder keeps six frames behind it and a decoder idle for five seconds goes, about 300 MB of 4K frames each. A texture size no frame has asked for in two seconds leaves the pool, where each size playback had been through stayed up to the budget. 384 MB of finished frames are kept, about eight seconds, down from 768, and 128 MB of frames as they decoded, down from 512, since going forward nothing asks for one twice. Those two trade instant replay for what an 8 GB machine has: past them a frame comes from the disk, or is drawn.

Three limits remain, in this order:

1. **More of the frame on the GPU.** Each effect without a kernel costs its whole
   frame the GPU. The transitions have theirs now: each mirrors its arm of `kerf_fx::transitions`, which stays the reference, and is held to it per value (`transitions/gpu_tests.rs`, at most 5e-4 on values from 0 to about 1.5, for each parameter of each one) and per frame (`tests/gpu.rs`; a one-off over all 71 between two 4K clips comes to a mean of at most 0.006 of 255 at 1080p and 0.055 at 4K, with no pixel off by more than 2). A test names which have kernels and which do not, so the list cannot drift. Only Morph Cut stays on the CPU. EffectCraft's WGSL (`effectcraft/crates/gpu` at `5c54866`) turned out not to help: its `fx_transition.wgsl` draws Block Dissolve and the CC transitions, which `retired.tsv` maps onto these by resemblance, and none has this arithmetic. A transition mixes what its clips draw, with no regard for HDR on the CPU, and so on the GPU. Building the kernels takes 1.6 s the first time a new build runs on an M1, on the background thread, and 6 ms after that, since Metal keeps what it compiled.

   With `examples/frame_bench.rs`, `transition` as its scene, on an M1 with 8 GB, between two 4K H.264 files, median frame of the middle half of the cut, in ms, decoding both included. The 1080p floor is about 24 and the 4K floor about 50, decoding and putting up the two clips:

   | Transition | 1080p CPU | 1080p GPU | 4K CPU | 4K GPU |
   |---|---|---|---|---|
   | Median of the 72 | 45 | 27 | 142 | 56 |
   | Iris Round | 29 | 27 | 81 | 66 |
   | Film Dissolve | 49 | 25 | 144 | 55 |
   | Push | 110 | 34 | 408 | 85 |
   | Glass | 55 | 28 | 184 | 56 |
   | Whip | 226 | 34 | 877 | 90 |
   | Lens Blur | 232 | 35 | 935 | 95 |
   | Cross Zoom | 394 | 41 | 1613 | 119 |
   | Radial Blur | 765 | 50 | 3396 | 207 |
   | Blur Dissolve | 60 | 49 | 227 | 127 |
   | Glow | 47 | 39 | 183 | 123 |

   So at 1080p 69 of the 72 play past 24 fps, where 35 did, and at 4K all but Morph Cut are between 5 and 20 fps, where the blurs through a cut took up to 3.4 s a frame. Two take least from it, since what they do is blur: Blur Dissolve and Glow, whose box passes cost what they cost.

   Of the effects, what a look or a grade reaches came first, and then what a person grading reaches. Film was the only look left on the CPU; Zoom Through, which this list used to name with it, had been on the GPU since Radial Zoom Blur was carried. They are held to the CPU per value as the rest are (`vfx/gpu.rs`, `kept.rs`, within 2e-3) and per frame (`tests/gpu.rs`). With `examples/frame_bench.rs` on 4K H.264, median frame in ms, decode included; a Lumetri with its plain numbers alone is 12 and 25 on the GPU:

   | Effect | 1080p CPU | 1080p GPU | 4K CPU | 4K GPU |
   |---|---|---|---|---|
   | Film (look) | 140 | 27 | 579 | 84 |
   | Add Grain | 59 | 14 | 233 | 36 |
   | Light Leaks, since cut | 62 | 21 | 239 | 64 |
   | Lumetri, luma curve | 54 | 14 | 192 | 31 |
   | Lumetri, hue vs sat | 69 | 14 | 239 | 31 |
   | Lumetri, LUT of 33 | 63 | 14 | 227 | 32 |
   | Lumetri, sharpening | 55 | 17 | 228 | 55 |
   | Lumetri, HSL Secondary | 75 | 15 | 273 | 37 |
   | Lumetri, HSL Secondary with Blur | 87 | 24 | 309 | 71 |
   | Ultra Key, Choke | 56 | 17 | 188 | 54 |
   | Ultra Key, Soften | 59 | 17 | 222 | 69 |

   What was measured and left, on the CPU, in the same terms, which no look or grade reaches. Mosaic stood in this table at 15 and 43 and has a kernel since (below), not yet timed. Ten more stood in it (Brush Strokes at 142 and 802, Echo Glow at 99 and 572, down to Replicate at 21 and 65), each a person's choice of a stylized picture; they were not given kernels, and have since been cut with the rest of what a model draws better (What Kerf draws, above).

   | Effect | 1080p CPU | 4K CPU |
   |---|---|---|
   | Lumetri, HSL Secondary with Denoise | 89 | 502 |

   After those came what an editor needs exact and no look reaches, by who writes it. Corner Pin was first: tracking a surface writes one, keyframed, on the clip that goes onto it (`look` with `track`), so every frame of a tracked shot went to the CPU. Its kernel is held to the CPU per value (`vfx/gpu.rs`, 1.5e-5 at worst of the 2e-3 allowed) and per frame (`tests/gpu.rs`, a mean of at most 0.004 of 255, with no pixel off by more than 2), and in an HDR sequence, where it moves what a picture holds over 1 and past BT.709 as the CPU does (`tests/hdr.rs`, `the_gpu_draws_an_hdr_picture_through_an_effect_as_the_cpu_does`, which allows twice what the footage alone comes apart by from decoding, 74 of 65535 here, and a little). Those were read on Mesa's llvmpipe, which is no GPU: **none of this was timed**, and `frame_bench` has a scene for it, `corner pin`, to time on a Mac. It is written by hand and not from `kerf-kernel`: the CPU maps each pixel in 64 bits before it reads the four around where it lands, which WGSL has not got and `Num` cannot say, so one source could not draw what the reference draws. Corners in a line, whose map divides by nothing, stay with the CPU.

   Then what frames a picture and what hides one: Drop Shadow, Rounded Crop, Spacer, and Mosaic. A drop shadow is the picture moved (`kerf-gpu`'s placing, which the CPU's `transformed` is), turned into the shadow's color by its alpha, blurred by the box passes every Gaussian is, and put under the picture; a spacer is the picture placed smaller, its corners rounded and a color put behind it; a mosaic is a small picture with a pixel to each block, which every pixel of the block then reads, softened toward what lies between. Their arithmetic at a pixel, where it is 32 bits of the pixel and where it is, comes from one source in `kerf-kernel`: the rounded rectangle, a rounded crop with its feather and border, what a spacer does once the picture is placed, and a shadow's color. `Num` got `abs` and `hypot` for them. The CPU's code was not moved onto that source: it stays where it is as the reference, and the source as `f32`, with the CPU's own passes around it, comes to that code's very values over a whole picture, each way the effects can be set (`kernels_from_one_source_are_the_effects_to_the_bit`), which needs no GPU to know and so runs in every CI job. Mosaic is by hand: it reads a second, smaller picture at whole-number places, which `Num` has no way to say. On the GPU all four are held per value (4.9e-6 at worst of the 2e-3 allowed) and per frame (a mean of at most 0.016 of 255, no pixel off by more than 2), and in an HDR sequence, on llvmpipe as before, and none of them was timed: `frame_bench` has `drop shadow`, `rounded crop`, `spacer`, `mosaic`, and `mosaic soft`.

   Then the color tools an editor reaches for past Lumetri: Curves, Hue/Saturation, Selective Color, Photo Filter, and Unmult, which Kerf keeps from EffectCraft, and FilmCraft's Cineon Converter. The five kept ones are written for a picture encoded and premultiplied, which the CPU makes of the frame on the way in and unmakes on the way out, and which Radial Zoom Blur and Add Grain take a pass each way for. These do both in the pass that does the effect, one where there would be three; and an effect that changes nothing as set, Curves with no curve or Selective Color at 0, still goes there and back as it does on the CPU, which clips what is over 1. Photo Filter, Unmult, and the Cineon Converter come from one source, for which `Num` got a power it is told, `exp`, and `log10`, and as `f32` they come to the CPU's very values, an Unmult with a hard edge included (`color_kernels_from_one_source_are_the_effects_to_the_bit` in `kept.rs`, and the test of that name in `vfx/gpu.rs`). Curves, Hue/Saturation, and Selective Color are by hand: a curve is a table read at a whole-number place; a hue turns by a remainder, and each of its six ranges is taken or passed over by the pixel's own hue; and Selective Color's ranges go by which channel is the largest and which the smallest. `Num` says none of those. So that a kernel is told an effect's numbers as the effect reads them, `kerf-raster-fx` lets out its curve with its table and ends, its hue ranges, its filters' colors, and Selective Color's names; what the effects compute is as it was. Curves with a point outside 0 to 1 stays with the CPU: only with its ends there is a curve a table between them and a line past them. In HDR each pixel is brought under 1 for all six and put back, as `catalog::headroom` says and the CPU does, and they come as far apart there as Gamma and Tint do, which have long been held (64 to 89 of 65535 against 82 and 115, the footage alone being 74). Held per value (1.4e-5 at worst of the 2e-3 allowed, 6.5e-7 for the Cineon Converter) and per frame (a mean of at most 0.008 of 255, no pixel off by more than 2), on llvmpipe, and not timed: `frame_bench` has `curves`, `hue saturation`, `selective color`, `photo filter`, `unmult`, and `cineon`. One thing the frames showed that is the footage's and no kernel's: a hue turned by 30 degrees puts the test footage's pure colors half way between two bytes, where the CPU and the GPU round apart for a third of the pixels, by 1 of 255. `color.procamp hue=30`, long held, does the same to the same count, and the scene here turns by 25.

   Then lens correction and the loupe: Lens Distortion and Magnify. Each reads the picture where a pixel comes from, toward or away from a center, and Magnify mixes what it reads with what is there in one of five ways, under a round or a square glass with a feathered edge. Both are by hand for Corner Pin's reason: the CPU works out in 64 bits where a pixel comes from. Held per value (6.1e-6 at worst of the 2e-3 allowed) and per frame (a mean of at most 0.006 of 255, no pixel off by more than 2), and in an HDR sequence, on llvmpipe, and not timed: `frame_bench` has `lens` and `magnify`.

   Last of this round, what generates a picture over the clip's, to put behind a title or lay over a frame: Gradient, Ramp, 4-Color Gradient, Checkerboard, and Grid. The 4-Color Gradient and the Grid are from one source, for which `Num` got a remainder, and as `f32` they come to the CPU's very values. The other three are by hand: a Ramp works out in 64 bits how far along a pixel is; a Gradient scatters by a hash of 64 bits of where a pixel is, which the kernels already had for Light Leaks; and a Checkerboard's squares go by whole numbers, odd and even, and it goes over the picture in one of six ways. Held per value (4.0e-7 at worst of the 2e-3 allowed) and per frame (a mean of at most 0.008 of 255, no pixel off by more than 2), and in an HDR sequence, on llvmpipe, and not timed: `frame_bench` has `four colors`, `grid`, `ramp`, `gradient`, and `checkerboard`.

   That is 18 kernels, and 45 of the catalog's 224 where there were 63. Eight take their arithmetic from one source and ten are by hand, and what decided each was whether its arithmetic, written over `Num`, could come to the CPU's very values: where the CPU works in 64 bits, reads another pixel or a table, or counts in whole numbers, it could not, and a kernel by hand held within a tolerance is what the rest already are. `kerf-kernel` is 1,472 lines for it, from 875. What the one source gave here is not fewer lines but a test with no GPU: the CPU's code stays where it is, so nothing is saved in writing, and each kernel from the source is held to that code to the bit on every machine, where a kernel by hand is held to it only where there is a GPU. The same round has `the_kernels_are_wgsl`, which reads all the kernels' WGSL with naga and no GPU: twice a name WGSL keeps for itself (`from`) stopped every kernel building, which on a machine with no GPU nothing else would have said. What is left without a kernel reads other frames, another track, the whole picture's statistics, or text, or is restoration, retouching, or 360°, and was not in this round.

   Three effects cost seconds a frame or close to it, at their defaults, and no look or grade reaches them. Each stayed on the CPU and got a better algorithm, as the numbers below decided. With the same bench, median frame in ms, and for the fills a box as wide as its share of the frame:

   | Effect | 1080p before | 1080p | 4K before | 4K | Peak memory at 4K, before and after |
   |---|---|---|---|---|---|
   | Bokeh Blur, since cut, default amount 10 | 427 | 31 | 6251 | 127 | 1.10 GB, 1.03 GB |
   | Bokeh Blur, amount 12 | | 32 | about 9000 | 148 | |
   | Bokeh Blur, amount 13 | | 32 | about 900 | 79 | |
   | Remove Grain | 184 | 138 | 667 | 435 | 1.95 GB, 1.53 GB |
   | Fill, 10% | 393 | 374 | 1870 | 706 | |
   | Fill, 30% | | 396 | | 770 | |
   | Fill, 60% | 6440 | 1199 | 32633 | 1625 | 2.03 GB, 1.70 GB |

   - **Bokeh.** It read one tap for each pixel of the iris, so its cost went with the square of the radius, and the radius is in the clip's pixels and so grows with the frame: 5 pixels at 1080p and 121 taps, 10 at 4K and 441. That is 3.6 times the taps on 4 times the pixels, the 14 to 16 times measured. It also fell off a cliff: amount 12 took 9 s at 4K, and amount 13, the first gathered on a half-size picture, took 0.9. A running total along each row reads a row's worth of the iris in two reads, so the cost now goes with the iris's height. On 4K footage it comes within 63 dB of the old kernel, and no channel is more than 5 of 255 away; on real footage, sunlit water from *Chimera* (Real footage, below), within 67 to 83 dB at 1080p and 4K, a round and a six-sided iris, with no channel more than 1.7 of 255 away, without the highlight boost, which the old kernel's reference here does not have. A kernel gathering tap by tap would not have caught this: Radial Zoom's rate on the GPU, about 12 billion reads a second at 4K, puts 441 taps near 300 ms, over twice what the CPU now takes, though it would leave the frame on the GPU.
   - **Fill.** At 4K, 24 of the 29 s went to one level, the finest, and nearly all of that to its 20 sweeps of searching, where a patch 7 pixels wide sees little but grain. A level with more than 64,000 patches and a shorter side over 540 now takes over the matches from the level above and copies through them, voting over one pixel around rather than a whole patch. On pictures with no way of knowing what was hidden it comes as close to the truth as the search did, within 0.1 dB, and where the truth can be found, a tiled texture, it finds it exactly. It keeps more grain than the search, since a vote over a patch averages it away: 81% of the fine detail of the picture around it, from 63%, at 4K on a synthetic grainy picture. On real footage, where what a hole hid is known (Real footage, below), it keeps less than that, 19 to 54% at 4K against the search's 7 to 33%, and comes within 0.05 dB of the search; a camera's noise is finer and fainter than that grain. A small fill, searched all the way down, keeps 3 to 20% of it at 1080p, so a real fill reads smoother than what is around it. Side by side at full size both show the same structure. A small fill is still searched all the way down and draws byte for byte as it did. Keeping more would not help footage that moves, since a frame has to fill the same whichever was asked for first, so one frame's matches cannot start the next; identical frames already share a fill. Random search per pixel, with what it has found shared along the row, is not something the kernels here can do.
   - **Remove Grain.** A third of it was measuring the grain over the whole picture, for 80 boxes covering a thousandth of it, and finding those boxes on one core. It now reads only around them, shares what its guide gives across the three channels, and holds fewer copies. Its frames are byte for byte what they were at 8 bits. On a real camera's noise, with the truth made from the shot (Real footage, below), its default amount of 1 brings a frame 1.1 dB nearer the truth and halves the noise in the sky, keeping 95% of the skyline's detail; amounts of 3 and 5 take more detail than noise, 1.9 and 4.0 dB further from the truth than the frame was, keeping 65% and 44%. What is left is its guided filter, a little over 200 ms at 4K, and the CPU's own path. It stays on the CPU because the grain it measures sets how hard it filters, so a kernel would have to read pixels back for every frame.
   **One source for three of them.** Every effect with a kernel is written twice, once in Rust and once in WGSL: 3,387 lines of WGSL for effects and transitions (`fx.wgsl`, `vfx/kernels.wgsl`, `transitions/kernels.wgsl`) and 2,110 more for drawing in `kerf-gpu`, against 190,000 lines of Rust. They have seldom had to be corrected against each other once in: of the 15 commits of Kerf's own to touch WGSL, one was such a correction, a bevel's ray steps rounded differently in 32 bits (`4d4ca01`), and only 48 lines have ever been taken out of a kernel for any reason, since a kernel lands with the test that holds it to the CPU. What the writing twice costs is the writing, which history does not show.

   `kerf-kernel` tries the idea, with no unsafe code and no compiler of its own: a kernel is Rust generic over a number, `Num`. With `f32` it runs on the CPU as the code it replaces did, and with `Traced` every step it takes writes a line of WGSL. On three of different kinds, a grade per pixel (Color Balance), a blur going along a line with a running sum (a box pass, which every Gaussian is), and a wipe reading two pictures (Iris Round and the edge of all wipes), median ms at 4K, by hand and from the one source, before anything else in a frame:

   | Kernel | CPU by hand | CPU one source | GPU by hand | GPU one source | Worst against the CPU, on the GPU |
   |---|---|---|---|---|---|
   | Color Balance | 83 | 80 | 4.9 to 5.4 | 4.3 to 4.8 | 6.2e-7, held to 2e-3 |
   | Box pass, any radius | 6.9 | 6.8 | 5.5 to 5.9 | 5.7 to 6.0 | 0, held to 2e-3 |
   | Iris Round | 14.3 | 13.1 | 10.4 to 10.6 | 5.6 to 8.2 | 9.6e-5, held to 5e-4 |

   On the CPU each comes to the same bits as what it replaced, tested against a copy of that code, and a whole frame takes as long as it did: on the GPU at 4K 25.4 ms to 25.5 with Color Balance, 51.4 to 51.7 with a blur of 40, and 214.3 to 215.9 for the iris, and on the CPU 73.5 to 74.7, and 101.2 to 98.1. The box pass came out 60% slower on the CPU at first, with its sum in a `Cell`, and level with the sum on a plain array and whole pixels read and written: a source shared this way still has to be written with the CPU's loop in mind. So these three are carried over, and the numbers go in from what they write out. A frame on the CPU, or on the GPU with no Color Balance, is the same bytes as before; with one, up to 25 of a 1080p frame's 8.3 million bytes moved by 1 of 255, within the tolerance, which looks like how Metal rounds scalars against vectors, though that was not confirmed. That what is written out is valid WGSL is tested with no GPU, by naga.

   It does not pay in lines yet. The crate is 608 lines for these three, about 400 of them the machinery, where they took out 78 net, and a kernel moved over gives up its WGSL, 10 to 40 lines, for a small function that writes it out, so it would take most of what exists to come out even. What it buys is that the two cannot disagree: with a slip in a kernel, the CPU and the GPU are now wrong together, which the tests against a reference on the CPU catch and those of one side against the other no longer can.

   The rest would take, and this is judged from the code and not measured: for most of the 73 per-pixel kernels, a few more operations on `Num` (`floor`, `sin`, `cos`, `atan2`, `exp`), a way to read a picture bilinearly and a LUT, and rewriting each branch as a closure, perhaps two or three weeks for the 3,387 lines; `kerf-gpu`'s depth and focus kernels should stay as they are, since they do in 32 bits what the CPU does in 64 by design. The better course is to write new kernels this way and move an old one when it has to change anyway, not to move them all, which would spend weeks on a problem that came up once.
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
   the brightness it has after the effect, on the CPU and the GPU, and the whole picture is lifted as well as scaled, which a filter over its pixels undoes exactly. What changes a pixel's channels apart (`stylize.posterize`, `color.autocolor`, an exposure that saturates) keeps the gamut but moves the hue of what lies far past it. ProRes and HEVC Main 10 carry HDR; H.264 is
   8-bit. `look` with `scopes` reads an HDR sequence by its PQ or HLG signal, with what that comes to in cd/m² and how much of it BT.709 cannot hold; so does the Finish tray, whose bridge draws that signal small again on a thread of its own while the scopes are in sight, since playback hands over only the 8-bit frame a monitor gets, and marks where SDR white falls. An HDR10 file (`hevc_hdr`) says what it was mastered on
   (P3 D65 at 1000 nits unless told otherwise) and its MaxCLL and MaxFALL, measured from the frames
   before encoding, which disk-kept frames make cheap; HLG carries its tags without them. Holding an effect costs GPU passes, measured with `examples/frame_bench.rs` on 4K ProRes in PQ on an M1 with 8 GB, ms a frame, decode included:

   | Scene | 1080p SDR | PQ | HLG | 4K SDR | PQ | HLG |
   |---|---|---|---|---|---|---|
   | Placed, nothing else | 18.5 | 19.7 | 19.8 | 46.2 | 51.3 | 51.7 |
   | `color.basic`, each pixel held | 19.9 | 25.5 | 23.1 | 33.0 | 49.3 | 49.8 |
   | `blur.unsharp`, Gamut held | 27.0 | 30.9 | 31.3 | 69.7 | 87.2 | 86.4 |
   | `light.wonderglow`, since cut, Gamut held | 60.5 | 69.3 | 65.9 | 211.7 | 224.2 | 218.1 |

   So a frame costs 1 ms at 1080p and 5 at 4K by being HDR, a held `color.basic` adds 3 to 6 ms at 1080p and 17 at 4K, and a Gamut effect 4 to 9 and 6 to 18. Handing VideoToolbox surfaces to the GPU without a copy was measured and left (`examples/upload_bench.rs`, an M1 with 8 GB), by the time a frame takes and not by the CPU it uses (see Playing). A 4K H.264 frame takes 4.5 ms to decode in software on all its cores, 1.4 to convert, and 4.7 to upload, 10.6 in all, against 14.5 ms just to decode it on a surface; 1080p takes 3.0 against 5.3, and 4K HEVC 14.2 against 14.6. Only 10-bit ProRes gains, 24 to 27 against 14.8, and more than that gain was its 16-bit RGBA. A frame for the GPU at its full size now goes up in the planes it decoded to where that comes out right, and a kernel (`planes.wgsl`) makes it RGB. At 8 bits it goes by libswscale's own whole-number arithmetic and so to the same bytes, for 4:2:0 and 4:2:2 on 64-bit ARM at the sizes its fast path takes; deeper footage comes within 3 of 255. Other formats and sizes, alpha, other machines' 8-bit footage, and footage scaled on decode still go up as RGBA. With a grade on it, on an M1, 4K ProRes 10-bit goes from 46 ms a frame to 34, past 24 fps, 4K H.264 from 34 to 27, and 1080p from 9.1 to 7.4. So there is no crate with an exception to the workspace's `forbid`. Footage scaled on decode still converts on the CPU, and a GPU scale from its planes was built and measured and left out (`KERF_BENCH_SIZE` in `examples/upload_bench.rs`, 4K H.264 on an M1 with 8 GB, ms a frame, converting and upload and kernels and bytes back). To 1920x1080, 7.3 through libswscale (9.4 in the compositor, whose copy and threads add to its converting) against 12.4 from planes, in two passes of libswscale's bicubic with its own coefficients; to 1728x972, 7.5 against 11.2; to 3456x1944, a clip at 0.9 in a 4K sequence, 21 (24 in the compositor) against 19. So it loses in the 1080p sequence it was meant for, where such a frame takes 14 ms in all, 70 fps, and gains 2 to 5 of 39 ms only near full size. Nor is it the same picture: a mean of 1.66 of 255 from libswscale's, 1.7% of pixels off by more than 2, and 9 at worst, which `tests/gpu.rs` would refuse, so it could only be used while playing, under frame keys of its own. What it would save is CPU: libswscale spends 19 ms of core time a frame across its threads to take 3 to 5.

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
takes 31 ms a frame where the CPU took 214.

Grades stack as plain `fx` lines, with no new words. Each runs on what the line above drew, which is
byte for byte what a grade gives on a clip of a sequence holding the other. `amount` animates because a
number a look's lines only scale stays a straight line through its keys, so the keys land as they are on
each value it reaches (`Look::animates`); a number inside the look's own keyframes does not, and is refused.
A Lumetri or Levels with all its numbers at rest does not run (`FxOp::leaves_alone`), on either the CPU or the
GPU, since the way to display encoding and back can move a byte, so a grade at 0 is not there. Three grades
take 18 ms a frame at 1080p and 50 ms at 4K (138 and 521 on the CPU). Three at 0 take 11 and 22 ms: the same
bytes as a clip with none, which at 4K has a shortcut of its own (`plain_frame`, 6 ms), since `untouched`
reads the document and not the frame.

### Motion

Has: keyframes (hold, linear, easings, Bezier curves), transforms, titles with reveals by character or
word, set in variable fonts at any axis (`wght`, `wdth`, `slnt`, `opsz`, `ital`, which animate, in `glyf` or CFF2 outlines, with line height left at the font's own since none of those axes changes it by more than 0.7% in any of 50 variable faces tried), shapes whose outlines combine and offset (`fx shape.outline@1`, `offset`, on a sweep carried from
EffectCraft's `path`) with trimmed and tapered strokes, and particles worked out from their birth (`kerf-motion`, so any frame
can be drawn first), generators, 15 playful looks, and clips in depth with cameras and lights (`draw/three_d.rs`): each pixel's ray meets the clip's plane, so
perspective is true and lighting is per pixel, on the GPU where there is one (see below). Where two adjacent clips in depth cross, their run goes down pixel by pixel, nearest last, with the pixels on the line shared; a run where none cross
takes the same path as ever, and a crossing costs 3 ms more at 1080p and 5 ms more at 4K than the same two apart. A light with a `shadow` sends a ray from each pixel back to itself through the other clips,
reading each one's alpha where it passes, which is exact where a shadow map would be a sampled copy. With a `size` the light is a disc or an angle, and 32 rays go across it on one fixed spiral, each reading as much of a clip's alpha as its share of the light covers (from a summed table, or from just its rectangle on an opaque clip), so the same pixel gets the same rays on every frame and the edge has no steps. Where the four rays around the light say a clip cannot matter, or hides it all, no ray goes: on the CPU at 1080p a hard shadow takes 50 ms and a soft one 72 to 140, at 4K 200 and 290 to 570. A camera with an `aperture` blurs each clip alone by its plane's distance from `focus` at each pixel (`draw/focus.rs`,
carried from EffectCraft's progressive blur with a round iris), so a sharp clip keeps its edge over a soft one.
A clip with a `depth` is a solid with no mesh: each pixel's ray goes between the planes of its front and its back, half a picture's pixel at a time, reading the picture's alpha, and takes the color of whatever more of it it reaches, lit by the plane's normal at the front and by the slope of the alpha on a side. So a title's sides are its letters', its holes stay holes, and its edges are as soft as a pixel. What is empty of alpha is passed over by a summed table without being read. Lights fall only on the way a solid's surface faces, and its shadow is what five planes through it cast between them. On the CPU a 1080p title takes 23 ms for 14 flat, and 4K 95 for 57; with a soft shadow on a wall, 176 and 698. An environment light (`kind=environment image=V3`) is a track's picture wrapped around the scene as a panorama and reduced to how much light reaches each way a surface can face, a table of 32 by 16 made for each frame, which is all that a surface with no gloss shows of it, and such a frame is not kept, since it reads another track.

On the GPU all of that is one kernel (`kerf-gpu/src/depth.wgsl`), and the camera's blur a few more after it, doing the same arithmetic in 32 bits, with the compositor's 64-bit numbers measured from the eye so that nothing is lost by it. A kernel cannot take a varying number of textures, so what stands in a light's way goes into buffers first: its alpha for a hard shadow, its summed table for a soft one, neither for a solid color, and a track around the scene into its table of light. Clips that cross go down one below another in a tall texture and are put in order at each pixel (`by_depth.wgsl`). Held to the CPU by `tests/depth_gpu.rs` and `tests/gpu.rs`: on flat clips, crossings, hard and soft shadows, solids, and the environment light, 960x540, the mean is at most 0.001 of 255, and at most 0.013% of pixels are off by more than 2, those on a solid's sides facing straight up or down, where a table of light has no one way round; the test allows 0.05 and 0.05%. The camera's blur (`kerf-gpu/src/focus.wgsl`) comes after a clip is drawn, from the depth it writes at each pixel. Which radii it gathers at, and so what size of texture each takes, depends on what was drawn, so two kernels find the box and the radii around what has alpha and the frame stops to read those 32 bytes back, one wait to each clip. Those are `focus.rs`'s own levels, so scenes with a blur come to a mean of at most 0.001 of 255, with under 0.001% of pixels off by more than 2 and none by more than 3, on a beveled title with a soft shadow; making the kernel's radius 5% too wide fails the test at once. A level at full size is only gathered at the pixels that read it, each pixel reading the two around its radius. On an M1 with 8 GB, 1080p goes from 19 to 41 ms a frame to 11 to 23, and 4K from 72 to 154 to 43 to 75, of which drawing and reading back a frame with no blur takes 10 and 37; what it holds at its height is about 200 MB more than the CPU's, the row sums and levels being textures.

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
| Soft shadow of a title | 98 | 14 | 393 | 53 |
| Solid square | 20 | 7 | 79 | 26 |
| Solid title | 23 | 9 | 95 | 33 |
| Flat title | 14 | 6 | 57 | 23 |
| Solid title with a soft shadow | 176 | 25 | 698 | 91 |

So one clip in depth draws past 24 fps at 1080p in every row here; at 4K a hard shadow is about 24 fps, a soft one 17 to 21, and a solid title with a soft shadow 11. These are single frames of one thing with no footage, and a scene adds up: a 4K H.264 clip in depth with a beveled solid, a beveled title, a soft shadow, and an aperture over it takes 129 ms a frame on the GPU at 1080p, about 8 fps, and plays smaller, as Playing says.

A title that stays the same from frame to frame is not made again. Glyphs on the CPU turned out to be the least of it: of 263 ms for a 4K beveled title with a soft shadow, 5 go to drawing them, 40 to sending the picture up as floats, 41 to the distance field, 16 to its outlines taken in and its summed table, and 129 to the rays. So the GPU keeps the picture (133 MB at 4K, 36 ms a frame) and the field (33 MB, 41 ms) in textures of their own (`Frame::image_kept`, `kerf-gpu/src/kept.rs`), by a key saying all that goes into them, which includes the numbers a title's axes and reveal come to at that moment. What was left out does not pay for its memory: outlines taken in hold 133 MB for 5 ms, and a summed table 10 ms is all it would save. On the CPU a field takes 476 ms at 4K and is kept, by a hash of the alpha it is made from that takes 3 (`bevel::kept_distance_field`), up to four at that size; a picture is not, since copying one out costs more than the 5 ms of drawing it. Only what is asked for twice is held, so a title whose numbers move holds nothing and takes what it did (41 ms at 4K with a reveal under way, 42 before). Nothing is held past 512 MB on the GPU, three such titles at 4K, or past ten seconds of frames without it. The peak footprint of the run in `examples/motion_bench.rs` goes from 1849 MB to 1922 and from 1350 MB to 1435 with a flat title, since each kept picture stands in for an upload the pool would have held. Drawing a frame from what was kept comes to the same bytes (`a_kept_title_draws_what_it_drew`), and no kernel writes a kept picture, since each makes a new one, whichever drawn from it.

A solid's `bevel` cuts its edges back at each face at 45 degrees, with no mesh either: at each step a ray asks how far inside the picture's outline it is, against how far a bevel takes the outline in at that depth, from a distance field made by jump flooding (`draw/bevel.rs`, `distance.wgsl`, the same passes on both). So a title's bevel follows its letters and leaves its holes, and is lit as a surface facing out of the outline and toward its face. Lights fall on it as on any surface, and its shadow is the same five planes through the solid, those within a bevel of a face with their outline taken in, each its own alpha and table. On an M1, a 1080p title with a bevel takes 20 ms on the GPU and 61 on the CPU, and 4K 76 and 336; with a soft shadow on a wall, 41 and 191, and 151 and 873. Before its field was kept those were 38 and 150, 159 and 740, 63 and 375, and 236 and 1748. `bevel_profile=round` makes it a quarter of a circle in place of the 45-degree cut: how far in a ray is along the field says where on the circle it is, so the way the surface faces turns from the side at the outline to the face at the bevel's width, and it meets each without a corner. It costs what a flat one costs, a square root at each step, and flat is the same code as ever. A distance field goes by the world rather than by a picture's pixels, each pixel counted across and down at the size it is, so a clip scaled unevenly has a bevel as wide on each side. It used to go by the mean of its two scales, which, measured on a square at 2 to 0.5 where 18 pixels were asked, came to 30 pixels across and 9 down, a cut at nowhere near 45 degrees. A bevel holds to the CPU as the rest does (`tests/depth_gpu.rs`); the first measure of this turned up a difference between them along turned edges, at any scale whose size comes out so that a solid's depth is all but a whole number of pixels, since a ray's steps are a rounding up of it, which 32 and 64 bits took differently. The GPU now gets that count worked out in 64 bits.

The rule: **a composition is a sequence.** Nesting already works, so there is no
second document type, and an agent learns one model.

The picture shows and edits the curve of a selected clip's `path=`, through its parents
(`ViewerWidget::setMotionPath`, `parentMotion`; both held to the compositor by a table of values), adding a
point with a double-click and removing one with Delete. The Graph Editor, which drew a clip's keyframed numbers as curves, was taken out of the editor (`e6fe569`), so a number's keys are written as text: by an agent, or with `set`. A signal's curve is the first such number a person would want to draw, under the picture and against it.

On a clip in depth the curve and the masks are drawn through what the camera does to a plane, which one projective
map says exactly, so `picturePlacement` and `pathPlacement` return it and the viewer paints through it and inverts it to drag
(`a_clip_in_depth_lands_where_the_table_says` holds it to `Scene::at` and `Plane::of`). A mask lies on the clip's plane; a path lies square to the world at the
clip's `z`, as a clip turns about its middle. What is keyed goes by the moment on the picture, so the rest of the curve says where
the clip would be if those stayed. A plane edge on or wholly behind the camera shows nothing, and in a drag what goes past a plane's horizon, or further than `path=` can say, stays where it last was. A path it faces along (`orient`) from an anchor off the middle of a turned clip leaves the plane by a different amount at each point, which no one map says, so its curve is drawn place by place from where the clip's own placement puts its anchor (`sampledPath`). A drag on one of its points cannot go back through a map either, since how the clip is turned there depends on where that point is, and is solved for from the clip's own placement at that point, a few steps of Newton's method (`sampledPathPointFor`). It lands within a millionth of the frame, and through what `path=` can say of a point, a ten-thousandth of the frame, within 0.3 pixels at 1080p, which is what a path one map shows comes to as well. Its points are dragged; adding and removing them stay with a path one map can show.

To add:

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
average, not as strands, which is all a soft edge along an outline can say. A subject going further in a frame than two of its bands (a band being a fortieth of the frame's width) is found again first, by matching its own pixels across the frame at a few sizes (`moved_by`), and the flow then says only what changed of it, in a band up to three times as wide the further it went; below that the flow keeps up alone and nothing changes. At 640x360, a disk 120 across going back and forth over 24 frames at 64 pixels a frame overlapped the truth by 0.17 on average, lost after 5 frames, and now by 0.996, 0.995 at worst; at 128, 0.04 and 0.996; colors closer to the background's, 0.16 and 0.04 and now 0.98. At 2 bands and below it holds what it had. Moving a picture to where its subject went moves what stayed behind by as much, and a block on the subject's edge may go with that: going by such a block read the matte from where the subject had been, and dragged it out by up to 15 pixels with a band of 5, further than a cut in the band could bring back, though the shifts found were right. That was the one frame at 4 bands of 200x150 at 0.938. A block nearer to what stayed behind than to the subject now says nothing, and that frame comes to 0.992 on a mean of 0.996. The search takes 8 of 48 ms a frame at 640x360, and now runs only where the subject went fast on the frame before or the flow has lost it: how unlike what the flow says each pixel under the matte came from that pixel is, read at 2500 of them, comes to 0.01 to 0.05 where the flow keeps up, up to 0.14 turning or growing fast or under a shutter, and 0.09 to 0.7 where it has lost the subject, which is told from 0.08 up. So a slow subject takes 39 ms a frame, its bytes as they were with the search, and a fast one 53. Beyond rigid disks, on an ellipse twice as wide as tall over 24 frames at 640x360, by the overlap with what the subject covers more than half of, colors plain and then close to the background's:

| Shot | Plain | Close |
|---|---|---|
| Rigid, 64 px a frame | 0.994 | 0.961 |
| Turning 20 degrees a frame, 64 px | 0.995 | 0.967 |
| Growing and shrinking by 20% over ten frames, 64 px | 0.995 | 0.967 |
| Turning 12 and growing 20%, 64 px | 0.995 | 0.967 |
| Turning 20 and growing 30%, 128 px | 0.994 | 0.968 |
So turning and changing size cost it nothing a band cannot follow, and blur is what is left. This used to say that the outline sits inside what is half covered at the ends of a blur. Measured, it is the other way: a cut goes by color in code values, in which a little of a bright subject's light is a lot, so it takes in nearly all of the blur, out to 44% of the blur's length from the middle where the end is at 50% on a subject plainly apart from what is behind it, and the measure across the edge brings it only part of the way back. An outline with the shutter open all the frame at 64 pixels was 12% larger than what is more than half covered. That middle is the subject at the middle of the exposure, and from what the cut took in it lies back along the way each edge went, by as far at the front as at the rear, around a turn as along a straight edge (`roto/shutter.rs`). So what edges measure where they can be read past their blur gives one number for a shot, how far the middle is from the cut, and how long the shutter was open, which is the camera's; the motion from frame to frame, with its turn and change of size, says the rest; and a picture of the cut is drawn in along that motion. A mask's soft edge there is as wide as that edge went while the shutter was open, and 1.33 times that, since a mask's fall-off is an S and a blur's a straight line. Where a frame turned around, the frames either side say little and its own edges are gone by. Over 24 frames at 640x360, by the overlap with what is covered more than half of the time, by the measure alone and then with this, an ellipse unless it says otherwise:

| Shot | Plain, measure | Plain, shutter | Close, measure | Close, shutter |
|---|---|---|---|---|
| Open half the frame, 8 px | 0.977 | 0.977 | 0.946 | 0.946 |
| Open half the frame, 64 px | 0.950 | 0.958 | 0.935 | 0.954 |
| Open half the frame, 128 px | 0.851 | 0.942 | 0.880 | 0.919 |
| Open half the frame, turning 12, 64 px | 0.941 | 0.954 | 0.917 | 0.935 |
| Open half the frame, 48 across and 40 down | 0.934 | 0.958 | 0.915 | 0.942 |
| Open all the frame, 32 px | 0.938 | 0.961 | 0.929 | 0.951 |
| Open all the frame, 64 px | 0.827 | 0.941 | 0.854 | 0.919 |
| Open all the frame, 48 across and 40 down | 0.868 | 0.932 | 0.836 | 0.898 |
| Open all the frame, turning 12 and growing 20%, 48 px | 0.885 | 0.916 | 0.874 | 0.912 |
| Open all the frame, a rounded box, 64 px | 0.886 | 0.967 | 0.878 | 0.930 |
| Open all the frame, 128 px | 0.643 | 0.702 | 0.728 | 0.780 |
| Open all the frame, half the size, 64 px | 0.649 | 0.649 | 0.700 | 0.749 |

These are not the numbers this table used to give, which counted what is covered for just half of 8 moments as more than half and so put the truth a sixteenth of the blur further out at both ends; the pictures still have 8 moments, and the truth now has 33. A mask drawn from it comes nearer the true alpha as well, 0.160 off to 0.106 at the longest of these with a hard subject that can be read, and an outline's area comes within 5% of what is half covered there. It takes 39.0 ms a frame at 640x360 on a shot it applies to, from 37.4. A blur has to be a band and a half long for this to apply, below which the measure does as well or better: a disk 128 across under a blur of 12 pixels comes within 0.052 of the true alpha by the measure and 0.077 by this. Anything else stays as it was, to its bytes and its time: a still subject, one going slowly, one going fast and sharp, and one out of focus, which reads as wide whichever way its edge faces. What stays hard is a blur near as long as the subject is wide, where no pixel shows it plainly: 0.649 to 0.78 above. All of this is on synthetic shots. On a real shutter it does not come in (Real footage, below): a box truck at 56 pixels a frame at 1024x540, whose front edge says the shutter was open about 0.78 of a frame, is followed by the measure alone, overlapping what is more than half covered by 0.892 on average and 0.780 at worst over 20 frames, at 93 ms a frame on the machine it was first measured on and 0.21 s on a 4-core 2.1 GHz Xeon. Its edges facing the way it goes read 45.8 pixels of blur, past the 38.4 a shot needs, but the slope over all 40 corners of a truck with a cab and wheels says 35.4, so `fit` says no; let in (the test fits it again with no least blur), it finds the shutter open all the frame and draws no better, 0.888. A trained model is optional and swappable (`kerf_segment::MaskModel`):
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

A lens that changes during a shot is solved too, and goes in as `fov` keyframes on `gen=camera@1`. How much the picture grew from frame to frame, read from how far apart what it shows is, starts each frame's lens, and each key frame's focal length then goes wherever fits, against what lies at other depths: a zoom grows all of a picture alike, and a move along its line grows what is near more. It stands only where it leaves under four fifths of one lens's error, so a shot one lens explains comes out as it did. A camera that holds its lens, in a room filmed over 120 frames at 960x540, has the same 0.099 pixels, way off by 0.097%, turned off by 0.136 degrees, and a field of view off by 0.26; asking costs a second solve wherever the picture grows or shrinks by 0.4%, which on that shot takes 2.1 s where one lens took 0.8. Over the same way:

| Lens | Solve | Reprojection error | Way | Turn at worst | Field of view at worst |
|---|---|---|---|---|---|
| 1.2x | One lens | 0.37 px | 25.6% | 9.6 deg | 19.1 deg |
| 1.2x | Zooming | 0.14 px | 0.54% | 0.17 deg | 0.46 deg |
| 2x | One lens | 0.45 px | 68.1% | 17.8 deg | 42.1 deg |
| 2x | Zooming | 0.17 px | 0.43% | 0.24 deg | 1.25 deg |

A lens that breathes as it focuses is a small zoom, out and back, and small is where one lens does worst for what it can see: it was asked about only from 3% of growth, and over the same way one lens took 1% of breathing up in a field of view 4 degrees off with only 0.19 pixels to show for it. Asked from 0.4%, with a focal length that changes by 0.3% enough to stand:

| Breathing | Solve | Reprojection error | Way | Turn at worst | Field of view at worst |
|---|---|---|---|---|---|
| 0.25% | One lens, as it finds nothing better | 0.12 px | 0.41% | 0.23 deg | 0.75 deg |
| 0.5% | One lens | 0.14 px | 0.79% | 0.61 deg | 1.76 deg |
| 0.5% | Lens that changes | 0.11 px | 0.17% | 0.17 deg | 0.35 deg |
| 1% | One lens | 0.19 px | 1.42% | 1.50 deg | 4.02 deg |
| 1% | Lens that changes | 0.11 px | 0.25% | 0.14 deg | 0.32 deg |
| 2% | One lens | 0.25 px | 2.60% | 3.23 deg | 8.01 deg |
| 2% | Lens that changes | 0.10 px | 0.34% | 0.05 deg | 0.06 deg |
| 5% | One lens | 0.35 px | 6.06% | 6.50 deg | 13.70 deg |
| 5% | Lens that changes | 0.12 px | 0.34% | 0.08 deg | 0.12 deg |

A held lens, a zoom of 1.2x and of 2x, and a push in come out as they did. All of this is on rendered rooms: a lens's distortion, which a real one has and no solve here models, might also be taken up by a focal length that changes.

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

## Real footage

Quality numbers made on synthetic pictures were checked on real ones (`scripts/fetch-real-footage.sh`, the ignored tests `real_footage_*` in `kerf-image`, `kerf-raster-fx`, `kerf-fx` and `kerf-track`). The footage is Netflix's *Chimera*, CC BY 4.0 (`licenses/README.md`): 24 uncompressed 4K frames of a locked-off freeway that a white box truck crosses under the camera's own shutter, and two of sunlit water in a fountain, measured at their PQ code values at 4K and 2048x1080. The shot gives its own truth where one can be made: what a hole hid, for Fill; the median of the 24 frames where nothing crosses, for Remove Grain; and how far a pixel has gone from that median towards the truck's white, for what the truck covers. What came of it is with each claim: Bokeh Blur and Fill's closeness to its search held, Fill keeps less of a picture's grain on real noise than on synthetic grain, Remove Grain helps at its default and harms above it, and roto's shutter does not come in on a real truck. Not tried: film grain, as the bucket's grained *Nocturne* is empty, and a glow's bloom, which no claim here measures.

## The agent

The agent still gets four tools and a document it can read. Each area adds
vocabulary to the language and numbers to `look`, and no new tool: scopes as
measurements, tracks as data, and check findings for what a viewer would notice.

`look` with `contrast` reads each title and caption against what is behind it, from three frames at each
of three moments it has come in by: as drawn, without it, and its glyphs alone, white on nothing, so that a
title the colour of its background is seen not to show. Each patch about half a line across goes by the
strongest of its ink, so a caption's outline counts and a title half over a highlight is caught, and
it is flagged below WCAG's 3:1 over a fifth of it. It draws frames, so it is asked for and not part of the fast check; the full QC has it as a row, Text contrast, after the eight of the Finish tray's mockup, in the last tenth of its progress, and it adds 0.36 s a clip at 1080p, for at most 40 clips spread over the sequence. The same frames say what the frame cuts off: Clipped text, the row after it, is a text whose box goes past an edge with its glyphs reaching that edge, so a box wider than its ink says nothing and text moved wholly off the frame has nothing left to cut; `look` gives those as `clipped`. Each of the two it used to miss takes one more frame of glyphs, about 0.04 s at 1080p against those 0.36 s, for only the clips it is about, so both are reported: a text whose box goes past an edge only between the three moments is read once more where it goes furthest with some of it left in the frame, its boxes costing no frames to work out; and a crop that does not move is read once with the crop off, and is said if it takes off any glyph. A crop that moves is a reveal, and a mask is drawn on purpose in a way a crop on a title seldom is, so those are taken as meant.

An agent starts with nothing to go on but the reference, so what it is told has to carry it. `look` on a fresh folder says in `unplaced` what footage is there and not in the document, and `look` with `file` gives the `media add` line that adds one, measured from the file. A refused line says what to do: which tracks there are and the `track add` that makes the one it asked for, the name a slip of a letter or two was for, and, where a clip or a media exists only in a pending proposal, that proposal's id and that `extend` builds on it. An `extend` that names a version since replaced is refused with the newest version's id, since built on the old one it would leave out what was added after and become the newest all the same. So do `look` and `render` where a pending proposal has what the project does not. Set where nothing reads it, such as a `bevel` with no `depth` or a `focus` with no `aperture`, a value draws nothing and fails nothing, so the check says so, as information that fails no row (`no_effect`). And `look` with `cost` and `at` says who draws a frame, which clip and effect send it to the CPU, and what it costs at full size and at half, against a frame's share of the time. `effects` looks through generators' parameters and clip properties too, since nothing else would lead from "shadow" to `gen=light@1` or from "bevel" to `bevel=`.

`look` with `shows` and `at` reads a frame as an eye would, for a scene whose text does not show what is wrong with it: of each clip that draws, where it is on the frame, how much of it other clips hide and which, the edges it reaches, and whether any of it is in the frame at all (What directs a model). It is asked for and not part of the check, since it draws a small frame for each clip.

`look` with `media` and `facts` gives a fact sheet beside the index: what the camera does in each shot, from the one move, turn, and zoom that
most of the picture agrees on between small frames four times a second (`kerf_track::shot`, so a subject crossing the frame is left out), and R128 loudness.
60 seconds of 1080p takes 7 s the first time, and it is kept with the media's other derivatives. It reads a push in as a zoom and a move sideways as a pan, and only `track` with `camera` tells those apart. It says in `notMeasured` what it
leaves out, since silence would read as a finding: faces and the subject, which take a trained model, and on-screen text, which without one cannot be found reliably.

That loudness is the media's own. `look` with `loudness` gives the sequence's, as it plays: its whole mix through the meter a delivery is checked with (`kerf_engine::measure_program`), as integrated loudness beside the target, loudness range, true peak beside its limit, and the loudest three seconds and 400 ms with where each ends. It is measured once for a revision, of the project or of a pending proposal, so a gain or a mix can be read back before it is accepted.

## Before shipping

Surveyed on 2026-11-08 at `3a175ca`. Each line names what shows it.

**Packaging.** Done: a Windows zip that runs on its own, checked by running it with nothing else on PATH, and its GPL source (`scripts/package-kerf-windows.ps1`, `.github/workflows/package.yml`), and a macOS app that carries Qt, FFmpeg and every Homebrew library it loads, checked to load nothing from outside itself and run with nothing of Homebrew's on PATH, with the source of each formula it carries (`scripts/package-kerf-macos.sh`, `collect-kerf-macos-sources.sh`). A published release is checked as someone downloads it: `verify-release.yml` fetches its Windows zip and macOS app onto fresh runners with no FFmpeg or Qt, on macOS with Homebrew moved out of reach, unpacks them, and runs `check-kerf-windows.ps1` and `check-kerf-macos.sh`, the checks the packaging scripts run on what they pack: ffmpeg, ffprobe, `kerf --version`, `kerfd --endpoint` and the editor's start window, with nothing else on PATH. Since a runner's System32 has the C++ runtime that a package might leave out, and a build machine names its Qt's plugins in `QT_PLUGIN_PATH` (install-qt-action sets it, and the checks now clear it), the Windows check lists each library the running editor loaded, which has to be the package's or Windows' own; on macOS dyld names each library loaded (`DYLD_PRINT_LIBRARIES`), which has to be inside the app or macOS's. It also holds that each package has its sha256 and source beside it, and that the checks fail a package with a library taken out. The Package workflow runs it on each release it publishes, and by hand it takes a tag. Fixed here: the Windows package was checked on the machine that built it with `QT_PLUGIN_PATH` naming that machine's Qt, so the packaged editor loaded its platform and style plugins from there and not from the package; the zip itself carries both and, on a fresh runner, loads them (v0.1.0, run 42586335808), so the check was what was wrong. And kerfd, kerf and the editor looked for ffmpeg and ffprobe on `PATH` and in Homebrew, not beside themselves, so on a Mac without Homebrew's FFmpeg the app's own went unused and media imported without a probe, at a made-up size, rate and length; Windows was spared only because a Windows program looks in its own folder first. They now look beside themselves after `KERF_FFMPEG` (`kerf-engine/src/ffmpeg.rs`), the checks have the package's kerfd import a clip its own ffmpeg made and read it back at 320x240, 25 fps and 2 s, and 0.1.1 carries the fix. Verify release on v0.1.0 fails that check on macOS. And the macOS app kept rpaths into Homebrew (the editor's into Homebrew's Qt, two libraries' into the Cellar) and Qt's plugins one to a `lib` folder beside the app, where dyld would look for an `@rpath` library before the app's own; only rpaths inside the app stay now. The Windows release also has an installer, from the same folder (`apps/kerf-qt/packaging/windows/kerf.iss`, Inno Setup): per user, with no administrator, into `%LOCALAPPDATA%\Programs\Industrial Kerf`, with a Start menu entry, `kerf://` registered under `HKCU\Software\Classes` so a review link opens the editor, and an uninstaller. `check-kerf-windows-installer.ps1` installs it silently on the runner, runs the installed copy through `check-kerf-windows.ps1`, finds the Start menu entry and the scheme as Explorer resolves it (`AssocQueryString`), follows a `kerf://` link through the shell and finds the installed editor started with it, then uninstalls and finds each of those gone; the Package workflow runs it on what it builds and Verify release on a published one. Not signed, so SmartScreen asks before it runs. Windows starts an editor for each followed link, as it does any program a link opens, and on Windows and Linux one started for links alone hands them to the editor already open and ends, so the link raises that editor as macOS's does; with none open, or none answering within two seconds, it opens them itself (`link_inbox.cpp`, `link_inbox_test.cpp`). The inbox is a local socket only the user's own processes can reach (`QLocalServer::UserAccessOption`), named by an id the user's settings keep, so another user can neither work the name out nor take it first, and on Linux it is in the user's own runtime folder; the one handing over lets the other come to the front (`AllowSetForegroundWindow`). Qt Network is linked on Windows and Linux only for it; macOS gets the link in the running app. Linux has a package too, a folder packed as a `.tar.xz` (`scripts/package-kerf-linux.sh`, built on Ubuntu 24.04 with the Qt and the BtbN FFmpeg the other two use): the programs in `bin/`, and in `lib/` each library they load in turn that is not one the system has to provide (`apps/kerf-qt/packaging/linux/system-libraries.txt`: glibc, libstdc++, X11 and libxcb, OpenGL, fontconfig, D-Bus, GLib, ALSA, Kerberos, OpenSSL), with an RPATH of `$ORIGIN/../lib`, which comes before `LD_LIBRARY_PATH`. `check-kerf-linux.sh` holds every library a binary names to be carried or on that list, runs the tools, the kerfd import and the editor's start window on Xvfb with nothing on `PATH` and no library or Qt plugin path, and fails if `LD_DEBUG` shows Qt, a Qt plugin, ICU, FFmpeg or anything the package carries loaded from outside it. Verify release runs it on a fresh runner and in a Debian 13 container, each with only that list installed. Fixed here: the fresh runner was given Xvfb alone, and its image has no `libOpenGL.so.0` or `libEGL.so.1`, which every desktop has and the list names, so v0.1.1's first verification failed there with the package itself sound (its build machine and the Debian run have them). Its source adds the source of each Ubuntu package it carries a library of (`collect-kerf-linux-sources.sh`). It carries the xcb platform only (Wayland desktops run it through XWayland), and runs only where glibc and libstdc++ are as new as Ubuntu 24.04's. A folder registers nothing of itself, so it carries `install-desktop-entry.sh` (`apps/kerf-qt/packaging/linux/`), which for the user alone writes a desktop entry that runs the folder's editor with a link and makes it the default for `x-scheme-handler/kerf` in `mimeapps.list`; `check-kerf-linux.sh` runs it in a scratch home, has GLib's `gio` name Kerf for `kerf://` and follow a link to the editor, and removes it, and a release from before it is checked without it. What Kerf runs on, by decision: Macs with Apple silicon, Windows x64 and Linux x86-64. Intel Macs are left out on purpose: Apple has sold none since 2023 and names macOS 26 the last macOS for them, an editor leans on Metal and VideoToolbox, where they are weakest, and it keeps one macOS build. Windows 11 on Arm can run the x64 build through Windows' emulation, and the installer lets it (`ArchitecturesAllowed=x64compatible`; Windows 10 on Arm runs no x64 program and is refused); no Arm machine has tried it. The macOS app needs macOS 26, also by decision: read from the v0.1.1 app's load commands (`minos`), the editor, FFmpeg and the other Homebrew libraries it carries, bottled for the runner's macOS 26, say 26.0, Homebrew's Qt frameworks 14.4, and the Rust programs 11.0. Going lower would take building it on an older macOS's runner, or the libraries from source, and is not worth it. Fixed here: `Info.plist` named no `LSMinimumSystemVersion`, so an older macOS refused v0.1.1 without saying why; `package-kerf-macos.sh` now writes 26.0 there (only the package, so an editor built on a developer's older Mac still opens), and `check-kerf-macos.sh` fails when any Mach-O file in the app needs a newer macOS than it names, as one built on a newer runner would, which the Package workflow shows on a copy that says 11.0. Kerf has an icon of its own, drawn once in `apps/kerf-qt/resources/icon/industrial-kerf.svg` and rendered by `tools/kerf-icon` to the PNG, `.ico` and `.icns` files beside it, which are in the repository so that no build needs a renderer; `cargo test -p kerf-icon` renders it again and fails when a file has drifted from the SVG. The editor sets it on its windows (`ui/app_icon.cpp`, `QApplication::setWindowIcon`; not on macOS, where the Dock shows the app's own); `kerf-qt.exe` carries the `.ico` as a resource (`kerf-qt.rc`), from which Explorer, the taskbar, the Start menu entry and `kerf://` links take it, and the installer wears it (`SetupIconFile`); the macOS app has the `.icns` in `Resources`, named by `CFBundleIconFile`, with its tile at macOS's 824 of 1024; and the Linux package carries the PNGs in `icons/`, which `install-desktop-entry.sh` installs into the user's hicolor theme for the entry's `Icon=industrial-kerf` and `--remove` takes out. The checks hold each: `check-kerf-windows.ps1` reads the first icon group in `kerf-qt.exe` and wants 16, 32, 48 and 256 pixels in it, `check-kerf-windows-installer.ps1` that links take their icon from the installed editor and that the installer's is the editor's, `check-kerf-macos.sh` that the file `Info.plist` names is in `Resources` and unpacks with `iconutil` to every size, which the Package workflow shows on a copy without it, and `check-kerf-linux.sh` that each size is found in the scratch home as a PNG of that size and gone after `--remove`. A release from before 0.1.2 has no icon and is checked without it. Its clip holds five photographs, all CC0 (`apps/kerf-qt/resources/icon/README.md`). Nobody has yet looked at it in a real Dock, taskbar or application menu. Linux has a `.deb` too, for Debian 13, Ubuntu 24.04 and what comes from them, made from the same folder (`scripts/package-kerf-linux-deb.sh`): the folder in `/opt/industrial-kerf`, the desktop entry in `/usr/share/applications` with `kerf://` in its `MimeType` (`packaging/linux/industrial-kerf.desktop`, held to the one `install-desktop-entry.sh` writes), the icon in the system's hicolor theme, and `kerf-qt`, `kerf`, `kerfd` and `kerf-mcp` on `PATH` as links into it, each finding the rest beside the file it really is; ffmpeg and ffprobe stay off `PATH`, where the distribution's own may be. It has no maintainer scripts: the desktop's own triggers read the entry and the icons. Its Depends are not written by hand: `dpkg-shlibdeps` reads out of the binaries which of the build machine's packages provide each library they name and the package does not carry, with the versions their symbols ask for, so apt brings what a bare system lacks and refuses one whose glibc is older than 2.39 rather than leave the editor not to start; OpenSSL and the Vulkan loader, loaded only when first used, are Recommends. `check-kerf-linux-deb.sh` installs it with apt, without the Recommends, finds every library its binaries name before anything else is installed, runs `check-kerf-linux.sh` on the installed folder, has kerfd started through its link import a clip and the editor through its link open its start window, finds the entry, the icon at each size and GLib naming Kerf for `kerf://` for a user who has set nothing, follows a link to the installed editor, and removes it and finds no file of it, nor its claim on `kerf://`, left. The Package workflow runs that in bare `ubuntu:24.04` and `debian:trixie` containers on what it builds, and shows it failing a `.deb` with `libopengl0` taken out of its Depends; Verify release runs it on a published one. Found here: FFmpeg's libavcodec, as BtbN builds it, has glibc's resolver built in and names nine of glibc's private symbols (`__resolv_context_get`, `__res_iclose`, `errno` and others; read from v0.1.1's), for which `dpkg-shlibdeps` allows the build machine's glibc alone, so the `.deb` keeps only the oldest glibc it asks for. The package runs on Debian 13's glibc 2.41, but a glibc that drops one of those would stop the folder and the `.deb` alike from loading FFmpeg, and nothing but a run on it would say so. Nobody has yet installed it from a desktop's software centre. Missing: an apt repository, so a newer Kerf comes as the download the editor's notice points to and not through `apt upgrade`; an rpm; and an AppImage.

**Signing.** Missing on both. macOS gets an ad-hoc signature (`codesign --sign -` in `install-kerf-local.sh`), with no Developer ID, hardened runtime, entitlements or notarization, so Gatekeeper refuses a downloaded copy; nothing signs the Windows zip (no `signtool`).

**Versions and updates.** Done: one version, the workspace `Cargo.toml`'s, which the editor's CMake project repeats (`the_editor_has_the_workspaces_version` holds them the same) and Help ▸ About Kerf shows; `CHANGELOG.md`; and a `v<version>` tag publishes the Windows zip, the macOS app and the source of each as a GitHub release, refusing a tag that is not the version or has no section in the changelog (`package.yml`). Releases are not signed: Windows SmartScreen and macOS Gatekeeper ask before the first run. Once a day the editor asks GitHub's releases whether a newer one is out and offers its page, Download, Skip This Version or Later; it never downloads or installs anything itself, and Settings ▸ Updates turns it off (`kerf_update_check`, `update_notice.cpp`, `update_notice_test.cpp`). It trusts the system's certificates, so a network that inspects TLS with its own CA still works. Missing: an update that installs itself.

**CI.** Done: Linux runs the workspace, the Qt UI tests and the bridge workflow; Windows the kernel, runtime, sidecars and Qt UI; macOS builds and runs the Qt UI and the bridge workflow (`rust-runtime.yml`). The parts of a system that do not wait on each other are jobs of their own, side by side: Linux's tests, clippy and editor, macOS's editor and workspace. macOS now runs `cargo test --workspace` too, beside the editor's job, on a runner with Apple's paravirtual Metal GPU, which `kerf-gpu` takes: the GPU parity tests run there, outside a developer's Mac, and passed with the rest (1637 tests, first run on #203); Linux has no GPU (it sets `WGPU_BACKEND=vulkan` so they skip) and runs only the catalog's list on llvmpipe (`KERF_GPU=software`). Unexplained: the Windows job runs the bridge's connection test five times because "a crash once came and went". It did not come back in the 100 runs from 2026-11-06 to 2026-11-08, 500 runs of the test, so there is no failure to read and its cause is still unknown; the five runs stay, to catch it. Every other red run in those 100 has a cause: two 504s from GitHub for the FFmpeg archive, Rust 1.99's new clippy lint, and an MCP test that took a zombie for a running process (`5338435`). Fixed here: main went red when GitHub answered 504 for the pinned FFmpeg archive on Linux and Windows in the same minute, so the downloads retry, and since GitHub's release downloads answered 504 for an hour on 2026-11-09 the pinned FFmpeg archives are kept in the Actions cache and the macOS source collection retries each Homebrew source for minutes; and Rust 1.99's clippy added `chunks_exact_to_as_chunks`, which failed the new lint step until the 107 sites moved to `as_chunks`. `dtolnay/rust-toolchain@stable` picks up such lints the day they ship. Fixed here: runs on main each finished, so six pull requests merged in a row on 2026-11-09 left the last one's checks queued for half an hour behind the first's; a newer push now cancels the runs still going for the one before it, on main as on a pull request, since main's newest commit holds all merged before it. A release, from a tag or run by hand, has a concurrency group of its own in `package.yml` and is never cancelled.

**Crashes.** Done: a panic in kerfd, the editor's Rust or the `kerf` CLI leaves a plain-text report, with the component, version, OS, thread, message, where, and a backtrace, in the crash folder (`%LOCALAPPDATA%\Kerf\crashes`, `~/Library/Logs/Kerf`, or `$XDG_STATE_HOME/kerf/crashes`; `KERF_CRASH_DIR` moves it), which keeps the newest 50 (`kerf-runtime/src/crash.rs`, `tests/crash.rs`). A panic `catch_unwind` turns into an error is reported too. The next time the editor opens it says what stopped and offers the folder, once for each report (`crash_notice.cpp`, `crash_notice_test.cpp`). The release profile now unwinds, so the `catch_unwind` that turns a failed editor task into an error (`kerf-qt-bridge/src/tasks.rs`) and the one that keeps kerfd's media queue going past a bad file (`kerf-runtime/src/kernel.rs`) work in a release build, where `panic = "abort"` took the editor or kerfd down; and it strips only debug info, not symbols, so a backtrace names functions (`release_builds_unwind_and_keep_symbols` holds both). A crash that is not a panic leaves the same report: a bad memory access or an abort in the editor's C++, Qt, or FFmpeg inside kerfd, caught by a signal handler on macOS and Linux and by an unhandled-exception filter and the C runtime's `SIGABRT` on Windows (`crash/native.rs`). The handler allocates nothing and takes no lock, writes one report per process, and then lets the signal go on, so the process still dies of it and the system's own crash reporter sees it; an abort right after a panic's report adds none. Its backtrace is glibc's `backtrace` on Linux, the interrupted code's frame pointers on macOS, and `RtlCaptureStackBackTrace` on Windows, each frame a module and an offset where the module exports no name for it (`addr2line`, or `atos` on macOS, turns one into a function, since release builds keep their symbols). Qt's fatal messages, such as a platform plugin that would not load, end in an abort that on Windows is a fail-fast no handler sees, so the editor reports the message before it (`reportFatalMessages`, `kerf_crash_fatal`). `tests/native_crash.rs` crashes a child process each way and reads its report back, and `crash_notice_test.cpp` crashes a child of the editor's test binary in its C++ and with `qFatal`, and the notice tells of each. Missing: nothing sends a report anywhere, and a crash that ends the process from outside it (the kernel's out-of-memory killer, `kill -9`) leaves none. `kerfd`'s own stderr also survives, in a temporary file (`kerf-runtime/src/ipc.rs`, `spawn_daemon`).

**Settings and projects.** Done: the editor's settings carry a version, `settings/version`, and `migrateSettings` brings an older Kerf's up to it once at start, before anything reads them, and leaves a newer Kerf's alone (`settings_migration.cpp`, `settings_migration_test.cpp`). Version 1 drops the nine keys no code reads any more, and the model the editor wrote into every save before `c58bca5` (`claude-sonnet-4-20250514`, or `claude-3-5-sonnet-latest` from its menu), which held a person who never chose one to a retired model; a model someone chose stays. The Deliver panel's keys from before W3 are still read in place (`render_queue_view.cpp`), since they name a folder and not the path a migration would need. API keys live in the system keychain, macOS Keychain, Windows Credential Manager, or the Secret Service on Linux (`kerf_secret`, `kerf-qt-bridge/src/secrets.rs`, through the `keyring` crate), and one left in plain QSettings moves there the next time the editor loads it; where the keychain cannot be reached, or refuses a key, they stay in QSettings as before (`secret_store.cpp`, `secret_store_test.cpp`). The Kerf Cloud token is kept there too (`cloud_session.rs`): one left in its 0600 file by an older Kerf moves into the keychain the first time the editor reads it, and where no keychain can be reached it stays in the file, as before (`docs/cloud.md`). Those tests take both ways with a keychain in memory and with `KERF_SECRETS=off`, and never touch a real one; `a_key_round_trips_through_the_real_keychain` and `real_keychain_keeps_the_token_out_of_the_file` do, and CI runs them on each system's own store: the login keychain on macOS, Credential Manager on Windows, and gnome-keyring's Secret Service on Linux (`rust-runtime.yml`, Test the system keychain). Credential Manager takes no value over 1280 UTF-16 units, so on Windows a key that long stays in QSettings and a token that long in the file. Fixed here: a key changed while the keychain refused it (locked, or a prompt denied) went to QSettings with the older one still in the keychain, and the next start read the older one and dropped the new; a key in QSettings is now taken first, since only a refused or an older Kerf's key is there (`aKeyTheKeychainRefusedWinsOverTheOneKeptBefore`). Fixed here: Windows sets no `HOME`, so the editor found no `cloud.json` there and Kerf Cloud said it was unavailable; it now takes `USERPROFILE`. A project is `kerf 1` and any other number is refused (`kerf-lang/src/doc.rs`), which is right until the first change to the format, and that change needs a reader of the old one. Fixed here: the editor offered retired Anthropic models and wrote one into new settings, so it overrode kerf-agent's `claude-opus-5-5` (`comments_test.cpp`).

**Engine.** Done, and measured: the GPU path and its parity with the CPU (The engine, Kernels). Missing: effects that still send a frame to the CPU: 39 of the 206 video effects, generators and transitions at their defaults (`kerf-compose/tests/catalog_cost.rs` lists them with why). They were 101 of 262 until the 38 that make a look a model makes better, or do what a clip's own properties do, were cut (What these rules cut), and 45 of 224 until the second cut took 15 more, six of them among those with no kernel (What is looked at again). None of the 29 looks and grades reaches one, and of the transitions only Morph Cut does, which the test holds; nothing in `skills/`, the presets or the agent's reference applies one either. So no use in the repository says which to give a kernel first, and they go by what an editor needs exact and who writes it: Corner Pin, which tracking writes, then Drop Shadow, Rounded Crop, Spacer, and Mosaic, then Curves, Hue/Saturation, Selective Color, Photo Filter, Unmult, and the Cineon Converter, then Lens Distortion and Magnify, then Gradient, Ramp, 4-Color Gradient, Checkerboard, and Grid have theirs, and they were 63 before those (The engine). `KERF_GPU=software` lets Mesa's llvmpipe stand in for a GPU, to check kernels where there is none; on it 378 of the 379 tests in `kerf-gpu`, `kerf-compose`, `kerf-engine` and `kerf-kernel` pass, and `every_scene_draws_as_the_cpu_draws_it` fails only on `color.basic`, every pixel within 2 of 255 but 0.9 off on average against the 0.6 it allows, from llvmpipe's `pow` and `exp`. CI's Linux job runs the catalog's list on llvmpipe, but not the parity tests, which it could only pass by loosening that one; and no timing comes from it. The quality numbers for roto under blur, `image.fill`, Bokeh Blur and Remove Grain are now checked on real footage too (Real footage); real film grain is still untried. A title turns its font's OpenType features on and off by tag, `features="tnum ss01 -liga salt=2"`: tabular figures, a slashed zero, fractions, small capitals where the font has them, stylistic sets and alternates, over what the font has on by itself (`kerf_model::parse_features`, `TextStyle::features`, which harfrust shapes by). A title that names what is no feature is refused when the document reads; `look` with `font` names the features each face has; the check measures a title with them, since tabular figures widen a row of Inter's 1s by more than half (`a_title_is_measured_with_its_features`), and says as information when the font has not the one asked for (`no_effect`). Captions, and text set by an effect, take none yet, and the FFmpeg graph draws no title that names any, as it draws none that sets an axis. The editor was not run with it: its Inspector shows a generator's text parameters as a line of text, and nobody has seen this one there. Text still has no colour glyphs, hinting or hyphenation (`kerf-text/README.md`).

**Delivery.** Done: a blocking note holds every delivery back, Deliver, the toolbar's Export and an agent's `render` alike, since kerfd refuses a `full` or `master` render while one is open (`review::delivery_held`; `a_blocking_note_holds_a_delivery_back_and_not_a_draft`, `exportWaitsForBlockingNotesAsDeliverDoes`). A QC error that no comment mirrors, past the bridge's cap of new QC comments, holds back only Deliver.

**Kerf Cloud.** The service is not deployed; the editor follows its contract (`docs/cloud.md`).

**User docs.** Done: `docs/user-guide.md` is for someone editing with Kerf, from a first project to delivery and what to do when something goes wrong, and a Help menu opens it (Kerf User Guide) and the crash folder (Show Crash Reports; `kerf-commands`, `everyEditorCommandIsWired`). The packages carry it as one HTML page, with its style inside and nothing to fetch: `tools/kerf-guide` renders it when the Windows zip and the macOS app are made, beside `kerf-qt.exe` and in the app's `Resources`, its links to the rest of the repository going to GitHub at the package's commit, and Help opens that copy, or the guide on GitHub where there is none, as in a build from the repository (`user_guide.cpp`, `user_guide_test.cpp`; `kerf-guide`'s tests render the guide itself and fail on anything left as Markdown or anything the page would fetch). Fixed here: `apps/kerf-qt/README.md` described a Window ▸ Inspector Follows Selection command and a saved layout version that the editor no longer has.

Since the survey: Export follows Deliver, the quality numbers are checked on real footage, the catalog's use of the CPU is listed, and crash reports, settings versions, the keychain, macOS tests in CI, the user guide, the Windows zip and the macOS app, and releases that the editor offers once a day are in. Of what is left only signing and notarizing need accounts and keys this repository does not hold; the rest is under Packaging.

## What others do, and what to take

- **Resolve 21** added a Photo page, and Fusion effects in the Edit page's
  keyframe and curve editors: where one model pays off.
- **Final Cut Pro and Motion** keep two apps but share Magnetic Mask, and added
  Auto Mask. On-device matting is now expected, not special.
- **After Effects 26** has parametric meshes, depth of field in its 3D renderer,
  variable-font animation, and a faster mask tracker with Object Matte.
- **Adobe's Sneaks at MAX 2026** (11 November, as reported; not products) showed
  directing a generated clip without a prompt. Project On Cue times what
  happens in one by a signal, drawn by hand or taken from a video or a piece
  of music; Project Live Set changes the subject, the setting, or the camera
  while the clip plays. The first is what a signal is here (What directs a
  model), with the curve kept in the document; the second wants a making
  that streams, which is last on that list.
- **[NKD VFX Tools](https://github.com/Nekodificador/ComfyUI-NKD-VFX-Tools)**
  (GPL-3.0, so none of its code comes here) are ComfyUI nodes that direct a
  model with a compositor's tools: a camera solved from a photograph, a 3D
  view to place a model in, relighting from depth and normals. Its premise is
  this document's: block the shot first, and let the model draw it.
- **ComfyUI's 3D** (0.45, read from the nodes a server lists, not from its
  code) is some 126 nodes: making a mesh from a picture or from words
  (Trellis 2, Hunyuan3D, and hosted Tripo, Meshy and Rodin), rigging and
  animating one, rebuilding a scene from a picture (MoGe, Depth Anything 3)
  or as splats, mending and texturing meshes, and drawing them: a view of a
  model that gives its picture, mask, normals and camera, and an orbit. What
  is fine there is the assets and the models that make them. The drawing is
  a plain one, since a model covers it. Kerf takes the assets, as files a
  recipe makes, and builds none of the models (What directs a model, Next
  2).
- **Jumper and the Resolve and Premiere MCP servers** bolt agents onto editors
  that were not built for them. Kerf's document, proposals, and Git merging are
  what they cannot have, and each area has to keep that: every feature is text in
  `project.kerf`.
- **[raea](https://github.com/shinyobjectz/raea)** (reviewed at `925bc03`) renders
  React and TypeScript compositions with wgpu and Bevy. It is a composition engine,
  with no timeline document or proposals, so it is not a base for Kerf. **Its
  license, FSL-1.1-ALv2, forbids a competing commercial product, and Kerf Cloud is a
  paid tier, so none of its code comes here;** each version turns Apache-2.0 two
  years after release. One idea of it was worth doing our own way, one kernel source for the CPU and WGSL, which is measured and begun under Kernels. Decoded IOSurfaces as wgpu textures were measured, and cost more than they save here; uploading decoded planes got more than that (see Depth and color). The fact sheet and the contrast check are in `look`.
- **[vork](https://github.com/voyager-so/vork)** (reviewed at `3389022`, 0.5.0) is a headless 3D
  renderer for agents: scene JSON in, a picture, a movie, or control passes (depth, normals, object
  ids, motion, the camera) out, in Rust and Slang on wgpu, with no timeline and no editor. It is
  what Kerf does not build, a guide renderer with real geometry, and a recipe's `run` reaches it
  ([shots.md](shots.md#guides)). **Its license (vork Preview License) lets anyone use it, and
  forbids distributing a modified version, selling it, and offering it as a hosted service that
  renders what others submit, so none of its code comes here, Kerf does not bundle it, and Kerf
  Cloud could not run it without its makers' leave;** it covers that version only, and it draws
  only on Apple silicon and NVIDIA cards with ray tracing. So Kerf names no renderer: a recipe
  runs whichever is installed. Four of its decisions bear on Kerf's own engine:
  - *A scene and how well it is drawn are apart* (`std/previs`, `draft`, `final`, `path`). Kerf's
    version of that is a shot's guide and its making: the same recipe, with a model as the last
    and dearest way of drawing it.
  - *One implementation.* Its CPU rendering is the same GPU code on Mesa's software Vulkan, not a
    second renderer. Kerf writes each kernel twice, and one effect with no kernel sends a whole
    frame to the CPU. `KERF_GPU=software` already passes 378 of 379 tests on llvmpipe, so the
    CPU's code could become what tests hold kernels to and stop being a way to draw a frame; that
    is not done, and is the largest thing left to decide about the engine.
  - *Frames stay on the GPU*, encoded there where the platform can, with reading pixels back as
    the fallback. Kerf reads every frame back, for a `QImage` the editor paints. Showing a frame
    without that trip needs a surface made from the editor's window, in the bridge, which already
    holds the unsafe code of its C interface (the workspace's `unsafe_code = "forbid"` is taken by
    the 16 crates that ask for it, and neither the bridge nor `kerf-playback` is one). It has not
    been built or measured.
  - *What comes out is a contract*: channels by name, and a sidecar saying what each frame's
    camera was. A request to a model is Kerf's (`recipe/model.rs`).
