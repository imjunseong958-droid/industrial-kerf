---
name: footage-parts
description: Change a part of filmed footage with the models set up, cutting a thing out by its name, taking it out, making it another thing, or putting it on a layer of its own, and choose each next step from what the take before it shows.
---

# A part of footage

- Start from the models. `look` lists `models`, and each one's `about` says
  what it makes: one makes a matte of a thing it finds by name, one draws
  what such a matte would find, each thing numbered, and two read a matte
  as `guide_mask`, one taking the thing out, one making it anew. `about`
  says which is which; `reads` does not. Use the names the list gives
  them. Where the one a step wants is not listed, say which is missing,
  and do not have another stand in.
- What is asked is seldom one making: a thing moved is its matte, a plate
  behind it, and the layers. Go from each to the next by what it shows,
  without being told to, and say at the end what was made and why.
- What a model makes is a media with its steps under it, written by the
  edit that adds it, with no file to write first: `media add gone
  path="shots/gone.mp4" …`, then the matte, `step gone add generate
  model=<matte> guide=footage/walk.mp4 prompt="man" out={man.mp4}`, then
  the step that reads it, `guide_mask={man.mp4}`, so a media makes the
  matte it reads, by the words of the one looked at. The guide is the
  footage's own file, named from the project's folder: nothing is drawn for
  it. A matte the document cuts by is a media of its own, its one step with
  no `out=`. A matte and a plate are of their guide frame for frame: their
  media's line takes the footage's `rate`, `dur`, and `video=`.
- A model reads its guide from the first frame, as many as its `frames`
  says at `most`. For a part further into a file, or of a longer one, put
  that part alone on a sequence of the footage's size and rate and draw it
  as the guide, `render <sequence> {part.mp4}`, then `guide={part.mp4}`:
  what is made starts where the part does.
- A matte's prompt is what to find, in a word or a few: `man`, `black
  pickup truck`, with `objects=1` for one thing. Among several of a kind,
  make the numbered picture first, read the numbers on a frame of it (`at`:
  a sheet is too small), keep one by `which=`, and take the picture's media
  out again (`media remove`): it was made to be read. Where words find the
  wrong thing or none, give points on it in the first frame, `inside="x,y
  x,y"`, each across and down from 0 to 1.
- Look at a matte before anything reads it (`media` with `sheet`): white on
  the thing through the shot, and on nothing else. All black is a prompt
  that found nothing. Other things white wants fewer `objects`, a `which`,
  or plainer words. A matte is seconds, and what reads it minutes.
- What is wanted says which model reads the matte. Another thing or another
  look in its place: the one that makes anew, its prompt the thing as it is
  to be, `grow` little where it keeps its shape and more for another. The
  thing gone: the one that takes out, its prompt what the picture shows
  without it, never the words for taking out. Do not take a thing out with
  the one that makes anew: it fills the shape of a man with a man.
- A matte does not cover a thing's shadow or reflection. Where the footage
  shows one, give the line that takes out `reach=`, in pixels, as far as it
  goes from the thing; without it the shadow stays where the thing was.
- A thing is a layer by its matte, in the document: the footage on a low
  track, the same footage on a track over it, shown through a third (`fx
  <clip> add key.trackmatte@1 matte=<track> composite=matte-luma`), and on
  the third the matte's media, the track turned off (`track set <track>
  enabled=false`). The three clips are at one `at` for one `dur`, each
  frame of the matte over its own of the footage. What is put between the
  two of the footage stands behind the thing.
- A layer that is moved, resized, or left out shows what was behind it, and
  the footage under it still has the thing there. Do not leave that: make a
  plate, the footage with the thing taken out, and once it is made put it
  on the low track in place of the footage. Set on the matte's clip
  whatever is set on the thing's clip. A layer made larger grows from the
  frame's middle: give both the `y` that keeps it on its ground. It has no
  shadow where it goes: say so.
- Look at each take where the thing was, and at the sequence's own frames
  (`at`) once the layers are laid, and mend what they show before going on.
  The thing still there: the matte missed it, or `grow` more. Its shadow
  left: more `reach`. The picture soft or strange around it: less `reach`.
  What was made anew not what was asked: plainer words, more `grow`, or
  another `seed`. The thing twice in the sequence: the low track wants the
  plate. Outside the mask a take is the footage's own: do not judge it there.
- A making costs minutes. Mend what `toMake`'s `cannot` says, then call
  `make` once the edit that wrote the steps is accepted, with `why`; let
  through, `wait` for the job, look at the take, and change one thing for
  the next (`step <media> set <n> reach=96`). Asked of the person instead,
  say which making comes first and what the next waits on. Stop and say
  what is left when the makings allowed run out, or when two takes running
  are no better.
