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
| `render <sequence> {name} [size=WxH] [rate=<fps>] [pass=ids\|depth\|relative\|normals] [matte=<clip>] [from=<seconds>] [seconds=<seconds>]` | Has Kerf draw a sequence of this project into a file, with no sound. A guide blocked with Kerf's own clips in depth, cameras, lights, and titles reaches a model this way, at the model's size and rate when the line names them. `pass=` draws, in place of the picture, which clip each pixel is of or how near it is. See [Guides](#guides). |
| `generate model=<name> …` | Asks a model for a file. See below. |
| `join {part} {part} … {name} [lap=<frames>]` | Sets files earlier steps made end to end into one, a picture with no sound: the parts of [a longer shot](#a-longer-shot). Each part after the first loses its first `lap` frames. |

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
| `guide_<name>=` | Another picture in motion to follow, by what it is of, for a model that reads more than one: `guide_depth={depth.mp4}` beside the picture, or `guide_ids=`. A name is lowercase letters and digits, each given once; the request holds them under `guides`. |
| `first=`, `last=` | The first and the last frame. A picture in motion given as `first=` is its last frame, and as `last=` its first: the shot goes on from where that one ended, or ends where it began. |
| `from=` | Where the shot starts in its sequences' time, in seconds, for a part of [a longer shot](#a-longer-shot): its signals and where its clips go are read from there. |
| `ref=` | A picture of who or what is in it, or of how it should look. As many as wanted. |
| `cast=`, `style=` | Who and what is in it, and how it looks, by the names the project's `cast.kerfcast` gives them. As many as wanted. See [The cast](#the-cast). |
| `signal=` | A number over time that says when what the prompt asks for happens, and how much: a signal clip of the project, by its id. As many as wanted. See [Signals](#signals). |
| `moves=` | Where a thing goes: a clip of the project, by its id, handed over as its place on every frame. As many as wanted. See [Where a thing goes](#where-a-thing-goes). |
| `seconds=`, `size=WxH` | How long and how large. |
| `seed=` | A whole number, for a model that gives the same file for the same seed. |
| `workflow=` | A graph of the model's own to run in place of the one its program would: a file of the project, for a model whose program takes one. See [A workflow of the shot's own](#a-workflow-of-the-shots-own). |
| `out={name}` | Writes a file for a later step in place of the shot's own, so one model's picture can be the next one's `first=`. |

Anything else on the line, such as `strength=0.6`, goes to the model as it is
written, for the models that read it. A file is a path from the recipe's
folder (`../refs/boat.png`) or a `{name}` an earlier step made. Every file a
recipe names has to be inside the project.

### A longer shot

A model makes so much at once, about five seconds for the ones `kerf-comfy`
runs (`seconds` in what it [takes](#what-a-model-takes)). A shot longer than
that is made in parts, each a guide of its own part of the sequence and a
`generate` that starts where the part before ended, and `join` sets them end
to end:

```text
kerf-recipe 1
# Ten seconds of a ball crossing a court: two parts of the same sequence.
render court {d1.mp4} pass=depth size=832x480 rate=16 seconds=5.0625
render court {d2.mp4} pass=depth size=832x480 rate=16 from=5 seconds=5.0625
generate model=video prompt="a basketball bounces on a court" guide={d1.mp4} moves=ball seed=7 out={a.mp4}
generate model=video prompt="a basketball bounces on a court" guide={d2.mp4} first={a.mp4} moves=ball from=5 seed=7 out={b.mp4}
join {a.mp4} {b.mp4} {out} lap=1
```

- `render … from=5 seconds=5.0625` draws that part of the sequence and no
  more. With `seconds` alone it starts at the sequence's start, and with
  `from` alone it goes to its end.
- `first={a.mp4}` hands the second part the last frame of the first, which
  Kerf takes from the file, so it starts as the picture the first ended on.
  The parts therefore share a frame: the second starts at 5 s, on the first's
  last frame (81 frames at 16 a second are 5.0625 s, the 81st at 5 s), and
  `lap=1` leaves that frame out of the join. Two parts of 81 frames make 161.
- `from=5` on the `generate` line has the part's signals and where its clips
  go read from 5 s of their sequence, as its guide is.
- A third part is two more lines, `from=10`, and one more name in the `join`.

What carries from part to part is what the last frame shows. A model starts
each part anew from that one picture, so how something was moving is not
carried, and a part can drift in tone from the one before: on the model this
was tried with, the second part is darker from its second frame
([the record](shots-measured.md)). `shot-directed` is the workflow that comes
with `kerf-comfy` that reads a first frame beside a guide.

The plan says so when a recipe asks one `generate` for more than its model
makes at once, or hands it a guide longer than that, and how to write it in
parts. In the shot's graph the join is a card of its own, each part's guide
says which part of the sequence it draws, and all of it is set there as any
word of the recipe ([As a graph](#as-a-graph)).

So a model is told three things, each its own way: **where** things are and
how the camera moves, by a guide; **when** something happens and how much, by
a signal; and **what** it all looks like, by the prompt and the references.
A model that takes the way a thing goes as numbers of its own is handed
that too, from the same clips (`moves=`).

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

### As a graph

In the editor a shot opens as a graph of how it is made: **Open Graph** on the
media in the bin, or a double-click on its clip. Each step is a node, a guide
drawn from a sequence or a model asked for a file, and so is each thing of the
project a step reads: a sequence, a picture, a signal, the way a clip goes, one
of the cast. An edge is a reading, in the colour of what goes along it: a picture
in motion, a still, the way a thing goes, a number over time. The last step's
file is the shot, so the graph ends in its takes.

Choosing a node shows what it holds, to set: a model's prompt, seed, length, size
and knobs, and what it reads; a guide's sequence, pass, size and rate; a signal's
curve, drawn by dragging across it. A change is written into the recipe's text,
that word alone, so its comments, its order and how each line was spelled stay,
and one a line cannot take is refused as a making would refuse it. A signal's
curve is its clip's `value` in the project, one edit that undo takes back. An
edge drawn from a node's port to a step that could read it adds the word to that
step's line, `signal=bounce` or `ref={ball.png}`, and one is let go under the
step's Reads. **Add step**, in the graph's head, adds a line to the recipe: a
guide drawn from a sequence, a model asked for a file, or a join of the parts
the steps make. It is written to read as it stands, by a name no step has yet
(`render court {guide2.mp4} pass=depth`), before the step that writes the
shot's file, and its card opens to set it and wire it. **Delete step**, at the
foot of a step's card, takes its line out; a step whose file a later one
reads, and the last, are refused until that is changed. Make,
Make again, Several and Stop are in the graph's head and do what the bin's do:
what would run is shown first, and each making is a new take, chosen among in the
takes' node. A model run with `kerf-comfy` opens further, into the ComfyUI
workflow it is run with ([A workflow of the shot's own](#a-workflow-of-the-shots-own)).

This is a person's own change, written in place as a text editor writes it: the
takes made before say the recipe changed since, and the next making is of the
recipe as it reads now. An agent's recipe is still never written over
([An agent's part](#an-agents-part)).

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
kerf shots               # every media a recipe makes: its take, its cast, whether it wants making
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
- What a person sees and sets in a shot's graph ([As a graph](#as-a-graph)),
  an agent reads and sets, through the same reading (`kerf-shots`):

  | In the graph, a person | An agent |
  |---|---|
  | opens a shot as its graph | `look` with `graph`: the nodes, the edges, `cannot`, `notes`, each model's `runner`, the takes |
  | sets a prompt, a seed, a knob | `edit` with `recipeSet`: one name on one line, comments and order kept |
  | adds a step, a guide, a model or a join, and deletes one | `recipeSet` with `add`, the step's line, after `line`; and with `remove` |
  | draws an edge to a step, or lets one go | the same, the name its reader gives it (`signal`, `ref`, `guide`, `moves`), with no `value` to let it go |
  | draws a signal's curve | an `edit` script that sets the clip's `value` |
  | sets a signal from a sound of its sequence | `look` with `signal`, and the line it gives in an `edit` |
  | opens a model into its workflow | `look` with `workflow`: every node and input, the blanks and what fills each now |
  | sets a value in the workflow | `edit` with `workflowSet`: a copy in the project, named by the line's `workflow=` |
  | goes back to the workflow as it comes | `recipeSet` of `workflow` with no `value` |
  | Make, Make again, Several, Stop | `make`, with `rolls`, and its `job` with `cancel` |
  | sees each take's picture and the guide as it was drawn | `look` with `media` and `sheet`, of the take and of the guide's file |
  | holds the takes to their guide | `look` with `takes`: each take's `overlap`, and the `best` |
  | chooses a take | the take's `use` line, in an `edit` |
  | restores the recipe a take was made from | `edit` with `restoreTake` |

  Where they differ is where a change is written. A person's is written in
  place. An agent's writes over nothing: the recipe with its word changed is
  the next version (`shots/bounce@2.kerfrecipe`), a workflow with a value set
  is a new copy (`bounce.video-2.comfy.json` where one is there), and the
  proposal moves the media to them and says, as comments, what was set. Once
  it is accepted the editor's open graph shows the shot as it now is, the
  cards where they were put.
- `look` lists what wants making under `toMake`, as an `edit` that names a
  recipe answers it, with what would refuse it (`cannot`) and the notes a
  person would be shown (`notes`), so a recipe is mended before anyone runs it.
  `look` with a proposal lists what that proposal would have made. A making is
  of the project: of a media a waiting proposal adds, `make` says which
  proposal, and that it comes first.
- `make` asks for a media to be made, and answers with the plan a person would
  see. By default the person is asked: the plan comes back as the answer's
  `ask`, which the agent says in the conversation, and `kerf make` or Make
  from Recipe… makes it. Where the person lets an agent make on its own, the
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
shots/open.takes/001.work/       the guide its steps drew, the request its model read,
                                 and the recipe it was made from
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

Several takes of one shot to choose among are made at once with `--rolls`:

```bash
kerf make m3 --rolls 3   # three takes, each seed one on from the last
```

A model that gives the same file for the same seed would make one picture
three times, so each roll asks with every `seed=` of the recipe one further
on: 7, 8, 9. The steps are shown once, with how many takes they make, and
asked for once. Rolls go on from the last one a take of the recipe as it is
written now had, so asking for three more gives 10, 11, 12. The media
becomes the last take made, `kerf takes` says which roll each is, and a roll
is another of the same shot: none of them is asked for again. A making that
fails leaves the takes made before it, and the media is the last of those.

In the editor it is **Make Several from Recipe** on the media in the bin, for
two, three, or four takes: the steps show once, with how many takes they
write, and run on a yes. An agent asks with `make` and `rolls`, up to eight.
Each roll is a making the person's limit counts, so where the limit lets
fewer than were asked, the person is asked for them all, and none runs.

`kerf shots` reads every media a recipe makes at once, a line each, with a
`!` on one that wants making and why, and what they come to:

```text
  m3  shots/open.kerfrecipe  take 2 of 3  4s  Ada, lamp, film
! m4  shots/pier.kerfrecipe  no take  2s  Ada, film  to make: its file is not there yet
2 shots, 1 to make (m4); 4s of 6s made
```

In the editor it is **Takes** on the media in the bin: every take there is
a file of, the one the media is ticked, and choosing another has the media
be it.

An agent sees the same with `look` and `media`, where each take the media is
not has the line that has the media be it, as `use`:
`media replace m3 path="shots/open.takes/001.mp4" rate=16 dur=65f video=832x480 recipe="shots/open.kerfrecipe"`.
The line is whole, since a take need not be as long as the one before it, and
a path alone is refused.

### Choosing among them

Looking is the first way: in the shot's graph ([As a graph](#as-a-graph)) the
takes' card shows each take's picture, and a guide's card shows the guide as
it was drawn for the take that shows, what the model was handed.

Where the recipe's line says where a clip goes (`moves=<clip>`), a take can
be held to that too. **Hold to guide**, in the takes' card, reads each take
beside its guide with no model: the subject is looked for in the take's own
picture where the guide's sequence has the clip, and followed. Each take then
says how much of where it has the subject and where it was asked for is the
same, from 0 to 1 (`0.62 of its guide`), with the closest said, and, on
hovering it, on how many of the frames the guide shows the clip on it was
followed. A subject followed on few frames says little: one that crosses a
third of the frame between two frames is lost, and reads low whatever the
take did. A take is a few seconds to read. A shot whose line names no clip
has nothing to hold a take to, and the card says so.

### Going back to a take

A take keeps the recipe it was made from in its working folder
(`made-from.kerfrecipe`): the text as that making read it, comments and all,
with each seed as that take had it, so the third of three rolls keeps
`seed=9`; and the workflow file its line named, as it then was. A recipe
goes on changing after a take is made, and the take you choose a week later
was made from one that is no longer written anywhere else.

In the shot's graph, the takes' card offers it for the take the shot is,
where its recipe has changed since: **Restore take 3's recipe** writes the
recipe back as that take was made from it. Its workflow comes back with it:
where the project still has that file as it was, nothing is written; where
the file now holds something else it is left alone, and the take's own is
written beside it (`bounce.take3.video.comfy.json`) and named on the line.
A making of the restored recipe is that take again where the model gives the
same file for the same seed. `kerf takes` and the bin's Takes still choose
which take the media is; this is the other half, the settings.

What a take was made from beside its recipe is the project's own: the
sequence its guide was drawn from, its signals, the cast. Those come back by
undoing in the project, and the takes' card says which of them changed
since. A take made before takes kept their recipe has none to restore.

An agent goes back the same way with `edit` and `restoreTake`. Its recipe
is then the version that holds that text, or the next one, never written
over the one there.

An agent reads the same numbers with `look` and `takes`, and for a take that
is the media, with its depth read back by a model, `held` adds how its depth
holds ([Guides](#guides)).

In a shared project the takes travel with it: a project made with Git LFS
stores `*.takes/` there, with each take's record and request left as text.

## What asks for a shot again

A take's record holds a hash of each thing it was made from:

- the recipe, without its comments;
- each file the recipe reads: what `needs` names and what a model is handed;
- each sequence it has Kerf render: that sequence, the sequences it nests,
  and the media lines they play;
- each signal it hands a model, as the model is handed it: the numbers, and
  nothing else of the sequence its clip is in;
- each cast and style it names, by its words and its pictures
  ([The cast](#the-cast)), and nothing else of `cast.kerfcast`;
- the sequence of each clip it hands a model the way of (`moves=`), as for
  one it renders: where a clip is on the frame is what its whole sequence
  draws of it.

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

## The cast

A model keeps a face, a boat, or a room from one shot to the next only when
each shot is asked for with the same words and the same pictures. Written
into every recipe, they drift apart, and changing one means finding every
recipe it is in. So a project says them once, in `cast.kerfcast` in its own
folder, beside `project.kerf`:

```text
kerf-cast 1
# Who and what the shots show.
cast Ada "an old keeper in a wool coat" ref=refs/ada-front.png ref=refs/ada-side.png
cast lamp "the lamp room of a lighthouse, brass and salt-stained glass" ref=refs/lamp.png
# How every shot looks.
style film "shot on 35mm colour negative, a cold blue-steel grade, blue-hour light"
```

| Line | What it says |
|---|---|
| `cast <name> "<words>" [ref=<picture> …]` | Someone or something a shot shows: a person, a place, a prop. |
| `style <name> "<words>" [ref=<picture> …]` | How a shot looks: its stock, its grade, its light. |

A name is letters, digits, `-` and `_`. The words are one quoted text, a
picture is a path from the project's folder, and a line has words, pictures,
or both. A `generate` line names what it shows:

```text
generate model=dawn prompt="Ada wipes the great lens" guide={guide.mp4} cast=Ada cast=lamp style=film seed=7
```

and the model is asked with the line's prompt, then each cast's name and
words, then each style's words, a sentence each:

```text
Ada wipes the great lens. Ada: an old keeper in a wool coat. lamp: the lamp room of a lighthouse, brass and salt-stained glass. shot on 35mm colour negative, a cold blue-steel grade, blue-hour light.
```

The name ties what the prompt says someone does to what they look like, so
write the prompt with the names the file has. Their pictures are handed over
as `ref=` is, after the line's own, each once. `kerf make` and Make from
Recipe… show the prompt as it is asked, and a model that reads no pictures
is noted as for any `ref=`.

A take is held to each one it names and to nothing else of the file. Saying
Ada another way, or giving her another picture, asks again for the shots Ada
is in, each naming `cast Ada` or the picture as what changed; adding the
gull to the file asks for none. A name the file has not is why a recipe
cannot run, said before anything does, with the names it has and the line
that adds it.

`look` with no arguments lists them, as `cast` and `styles`, so an agent
names ones that exist. One a shot needs that the file has not, an agent adds
with its edit: `edit` takes `cast`, lines as the file writes them, and adds
each after what the file holds, its comments left as they are. They are
written at once and not proposed, since adding one asks for no shot again.
A line for one the file has is refused: changing what shots were made from
is the person's.

In the editor it is **Cast and Styles…** in the bin's menu: a table of them,
a row each, with its words and its pictures. Add Picture… names a picture of
the project as it is, and copies one from elsewhere into the project's
`refs` folder. Save writes the file, and says which shots want making for
the change; the dialog shows which want making already. Saving from the
editor keeps what was written into the file by hand: its comments and empty
lines stay where they are, a line that is unchanged stays as it was written,
and a new one follows what the file holds. In the bin, a shot's Make from Recipe… says who it shows.

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
  "workflow": null,
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
| `reads` | What a `generate` line hands it that it has a use for, of `guide`, `first`, `last`, `ref`, `signal` and `moves`, and a guide beside the first by its name, as `guide_depth`. An empty list is a model that takes a prompt alone. |
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
  "shot": { "run": ["/path/to/kerf-comfy", "--workflow", "shot", "{request}"] }
}
```

`kerf-comfy --workflow shot --describe` prints that entry whole, with
[what the model takes](#what-a-model-takes) as its workflow has it, to put
under the model's name. It reads the workflow and asks no server:

```json
{
  "run": ["/path/to/kerf-comfy", "--workflow", "shot", "{request}"],
  "takes": {
    "about": "A shot that follows a guide video, a reference picture, both, or the prompt alone. Of a guide it follows depth best: draw it with pass=depth.",
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
large. `shot` comes with the program, for a model that follows a guide
video, a reference picture (`ref=`), both, or the prompt alone. `--workflow`
takes a file too, and `crates/surfaces/kerf-workflow/workflows/shot.json` is the one to
start from. `kerf-comfy --list` names those that come with it; the others
[read a picture back](#a-picture-read-back).

`still` comes with it too, for a picture and not a shot: one frame of the
model that makes the shots, from the prompt, or of who or what a reference
shows (`ref=`), saved as a picture. It is what a recipe's first `generate`
asks for when a later one wants a picture to look like or to start from:

```text
generate model=still prompt="an orange basketball, studio light" seed=3 out={ball.png}
generate model=video prompt="a basketball bounces on a court" guide={depth.mp4} ref={ball.png}
```

```json
"still": { "run": ["/path/to/kerf-comfy", "--workflow", "still", "{request}"] }
```

A workflow whose `frames.most` is 1 is a still: it names no `fps`, what it
`takes` says `"still": true` in place of a rate and a length, and its graph
ends in a node that saves a picture. Being one frame of a model made for
motion, it is at that model's size, 832x480 here, and takes seconds where a
shot takes minutes.

| Blank | What fills it |
|---|---|
| `{prompt}`, `{seed}` | What the line asks. With no `seed=`, one is picked and said. |
| `{width}`, `{height}`, `{frames}`, `{fps}` | What `size=` and `seconds=` come to for this model. |
| `{guide_depth}`, `{guide_ids}` … | A guide the line gives beside the first, by its name: sent up as `{guide}` is, brought to the size, rate, and frames asked for. |
| `{guide}`, `{first}`, `{last}`, `{ref}`, `{ref2}` … | The file, as the server names it once sent up. A node that reads one the line does not give is left out, with whatever cannot do without it. |
| `{moves}` | Where the clips the line names with `moves=` go, as text a node reads coordinates from: a list of tracks, one a clip, each a list of `{"x": …, "y": …}` a frame asked for, in pixels of the size asked for, the sequence filling that size as its guide does. On a frame a clip does not show on it is where it last showed. |
| `{moves_box}` | The same, five tracks a clip: its box's middle and four points toward its corners, so how large it is on each frame is said too. |
| `{signal_video}`, `{signal2_video}` … | The same signal as a picture in motion: a grey video of the size and frames asked for, each frame as light all over as the signal is then, sent up as a file is. For a node that reads a mask a frame, which a list of numbers does not reach. |
| `{signal}`, `{signal2}` … | A signal the line gives, in the order it gives them, as a number for each frame asked for, with commas between: `0,0.25,0.5,1`. A node that reads one the line does not give is left out the same way. |
| any other, such as `{steps}` | The value the workflow sets, or the same name on the line: `steps=30 strength=0.6`. |

A node left out takes with it what the server says cannot do without it,
and an input the server lets a node do without loses its link. Where a node
of your workflow is nothing without an input the server would let it run
without, a second guide laid over the first in a node of its own, say so
beside `workflow`, and which node stands in for it:

```json
"without": { "22": { "wants": ["control_video"], "reads": "9" } }
```

Node 22 is then left out when its `control_video` is, and what read node 22
reads node 9, output for output. `shot-directed` is written this way.

What it does with a request:

- It says how far along the job is. Asked after over HTTP, a server says only
  that a job waits, runs, or is done, so each line is how long it has run
  (`running, 45 s`). Which step of how many a sampler is on, ComfyUI says
  over a websocket and nowhere else: `kerf-comfy` listens there too, at the
  server's own address (`/ws`), with the key it sends its other requests, and
  where that opens the line says it (`running, step 7 of 20, 45 s`), in the
  editor's toolbar, the graph's head and an agent's job alike. Where it does
  not open, behind a proxy that passes no upgrade, the making goes on as
  before with what it has. `COMFY_PROGRESS=off` does not try.
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

### A workflow of the shot's own

A workflow that comes with `kerf-comfy` is one way of running its model.
Changing how a shot's model is run, its sampler, how a loader reads its
weights, a node's own number that no blank reaches, does not mean changing
`kerf-comfy` or anything on the ComfyUI server. The shot keeps a copy of the
workflow in the project, beside its recipe, and its line names it:

```text
generate model=video prompt="a basketball bounces on a court" guide={depth.mp4} seed=7 workflow=bounce.video.comfy.json
```

`workflow=` is a file of the project, named from the recipe's folder as any
file a line names. The request holds where it is (`"workflow"`), and
`kerf-comfy` runs it in place of the workflow its entry names, saying so
(`with the recipe's own workflow, …`); the model's entry in your models file
is as it was, so every other shot is made as before. Being a file the recipe
reads, it travels with the project, a recipe that names one that is not there
cannot run, and a take is held to it: change the copy and the shot wants
making, naming the file. Take the word off the line and the shot is made with
the workflow as it comes again. A model whose program takes no workflow has
no use for the word.

In the editor the copy is made for you. In a shot's graph
([As a graph](#as-a-graph)), a model your entry runs with `kerf-comfy` opens
into its workflow: every node of the ComfyUI graph and every input of each. An
input is a link to another node, a blank the recipe fills (marked, with what
fills it and what the line gives for it now), or a value the workflow itself
sets, which is yours to set there. The first value set copies the workflow as
it comes into the project as `<shot>.<model>.comfy.json`, with that value
changed and every other byte as it was, and adds `workflow=` to the line; the
next is a change to that file. A blank is set on the recipe's line, and a link
is the graph's own shape: to rewire a workflow, edit its file, or export one
from ComfyUI for its API and put the blanks in.

Once the workflow shows, what each value's input takes is asked of the server
the model is run on, by the model's own program (`kerf-comfy --workflow … --inputs`,
which asks the server what each kind of node takes): a sampler's name, a
loader's weights, become a menu of the names that server has, and a number
says the least and the most it takes and is not sent outside them. Asking
takes some seconds and the workflow is shown and set meanwhile; with no
server named or none answering it stays as it was, a value typed as text, and
is held to nothing until a making. A value the server does not take is
refused when it is set, with what it does take.

An agent reads the same workflow with `look` (`workflow`) and sets a value
with `edit` (`workflowSet`), by the same rule ([An agent's part](#an-agents-part)).
Its copy is always a new file, so that the recipe before, and every take made
with it, still has the workflow it names.

ComfyUI is another program. Kerf reads workflow files, and `kerf-comfy` asks
a server over HTTP when a shot is made, and listens at its websocket for how
far along the job is; it loads none of ComfyUI's code and changes none of
its files.

### A picture read back

A model can go the other way from a shot: from a picture in motion to what
the picture is of, how near each pixel is, the shape of what it shows, where
the camera was. To Kerf it is a model like any other, whose file is not a
shot: `generate model=<reader> guide=<footage>`, with `out={depth.mp4}`
where a later step reads what it makes. Five such workflows come with
`kerf-comfy`, for the models ComfyUI has nodes for:

| Workflow | What it makes of its guide | As |
|---|---|---|
| `depth` | How near each pixel is, nearer whiter, black where there is no surface (a sky). Each frame is stretched to its own nearest and farthest. `output=normal_opengl` gives which way each pixel faces, and `output=mask` what is a surface. | a video |
| `set` | One frame rebuilt as a mesh with the frame as its colors, for a `gen=mesh@1` clip. `frame=<n>` picks the frame, from 0; `decimation=1` keeps every pixel as a corner, where 2, as it comes, keeps every other. With `out={lens.txt}` it writes the lens that frame was seen through, degrees down the picture, in place of the mesh. | a `.glb`, or a `.txt` |
| `depth-whole` | How near each pixel is, a picture at a time, by another model: it gives a sky a distance, where `depth` gives it none. | a video |
| `cameras` | The frames together as views of one scene: where the camera was on each (`out={cameras.txt}`), or the depth it then agrees on across them (`out={depth.mp4}`). At most 81 frames. | a `.txt`, or a video |
| `set-views` | The frames together as views of one scene, and one of them rebuilt as a mesh with the frame as its colors: a set in the units of the camera path found of the same frames. `frame=<n>` picks the frame; `confidence=0` keeps every pixel, where 0.1, as it comes, keeps what the model is surest of, the near ground of a street. At most 81 frames. | a `.glb` |

One more reads a file that is not a picture. `set-splats` takes a scene
kept as splats (`ref=<scene.ply>`, or `.spz`, `.splat`, `.ksplat`) and makes
the mesh of it a `gen=mesh@1` clip reads, a `.glb` with the splats' colors:
`generate model=<name> ref=sets/scene.ply out={set.glb}`. It runs no model,
only the server's own nodes, so whatever made the splats, the set comes to
Kerf the same way. The mesh is in the file's units, turned from a splat
file's axes (y down, z forward) to Kerf's, so the camera the scene was
filmed from is where it was. `resolution=<n>` is how fine the surface is,
384 as it comes and up to 768; `solid=true` closes a single thing, where a
scene is left open.

A workflow is named for what it makes, and the model it runs is in its
graph. A recipe names neither: it says `model=depth`, the name the person's
file gives whichever reads depth for them, so that the recipe still reads
when the model behind it is another.

```json
{
  "depth": { "run": ["/path/to/kerf-comfy", "--workflow", "depth", "{request}"] },
  "cameras": { "run": ["/path/to/kerf-comfy", "--workflow", "cameras", "{request}"] }
}
```

```text
generate model=shot prompt="a red sports car drives along a wet street at dusk" guide={guide.mp4} seconds=4 seed=7 out={take.mp4}
generate model=depth guide={take.mp4} out={depth.mp4}
generate model=cameras guide=../footage/shot.mp4 out={cameras.txt}
```

Such a workflow says `"of": "guide"` in place of a rate, and what it makes is
of its guide frame for frame: the guide's own rate, each frame it has up to
the workflow's `frames.most`, and its own size, each side made even, down to
the workflow's `size` by area for a guide larger than that. `seconds=` reads
that much of the guide from its start. The guide goes up as it is but for
those, and with no `guide=` nothing is asked of the server. Its entry, as
`--describe` prints it, has no `size` and no `rate`, so `look` offers no
`render` line for it: a guide is not drawn for it, it is given one.

- **Which file is fetched goes by the name asked for.** `cameras`
  saves a video and a text, and `out={cameras.txt}` takes the text.
- **`depth` refines once**, where its model comes refining three times.
  Three took over twice the time on the server it was measured on (67 s
  for 29 s, 65 frames of a take), for a depth a grey of 255 from this
  one's, at the mean; `refine_steps=3` asks for it. A set of one frame
  takes 6 s either way, so `set` is left as its model comes.
- **A camera's path is found only with the model's weights as 32-bit
  floats**, which `cameras` sets where it loads them
  (`weight_dtype=fp32`): as it comes, or as `bf16`, the job fails on a
  dtype. `pose=ray_pose` finds the path from the model's rays in place of
  its camera decoder, three times as slowly.
- **The cameras are what ComfyUI prints of the model's answer**, there
  being no node that writes them: a text holding `'extrinsics': tensor(…)`,
  three rows of four numbers a frame, the way from the world to the camera
  with x right, y down, and z ahead, and `'intrinsics': tensor(…)`, each
  frame's lens in pixels of the size the model ran at (`resolution`, 504 on
  its longer side), among the depth it also holds, cut short. PyTorch prints
  a tensor whole up to a thousand numbers, which is 83 frames, so the
  workflow reads 81 at most, and a file cut short is refused.
- **A set can be a media of its own**, whose recipe's last line makes the
  `.glb` (`media set path="shots/set.glb" … recipe="shots/set.kerfrecipe"`),
  shown by a `gen=mesh@1` clip whose `file=` is the media's path. Each take
  is kept as any take is, in `shots/set.takes/`, and the mesh clips that
  read the media's file read the take; choosing another moves them with it.
  A line names one `out=`: the lens of the same frame is a `generate` line
  of its own.
- **A set comes without its camera.** `set`'s mesh is as the frame's
  camera saw it, the camera at the file's own origin looking along its z
  backward, as a glTF camera does. The file holds no camera and no lens, so
  `look` with `mesh` and `camera` has none to write. The lens comes as a
  file of its own, one number (`out={lens.txt}`).
- **A set and a path of one size come from one answer.** A set one model
  rebuilt and a path another found are each in units of their own.
  `set-views` makes the set from the same reading of the frames a path
  comes from, so the two fit as they are; for two that do not, `look`
  finds the size from the footage (`fits`, below).
- **A set of that reading is in the space of the frame the path counts
  from**, whichever frame it shows. Read together, the frames are placed
  from one of them, which the model picks and which is seldom the first:
  of 65 frames filmed from a car it was frame 36. The path has that frame's
  camera at its origin, not turned, and a set rebuilt of frame 0 is where
  frame 36 would see it. So for such a set `frame` is the path's `anchor`,
  which `look` returns, and not the frame the set is of; a shot that hardly
  moves hides the difference, and one that travels draws nothing without
  it. Sets of several frames are all in that one space, and `look` with
  `mesh` and `beside` puts one where another is: `{"mesh": {"clip":
  "set32", "beside": "set"}}` returns the `height`, `x`, `y`, and `z` that
  make the two one set, the clips standing still and not turned.
- **A set of one frame is what the camera leaves.** What a frame rebuilds
  is what it saw, nearest best, and a camera that travels is past the near
  ground of its first frame within a second. As it comes (`confidence=0.1`)
  a set of frame 0 was off the picture by the middle of a four-second
  drive; with `confidence=0` it held the far houses to the end and no road.
  A set of a later frame, put in beside it, has the road again: five sets a
  second apart, each with every pixel, left 6 to 13% of the frame empty
  after the first, where one left 35 to 59%. What moved in the footage is
  in each set where it then was, so with every set shown throughout, a car
  ahead is a smear along the road by the end. Have each set's clip last
  only about its own frame, overlapping the next, and what moved is where
  it is at each time and nowhere else; fewer sets then show at once, and
  17 to 32% of the frame was empty.
- **`set-views` sends its guide up at the size its model runs at**, 504 on
  the longer side. The server's node makes the mesh over the guide's own
  pixels with a lens measured in the model's, so a guide of another size
  comes back as a set seen through the wrong lens: at 640x360 its ground
  stood a quarter too wide.

Both come into the sequence as a `gen=camera@1` clip, by `look` with `mesh`
and `from` in place of `camera`: a file in the project's folder that says
where a camera the mesh's file does not hold was.

```json
{ "mesh": { "clip": "set", "from": "shots/views.takes/001.work/cameras.txt", "rate": 16 } }
```

It returns the lines to run with `edit`, as it does for a glTF's camera:
the camera where the file has it on each frame of the mesh's clip, the mesh
being where its clip puts it, with as few keyframes as say it.

| In `mesh` | Says |
|---|---|
| `from` | The file. What a model wrote of a shot's camera is a path. A file that is one number is a lens alone, degrees down the picture: the camera a set was seen by, standing at the set's own origin. |
| `frame` | The frame of `from` whose camera is at the mesh's own origin (the `anchor` is the frame nearest it, a model leaving it a little off), 0 unless given, the others being where they were from it. For a set one model rebuilt of a frame alone (`set`), that frame, the `frame=` its line gave. For a set of the path's own reading (`set-views`), the path's `anchor`, whichever frame the set shows. |
| `rate` | How many frames of `from` a second has, the sequence's unless given: a path of a shot at 16 a second in a sequence at 24 is spread over half as many frames again. |
| `scale` | How much of the mesh's file one of the path's units is, 1 unless given. A set and a path come from two models, and neither is in metres the other knows. |
| `fits` | In place of `scale`: the media the path is the camera of, to find the size from. |
| `lens` | A lens for the camera, degrees down the picture, in place of the one `from` has: the lens a set's own model found (`out={lens.txt}`), where the two models do not agree on it. With `fits` it need not be given: the set's own lens is found from the footage first. |
| `as` | A camera clip to `set`, where a new one is not wanted. |

What comes back with the lines is `from`: its `frames`, how many of the
clip's were `placed`, and the least and most `fov` it found. The camera
gets the middle one of them: a `gen=camera@1` has one lens, and what a
model finds of a lens wavers by the frame, so a zoom does not come through.
A path shorter than the mesh's clip holds still after its last frame, and
is said to. Read by hand it is `kerf-mcp/src/camera_file.rs`. `anchor` is
the frame a path counts its places from, and a `note` says so where `frame`
is another.

**A size found.** A set of one model and a path of another are each in
units of their own. With `fits`, `look` finds how many of the set's one of
the path's is, from the footage and with no model
(`kerf-mcp/src/set_scale.rs`): the set carries its frame as its colors, so
seen from where the path has the camera on another frame it is a picture
of that frame, at the right size and at no other. It is drawn through the
path at 49 sizes from a sixty-fourth to sixty-four, on the four frames
furthest from `frame`, each held to the footage's own frame over the
pixels the set covers, and then more finely about the best.

```json
{ "mesh": { "clip": "set", "from": "cameras.txt", "frame": 0, "fits": "footage" } }
```

| In `fit` | Says |
|---|---|
| `lens` | The set's own lens, found first where none is given: the one through which the set, seen from `frame`, is that frame of the footage again, with its `error` and `sure`. A set is rebuilt along the rays of the lens its model took the frame to have, so only through that lens does it draw its frame; where it is sure (1.1 or more) the camera gets it, and else the path's. |
| `deep` | How many times further the path's travel along the way the set was seen is made, where the set's lens is not the path's: a set through a narrower lens is deeper by the halves of the two, and a path that goes forward in it goes that much further. 1 where the lenses are one. |
| `scale` | The size found, which the lines returned are written at. |
| `error` | How far the set's picture is from the footage there, of 255 a pixel, over what the set covers. |
| `sure` | How many times worse half or twice that size is. Near 1, any size fits as well: the camera turns and does not travel, or the set is of another frame than `frame`, and a `note` says so. |
| `frames` | The frames of the path it was read on. |

It takes a camera that travels, and about ten seconds. Where the two
models do not agree on the lens the path is stretched along the way the
set was seen (`deep`), which brings the frames nearer and not to the
pixel, and `sure` stays low: look at the frames.

What is added to the set then goes on a track of its own, over the
camera's or under it: the lines `look` writes put the camera on top, and a
camera's track does not come between what is in depth either side of it,
so the set hides what is behind its surfaces wherever the camera's clip
is. A track with a flat clip between the two does part them. `look` with
`shows` says how much of what is added the set hides, and so whether it
stands on the set's ground or is in it.

A take's depth read back is held to its guide by `look` with `follows`, the
media that depth is, and `sequence`, the guide's:

```json
{ "follows": "readback", "sequence": "s2" }
```

The media's frames are the sequence's from its start, as a guide drawn at
the model's rate has them. Each pixel the guide has a depth for is given
its place in its frame's order of near and far, from 0 to 1, in the guide's
depth pass and in the take, since order is what the two share: a pass goes
by the scene's own numbers and a model stretches each frame to its own. For
each clip the guide has in depth, by the pixels its ids pass gives that
clip:

| | Says |
|---|---|
| `off` | How far apart the two places are over the clip's pixels, as a mean over the `frames` it shows on. |
| `farther` | The same with its direction kept: above 0 where the take has those pixels farther, as it does where the subject is not there and what was behind it is. |
| `worst` | The frame it is most off on, from 0, and how far. |
| `shows` | The share of the frame it has. |

`off` at the top is the same over every pixel, and `order` the rank
correlation of the whole frame, which a take with its subject elsewhere
still has near 1 where most of the frame is ground. A subject's `off` is
read beside the frame's and the floor's: a take that followed has them
alike, and one that did not has the subject several times the floor
(`kerf-engine/src/follows.rs`).

`follows` reads depth, so it does not say where the subject is: a crate a
model drew a tenth of the frame to one side reads as the ground behind it,
and a take with no crate at all can have the right depth where one was
asked for. Where the take has it is `look` with `placed`, which reads the
take's own picture and needs no model:

```json
{ "placed": { "take": "take", "clip": "crate" }, "sequence": "s2" }
```

`take` is the media whose picture is the take, and `clip` a clip the guide
draws. The guide's ids pass gives the box around the clip on each frame, as
`shows` does. The subject is found on the frame the guide shows most of it
on, in that box with half as much room again on each side, and followed
from there to both ends (`kerf_track::roto`), once in the take and once in
the guide's own picture, and it is those two boxes that are held together:
what a box followed in a picture adds to the box of an ids pass, its soft
edge and what is hidden of it, is then in both
(`kerf-engine/src/placed.rs`).

| | Says |
|---|---|
| `off` | How far the middle of the take's box is from the middle of the guide's, in frame heights, as a mean over `frames`. |
| `across`, `down` | The same with its direction: above 0 where the take has it to the right, or lower. |
| `size` | How large the take drew it beside the guide, by the sides of the two boxes: above 1 larger. |
| `worst` | The frame it is furthest off on, from 0, and how far. |
| `overlap` | How much of the two boxes is in both, of what is in either, as a mean over every frame the guide shows the clip on, a frame it was not followed on counting as none. 1 where the take has the subject where and as large as asked throughout, 0 where it has none: the one number to choose between two takes by. The guide's own picture through an encoder read 0.90, and moved a crate's width aside 0.22. |
| `shown`, `frames` | How many frames the guide shows the clip on, and on how many of them it was followed in both pictures. |
| `from`, `read` | The frame it was found on, and the first and last it was followed on. |

A take that has nothing to follow where the guide has the subject is
followed on a frame or two, or refused: read `frames` beside `shown` before
`off`, a `note` saying so where they differ. The tracker goes by color and
edges and is not told what the subject is, so it holds what the picture
shows apart from its ground.

Both at once, to choose between takes by, is `look` with `held`: the
take, its depth read back, and the guide's clips that are its subjects.

```json
{ "held": { "take": "take", "depth": "readback", "clips": ["crate"] }, "sequence": "s2" }
```

It is `follows` of the depth and `placed` of each subject, and for each a
`held` from 0 to 1: the `overlap` of where the take has it with where it
was asked for, less by how far its depth is `off`. The `held` at the top is
the mean over the subjects. A subject the take has nothing of is held 0,
whatever its depth: a set is not named, since it is where it is whatever
the take did.

What they took, and how a take read back holds beside its guide, is in
[shots-measured.md](shots-measured.md) (3 and 4).

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
model that reads one by name. Of the workflows that come with `kerf-comfy`
one reads a second guide, `shot-into`: a shot that keeps its guide where
`guide_mask` is black and makes it anew from the prompt where that is
white, for a thing put into footage. The guide is the footage as it is,
and the mask the thing in white on black, drawn from the sequence that
holds it, its camera, and the set that hides it:

```text
render s2 {mask.mp4} matte=crate size=832x480 rate=16
generate model=into prompt="a red shipping container on the street" guide=../footage/shot.mp4 guide_mask={mask.mp4}
```

And
one reads a signal, `shot-timed`: a shot that follows its guide where the
signal is 0 and is made anew from the prompt where it is 1, the signal
reaching the model as a mask a frame. That is for a guide that is a
picture: where the signal is 0 the take keeps the guide's own pixels, so a
depth guide read that way comes back as its depth. For a depth guide there
is `shot-directed`, which reads a signal beside a guide, a reference and
where a clip goes: on the frames where the signal is 1 the guide is handed
over as plain grey, which tells the model nothing, and those frames are made
from the prompt, the reference and the layout alone
([Where a thing goes](#where-a-thing-goes)). `shot` has no blank for one, so a
line that names a signal for it is told that it is left out. A workflow of
your own takes one at a `{signal}` blank, in a node that takes a list of
numbers, a frame each, or at `{signal_video}`, as a grey video of the
frames asked for, in a node that reads a mask or a picture a frame
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

A person asks for the same in the shot's graph: a signal's card offers
**From a sound**, the clips of its sequence that play one, and choosing one
sets the signal's keys from where its sounds start, one edit that undo takes
back.

`hold` is how many frames each lasts, 3 unless said; onsets nearer each other
than that run together, and those that fall outside the signal clip are
counted and left out. It reads onsets, where a sound starts, and not how
loud it is.

The editor shows a signal where its clip is, in the timeline: its number
along the clip as a curve, 0 at the bottom and 1 at the top.

It is drawn by hand there too: Option-drag across the clip, or drag across its
curve in the shot's graph ([As a graph](#as-a-graph)). What is drawn is kept as
the keys that say it, a straight run as its two ends.

Not here yet: a signal from how loud a sound is, as a voice swells, and from
footage, such as how fast something moves.

## Where a thing goes

A guide shows a model where things are, as a picture it follows. Some models
take the way a thing goes as numbers of their own, a point for each frame,
and make the picture go that way. The document has those numbers: a clip is
where its sequence draws it. A `generate` line names the clip, and the
request holds its place on each frame of its sequence, from the first:

```text
generate model=led prompt="a crate on a street" first={first.png} moves=crate
```

```json
"moves": [{ "name": "crate", "rate": 16.0, "frame": [640, 360],
            "places": [[0.4701, 0.5602, 0.0286, 0.0648], null] }]
```

- A place is the middle of the clip's box across and down the sequence's
  frame, and the box's width and height, each from 0 to 1. It is read from
  the frame and not from the clip's own numbers, as `look` with `shows`
  reads it, so a camera that moves, a parent, and what stands before the
  clip are all in it. On a frame the clip does not show on it is `null`.
- `frame` is the sequence's size. A guide of another shape has its sides
  cut off, so a share across the sequence is not the same share across the
  model's picture; a model's program brings one to the other, as
  `kerf-comfy` does at a `{moves}` blank.
- A clip that draws nothing, a camera or a light, is nowhere and is refused.
  A model whose `reads` has no `moves` is said to have no use for it, and
  the making goes on.
- A take is held to the clip's whole sequence, as to one it renders: moving
  the clip, the camera, or what hides it asks for the shot again.

A workflow reads it at `{moves}`, as tracks of points, or at
`{moves_video}`, as a layout: a video of the size and frames asked for,
white, with each clip's box outlined in black where it is on that frame.
`shot-led`, which comes with `kerf-comfy`, hands a model that layout and the
prompt and no picture of the scene: the subject the prompt names goes where
the clip goes. `{moves_box}` is the same text with five tracks a clip, its box's middle and
four points toward its corners, so that how large the clip is on each frame
is said with where. `shot-moved`, which comes with `kerf-comfy`, hands a
model that, a picture to start from (`first=<picture>`), and the prompt:
`generate model=<name> first=refs/start.png moves=<clip> prompt="…"`. What
is in the picture where the clip starts goes where the clip goes, in 4
steps, by weights added to the model's for that (`lora=`); the rest
of the picture stays about where it was, since nothing is said of it.
`shot-directed`, which comes with `kerf-comfy`, reads all four directions on
one line:

```text
render court {depth.mp4} pass=depth size=832x480 rate=16
generate model=video prompt="a basketball bounces on a court" guide={depth.mp4} ref={ball.png} moves=ball signal=bounce
```

The depth guide and the reference go to the model as `shot` hands them, the
clip's layout is laid over them in a node of its own (`moves_strength=` is
how hard it holds, 1 unless the line says), and the signal lets the guide go
where it is 1. A line that gives only some of the four is served by the same
workflow: with no `moves` and no `signal` it sends what `shot` sends. The
layout has a clip's box only on the frames the guide's sequence shows it, so
a subject hidden there, behind a floor it dips under, is not in the take on
those frames either.

`moves-shown`, which comes
with `kerf-comfy` too, has the node that reads points read `{moves}` and draws them over a
guide (`generate model=<name> guide=<take> moves=<clip>`), with no model.
It is what a model that follows points is told, to look at before a take or
over one.

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
  A model that follows depth is handed the footage's depth, which another
  model reads from it first ([A picture read back](#a-picture-read-back)):
  `generate model=depth guide=../footage/shot.mp4 out={depth.mp4}`, and then
  `guide={depth.mp4}`.

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
- **`matte=<clip>`**, in place of a pass: that one clip in white on black,
  as much of it as shows, so what stands before it is cut out of it. It is
  the mask a model is handed to make a thing anew in one place
  (`guide_mask={mask.mp4}`).
- **`pass=normals`**: which way each pixel faces, as a color, in the
  camera's own space: to the right as red, up as green, and toward the
  camera as blue, each from -1 to 1 as black to full, as a normal map has
  them (the one a model that reads a picture back calls `normal_opengl`).
  A clip in depth is flat and so one color; a mesh has the way each place
  of it faces; a surface seen from behind faces whoever sees it. What is
  not in depth is black.
- **`pass=relative`**: the same as `pass=depth`, each frame stretched to its own nearest
  and farthest: white at the nearest thing in the frame and one step above
  black at the farthest, as a model that reads depth from a picture gives
  it and as one trained on such depth expects it. It does not hold still:
  a place is another grey when something nearer comes into the frame. It
  is what to try where a scene's own depth comes out flat, a far set with
  one thing near.

A pass of how near things are or which way they face, of a sequence with no
clip in depth, is black on every frame; a sequence that only nests another
is such a sequence, the nest being one flat clip. Making it says so before
the model is asked: render the nested sequence itself.

A pass has no lights, no focus, no adjustment layers, no dips, and no
captions, and every clip is laid over what is behind it whatever its blend.
It is drawn on the CPU. The sides of a clip with a `depth` take the grey of
its face beside them, as they take its color, and not their own distance.

A pass is a file as a guide is, and a model that follows one is handed it as
its guide (`guide={depth.mp4}`); what kind of guide a model follows is its
workflow's to say. One model is handed a picture and a pass of it at once
by naming the second, `guide={picture.mp4} guide_depth={depth.mp4}`
([Recipes](#recipes)).

Which kind a model follows is the model's, and its entry's `about` says. A
model that covers a render is handed the picture. The
small model `kerf-comfy`'s `shot` is written for made more of a depth
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
