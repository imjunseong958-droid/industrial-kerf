---
name: wait-speedup
description: Play a stretch of a screen recording fast where nobody speaks but the screen shows something going on, an install, a build, a page loading, and cut the stretches where nothing does.
---

# Waits in a screen recording

- A wait is a stretch where nobody speaks. What the screen shows in it
  decides: play it fast where something is going on, cut it where nothing
  is.
- Find them in the transcript. `look` with `media` and `transcript`: where
  one cue's `to` is seconds before the next one's `from`, nothing is said.
  `unheard` marks those that hold sound as loud as speech, keys or a
  system sound. `look` with `cuts` as `{"pauses": 2}` lists the quiet ones
  as the sequence plays them, by the words `after` and `before`. A wait
  with sound in it reads short there, so go by the cues for those. Under
  two or three seconds is a pause, for `speech-cleanup`.
- Look at each before deciding: `look` with `sheet`, eight frames or so,
  `from` and `to` the wait's ends on the sequence. A media's time is a
  sequence position as `<clip>@<media time>`.
- Speed it where the frames differ in a way that tells the viewer
  something: a bar filling, a log scrolling, a page drawing. Cut it where
  they are one frame, or differ by a spinner or a clock alone:
  `extract <clip> pause="…" leave=<dur>`. Where that line is refused for
  sound in the wait, tell the user where it is and do not cut it by its
  times. Where it is not clear that what shows matters, ask.
- A wait ends before its result shows. The page that has loaded, the test
  gone green, plays at its own speed for as long as it takes to read.
- Words said during a wait play as they were said. Speed the silent parts
  either side of them, never speech.
- Three clips make a sped wait, split from the sound clip `cuts` names:
  `split <clip> at=<pos> snap=pause as=wait1` after the last word before
  it, `split wait1 at=<pos> snap=pause as=rest1` before the first word
  after it, `speed wait1 <speed>`, then
  `extract from=wait1.end to=rest1.start`. A speed shortens a clip where
  it stands and moves nothing after it, so the last line closes the gap.
- The picture linked to the sound is split and sped with it. A clip on
  another track that is not linked to them, a camera beside the screen, is
  only cut by the last line: before it, split that clip at `wait1.start`
  and `rest1.start` and give the part between the same speed.
- Where a wait's own sound leaves no pause beside a word, `snap=pause` is
  refused. Split a few frames clear of the word's time without it, and
  read what `cuts` says of that cut afterwards: `quiet` or `low` will do,
  `sound` or `inWords` will not.
- Choose the speed so the wait plays for two to five seconds: its length
  over that, so forty seconds at 10 play four, and 100 is the most a clip
  takes. Give one with stages worth seeing more time.
- Sound played fast keeps its pitch and turns to chatter. Turn the wait's
  sound off: `set wait1 enabled=false`, which is the sound clip when the
  splits named the sound clip.
- Do the last wait of the sequence first, so that the positions read for
  the earlier ones still hold.
- Look at what it made. `look` with `sheet` from `wait1.start` to
  `wait1.end` shows the thing going on, and the frame after it the
  result. `look` with `cuts`: `summary` has no `inWords` and no `sound`,
  and the `joins` either side are whole. `look` with `transcript` over
  it: nothing heard in the wait, and every word either side still there.
- Say for each wait where it was, how long it ran, and what it plays for
  now or that it was cut, and which were left as they were.
