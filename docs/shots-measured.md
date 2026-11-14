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
