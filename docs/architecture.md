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
adjustment layers; `color.basic`, `blur.gaussian`, the 29 kinds of operation in
`FxOp` with Lumetri among them, and Wonder Glow, Glint, RGB Split, Wave Warp, Light Leaks, Add Grain, and
Noise, Color, Luma, and Ultra Keys with their mattes cleaned up, Logo Cutout, Extract, and Radial Zoom Blur, and a Lumetri's curves, LUTs, HSL Secondary, and sharpening. So all 15 playful looks and all
18 grades stay on the GPU, including the transitions that are looks (Whip, Flash Cut, Zoom Through), since a look
transition is layers rather than a mix.
**What sends a frame to the CPU:** Morph Cut, whose optical flow would need both clips read back; every other
effect (other lights, stylize, and the other kept EffectCraft ones); a Lumetri with an HSL Secondary's Denoise, a median at each pixel, or a LUT with a 1D shaper; Add Grain with a Softness, whose planes are blurred, or showing its matte; and in HDR, a Lumetri using a LUT, curves, sharpening, or an HSL Secondary, none of which any grade does. In HDR, so does an effect that
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
   | Light Leaks | 62 | 21 | 239 | 64 |
   | Lumetri, luma curve | 54 | 14 | 192 | 31 |
   | Lumetri, hue vs sat | 69 | 14 | 239 | 31 |
   | Lumetri, LUT of 33 | 63 | 14 | 227 | 32 |
   | Lumetri, sharpening | 55 | 17 | 228 | 55 |
   | Lumetri, HSL Secondary | 75 | 15 | 273 | 37 |
   | Lumetri, HSL Secondary with Blur | 87 | 24 | 309 | 71 |
   | Ultra Key, Choke | 56 | 17 | 188 | 54 |
   | Ultra Key, Soften | 59 | 17 | 222 | 69 |

   What was measured and left, on the CPU, in the same terms: no look or grade reaches any of it, and a frame reaching them is a person's choice of a stylized picture rather than a frame of a grade, so kernels for them would be written for few frames.

   | Effect | 1080p CPU | 4K CPU |
   |---|---|---|
   | Brush Strokes | 142 | 802 |
   | Echo Glow | 99 | 572 |
   | Volumetric Rays | 93 | 167 |
   | Edge Glow | 75 | 353 |
   | Roughen Edges | 66 | 305 |
   | Color Emboss | 40 | 135 |
   | Emboss | 31 | 105 |
   | Alpha Glow | 33 | 127 |
   | Find Edges | 23 | 69 |
   | Replicate | 21 | 65 |
   | Mosaic | 15 | 43 |
   | Lumetri, HSL Secondary with Denoise | 89 | 502 |

   Three effects cost seconds a frame or close to it, at their defaults, and no look or grade reaches them. Each stayed on the CPU and got a better algorithm, as the numbers below decided. With the same bench, median frame in ms, and for the fills a box as wide as its share of the frame:

   | Effect | 1080p before | 1080p | 4K before | 4K | Peak memory at 4K, before and after |
   |---|---|---|---|---|---|
   | Bokeh Blur, default amount 10 | 427 | 31 | 6251 | 127 | 1.10 GB, 1.03 GB |
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
   the brightness it has after the effect, on the CPU and the GPU, and the whole picture is lifted as well as scaled, which a filter over its pixels undoes exactly. `time.echo` adds up what it is given as it is, below 0 and above 1, since adding and weighing need nothing brought under 1, and its Screen operator goes as ever between 0 and SDR white and adds what is past either, where it used to stop at white and multiply what was below 0. Light from `light.echoglow` and `light.glint` adds to a color past BT.709 and leaves how far it stands past the rest, white light by the same in each channel. What changes a pixel's channels apart (`stylize.posterize`, `color.autocolor`, an exposure that saturates) keeps the gamut but moves the hue of what lies far past it. ProRes and HEVC Main 10 carry HDR; H.264 is
   8-bit. `look` with `scopes` reads an HDR sequence by its PQ or HLG signal, with what that comes to in cd/m² and how much of it BT.709 cannot hold; so does the Finish tray, whose bridge draws that signal small again on a thread of its own while the scopes are in sight, since playback hands over only the 8-bit frame a monitor gets, and marks where SDR white falls. An HDR10 file (`hevc_hdr`) says what it was mastered on
   (P3 D65 at 1000 nits unless told otherwise) and its MaxCLL and MaxFALL, measured from the frames
   before encoding, which disk-kept frames make cheap; HLG carries its tags without them. Holding an effect costs GPU passes, measured with `examples/frame_bench.rs` on 4K ProRes in PQ on an M1 with 8 GB, ms a frame, decode included:

   | Scene | 1080p SDR | PQ | HLG | 4K SDR | PQ | HLG |
   |---|---|---|---|---|---|---|
   | Placed, nothing else | 18.5 | 19.7 | 19.8 | 46.2 | 51.3 | 51.7 |
   | `color.basic`, each pixel held | 19.9 | 25.5 | 23.1 | 33.0 | 49.3 | 49.8 |
   | `blur.unsharp`, Gamut held | 27.0 | 30.9 | 31.3 | 69.7 | 87.2 | 86.4 |
   | `light.wonderglow`, Gamut held | 60.5 | 69.3 | 65.9 | 211.7 | 224.2 | 218.1 |

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

Has: keyframes (hold, linear, easings, Bezier curves) with a graph editor in the editor (`apps/kerf-qt/ui/graph_editor.h`), transforms, titles with reveals by character or
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
point with a double-click and removing one with Delete. The graph editor zooms and pans both axes.

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

An agent starts with nothing to go on but the reference, so what it is told has to carry it. `look` on a fresh folder says in `unplaced` what footage is there and not in the document, and `look` with `file` gives the `media add` line that adds one, measured from the file. A refused line says what to do: which tracks there are and the `track add` that makes the one it asked for, the name a slip of a letter or two was for, and, where a clip or a media exists only in a pending proposal, that proposal's id and that `extend` builds on it. So do `look` and `render` where a pending proposal has what the project does not. Set where nothing reads it, such as a `bevel` with no `depth` or a `focus` with no `aperture`, a value draws nothing and fails nothing, so the check says so, as information that fails no row (`no_effect`). And `look` with `cost` and `at` says who draws a frame, which clip and effect send it to the CPU, and what it costs at full size and at half, against a frame's share of the time. `effects` looks through generators' parameters and clip properties too, since nothing else would lead from "shadow" to `gen=light@1` or from "bevel" to `bevel=`.

`look` with `media` and `facts` gives a fact sheet beside the index: what the camera does in each shot, from the one move, turn, and zoom that
most of the picture agrees on between small frames four times a second (`kerf_track::shot`, so a subject crossing the frame is left out), and R128 loudness.
60 seconds of 1080p takes 7 s the first time, and it is kept with the media's other derivatives. It reads a push in as a zoom and a move sideways as a pan, and only `track` with `camera` tells those apart. It says in `notMeasured` what it
leaves out, since silence would read as a finding: faces and the subject, which take a trained model, and on-screen text, which without one cannot be found reliably.

## Before shipping

Surveyed on 2026-11-08 at `3a175ca`. Each line names what shows it.

**Packaging.** Done: a Windows zip that runs on its own, checked by running it with nothing else on PATH, and its GPL source (`scripts/package-kerf-windows.ps1`, `.github/workflows/package.yml`), and a macOS app that carries Qt, FFmpeg and every Homebrew library it loads, checked to load nothing from outside itself and run with nothing of Homebrew's on PATH, with the source of each formula it carries (`scripts/package-kerf-macos.sh`, `collect-kerf-macos-sources.sh`). A published release is checked as someone downloads it: `verify-release.yml` fetches its Windows zip and macOS app onto fresh runners with no FFmpeg or Qt, on macOS with Homebrew moved out of reach, unpacks them, and runs `check-kerf-windows.ps1` and `check-kerf-macos.sh`, the checks the packaging scripts run on what they pack: ffmpeg, ffprobe, `kerf --version`, `kerfd --endpoint` and the editor's start window, with nothing else on PATH. Since a runner's System32 has the C++ runtime that a package might leave out, and a build machine names its Qt's plugins in `QT_PLUGIN_PATH` (install-qt-action sets it, and the checks now clear it), the Windows check lists each library the running editor loaded, which has to be the package's or Windows' own; on macOS dyld names each library loaded (`DYLD_PRINT_LIBRARIES`), which has to be inside the app or macOS's. It also holds that each package has its sha256 and source beside it, and that the checks fail a package with a library taken out. The Package workflow runs it on each release it publishes, and by hand it takes a tag. Fixed here: the Windows package was checked on the machine that built it with `QT_PLUGIN_PATH` naming that machine's Qt, so the packaged editor loaded its platform and style plugins from there and not from the package; the zip itself carries both and, on a fresh runner, loads them (v0.1.0, run 42586335808), so the check was what was wrong. And the macOS app kept rpaths into Homebrew (the editor's into Homebrew's Qt, two libraries' into the Cellar) and Qt's plugins one to a `lib` folder beside the app, where dyld would look for an `@rpath` library before the app's own; only rpaths inside the app stay now. Missing: a Windows installer, so `kerf://` links do not open the editor there (`apps/kerf-qt/README.md`, Build); a macOS release for Intel Macs, as the runner builds for Apple silicon only; a Linux package of any kind (the same script, with `patchelf`).

**Signing.** Missing on both. macOS gets an ad-hoc signature (`codesign --sign -` in `install-kerf-local.sh`), with no Developer ID, hardened runtime, entitlements or notarization, so Gatekeeper refuses a downloaded copy; nothing signs the Windows zip (no `signtool`).

**Versions and updates.** Done: one version, the workspace `Cargo.toml`'s, which the editor's CMake project repeats (`the_editor_has_the_workspaces_version` holds them the same) and Help ▸ About Kerf shows; `CHANGELOG.md`; and a `v<version>` tag publishes the Windows zip, the macOS app and the source of each as a GitHub release, refusing a tag that is not the version or has no section in the changelog (`package.yml`). Releases are not signed: Windows SmartScreen and macOS Gatekeeper ask before the first run. Once a day the editor asks GitHub's releases whether a newer one is out and offers its page, Download, Skip This Version or Later; it never downloads or installs anything itself, and Settings ▸ Updates turns it off (`kerf_update_check`, `update_notice.cpp`, `update_notice_test.cpp`). It trusts the system's certificates, so a network that inspects TLS with its own CA still works. Missing: an update that installs itself.

**CI.** Done: Linux runs the workspace, the Qt UI tests and the bridge workflow; Windows the kernel, runtime, sidecars and Qt UI; macOS builds and runs the Qt UI and the bridge workflow (`rust-runtime.yml`). macOS now runs `cargo test --workspace` too, after the editor's steps, on a runner with Apple's paravirtual Metal GPU, which `kerf-gpu` takes: the GPU parity tests run there, outside a developer's Mac, and passed with the rest (1637 tests, first run on #203); Linux has no GPU (it sets `WGPU_BACKEND=vulkan` so they skip) and runs only the catalog's list on llvmpipe (`KERF_GPU=software`). Unexplained: the Windows job runs the bridge's connection test five times because "a crash once came and went". It did not come back in the 100 runs from 2026-11-06 to 2026-11-08, 500 runs of the test, so there is no failure to read and its cause is still unknown; the five runs stay, to catch it. Every other red run in those 100 has a cause: two 504s from GitHub for the FFmpeg archive, Rust 1.99's new clippy lint, and an MCP test that took a zombie for a running process (`5338435`). Fixed here: main went red when GitHub answered 504 for the pinned FFmpeg archive on Linux and Windows in the same minute, so the downloads retry, and since GitHub's release downloads answered 504 for an hour on 2026-11-09 the pinned FFmpeg archives are kept in the Actions cache and the macOS source collection retries each Homebrew source for minutes; and Rust 1.99's clippy added `chunks_exact_to_as_chunks`, which failed the new lint step until the 107 sites moved to `as_chunks`. `dtolnay/rust-toolchain@stable` picks up such lints the day they ship.

**Crashes.** Done: a panic in kerfd, the editor's Rust or the `kerf` CLI leaves a plain-text report, with the component, version, OS, thread, message, where, and a backtrace, in the crash folder (`%LOCALAPPDATA%\Kerf\crashes`, `~/Library/Logs/Kerf`, or `$XDG_STATE_HOME/kerf/crashes`; `KERF_CRASH_DIR` moves it), which keeps the newest 50 (`kerf-runtime/src/crash.rs`, `tests/crash.rs`). A panic `catch_unwind` turns into an error is reported too. The next time the editor opens it says what stopped and offers the folder, once for each report (`crash_notice.cpp`, `crash_notice_test.cpp`). The release profile now unwinds, so the `catch_unwind` that turns a failed editor task into an error (`kerf-qt-bridge/src/tasks.rs`) and the one that keeps kerfd's media queue going past a bad file (`kerf-runtime/src/kernel.rs`) work in a release build, where `panic = "abort"` took the editor or kerfd down; and it strips only debug info, not symbols, so a backtrace names functions (`release_builds_unwind_and_keep_symbols` holds both). Missing: nothing sends a report anywhere, and a crash in the Qt editor's C++ (a segfault, an abort in Qt) leaves none, since only Rust panics are caught. `kerfd`'s own stderr also survives, in a temporary file (`kerf-runtime/src/ipc.rs`, `spawn_daemon`).

**Settings and projects.** Done: the editor's settings carry a version, `settings/version`, and `migrateSettings` brings an older Kerf's up to it once at start, before anything reads them, and leaves a newer Kerf's alone (`settings_migration.cpp`, `settings_migration_test.cpp`). Version 1 drops the nine keys no code reads any more, and the model the editor wrote into every save before `c58bca5` (`claude-sonnet-4-20250514`, or `claude-3-5-sonnet-latest` from its menu), which held a person who never chose one to a retired model; a model someone chose stays. The Deliver panel's keys from before W3 are still read in place (`render_queue_view.cpp`), since they name a folder and not the path a migration would need. Missing: API keys now live in the system keychain, macOS Keychain, Windows Credential Manager, or the Secret Service on Linux (`kerf_secret`, `kerf-qt-bridge/src/secrets.rs`, through the `keyring` crate), and one left in plain QSettings moves there the next time the editor loads it; where the keychain cannot be reached they stay in QSettings as before (`secret_store.cpp`, `secret_store_test.cpp`). A key round-trips through a real Secret Service (`a_key_round_trips_through_the_real_keychain`, run by hand); the macOS and Windows stores are untried. The Kerf Cloud token is kept there too (`cloud_session.rs`): one left in its 0600 file by an older Kerf moves into the keychain the first time the editor reads it, and where no keychain can be reached it stays in the file, as before (`docs/cloud.md`); the tests take both ways with a keychain in memory and with `KERF_SECRETS=off`, and never touch a real one, while `real_keychain_keeps_the_token_out_of_the_file` is run by hand. Fixed here: Windows sets no `HOME`, so the editor found no `cloud.json` there and Kerf Cloud said it was unavailable; it now takes `USERPROFILE`. A project is `kerf 1` and any other number is refused (`kerf-lang/src/doc.rs`), which is right until the first change to the format, and that change needs a reader of the old one. Fixed here: the editor offered retired Anthropic models and wrote one into new settings, so it overrode kerf-agent's `claude-opus-5-5` (`comments_test.cpp`).

**Engine.** Done, and measured: the GPU path and its parity with the CPU (The engine, Kernels). Missing: effects that still send a frame to the CPU: 101 of the 262 video effects, generators and transitions at their defaults (`kerf-compose/tests/catalog_cost.rs` lists them with why). None of the 33 looks and grades reaches one, and of the transitions only Morph Cut does, which the test holds; nothing in `skills/`, the presets or the agent's reference applies one either. So no use in the repository says which of the 101 to give a kernel first, and none was written: that waits on knowing what people apply by hand. `KERF_GPU=software` lets Mesa's llvmpipe stand in for a GPU, to check kernels where there is none; on it 378 of the 379 tests in `kerf-gpu`, `kerf-compose`, `kerf-engine` and `kerf-kernel` pass, and `every_scene_draws_as_the_cpu_draws_it` fails only on `color.basic`, every pixel within 2 of 255 but 0.9 off on average against the 0.6 it allows, from llvmpipe's `pow` and `exp`. CI's Linux job runs the catalog's list on llvmpipe, but not the parity tests, which it could only pass by loosening that one; and no timing comes from it. The quality numbers for roto under blur, `image.fill`, Bokeh Blur and Remove Grain are now checked on real footage too (Real footage); real film grain is still untried. Text has no OpenType features beyond kerning and ligatures, variable axes, colour glyphs or hyphenation (`kerf-text/README.md`).

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
