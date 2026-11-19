---
name: target-length
description: Bring an edit of someone talking down to a length the user names by taking out what costs least first, pauses, then phrases said again, then whole sentences, and reading the length after each step.
---

# Cutting to a length

- Know the numbers before cutting. The sequence's line of the document
  ends `# duration`: `look` with no arguments, or with `proposal` for what
  a pending edit would make. "Under" means under, and "about" within a few
  seconds. Work out how much has to go.
- Read first, as `speech-cleanup` does: `look` with `media` and
  `transcript`, then `look` with `cuts` as `{"pauses": 0.4}`. Read the talk
  through and mark what stays whatever happens: what the opening promises
  and where that is paid off, a line a later one points back to, anything
  the user named, and the only place a needed point is made.
- Take out in three steps, the cheapest first, and stop at the first step
  that meets the length. After each step, read `# duration` again.
- First the pauses, which cost no words: `extract <clip> pause="…"
  leave=<dur>`. `pauses` gives their `count` and the `seconds` they hold,
  so what this step can give is known before it runs. Leave 0.3 s between
  sentences and never under 0.25 s: talk with no air sounds cut.
- Second, what was said twice: `restarts` with the later saying kept,
  `repeats`, and fillers, each taken out as `speech-cleanup` takes it.
- Third, whole sentences, the one the video needs least first: an aside
  that never comes back, a second example of a point already made, a
  recap, set-up the viewer does not need. A sentence goes whole, first
  word to last, in one `extract <clip> said="…"`, never part of one to
  save a second. Its cue's `from` and `to` in the transcript say what it
  gives.
- After each sentence, read its join in `look` with `cuts`: `plays` still
  follows without what it `took`. A line left pointing at what went, an
  "it", a "that", an answer to a question now gone, goes too, or the
  sentence comes back.
- On a screen recording, a wait with nothing said goes before any
  sentence does: `wait-speedup`.
- Scripts run as in `speech-cleanup`: from the end of the sequence toward
  its start, clips named as the last diff has them, and a refused line
  left out and reported, never forced.
- Where edits wait for review, keep it one edit for the user: pass the
  pending proposal's newest id as `extend` for each step, and `look` with
  `proposal` reads what it makes so far.
- Stop at the target. Do not go on because more could go.
- Where pauses and repeats do not reach it and what is left is content,
  take out only what clearly does not belong, then stop and give the user
  the choices: each passage that could go, the seconds it gives, and in a
  sentence what the viewer would lose. Which point the video can do
  without is theirs to say.
- Check as `speech-cleanup` does before handing over: the `joins` read as
  sentences, `summary` has no `inWords` and no `sound`, the edit's check
  findings have no `cut.in_word`, `cut.in_sound`, or `cut.fade_in_sound`,
  and `look` with `transcript` of the sequence reads through.
- Say the length before, after each step, and now; what went at each
  step; what was refused; and what could still go, with what it costs.
