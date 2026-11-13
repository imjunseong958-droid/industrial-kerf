# Shots another program makes

> Status: recipes with several steps, takes, what a take was made from, and
> the place a model plugs into are in. No model comes with Kerf, and none is
> set up until a person names one in their own `~/.kerf/models.json`; see
> [Models](#models).

Kerf draws what has to be exact: cuts, titles, color, sound, and the layers
between them. It does not try to draw what a renderer or a video model draws
better. Such a shot is made outside Kerf and comes in as media, and the
project keeps how it was made beside it, so it can be made again:

- A **recipe** is a text file with the steps that make the shot's file.
- Each making is a **take**. Takes are kept, and the document says which one
  the media is.
- A take records what it was made from, so Kerf can say when a shot wants
  making again and for what, and never asks for one that only moved.

## Recipes

A media line names its recipe:

```text
media m3 path="shots/open.mp4" rate=24 dur=96f video=1920x1080 recipe="shots/open.kerfrecipe"
```

and the recipe says how the file is made, a step a line:

```text
kerf-recipe 1
# The opening shot: blocked in a renderer, then filmed by a model.
needs open.scene.json
run vork render open.scene.json --profile std/previs --frame-end 95 --out {guide.mp4}
generate model=dawn prompt="a fishing boat leaves the harbour at dawn, fog" guide={guide.mp4} first=../refs/boat.png seconds=4 seed=7
```

| Line | What it does |
|---|---|
| `needs <file> …` | Names the files the recipe reads, from the recipe's own folder, so that a change to one is seen. A file a `run` reads belongs here; a file handed to a model is counted without it. |
| `run <program> <arguments>` | Runs a program, in the recipe's folder. Quote an argument that holds spaces. It is run as it is named, supervised, never through a shell, so `$HOME` and `;` are only characters. |
| `render <sequence> {name} [size=WxH] [rate=<fps>] [pass=ids\|depth]` | Has Kerf draw a sequence of this project into a file, with no sound. A guide blocked with Kerf's own clips in depth, cameras, lights, and titles reaches a model this way, at the model's size and rate when the line names them. `pass=` draws, in place of the picture, which clip each pixel is of or how near it is. See [Guides](#guides). |
| `generate model=<name> …` | Asks a model for a file. See below. |

`{out}` stands for the shot's file, and the last step writes it. Any other
`{name}`, such as `{guide.mp4}`, is a file a step makes on the way and a later
step reads; its extension tells a program the format. Braces around anything
but a file's name stand for themselves.

`generate` says what is wanted in words any video or image model takes:

| | |
|---|---|
| `model=` | The model, by the name the person's models file gives it. |
| `prompt="…"` | What the file should show. |
| `guide=` | A picture in motion to follow: a blocking, a rough render, an earlier take. |
| `first=`, `last=` | The first and the last frame. |
| `ref=` | A picture of who or what is in it, or of how it should look. As many as wanted. |
| `signal=` | A number over time that says when what the prompt asks for happens, and how much: a signal clip of the project, by its id. As many as wanted. See [Signals](#signals). |
| `seconds=`, `size=WxH` | How long and how large. |
| `seed=` | A whole number, for a model that gives the same file for the same seed. |
| `out={name}` | Writes a file for a later step in place of the shot's own, so one model's picture can be the next one's `first=`. |

Anything else on the line, such as `strength=0.6`, goes to the model as it is
written, for the models that read it. A file is a path from the recipe's
folder (`../refs/boat.png`) or a `{name}` an earlier step made. Every file a
recipe names has to be inside the project.

So a model is told three things, each its own way: **where** things are and
how the camera moves, by a guide; **when** something happens and how much, by
a signal; and **what** it all looks like, by the prompt and the references.

## Making a shot

A person makes a shot: `kerf make m3`, or Make from Recipe… on the media in
the editor's bin. Both show every step as it would run and the take it would
write, and ask first; `kerf make m3 --yes` does not ask. An agent asks for one
with `make`, and its asking runs a recipe only where the person has let agents
make on their own ([An agent's part](#an-agents-part)). Opening a project and
drawing it never run one. A recipe that could not run is refused before
anything does, with why: a model that is not set up, a sequence the project
has not, a file it needs that is not there.

What is shown with the steps, as `note` lines, is what a model has said it
does not take ([What a model takes](#what-a-model-takes)) and the recipe asks
of it all the same: a guide drawn at another size or rate than the model
reads, or from a sequence of another shape, a shot longer than the model
makes at once, a file it has no use for, a name it has not got. None of them
stops a making; each is cheaper to mend before one than after.

A step that fails, or a last step that writes nothing, leaves no take and the
media as it was.

While a shot is made, each line its steps say is shown as it is said: how
much of a sequence Kerf has drawn, and whatever a program writes to stderr,
such as where a model's job is in its server's queue. `kerf make` prints
them, and the editor shows the latest in its toolbar, under the project's
name.

A making can be stopped: **Stop Making** on the media in the editor's bin,
which is there in place of Make from Recipe… for as long as it runs, or
Ctrl-C in the terminal. The step that is running is asked to end before it is
made to. On macOS and Linux its program gets SIGTERM, and five seconds to
finish in, so that a program that has asked a server for a shot can take its
job back and say so; one still running after that is killed. On Windows there is nothing to ask with, and it
is ended outright. A stopped making leaves no take, nothing of one, and the
media as it was. A second Ctrl-C ends `kerf` and its step at once, and what
the step asked a server for goes on there.

### From a terminal alone

Nothing here needs the editor open. A scene and its recipe written as files,
by hand or by an agent that has only a shell and file tools, are taken into
the project and made with `kerf`, a person saying yes at each step that
changes the project:

```bash
kerf load                # take project.kerf, as edited on disk, as the project
kerf proposals           # the proposals an agent's `edit` left waiting, and what it asked to have made
kerf proposals <id>      # what one would change: its script and the document's lines
kerf accept [<id>]       # accept the one that waits, or the one named
kerf reject [<id>]       # leave the project as it is
kerf make m3             # run its recipe
kerf takes m3            # its takes
```

- `kerf load` is the editor's Load from Disk. Kerf reads its own state of a
  project, not the file, so a `project.kerf` changed by hand is not the
  project until it is loaded: `kerf make`, `kerf takes`, `kerf accept` and
  `kerf reject` refuse, with that said, before anything runs. Loading shows
  the lines that change and asks. It becomes one revision that undo does not
  go back past, and proposals that wait, made for the project as it was, are
  dropped. A file that does not read is refused with its line, and Kerf keeps
  what it has.
- `kerf accept` shows what the proposal would change and asks, as the editor's
  Accept does. Where more than one waits it wants the one named: by its id,
  or by enough of its start or end to be one proposal's alone.
- Each asks at the terminal, and `--yes` does not ask. With no terminal to
  ask at and no `--yes`, nothing is loaded or accepted.

### An agent's part

An agent writes the scene and the recipe, asks for the making, and looks at
the take; the person says how much of that it does without asking them.

- `edit` takes recipe files with its script (`recipes`, by the path a media
  line's `recipe=` names, each with its text), and they are part of its
  proposal: its script shows each recipe as comments, for the person to read
  before they say yes. A recipe is never written over. A text that is not the
  one there is kept beside it as its next version, `shots/open@2.kerfrecipe`,
  and the proposal moves the media that name it there with `media replace`,
  so undo has them made from the one before; a text a version holds already
  is that version's. Every version of a recipe keeps its takes in one folder,
  `shots/open.takes/`.
- `look` lists what wants making under `toMake`, as an `edit` that names a
  recipe answers it, with what would refuse it (`cannot`) and the notes a
  person would be shown (`notes`), so a recipe is mended before anyone runs it.
  `look` with a proposal lists what that proposal would have made. A making is
  of the project: of a media a waiting proposal adds, `make` says which
  proposal, and that it comes first.
- `make` asks for a media to be made, and answers with the plan a person would
  see. By default the person is asked: the plan is left in the review thread,
  `kerf proposals` lists it, and `kerf make` or Make from Recipe… makes it and
  answers the asking. Where the person lets an agent make on its own, the
  making runs in the agent's session, as a job it reads, waits for, or stops,
  as Stop Making stops one, and the media becomes the new take once it is made.
  The agent then looks at the take, and changes one thing for the next.

What an agent may do without asking is the person's to say, in their own
`~/.kerf/agent.json` (`KERF_AGENT` names another file), outside any project,
so that a project cannot let an agent run its recipes:

```json
{ "permissions": "auto", "makes": 5 }
```

| `permissions` | An agent's edit | A making it asks for |
|---|---|---|
| `ask`, the default | a proposal the person accepts | the person's to run |
| `accept-edits` | accepted as it is made | the person's to run |
| `auto` | accepted as it is made | runs, up to `makes` in one session (5 unless it says), and is asked for past them |

An edit accepted this way is still one revision, which undo takes back, and a
making still keeps every take beside the ones before, so what was let through
is seen and undone as anything else. The file is read at each asking, so a
change to it holds from the next; one that does not read lets nothing more
through, and the agent is told so. A making the editor's own assistant runs is
shown in the editor's toolbar as the person's own is, each line its steps say,
and Stop Making in the bin stops it. One that an MCP client runs, or a Claude
Code session, which is another program, is shown in its own session only.
Either ends with its session: an MCP client that goes away stops it as Stop
Making does.

## Takes

A model does not give the same shot twice, so a making never writes over the
one before it. Takes are kept in a folder beside the recipe:

```text
shots/open.kerfrecipe
shots/open.takes/001.mp4         the first take
shots/open.takes/001.take.json   what it was made from
shots/open.takes/001.work/       the guide its steps drew, the request its model read
shots/open.takes/002.mp4
```

After a making, the media's line names the new take
(`path="shots/open.takes/002.mp4"`), with its length and streams, and clips
keep their timing. Because the take is named in the document, choosing one is
an edit like any other: it is undone, proposed, reviewed, and merged as one.

```bash
kerf takes m3            # the takes, a * on the one the media is
kerf takes m3 --use 1    # have the media be take 1 again
```

An agent sees the same with `look` and `media`, where each take the media is
not has the line that has the media be it, as `use`:
`media replace m3 path="shots/open.takes/001.mp4" rate=16 dur=65f video=832x480 recipe="shots/open.kerfrecipe"`.
The line is whole, since a take need not be as long as the one before it, and
a path alone is refused.

In a shared project the takes travel with it: a project made with Git LFS
stores `*.takes/` there, with each take's record and request left as text.

## What asks for a shot again

A take's record holds a hash of each thing it was made from:

- the recipe, without its comments;
- each file the recipe reads: what `needs` names and what a model is handed;
- each sequence it has Kerf render: that sequence, the sequences it nests,
  and the media lines they play;
- each signal it hands a model, as the model is handed it: the numbers, and
  nothing else of the sequence its clip is in.

A media wants making when one of those is no longer what its take was made
from, and it says which: `look` lists it under `toMake` with
`shots/open.scene.json changed after its file was made`, and `kerf takes`
marks each take with what changed since.

Nothing else is in the record. Trimming, moving, retiming, grading, titling,
or mixing a made shot changes none of it, so those edits are free: they play
at once and never ask for the shot again. Changing the guide, the prompt, a
reference, a signal's keys, or the seed is what costs a making, and Kerf says
so before a person pays for one.

A making of a recipe that has not changed is another take: another roll of
the same shot.

Two limits, as they are now: a media file a rendered sequence plays is seen
by its length, so one replaced in place by another of the same length is not
seen; and a file that no take made (one put there by hand, or made before
takes) is held to its recipe by their times alone.

## Models

Kerf has no model of its own and knows none by name. A model is a program the
person names in their own `~/.kerf/models.json` (`KERF_MODELS` names another
file), outside any project, so a project cannot say what runs:

```json
{
  "dawn": { "run": ["dawn-adapter", "--request", "{request}"] }
}
```

For a `generate` step Kerf writes the request, a JSON file, and runs that
program with `{request}` as the file's path:

```json
{
  "kerf-request": 1,
  "model": "dawn",
  "prompt": "a fishing boat leaves the harbour at dawn, fog",
  "guide": "/project/shots/open.takes/003.work/guide.mp4",
  "first": "/project/refs/boat.png",
  "last": null,
  "references": [],
  "seconds": 4.0,
  "size": null,
  "seed": 7,
  "signals": [],
  "more": { "strength": "0.6" },
  "out": "/project/shots/open.takes/.making-003.mp4"
}
```

`signals` holds each signal the line names, as [Signals](#signals) says.

The program asks its model however that model is asked, writes the file at
`out`, writes what it has to say to stderr a line at a time, and ends with
success. It runs supervised, as a recipe's `run` does, and never through a
shell. A hosted model's program holds its own key; Kerf passes none.

When a making is stopped, the program is sent SIGTERM and has five seconds to
end by itself. That is when it
takes back what it asked a server for, so that nothing goes on being made,
and billed, that nobody will fetch. A program that a script starts gets that
time only if the script hands over to it (`exec program "$@"`): once the
script itself has ended, what it started is ended with it.

The request is the whole contract, so a model that takes only a prompt and a
first frame, one that follows a guide video, and one that takes depth are the
same to Kerf: each program reads the fields its model has a use for.

### What a model takes

A recipe written for a model wants to know the model: how large a picture it
makes, at what rate, for how long, and whether it follows a guide at all.
Beside `run`, a model's entry can say so under `takes`:

```json
{
  "dawn": {
    "run": ["dawn-adapter", "--request", "{request}"],
    "takes": {
      "about": "follows a guide video, a reference picture, or the prompt alone",
      "size": [832, 480],
      "rate": 16,
      "seconds": 5,
      "reads": ["guide", "ref", "signal"],
      "more": { "steps": 20, "strength": 1.0 }
    }
  }
}
```

| | |
|---|---|
| `about` | What it makes, in a line. |
| `size` | The picture it makes, width and height: the size a guide for it is drawn at, and the most a `size=` asks of it. |
| `rate` | The frames a second it makes: the rate a guide for it is drawn at. |
| `seconds` | The longest shot it makes at once. |
| `reads` | What a `generate` line hands it that it has a use for, of `guide`, `first`, `last`, `ref` and `signal`. An empty list is a model that takes a prompt alone. |
| `more` | The other names a line can set for it, each with the value it has when the line does not. |

Each is there only where the person says it, and anything else under `takes`
is kept as it is written. Kerf holds no making to it: the program still asks
its model as it sees fit, and says so where that is not what the line asked.

It is written in the file, and no program is asked for it, because `look`
is an agent's tool and looking runs nothing: reading it starts no program, cannot hang on one, and cannot be taken by a program that does not
know the question for a request to make something. A model's program can
write the entry, though, for the person to put there: `kerf-comfy` does
([below](#a-model-on-a-comfyui-server)).

`look` with no arguments lists the models set up, as `models`, each with what
its entry says it takes and, for one that follows a guide, the `render` line
that draws a guide for it, so an agent writes `generate model=` with a name
that exists and a guide of the model's own size and rate:

```json
"models": [
  {
    "name": "dawn",
    "about": "follows a guide video, a reference picture, or the prompt alone",
    "size": "832x480", "rate": 16, "seconds": 5,
    "reads": ["guide", "ref"],
    "more": { "steps": 20, "strength": 1.0 },
    "guide": "render <sequence> {guide.mp4} size=832x480 rate=16"
  }
]
```

A model with a name and nothing else has not said what it takes. A models
file that cannot be read is `modelsError`, with what is wrong in it.

With no model set up, a recipe that asks one is refused when a person tries
to make it:

```text
no model `dawn` is set up: /home/you/.kerf/models.json names none. A model is
a program named there, as {"dawn": {"run": ["<program>", "{request}"]}}
```

### A model on a ComfyUI server

`kerf-comfy` is such a program, for a model kept on a
[ComfyUI](https://github.com/comfyanonymous/ComfyUI) server. It is in this
repository (`tools/kerf-comfy`, built with `cargo build --release -p
kerf-comfy`); the packages do not carry it.

```json
{
  "vace": { "run": ["/path/to/kerf-comfy", "--workflow", "wan-vace", "{request}"] }
}
```

`kerf-comfy --workflow wan-vace --describe` prints that entry whole, with
[what the model takes](#what-a-model-takes) as its workflow has it, to put
under the model's name. It reads the workflow and asks no server:

```json
{
  "run": ["/path/to/kerf-comfy", "--workflow", "wan-vace", "{request}"],
  "takes": {
    "about": "Wan 2.1 VACE: a shot that follows a guide video, a reference picture, both, or the prompt alone. Of a guide it follows depth best: draw it with pass=depth.",
    "size": [832, 480],
    "rate": 16,
    "seconds": 5.0625,
    "frames": { "most": 81, "step": 4 },
    "reads": ["guide", "ref"],
    "more": { "steps": 20, "cfg": 6.0, "strength": 1.0, "…": "…" }
  }
}
```

`more` is each value the workflow sets that a `generate` line can replace.
Print it again when the workflow changes.

The server is `COMFY_URL`, with `COMFY_API_KEY` if it asks for a key. The key
goes in the header `X-API-Key` unless `COMFY_API_HEADER` names another, and
`Authorization` sends it as a bearer token. An editor is not started from a
shell, so the same can be kept in `~/.kerf/comfy.json` (`KERF_COMFY` names
another file), which is yours and outside any project:

```json
{ "url": "https://comfy.example", "key": "…", "header": "X-API-Key" }
```

A workflow is a ComfyUI graph, as ComfyUI exports it for its API, with blanks
in it, and what the model it runs makes: its rate, how many frames, how
large. `wan-vace` comes with the program: Wan 2.1 VACE, which follows a guide
video, a reference picture (`ref=`), both, or the prompt alone. `--workflow`
takes a file too, and `tools/kerf-comfy/workflows/wan-vace.json` is the one to
start from.

| Blank | What fills it |
|---|---|
| `{prompt}`, `{seed}` | What the line asks. With no `seed=`, one is picked and said. |
| `{width}`, `{height}`, `{frames}`, `{fps}` | What `size=` and `seconds=` come to for this model. |
| `{guide}`, `{first}`, `{last}`, `{ref}`, `{ref2}` … | The file, as the server names it once sent up. A node that reads one the line does not give is left out, with whatever cannot do without it. |
| `{signal}`, `{signal2}` … | A signal the line gives, in the order it gives them, as a number for each frame asked for, with commas between: `0,0.25,0.5,1`. A node that reads one the line does not give is left out the same way. |
| any other, such as `{steps}` | The value the workflow sets, or the same name on the line: `steps=30 strength=0.6`. |

What it does with a request:

- It fits the guide to the model. A guide drawn at 1920x1080 and 24 frames a
  second goes up at the model's 832x480 and 16, filling the picture with what
  is over cut off evenly, and is kept as `guide.comfy.mp4` beside the request.
  A guide Kerf drew at the model's size and rate already
  (`render s2 {guide.mp4} size=832x480 rate=16`) goes up as it was drawn, cut
  the same way, for a fraction of the drawing.
- It asks for a length and a size the model takes. `seconds=4` is 65 frames of
  a model that makes one frame and a multiple of four: never fewer than asked,
  and never more than the model makes at once. A size with more area than the
  model's own comes down to it, keeping its shape. It says so when what it asks
  is not what the line said, and when the guide is shorter than the shot.
- It queues the graph, kept as `workflow.comfy.json` beside the request, says
  where the job is while it waits, and writes what the server saved as the
  take. A job waited for past half an hour (`COMFY_TIMEOUT`, in seconds) is
  taken back from the server.
- It takes its job back when the making is stopped, in Kerf or with Ctrl-C:
  out of the server's queue if it waits there, and interrupted if it is the
  one running. It says so (`asked to stop: took job … back from the server`).

Its limits, as they are now: it learns how a job is doing by asking, so it
says that one is running and for how long, not how far along it is; on
Windows a making that is stopped ends the program outright, so its job stays
on the server; and what it sends up and what the server saves stay on the
server.

## Signals

A guide says where things are, and a prompt what they are. Neither says
when: the frame a ball meets the floor, how a light comes up, how much smoke
there is from one second to the next. A signal does. It is a number from 0
to 1 over the shot's time, and the prompt says what it is of.

A signal is a clip of the project, `gen=signal@1`. It draws nothing, and its
`value` takes keyframes as any number does, so it is cut, moved, proposed,
and merged with the rest of its sequence. Put it in the guide's sequence,
from the shot's first frame, on a track of its own:

```text
clip bounce gen=signal@1 at=0f dur=96f
  set value={0f:0 11f:0 12f:1 15f:1 16f:0 35f:0 36f:1 39f:1 40f:0}
```

```text
render s2 {guide.mp4} size=832x480 rate=16
generate model=cued prompt="a man dribbles a basketball; the ball meets the court on each beat" guide={guide.mp4} signal=bounce seed=7
```

The request then holds what the signal comes to on each frame of its
sequence, from the sequence's first, at the sequence's rate:

```json
"signals": [
  { "name": "bounce", "rate": 24.0, "values": [0.0, 0.0, 0.0, 1.0, 1.0, 0.0] }
]
```

- **Where the clip is not, the signal is 0**: before it, after it, and while
  it or its track is turned off (`enabled=false`). Inside the clip its keys hold before the first and after
  the last, as keys do.
- **A shot's time is the sequence's.** Frame 0 of the signal is the first
  frame of the sequence its clip is in, which is the guide's first frame
  when that sequence is the guide. A signal needs no guide: its sequence can
  hold nothing else.
- **It costs a making only when it changes.** A take is held to the numbers
  a model was handed, to four places, and not to the rest of the sequence:
  recoloring a clip beside it asks for nothing, and moving one of its keys
  does.
- **One that never moves says nothing of when**, and the plan says so before
  a making is paid for.
- **The model's program brings it to the model's frames.** `kerf-comfy` gives
  a workflow one number for each frame asked for, at the model's rate. Where
  the model makes fewer frames a second than the sequence has, each is the
  mean of the signal over that frame's time, so a peak one frame long at 24 a
  second reaches a model at 16 lower than it was, and spread over two frames.
  Hold a peak for as long as two of the model's frames last and one of them
  has it whole: three frames at 24 for a model at 16.

What a signal does to the picture is the model's, not Kerf's. Kerf knows no
model that reads one by name, and the workflow that comes with `kerf-comfy`
has no blank for one, so a line that names a signal for it is told that it
is left out. A workflow of your own takes one at a `{signal}` blank, in a
node that takes a list of numbers, a frame each
([A model on a ComfyUI server](#a-model-on-a-comfyui-server)); what it does
there, a strength, a mask's weight, a motion scale, is the workflow's to say.

A signal's moments can be a sound's. An agent's `look` with `signal` reads
the onsets of the sound a clip on the timeline plays, the beats of a track
or a ball on a floor, and gives the line that has a signal clip of the same
sequence be 1 for a few frames from each and 0 between:

```text
look {"signal": {"of": "a1", "clip": "bounce", "hold": 3}}
  → set bounce value={0f:0:hold 12f:1:hold 15f:0:hold 36f:1:hold 39f:0:hold}
```

`hold` is how many frames each lasts, 3 unless said; onsets nearer each other
than that run together, and those that fall outside the signal clip are
counted and left out. It reads onsets, where a sound starts, and not how
loud it is.

Not here yet: a signal from how loud a sound is, as a voice swells, and from
footage, such as how fast something moves. Nor is there a place in the editor
to draw one under the picture: its keys are text, which an agent writes, or a
person with `set`.

## Guides

What a model is given to follow decides how much of a shot is directed and how
much is left to it. Three places a guide comes from, all through the same
steps:

- **A sequence of the project** (`render s2 {guide.mp4} size=832x480 rate=16`):
  clips placed in depth, meshes (`gen=mesh@1`: a box, a ball, a cylinder, or
  a glTF file's), a camera move, lights, a title, cut and timed in Kerf. It needs nothing
  installed and runs on any machine Kerf does.
- **A 3D renderer** (`run vork render … --out {guide.mp4}`, or Blender the
  same way): a blocked scene with real geometry and a camera, drawn fast and
  rough. The renderer's own passes, such as depth, are files like any other
  for a model that reads them.
- **Footage**: a take of another shot, or something filmed, named as a file.

### Passes

A picture leaves out what the document knows: which clip a pixel is of, and
how far away it is. A `render` step draws either in place of the picture,
from the same clips, placed and seen as the picture's are:

```text
render s2 {guide.mp4} size=832x480 rate=16
render s2 {depth.mp4} pass=depth size=832x480 rate=16
render s2 {ids.mp4} pass=ids size=832x480 rate=16
```

- **`pass=ids`**: every clip that draws, in one flat color of its own, over
  black where there is none. A clip keeps its color from frame to frame, so
  the file says which thing is which as the camera and the clips move. A
  nested sequence is one clip. The colors go by the clips' order, the lowest
  track first, so adding a clip below others gives those above it new ones.
- **`pass=depth`**: how near each pixel is, as a grey: white at the camera,
  mid grey where a clip at `z=0` stands before a camera left where it is,
  and darker the farther, by one over the distance, as a model that reads
  depth is given it. It goes by the scene's own numbers and by nothing in
  the frame, so it holds still from one frame to the next. A clip out of
  depth, such as a backdrop on the lowest track, has no distance and is
  black, and so is where there is no clip.

A pass has no lights, no focus, no adjustment layers, no dips, and no
captions, and every clip is laid over what is behind it whatever its blend.
It is drawn on the CPU. The sides of a clip with a `depth` take the grey of
its face beside them, as they take its color, and not their own distance.

A pass is a file as a guide is, and a model that follows one is handed it as
its guide (`guide={depth.mp4}`); what kind of guide a model follows is its
workflow's to say. Handing one model a picture and a pass of it at once has
no word on a `generate` line yet, and normals are not drawn.

Which kind a model follows is the model's, and its entry's `about` says. A
model that covers a render is handed the picture. Wan 2.1 VACE 1.3B, the
small model `kerf-comfy` comes with a workflow for, made more of a depth
pass than of a picture in flat colors, and nothing of an ids pass, which is
what `look` with `shows` reads a frame by
([architecture.md](architecture.md#measured)).

### A guide at the model's size and rate

`render s2 {guide.mp4}` draws the sequence as it is: every frame of it, at its
own size. A model follows a guide at its own size and rate, most often
smaller and slower, and whatever is drawn past those is drawn to be thrown
away. So a guide's line names them:

```text
render s2 {guide.mp4} size=832x480 rate=16
```

- `size=WxH` is the size of the file's picture; each side is even. Kerf draws
  the frames at that size. It does not draw them whole and bring them down.
- Where the size's shape is not the sequence's, **the picture fills the size
  and what is over is cut off evenly**: both sides of a wider sequence, the top
  and the bottom of a taller one. Nothing is squeezed, and no bars are added,
  since a model follows bars as it follows anything else in the picture. Keep
  what matters away from the edges that will go, or give the sequence the
  model's shape. A size larger than the sequence's is the sequence's picture
  scaled up.
- `rate=<fps>` is how many frames a second the file has: `16`, `23.976`,
  `24000/1001`. Each frame is the sequence at that frame's time, so the guide
  plays for as long as the sequence does, in fewer frames, and what moves in it
  moves at the same speed.
- The file has no sound track, whatever the sequence plays.

What that saves, measured on four cores with no GPU: a guide of clips in
depth under two lights with a moving camera, 96 frames at 1920x1080 and 24 a
second, took 54.5 s to draw as it is, and 7.8 s as `size=832x480 rate=16`, 64
frames. Made again with nothing changed, either comes from the frames Kerf
keeps on disk, in about a second.
