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
| `render <sequence> {name} [size=WxH] [rate=<fps>]` | Has Kerf draw a sequence of this project into a file, with no sound. A guide blocked with Kerf's own clips in depth, cameras, lights, and titles reaches a model this way, at the model's size and rate when the line names them. See [Guides](#guides). |
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
| `seconds=`, `size=WxH` | How long and how large. |
| `seed=` | A whole number, for a model that gives the same file for the same seed. |
| `out={name}` | Writes a file for a later step in place of the shot's own, so one model's picture can be the next one's `first=`. |

Anything else on the line, such as `strength=0.6`, goes to the model as it is
written, for the models that read it. A file is a path from the recipe's
folder (`../refs/boat.png`) or a `{name}` an earlier step made. Every file a
recipe names has to be inside the project.

## Making a shot

Only a person asking makes a shot: `kerf make m3`, or Make from Recipe… on the
media in the editor's bin. Both show every step as it would run and the take
it would write, and ask first; `kerf make m3 --yes` does not ask. Opening a
project, drawing it, and an agent's tools never run a recipe. A recipe that
could not run is refused before anything does, with why: a model that is not
set up, a sequence the project has not, a file it needs that is not there.

A step that fails, or a last step that writes nothing, leaves no take and the
media as it was.

### From a terminal alone

Nothing here needs the editor open. A scene and its recipe written as files,
by hand or by an agent that has only a shell and file tools, are taken into
the project and made with `kerf`, a person saying yes at each step that
changes the project:

```bash
kerf load                # take project.kerf, as edited on disk, as the project
kerf proposals           # the proposals an agent's `edit` left waiting
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

An agent sees the same with `look` and `media`, and proposes a take with
`media replace m3 path="shots/open.takes/001.mp4"`.

In a shared project the takes travel with it: a project made with Git LFS
stores `*.takes/` there, with each take's record and request left as text.

## What asks for a shot again

A take's record holds a hash of each thing it was made from:

- the recipe, without its comments;
- each file the recipe reads: what `needs` names and what a model is handed;
- each sequence it has Kerf render: that sequence, the sequences it nests,
  and the media lines they play.

A media wants making when one of those is no longer what its take was made
from, and it says which: `look` lists it under `toMake` with
`shots/open.scene.json changed after its file was made`, and `kerf takes`
marks each take with what changed since.

Nothing else is in the record. Trimming, moving, retiming, grading, titling,
or mixing a made shot changes none of it, so those edits are free: they play
at once and never ask for the shot again. Changing the guide, the prompt, a
reference, or the seed is what costs a making, and Kerf says so before a
person pays for one.

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
  "more": { "strength": "0.6" },
  "out": "/project/shots/open.takes/.making-003.mp4"
}
```

The program asks its model however that model is asked, writes the file at
`out`, writes what it has to say to stderr a line at a time, and ends with
success. It runs supervised, as a recipe's `run` does, and never through a
shell. A hosted model's program holds its own key; Kerf passes none.

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
      "reads": ["guide", "ref"],
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
| `reads` | The files of a `generate` line it has a use for, of `guide`, `first`, `last` and `ref`. An empty list is a model that takes a prompt alone. |
| `more` | The other names a line can set for it, each with the value it has when the line does not. |

Each is there only where the person says it, and anything else under `takes`
is kept as it is written. Kerf holds no making to it: the program still asks
its model as it sees fit, and says so where that is not what the line asked.

It is written in the file, and no program is asked for it, because `look`
is an agent's tool and an agent's tools run nothing: reading it starts no
program, cannot hang on one, and cannot be taken by a program that does not
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
    "about": "Wan 2.1 VACE: a shot that follows a guide video, a reference picture, both, or the prompt alone.",
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

Its limits, as they are now: it learns how a job is doing by asking, so it
says that one is running and for how long, not how far along it is; Kerf ends
a program outright, so a making cancelled in Kerf leaves its job running on
the server; and what it sends up and what the server saves stay on the
server.

## Guides

What a model is given to follow decides how much of a shot is directed and how
much is left to it. Three places a guide comes from, all through the same
steps:

- **A sequence of the project** (`render s2 {guide.mp4} size=832x480 rate=16`):
  clips placed in depth, a camera move, lights, a title, cut and timed in
  Kerf. It needs nothing installed and runs on any machine Kerf does.
- **A 3D renderer** (`run vork render … --out {guide.mp4}`, or Blender the
  same way): a blocked scene with real geometry and a camera, drawn fast and
  rough. The renderer's own passes, such as depth, are files like any other
  for a model that reads them.
- **Footage**: a take of another shot, or something filmed, named as a file.

Kerf does not draw depth, normals, or object ids of its own sequences as
passes yet; a model that needs them takes them from a renderer.

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
