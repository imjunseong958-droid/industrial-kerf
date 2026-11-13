# Effects, looks, and versions

> Status: the plan at the end is in: versions, the built-in effects, looks
> (the reader, the catalog layers, expansion, `flatten`, and 29 bundled
> looks in `crates/foundation/kerf-model/looks`), presets that carry their looks, and
> markers placed on new music's beats.

Kerf draws with a small set of **built-in effects**: transitions, color,
blur, keys, distortion, transform, and the few others an editor reaches for.
Everything playful on top of them, the shakes, zoom pulses, and flashes of
a short-form editor, is a **look**: a text file in the Kerf
language that combines built-in effects, keyframes, and masks behind a few
parameters. Looks need no new Kerf build, can be shared like presets, and are
written by people and agents alike.

## Versions

Every effect, transition, generator, and look has a version, a whole number
from 1. Documents name them as `id@version`:

```text
clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00
  fx blur.gaussian@1 radius=8
  fx look.shake@2 intensity=0.7 hit=12f
clip t1 gen=title@1 at=00:00:00:00 dur=00:00:02:00
transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
```

- Kerf always writes the version. A reference without one, from an older
  document or a hand-written script, takes the version this Kerf has.
- A built-in effect's version goes up when the same parameters draw a
  different picture. Kerf carries only the current version of each built-in
  effect, and keeps reading documents of the versions it says it still
  reads; such a document is drawn by the current version, and checks say
  so. A version newer than this Kerf's, or older than it reads, is an error
  that names the version it has.
- Looks are data, so several versions of one look can be installed side by
  side, and a document draws exactly the version it names.

## Looks

```text
kerf-look 1
look look.shake version=1 name="Shake" category="Fun" on=clip
  doc "A hand-held jolt that settles: hits, impacts, punchlines."
  param intensity number default=0.5 range=0..1
  param hit time default=0f
  param settle time default=10f range=2f..2s
  set x={$hit:0 $hit+$settle*0.2:0.02*$intensity $hit+$settle*0.4:-0.015*$intensity:ease $hit+$settle:0}
  set rotation={$hit:0 $hit+$settle*0.2:1.5*$intensity:ease $hit+$settle:0}
  fx blur.directional@2 direction=90 length={$hit:0 $hit+$settle*0.2:30*$intensity $hit+$settle*0.6:0}
```

A clip uses it like any effect, `fx look.shake@1 intensity=0.7 hit=12f`, and
the document keeps that one line. The look is expanded only to draw, so
people, agents, and presets read the intent rather than its keyframes.
`fx c1 flatten 1` replaces the clip's first effect, the look, with what it
expands to, for hand tuning.

A time can instead name a marker of the clip's sequence:
`fx look.shake@1 hit=beat1` hits wherever `beat1` is, and moves with it. A
preset applied to new music can move such markers onto its beats
([presets.md](presets.md#looks-and-beats)).

- **Ids** start with `look.`, which no built-in effect uses. A shared look
  can carry its author: `look.alice.shake`.
- **`on=`** is what the look applies to: `clip` (a picture clip), `title` (a
  title generator), or `transition` (see below).
- **`doc`** says what it does and when to reach for it, as the catalog's
  guidance does for built-in effects; agents choose looks by it.
- **`param`** lines declare its parameters: `number`, `time`, `angle`,
  `color`, `choice` (`options=a|b|c`), or `switch`, each with a default and
  an optional `range=` and `doc=`. Times are relative to the clip, like
  keyframe times, or name a marker; frames in a `param` line count at 30 a second for the
  catalog's ranges, and at the sequence's rate when the look expands. A
  number animates, `amount={0f:0 24f:1:ease}`, where the look's lines keep
  it a straight line through its keys: scaled and added to, alone in its
  expression, outside the look's own keyframes, and going to something that
  animates. The same keys land on each value it reaches, so the look draws
  as if each of them were keyed by hand. Other parameters do not animate,
  nor do a transition's, and a document that tries is refused.
- **`set`** lines change the clip's properties, combined with the clip's own
  so a look rides on top of the edit: `x`, `y`, and `rotation` add; `scale`,
  `scale_x`, `scale_y`, and `opacity` multiply. Other properties are not
  available to looks. A keyframe's interpolation (`:ease`, `:hold`, …) is
  that of the move from it to the next, as in documents.
- **`fx`** lines are built-in effects with a pinned version, run where the
  look sits in the clip's effect stack. Looks do not contain other looks.
- **`mask`** lines add masks for the look's own effects to read. Their names
  are local to the look and do not clash with the clip's masks.

### Expressions

Where a document takes a number or a time, a look may write an expression:
`$name` for a parameter, numbers and times (`12f`, `0.5s`), `+ - * /`, and
parentheses. Nothing else: there are no functions, conditions, loops, or
calls out, so a look always expands the same way and is safe to open from
anyone.

### Titles

A look `on=title` animates a title's properties and runs effects on it: a
pop-in, a wobble. Setting a title's own parameters (its text, size, color)
is for later.

### Transitions

A look `on=transition` combines the clip it leaves (`a`) and the clip it
enters (`b`). Its keyframe times are percentages of the transition, from
`0%` to `100%`:

```text
kerf-look 1
look look.zoom-through version=1 name="Zoom Through" category="Transitions" on=transition
  doc "Punches into the outgoing shot and lands on the next; energetic cuts on a beat."
  param strength number default=1 range=0..2
  a set scale={0%:1:ease-in 100%:1+0.6*$strength}
  a fx blur.radialzoom@1 amount={0%:0 100%:40*$strength}
  b set scale={0%:1-0.3*$strength:ease-out 100%:1}
  b set opacity={0%:0 30%:1}
```

It draws over the built-in `stack` transition: both clips show across the
cut, the incoming one over the outgoing, as the look's lines move them.

## Where looks come from

The catalog a document is checked and drawn with is layered:

1. Kerf's built-in effects.
2. The looks bundled with Kerf (`look.shake`, `look.zoom-pulse`, …), which
   are ordinary look files.
3. The project's looks, in `.kerf/looks/*.kerflook`.
4. Looks carried inside a preset, so a shared `.kerfpreset` draws the same
   on another machine without anything installed. Applying the preset
   copies the ones the project lacks into `.kerf/looks`.

A process that edits or plays a project reads its looks folder again when
a file there is added, removed, or changed, so looks one adds (an applied
preset's) are ones the others draw.

An id and version is defined once; two different looks claiming the same
`id@version` is an error that names both files.

## Limits

A look file is at most 256 KiB, with at most 32 parameters and 16 effects,
and every animated property expands to at most 256 keyframes. Looks are
checked when they are read: unknown properties, effects, versions, and
parameters, and expressions that do not give a number or a time where one
is needed, are errors with the line and column.

## Plan

1. Versions on every catalog definition and every reference to one.
2. A compositor that draws Kerf's model directly, built from FilmCraft's
   renderer, beside the current one.
3. That compositor by default.
4. FilmCraft's transitions as built-in transitions, drawn from both clips.
5. FilmCraft's video effects and a few kept EffectCraft ones as the
   built-in effects.
6. EffectCraft removed.
7. Looks: the catalog layers, the reader, expansion, `flatten`, and the
   bundled looks.
8. Presets that carry their looks, and beat markers re-placed on new music.
