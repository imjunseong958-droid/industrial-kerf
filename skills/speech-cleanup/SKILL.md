---
name: speech-cleanup
description: Tidy what someone talking says by taking out a phrase said and taken again, doubled words and fillers, and long pauses, with every cut in quiet and every sentence still whole.
---

# Tidying speech

- Take out only what the speaker would not miss: a start taken again, a
  doubled word, a filler, dead air. No meaning changes.
- Read first. `look` with `media` and `transcript` for each media the
  sequence speaks from, 15 minutes a call with `from` and `to`: `said=`,
  `pause=`, and the lists below all read that transcript. Read it through
  before cutting. `unheard` is sound like speech with no words, not silence.
- Then `look` with `cuts`, as `{"pauses": 0.5}` for ordinary talk, since
  it lists them from 0.75 s otherwise: `restarts`, `repeats`, and `pauses`,
  each with its `clip` and where it is `at`. Each lists eight of its
  `count`; `from` and `to` take a long sequence a part at a time.
- Take out in this order: restarts, then doubled words and fillers, then
  pauses. Words go first because each cut of them leaves a pause of its
  own, and pauses are judged on what is left.
- A restart is `said`, then `between`, then `again`. Keep the later saying,
  the one the speaker settled on: `extract <clip> said="…"` with the words
  of `said` and `between`. Keep the first only where the second is the
  worse take, and leave both where the sentence means the repeat, as a list
  or an emphasis does.
- `said=` and `pause=` take words as the transcript heard them, found once
  in the clip named. Said more than once there, the line is refused with
  the times: give more words where those go too, or take the one saying by
  its times, `extract <clip> from=<pos> to=<pos> snap=pause`, read from
  `transcript` with `words`. A media's time is a sequence position as
  `<clip>@<media time>`.
- A doubled word is said twice in its clip, so `said=` cannot name one
  saying of it: take one by its times. Fillers are in no list: find them
  in the transcript, and take one out only where the line is better
  without it.
- A pause is `extract <clip> pause="…" leave=<dur>`, by the words it comes
  `after`. Leave 0.3 s to 0.5 s between sentences, and more where a thought
  turns or a point lands. One with `cutTo` lies across a cut, and the line
  shortens its own clip's side.
- In a script, work from the end of the sequence toward its start. A cut
  gives the later part of its clip a new id, so the clip a line names
  still plays its words only while nothing before them has gone. For the
  next script, name clips as the edit's diff has them.
- Do not force a refused line. Words that run on into the next have no
  quiet to cut in. A pause that holds sound at the level of speech which
  no word accounts for may be a word the transcriber missed, a cough, or a
  laugh, and the refusal says where to listen. Do not fall back on a plain
  `extract from=<pos> to=<pos>` or an edge without `snap=pause`. A script
  is all or nothing, so take the line out, run the rest, and tell the user
  each line refused, where it is, and why. It goes only if they say so.
- Read `snapped` after each edit: what went, from where to where, and the
  pause left between which words. A word there that was not meant to go is
  a wrong edit: put it right before going on.
- Then read it as it will be heard. `look` with `cuts` again, with
  `proposal` while the edit waits for review. Each of the `joins` reads as
  a sentence in `plays`, beside what it `took`: a cut that takes a word the
  sentence needs still measures clean. `summary` has no `inWords` and no
  `sound`, and a `tight` cut is moved with `trim` and `snap=pause`. Of
  `restarts` and `repeats`, none is counted in `atCuts` and none is left
  by mistake.
- The edit's check findings have no `cut.in_word`, `cut.in_sound`, or
  `cut.fade_in_sound`. `phrase.restarted`, `word.repeated`, and
  `pause.kept` are information: each one left is one you chose to leave.
- `look` with `transcript` and no `media` hears the sequence as it plays.
  Read it top to bottom once.
- Say what went, how long the sequence was and is (`# duration` on its
  line of the document), what was left on purpose, and what was refused.
