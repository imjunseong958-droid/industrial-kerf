# What was measured of shots

The record behind [architecture.md](architecture.md#next), Next 3 to 8: what
was run, on which footage, with which numbers, and what was found on the
way, as it was written when it was done. The architecture keeps what each
came to; [shots.md](shots.md) says how each is used. The numbers at the
head of each part are the architecture's.

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
   this waited only on 2, and on a glTF's camera and animation being read
   with its mesh, and both are (2026-11-13). `look` with `mesh` says what a
   mesh clip's file holds, its animations and its cameras, and with `camera`
   writes one of them as a `gen=camera@1` clip (`Compositor::mesh_camera`,
   `kerf-mcp/src/mesh_camera.rs`): where the file has the camera on each
   frame of the mesh's clip, the mesh being where its clip puts it and its
   animation as far in as the clip plays it, so that the mesh seen through
   it is the mesh as the file's camera saw it, wherever the clip stands,
   and with as few keyframes as say it. It is the document's camera from
   then on: nothing ties it to the file, and it is asked for again when the
   mesh's clip is moved or retimed. A glTF camera has one lens for the whole
   of a file, so a zoom does not come through, and one with no perspective
   is refused. What is added then goes down among the set's surfaces by the
   pixel, as any two meshes do
   (`what_is_added_to_a_set_is_hidden_by_the_sets_sides_before_it`). *Not
   done:* no rebuilder was run, so no set a model made from a shot was read,
   only files the tests wrote and Khronos's samples; and splats.

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
   | for scale, `generate model=shot …`, the take itself | 94 s |

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

   *A camera the mesh's file does not hold.* A path such as that, and the
   camera a set was seen by, come into the sequence the way a glTF's camera
   does. `look` with `mesh` takes `from`, a file that says where a camera
   was, in place of `camera`, one the file holds
   ([shots.md](shots.md#a-picture-read-back)): a path a model wrote, or
   a lens alone, which is a camera standing at the set's own origin. The
   compositor learned one thing for it, where a camera that is at a given
   place in a mesh file's space is in the sequence
   (`Compositor::mesh_view`), which is `mesh_camera` without the file's own
   camera; reading the text is `kerf-mcp`'s (`camera_file.rs`). A path is in
   its own model's units and a set in another's, so how much of the set one
   of the path's units is is given (`scale`), or found from the footage
   (`fits`, below), and so is which of the path's frames has its camera at
   the set's origin (`frame`). The workflow that rebuilds a set writes
   its lens as a file of its own (`out={lens.txt}`), and a fifth workflow,
   `set-views`, makes a set from the same reading of the frames a path
   comes from, so that the two are one size and `scale` is 1.

   *The whole of it, once, on a filmed shot* (2026-11-13). Four seconds of
   a walk along an overgrown railway, handheld, 65 frames at 16 a second
   and 640x360, from Wikimedia Commons (*Walking Keage Incline*, Kamigata0,
   CC BY-SA 4.0); it is not in the repository. One recipe read it back, in
   1 min 45 s: a set and a path in one size, a set with its lens, and the
   shot's depth.

   | Step | What came of it |
   |---|---|
   | The set with its lens as a mesh clip, and the camera `look` wrote from `lens.txt` | The frame it was rebuilt from, drawn again: the stones, the hedge, and the sign where the footage has them. A lens of 39.9 degrees. |
   | The set in the path's size, and the camera `look` wrote from `cameras.txt` | The footage's first, middle, and last frames, each drawn again with the stones where that frame has them: the camera turns 2 degrees and the set turns with it. 65 frames placed, in 3 to 11 keys a value. |
   | A box added on the path, on a track under the camera's | Hidden by the set's ground where it is under it, by the pixel; `look` with `shows` says how much. |
   | `render s1 {guide.mp4} pass=depth`, then the model that makes the shot | A red crate where the box is, on every frame, in a grassy scene of the model's own: 100 s. |
   | That take's depth read back, and `look` with `follows` | In 4, below. |

   What it shows, and what it does not. The way works from footage to a
   take, with every step a line of a recipe or of `look`, and the numbers
   that place a camera hold on a set a model made. Three things were found
   on the way. A box set down by eye, to look as if it stood on the path,
   was two frame heights off in the air behind it, where the same pixels
   are: the picture did not say so and the depth pass did. A clip on a
   track above the camera's was laid over the set and not sorted with it,
   the camera's track parting the two, and the lines `look` writes put the
   camera on top; a track that draws nothing no longer parts what is in
   depth (Motion). And the server makes a set seen through the wrong lens from a
   guide that is not the size its model runs at, so `set-views` sends its
   guide at that size ([shots.md](shots.md#a-picture-read-back)). The set
   that comes in the path's size is, as it comes, the near ground and
   little else; with `confidence=0` it is the whole frame, sky and all, as
   surfaces. It is one shot, in which the camera hardly moves, and one
   small model.

   *A shot whose camera travels* (2026-11-13). Four seconds filmed from a
   car driving down a street between houses in daylight, 65 frames at 16 a
   second and 640x360, cut from the middle of a dashboard camera's frame to
   leave out the dashboard, from Wikimedia Commons (*On the Road: Driving
   through Kranj*, Renato Lozinšek, CC BY 3.0); it is not in the
   repository. The camera goes 2.5 of the path's units down the street and
   pans 6 degrees, where the walk above turned 2 and went nowhere. One
   recipe read it back in 1 min 42 s: a set and a path of one reading, a
   set and its lens by the other model, and the shot's depth.

   | Step | What came of it |
   |---|---|
   | The set of frame 0 in the path's size, and the camera `look` wrote with `frame` 0, as for the walk | Nothing: every frame black. The path counts its places from frame 36, which the model picked, and the set of frame 0 is in that frame's space; taken to be in frame 0's, the camera starts past it and drives away. |
   | The same with `frame` 36, the path's `anchor` | Frame 0 drawn again where the footage has it. By the middle of the shot the camera is past all of it: as it comes (`confidence=0.1`) a set is the near ground. |
   | The set of frame 0 with `confidence=0` | The whole frame as surfaces. At frames 32 and 64 the far houses and the building on the right stand where the footage has them and as large, and the road is gone. |
   | Sets of frames 32 and 64 beside it, each a mesh clip of its own on a track over the camera's | The road, the kerb, the stone wall, and the houses where the footage has them on frames 0, 32, and 64. All three are in the one space, so each was made as tall as its file is beside the first one's and moved until its file's origin, which `look` with a lens alone places, was the first one's. |
   | A crate on the pavement at the end of the wall, on a track over the camera's and the sets' | 0.2% of the frame at the start and 1.6% at the end, a third of it hidden by the sets' surfaces at the start and none at the end (`look` with `shows`), by the pixel: the three sets and the crate are one run in depth across the camera's track (Motion). |
   | `render s2 {guide.mp4} pass=depth`, then the model that makes the shot | A street of yellow stone walls that keeps the guide's shapes and its holes, with no crate to be told from the wall: 94 s. |
   | The same with the guide drawn as its picture | The houses, the road, and a wooden crate where the box is: 66 s. |
   | Those takes read back, held to the guide | In 4, below. |

   What it shows, and what it does not. A camera a model found of a
   travelling shot holds through the whole of it, once the set is taken to
   be where the path has it: four seconds down a street, and the houses
   stay on the houses. Three things were found on the way. **A set of the
   path's own reading is in the space of the frame the path counts from**,
   and not of the frame it shows; the walk, whose camera stood nearly
   still, could not show the difference, and here it was the whole picture.
   `look` now returns that frame as `anchor` and says so where `frame` is
   another, and what the workflow says of itself was wrong and is put
   right. **A set of one frame is what the camera leaves**, so a travelling
   shot wants sets of several frames, which the one space allows, and
   `look` with `mesh` and `beside` writes the line that puts one where
   another is (it was four numbers a set by hand the first time). Five
   sets a second apart, each with every pixel, leave 6 to 13% of the frame
   empty after the first frame, where the set of frame 0 alone left 35 to
   59%; what moved in the footage, a car ahead, is in each set where it
   then was. **The two models do not agree on the lens**: the path's has
   37.2 degrees down the frame, between 36.4 and 38.1, and the one that
   rebuilds a frame alone has 22.8, of the same footage. The sets have
   holes where a surface turns from the camera, a third to a half of the
   frame, and the model that makes the shot kept them as black.

   *A size found for two models* (2026-11-13). With `fits`, `look` finds
   how much of a set one of a path's units is from the footage, with no
   model ([shots.md](shots.md#a-picture-read-back),
   `kerf-mcp/src/set_scale.rs`): the set is drawn through the path at a run
   of sizes on the four frames furthest from the set's own, and held to
   the footage by the pixel. On the shot above, each in about ten seconds:

   | The set | The size found | The picture's error there, of 255 | `sure`: half or twice the size, over that |
   |---|---|---|---|
   | Of the path's own reading, where the size is 1 | 0.956 | 29.1 | 1.57 |
   | Of the other model, with no lens given: `look` finds 22.7 from frame 0 of the footage, 6.3 times surer than a quarter either way, where the model's own file says 22.8 | 20.0 | 46.8 | 1.12 |
   | Of the other model, seen through its own lens of 22.8 degrees | 20.1 | 46.8 | 1.13 |
   | The same, the path's travel along the way the set was seen made 1.68 times longer, by the two lenses (`deep`) | 17.9 | 45.2 | 1.21 |
   | Of the other model, through the path's lens of 37.2 | 3.24 | 44.5 | 1.04, which is no finding |

   Where the answer is known it is found to within a twentieth. For the
   two models it is 20.1, and with the camera written at that size frame 0
   is the footage's, and at frames 32 and 64 the houses stand near where
   the footage has them with the camera a little short of where it went:
   the two disagree on the lens by more than a half, so no one size is
   right on every frame, which `sure` at 1.13 says. Through the wrong lens
   the fit is flat and is said to be. Making the path's forward travel
   longer by the ratio of the two lenses, since a set through a narrower
   lens is that much deeper, has the houses larger at frames 32 and 64, as
   the footage has them, and still not to the pixel; `look` does that
   where it finds the set's lens is not the path's. So two models can be
   used together coarsely, and one reading is still the way to have them
   meet.

   What moved is left out by time and not by a new kind of set: each
   set's clip made to last only about its own frame, overlapping the next,
   the car ahead is where it is on each frame and the smear is gone, at
   the cost of the holes fewer sets leave, 17 to 32% of the frame.

   *Not done:* one set of a whole shot, whole and with nothing in it that
   moved. Two ways to it with what is in reach were tried (2026-11-13) and
   left. Joined on the server, four frames' sets come back as one mesh
   with one picture, the first frame's, on all four, and fit the footage
   worse than one frame's set (a picture error of 43.7 for 29.1). Shown at
   once in Kerf, three frames' sets as three clips under the one camera,
   the first frame's surfaces, stretched where that frame saw them
   edge-on, stand in front of the later frames' from where those were
   filmed. With those surfaces dropped (`discontinuity=0.01`), four
   frames' sets at once cover more of the frame than one (0.33 to 0.64 of
   it on five frames, for 0.05 to 0.19) and match the footage worse where
   they draw (a mean difference of 63 to 85 of 255, for 47 to 66): the
   road goes with the stretched surfaces, and the same house stands in
   several places, a frame's set not lying quite where another's does. A whole set takes fusing the views, dropping what each sees
   badly, which is a rebuilder's work; sets timed to the part of the shot
   each was seen in stay the way. Also not done: the two
   models' lenses reconciled further than by their ratio along one way
   (letting the footage choose the stretch about that ratio was tried: it
   chose more, 2.2 for 1.68, for a picture error a hundredth less, and the
   houses at the last frame were no nearer their size in the footage, so
   the ratio is kept); splats. Of those the server has
   a node that makes a mesh of one, so a workflow can hand Kerf the `.glb`
   a mesh clip reads and the compositor need draw none; what it has no
   model for is rebuilding a scene as splats, only a thing. So the half
   that needs no model is in (2026-11-13): `set-splats`, a workflow of
   `kerf-comfy`, takes a scene kept as splats, whatever made it, as a
   line's `ref=`, and has the server's nodes make the mesh a `gen=mesh@1`
   clip reads. Run on a scene written by hand as 14,300 splats, a floor, a
   wall, and a red box of side 1 three units ahead, it gave a `.glb` of
   554,166 corners in 13 s whose box is within 0.06 of the one written
   on every side (x -0.56 to 0.56 for -0.5 to 0.5, y -0.94 to 0.06 for -1
   to 0, z -3.56 to -2.44 for -3.5 to -2.5: fatter by a splat's width),
   the floor within 0.05, in the file's units and turned to Kerf's axes,
   so a camera at the origin sees the box ahead of it on the floor before
   the wall, looked at. Splats of a filmed scene were not tried: nothing
   in reach makes them.
4. **A take held to its guide by numbers.** Every row of the tables above
   was judged by eye: place, path, size. The whole frame's rank correlation
   (3) does not replace the eye: the wrong take scored 0.966. What does is
   the same two depths read by the clip, which is in: `look` with `follows`
   ([shots.md](shots.md#a-picture-read-back), `kerf-engine/src/follows.rs`).
   The guide's ids pass says which pixels are a clip's, and over those it
   says how far the take's order of near and far is from the guide's, as a
   share of the frame's order, with the direction kept: where a subject is
   not where the guide has it, what was behind it is there, and is farther.
   On the scene of `fixtures/shots/car`, 64 frames, with no model: the
   guide's own depth pass handed back as the take, and the same played
   backward, each through an encoder as a take is.

   | The take's depth | `order`, the whole frame | `off`, the whole frame | the road | the car's body | its cabin |
   |---|---|---|---|---|---|
   | The guide's own | 0.999 | 0.009 | 0.008 | 0.016 | 0.021 |
   | The guide's, played backward | 0.971 | 0.032 | 0.023 | 0.094 | 0.217 |

   The whole frame moves by a thirtieth, and the subject by six and ten
   times. It needs a model to read the take's depth and nothing else, and
   draws the guide's two passes itself.

   On the filmed shot of 3, with real takes and their depth read back by
   a model, the box added to the set:

   | The guide | `order` | `off`, the whole frame | the set | the box |
   |---|---|---|---|---|
   | The box standing on the path | 0.833 | 0.112 | 0.112 | 0.118 |
   | The box in the air two frame heights behind, looking as if it stood there | 0.787 | 0.122 | 0.111 | 0.507, all of it nearer |

   In both the take has a crate where the box is in the picture. Where the
   box stood on the ground it is as far off as the frame is. Where the
   guide had it far away and the model drew it standing on the near ground,
   as it looked, it is four times the frame, and `farther` says which way.
   So it reads a subject's depth and not only its place, and it is read
   beside the frame's own `off`, which here is a tenth: the set has holes
   and a sky as a surface, and the model drew trees there. Two takes of one
   shot.

   What it does not say is where the subject is, and that is the other
   half, which is in: `look` with `placed`
   ([shots.md](shots.md#a-picture-read-back), `kerf-engine/src/placed.rs`).
   The guide's ids pass gives the box around a clip on each frame, as
   `shows` does; the subject is found on the frame the guide shows most of
   it on and followed from there to both ends (`kerf_track::roto`), in the
   take and in the guide's own picture, and the two boxes are held
   together in frame heights. It reads the take's picture, so it needs no
   model. Following the guide's own picture as well came of measuring: held
   to the box of the ids pass, the guide's own picture was 0.030 off and
   1.40 times the size, a tracker's box being wider than a pass's by its
   edge and by what the set hides of the crate.

   On the travelling shot of 3, the crate, 65 frames:

   | The take | `follows`: `order` | `off`, the frame | `off`, the crate | `placed`: frames followed | `off` | `across` | `size` |
   |---|---|---|---|---|---|---|---|
   | The guide's own picture, through an encoder | | | | 65 | 0.006 | 0.001 | 1.04 |
   | The same moved 40 pixels of 832 to the right, which is 0.083 | | | | 65 | 0.085 | 0.085 | 1.05 |
   | Guided by the sets' picture | 0.859 | 0.109 | 0.068 | 62 | 0.024 | 0.017 | 1.23 |
   | Guided by the sets' depth | 0.912 | 0.080 | 0.029 | 1 | | | |

   What it says. `placed` measures a move as the move it is: 0.085 for
   0.083, over a floor of 0.006. The picture-guided take has its crate
   0.024 of a frame height from where it was asked for, four times the
   floor and a quarter of a crate's width, and drew it a fifth larger. The
   depth-guided take is the better by `follows`, on the whole frame and on
   the crate's pixels alike, and has no crate: it was followed on one frame
   of 65, since what the model drew there is wall. So the two are read
   together. Depth by the clip says the shape was kept; the place says
   there is a thing there to find; and a take can pass either alone. One
   shot and one small model, again, and a subject that is 1% of the frame.

   *One number for the place*, and the guide with fewer holes. `placed`
   also gives `overlap`: how much of the two boxes is in both, of what is
   in either, as a mean over every frame the guide shows the clip on, with
   a frame the subject was not followed on as none. So it falls for a
   subject that is aside, for one drawn another size, and for one that is
   not there, and is what two takes are chosen between by. With the five
   sets of 3 in the guide, which leave a tenth of the frame empty where
   the three left a third to a half, both takes were made again, the
   prompt and the seed as they were:

   | The take | `follows`: `order` | `off`, the frame | `placed`: frames followed, of 65 | `off` | `size` | `overlap` |
   |---|---|---|---|---|---|---|
   | The guide's own picture, through an encoder | | | 65 | 0.006 | 1.00 | 0.90 |
   | The same moved 0.083 to the right | | | 65 | 0.091 | 1.00 | 0.22 |
   | Guided by the five sets' picture | 0.893 | 0.100 | 32 | 0.027 | 1.16 | 0.35 |
   | Guided by the five sets' depth | 0.928 | 0.079 | 44 | 0.038 | 0.82 | 0.38 |
   | The first picture-guided take, held to this guide | | | 62 | 0.054 | 0.96 | 0.42 |
   | The first depth-guided take, which had no crate | | | 1 | | | 0.002 |

   What it says. A guide with fewer holes is followed better by both
   (`order` 0.859 to 0.893 and 0.912 to 0.928), and the depth-guided take
   is now a street with a road and houses where it was walls about a void,
   with a crate where one was asked for on two thirds of the frames. No
   take has the crate as the guide has it: the best is 0.42 where the
   guide's own picture is 0.90, and the take with nothing there is 0.002,
   so the number orders them as the eye does. It also shows what a set of
   several frames costs: the car ahead in the footage is in each set where
   it then was, a smear along the road by the last frame, and both takes
   drew things there, the depth-guided one a second crate.

   *One number for a take.* `look` with `held` reads both at once, of the
   clips named as the guide's subjects, and gives each a number from 0 to
   1: the overlap of where the take has it with where it was asked for,
   less by how far its depth is off, and their mean for the take. Of the
   two takes above it is 0.31 for the picture-guided one and 0.35 for the
   depth-guided one, 0.25 for one guided by `pass=relative`, where the first depth-guided take, with no crate, is
   0. The subjects are named and not guessed: by how much of the frame a
   clip has, four of the five sets were taken for subjects.

   *On a third scene* (2026-11-14), the car of `fixtures/shots/car`, its
   takes as they were made, 64 frames, each clip followed on all of them:

   | The take's guide | the body: `off` | `size` | `overlap` | the cabin: `off` | `size` | `overlap` |
   |---|---|---|---|---|---|---|
   | The picture, in flat colors | 0.024 | 0.83 | 0.72 | 0.018 | 0.73 | 0.58 |
   | `pass=depth` | 0.119 | 1.26 | 0.40 | 0.041 | 0.64 | 0.40 |
   | `pass=ids` | 0.065 | 0.98 | 0.52 | 0.049 | 0.54 | 0.29 |

   `held` of the depth-guided take is 0.37, its depth read back in the
   guide's order over the frame (`order` 0.992) and the body 0.11 off,
   nearer. By eye the depth-guided take was the best of the three, a car
   on its path, the picture-guided one red boxes in a van, and the
   ids-guided one nearly black. The numbers have them the other way
   round: what kept the boxes as boxes is nearest the boxes, and a car is
   larger than the two solids it was drawn from. So the place says how
   nearly what is there is what the guide's box asked for, and where a
   guide's shapes stand for something else, it is not how good the take
   is.

   *On the walk again* (2026-11-14), another four seconds of the same
   film as 3's first shot (*Walking Keage Incline*, Kamigata0, CC BY-SA
   4.0; not in the repository), the path between the rails, read back,
   with a crate put a little above the path, both takes made and read as
   on the travelling shot:

   | The take's guide | `follows`: `order` | `off`, the frame | `off`, the crate | `placed`: frames followed, of 65 | `off` | `size` | `overlap` | `held` |
   |---|---|---|---|---|---|---|---|---|
   | The set's picture | -0.098 | 0.379 | 0.588, nearer | 65 | 0.010 | 1.03 | 0.86 | 0.35 |
   | The set's depth | 0.867 | 0.100 | 0.179, nearer | 1 | | | 0.005 | 0.004 |

   The two readings part again, the other way round from the travelling
   shot's first takes. The picture-guided take has the crate where it was
   asked for, a red square kept as a red square, and a depth that reads
   against its guide's: the set of one frame has holes over a third of
   the frame, the model that makes the shot kept them black, and the one
   that reads depth took the bright band along the horizon between them
   for the nearest thing in the picture. The depth-guided take kept the
   scene's depth and drew its crate on the ground by the path, where a
   crate would stand, and not above it where the box was: followed on one
   frame. So `held` is low for both, each for its own half, and says
   which half by its parts. One thing was found on the way: this path has
   no frame at its origin to the last digit, the nearest being frame 14
   and three ten-thousandths off, so no `anchor` was reported and the set
   was taken to be in frame 0's space; the frame nearest the origin is
   now the anchor where it is within two thousandths of it
   (`camera_file.rs`).

   What is still not done is a set with what moved left out. Either is what lets an agent say of a
   take what a person says of it, and choose between two.
5. **More than one guide on a line**, which is in (2026-11-13): a line names each guide beside the first by what it is
   of, `guide={picture.mp4} guide_depth={depth.mp4}`, the request holds
   them under `guides`, a take is held to each, and `kerf-comfy` sends one
   to the workflow blank of its name ([shots.md](shots.md#recipes)). A
   It was run on the one model in reach, which reads a picture and a
   mask of it (`kerf-comfy`'s `shot-into`): the footage of Next 3 with a
   box's place in flat grey as the guide, the same box in white on black
   as `guide_mask`, both drawn by Kerf through the camera a model found.
   The take is the footage with a red container standing where the box
   was put, on all 65 frames of a shot that travels: outside the mask it
   is 7.3 of 255 from the footage, a grey a pixel, and inside it 49.1;
   `placed` has the container 0.027 of a frame height from the box, 0.90
   its size, with an `overlap` of 0.66, where the best take that made the
   whole picture had 0.42. A box 1% of the frame was filled in from its
   surroundings and no crate drawn; one of 2.7% was made. So what the
   loop was for is done once, end to end: a thing put into a filmed shot
   by where it stands in the scene. The set hides the thing in a second
   making: `render … matte=<clip>` draws one clip in white on black, as
   much of it as shows among the sets, and the workflow takes the footage
   as it is and greys the masked place itself. That take has a box where
   the matte is, cut by the wall's end, and brown where a red one was
   asked for: the smaller and more ragged the place, the less this model
   draws what the prompt says there. A
   `generate` line handed a model one
   guide. A model that reads a picture and its depth, or depth and ids,
   wants each named. No model in reach reads two of those, and Wan VACE takes one
   control video, so a picture with its depth waits for one that does.
6. **How a point moves, as a direction of its own**, which is in
   (2026-11-13): `generate … moves=<clip>` puts a clip's
   place on each frame in the request, read from the frame as `shows`
   reads it, and `kerf-comfy` fills a workflow's `{moves}` blank with them
   as tracks of points in the model's pixels
   ([shots.md](shots.md#where-a-thing-goes), `recipe/moves.rs`). On the
   travelling shot the crate's 65 places are the boxes `shows` gives. The
   model that follows points was not then on the server, so it was first run on
   the one that is, which follows a layout: `kerf-comfy` draws the boxes
   as a video, white with each box's edge in black (`{moves_video}`), and
   its workflow `shot-led` hands the model that and the prompt, with no
   picture of the scene at all. The take is a street of the model's own
   with a red container that goes where the box goes, small at the left
   at the start and large at the right at the end, on all 65 frames:
   `placed` has it 0.040 of a frame height from the box, 0.002 across and
   0.027 down, 1.14 times its size, with an `overlap` of 0.57. So where a
   thing goes is a direction a model follows by itself, apart from what
   the scene looks like, and the document says it by naming a clip. ComfyUI has nodes for
   Wan-Move, which follows paths of points given as coordinates
   (`WanMoveTracksFromCoords`). A clip's place on each frame is what `shows`
   reads and what the document says outright, so its path is there to hand
   over, as a signal's numbers are. The weights were not on the server. The node
   that reads the points is, and was run (2026-11-13): `moves-shown` has it
   read what `kerf-comfy` fills `{moves}` with and draws the points over a
   guide. Over the take `shot-led` made, the crate's point is on the
   container on the frames looked at, so the text Kerf hands is the text
   the node reads, in the model's pixels. The weights came the next day,
   and the model was run (2026-11-14): `shot-moved` hands it a picture to
   start from, the first frame of the take with the container put into the
   footage, the crate's points, and the prompt. `placed` against the crate,
   beside the layout's take:

   | What is handed | Frames followed | Off | Size | Overlap | Made in |
   |---|---|---|---|---|---|
   | A layout of the box (`shot-led`) | 65 of 65 | 0.040 | 1.14 | 0.57 | |
   | One track, the box's middle, 4 steps | 59 of 65 | 0.028 | 1.37 | 0.48 | 63 s |
   | Five tracks, the middle and toward the corners (`{moves_box}`), 4 steps | 65 of 65 | 0.040 | 1.13 | 0.61 | 71 s |
   | Five tracks, 20 steps | 65 of 65 | 0.035 | 0.96 | 0.60 | 460 s |
   | Five tracks, 20 steps, steps reused (`cache=0.2`) | 65 of 65 | 0.047 | 0.98 | 0.58 | 216 s |

   A point says where and not how large: the container is nearest its
   place with one track and a third too large. Five tracks say both, and
   hold the container to the box better than the layout did, so
   `shot-moved` hands five. Looked at, the container starts small at the
   left and ends large at the right in each, and the street stays about
   where the first frame has it and does not travel as the footage does:
   nothing was said of it. With steps reused the picture is coarse from
   the first frame. The 4 steps are by weights added to the model's for
   that; their picture is as clean as 20 steps' and the container is held
   as well, in a seventh of the time, so `shot-moved` is made that way as
   it comes (66 s, 0.61 again), and `steps=`, `cfg=` and `lora=` on a line
   say otherwise. It
   is the fourth row of the table at the top: where, when, what, and
   how each thing goes.
7. **A signal measured** (2026-11-13), on the one model in reach, which
   takes no number a frame and does take a mask a frame. `kerf-comfy`
   hands a signal to a workflow as a grey video, each frame as light as
   the signal is then (`{signal_video}`), and a workflow that comes with
   it, `shot-timed`, has the model keep its guide where the signal is 0
   and make the frame anew where it is 1. On the travelling shot of 3,
   the sets' picture as the guide, a signal that steps at the middle
   frame, and how far each frame of the take is from its guide, in grey
   of 255, by a script kept out of the repository:

   | The signal | Frames where it is 0 | Frames where it is 1 | The two, frame by frame (correlation) |
   |---|---|---|---|
   | 0 for the first 32 frames, then 1 | 7.6 from the guide | 17.1 | 0.93 |
   | 1 for the first 32 frames, then 0 | 6.8 | 12.1 | 0.84 |

   The take leaves its guide where the signal says and when it says, the
   other way round as readily, and begins three frames before the step:
   the model makes four frames as one. So a number over time reaches a
   model and is followed, and what it does there is a workflow's to say;
   here it is how much of the guide is kept. Still wanting is a model
   that takes the number itself, for what a mask cannot say: how fast,
   how bright, how much.
8. **In the editor**, both in (2026-11-13). A signal's curve is drawn
   where its clip is, in the timeline under the picture: its number along
   the clip, 0 at the bottom and 1 at the top, filled under as a sound's
   level is, from values the timeline's view now carries for a signal
   clip. It is seen and not yet drawn by hand: its keys are still text.
   Choosing a take: Takes, on a media in the bin, lists
   the takes its recipe has made, which a snapshot's sources now carry,
   and has the media be another.
9. **All four on one line** (2026-11-17), on the same model, Wan 2.1 VACE
   1.3B: a court blocked in depth, a ball on a path that bounces across it
   (`moves=ball`), a reference picture of the ball, and a signal that is 1
   for four frames at each bounce, 65 frames at 832x480 and 16 a second,
   seed 7. `shot` reads the guide and the reference and says the other two
   are left out. What reads all four is `shot-directed`: the depth guide
   and the reference as `shot` hands them, the clip's layout laid over them
   by a second node of the same kind, and the guide handed as plain grey on
   the frames where the signal is 1.

   | The workflow | Reads | On the server | The ball in the take |
   |---|---|---|---|
   | `shot` | guide, ref | 88 s | the guide's box, as a flat orange slab |
   | the signal as a mask a frame, as `shot-timed` hands it | all four | 105 s | no court and no ball: the depth guide itself, in grey |
   | `shot-directed` | all four | 124 s | a round ball where the layout has its box |
   | `shot-directed`, `moves_strength=1.6` | all four | 122 s | the same, the wall and floor coarser |
   | `shot-directed`, no `signal=` on the line | guide, ref, moves | 123 s | a round ball where the layout has its box |

   The second row is why `shot-directed` does not hand a signal as a mask:
   to this model a mask of 0 keeps the guide's own pixels, which for a
   picture guide is the footage kept and for a depth guide is its depth
   given back as the take. Laid over more strongly, the layout holds no
   better, so it is 1. The last row is the stand-in rule on the real
   server: with no signal the node that lays grey over the guide is left
   out and the guide is read as it is.

   Not shown by numbers. `look` with `placed` followed the ball on 12 frames
   of the 51 the guide shows it on in `shot`'s take and on 8 in
   `shot-directed`'s (`overlap` 0.16 and 0.07): a ball that crosses a third
   of the frame between bounces is lost by what follows it, so the two
   takes were told apart by looking at every eighth frame, not by measure.
   On the frames where the ball dips behind the tipped floor the guide's
   sequence shows nothing of it, the layout has no box, and the take has no
   ball: the model was handed nothing there. The reference was a flat
   stand-in, an orange square, so what the reference adds was not looked
   at. Still wanting: a subject followed fast enough to measure, and a
   model that takes the signal as a number.
10. **A shot longer than the model makes at once** (2026-11-17), the same
   model and court, the ball crossing it for ten seconds on a path that
   stays above the floor. Two parts of 81 frames, the second's guide drawn
   from 5 s of the sequence and its first frame the first part's last
   (`first={a.mp4}`, kept by a mask of 0 on that one frame of the guide),
   set end to end with the shared frame left out (`join … lap=1`): 161
   frames at 832x480 and 16 a second, 165 s and 151 s on the server.

   Looked at on every twentieth frame and on the ten frames around the
   seam: one court and one ball all through, the ball where the layout has
   it on both sides of the seam, with no jump in where it is. What does
   change at the seam is tone. The mean brightness of a frame, of 255:

   | Frames | 76 to 80, the first part's last | 81 | 82 to 86 | 120 | 160 |
   |---|---|---|---|---|---|
   | Mean | 104.0 to 104.3 | 94.6 | 93.5 to 91.8 | 89.6 | 90.1 |

   The second part is 10 of 255 darker from its second frame and stays
   there. It was handed one picture and its own guide; nothing tells it how
   bright the part before went on to be. Not tried: a tone matched across
   the join, a part led in by more than one frame of the one before, three
   parts or more, a signal across a seam.
11. **The whole example by models** (2026-11-17). Until here the ball's
   reference came from a stand-in script. `still` asks the same model for
   one frame: "an orange basketball, studio light", seed 3, 832x480, 8 s on
   the server, a basketball on an orange ground with its seams. The recipe
   of 9 then ran with nothing standing in: the court's depth drawn by Kerf,
   the still, and `shot-directed` reading the depth, the still as `ref=`,
   the ball's way and the signal, 124 s. Looked at on every eighth frame,
   the ball in the take is rounder and nearer the still's orange than with
   the stand-in, and has none of its seams: at the size the ball is in this
   frame, a twentieth of its width, a reference's detail does not show.
   What a reference adds was not measured.
12. **A thing found in footage by its name, and one reader for each thing
   read** (2026-11-18). The server has had SAM 3.1 since
   (`sam3.1_multiplex_fp16.safetensors`, in `models/checkpoints/`), which
   ComfyUI's own nodes run: it finds what a text names, or what points are
   on, and follows it through a video. So there were three models that
   read a picture back, and a fourth in Kerf itself, MobileSAM in plain
   Rust behind `look` with `track`; this is what each was found to do
   beside the others, and which went. Two shots, from Netflix's *Chimera*
   (CC BY 4.0, `licenses/README.md`), cut from the 960x540 file of the Open
   Content bucket, neither in the repository: *the walk*, a man coming
   along a dirt path toward a camera that turns with him, and *the
   freeway*, a camera that does not move over two directions of traffic, a
   white box truck crossing and a black pickup creeping across the near
   lane. Each is 81 frames of 832x480 at 16 a second; the long ones are
   240 frames of 960x540 at 25. The models that make a shot are the small
   ones the rest of this record was made with, so what a take looks like
   says that the way works and little of how good it can be.

   *A matte* (`matte`: the guide, `SAM3_VideoTrack` by the prompt,
   `SAM3_TrackToMask`, saved as a video). From the line being read to the
   file being there:

   | Line | Took | What it found |
   |---|---|---|
   | the walk, `prompt="man"` | 16 s, 13 once the model is loaded | the man on all 81 frames, 1.1% of the frame growing to 4.2% |
   | the freeway, `prompt="black pickup truck" objects=1` | 13 s | the pickup on the 72 frames it is in the picture, and nothing on the 9 after it has left |
   | `prompt="white box truck"`, `objects=4` as it comes | 14 s | the truck from the frame it drives in on, and pickups taken for trucks beside it |
   | `prompt="car" objects=16` | 15 s | sixteen cars; those that drive in once sixteen are followed are not taken up |
   | the same with `which=4`, the pickup's number in `matte-shown` | 13 s | the pickup, 0.991 of the matte its own words make, by what is in both of what is in either |
   | `inside="0.55,0.80 0.45,0.82"`, no prompt | 13 s | the pickup, 0.968 of that matte, 0.928 at worst |
   | `prompt="elephant"` | 13 s | black, but for a speck of a thousandth of the frame on 12 frames |
   | the walk, 240 frames of 960x540, `prompt="person"` | 47 s | two words of the title card the file starts on, and then the man from his first frame to the last |
   | `prompt="t-shirt" objects=1` | 13 s | his shirt |
   | `prompt="sky"` | 13 s | the sky |

   Four things were found on the way, and each is in how the workflow is
   written. **An empty text is words to the model**: with no prompt the
   matte was 16% of the frame, cars of its own choosing, so the node that
   reads the prompt is left out when a line has none (`"given":
   ["prompt"]`). **Words and points do not go to one follower**: handed
   both, it had the pointed thing on the first frame and nothing of it by
   frame 20, so there are two, and points win. **A follower handed no
   mask at all fails** on the server's GPU (`invalid configuration
   argument`), so the node that makes masks of points goes when no points
   are given, and the follower of masks with it. **Which thing is which
   is a picture**, the one ComfyUI's preview node draws; it saves among
   the server's temporary files, which `kerf-comfy` did not fetch, and
   does now where nothing was saved to keep (`matte-shown`).

   *Beside Kerf's own cut, and MobileSAM behind it.* The same two things,
   by `kerf_track::follow_object` with two points on the thing in the
   first frame, on four cores, each outline drawn as a matte and held to
   SAM 3.1's by what is in both of what is in either:

   | | The man | The pickup | 81 frames took |
   |---|---|---|---|
   | The cut, by colour and edges | lost at frame 68; 0.69 where it had him | 0.50, and 0.004 at the end | 6 s |
   | The cut with MobileSAM behind it | all 81 frames; 0.78 | 0.68, and 0.003 at the end | 111 s |

   Looked at on four frames of each: the cut takes a wedge of the hillside
   behind the man with him, with the model and without; of the pickup it
   takes the stack of tires behind it as well, and stays on the tires once
   the pickup has left the picture, where SAM 3.1 has the pickup alone and
   then nothing. A thing among others like it, and a camera that moves
   past what is behind, are what colour and edges cannot say, and the
   small model did not say them either, in eight times the time of the
   large one on the server. **So MobileSAM is out of Kerf**: the crate
   that ran it (`kerf-segment`, its PyTorch reader and its kernels, 1,900
   lines) and `model` of `look` with `track`. The cut stays as what needs
   nothing set up, and a model's matte is a recipe's line as any model's
   file is. The roto's tests pass as they did, the model having been a
   prior the cut ran without.

   *A part made anew* (`shot-into`, Wan 2.1 VACE 1.3B, 20 steps, seed 7,
   the walk, `guide_mask=` the man's matte, "an astronaut in a white
   spacesuit and helmet walks along a dirt path"). As the workflow was, the
   whole frame was the model's: an astronaut where the man walked, and the
   hillside around him 19 of 255 from the footage, a mosaic of it, since
   what a model keeps it still draws. The model's picture is now laid back
   over the guide through the mask, grown and softened (`grow`,
   `feather`), and away from the mask the take is 2.1 of 255 from the
   footage, which is the file's own encoding. 97 to 104 s.

   | Line | What the take shows |
   |---|---|
   | the man's matte, the astronaut, `grow=12` | an astronaut in his place and at his size on each frame looked at, walking as he does |
   | his shirt's matte, "a man wearing a bright red hawaiian shirt with yellow flowers", `grow=4 feather=5` | the man in a red flowered shirt, his face, arms and jeans the footage's own |
   | the pickup's matte, "a yellow taxi cab parked at the side of a freeway" | the pickup, as dark as it was, with yellow lettering on its door |
   | the same, "a bright yellow car", `grow=24` | no pickup and the tires behind it at frame 10, and a dark red car by frame 40 |

   The first two are the way working. The last two are the model: a mask
   that hugs a pickup is filled with a pickup, and its color is the
   prompt's only as far as a small model follows a prompt.

   *A thing taken out*, by the same workflow with what is behind the thing
   as its prompt:

   | Line | What the take shows |
   |---|---|
   | the walk, "an empty dirt path below a dry grassy hillside, nobody, no people", `grow=20` | a figure in camouflage where the man was |
   | the same without the last words, `negative=` naming a person, a man, a figure, `grow=40` | a blurred blue figure |
   | "a dry grassy hillside above a pale dirt path, bushes and trees, daylight", the same `negative=`, `grow=64 feather=21`, seed 11 | no figure: a patch of bush and slope, rougher than what is around it |
   | the freeway, the pickup, "a freeway shoulder with a stack of black tire barrels by a concrete barrier, empty road", `negative=` naming a car, a truck, a vehicle, `grow=24` | no pickup: the road, and the tires carried on to where it stood |

   A mask in the shape of a man walking is filled with one, whatever the
   words: it comes out only when the mask is grown past telling what it
   was the shape of, and what the thing is goes under `negative=`, not in
   the prompt. Where the camera holds still the model has what is behind
   the thing from the frames it has moved on in, and the plate is one to
   use; where the camera turns it is a patch. ComfyUI has nodes for a
   model made for this (VOID), whose weights the server has not got.

   *Layers, in the document.* A project of the walk with three media a
   recipe makes, each by `kerf make` and no other way: the man's matte
   (one line, 43 s with another job ahead of it on the server); his shirt made anew (the two lines
   above, 35 s and 179 s on a server doing other things); and a sequence
   Kerf draws of the footage, a title over it, and the footage again over
   that shown through the matte (`key.trackmatte`, the matte's track
   turned off). In the frames looked at the title stands behind the man:
   his legs are before its letters, and the path is behind them. The
   project is kept, with its recipes and how to cut the footage, in
   `fixtures/shots/walk`.

   *One depth.* Three ways to how near each pixel is came with
   `kerf-comfy`, by two models: each frame alone by MoGe 3 (`depth`), each
   frame alone by Depth Anything 3 (`depth-whole`), and the frames read
   together by Depth Anything 3 (`cameras` with `out={depth.mp4}`). Each
   on both shots, and on 240 frames of the freeway:

   | | 81 frames took | 240 frames took | Its grey moves, where the picture holds still, of 255: the walk | the freeway | 240 frames of it | The same order of near and far as MoGe's (rank correlation) |
   |---|---|---|---|---|---|---|
   | MoGe 3, `refine_steps=1` | 37 to 39 s | 114 s | 5.9 | 2.8 | 3.0 | |
   | MoGe 3, `refine_steps=3` | 83 to 97 s | | 5.9 | 2.5 | | 0.999 |
   | Depth Anything 3, a frame at a time | 14 s | 41 s | 8.1 | 3.6 | 6.6 | 0.88 and 0.95 |
   | Depth Anything 3, the frames together | 13 to 14 s | 45 s | 1.8 | 0.9 | 1.1 | 0.88 and 0.94 |

   How much a grey moves is the mean, over the pixels whose picture stays
   within 2 of 255 through the shot and that MoGe gives a surface, of how
   far each one's grey is from its own mean over the frames: a tenth of
   the frame in each shot, the far ground and the barrier. Read together,
   a place that does not move holds its grey three times as well as by
   MoGe and four to six times as well as by its own model a frame at a
   time, in a little over a third of MoGe's time; a guide is followed frame by frame, and
   a depth that wavers is a thing that moves. The two models agree on the
   order of near and far less than a model agrees with itself, 0.88 on the
   walk, and which is right no footage says; against a depth Kerf drew, 3
   above had them 0.993 and 0.996. **So `depth` is the frames read
   together**, up to the 240 it was tried on, `depth-whole` is gone, and
   `cameras` writes a path and no video, the depth of its reading being
   `depth`'s to the grey (0.4 of 255 apart, two encodings of one answer).

   What went with MoGe's depth: a sky read as no surface, which the frames
   together give a distance, the farthest; and its other two pictures,
   which way each pixel faces and what is a surface, which nothing in Kerf
   read. What is sky is a matte's to say now: SAM 3.1's `sky` and MoGe's
   no-surface are 0.89 of each other on the freeway (37% and 42% of the
   frame, MoGe leaving out the far skyline with the sky), in 13 s where
   MoGe's took 36 to 40.

   *What MoGe is still for*: a frame alone as a mesh, with its lens
   (`set`). The two models' lenses, degrees down the frame, of the same
   footage: 27.6 by MoGe and 30.2 by Depth Anything 3 on the walk (29.3 to
   32.1 over its frames), 30.5 and 28.9 on the freeway; nearer each other
   than on the street of 3 above, 22.8 and 37.2. Whether the other model
   could do this too was then tried, with the first frame of the walk as
   a guide of one frame. `set-views` made a mesh of it in 3 s, and the
   mesh is wrong: its corners run from 0 to 638 across and 0 to 346 down,
   the picture's own pixels, at depths of 0.2 to 2.9, where `set`'s mesh
   of the same frame is seen through a lens of 26.5 degrees. The model's
   answer says why: of one frame its lens comes back as the identity, a
   focal length of 1, and its camera as not moved. It finds a lens by
   seeing frames beside each other, and of the walk's 81 together its set
   of that frame has one, 26.8 degrees, and the same order of near and
   far as `set`'s at 0.83. So a frame alone is MoGe's and stays so, and
   `cameras` and `set-views` now refuse a guide of one frame
   (`"frames": {"least": 2}`) where they answered with a flat mesh and
   no word of it. Which of the two rebuilds a frame of many better was
   not looked at, there being no set of this footage to hold either to.

   *A workflow that read no guide.* `shot-moved` made a shot from a
   picture to start from and the way one clip goes, as tracks of points,
   by a model of its own (6 above), and loaded three files nothing else
   did. A shot here is made from a guide, which says where everything is
   and how the camera moves; told the way of one clip alone, it left the
   rest of its picture about where the first frame had it. It went, by
   that and not by a measure: it was not held again beside
   `shot-directed`, which reads a first frame and a clip's way beside a
   guide. The server then holds six files, one model that makes shots and
   three that read a picture back.

   What it shows, and what it does not. A matte by its name is a line of
   a recipe, and with it a part of footage is made anew, a thing is cut
   out to stand before a title, and, with the mask grown, taken out. Of
   the four models that read a picture, one is gone and each of the
   others has its own work: SAM 3.1 says what a pixel is of, Depth
   Anything 3 how near it is and where the camera was, MoGe 3 what one
   frame alone is the shape of. Every file the server holds is loaded by
   a workflow, and which by which is in
   [shots.md](shots.md#what-the-server-holds). It is two shots, by day, with one thing at a
   time and nothing passing in front of it for long. Not tried: a thing
   that goes behind another and comes back; hair, smoke, glass, and what
   else has no edge, which a matte of white and black cannot say; two
   mattes in one take; a matte led by Kerf's own points on the picture in
   the editor, whose cards still cut by colour and edges; a depth of more
   than 240 frames, which wants reading in parts.
13. **A thing taken out by a model made for that** (2026-11-18). The server
   was given VOID the same day, the model 12 named as wanting: two passes
   of CogVideoX-Fun, `void_pass1` and `void_pass2`, with their own coder
   (`cogvideox_vae`), words' reader (`t5xxl_fp16`) and the flow the second
   pass is led by (`raft_large`). `plate` is the workflow: the guide, the
   mask of the thing as `matte` finds it, and a prompt that says what the
   picture shows once the thing is gone. The same two shots as 12, each 81
   frames of 832x480, with the mattes made there.

   *How it is written*, each for a thing found on the way. **The model
   takes five frames and a multiple of eight**, and where it is handed
   another count it rounds down and says nothing: of 81 it would make 77.
   The graph holds the guide's last frame for eight more, asks the model
   for the next count it takes, 85, and cuts what comes back to the
   guide's own 81. **The model reads at about 672x384**, so the graph
   brings the guide to that area in the guide's own shape, by its own
   arithmetic, and brings only what was made under the mask back up: the
   plate is the guide's size, and away from the mask it is 1.9 to 2.1 of
   255 from the footage, the file's encoding. **A mask of the thing does
   not cover its shadow.** The model takes the shadow out all the same,
   which its own picture shows; laid back through the mask alone, the
   footage's pixels put it back. So `reach=` lays the model's picture back
   further than the mask (below). Two other ways to the shadow were tried
   and left: a matte by its name, which `matte` does not find (`shadow of
   the man on the ground` was a speck on the first frames and nothing
   after), and taking the model's picture wherever it differs from the
   footage, which it does everywhere, as much far from the man as beside
   him, being a picture made again.

   *Beside the model that makes shots.* On the freeway the camera does not
   move, so the footage itself shows what is behind the pickup once the
   pickup has gone: the last frames, where they hold still. Each take's
   first 40 frames held to that, over the pixels the pickup then covered
   (34,000 a frame):

   | | From what is behind the pickup, of 255 | Worst frame | Took |
   |---|---|---|---|
   | The footage, the pickup still in it | 68.1 | | |
   | `shot-into`, as 12 took it out | 47.1 | 73.7 | 111 s |
   | `plate`, both passes | 9.7 | 11.3 | 344 s |
   | `plate`, one pass, as it comes | 7.4 | 9.2 | 186 to 211 s |

   Looked at, `plate` has the stack of tires with its striped board and
   the rail before it, as the footage has them once the pickup has passed;
   `shot-into` had a row of tires of its own making, and no board. On the
   walk, where the camera turns and nothing shows what was behind the man,
   `plate` has the hillside and the path through him on every frame looked
   at, where 12 had a patch.

   *His shadow.* The ground beside the man's feet at frame 72, where his
   shadow falls, by its darkest twentieth, of 255: 89 in the footage and
   226 on the path just below it. With `reach=0` the plate has 93 there,
   the shadow as it was; with `reach=72`, 224, the path. It is 0 as it
   comes: what it reaches is the model's picture and no longer the
   footage's, and how far a shadow goes is the shot's.

   *One pass or two.* The model comes with a second pass over the first,
   for a picture that holds stiller. Measured, it did not:

   | | From what is behind the pickup | From a frame to the next, there | The walk, from a frame to the next, within 60 px of the man | Took, the freeway and the walk |
   |---|---|---|---|---|
   | One pass | 7.4 | 0.68 | 7.73 | 211 s, 199 s |
   | Two | 9.7 | 0.79 | 7.74 | 344 s, 353 to 367 s |

   So `plate` is one pass, and `refine=true` asks for the second. It is two
   shots of five seconds; a longer one, or one with more in it that moves,
   may be what the second pass is for.

   *As many frames as it reads.* 240 frames of the walk at 960x540, of
   which it reads 197 and says so: a plate of 197 frames at the guide's
   size in 638 s, the man gone on each frame looked at.

   *With no mask* it is refused before the server is asked (`this workflow
   makes nothing without guide_mask=, which the line does not give`),
   where the server would have run a graph with its middle left out.

   *Layers apart, in the document* (`fixtures/shots/walk`, by `kerf make`
   and no other way). `gone` is the man's matte and the plate of it with
   `reach=72`, 22 s and 436 s: the plate took twice what it took above,
   the server having the model to load again after the matte's. `moved` is
   a sequence Kerf draws: the plate on the lowest track, and over it the
   footage shown through the man's matte, a quarter of the frame further
   along and half as large again, the same set on the matte's clip.
   Looked at on three frames, the man walks where he was put and the path
   is whole where he was. He has no shadow there: it is not in his matte.

   What it shows, and what it does not. Taking a thing out is a line of a
   recipe, by the model made for it, and what comes back is the footage
   with that one thing gone: held to what the footage itself shows, it is
   a sixth as far off as the model that makes shots was. With it a thing
   is a layer of its own over a picture that is whole. It is two shots of
   five seconds and one of eight, one thing in each. Not tried: a thing
   that others pass in front of, two things taken out at once, a thing
   that fills much of the frame, footage larger than 960x540, and what the
   model's own affected region, the level of its mask this workflow does
   not write, does for a shadow in place of `reach`.
14. **A part of footage by an agent's tools alone** (2026-11-18). A
   playbook, `footage-parts`, now says of 12 and 13 what an agent cannot
   see for itself: which model fits what is asked, what to look at after
   each making, and what a take asks for next. It was held to the tools,
   not yet to an agent: an MCP client worked through it by hand on the
   walk, from a project of the footage on one track, with `kerf-mcp`'s
   four tools and no other, `permissions: auto`, and the same server.
   **Kerf's own chat agent was not run**, there being no key for one, so
   whether a model follows the playbook unaided is still to see. Asked for
   the man further right and larger:

   | Step | Took | What it showed |
   |---|---|---|
   | `man`, his matte, `objects=1` | 15 s | white on him alone, on each frame of the sheet |
   | The layers, the footage under them, `x=0.2 scale=1.4` | no making | the man twice: once where he walked, once where he was put |
   | `gone`, the matte and the plate, `reach=64` | 218 s, 197 of them the plate | the path whole and no shadow, on each frame of the sheet |
   | The plate on the low track | no making | the man once; at the last frames his feet below the frame, the layer having grown from the frame's middle |
   | `y=-0.12` on his clip and his matte's | no making | on his feet on the path throughout |
   | `shown`, `man, tree` | 19 s | four things in four colors; their numbers too small to read in a sheet, and plain on one frame |
   | A matte of frames 20 to 60 alone, its guide a sequence of that part (`render s2 {part.mp4}`) | 14 s | 41 frames, the man where the footage has him on each |
   | `shirt`, his shirt's matte and `shot-into`, `grow=4` | 171 s | a red flowered shirt, the rest the footage's own |

   What was found is in the playbook or a model's own words. **`reads`
   does not tell the two that read a mask apart**: both list `guide_mask`,
   and `shot-into` still said of itself that it takes a thing out, which
   12 measured it not to do; its `about` now says so, and names `plate`.
   **A plate laid on its track before it is made** is a finding
   (`missing_media`) and an edit with no preview, so it is made first and
   laid then. **A layer made larger grows from the frame's middle**, and
   what is set to mend that is set on the matte's clip too. **The numbers
   `matte-shown` draws** are read on a frame, not on a sheet. **A part
   further into a file** is a sequence of that part drawn as the guide,
   and what is made of it starts where the part does.
