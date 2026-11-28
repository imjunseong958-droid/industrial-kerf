//! What an agent is told of the tools, and what each is given: the guide, each tool's
//! description, and the inputs its arguments are read into, whose types give the input schema
//! an agent is shown.

use super::{MAX_LOOK_FRAMES, MAX_SHEET_FRAMES};
use crate::{McpCode, McpError, ToolAnnotations};
use kerf_lang::reference;
use kerf_model::Catalog;
use kerf_skills::Skill;
use schemars::generate::SchemaSettings;
use schemars::transform::RecursiveTransform;
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;

const GUIDE: &str = "You edit video in Kerf. The project is a `.kerf` text \
document, annotated with each clip's end and source out. Change it with edit \
scripts in the language below.

- `look` shows what is there: with no arguments the document; with `media` \
that media's index (where it is used, its scene cuts, pauses, loudest \
moments, sound onsets, and tempo, and what is said on request, word by word \
with `words`); with `sheet` a contact sheet of a range in one image, the \
cheap way to scan; with `at` frames — source frames of a media, or program \
frames of the sequence; with `transcript` and no media, what the sequence \
says. `sequence` picks one other than the first. \
With `file`, the `media add` line for a file in the project's folder, or for each file of a folder of it with a `script` of them all, and with no arguments `unplaced` lists those not yet in the document, and `toMake` the media whose steps, the lines under it that say how Kerf or a model makes it, have yet to be run, or read something that has changed since its take was made, which it names: make it with `make`. With `media`, a media its steps make lists its `takes`, each making kept, with the line (`use`) that in an edit has the media be that take; a `toMake` entry's `cannot` is what would refuse its making, to mend before `make`, and its `notes` what its model has said it does not take, and what `stands` finds floating, sunk, or going through another in the sequence a `render` step of it draws, on the first, middle, and last frame it draws; an edit that writes steps answers with `toMake` too; with no arguments, `cast` and `styles` list who and what the project's shots show and how they look, each by the name a `generate` line gives as `cast=` or `style=` and with the words and pictures a model is then handed, `brand` the project's colors, fonts, tone, and the words it says and avoids, which a title and a caption keep to, and `models` lists the models a `generate` step can name, each with what its owner says it takes: the picture it makes (`size`), its `rate`, the most `seconds` of one shot, the files it `reads` (of `guide`, `first`, `last`, `ref`), the other names a step can set with what they are when it does not (`more`), and, for one that follows a guide, `guide`, the `render` step that draws a guide at its size and rate. Write that step and a sequence of the model's shape, no longer than its `seconds`. A model with a name alone has not said, and none listed means none is set up: both are the person's to set, in Settings ▸ Generation. \
With `cuts`, how each cut of the sequence's sound sits in the sound, by its media's own levels: in a pause, or through a word or a sound; what each join plays and took out, to read as a sentence, and the word or two it took with a restart (`tookMore`); and the pauses, repeated words and restarts the sequence still plays. Look after cutting speech, and trim with `snap=pause` what it finds. \
With `eye`, where the eye is either side of each cut of the sequence's picture, between two shots whose guides the document has: where the subject of the shot that ends is on the frame and where the next one's is, how far that jumps, how much larger or smaller it is, and whether it turns back, read from the guides before anything is made and from the takes once they are. Read it of a shot with another before or after it. \
With `cost` and `at`, who draws each frame, what sends it to the CPU, and what it costs. \
With `mesh`, what a mesh clip's glTF file holds, its animations and cameras, and a camera of it, or one a model found and wrote to a file, as a camera clip's keyframes. \
With `follows`, how a take's depth read back holds beside the guide a sequence draws, clip by clip; with `placed`, where the take has one clip's subject, which `follows` does not say; with `held`, both at once and one number for the take. \
With `signal`, the keys of a signal clip from the onsets of a sound on the timeline, so that what a model is told happens on its beats. \
With `shows` and `at`, what shows of each clip on those frames, read from the frame and not from the document: where each is, what hides it, and what has left the frame. Read it of a scene in depth before a making is asked for. \
With `stands`, what each clip in depth stands on, measured in the scene and not on a frame: which floats over the floor and by how much, which is sunk in it, which goes through another, and the line that sets it down. Place a thing roughly, read this, and run the line. \
With `graph`, a media its steps make as the graph the person sees of it in the editor: its steps and what each reads, wired, with what would refuse its making. With `takes`, each take of a shot held to its guide by where its subject is, to choose among them by. What they can see and set there you can: read the graph before you change a shot, and say what you changed by its nodes. \
With `proposal` it shows what a pending proposal would make: its document, \
what it would have made (`toMake`), its frames, and what it says.
- `edit` runs one script. It becomes a proposal for the user, returned with \
the document diff, check findings, `snapped` where a `snap=` moved an edge \
(how far, into which pause, between which words: read it, since a move may \
have left a word in or out) or a `said=` or `pause=` took something out \
(from where to where, and the pause it leaves), and a before/after preview: the frames \
either side of each cut it made or moved, the middle of a clip it changed \
in place (with its first and last frame when the clip moves), three \
frames through a transition it added or changed, and the frame that shows \
most of a title, shape, particles or solid it added or changed (revealed, \
at its full opacity and size, nearest its middle). A sheet holds eight frames: \
those of the generators first, then the earliest of the rest. Each sheet is an image \
after a line that says which it is and its frames in reading order, the \
one before the edit first. The project does not change until the user accepts it. Several \
proposals can wait side by side. Pass a pending proposal's id as `extend` to \
run more lines on top of it, or as `revise` to replace it: either way it \
becomes that proposal's next version (v2, v3) under a new id, and the user \
can still see the versions before it. `extend` takes the newest version's \
id, the one the last `edit` returned. A proposal goes stale when the project \
changes under it, as when another is accepted first, and cannot be accepted \
then. Pass its id as `forward`, alone, to bring it onto the project as it is: \
its own operations run again there, each time they name moved with the clips \
that were at it, and that is its next version, for the user to review again. \
It is refused, naming the clip, where a clip a line names, or one a time it \
names was on, has been cut, moved or removed since, or where the line would \
change other clips than it did; then `revise` it with a script written on \
the project as it is. What Kerf or a model makes is written in the script too, \
as the steps under its media: `media add` its line, naming the file it will be with the \
model's rate and size, then a `step <media> add` line for each step, in the order they run \
(the language below). There is no file to write first, and undo takes a step back as it does \
any edit. To change a shot that is there, set a name on a step (`step <media> set <n> seed=8`): \
a prompt, a seed, a knob, and what a step reads, since wiring a node in or letting one go is a \
name on its reader. `restoreTake` writes back the steps a take was made from. \
A shot longer than its model makes at once (`models` says its `seconds`) is written in parts: \
`render <sequence> {d2.mp4} from=<seconds> seconds=<seconds>` draws a part of the guide, \
`generate … guide={d2.mp4} first={a.mp4} from=<seconds> out={b.mp4}` goes on from the last frame \
of the part before, and `join {a.mp4} {b.mp4} {out} lap=1` sets them end to end. \
What was made only to be looked at, a numbered picture or a trial, is taken out of the document \
once it has been read (`media remove <id>`); its takes stay on disk. A media that still names a \
recipe file (`recipe=`) has its steps in that file: `carry` writes them under the media, in the \
same edit as the lines that then change them. \
Where the person's own permissions accept an agent's edits, \
the answer says `accepted`: it is in the project, one revision undo takes \
back.
- `make` makes a media from its steps, and answers with what runs. When what \
is asked needs a model, write the steps and call it once the edit is accepted, \
without being told to: which model fits and what comes next are yours to judge. \
How much runs without asking is the person's to say: where they let an agent \
make on its own it runs (`making`), as a `job` to read, `wait` for, or `cancel`, \
and the media is the new take once it is made; else they are asked (`asked`), in \
the conversation, and make it. Then look at the take, and change one thing for the next.
- `render` renders the sequence, or a range of it, to a file; with \
  `proposal`, what that proposal would make. With `handoff` it writes the \
  sequence as a timeline file another editor opens instead, `otio` for DaVinci \
  Resolve and Premiere Pro or `fcpxml` for Final Cut Pro and DaVinci Resolve, \
  and answers with `notCarried`, what that file leaves out: say it to the person.

Reusable presets are portable .kerfpreset files. `look` with `presets: true` \
lists project presets: the shared `presets/` library first, then \
`.kerf/presets` and the project folder. With `preset: \"file.kerfpreset\"` it shows the pattern, \
text slots, and each media slot with the clips that use it and the imported \
media that fit it (`fits`, with the latest `from` frame as `latestFrame`), and \
the media the preset makes from steps (`made`), which take no binding. \
`edit` with `savePreset: \"file.kerfpreset\"` \
saves the accepted sequence (optional `sequence` and `name`), without changing \
the project. To reuse one, `edit` with `preset`, `bindings` mapping each slot \
to {media: \"imported-media-id\", from: \"0f\"}, and optional `texts` mapping \
template clip ids to new text. This replaces the chosen sequence (the first \
by default) as a normal proposal. Every slot must be bound; its minimum \
duration includes transition handles. A made media becomes a new media of the \
project, its steps reading the media bound to the slots they name, and the \
sequences its steps draw come with it; `ids` says what each became, and \
`make` makes it. Inspect the preset before binding it.

A script's `captions` line needs a transcript of the clip's media made \
earlier with `look`; the proposal's script shows the caption lines it became.

Check findings include what an editor would catch: black frames, picture that \
leaves the frame's edges black, flash frames, a frame or two of another shot at \
a clip's edge or in what a transition plays past it (from the media's index), \
a frame or two of a shot between what \
is over it and a cut (an overlay that ends just before the cut under it, or \
starts just after), sound cut inside a word (known once \
a transcript of that media has been made), titles and captions outside 90% \
title safe or over each other (with the overlap's rect and depth in pixels), \
and captions faster than 17 characters a second, up for less than 5/6 of a \
second, or less than 2 frames after the one before.

Work like an editor. Look at the footage before choosing ranges, and look at \
the proposal before saying it is done: its preview, its findings, and with \
`proposal` the frames around its cuts and, for speech, what it says. Base every time, word, and frame on something you \
saw or heard; never invent them. A refused script names the line and the \
clip that stopped it.";

/// Fixed instructions for an agent: the guide, the language reference, and
/// the editorial guidance of each skill.
pub fn system_prompt(skills: &[Skill]) -> String {
    kerf_skills::compose_system_prompt(
        &format!("{GUIDE}\n\n{}", reference(Catalog::builtin())),
        skills,
    )
}

/// What a time is given as, said of each argument that takes one.
const TIME: &str = "Timecode (`01:00:04:12`), frames (`96f`), or seconds (`4.5s`).";
/// What a position of the sequence takes besides.
const ANCHORS: &str = "A sequence position may also be an anchor, as in `edit`: `c3.end-1f`, `c3.start`, a marker's id, `end`, or `c3@<media timecode>`.";
const PROPOSAL: &str = "The id of the pending proposal to see: what it would make of the project.";
const SEQUENCE: &str = "A sequence id from the document; the first sequence when absent.";

pub(crate) fn metadata(name: &str) -> Option<(&'static str, Value, ToolAnnotations)> {
    Some(match name {
        "look" => (
            "See the project and its footage. No arguments: the annotated document and \
             proposal state. `media`: that media's index — where it is used, scene cuts, \
             pauses, loudest moments, sound onsets, tempo, and with `transcript` what \
             is said (`words` times each word), and with `facts` what the camera does in \
             each shot and how loud it is. `contrast`: titles and captions too faint to read. \
             `sheet`: a contact sheet of a range. `loudness`: how loud the sequence \
             plays, by R128. `cuts`: whether each cut of its sound is in a pause or \
             goes through a word or a sound, and the pauses and repeated words it \
             still plays. \
             `at`: frames, as source frames of `media` or program frames of the \
             sequence. `transcript` without `media`: what the sequence says. \
             `track`: where a point, a surface, an object, or the camera goes, as keyframes. \
             `proposal`: the same, as a pending proposal would make the project.",
            input_schema::<LookInput>(),
            ToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
            },
        ),
        "edit" => (
            "Run an edit script. It becomes a proposal for the user, returned with the \
             document diff, check findings, and a before/after preview of the frames \
             around each cut it changed. The project is unchanged until the user accepts it, \
             unless their own permissions accept an agent's edits (`status: accepted`). `extend` builds \
             on a pending proposal; `revise` replaces it; `forward`, alone, brings one the project \
             has changed under onto the project as it is. The steps that make a media are lines of the script (`step <media> add …`); `restoreTake` goes back to the steps a take was made from, and `carry` writes the steps of a media that still names a recipe file under it. `savePreset` saves a portable \
             .kerfpreset file without changing the project. `preset` with `bindings` \
             applies one as a proposal replacing the chosen sequence.",
            input_schema::<EditInput>(),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
            },
        ),
        "render" => (
            "Render the sequence, or the range `from`–`to`, to a file — of the project, or \
             of what a pending `proposal` would make. Returns a job that renders while this \
             session lasts; pass `job` to read its progress and output, with `cancel` to stop it. \
             `handoff` writes the sequence as a timeline file for another editor instead, \
             an OpenTimelineIO `.otio` or a Final Cut Pro `.fcpxml`, at once and with no \
             Python, and lists what the file does not carry.",
            input_schema::<RenderInput>(),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
            },
        ),
        _ => return None,
    })
}

/// The input schema of a tool that reads its arguments into `T`, as an agent is shown it:
/// `T`'s schema, with each type in it written out in place, less what [`as_written`] leaves
/// out.
pub(crate) fn input_schema<T: JsonSchema>() -> Value {
    let mut schema = SchemaSettings::draft2020_12()
        .with(|settings| {
            settings.meta_schema = None;
            settings.inline_subschemas = true;
        })
        .with_transform(RecursiveTransform(as_written))
        .into_generator()
        .into_root_schema_for::<T>();
    // The type's name, which tells an agent nothing.
    schema.remove("title");
    schema.to_value()
}

/// Leave out of a schema what schemars adds and the input schemas do not say, which agents
/// have been shown and work by: a number's `format`, a field's `default` (where it matters,
/// the field's description says it), and the `null` schemars adds to an optional field's
/// type. The schemas say what a field is when it is given.
fn as_written(schema: &mut Schema) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    object.remove("format");
    object.remove("default");
    if let Some(Value::Array(types)) = object.get_mut("type") {
        types.retain(|name| name != "null");
        if let [only] = types.as_slice() {
            let only = only.clone();
            object.insert("type".to_owned(), only);
        }
    }
}

/// What `cuts` is said to take: `true`, `false`, or an object. What the object says is in the
/// field's description; the schema does not list it.
fn true_or_object(_: &mut SchemaGenerator) -> Schema {
    json_schema!({"type": ["boolean", "object"]})
}

// On a type that derives `JsonSchema`, a doc comment is what an agent is shown: on a field,
// its description in the input schema, and on the type, the description of a field of that
// type that has none of its own. What is said of these for the code alone is a plain comment.

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct LookInput {
    /// Inspect a .kerfpreset file: pattern, text slots, media slots with the imported media that fit each, and the media the preset makes from steps (`made`, which take no binding). Use alone.
    pub(super) preset: Option<String>,
    /// List .kerfpreset files in the project's shared presets/ library, then .kerf/presets and the project root. Use alone with true.
    #[serde(default)]
    pub(super) presets: bool,
    /// An effect, transition, generator, or clip property: every parameter of it, where the reference keeps a few of a generator that has many. Raster effects are listed by id only. Use alone.
    pub(super) effect: Option<String>,
    /// Words for the look or moment you want, such as "glow neon", "zoom hit", or "retro tv": the effects, transitions, and generators whose id, category, guidance, or parameters match, best first, each with when to use it, and in `properties` the clip properties that do. Use alone, or with `area`.
    pub(super) effects: Option<String>,
    /// A font family, or `sans-serif`, `serif`, or `monospace`, or a face's own name ("Inter Black"): its faces, by the `style` and `weight` a `title` picks one with, and for a variable font each axis with its `tag`, `min`, `default`, and `max`, on the font's own scale, which is what a `title`'s `wght`, `wdth`, `slnt`, `opsz`, and `ital` are set on. For a name no font has, the families that contain it; for "", all of them. Use alone.
    pub(super) font: Option<String>,
    /// With `effects`: only what belongs to this part of the editor.
    #[schemars(extend("enum" = kerf_model::Area::ALL.map(kerf_model::Area::name)))]
    pub(super) area: Option<String>,
    /// A media id from the document.
    pub(super) media: Option<String>,
    #[serde(default)]
    #[schemars(
        length(min = 1, max = MAX_LOOK_FRAMES),
        description = format!("Frames to see: media positions with `media`, otherwise sequence positions. {TIME} {ANCHORS}")
    )]
    pub(super) at: Vec<String>,
    /// What is said, by local transcription, up to 15 minutes per call: of `media`, or else of the sequence as it plays. `unheard`, when it is there, lists stretches that sound like speech and have no words: the transcriber may have missed something, so listen there.
    #[serde(default)]
    pub(super) transcript: bool,
    /// With `transcript`: time every word, for cutting on speech. Choose a short range; it is long.
    #[serde(default)]
    pub(super) words: bool,
    /// With `at`: instead of frames, what scopes read at those program positions, as numbers, in each of `readings` under `scopes`: `luma`, `red`, `green`, and `blue` (`min`, `mean`, `max`, in percent); `clipped` (the share of samples at `black` and at `white`, each as [red, green, blue]); `zones` (the share of samples in `shadows`, under 25% luma, `midtones`, and `highlights`, over 75%); `histogram` (the share in each of 16 even bands of luma, darkest first); `chroma` (`castHueDeg` and `castMagnitude`, the mean color as an angle on the vectorscope and how far out in percent, a cast when it is far from 0; `saturation`, the mean of how far out each sample is; `onSkinLine`, the share of colored samples near the skin tone line); `peaks` (the densest colors, each `hueDeg`, `magnitude`, `share`). Angles are a vectorscope's, from +Cb: magenta 50, red 103, skin 123, yellow 175, green 230, cyan 283, blue 355. Grade by these: compare shots with each other and the result with where it should land; with `proposal`, each reading also has `before`, what the project reads there, so one call compares the two. Read from 8-bit frames, or in an HDR sequence from its PQ or HLG signal in BT.2020, with `hdr` saying what that comes to: `peakNits`, `meanNits`, `overWhite` and `outsideBt709` as shares of the samples, and `referenceWhite`, where SDR white falls on the signal in percent.
    #[serde(default)]
    pub(super) scopes: bool,
    /// With `media`: its fact sheet beside its index, the first time slowly: what the camera does in each shot (`camera`, such as `pan right` or `handheld`; `pan`, `tilt`, and `zoom` in shares of the frame a second, `roll` in degrees a second, and `agreeing`, how much of the picture follows that), how loud it is (`loudness`, in LUFS and dBTP), and in `notMeasured` what it does not say and why: faces, the subject, and on-screen text.
    #[serde(default)]
    pub(super) facts: bool,
    /// Read each title and caption of the sequence over `from`–`to` (all of it by default) against what is behind it, from rendered frames, and return `contrast`, findings for those too faint to read or that do not show, with `read` and `of`, how many of them it read. Slower than a check, so it is not part of one; the editor's full QC has it as a row.
    #[serde(default)]
    pub(super) contrast: bool,
    /// How loud the sequence is as it plays, by R128, measured from its whole mix: `integrated` in LUFS beside the `target`, `range` in LU, `truePeak` in dBTP beside its `limit`, and the loudest three seconds (`shortTermMax`) and 400 ms (`momentaryMax`) with where each ends. A silent sequence has `integrated: null`. Measured once for a revision; a media's own loudness is in its `facts`. Use with `sequence` or `proposal` only.
    #[serde(default)]
    pub(super) loudness: bool,
    /// How the cuts of the sequence's sound sit in the sound, measured from each media's own levels a hundredth of a second at a time and from its transcripts, so an edit is checked without listening: `summary` counts every cut as `quiet` (in a pause), `low` (through a breath or a decay), or `sound` (through sound at the level of speech), with `inWords` (inside a word a transcript heard), `tight` (quiet, but within 0.05 s of the sound the clip plays), `faded` (under a transition, which fades what the cut goes through, so the cut is not judged), `fadesInSound` (of those, a cut in quiet whose transition plays on past it into sound at the level of speech, what the cut left out heard fading: listed with `fade`, its `seconds` and `level`), `notMeasured`, and `air`, the `least` and `median` quiet a cut leaves beside what its clip plays, in seconds. `cuts` lists each that is not clean, 40 at most with `cutsNotListed` for the rest: its `clip` and which `end`, `at` on the sequence and `source` on the `media`, the level `before` and `after` it in dBFS, and the `word`. `media` gives the level each media speaks at and pauses under. `joins` are where the sound goes from one stretch of footage to another, in the order they play: what each `plays`, the words either side with ‖ for the cut, and what it `took` out, the words between where the one clip stops and the other starts with `tookSeconds`, or `replays` where the second starts back before the first stopped: read `plays` as a sentence, since a cut that takes a word the sentence needs measures clean. A join has `tookMore` where it took a restart out, a start taken again beside it, and a word or two with it that nothing within six words either side says again: `out` the end of the phrase kept before the cut, two words at most, and `in` the start of the take it goes on with, three at most, a hesitation such as "um" not counted; `summary` counts those joins as `tookMore`. It is found by the words alone and says where to read, since the sentence may do without them, as it does without a correction: read `plays`, and trim the clip to play a word the sentence needs. With a transcript of what plays, `pauses` are the waits still played between words, longest first, of 0.75 s or more unless `cuts` is {"pauses": <seconds>} (0.3 at least: what is dead air is the edit's to say); `repeats` the words and phrases said twice in a row; and `restarts` the phrases said and taken `again` a moment later, with what is said `between`: a start taken again, a take left beside the one that replaced it, or something meant, which the sentence tells. Each has `cutTo` where a cut sits in it. `from` and `to` take a part of the sequence, for one too long to list. A trim or split with `snap=pause` moves a cut into the pause beside it, past no word, and with a transcript into the gap two words said in one breath part for; `extract <clip> pause="…" leave=<dur>` brings a pause listed here down to a length, by the words it comes after, and `extract <clip> said="…"` takes a repeated word or phrase out. Use with `sequence` or `proposal`.
    #[serde(default)]
    #[schemars(schema_with = "true_or_object")]
    pub(super) cuts: Option<Toggle<CutsWith>>,
    /// Where the eye is either side of each cut of the sequence's picture, a cut being where one clip of a video track ends on the frame the next starts: `true`, or an object with `subjects`. Under `eye`, `summary` counts the `cuts`, those `read`, of them `held` (the subject jumps less than 0.1 of the frame's height, about what an eye sees sharply without moving) and `jumps`, `reversed`, `takes` (read in the takes too), and `notMeasured`; `cuts` lists each as it plays: `at`, the frame the second clip starts on, and the clips `from` and `to`. A cut is read where both clips play a shot whose steps draw its guide (`render <sequence> …`). `on` is what the eye is on in each: the clip of the guide's sequence its `generate` step names with `moves=`, the first of them, or the one `subjects` gives for that media, as {"subjects": {"<media>": "<clip of its guide>"}}. `guide` is read from the two guides' ids passes, as `shows` reads a clip, with nothing made: `out` and `in`, the subject's box on the last frame before the cut and on the first after it, [left, top, right, bottom] in shares of the program's frame, the clip being where the program has it, trimmed, retimed, scaled, turned, moved, cropped, or parented, and a guide of another rate, size, or shape than its take read at the moment and the place the take has of it; `jump`, how far the middle of the second is from the middle of the first, in frame heights; `across` and `down`, the same with their direction, above 0 to the right, and lower; `size`, how large the second is beside the first, above 1 larger; `going`, which way each goes across the frame over a quarter of a second before the cut and after it, `left`, `right`, or `still` (under 0.1 of a frame height a second), absent where a clip is a frame long or its guide does not show the subject that far off; and `reversed: true` where one goes left and the other right. `take` is the same boxes and numbers where both shots have a file: the subject found in the take's own picture on the frame at the cut, where the guide has it with room around it, as `placed` finds it, and in the guide's own picture the same way, the take's box being the guide's moved by how far apart those are. Where `guide` jumps and that is not meant, change a guide, which costs no making; where `guide` holds and `take` does not, the model did not keep its guide: make it again, or choose another take. `notMeasured` says why a cut was not read, in place of both readings: footage with no guide, a shot whose subject nothing names, a clip in depth, a mask, an effect that may move its picture (a grade, a blur, and grain are read through), a crop that cuts the subject, a subject its guide does not show there; under `take`, why the takes were not. `transition` names the transition across a cut, which is not read: the boxes are the two clips' at the cut. `under` lists the clips of tracks above that draw there and may hide it. A cut costs four small frames of its guides, about a tenth of a second, and half a second where the takes are read, so it is asked for and no part of a check; `from` and `to` choose the cuts of a range, and at most 64 are read (`unread`, how many more there are). What is said is what is so, not that it is wrong, and fails no check: a hard cut jumps on purpose. Use with `sequence`, `proposal`, `from`, or `to` only.
    pub(super) eye: Option<Toggle<EyeWith>>,
    /// Follow a point of the program's picture over `from`–`to`: `x` and `y` where it is at `from`, 0 to 1 across the frame, on something with detail, such as a corner rather than a flat area. Returns where it went, and with `clip` a line setting that clip's x and y to follow it: run it with `edit`, and parent what should stick to it to that clip, a `gen=null@1` for instance. `lostAt` says where the point was lost, if it was. Or give `corners` instead of `x` and `y`, four [x, y] at `from` going upper left, upper right, lower right, lower left, around a flat surface with detail on it, such as a screen or a sign, to follow how it moves and turns: it returns those corners over time, `outline`, the keyframes a moving mask takes (`mask c add k1 points=...`), and with `clip` a line that puts that clip in depth on the surface, as the default camera sees it (`unplaced` says why not, with a camera or a parent). Or to cut out an object that changes shape, a person for instance, give `inside`, [x, y] points on it at `from`, with `outside`, points off it, and `box`, [left, top, right, bottom] around it, as needed: it returns its outline over time, as few corners as shape it and the same ones in every keyframe: `outline` as above, or with `clip` a mask line for that clip instead, `script`, which holds it (`mask c add object points=...`). A third number on each corner says where the edge measures soft, such as out of focus or moving fast. It goes by colour and edges, with nothing to install; where a model that finds a thing by its name is set up (`models`, one whose `about` says it makes a matte), a `generate model=<it> guide=<footage> prompt=<the thing>` step makes the matte as a file instead, and holds a thing among others like it. With `clip` and no `from` and `to` it is followed for as long as that clip runs, and a clip that follows a point is not drawn while the point is followed: this is how a clip a `generate` step names with `moves=` is led by footage, and how the clip a `render` step names with `matte=` is cut to what it shows, as a person does on the picture in a shot's graph. Or give `camera: true` to solve where the camera went over the range, one shot of a camera that moves through a still scene, to pin clips in depth to moving footage: it returns a `script` of `gen=camera@1` keyframes (on `clip`, or on a new camera without one), `fit` (`error` and `worst`, how far what was followed lands from where the solve puts it in the sequence's pixels; under a pixel holds, and past two a clip slides; `agreeing`, the share of it that fits), `fov`, and `points`, places in the world to set a clip's `x`, `y`, and `z` to with `three_d=true`, each with where it shows (`at`, on `frame`) and the frame it was last seen on (`lastFrame`), so that it stays on that spot. Hide what is not in the footage first. `solved: false` with a `reason` says when a shot cannot be solved, such as a camera that only turns; `lostAt` says where a cut or lost frame ended it. `lensKnown: false` means the camera never turned, so any lens fits and `fov` is the default: give `fov`, down the frame in degrees, if you know it. A zoom reads as a move along the camera's way, with a higher `error`. Use with `from`, `to`, and optionally `sequence` or `proposal`.
    pub(super) track: Option<TrackInput>,
    /// Media ids of recordings of the same thing to line up by their sound: the cameras of a multicam, or a camera and the recorder beside it. `recordings` says of each how far into the first it begins, `offset` in seconds, with `confidence`, how far the match of the whole of it stands above the next best: low means they may have nothing in common, or that no one offset fits the whole. Each is then placed again a minute at a time (less for a short one), and `course` says what those stretches come to: `steady`, one offset fits them all, to `within` seconds; `drift`, the offset changes evenly, one clock running faster than the other, with `drift`: `msPerMinute`, over how many `minutes`, and the offset `from` the first stretch `to` the last; `jump`, the offset steps where one of the two stopped and went on or lost sound, with `jumps`: each is between `after` and `before`, seconds of that recording, as near as the sound around it says, and the offset goes `from` one `to` another; `unknown`, fewer than two stretches could be placed, so nothing says. `read` of `of` stretches were placed; one that is quiet, or that the first has nothing like, is given no offset and is listed in `unknown`, each [from, to] in seconds of the recording: it is not guessed at. `script` builds a sequence with a camera on each track, each on the frame nearest its offset (`at`), and writes how each lines up on its media's line (`media sync mb with=ma@1.5s`, in the document `sync=ma@1.5s`: it begins 1.5 s into `ma`; `with=""` lets go), which a check then holds clips to: two of them that play the same moment at once, more than a frame apart, are `sync.slipped`, as a sound that left the picture of its own file it is linked to is. Change the line where the sound is not what should line up, a camera far from what it hears. A recording is in the script where one offset is within half a frame of every stretch read; one that drifts further or jumps is left out of it, its `notPlaced` saying what does not fit and what to do, and with fewer than two to place there is no `script`. Then a clip of that sequence (`seq=multicam`) with `angle=V2` shows one camera, and splitting it cuts between them. How well drift and jumps are told has been measured on recordings made for the test and on none recorded in a room: listen to what it says before cutting by it.
    #[serde(default)]
    pub(super) sync: Vec<String>,
    /// A contact sheet of this many frames spread evenly over `from`–`to` (all of it by default) as one image, four to a row, each with its time in its corner (`frames` lists them in order): of `media`, or else of the sequence. Scan with a sheet, then look `at` the moments that matter.
    #[schemars(range(min = 2, max = MAX_SHEET_FRAMES))]
    pub(super) sheet: Option<usize>,
    #[schemars(description = format!("The start of the range for `transcript`, `sheet`, `contrast`, `eye`, or a media's onsets. Positions of `media`, or else of the sequence. {TIME} {ANCHORS}"))]
    pub(super) from: Option<String>,
    /// The end of that range.
    pub(super) to: Option<String>,
    /// Frame width in pixels (default 960; 320 for each frame of a sheet).
    #[schemars(range(min = 64, max = 1920))]
    pub(super) width: Option<u32>,
    #[schemars(description = SEQUENCE)]
    pub(super) sequence: Option<String>,
    #[schemars(description = PROPOSAL)]
    pub(super) proposal: Option<String>,
    /// A file in the project's folder: what it holds, and in `add` the `media add` line that puts it in the document. Or a folder of it, such as `media` (`.` for the project's own): each footage file there the same way, in `files`, and in `script` the lines for those not in the document, to run as one edit. With no arguments `look` gives in `unplaced` the footage in the project's folder and in `media` not yet in the document. Use alone.
    pub(super) file: Option<String>,
    /// With `at`, program positions: who draws each frame and what it costs. `draws` is `gpu` or `cpu`, `because` says what sends it to the CPU, one line a cause, `ms` is a frame at full size and `msHalf` at half, against `frameMs`, a frame's share of the time. Playing, the editor draws smaller until frames keep up, and a frame the CPU draws takes every core.
    #[serde(default)]
    pub(super) cost: bool,
    #[serde(default)]
    #[schemars(description = format!("With `at`, program positions: what shows of each clip that draws there, whichever track it is on, in `frames`, each with `clips` from the lowest track up. `box` is where the clip would be with nothing before it, [left, top, right, bottom] in shares of the frame, absent when none of it is in the frame; `area` the share of the frame that is; `shows` the share of the frame it has as the frame is drawn; `hidden` the share of it other clips hide, and `by` which, each with its share, the most first; `edges` the edges of the frame it reaches, where it may go on past them; `opacity` where it is not wholly opaque, and such a clip has that much less of each pixel it is on, in `area` and in `shows`. A pixel clips share, along an edge or all over a clip of thin strokes, is each one's by its part of it. `empty` is the share of the frame no clip has. A nested sequence is one clip. A floor in `by` of what stands on it, a subject with no `box`, and a subject at an edge are what to mend. `unread` names clips left out, the lowest tracks' first: those past {most} on one frame, and one whose color in the ids pass a higher clip has too, which only a program of more than {colors} clips has. They are out of the frame that is read, and the others are read as if those were not there. Read on a small frame, to about a hundredth of it. Not with `media`.", most = kerf_engine::analysis::shows::MOST_CLIPS, colors = kerf_compose::ID_COLORS))]
    pub(super) shows: bool,
    /// What each clip in depth of the sequence stands on, measured in the scene with nothing drawn, in frame heights, down being the way `y` grows: under `stands`, `frames`, how many were read, every frame of the sequence unless `at` gives positions (`every` where one in that many was, of a long one); `ground`, the largest clip with nothing under it, which has no entry; and `clips`, from the lowest track up. A clip's feet are its lowest corners, and a line straight down through each meets the other clips' surfaces: a flat clip's rectangle, the box of one with a `depth`, a mesh's triangles as its file's animation has them on that frame. The underside of what stands on a clip, a ball set a little way into the one under it, is no floor through the lower clip: the upper one is sunk in it, and a box, a ball, a cylinder, or a solid whose corners are over nothing stands by what is under its middle, a roof on walls it is wider than. `on` is the clip under its feet, and of two it is as near to, an axle across two wheels, the one of the lower track. A clip that rests on it on every frame read has one entry, and no more is said of it than `slope`, how far from level what it stands on leans, in degrees, and `gap`, the room that leaves under its far feet where its own foot is level: it rests on the feet that are down first. Else it has an entry for each run of `frames`, [first, last] from 0, on which it does not rest: `floats`, how far it would come down to rest; `sunk`, how far its deepest foot is under a surface that goes through it, with `share`, how many of its feet are; or `air: true`, nothing under it within its own height, with `over` and `up` where something is further down. `worst` is the frame it is furthest off on. `script` is the line that sets it down, `set <clip> y=…`, one number or the fewest keys that keep it down on every frame read, in its own `y` under whatever parents it has, said once for a clip, with the first of its entries it mends; `cannot` says why where no line does. A clip that is in the air on some frame, a ball thrown, is brought up where it is sunk and left where it floats. Run the line with `edit` and read again: what stood on a clip that was moved has not moved with it. `through` lists the clips one of its edges goes through, other than what it stands on: `deep`, how far past that surface it reaches on the side it has less of, `share`, how much of it is on that side, and the `frames`. `notMeasured` names what of this scene was not read, which is not thereby right: a mesh into another mesh, the outline of a round shape within the box around it, a clip whose outline is its picture's own (a title, a nested sequence, a masked clip, media with alpha), and clips out of depth. A camera, a light, a null, and a signal are not things. A clip wholly under a surface other than the ground is taken to be under a roof. What is said is what is so, not that it is wrong, and fails no check. Not with `media`; with `shows`, both are answered.
    #[serde(default)]
    pub(super) stands: bool,
    /// A signal's keys from a sound: `of` is a clip that plays a media with sound, music or what was recorded, and `clip` a `gen=signal@1` clip of the same sequence. It returns `script`, a `set <clip> value={…}` line to run with `edit`, which has the signal at 1 for `hold` frames (3 unless given) from each sound onset of `of` that falls in the signal clip and 0 between, onsets nearer than that running together; `onsets`, how many it placed, and `outside`, how many of the sound's fall outside the clip. Hold for as long as two of the model's frames last. Used alone.
    pub(super) signal: Option<SignalInput>,
    /// What the glTF file of a `gen=mesh@1` clip holds: `triangles`; `size`, how wide, tall, and long it is at rest for a `height` of 1; `animations`, each with its `name`, `number`, and `seconds`, and `playing`, the one the clip's `animation=` names; `cameras`, each with its `name`, `number`, and `fov`. With `camera`, a name or a number of those, it returns `script`: that camera as a `gen=camera@1` clip over the mesh's clip (or `set` on the camera clip `as` names), where the file has it on each frame, the mesh being where its clip puts it and its animation as far in as the clip plays it. Through it the mesh is seen as the file's camera sees it, and what is added to the sequence stands in the same space. `moves` says whether it has keyframes. With `beside`, another mesh clip of the sequence whose file is in the same space, as sets a model rebuilt of several frames of one reading are, it returns `script`: the `height`, `x`, `y`, and `z` that put this clip's file where that one's is, so that the two are one set; neither clip turned or keyed. Used without `camera` or `from`. Ask again after moving or retiming the mesh's clip. For a camera the file does not hold, `from` in place of `camera`: a file in the project's folder that says where one was, as a model wrote it. A model that finds a shot's camera writes it on each frame, as `extrinsics` and `intrinsics`; a file that is one number is a lens alone, degrees down the picture, for the camera a set was seen by, which is at the set's own origin. `frame` is the frame of `from` the mesh was rebuilt from, 0 unless given, whose camera is put at the mesh's origin; `rate` how many frames of `from` a second has, the sequence's unless given; `scale` how much of the mesh's file one of `from`'s units is, 1 unless given, a path and a set from two models not being the same size; or `fits`, the media `from` is the camera of, finds it with no model: the set is drawn through the path at a run of sizes, on a few frames far from `frame`, and held to the footage's own frames, and `fit` says the `scale` found, the `error` there of 255 a pixel, and `sure`, how many times worse half or twice that size is, near 1 where the camera does not travel far enough to say. With no `lens` given it first finds the set's own, the one through which the set seen from `frame` is that frame of the footage again (`fit.lens`: `fov`, `error`, `sure`), and the camera gets it where it is sure. Where that lens is not the path's, the path's travel along the way the set was seen is made longer by as much as the set is deeper for it (`fit.deep`). `lens` gives the camera a lens other than `from`'s, degrees down the picture: the one a set's own model found, where the two differ. It returns `from`: its `frames`, how many of the clip's were `placed`, the least and most `fov` it has, of which the camera gets the middle one, and `anchor`, the frame a path counts its places from. A set made of the same reading as the path is in that frame's space whichever frame it shows, so `frame` is the `anchor` for it; a `note` says so where they differ. Used alone.
    pub(super) mesh: Option<MeshInput>,
    /// A media whose picture is a take's depth, as a model read it back (`generate model=depth guide={take.mp4}`): how it holds beside the guide `sequence` draws, the first unless given, the media's frames being the sequence's from its start. Each pixel has a place in its frame's order of near and far, from 0 to 1, in the guide's depth pass and in the take. `clips` is each clip the guide has in depth, by the pixels the ids pass gives it: `off`, how far apart those places are over its pixels, as a mean over the `frames` it shows on; `farther`, the same with its direction, above 0 where the take has those pixels farther, as it does where the subject is not there and what was behind it is; `worst`, the frame it is most off on, from 0; `shows`, its share of the frame. `off` at the top is the same over every pixel, and `order` the rank correlation of the whole frame, which a take with its subject elsewhere still has near 1 where most of the frame is ground. Read a subject's `off` beside the frame's and the floor's. Used alone, or with `sequence`.
    pub(super) follows: Option<String>,
    /// Where a take has one clip's subject beside where its guide has it, read from the take's own picture with no model: `take`, the media whose picture is the take, and `clip`, a clip the guide `sequence` draws, the first unless given. The guide's ids pass gives the box around the clip on each frame, as `shows` does; in the take the subject is found on the frame the guide shows most of it on (`from`), in that box with room around it, and followed to both ends. `off` is how far the middles of the two boxes are apart, in frame heights, as a mean over the `frames` it was followed on, of the `shown` the guide has it on; `across` and `down` the same with their direction, above 0 where the take has it to the right, or lower; `size`, how large the take drew it beside the guide, above 1 larger; `worst`, the frame it is furthest off on; `overlap`, how much of the two boxes is in both, of what is in either, as a mean over every frame the guide shows it on with a frame it was not followed on as none, which is the one number to choose between takes by: 1 where the take has the subject where and as large as asked on every frame, 0 where it has none; `read`, the first and last frame it was followed on. Read it beside `follows`, which says how a take's depth holds and not where the subject is. Used alone, or with `sequence`.
    pub(super) placed: Option<PlacedInput>,
    /// A take held to its guide in one reading, to choose between takes by: `take`, the media whose picture is the take, `depth`, the media that is its depth as a model read it back, and `clips`, the guide's clips that are its subjects. It is `follows` of the depth and, for each subject, `placed` of the take: `clips`, each with its `depth` (`off`, `farther`), its `place` (`overlap`, `off`, `size`, `frames` of `shown`), and `held`, the overlap of where it is with where it was asked for, less by how far its depth is off, from 0 to 1; and at the top `held`, the mean of them, with `order` and `off` of the whole frame. A subject the take has nothing of is held 0. Used alone, or with `sequence`.
    pub(super) held: Option<HeldInput>,
    /// A media its steps make, as the graph the editor shows of it: `text`, its steps, a line each; `nodes`, each step (`kind` `guide` or `model`, with its `line`, the step's number, and what it holds: a model's `prompt`, `seed`, knobs) and each thing of the project a step reads (a sequence, a file, a signal, the way a clip goes, one of the cast), ending in its `takes`; `edges`, each reading, `from` a node `to` the step that reads it under the name `port`, the `nth` word of that name on the line; `cannot`, what would refuse its making; `notes`; and `takes`, each making with the line (`use`) that has the media be it. `change` says how each is changed. Used alone, or with `proposal`.
    pub(super) graph: Option<String>,
    /// A media its steps make: each of its `takes` held to its guide by where its subjects are, the clips its `generate` step names with `moves=`, with no model run. For each take, `overlap`, from 0 to 1, how much of where it has the subject and where the guide asked for it is the same over every frame the guide shows it on, and under `clips` the same for each subject with `off`, how far its middle is from where it was asked for in frame heights, `size`, how large beside what was asked, and `frames` of `shown`, how many of the frames the guide shows it on it was followed on: read `overlap` beside that, since a subject followed on few frames says little. `best` is the take that holds best, of more than one. The person sees the same numbers beside each take in the editor's graph. Used alone, or with `proposal`.
    pub(super) takes: Option<String>,
}

/// What `cuts` or `eye` is given: `true`, or an object that says more of what is asked
/// (`With`). It takes whatever it is given, and keeps what is neither as `Wrong`, to be
/// refused where `look` reads it: a call is refused for the arguments `look` reads first
/// before it is refused for this one.
#[derive(Debug)]
pub(super) enum Toggle<T> {
    Off,
    On,
    With(T),
    Wrong,
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Toggle<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Value::deserialize(deserializer)? {
            Value::Bool(false) => Self::Off,
            Value::Bool(true) => Self::On,
            with @ Value::Object(_) => serde_json::from_value(with).map_or(Self::Wrong, Self::With),
            _ => Self::Wrong,
        })
    }
}

impl<T: JsonSchema> JsonSchema for Toggle<T> {
    fn schema_name() -> Cow<'static, str> {
        format!("Toggle_{}", T::schema_name()).into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = T::json_schema(generator);
        schema.insert("type".to_owned(), json!(["boolean", "object"]));
        schema
    }
}

// What `cuts` says as an object: from what length a pause is listed, in seconds.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CutsWith {
    #[serde(default, deserialize_with = "given")]
    pub(super) pauses: Option<f64>,
}

// What `eye` says as an object: the clip of its guide the eye is on in a shot, by the
// shot's media.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct EyeWith {
    #[serde(default, deserialize_with = "given")]
    pub(super) subjects: Option<BTreeMap<String, String>>,
}

/// A field that is a `T` where it is given: `null` is not one, where an `Option` read as it
/// comes would take it as the field not given.
fn given<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

// A take of a media, to go back to the steps it was made from.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RestoreInput {
    pub(super) media: String,
    #[schemars(range(min = 1))]
    pub(super) take: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = fields_unsaid)]
pub(super) struct HeldInput {
    // The media whose picture is the take.
    pub(super) take: String,
    // The media whose picture is the take's depth, as a model read it back.
    pub(super) depth: String,
    // The guide's clips that are its subjects, which the take is held to by their place.
    pub(super) clips: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = fields_unsaid)]
pub(super) struct PlacedInput {
    // The media whose picture is the take.
    pub(super) take: String,
    // The clip of the guide whose subject is looked for.
    pub(super) clip: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = fields_unsaid)]
pub(super) struct MeshInput {
    pub(super) clip: String,
    // A camera of the file, by its name or its number.
    pub(super) camera: Option<CameraName>,
    // A file that says where a camera the mesh's file does not hold was.
    pub(super) from: Option<String>,
    // Which frame of `from` the mesh was rebuilt from.
    pub(super) frame: Option<usize>,
    // How many frames of `from` a second has.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub(super) rate: Option<f64>,
    // How much of the mesh's file one of `from`'s units is.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub(super) scale: Option<f64>,
    // Another mesh clip whose file is in the same space, to be put in it with.
    pub(super) beside: Option<String>,
    // The media `from` is the camera of, to find `scale` from.
    pub(super) fits: Option<String>,
    // The lens the camera gets, degrees down the picture, in place of the one `from` has.
    #[schemars(extend("exclusiveMinimum" = 0, "exclusiveMaximum" = 180))]
    pub(super) lens: Option<f64>,
    // The camera clip its keys are written on.
    #[serde(rename = "as")]
    pub(super) onto: Option<String>,
}

/// A camera of a mesh's file, by its name or its number, as text: a number is read as the
/// text it is written as, and so is anything else given.
#[derive(Debug)]
pub(super) struct CameraName(pub(super) String);

impl<'de> Deserialize<'de> for CameraName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(match Value::deserialize(deserializer)? {
            Value::String(name) => name,
            other => other.to_string(),
        }))
    }
}

impl JsonSchema for CameraName {
    fn schema_name() -> Cow<'static, str> {
        "CameraName".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": ["string", "integer"]})
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = fields_unsaid)]
pub(super) struct SignalInput {
    pub(super) of: String,
    pub(super) clip: String,
    #[schemars(range(min = 1))]
    pub(super) hold: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = fields_unsaid)]
pub(super) struct TrackInput {
    pub(super) x: Option<f64>,
    pub(super) y: Option<f64>,
    // The schemas of these four say lists of numbers; the reading takes four pairs
    // (`corners`), pairs (`inside`, `outside`), and four (`box`).
    #[schemars(with = "Option<Vec<Vec<f64>>>")]
    pub(super) corners: Option<[[f64; 2]; 4]>,
    #[serde(default)]
    #[schemars(with = "Vec<Vec<f64>>")]
    pub(super) inside: Vec<[f64; 2]>,
    #[serde(default)]
    #[schemars(with = "Vec<Vec<f64>>")]
    pub(super) outside: Vec<[f64; 2]>,
    #[serde(rename = "box")]
    #[schemars(with = "Option<Vec<f64>>")]
    pub(super) bbox: Option<[f64; 4]>,
    #[serde(default)]
    pub(super) camera: bool,
    pub(super) fov: Option<f64>,
    pub(super) clip: Option<String>,
}

/// What the schemas of `look`'s `track`, `mesh`, `held`, `placed` and `signal` say
/// of fields they do not have: nothing, though each of them refuses one.
fn fields_unsaid(schema: &mut Schema) {
    schema.remove("additionalProperties");
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("anyOf" = [{"required": ["script"]}, {"required": ["restoreTake"]}, {"required": ["carry"]}, {"required": ["cast"]}, {"required": ["preset"]}, {"required": ["savePreset"]}, {"required": ["forward"]}]))]
pub(super) struct EditInput {
    /// One operation per line, applied in order and atomically.
    #[schemars(length(min = 1))]
    pub(super) script: Option<String>,
    /// Save the accepted sequence to a new .kerfpreset file inside the project. Existing files are never overwritten.
    #[serde(rename = "savePreset")]
    pub(super) save_preset: Option<String>,
    /// Name for savePreset.
    pub(super) name: Option<String>,
    /// A .kerfpreset file to apply. Inspect it with look first. Its clips, markers and transitions are given ids of the project's own, not the ones the preset has: the answer's `ids` says which each became (`{"sa": "p9"}`), and later edits name them by the new ones.
    pub(super) preset: Option<String>,
    /// Sequence to save or replace with a preset; first by default.
    pub(super) sequence: Option<String>,
    #[serde(default)]
    pub(super) bindings: BTreeMap<String, crate::presets::BindingInput>,
    /// Replacement text for a caption or title of the preset, by the clip id the preset has for it (as `look` with `preset` lists them), not an id of the project.
    #[serde(default)]
    pub(super) texts: BTreeMap<String, String>,
    /// The id of a pending proposal to build on, its newest version's: the script runs on what it makes, the two become one proposal, and the preview shows what this script changed.
    pub(super) extend: Option<String>,
    /// The id of a pending proposal this one replaces; the script runs on the project.
    pub(super) revise: Option<String>,
    /// The id of a pending proposal the project has changed under (stale), its newest version's, passed alone. Its own operations run again on the project as it is, each time they name moved with the clips that were at it, and that is the proposal's next version, which always waits for the user's review. Refused, naming the clip, where a clip a line names, or one a time it names was on, was cut, moved or removed since, or the line would change other clips than it did: nothing is staged, and `revise` replaces it.
    pub(super) forward: Option<String>,
    /// Go back to the steps a take was made from: `media`, and the `take`'s number as `look` with `graph` or `media` lists it, where it says `kept`. The steps are written as that making read them, each seed as that take had it, so a making of them is that take's again where the model gives the same file for the same seed. They are written back by `step` lines of this edit, which the proposal shows. What else the take was made from, its sequences and signals, is the project's and is not changed. Used alone: change the steps in an edit after it.
    #[serde(rename = "restoreTake")]
    pub(super) restore_take: Option<RestoreInput>,
    /// Media that still name a recipe file (`recipe=`), to carry into the document: each recipe's `render`, `generate` and `join` lines are written as steps under its media, files named from the project's folder, and `recipe=` comes off its line. Its takes stay, and those made from the recipe as it stands are not asked for again; the answer's `carried` says how many (`takesHeld`) and what did not come with it (`notes`). The lines are first in the edit's script, so the `script` of the same edit can change the steps (`step <media> set <n> …`). A recipe with a `run` line cannot be carried, a document naming no program: its program is set up in the person's own generation file, and a `generate` step asks it.
    #[serde(default)]
    #[schemars(length(min = 1))]
    pub(super) carry: Vec<String>,
    /// Lines to add to the project's cast file, each `cast <name> "<who or what it is>" [ref=<picture> …]` or `style <name> "<how a shot looks>"`, for who and what a step of this edit shows that `look`'s `cast` and `styles` have not. They are written at once, since adding one asks for no shot again; one the file has is the person's to change, and is refused.
    #[serde(default)]
    pub(super) cast: Vec<String>,
    /// Sequence positions to read scopes at once the edit is made, as `look` with `scopes` reads them: the answer's `scopes` has a reading for each, with `before`, what was read there before the edit. A grade and its numbers in one call.
    #[serde(default)]
    #[schemars(length(max = MAX_LOOK_FRAMES))]
    pub(super) scopes: Vec<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RenderInput {
    /// `draft`, the default, is 360 high at 15 frames a second, to check quickly; `full` is the sequence's own size and rate, a delivery.
    #[schemars(extend("enum" = ["draft", "full"]))]
    pub(super) quality: Option<String>,
    #[schemars(description = format!("Sequence position. {TIME} {ANCHORS}"))]
    pub(super) from: Option<String>,
    /// Sequence position.
    pub(super) to: Option<String>,
    #[schemars(description = SEQUENCE)]
    pub(super) sequence: Option<String>,
    #[schemars(description = PROPOSAL)]
    pub(super) proposal: Option<String>,
    /// A render job id.
    pub(super) job: Option<String>,
    #[serde(default)]
    pub(super) cancel: bool,
    /// Write the sequence, whole, as a timeline file another editor opens, in place of rendering it: `otio`, OpenTimelineIO JSON, which DaVinci Resolve and Premiere Pro import, or `fcpxml`, Final Cut Pro XML 1.9, which Final Cut Pro and DaVinci Resolve import and Premiere Pro does not. It is written at once, to `output` in the project's `deliveries` folder, with the media named by where their files are. `notCarried` lists what the file leaves out, each with the `code` of its kind, `at` the clip, track, or transition it is, and what happened to it: clips that play no media file (titles, generators, nested sequences) arrive as gaps; captions, clip properties, masks and effects, links, and the mix are not written; a speed ramp is written as its speed overall and reverse as forward; a transition that is no dissolve arrives as one, and in an `fcpxml` one off the first video track as a cut; an `otio` has no frame size. `check` lists what arrives changed in form: an `fcpxml` has no tracks, so every track but the first video track arrives as connected clips. Tell the person what is in both. Use with `sequence` or `proposal` only.
    #[schemars(extend("enum" = kerf_interchange::Format::ALL.map(kerf_interchange::Format::name)))]
    pub(super) handoff: Option<String>,
}

pub(crate) fn input<T: DeserializeOwned>(arguments: Value) -> Result<T, McpError> {
    let arguments = if arguments.is_null() {
        json!({})
    } else {
        arguments
    };
    serde_json::from_value(arguments)
        .map_err(|error| McpError::new(McpCode::InvalidArguments, error.to_string()))
}
