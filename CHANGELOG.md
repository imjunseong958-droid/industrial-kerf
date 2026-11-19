# Changelog

What changed in each release of Industrial Kerf. A release is a `v<version>` tag
on `main`; its section here becomes the release's notes on GitHub
(`.github/workflows/package.yml`), so write it for someone using Kerf.

## Unreleased

- An agent's `look` with `shows` reads every clip that draws on a frame,
  whichever track it is on. It read 32 from the lowest track up, so of a
  street of 55 solids the cart on the top tracks, its subject, was left
  out. A clip's frame is drawn beside the others', on as many cores as
  there are: that street's frame took 390 ms for its 32 clips on four CPU
  cores and takes 200 for all 55. In a program of more than 144 clips, a
  clip is no longer read as hidden by one that is not on the frame
  ([docs/architecture.md](docs/architecture.md#what-directs-a-model)).
- A guide of solids draws several times faster under a light that casts a
  shadow, and to the same picture. A street of 53 solids with a soft shadow
  (`gen=light@1 shadow=0.55 size=2.5`) took 42 s a frame at 1080p on four
  CPU cores and takes 2.5, where it takes 1.8 with no shadow at all; with a
  hard shadow it took 4.8 s and takes 2.1; and three solids with a soft
  shadow took 2.4 s and take 0.4. A ray to a light asked every mesh of the
  scene, and now asks the ones it comes to; the many rays to a light with a
  `size` are sent only where a mesh is between the place and the light; and
  a mesh of many triangles, a ball or a model, no longer costs each ray
  that comes to it a walk through much of its tree. `motion_bench` has the
  scenes
  ([docs/architecture.md](docs/architecture.md#motion)).
- An agent's `look` with `eye` says where the eye is either side of each
  cut between two shots: where the subject of the shot that ends is on the
  frame and where the next one's is, how far that jumps, how much larger or
  smaller it is, and whether it turns back. A model makes each shot by
  itself, so nothing held one to the next but a person looking. It is read
  from the two shots' guides, which the document has, before either is
  made, and from the takes once they are, side by side: where the guides
  jump the guide is mended, and where they hold and the takes do not the
  shot is rolled again. A clip is read where the program has it, trimmed,
  retimed, scaled, turned, moved, or cropped, and a guide at another rate,
  size, or shape than its take reads the same. What it cannot place is said
  with why: a clip in depth, a mask, an effect that may move the picture,
  footage with no guide. It is what is so and fails no check, a hard cut
  jumping on purpose
  ([docs/architecture.md](docs/architecture.md#edit)).
- An agent's `look` with `stands` says what each thing in a scene in depth
  stands on, which no frame shows: a crate a hundredth of the frame over
  the road, a car's far wheels in it, a box through a wall. It is measured
  in the scene with nothing drawn, in milliseconds, on every frame of the
  sequence or the ones asked for, and comes with the line that sets a thing
  down (`set crate y=0.839`, or keys where it or its floor moves). So a
  thing is placed roughly and set down by that line, where the `model-shot`
  playbook had its height worked out by hand. What it finds of a guide's
  sequence is among the `notes` of what `look` lists to make. It is what is
  so and fails no check, a ball thrown being in the air, and it names what
  it does not measure: a mesh into another mesh, the outline of a round
  shape within its box, and a clip whose outline is its picture's own
  ([docs/shots.md](docs/shots.md#guides)).
- How a media is made is written in the document, under the media, and no
  longer in a recipe file (`.kerfrecipe`) beside the project. A media that
  Kerf or a model makes has its steps under its line (`render <sequence>
  {guide.mp4} …`, `generate model=… prompt="…" …`, `join …`), an edit script
  writes and changes them (`step <media> add <step>`, `step <media> set <n>
  seed=8`, `step <media> remove <n>`), and undo takes a step back as it does
  any edit. An agent that judges a request needs a model writes the steps
  with its edit and calls `make`, with no file to write first; how much it
  runs without asking is still yours to say in `~/.kerf/agent.json`. A take
  keeps the steps it was made from, each making is still a new take, and a
  media says it wants making when its steps, the footage they read, or the
  sequence its guide is drawn from has changed. A file a step names is
  named from the project's folder (`guide=footage/walk.mp4`), a guide's
  mesh files are found from its clips with no `needs` line, and takes are
  kept in a folder named for the media's file (`shots/open.takes/`).
  ([docs/shots.md](docs/shots.md#steps))
- **Recipe files are no longer made from.** A project that has them opens,
  plays and exports as it did, and its takes are still listed and chosen
  among, but `kerf make`, the editor's Make and an agent's `make` refuse a
  media whose steps are still in a recipe file, and say how they come into
  the document: `kerf carry <media>` (or `--all`), Carry Steps into the
  Document… on the media in the editor's bin, or an agent's `edit` with
  `carry`. Carrying writes the recipe's steps under the media, each file
  named from the project's folder, and takes `recipe=` off its line, as one
  edit that undo takes back. The takes the recipe made stay where they are,
  and those made from it as it stands are not asked for again for being
  carried. The recipe's file is left where it was. An agent's `edit` no
  longer takes `recipes` or `recipeSet`: steps are lines of its script.
  ([docs/shots.md](docs/shots.md#carrying-a-recipe-into-the-document))
- **A recipe's `run <program>` line has no place in a document**, which
  names no program: what may run on your machine is only what you named, in
  your own `~/.kerf/models.json`, so a document, a preset, or an agent's edit
  cannot say what runs. A renderer or a script is named there as a model is
  and handed its arguments, `{"previs": {"run": ["vork", "render", "{ref}",
  "--out", "{out}"]}}`, and a step asks it: `generate model=previs
  ref=scenes/open.scene.json out={guide.mp4}`. A recipe with a `run` line is
  carried once that line is written so; `kerf carry` says how.
  ([docs/shots.md](docs/shots.md#a-program-of-your-own))
- A preset carries the steps that make a media, and every sequence a step
  draws. A media with steps under its line travels as one the preset makes
  (`path="made:<id>.<ext>"`), its steps naming the footage they read by
  its slot (`guide=slot:slot1`); applying adds it as a new media of the
  project with its steps, and `make` makes it there. So "a thing as a moved
  layer" applied to other footage makes that footage's matte and plate, and
  a blocked 3D scene applied again makes the same shot, with another prompt
  or seed by a `step … set` after it. Footage a making reads is one slot
  for every cut of it, and a matte or a plate takes the bound media's rate
  and length, its clips starting from the binding's `from`. Inspecting a
  preset lists what it makes apart from its slots. Applying a preset whose
  track matte names a track above its own no longer fails.
  ([docs/presets.md](docs/presets.md))
- In the editor a made media's bin menu reads Make… and Make Several (they
  were Make from Recipe… and Make Several from Recipe), and a change made in
  a shot's graph, a prompt, a seed, a step added, a take's steps restored, a
  value of a model's workflow, is an edit of the document, one revision that
  undo takes back, where it was a change to a file.
- Both shot fixtures (`fixtures/shots/car`, `fixtures/shots/walk`) have
  their steps in their documents.
- `kerf-comfy` comes with `matte`, for a ComfyUI server that has SAM 3.1: a
  thing in footage found by its name and followed, as a matte, white on
  black on every frame (`generate model=matte guide=<footage> prompt="man"`).
  `objects=` is how many things are followed, `matte-shown` draws each with
  its number over the footage, `which=` keeps some of them by number, and
  `inside="x,y x,y"` names a thing by points on it in the first frame in
  place of words. A matte is the mask `shot-into` reads, and media the
  document cuts footage by (`key.trackmatte`), so a title stands behind a
  man who walks in front of it.
- `shot-into` changes the part of the footage its mask is white on and
  nothing else: what the model made is laid back over the footage through
  the mask, where before the whole frame was the model's and what it kept
  came back worse. `grow=` makes anew that much around the mask as well, and
  `feather=` is how softly the two meet. A thing is taken out by `plate`,
  below, and not by this.
- There is one `depth`, and it holds still: the frames are read together,
  so a place that does not move in the picture keeps its grey from frame to
  frame, three times as well as before and in about a third of the time.
  `depth-whole` is gone, and `cameras` writes a camera's path and no video:
  a recipe that asked `cameras` for a video asks `depth`. `depth` no longer
  takes `refine_steps=` or `output=`; a sky has a distance in it, and what
  is sky is a matte's to say (`prompt="sky"`). Print a model's entry again
  (`kerf-comfy --workflow depth --describe`).
- `cameras` and `set-views` refuse a guide of one frame, where they wrote
  a mesh that was the frame's pixels laid flat: their model finds a lens
  only of frames seen together. One frame alone is `set`'s.
  `docs/shots.md` lists the files a ComfyUI server holds for the
  workflows that come with `kerf-comfy`, and which workflow loads which.
- `kerf-comfy` comes with `plate`, for a ComfyUI server that has VOID: a
  thing taken out of footage, with what was behind it there
  (`generate model=plate guide=<footage> guide_mask={man.mp4} prompt="an
  empty path"`). The mask is a matte of the thing, and the prompt says what
  the picture shows once it is gone. Outside the mask the plate is the
  footage's own pixels; `reach=` takes the thing's shadow with it, and
  `refine=true` has the model's second pass go over the first. With a plate
  under it, a thing cut out by its matte is a layer to move, make larger or
  leave out, and the picture behind it is whole. A plate wants its mask:
  without `guide_mask=` it is refused before the server is asked.
- `shot-moved` no longer comes with `kerf-comfy`. It made a shot from a
  picture to start from and the way one clip goes, and read no guide.
  Beside a guide, `shot-directed` reads a first frame (`first=`) and where
  a clip goes (`moves=`); with no guide there is nothing that does. The three
  files a server held for it alone can go: `Wan21-WanMove_fp8_scaled_e4m3fn_KJ`,
  `lightx2v_I2V_14B_480p_cfg_step_distill_rank64_bf16` and `clip_vision_h`.
  A workflow of your own still reads a clip's way as tracks at
  `{moves_box}`.
- Kerf no longer runs MobileSAM: `look` with `track` takes no `model`, and
  nothing is fetched to `~/.kerf/models`. Its cut by colour and edges is as
  it was. A model's matte is `matte`, above, which held a man before a
  hillside and a pickup among cars where the cut with MobileSAM behind it
  held neither.
- A workflow of your own reads points a line gives at `{inside}` and
  `{outside}`, leaves a node out when the line has no words (`"given":
  ["prompt"]` under `without`), and may end in a node that saves a preview
  and nothing else: `kerf-comfy` fetches that where nothing was saved to
  keep.
- An agent is told how to work on a part of footage without being led
  through it. A new playbook, `footage-parts`, is in every agent
  conversation beside `model-shot`: which of the models set up fits what is
  asked, a matte by a thing's name, a thing taken out, a part made anew, a
  thing as a layer of its own; what to look at after each making; and what
  a take asks for next, so that a man moved in his footage is his matte,
  then a plate under him where the footage still had him, and then the
  layers. How much of that runs without asking is still yours to say, in
  `~/.kerf/agent.json`. `shot-into`'s entry no longer says it takes a thing
  out, which is `plate`'s: print it again (`kerf-comfy --workflow shot-into
  --describe`).
- In a model's workflow, a value given to an input a blank fills is refused
  as the step's to fill, with where it is set, whether or not the ComfyUI
  server has said what the input takes. Once the server had listed the
  input's choices it was refused for not being among them, and a value that
  was among them was refused all the same.

## 0.1.4

- A shot made from a recipe opens as a graph of how it is made: Open Graph on
  the media, or a double-click on its clip. Its steps and what they read are
  cards wired by what goes along each edge, a picture in motion, a still, the
  way a thing goes or a number over time, ending in the shot's takes. Choosing
  a card sets what it holds: a model's prompt, seed and knobs, a guide's pass
  and size, a signal's curve drawn by hand; an edge drawn from a card's port to
  a model hands it over. A change is written into the recipe where it stands,
  comments and order kept, and Make again makes a new take from it; Several,
  Stop and choosing a take are there too.
- A model that runs on a ComfyUI server opens, in its shot's graph, into the
  workflow it is run with: every node and every input, the blanks the recipe
  fills marked, and any other value to set. A value set is kept in a copy of
  the workflow in the project, beside the recipe, which that shot's line then
  names (`workflow=`) and `kerf-comfy` runs for it; nothing on the server and
  no other shot changes, and a take is held to the copy.
- Kerf in the conversation can do in a shot's graph what you can: read the
  graph and the workflow behind a model, set one word of a recipe's line,
  wire an input in or let it go, and set a value of the workflow. Its change
  is kept as the recipe's next version and a new copy of the workflow, never
  over yours, and the open graph shows it once it is accepted, where you had
  it. (`look` with `graph` and `workflow`, `edit` with `recipeSet` and
  `workflowSet`, for an MCP client too.)
- `kerf-comfy` comes with `shot-directed`: a shot that follows a depth guide
  (`guide=`), looks like a reference (`ref=`), has its subject go where a clip
  goes (`moves=<clip>`) and lets the guide go where a signal is 1
  (`signal=<clip>`), all on one `generate` line, or any of them. A workflow of
  your own can lay one thing over another the same way: `without` says which
  node goes when an input of it is not given, and which stands in for it.
- A shot's takes are chosen among in its graph by looking and by number: each
  take shows its picture, a guide shows as it was drawn for the take, and
  Hold to guide says how much of where each take has the shot's subject and
  where its line asked for it (`moves=<clip>`) is the same, with the closest
  said. No model is run. Kerf in the conversation reads the same numbers
  (`look` with `takes`).
- A shot longer than a model makes at once is made in parts and joined: a
  recipe's `render` draws a part of a sequence (`from=`, `seconds=`), a
  `generate` goes on from the last frame of the part before
  (`first={a.mp4}`, with `from=` for its signals and moves), and `join` sets
  the parts end to end. The plan says how when a recipe asks for too long a
  shot, the graph shows the join as a card of its own, and `shot-directed`
  reads the first frame.
- `kerf-comfy` comes with `still`: one picture from the prompt, or of what a
  reference shows, made by the model that makes the shots, for a later step's
  `ref=` or `first=`. A workflow of your own that makes one frame is a still
  the same way.
- Steps are added and deleted in a shot's graph: Add step puts a guide, a
  model or a join into the recipe as a line that reads as it stands, and its
  card opens to set it; Delete step, in a step's card, takes its line out.
  Kerf in the conversation does the same (`edit` with `recipeSet`, `add` and
  `remove`).
- A take keeps the recipe it was made from, its seeds and its workflow with
  it, and the shot's graph goes back to it: Restore take's recipe, in the
  takes' card, for the take the shot is when the recipe has changed since.
  Kerf in the conversation does the same (`edit` with `restoreTake`). Takes
  made before this keep none.
- In a model's workflow, an input says what the ComfyUI server takes there:
  a sampler or a model file is chosen from the names that server has, and a
  number is held to its least and most. `kerf-comfy --workflow NAME --inputs`
  prints it, and Kerf in the conversation reads it with the workflow.
- A signal is set from a sound in a shot's graph: From a sound, in a
  signal's card, lists the clips of its sequence that play one and keys the
  signal to 1 where that sound's sounds start, as Kerf in the conversation
  already could.
- While a model on a ComfyUI server makes a shot, the line says which step
  of how many it is on (`running, step 7 of 20`), where the server's
  websocket can be reached; elsewhere it says how long it has run, as before.
- A link of a model's workflow is pointed at another node in the shot's
  graph: draw an edge from a node's port to the node that should read it. It
  is kept in the shot's own copy, held to what the ComfyUI server says each
  node gives and takes, and refused where the graph would go round. Kerf in
  the conversation does the same (`workflowSet` with `link`).
- A clip is read from footage in a shot's graph. Follow the picture, on the
  card of a clip a recipe hands a model the way of (`moves=`), draws the
  clip's first frame: press on something and the clip follows it for as long
  as it runs. Cut out of the picture, on the card of a guide that is a matte
  (`matte=`), masks its clip to the thing pressed on, as it moves and changes
  shape. Kerf in the conversation follows a clip's whole length with `track`
  when no range is given, and no longer follows the clip itself.
- A project says its cast and styles once, in `cast.kerfcast`: who and what
  the shots show and how they look, in words and pictures. A `generate` line
  names them (`cast=`, `style=`), and changing one asks again only for the
  shots it is in. The bin's menu has Cast and Styles… to set them; Kerf in the
  conversation adds to the file with `edit`, and the file's comments are kept
  either way.
- Several takes of a shot are made at once: Make Several from Recipe makes
  two, three or four, each with its seeds one on from the last, as do
  `kerf make --rolls N` and Kerf in the conversation (`make` with `rolls`).
  `kerf shots` lists every shot a recipe makes, with its take, its cast, and
  whether it wants making and why.
- `brand.kerfbrand` says a project's colours, fonts, tone and the words it
  says and avoids. Kerf reads it, and the check says, as information, when a
  title is set in a font or colour the brand has not, or a title or caption
  says a word it avoids.
- A cut in speech is placed and checked by the recording's own sound, not by
  a fixed level and a transcript's loose word times. `snap=pause` finds
  shorter pauses and leaves more air after a word; the check says when a cut
  falls in sound no transcript word accounts for, and names long pauses and
  words said twice that the cut still plays (`look` with `cuts` gives the
  numbers). Media indexed by an older Kerf is looked at again first.
- Exporting again after an edit draws and encodes only what the edit
  touched: a render keeps its picture in stretches of about four seconds and
  its sound in pieces. Five minutes of 1080p that took over two minutes to
  export takes under ten seconds again after one clip changes.
  `KERF_SECTION_CACHE` moves what is kept or turns it `off`, and
  `KERF_SECTION_CACHE_GB` sets its size, 10 by default.
- Chat on Claude Code no longer needs a `kerf-mcp` built beside `kerfd`: where
  there is none, the editor serves Kerf's tools to Claude Code itself.
- Kerf's replies read as text, not Markdown: bold, code and list items show
  as such, in the thread and in the sidebar's titles.
- A shot made from a recipe keeps its name in the bin and on the timeline
  after each take, instead of becoming "001", and making it says its name.
- A failed turn's error leaves the bar at the top once the next turn starts;
  a bar no longer covers the Media sheet's tabs and close; an edit Kerf
  refuses says why without its code; and a finished export names its file.
- Edit ▸ Extract takes a clip's time out of every track, for when Ripple
  Delete refuses because it would cut a clip on another track.
- A Draft export goes to `deliveries/` as `<project>_draft`, beside the
  master, instead of the project's top folder under its job's id.
- `kerf-comfy` passes over an FFmpeg that is there but does not start, as
  Kerf does, so a Homebrew `ffmpeg-full` an upgrade broke no longer fails
  every making while a working FFmpeg is on `PATH`.
- A set a model makes, a `.glb`, is kept as its media's take instead of being
  refused as a file no probe reads, and the `gen=mesh` clips that read the
  media's file read the take; choosing another take moves them with it.
- When a ComfyUI node fails naming its own inputs (`confidence_threshold`),
  `kerf-comfy` says which name a `generate` line sets them by (`confidence=`).
- Making a shot says so first when its guide would be black: a depth,
  relative or normals pass of a sequence with nothing in depth, as one that
  only nests another. A `generate` line that names `out=`, `seed=` or any
  one-value name twice is refused, where the first was dropped unsaid.
  `kerf shots` counts a mesh clip that reads a set as playing it.
- The conversation reads as a coding agent's chat does: what you asked as a
  bubble at the right, Kerf's answer under it, with "Working" and the step it
  is on while it works, folding to "Worked for 1m 12s · 6 steps" once done.
  A proposal's card sits under the answer that made it.
- Conversations are kept beside the project (`.kerf/conversations/`), each
  with what Kerf did in it; the sidebar lists them by day, with a search, and
  archives, renames and deletes them. Going on in one, Kerf hears what was
  said before.
- Review notes are gone: the comment threads, blocking notes that held an
  export back, the guest view, drawings on the frame and the `note` tool for
  agents. Ask Kerf in the conversation instead.
- While anything runs, light goes round the picture (blue while Kerf works,
  amber while a render or a model does) and the chip on it says what runs; a
  clip being made shows moving stripes on the timeline, and the Media sheet's
  tiles say whether each is made, being made, queued or offline.
- The conversation is 560 wide beside the picture, as in the design.
- The composer says how far Kerf goes without asking: **Ask before edits**
  (every edit a proposal, as before), **Accept edits** (edits go in as Kerf
  makes them, each one step for Undo) or **Auto** (edits go in and Kerf makes
  the shots it asks for). Its chip opens the three, Shift+Tab goes round them,
  and Settings ▸ AI connection has the same choice. It holds for what you ask
  in the editor; agents over MCP still follow `~/.kerf/agent.json`.
- Beside it the composer names the model that answers, with a menu where there
  is a choice, and a ring shows how full Kerf's context is.
- The sidebar is wider, and its edge drags. Every folder unfolds to its own
  conversations, the latest first, with the first six showing; a conversation
  in another folder opens that project on it. A folder has a menu: New
  Conversation, Open Project, Show in Finder, Copy Path, Remove from Sidebar.
- Opening and closing the panel, the sidebar and Media moves them from the
  window's edge, against the edge of what makes room, in less than half the
  time. The panel no longer jumps in over the conversation, and what is said
  goes with its panel instead of jumping to where it will end.
- Media opens as a column of its own between the conversation and the picture.
  It used to lie over half of the conversation, the composer included.
- Settings ▸ Appearance chooses the fonts: Inter, Pretendard or the system's
  for the interface, and for the conversation the same or a serif (Noto Serif,
  with Nanum Myeongjo for Korean) in three sizes. Korean under Inter is set in
  Pretendard instead of whatever the system has. The fonts come with Kerf.
- Settings' pages are rows, not boxes: each setting's name with a short line
  under it and its control at the row's end, a hairline between rows. A key of
  your own shows one protocol's fields at a time and keeps what you type as
  you leave a field; update notices and MCP tools are switches.
- The sidebar is narrower, quieter and set a size larger. New and Search are
  an icon and a word with nothing round them, and Search is typed into where
  it is. Under them is the open folder's name over its conversations, and
  nothing else: pressing the name opens the other folders. A line marks where
  the sidebar ends, and its button keeps beside the window buttons.
- The toolbar is titled with the conversation, and the chevron after its name
  renames, archives or deletes it; the conversation no longer has a strip of
  its own under the toolbar. The window is still titled with the project.
- A new conversation offers the folders, the open one ticked, with Open Folder
  and New Project, and under them the latest conversations, or things to ask
  where there are none yet.
- The composer's box holds only what you ask and Send, with no line over it.
  Under it are the project's folder, which opens the other folders, the
  model and the mode. A request still goes with the playhead's time, without
  a clock saying so; untick Send playhead time in the conversation's menu and
  a struck clock says it is off, and puts it back when pressed.
- Settings opens a little above the middle of the window, and the toolbar
  dims with the rest of the window behind it. On a Mac the project file's
  icon no longer shows beside the window buttons, which sit in the middle of
  the toolbar's height, on the sidebar's column of icons.
- The interface is set in IBM Plex: IBM Plex Sans, with IBM Plex Sans KR for
  Korean and IBM Plex Mono for timecodes. It comes with Kerf; Inter,
  Pretendard and your system's face are still in Settings ▸ Appearance.
- The toolbar is one line: the window buttons on a Mac, the sidebar's and the
  panel's buttons, which are plain icons, the title and Export all sit on its
  middle, the first and the last the same distance from the window's edges.
  The line the sidebar ends on holds still while the panel opens and closes,
  and on a Mac no ⋯ comes and goes in the toolbar with the sidebar.
  The window's foot is one line too: the account and Settings in the
  sidebar, and the folder, model and mode under the composer, at one height
  and one size. The timeline's timecode and track names start where its
  strip of tools does.
  The models Kerf offers are named with a capital (Default, Opus).
- Kerf opens on the project you last worked on. The start window, with its
  one question, is gone: a new conversation already lists your folders, with
  Open Folder and New Project, and that is what the first run shows.
  Nothing goes away for want of a project: the sidebar keeps New and Search,
  off, and says No folder where its name would be, the composer's folder chip says
  the same, and Export stays in the toolbar, off until there is a cut.
- Text is the colour of paper, not white: the same greys, warmed a little.
  The sidebar, the toolbar's title, the composer's chips and Settings are set
  a size larger, and what you type sits in the middle of the composer's box.
- Menus, popovers and the context card share one look, and the colours that
  say something are faded: ochre for a check to fix, sage for one passed and
  for what a proposal changes, and a violet playhead. Every track in the
  timeline is one height and its head says what kind of track it is; the
  hosted model is called Kerf.

## 0.1.3

- A first cut Kerf makes lands in the conversation as a card with its picture:
  the first clip's poster, its length and a play mark, and moving over it runs
  through its clips. While Kerf is still cutting, the card is already there,
  with light running round its picture as each clip is placed. Pressing it
  grows the picture out of the card onto the stage and brings the timeline in;
  closing the panel puts it back. Its title sits beside Reject and Accept.
- The editor draws less while panels open and close and while the picture
  plays: the timeline moves as a picture of itself, the conversation is laid
  out once, unchanged threads are kept rather than made again, and nothing is
  drawn under what covers it. A long conversation no longer makes each new
  comment slower.
- On a Mac in a virtual machine, Kerf encodes in software, since VideoToolbox
  there can stop answering and leave an export waiting for ever;
  `KERF_ENCODER=hardware` still asks for it.

## 0.1.2

- On Windows, a `kerf://` review link opens in the Kerf you already have open,
  and brings it to the front, instead of starting another. Linux does the same.
- On Linux, `./install-desktop-entry.sh` in the unpacked folder adds Kerf to your
  applications and has `kerf://` review links open in it, for you alone;
  `--remove` takes it away.
- A `.deb` for Debian 13, Ubuntu 24.04 and what comes from them:
  `sudo apt install ./Industrial-Kerf-…-linux-x64.deb` installs Kerf with what it
  needs from the system, adds it to your applications, opens `kerf://` review
  links in it, and puts `kerf`, `kerfd` and `kerf-mcp` on your `PATH`;
  `sudo apt remove industrial-kerf` takes it away. The `.tar.xz` is still there
  for other distributions, and for those who want a folder.
- A title can turn its font's OpenType features on and off:
  `features="tnum zero"` gives tabular figures and a slashed zero, `smcp` small
  capitals where the font has them, `-liga` turns ligatures off, and `ss01` or
  `salt=2` take a stylistic set or an alternate. An agent's `look` with `font`
  names the features a font has.
- Kerf has an icon of its own, a crane cutting a clip: on its windows and in the
  taskbar, in the Dock and Finder on a Mac, on `kerf-qt.exe`, the installer, the
  Start menu and `kerf://` links on Windows, and among your applications on Linux
  with the `.deb`, or once `./install-desktop-entry.sh` has run.
- Kerf for Mac needs macOS 26 or later, and on an older macOS it now says so
  instead of failing to open.
- A number that takes keyframes can be an expression of the clip's time:
  `rotation=(t*90)`, `x=(sin(t*pi)*0.1)`, `opacity=(0.5+0.5*noise(t*3))`. The
  Inspector shows what it gives at the clip's first frame; setting the value
  there replaces it.
- `KERF_GRAPH_RENDER=1` and `kerf-engine render --graph`, which had FFmpeg draw
  the picture as renders did before Kerf's own renderer, are gone. Renders,
  playback, and previews already drew with that renderer; nothing about them
  changes.
- A media line can say what its file is made from:
  `recipe="shots/open.kerfrecipe"` names a recipe, a text file with the steps
  that make it: a program to run, a sequence of the project for Kerf to
  render, a model to ask. `kerf make <media>` shows the steps, asks, and runs
  them. A 3D render or a video model's shot can be made again from the
  project. In the editor, such a media has Make from Recipe… in the bin's
  menu, which shows every step and asks before any runs.
- Each making is a take, kept beside the ones before it, and the media is the
  newest: `kerf takes <media>` lists them and `kerf takes <media> --use 1`
  goes back to one. Kerf says when a shot wants making again and for what (its
  recipe, a file the recipe reads, a sequence it renders), and trimming,
  moving, grading or titling a made shot never asks for it again.
- A recipe's `generate` line asks a video or image model for the shot: a
  prompt, a guide to follow, first and last frames, references, a seed. Kerf
  comes with no model. One is a program you name in `~/.kerf/models.json`,
  which is handed what was asked as a file; see
  [docs/shots.md](docs/shots.md#models). The repository has such a program
  for a model on a ComfyUI server, `kerf-comfy`, which the packages do not
  carry yet: it fits the guide to the model's size and rate, asks the server,
  and writes the take
  ([docs/shots.md](docs/shots.md#a-model-on-a-comfyui-server)).
- `kerf-comfy` comes with four workflows that read a picture back, for a
  ComfyUI server that has models for it: `depth` and
  `depth-whole` make a video of how near each pixel of a guide is, `set`
  rebuilds a frame as a `.glb` mesh with the frame as its colors, and
  `cameras` writes where the camera was on each frame. Each is a model
  to a recipe, `generate model=depth guide={take.mp4} out={depth.mp4}`, and
  what it makes has its guide's own size, rate, and frames
  ([docs/shots.md](docs/shots.md#a-picture-read-back)).
- What a model reads back comes into the document. An agent's `look` with
  `mesh` and `from` writes a camera a model found as a `gen=camera@1` clip
  with keyframes: the path a model wrote for a shot, or a lens alone
  for the camera a rebuilt set was seen by, placed against the set's mesh
  clip. `look` with `follows` holds a take's depth, read back from it, to
  the guide its sequence draws, clip by clip: how far the take has each
  clip's pixels from where the guide has them in the order of near and
  far, which says of a take whether its subject is where the guide put it.
  For these `kerf-comfy` writes the lens a rebuilt set was seen through
  (`out={lens.txt}`), and has a workflow, `set-views`, that rebuilds a set
  in the units of the camera path found of the same frames.
- A signal clip shows its number in the timeline, as a curve along the
  clip.
- Option-drag across a signal clip in the timeline draws its number by hand.
- A media's takes are in the editor: **Takes**, on the media in the bin,
  lists every take its recipe has made, with the one it is ticked, and
  choosing another has the media be that take again.
- A glTF file's morph targets are read: a face or a shape that the file
  reshapes by weights is reshaped as its animation has it.
- A recipe's `render` line draws one clip as a matte, `matte=<clip>`: white
  where that clip shows and black elsewhere, with what stands before it cut
  out, which is the mask a model is handed to make a thing in one place.
- An `.rpm` for Fedora: `sudo dnf install ./Industrial-Kerf-…-linux-x64.rpm`
  installs Kerf with what it needs from the system, as the `.deb` does on
  Debian and Ubuntu; `sudo dnf remove industrial-kerf` takes it away.
- Optics Compensation is drawn on the GPU, but where it grows the picture,
  the Advanced Spill Suppressor with its Ultra method, the Simple Choker
  where it draws a matte in, and the Key Cleaner.
- VR Rotate Sphere, VR Projection and VR Plane to Sphere are drawn on the
  GPU, so a 360° frame turned or re-projected no longer sends each frame to
  the CPU.
- `kerf-comfy`'s `depth` reads a take's depth in under half the time: it
  refines once, for a depth a grey of 255 from before. `refine_steps=3` is
  as it was.
- `kerf-comfy` comes with `shot-moved`: a shot that starts as a picture
  (`first=`) and whose subject goes where a clip goes (`moves=<clip>`), the
  clip's box handed to a model as tracks of points.
- `kerf-comfy` comes with `set-splats`: a scene kept as splats (`ref=scene.ply`)
  becomes the mesh a mesh clip reads, on the server's own nodes, with no model.
  And with `moves-shown`, which draws the points `moves=<clip>` hands a model
  over a guide.
- `kerf-comfy` comes with `shot-led`: a shot whose subject goes where a
  clip of the project goes, the clip's box on each frame handed over as a
  layout (`moves=<clip>`), with no picture of the scene.
- `kerf-comfy` comes with `shot-into`: a shot that keeps its guide where a
  second guide, `guide_mask`, is black and makes it anew where that is
  white, which puts a thing into footage where the scene has it.
- `kerf-comfy` hands a signal to a workflow as a grey video at a
  `{signal_video}` blank, for a node that reads a mask a frame, and comes
  with `shot-timed`: a shot that keeps its guide where a signal is 0 and
  is made anew where it is 1.
- A guide can be drawn as which way each pixel faces, `render …
  pass=normals`: a normal map in the camera's space, of clips in depth and
  of meshes.
- A guide can be drawn as depth stretched to each frame's own nearest and
  farthest, `render … pass=relative`, as a model that reads depth from a
  picture gives it; `pass=depth` still goes by the scene's own numbers.
- A `generate` line can hand a model more than one guide, each by what it
  is of: `guide={picture.mp4} guide_depth={depth.mp4}`.
- A recipe's `generate` line can hand a model where a thing goes:
  `moves=<clip>` puts the clip's box on every frame in the request, read
  from the picture, for a model that takes the way a thing goes as points.
  A `kerf-comfy` workflow reads them at a `{moves}` blank.
- `look` with `placed` says where a take has a clip's subject beside where
  its guide has it, in frame heights, read from the take's own picture with
  no model: how far off, which way, and how large the take drew it. It
  goes with `follows`, which says how a take's depth holds and not whether
  the subject is there.
- `look` with `held` reads a take's depth and the place of its subjects at
  once and gives the take one number, to choose between takes by.
- `look` with `mesh` and `from` finds the size of a camera path against a
  set another model rebuilt, from the footage both are of (`fits`), and
  takes a `lens` for the camera, or finds the set's own from the footage.
  With `beside` it puts one mesh's file where another's is, for sets
  rebuilt of several frames of one reading. It also says which frame a path counts its
  places from (`anchor`): a set made of the path's own reading is in that
  frame's space, whichever frame it shows, and without it a travelling
  shot's set was drawn from the wrong place.
- Clips in depth are sorted, and cut where they cross, across a track that
  draws nothing: a camera, a light, or a null on a track between two 3D
  clips, or an empty track there, no longer has the upper one laid over the
  lower whatever their depth. A track with a flat clip still parts them.
- A guide can hold solid things: a `gen=mesh@1` clip is a box, a ball, or a
  cylinder (`kind=`, with `width`, `height`, and `length` in frame heights,
  and a `color`), or with `file="car.glb"` a glTF model at rest, `height`
  tall and a color a part, with every side it has, so a camera can go round
  it. A part the file gives a picture (its material's base color texture) or
  a color at each corner is drawn in those, as they are and unlit.
  `animation="Walk"` plays one of the file's animations, bones and all, as
  the clip runs, and `seconds={0f:0 48f:1}` retimes it. An agent's `look`
  with `mesh` says which animations and cameras a file has, and writes one
  of its cameras as a `gen=camera@1` clip with keyframes, so that a set
  comes with the camera that saw it and what you add stands in it. It is
  placed, keyed, and parented as a clip in depth is, lit by the sequence's
  lights, and is in the ids and depth passes and in an agent's `look` with
  `shows` as any clip is. It casts its shadow on other clips in depth, takes effects and masks on
  its picture as the camera sees it,
  and no longer sends its whole frame to the CPU: its triangles are drawn
  there and laid among the rest on the GPU, where no light casts a shadow.
- A recipe can tell a model when something happens, and not only where and
  what: a `gen=signal@1` clip draws nothing and holds a number from 0 to 1
  that you keyframe, and `generate … signal=<clip>` hands the model what it
  comes to on each frame. `kerf-comfy` gives it to a workflow at a `{signal}`
  blank, a number for each frame the model makes; the workflow it comes with
  reads none yet. Changing a signal asks for the shot again; nothing else in
  its sequence does ([docs/shots.md](docs/shots.md#signals)). An agent's
  `look` with `signal` writes a signal's keys from the onsets of a sound on
  the timeline, so that what a model is told happens on its beats.
- A recipe's `render` line can draw a pass in place of the picture:
  `pass=ids` gives every clip one flat color of its own, and `pass=depth`
  how near each pixel is, for a model that follows one
  ([docs/shots.md](docs/shots.md#passes)).
- An agent's `look` with `shows` reads what shows of each clip on a frame:
  where it is, what hides it, and what has left the frame. A floor drawn
  over what stands on it is said before a making is asked for.
- A recipe's `render` line takes a size and a rate
  (`render s2 {guide.mp4} size=832x480 rate=16`), so a guide is drawn as the
  model reads it and not at the sequence's own size and rate: a guide that
  took 54 s to draw took 8. A picture of another shape fills the size, with
  what is over cut off evenly. What `render` draws no longer has a sound
  track.
- A model's entry in `~/.kerf/models.json` can say what the model takes,
  under `takes`: the picture it makes, its rate, its longest shot, and the
  files it reads. An agent's `look` lists it with the model, with the
  `render` line that draws a guide at that size and rate, so a recipe is
  written for the model it asks. `kerf-comfy --workflow shot --describe`
  prints the entry for a workflow
  ([docs/shots.md](docs/shots.md#what-a-model-takes)).
- A project can be worked on from a terminal with no editor open. `kerf load`
  takes a `project.kerf` you changed by hand as the project, showing the
  lines that change first; `kerf proposals` lists what an agent proposed,
  and `kerf accept` and `kerf reject` answer it. `kerf make` and `kerf
  takes` now say so when the file is not what Kerf has, before anything
  runs.
- While a shot is made from its recipe, the editor shows what its steps say
  as they say it, and **Stop Making** on the media in the bin stops it. A
  step that is stopped is asked to end before it is made to, on macOS and
  Linux, so a model's program can take back the job it gave a server:
  `kerf-comfy` does, in the editor and on Ctrl-C in a terminal, where before
  the job ran on with nobody to fetch it.
- Before a shot is made, `kerf make` and the editor note what its recipe asks
  that its model has said it does not take: a guide drawn larger than the
  model reads, a shot longer than it makes, a file it has no use for. An
  agent is told the same, and what would refuse the making, as soon as it
  proposes the media or looks, and a new skill, `model-shot`, carries what
  it cannot see for itself of blocking a shot for a model.
- An agent can go round the whole loop of a shot a model makes: it writes the
  recipe as part of its proposal, where you read it before you accept, asks
  for the making with its new `make` tool, and looks at the take. A recipe it
  writes again is kept beside the one before (`open@2.kerfrecipe`), so undo
  goes back, and choosing an earlier take brings back the recipe it was made
  from. By default you are asked: `kerf proposals` lists what it wants made,
  and `kerf make` makes it. In `~/.kerf/agent.json` you can let it do more on
  its own, as a coding agent's permission modes do: `accept-edits` accepts its
  edits as it makes them, and `auto` also runs the makings it asks for, up to
  `makes` a session ([docs/shots.md](docs/shots.md#an-agents-part)).
- A shot that does not fill the frame says the `scale` that does: a picture is
  fit inside the frame first, so it is seldom what the sizes alone say.
- Kerf no longer has 15 more effects and four looks, those that make how a
  shot looks, which is now a model's by the prompt and references of the
  shot's recipe: Glint, Light Leaks, RGB Split, Wonder Glow, Wave Warp, Mirror,
  Posterize and Echo; Bokeh, Focus, Compound and Channel Blur (for a plain
  blur there is `blur.gaussian@1`, under a mask if you like, which Camera Blur
  also was, and a camera's `focus` on clips in depth); Offset (keyframes on
  `x` and `y`) and Write-on (a `gen=shape@1` clip with keys on `trim_end`);
  and the looks VHS, Glitch, Dreamy and Sparkle. The Film look is now grain
  and darkened corners, as `look.film@2`. What is done to a shot once it is
  made stays: color and grades, grain, keys and mattes, tracking, restoration,
  retouching, text, transitions, and time. A project that uses one of those
  gone does not open until its line is changed: the error names the line and
  what to use instead.
- Kerf no longer has 38 effects, none of which a look, a grade or a preset
  used. Gone are those that make a look a video model makes better: Brush
  Strokes, Emboss and Color Emboss, Find Edges, Roughen Edges, Motion Tile,
  Replicate, Strobe Light, Alpha, Echo and Edge Glow, Lighting Effects,
  Volumetric Rays, Lens Flare, Lightning, Cell Pattern, Bulge, Spherize, Twirl,
  Turbulent Displace, Long Shadow, and five of the 360° effects (Chromatic
  Aberrations, Color Gradients, Digital Glitch, Fractal Noise, Glow). And
  those a clip does by itself: Move, Grow, Shrink, Spin, Wiggle and Camera
  Shake (keyframes, an expression such as `x=(noise(t*3)*0.02)`, or a look),
  3D Rotate and Basic 3D (`three_d=true` with `rotate_x`, `rotate_y`, `z`),
  Bevel Alpha (`depth=` and `bevel=`), Circle and Ellipse (`gen=shape@1`), and
  Clone. A project that uses one does not open until its line is changed: the
  error names the line and what to use instead.
- An effect's preview in the browser shows the clip's effects in the order they
  are in, where it had run some kinds before others, and a title or a nested
  sequence now has a preview of every effect.
- Moving the playhead just as an edit lands no longer leaves the viewer on the
  frame it showed before: the picture follows the playhead to where it was put.
- A shape whose offset shrinks it past its middle is gone at once, where the
  editor had stopped for several seconds to find that nothing was left.
- Rolling Shutter Repair now repairs HDR footage in an HDR sequence, where it had
  left the picture as it was.
- The editor is being cut down to the conversation, the picture and the
  timeline. Gone so far: Customise Keyboard, the Graph Editor, the Finish and
  Deliver trays (scopes, the loudness meter, the QC pane, the render queue and
  the paperwork), the Inspector with its Source Monitor, the Effects, Titles and
  Presets browser, Sequence Settings, and "Try 3 versions at once".
- The toolbar is cut down to the sidebar's button, the project's name, Export
  (once there is a cut) and one button for the picture and timeline. The ≡ menu
  is gone: your account sits at the foot of the sidebar and opens Settings,
  Media, Import and View. The format picker, Follow Kerf and Media's button moved
  to the timeline's strip. "Edited 2 min ago" is the title's tooltip; the bar
  speaks only when something is going on.
- Settings is a page in the window, not a dialog: Account, AI connection,
  General and Advanced, with Esc or Back to leave.
  - Edit ▸ Keyboard Shortcuts still chooses between the Kerf, Premiere Pro and
    Final Cut Pro layouts; keys of your own no longer apply.
  - Keyframes and their curves are still the project's, for Kerf and `kerf` to
    write; a motion path is still drawn and dragged on the picture.
  - Sequence ▸ Run QC still runs the checks, and the Export menu's Preflight
    lists what they found. Export still renders a Draft or a Master, the master
    to `deliveries/` as before. The editor no longer writes a delivery's
    paperwork (comments CSV, markers, EDL, captions).
  - A proposal that changes one clip's colour says so in a line, without the
    small vectorscope.
  - Media goes into the timeline from the Media sheet: double-click it, or
    choose it and use Clip ▸ Insert or Overwrite, and the whole of it lands at
    the playhead. A clip's position, crop, sound, effects and titles are Kerf's
    to change: ask in the conversation. The mask tools still draw on the
    picture.
  - File ▸ Save Current as Preset… and Import Editing Preset… still save and
    apply a preset; the list of a project's presets is gone.
- A clip with a Corner Pin on it, which is what tracking a surface writes, now
  draws on the GPU with the rest of its frame, where one had sent every frame
  it was in to the CPU. The picture is the same. How much faster such a shot
  plays and renders has not been measured yet.
- So do clips with a Drop Shadow, a Rounded Crop, a Spacer or a Mosaic: a
  picture in a picture with rounded corners and a shadow, or a face hidden
  behind blocks, no longer takes its frames off the GPU. These are not timed
  yet either.
- Curves, Hue/Saturation, Selective Color, Photo Filter, Unmult and the Cineon
  Converter draw on the GPU too, in SDR and in HDR sequences, so a clip graded
  with one of them keeps its frames there. Curves with a point outside 0 to 1
  is still drawn on the CPU. Not timed yet.
- Lens Distortion and Magnify draw on the GPU as well: straightening a wide
  lens, or a loupe over a detail, no longer sends the frame to the CPU. Not
  timed yet.
- Gradient, Ramp, 4-Color Gradient, Checkerboard and Grid draw on the GPU: a
  gradient behind a title or a grid over a frame keeps the frame there. Not
  timed yet. With these, 45 of Kerf's effects and transitions still send a
  frame to the CPU, where 63 did.

## 0.1.1

On a Mac without Homebrew's FFmpeg, 0.1.0 did not find the FFmpeg inside the app:
media came in at a made-up size, rate and length, and what needs FFmpeg to read or
write a file did not work. Kerf now uses the FFmpeg it comes with, on Windows as
before and on macOS too. Windows was not affected.

- Kerf for Linux (x86-64): a folder that runs on its own, from Ubuntu 24.04 on,
  with its complete source beside it.
- A Windows installer: Kerf installs for you alone, with no administrator, adds
  itself to the Start menu, opens `kerf://` review links, and uninstalls from
  Settings ▸ Apps. The zip is still there for those who want a folder.
- Help ▸ Kerf User Guide opens the guide that comes with Kerf, so it works with
  no connection.
- The Kerf Cloud sign-in is kept in the system keychain, as API keys are; one
  kept in a file by 0.1.0 moves there by itself.
- An API key changed while the system keychain could not take it (locked, or
  access denied) is no longer replaced by the older key the next time Kerf
  opens.
- A crash in the editor's own code, in Qt or in FFmpeg now leaves a crash report
  too, not only one in Kerf's Rust, and Kerf tells of it the next time it opens.

## 0.1.0

The first release, for Windows and for Macs with Apple silicon: a zip that runs on
its own for each, and the complete source of each, since their FFmpeg makes them GPL
version 3. Neither is signed: Windows SmartScreen asks before the first run (More
info ▸ Run anyway), and so does macOS (right-click the app ▸ Open, or System
Settings ▸ Privacy & Security ▸ Open Anyway).

- Edit by asking: Kerf proposes each edit on the timeline, and nothing changes
  until you accept it.
- A timeline, source monitor, Inspector, effects and titles, masks, and
  hundreds of looks, grades and transitions, drawn on the GPU.
- Review notes kept with the project, blocking notes that hold delivery back, and
  `kerf://` links to a version and a timecode.
- Projects shared through Git and GitHub, merged clip by clip.
- Finish checks (loudness, gamut, title safe, captions, flash and black) and
  Deliver: ProRes masters, an EDL, markers, and captions.
- Crash reports kept on your computer, settings that carry over from one version
  to the next, and API keys in the system keychain.
- Help ▸ Kerf User Guide, and Help ▸ About Kerf.
- Kerf tells you when a newer release is out, once a day; Settings ▸ Updates
  turns it off.
