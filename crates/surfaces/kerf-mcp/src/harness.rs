//! The agent harness on the kernel: `look`, `edit`, `render`, and `note`.
//!
//! An agent works from two pieces of context: the annotated `.kerf` document
//! and the language reference. It looks at media and frames, changes the
//! project with one edit script, renders, and leaves notes for the people
//! reviewing. Each edit becomes a proposal for the user, returned with its
//! document diff, the check findings, and a before/after preview of the
//! range it changed. Proposals and notes are by the router's author: Kerf,
//! or the MCP client that made them.

use super::{engine_error, KerfMcpRouter, McpError, ToolAnnotations};
use base64::Engine;
use kerf_engine::derivatives::{DerivativeCache, SourceIdentity};
use kerf_engine::render::{render_program, RenderOptions};
use kerf_lang::{
    apply_script, apply_script_with, parse_document, print_document, reference, Clock, PrintOptions,
};
use kerf_model::{Catalog, Clip, Content, Media, Project, Sequence, Time};
use kerf_runtime::{
    CachedFacts, NewComment, PendingProposal, ProposalEntry, ProposalFreshness, RenderJob,
    RenderRange, RuntimeClient, RuntimeError, StateSnapshot,
};
use kerf_skills::Skill;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
const MAX_INLINE_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

/// The tools every agent and MCP client uses.
pub const TOOL_NAMES: &[&str] = &["look", "edit", "render", "note", "make"];

/// As many frames as one tool result carries to a model.
const MAX_LOOK_FRAMES: usize = kerf_agent::MAX_TOOL_RESULT_IMAGES;
const DEFAULT_FRAME_WIDTH: u32 = 960;
/// A preview shows the frames either side of the first four cuts an edit
/// made or moved, two cuts to a row.
const PREVIEW_FRAMES: usize = 8;
const PREVIEW_COLUMNS: usize = 4;
const PREVIEW_WIDTH: u32 = 480;
/// The largest document carried as context; `look` returns all of it.
const MAX_CONTEXT_DOCUMENT_BYTES: usize = 96 * 1024;
const MAX_CONTEXT_DIFF_BYTES: usize = 8 * 1024;
/// Onsets a media index lists without a range.
const MAX_INDEX_ONSETS: usize = 100;
const DEFAULT_SHEET_TILE_WIDTH: u32 = 320;
const SHEET_COLUMNS: usize = 4;
const MAX_SHEET_FRAMES: usize = 16;

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
With `file`, the `media add` line for a file in the project's folder, and with no arguments `unplaced` lists those not yet in the document, and `toMake` the media whose recipe (`recipe=`) has yet to be run, or reads something that has changed since its take was made, which it names: a recipe is commands, so you ask for a making with `make`, and the person runs it or has let you. With `media`, a media that names a recipe lists its `takes`, each making kept, with the line (`use`) that in an edit has the media be that take; a `toMake` entry's `cannot` is what would refuse its making, to mend before a person is asked, and its `notes` what its model has said it does not take; an edit that names a recipe answers with `toMake` too; with no arguments, `models` lists the models a recipe's `generate` can name, each with what its owner says it takes: the picture it makes (`size`), its `rate`, the most `seconds` of one shot, the files it `reads` (of `guide`, `first`, `last`, `ref`), the other names a line can set with what they are when it does not (`more`), and, for one that follows a guide, `guide`, the `render` line that draws a guide at its size and rate. Write that line and a sequence of the model's shape, no longer than its `seconds`. A model with a name alone has not said, and none listed means none is set up: both are the person's to set, in their own models file. \
With `cost` and `at`, who draws each frame, what sends it to the CPU, and what it costs. \
With `proposal` it shows what a pending proposal would make: its document, \
what it would have made (`toMake`), its frames, and what it says.
- `edit` runs one script. It becomes a proposal for the user, returned with \
the document diff, check findings, and a before/after preview: the frames \
either side of each cut it made or moved, and the middle of a clip it changed \
in place. The project does not change until the user accepts it. Several \
proposals can wait side by side. Pass a pending proposal's id as `extend` to \
run more lines on top of it, or as `revise` to replace it: either way it \
becomes that proposal's next version (v2, v3), and the user can still see \
the versions before it. `recipes` writes recipe files with it, by the path \
a media's `recipe=` names: a file that holds another text is kept as it is, \
with the new text beside it as its next version, and the media move to that \
in the proposal. Where the person's own permissions accept an agent's edits, \
the answer says `accepted`: it is in the project, one revision undo takes \
back.
- `make` asks for a media to be made from its recipe, and answers with what \
would run. By default the person is asked (`asked`), in the review thread, \
and makes it; where they let an agent make on its own it runs (`making`), as \
a `job` to read, `wait` for, or `cancel`, and the media is the new take once \
it is made. Then look at the take, and change one thing for the next.
- `render` renders the sequence, or a range of it, to a file; with \
  `proposal`, what that proposal would make.
- `note` leaves a comment in the project's review thread, where the people \
  reviewing the cut read it: at a time or range of the sequence, as a reply \
  to a comment with `parent`, or about a proposal with `proposal` (say what \
  changed in a new version there). Use it for what is worth keeping beside \
  the cut, not to answer the person you are talking with.

Reusable presets are portable .kerfpreset files. `look` with `presets: true` \
lists project presets: the shared `presets/` library first, then \
`.kerf/presets` and the project folder. With `preset: \"file.kerfpreset\"` it shows the pattern, \
text slots, and each media slot with the clips that use it and the imported \
media that fit it (`fits`, with the latest `from` frame as `latestFrame`). \
`edit` with `savePreset: \"file.kerfpreset\"` \
saves the accepted sequence (optional `sequence` and `name`), without changing \
the project. To reuse one, `edit` with `preset`, `bindings` mapping each slot \
to {media: \"imported-media-id\", from: \"0f\"}, and optional `texts` mapping \
template clip ids to new text. This replaces the chosen sequence (the first \
by default) as a normal proposal. Every slot must be bound; its minimum \
duration includes transition handles. Inspect the preset before binding it.

A script's `captions` line needs a transcript of the clip's media made \
earlier with `look`; the proposal's script shows the caption lines it became.

Check findings include what an editor would catch: black frames, picture that \
leaves the frame's edges black, flash frames, a frame or two of another shot at \
a clip's edge (from the media's index), sound cut inside a word (known once \
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

/// The context read before every model request: the annotated document at
/// the current revision and the state of the agent's proposal.
pub fn context(runtime: &RuntimeClient) -> Result<String, String> {
    let error = |error: RuntimeError| format!("{}: {}", error.code, error.message);
    let (revision, document) = runtime.get_document(true).map_err(error)?;
    let state = runtime.get_state().map_err(error)?;
    let mut text = format!(
        "# Current project (revision {revision})\n\n```kerf\n{}```\n",
        bounded_lines(
            &document,
            MAX_CONTEXT_DOCUMENT_BYTES,
            "# … the document continues; `look` returns all of it"
        )
    );
    if let Some(status) = proposals_status(&state) {
        text.push('\n');
        text.push_str(&status);
    }
    Ok(text)
}

/// The proposals waiting for review, each with who made it, its version,
/// and its diff; with none waiting, what became of the last one.
fn proposals_status(state: &StateSnapshot) -> Option<String> {
    let open: Vec<&ProposalEntry> = state
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
        .collect();
    if open.is_empty() {
        return proposal_status(state.pending_proposal.as_ref());
    }
    let budget = (MAX_CONTEXT_DIFF_BYTES / open.len()).max(1024);
    let mut text = String::new();
    for entry in open {
        let stale = if entry.freshness == ProposalFreshness::Stale {
            " The project has changed since; revise it before the user can accept it."
        } else {
            ""
        };
        let earlier: Vec<String> = entry
            .versions
            .iter()
            .filter(|version| version.proposal_id != entry.proposal_id)
            .map(|version| format!("v{} {}", version.version, version.proposal_id))
            .collect();
        let earlier = if earlier.is_empty() {
            String::new()
        } else {
            format!(" Earlier versions: {}.", earlier.join(", "))
        };
        let _ = write!(
            text,
            "Proposal {} (v{}, by {}, from revision {}) is waiting for the user's \
             review.{stale}{earlier}\n\n```diff\n{}```\n",
            entry.proposal_id,
            entry.version,
            entry.author.name,
            entry.base_revision,
            bounded_lines(&entry.document_diff, budget, "… diff continues")
        );
    }
    Some(text)
}

fn proposal_status(proposal: Option<&PendingProposal>) -> Option<String> {
    Some(match proposal? {
        PendingProposal::Pending {
            proposal_id,
            base_revision,
            freshness,
            document_diff,
            ..
        } => {
            let stale = if *freshness == ProposalFreshness::Stale {
                " The project has changed since; revise it before the user can accept it."
            } else {
                ""
            };
            format!(
                "Proposal {proposal_id} (from revision {base_revision}) is waiting for the \
                 user's review.{stale}\n\n```diff\n{}```\n",
                bounded_lines(document_diff, MAX_CONTEXT_DIFF_BYTES, "… diff continues")
            )
        }
        PendingProposal::Accepted {
            proposal_id,
            applied_revision,
            ..
        } => format!("Proposal {proposal_id} was accepted at revision {applied_revision}.\n"),
        PendingProposal::Rejected { proposal_id, .. } => {
            format!("Proposal {proposal_id} was rejected.\n")
        }
    })
}

/// `text` cut at a line boundary within `max` bytes, ending with `note`.
fn bounded_lines(text: &str, max: usize, note: &str) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').map_or(0, |line| line + 1);
    format!("{}{note}\n", &text[..end])
}

pub(super) fn metadata(name: &str) -> Option<(&'static str, Value, ToolAnnotations)> {
    let time = "Timecode (`01:00:04:12`), frames (`96f`), or seconds (`4.5s`).";
    let proposal = "The id of the pending proposal to see: what it would make of the project.";
    let sequence = "A sequence id from the document; the first sequence when absent.";
    Some(match name {
        "look" => (
            "See the project and its footage. No arguments: the annotated document and \
             proposal state. `media`: that media's index — where it is used, scene cuts, \
             pauses, loudest moments, sound onsets, tempo, and with `transcript` what \
             is said (`words` times each word), and with `facts` what the camera does in \
             each shot and how loud it is. `contrast`: titles and captions too faint to read. \
             `sheet`: a contact sheet of a range. \
             `at`: frames, as source frames of `media` or program frames of the \
             sequence. `transcript` without `media`: what the sequence says. \
             `track`: where a point, a surface, an object, or the camera goes, as keyframes. \
             `proposal`: the same, as a pending proposal would make the project.",
            json!({
                "type": "object",
                "properties": {
                    "media": {"type": "string", "description": "A media id from the document."},
                    "at": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1,
                        "maxItems": MAX_LOOK_FRAMES,
                        "description": format!("Frames to see: media positions with `media`, otherwise sequence positions. {time}")
                    },
                    "transcript": {"type": "boolean", "description": "What is said, by local transcription, up to 15 minutes per call: of `media`, or else of the sequence as it plays."},
                    "words": {"type": "boolean", "description": "With `transcript`: time every word, for cutting on speech. Choose a short range; it is long."},
                    "sync": {"type": "array", "items": {"type": "string"}, "description": "Media ids of recordings of the same thing to line up by their sound, as for a multicam. Returns how far each falls after the first, `offset` in seconds, with `confidence`, and a `script` that builds a sequence with a camera on each track. Then a clip of that sequence (`seq=multicam`) with `angle=V2` shows one camera, and splitting it cuts between them. Check `confidence`: low means they may have nothing in common."},
                    "track": {"type": "object", "description": "Follow a point of the program's picture over `from`–`to`: `x` and `y` where it is at `from`, 0 to 1 across the frame, on something with detail, such as a corner rather than a flat area. Returns where it went, and with `clip` a line setting that clip's x and y to follow it: run it with `edit`, and parent what should stick to it to that clip, a `gen=null@1` for instance. `lostAt` says where the point was lost, if it was. Or give `corners` instead of `x` and `y`, four [x, y] at `from` going upper left, upper right, lower right, lower left, around a flat surface with detail on it, such as a screen or a sign, to follow how it moves and turns: it returns those corners over time, `outline`, the keyframes a moving mask takes (`mask c add k1 points=...`), and with `clip` a corner pin line that fits that clip, one filling the frame, onto the surface. Or to cut out an object that changes shape, a person for instance, give `inside`, [x, y] points on it at `from`, with `outside`, points off it, and `box`, [left, top, right, bottom] around it, as needed: it returns its outline over time, the same `outline`, and with `clip` a mask line for that clip, with a third number on each corner where the edge measures soft, such as out of focus or moving fast. `model` picks what helps find it: `classical`, the default, goes by colour and edges with nothing to install; `mobilesam` does better where colours are alike, and names the file to fetch when it is missing. Or give `camera: true` to solve where the camera went over the range, one shot of a camera that moves through a still scene, to pin clips in depth to moving footage: it returns a `script` of `gen=camera@1` keyframes (on `clip`, or on a new camera without one), `fit` (`error` and `worst`, how far what was followed lands from where the solve puts it in the sequence's pixels; under a pixel holds, and past two a clip slides; `agreeing`, the share of it that fits), `fov`, and `points`, places in the world to set a clip's `x`, `y`, and `z` to with `three_d=true`, each with where it shows (`at`, on `frame`) and the frame it was last seen on (`lastFrame`), so that it stays on that spot. Hide what is not in the footage first. `solved: false` with a `reason` says when a shot cannot be solved, such as a camera that only turns; `lostAt` says where a cut or lost frame ended it. `lensKnown: false` means the camera never turned, so any lens fits and `fov` is the default: give `fov`, down the frame in degrees, if you know it. A zoom reads as a move along the camera's way, with a higher `error`. Use with `from`, `to`, and optionally `sequence` or `proposal`.", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "corners": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}}, "inside": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}}, "outside": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}}, "box": {"type": "array", "items": {"type": "number"}}, "model": {"type": "string"}, "camera": {"type": "boolean"}, "fov": {"type": "number"}, "clip": {"type": "string"}}},
                    "facts": {"type": "boolean", "description": "With `media`: its fact sheet beside its index, the first time slowly: what the camera does in each shot (`camera`, such as `pan right` or `handheld`; `pan`, `tilt`, and `zoom` in shares of the frame a second, `roll` in degrees a second, and `agreeing`, how much of the picture follows that), how loud it is (`loudness`, in LUFS and dBTP), and in `notMeasured` what it does not say and why: faces, the subject, and on-screen text."},
                    "contrast": {"type": "boolean", "description": "Read each title and caption of the sequence over `from`–`to` (all of it by default) against what is behind it, from rendered frames, and return `contrast`, findings for those too faint to read or that do not show, with `read` and `of`, how many of them it read. Slower than a check, so it is not part of one; the editor's full QC has it as a row."},
                    "scopes": {"type": "boolean", "description": "With `at`: instead of frames, what scopes read at those program positions, as numbers: levels of luma and each channel in percent, the share of samples clipped by channel, in shadows, midtones, and highlights, and in 16 bands of luma; the color cast, saturation, the share of colored samples on the skin tone line, and the densest colors. Grade by these: compare `proposal` with the project, shots with each other, and the result with where it should land. Read from 8-bit frames, or in an HDR sequence from its PQ or HLG signal in BT.2020, with `hdr` saying what that comes to: `peakNits`, `meanNits`, `overWhite` and `outsideBt709` as shares of the samples, and `referenceWhite`, where SDR white falls on the signal in percent."},
                    "sheet": {"type": "integer", "minimum": 2, "maximum": MAX_SHEET_FRAMES, "description": "A contact sheet of this many frames spread evenly over `from`–`to` (all of it by default) as one image, four to a row: of `media`, or else of the sequence. Scan with a sheet, then look `at` the moments that matter."},
                    "from": {"type": "string", "description": format!("The start of the range for `transcript`, `sheet`, or a media's onsets. Positions of `media`, or else of the sequence. {time}")},
                    "to": {"type": "string", "description": "The end of that range."},
                    "width": {"type": "integer", "minimum": 64, "maximum": 1920, "description": "Frame width in pixels (default 960; 320 for each frame of a sheet)."},
                    "sequence": {"type": "string", "description": sequence},
                    "proposal": {"type": "string", "description": proposal},
                    "preset": {"type": "string", "description": "Inspect a .kerfpreset file: pattern, text slots, and media slots with the imported media that fit each. Use alone."},
                    "presets": {"type": "boolean", "description": "List .kerfpreset files in the project's shared presets/ library, then .kerf/presets and the project root. Use alone with true."},
                    "effect": {"type": "string", "description": "An effect, transition, generator, or clip property: every parameter of it, where the reference keeps a few of a generator that has many. Raster effects are listed by id only. Use alone."},
                    "file": {"type": "string", "description": "A file in the project's folder: what it holds, and in `add` the `media add` line that puts it in the document. With no arguments `look` gives in `unplaced` the footage there not yet in the document. Use alone."},
                    "cost": {"type": "boolean", "description": "With `at`, program positions: who draws each frame and what it costs. `draws` is `gpu` or `cpu`, `because` says what sends it to the CPU, one line a cause, `ms` is a frame at full size and `msHalf` at half, against `frameMs`, a frame's share of the time. Playing, the editor draws smaller until frames keep up, and a frame the CPU draws takes every core."},
                    "font": {"type": "string", "description": "A font family, or `sans-serif`, `serif`, or `monospace`: its faces, and for a variable font each axis with its `tag`, `min`, `default`, and `max`, on the font's own scale, which is what a `title`'s `wght`, `wdth`, `slnt`, `opsz`, and `ital` are set on. For a name no font has, the families that contain it; for \"\", all of them. Use alone."},
                    "effects": {"type": "string", "description": "Words for the look or moment you want, such as \"glow neon\", \"zoom hit\", or \"retro tv\": the effects, transitions, and generators whose id, category, guidance, or parameters match, best first, each with when to use it, and in `properties` the clip properties that do. Use alone, or with `area`."},
                    "area": {"type": "string", "enum": ["edit", "color", "motion", "vfx", "image"], "description": "With `effects`: only what belongs to this part of the editor."}
                },
                "additionalProperties": false
            }),
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
             on a pending proposal; `revise` replaces it. `recipes` writes recipe files as part of it. `savePreset` saves a portable \
             .kerfpreset file without changing the project. `preset` with `bindings` \
             applies one as a proposal replacing the chosen sequence.",
            json!({
                "type": "object",
                "properties": {
                    "script": {"type": "string", "minLength": 1, "description": "One operation per line, applied in order and atomically."},
                    "savePreset": {"type": "string", "description": "Save the accepted sequence to a new .kerfpreset file inside the project. Existing files are never overwritten."},
                    "name": {"type": "string", "description": "Name for savePreset."},
                    "preset": {"type": "string", "description": "A .kerfpreset file to apply. Inspect it with look first."},
                    "sequence": {"type": "string", "description": "Sequence to save or replace with a preset; first by default."},
                    "bindings": {"type": "object", "additionalProperties": {"type": "object", "properties": {
                        "media": {"type": "string", "description": "Imported replacement media id."},
                        "from": {"type": "string", "description": "Source start, including handles; default 0f."},
                        "beats": {"type": "boolean", "description": "Music: move the markers over its clips to its nearest sound onsets, so looks that hit them land on its beats."}
                    }, "required": ["media"], "additionalProperties": false}},
                    "texts": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Template caption/title clip id to replacement text."},
                    "extend": {"type": "string", "description": "The id of a pending proposal to build on: the script runs on what it makes, the two become one proposal, and the preview shows what this script changed."},
                    "revise": {"type": "string", "description": "The id of a pending proposal this one replaces; the script runs on the project."},
                    "recipes": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Recipe files to write, by the path a media line's `recipe=` names, each with its whole text. A file that is there and holds another text is not written over: the text is kept beside it as its next version (`shots/open@2.kerfrecipe`), and the media that name it move to it in this proposal, so that undo goes back. Alone, or with a script that adds the media line."}
                },
                "oneOf": [{"required": ["script"]}, {"required": ["recipes"]}, {"required": ["preset"]}, {"required": ["savePreset"]}],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
            },
        ),
        "render" => (
            "Render the sequence, or the range `from`–`to`, to a file — of the project, or \
             of what a pending `proposal` would make. Returns a job that renders while this \
             session lasts; pass `job` to read its progress and output, with `cancel` to stop it.",
            json!({
                "type": "object",
                "properties": {
                    "quality": {"type": "string", "enum": ["draft", "full"], "description": "`draft`, the default, is 360 high at 15 frames a second, to check quickly; `full` is the sequence's own size and rate, a delivery, refused while a blocking note is open (a person resolves it in the review)."},
                    "from": {"type": "string", "description": format!("Sequence position. {time}")},
                    "to": {"type": "string", "description": "Sequence position."},
                    "sequence": {"type": "string", "description": sequence},
                    "proposal": {"type": "string", "description": proposal},
                    "job": {"type": "string", "description": "A render job id."},
                    "cancel": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
            },
        ),
        "note" => (
            "Leave a note in the project's review thread, where the people reviewing the cut \
             read it beside the timeline: at `at` (for `duration`) on the program sequence, as \
             a reply to the comment `parent`, or about the proposal `proposal`. It changes \
             nothing in the project. Returns the comment as the review log keeps it.",
            json!({
                "type": "object",
                "properties": {
                    "body": {"type": "string", "minLength": 1, "description": "What the note says."},
                    "at": {"type": "string", "description": format!("Where on the program sequence (the first). {time}")},
                    "duration": {"type": "string", "description": "How long a range it is about, from `at`: frames (`60f`), seconds (`2.5s`), or a timecode length (`00:00:02:12`)."},
                    "parent": {"type": "string", "description": "The id of the comment this replies to; a reply to a reply joins its thread."},
                    "proposal": {"type": "string", "description": "The id of a proposal (any version) the note is about."},
                    "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 32, "description": "Short topic tags without `#`, such as `captions` or `audio`."},
                    "blocking": {"type": "boolean", "description": "The cut should not be delivered until this is resolved."}
                },
                "required": ["body"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
            },
        ),
        _ => return None,
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LookInput {
    preset: Option<String>,
    #[serde(default)]
    presets: bool,
    effect: Option<String>,
    effects: Option<String>,
    font: Option<String>,
    area: Option<String>,
    media: Option<String>,
    #[serde(default)]
    at: Vec<String>,
    #[serde(default)]
    transcript: bool,
    #[serde(default)]
    words: bool,
    #[serde(default)]
    scopes: bool,
    #[serde(default)]
    facts: bool,
    #[serde(default)]
    contrast: bool,
    track: Option<TrackInput>,
    #[serde(default)]
    sync: Vec<String>,
    sheet: Option<usize>,
    from: Option<String>,
    to: Option<String>,
    width: Option<u32>,
    sequence: Option<String>,
    proposal: Option<String>,
    file: Option<String>,
    #[serde(default)]
    cost: bool,
}

/// What `look` says of a font: its faces, their axes and the OpenType features each has, or the
/// families a name no font has could mean.
fn font_description(wanted: &str) -> Value {
    use kerf_text::fonts;
    // Four places say all a font does, where a float says it with noise.
    let place = |value: f32| (f64::from(value) * 10000.0).round() / 10000.0;
    let family = crate::text_lint::family_of(wanted);
    let faces = fonts::faces_of(family);
    if faces.is_empty() {
        fonts::scan_system();
        let words = wanted.trim().to_lowercase();
        // A name with a dot in front is the system's own, and it does not offer them.
        let families: Vec<String> = fonts::families()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|name| !name.starts_with('.') && name.to_lowercase().contains(&words))
            .collect();
        return json!({"font": wanted, "found": false, "families": families});
    }
    let faces: Vec<Value> = faces
        .iter()
        .map(|face| {
            let axes: Vec<Value> = face
                .axes()
                .iter()
                .map(|axis| {
                    json!({
                        "tag": String::from_utf8_lossy(&axis.tag),
                        "min": place(axis.min),
                        "default": place(axis.default),
                        "max": place(axis.max),
                    })
                })
                .collect();
            json!({
                "style": face.info.style,
                "weight": face.info.weight,
                "italic": face.info.italic,
                "axes": axes,
                "features": face.features().iter().map(|tag| String::from_utf8_lossy(tag).into_owned()).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"font": family, "found": true, "faces": faces})
}

/// A trained model by its id, loaded once. Its weights are never bundled: they are read from
/// `KERF_MOBILESAM` or the models folder in the user's home, and checked against the registry.
fn trained_model(id: &str) -> Result<std::sync::Arc<dyn kerf_segment::MaskModel>, McpError> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    type Loaded = HashMap<String, Arc<dyn kerf_segment::MaskModel>>;
    static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);
    let invalid = |message: String| McpError::new("INVALID_ARGUMENTS", message);
    let info = kerf_segment::info(id).ok_or_else(|| {
        invalid(format!(
            "no model `{id}`; there is `{}` and {}",
            kerf_segment::CLASSICAL,
            kerf_segment::MODELS
                .iter()
                .map(|model| format!("`{}`", model.id))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let mut loaded = LOADED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let loaded = loaded.get_or_insert_with(HashMap::new);
    if let Some(model) = loaded.get(id) {
        return Ok(model.clone());
    }
    let path = std::env::var_os("KERF_MOBILESAM")
        .filter(|_| id == "mobilesam")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                std::path::Path::new(&home)
                    .join(".kerf/models")
                    .join(info.file_name)
            })
        })
        .ok_or_else(|| invalid(format!("nowhere to find `{}`", info.file_name)))?;
    let bytes = std::fs::read(&path).map_err(|_| {
        invalid(format!(
            "`{id}` is not installed: its weights ({}, {} MB, {}, by {}) go at {}, from {}",
            info.file_name,
            info.size / 1_000_000,
            info.licence,
            info.authors,
            path.display(),
            info.url
        ))
    })?;
    let kerf_segment::Loaded::Mask(model) =
        kerf_segment::load(id, &bytes).map_err(|error| McpError::new("RENDER_FAILED", error))?;
    loaded.insert(id.to_owned(), model.clone());
    Ok(model)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackInput {
    x: Option<f64>,
    y: Option<f64>,
    corners: Option<[[f64; 2]; 4]>,
    #[serde(default)]
    inside: Vec<[f64; 2]>,
    #[serde(default)]
    outside: Vec<[f64; 2]>,
    #[serde(rename = "box")]
    bbox: Option<[f64; 4]>,
    model: Option<String>,
    #[serde(default)]
    camera: bool,
    fov: Option<f64>,
    clip: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditInput {
    script: Option<String>,
    #[serde(rename = "savePreset")]
    save_preset: Option<String>,
    name: Option<String>,
    preset: Option<String>,
    sequence: Option<String>,
    #[serde(default)]
    bindings: BTreeMap<String, crate::presets::BindingInput>,
    #[serde(default)]
    texts: BTreeMap<String, String>,
    extend: Option<String>,
    revise: Option<String>,
    /// Recipe files the edit writes, by the path a media line names, each with its text.
    #[serde(default)]
    recipes: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderInput {
    quality: Option<String>,
    from: Option<String>,
    to: Option<String>,
    sequence: Option<String>,
    proposal: Option<String>,
    job: Option<String>,
    #[serde(default)]
    cancel: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteInput {
    body: String,
    at: Option<String>,
    duration: Option<String>,
    parent: Option<String>,
    proposal: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    blocking: bool,
}

pub(crate) fn input<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, McpError> {
    let arguments = if arguments.is_null() {
        json!({})
    } else {
        arguments
    };
    serde_json::from_value(arguments)
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))
}

fn catalog() -> &'static Catalog {
    Catalog::builtin()
}

fn parse(text: &str) -> Result<Project, McpError> {
    parse_document(text, catalog())
        .map(|document| document.project)
        .map_err(|errors| McpError::new("INVALID_PROJECT", diagnostics(&errors)))
}

fn diagnostics(errors: &[kerf_model::Diagnostic]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The sequence a request is about: `id`, or the first one.
fn program<'a>(project: &'a Project, id: Option<&str>) -> Result<&'a Sequence, McpError> {
    match id {
        Some(id) => project.sequence(id).ok_or_else(|| {
            McpError::new(
                "UNKNOWN_SEQUENCE",
                format!("no sequence `{id}` in the project"),
            )
        }),
        None => project
            .sequences
            .first()
            .ok_or_else(|| McpError::new("NO_SEQUENCE", "the project has no sequence")),
    }
}

/// The cues of `transcript`, `offset` seconds later, on `clock`; with
/// `words`, each word's time too.
fn cues(transcript: &kerf_engine::Transcript, offset: f64, clock: &Clock, words: bool) -> Value {
    let at = |seconds: f64| print_seconds(clock, offset + seconds);
    json!(transcript
        .cues
        .iter()
        .map(|cue| {
            let mut value = json!({
                "from": at(cue.start),
                "to": at(cue.end),
                "text": cue.text.trim(),
            });
            if words {
                value["words"] = json!(cue
                    .words
                    .iter()
                    .map(|word| json!({
                        "from": at(word.start),
                        "to": at(word.end),
                        "text": word.text.trim(),
                    }))
                    .collect::<Vec<_>>());
            }
            value
        })
        .collect::<Vec<_>>())
}

fn position(clock: &Clock, text: &str) -> Result<Time, McpError> {
    clock
        .position(text)
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error))
}

/// `seconds` from the clock's origin, on the nearest frame.
fn print_seconds(clock: &Clock, seconds: f64) -> String {
    let time = Time::from_seconds(seconds).unwrap_or(Time::ZERO);
    clock.print_position(clock.rate.round(time))
}

impl KerfMcpRouter {
    /// The proposal `id` waiting for review: the operations it runs, the
    /// script as it was written, and its base revision.
    fn pending(&self, id: &str) -> Result<(String, String, u64), McpError> {
        let state = self.runtime.get_state()?;
        // Proposals staged before they kept their source wrote none.
        let written =
            |source: &str, script: &str| if source.is_empty() { script } else { source }.to_owned();
        // Any version of an open proposal: the current one, or one the
        // user can still accept in its place.
        for entry in state
            .proposals
            .iter()
            .filter(|entry| entry.status == "pending")
        {
            if entry.proposal_id == id {
                return Ok((
                    entry.script.clone(),
                    written(&entry.source, &entry.script),
                    entry.base_revision,
                ));
            }
            if let Some(version) = entry
                .versions
                .iter()
                .find(|version| version.proposal_id == id)
            {
                return Ok((
                    version.script.clone(),
                    written(&version.source, &version.script),
                    version.base_revision,
                ));
            }
        }
        if !state.proposals.is_empty() {
            return Err(not_pending(id));
        }
        match state.pending_proposal {
            Some(PendingProposal::Pending {
                proposal_id,
                script,
                source,
                base_revision,
                ..
            }) if proposal_id == id => {
                // Proposals staged before they kept their source wrote none.
                let source = if source.is_empty() {
                    script.clone()
                } else {
                    source
                };
                Ok((script, source, base_revision))
            }
            _ => Err(not_pending(id)),
        }
    }

    /// Post a note to the review log, by this router's author.
    pub(super) fn note(&self, arguments: Value) -> Result<Value, McpError> {
        let input: NoteInput = input(arguments)?;
        let body = input.body.trim();
        if body.is_empty() {
            return Err(McpError::new("INVALID_ARGUMENTS", "a note needs a body"));
        }
        if input.duration.is_some() && input.at.is_none() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "duration is a range from `at`; pass at too",
            ));
        }
        let tags = input
            .tags
            .iter()
            .map(|tag| tag.trim().trim_start_matches('#').trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();
        let mut timecode = None;
        let (mut at, mut duration) = (None, None);
        if let Some(text) = &input.at {
            let (_, project) = self.project_at(None)?;
            let sequence = program(&project, None)?;
            let clock = Clock::of_sequence(sequence);
            let start = position(&clock, text)?;
            if start.is_negative() || start > sequence.duration() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!(
                        "at {text} is outside sequence `{}`, which ends at {}",
                        sequence.id,
                        clock.print_position(sequence.duration())
                    ),
                ));
            }
            if let Some(length) = &input.duration {
                let length = clock
                    .duration(length)
                    .map_err(|error| McpError::new("INVALID_ARGUMENTS", error))?;
                if !length.is_positive() {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "a duration must be longer than 0",
                    ));
                }
                duration = Some(length.min(sequence.duration() - start).seconds());
            }
            at = Some(start.seconds());
            timecode = Some(clock.print_position(clock.rate.round(start)));
        }
        let state = self.runtime.get_state()?;
        if let Some(id) = &input.proposal {
            let known = state.proposals.iter().any(|entry| {
                entry.proposal_id == *id
                    || entry
                        .versions
                        .iter()
                        .any(|version| version.proposal_id == *id)
            });
            if !known {
                return Err(McpError::new(
                    "UNKNOWN_PROPOSAL",
                    format!("no proposal {id} in the project"),
                ));
            }
        }
        // Threads are a comment and its replies: a reply to a reply joins
        // the thread of the comment it answers.
        let parent = match &input.parent {
            None => None,
            Some(id) => {
                let log = self.runtime.comments()?;
                let comment = log
                    .items
                    .iter()
                    .find(|comment| comment.id == *id)
                    .ok_or_else(|| {
                        McpError::new("UNKNOWN_COMMENT", format!("no comment {id} in the review"))
                    })?;
                Some(comment.parent.clone().unwrap_or_else(|| comment.id.clone()))
            }
        };
        let author = self.author();
        let actor = if author.kind == "mcp" { "mcp" } else { "agent" };
        let mut comment = NewComment::new(author, body);
        comment.parent = parent;
        comment.at = at;
        comment.duration = duration;
        comment.proposal = input.proposal;
        comment.tags = tags;
        comment.blocking = input.blocking;
        let (comment, log) = self.runtime.add_comment(actor, comment)?;
        let mut result = json!({
            "comment": comment,
            "reviewRevision": log.review_revision,
        });
        if let Some(timecode) = timecode {
            result["timecode"] = json!(timecode);
        }
        Ok(result)
    }

    /// The current revision and the project, or what the pending
    /// `proposal` would make of it.
    pub(crate) fn project_at(&self, proposal: Option<&str>) -> Result<(u64, Project), McpError> {
        let (revision, document) = self.runtime.get_document(false)?;
        let project = parse(&document)?;
        let Some(id) = proposal else {
            return Ok((revision, project));
        };
        let (script, _, base_revision) = self.pending(id)?;
        if base_revision != revision {
            return Err(McpError::new(
                "PROPOSAL_STALE",
                format!(
                    "proposal {id} was made at revision {base_revision} and the project is \
                     at revision {revision}; revise it"
                ),
            ));
        }
        let proposed = apply_script(&project, catalog(), &script)
            .map_err(|errors| McpError::new("PROPOSAL_STALE", diagnostics(&errors)))?;
        Ok((revision, proposed.project))
    }

    /// What a pending proposal holds that the project does not, and how to reach it: ` <id> is in
    /// pending proposal p: pass it as `extend`` for what to say after a name was not found.
    fn in_pending(&self, name: &str, argument: &str) -> String {
        self.pending_with(name).map_or_else(String::new, |id| {
            format!("; `{name}` is in pending proposal {id}: pass it as `{argument}`")
        })
    }

    /// The pending proposal whose script names `name`, if one does.
    pub(crate) fn pending_with(&self, name: &str) -> Option<String> {
        let state = self.runtime.get_state().ok()?;
        let holds = |script: &str| {
            script
                .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
                .any(|word| word == name)
        };
        state
            .proposals
            .iter()
            .filter(|entry| entry.status == "pending")
            .find(|entry| holds(&entry.script))
            .map(|entry| entry.proposal_id.clone())
    }

    /// Where the rest of a cut may be, for a sequence that has too little in it.
    fn any_pending(&self, argument: &str) -> String {
        let Ok(state) = self.runtime.get_state() else {
            return String::new();
        };
        state
            .proposals
            .iter()
            .find(|entry| entry.status == "pending")
            .map_or_else(String::new, |entry| {
                format!(
                    ", which stays as it is until a proposal is accepted; to see pending proposal {}, pass it as `{argument}`",
                    entry.proposal_id
                )
            })
    }

    /// `look` with no arguments adds this: footage in the project's folder not yet in the document.
    fn unplaced(&self, project: &Project) -> Vec<String> {
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let known: Vec<std::path::PathBuf> = resolved
            .media
            .iter()
            .map(|media| std::path::PathBuf::from(&media.path))
            .collect();
        let mut files: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_file() && is_footage(&entry.path()))
            .filter(|entry| !known.contains(&entry.path()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        files.sort();
        files
    }

    /// `look` with no arguments adds this: the media that name a recipe and want making, each with
    /// why and who can. An agent's tools run no recipe. What would refuse the making is said here
    /// (`cannot`), read as a person's plan is and with nothing run, and so is what a model has said
    /// it does not take (`notes`): a recipe is mended before anyone is asked to make it.
    /// `waiting` says that `project` is what a proposal not yet accepted would make: a making
    /// is asked of the project, so of such a media only once its proposal is accepted.
    fn to_make(&self, project: &Project, waiting: bool) -> Vec<Value> {
        let first = if waiting {
            "once this proposal is accepted, "
        } else {
            ""
        };
        let models = kerf_engine::recipe::model::Models::load();
        project
            .media
            .iter()
            .filter_map(|media| {
                let recipe = media.recipe.as_deref()?;
                let why = kerf_engine::recipe::unmade(project, &self.dir, media)?;
                let mut entry = json!({
                    "media": media.id.as_str(),
                    "recipe": recipe,
                    "why": why.reason(),
                });
                let id = media.id.as_str();
                let plan = models
                    .as_ref()
                    .map_err(ToString::to_string)
                    .and_then(|models| {
                        kerf_engine::recipe::plan(project, &self.dir, id, models)
                            .map_err(|error| error.to_string())
                    });
                match plan {
                    Ok(plan) => {
                        if !plan.notes.is_empty() {
                            entry["notes"] = json!(plan.notes);
                        }
                        entry["make"] = json!(format!(
                            "{first}ask for it with `make`: a person runs `kerf make {id}`, or Make from Recipe… in the editor's bin, unless they let an agent make on its own"
                        ));
                    }
                    Err(cannot) => {
                        entry["cannot"] = json!(cannot_said(cannot, models.as_ref().ok()));
                        entry["make"] = json!(format!(
                            "it cannot be made as it is: mend what `cannot` says first; then, {first}ask for it with `make`"
                        ));
                    }
                }
                Some(entry)
            })
            .collect()
    }

    /// `look` with `media` adds this for a media that names a recipe: the takes its recipe has
    /// made, which one the media is, and what has changed since each was made.
    fn takes(&self, project: &Project, media: &kerf_model::Media) -> Option<Value> {
        let recipe = media.recipe.as_ref()?;
        let takes = kerf_engine::recipe::takes(project, &self.dir, media).ok()?;
        Some(json!({
            "made": takes
                .iter()
                .map(|take| {
                    let mut entry = json!({
                        "take": take.number,
                        "path": take.stored,
                        "chosen": take.chosen,
                    });
                    if !take.changed.is_empty() {
                        entry["changedSince"] = json!(take.changed);
                    }
                    // The whole line, since a take need not be as long as the one before it, and
                    // with the version of the recipe it was made from.
                    if !take.chosen {
                        if let Ok(fields) = self.media_fields(&take.stored) {
                            entry["use"] = json!(format!(
                                "media replace {} {fields} recipe={:?}",
                                media.id,
                                take.recipe.as_deref().unwrap_or(recipe)
                            ));
                        }
                    }
                    entry
                })
                .collect::<Vec<_>>(),
            "choose": format!(
                "a take's `use` is the line that, in an edit, has the media be that take, made from the recipe it was made from; ask for a new one with `make`, which a person runs with `kerf make {}` unless they let an agent make on its own",
                media.id
            ),
        }))
    }

    /// What a media line says of the file `file` of the project after its id: its path, rate,
    /// length in whole frames, and streams, as an import reads a file.
    fn media_fields(&self, file: &str) -> Result<String, McpError> {
        let path = self.dir.join(file);
        let info = kerf_engine::info(&path.to_string_lossy())
            .map_err(|error| McpError::new("UNREADABLE_FILE", format!("{file}: {error}")))?;
        let fps = info.fps.unwrap_or(30.0);
        let seconds = info.duration.unwrap_or(0.0);
        // Whole frames and stereo at 48 kHz, as an import reads a file.
        let mut fields = format!(
            "path={file:?} rate={} dur={}f",
            fps_text(fps),
            frames_of(seconds, fps)
        );
        if let (Some(width), Some(height)) = (info.width, info.height) {
            fields.push_str(&format!(" video={width}x{height}"));
        }
        if info.has_audio == Some(true) {
            fields.push_str(" audio=2ch@48000");
        }
        Ok(fields)
    }

    /// What a file holds, and the `media add` line that puts it in the document.
    fn look_at_file(&self, file: &str) -> Result<Value, McpError> {
        let path = self.dir.join(file);
        if !path.is_file() {
            return Err(McpError::new(
                "UNKNOWN_FILE",
                format!("no file `{file}` in the project's folder"),
            ));
        }
        let fields = self.media_fields(file)?;
        let info = kerf_engine::info(&path.to_string_lossy())
            .map_err(|error| McpError::new("UNREADABLE_FILE", format!("{file}: {error}")))?;
        let fps = info.fps.unwrap_or(30.0);
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("media");
        let id: String = stem
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let seconds = info.duration.unwrap_or(0.0);
        let line = format!("media add {id} {fields}");
        Ok(json!({
            "file": file,
            "seconds": seconds,
            "fps": fps,
            "width": info.width,
            "height": info.height,
            "add": line,
        }))
    }

    /// Who draws each frame at `at`, why, and what it costs.
    fn frame_costs(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        at: &[String],
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let compositor = kerf_compose::Compositor::new(
            &resolved,
            catalog(),
            &kerf_compose::ComposeOptions {
                sequence: Some(sequence.id.to_string()),
                accel: kerf_compose::Accel::Always,
                ..Default::default()
            },
        )
        .map_err(|error| McpError::new("RENDER_FAILED", error.to_string()))?;
        // The middle of three, the first having had a decoder to open.
        let median_ms = |at: Time, scale: f64| {
            let _ = compositor.frame(at, scale);
            let mut times: Vec<f64> = (0..3)
                .map(|_| {
                    let started = std::time::Instant::now();
                    let _ = compositor.frame(at, scale);
                    started.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            times.sort_by(f64::total_cmp);
            (times[1] * 10.0).round() / 10.0
        };
        let mut costs = Vec::with_capacity(at.len());
        for text in at {
            let time = position(&clock, text)?;
            if time.is_negative() || time >= duration {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!(
                        "`{text}` is outside sequence `{}`{}",
                        sequence.id,
                        self.any_pending("proposal")
                    ),
                ));
            }
            let because = compositor.why_on_cpu(time, 1.0);
            costs.push(json!({
                "at": text,
                "draws": if because.is_empty() { "gpu" } else { "cpu" },
                "because": because,
                "ms": median_ms(time, 1.0),
                "msHalf": median_ms(time, 0.5),
            }));
        }
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "size": format!("{}x{}", sequence.width, sequence.height),
            "frameMs": (1000.0 / sequence.rate.fps() * 10.0).round() / 10.0,
            "costs": costs,
        }))
    }

    pub(super) fn look(&self, arguments: Value) -> Result<Value, McpError> {
        let preset_only = arguments.as_object().is_some_and(|o| o.len() == 1);
        let only_area_beside = arguments.as_object().is_some_and(|o| o.len() == 2);
        let input: LookInput = input(arguments)?;
        let area = match input.area.as_deref() {
            Some(name) => Some(kerf_model::Area::parse(name).ok_or_else(|| {
                McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("no area `{name}`: edit, color, motion, vfx, or image"),
                )
            })?),
            None => None,
        };
        if let Some(font) = &input.font {
            if !preset_only {
                return Err(McpError::new("INVALID_ARGUMENTS", "use font alone"));
            }
            return Ok(font_description(font));
        }
        if let Some(words) = &input.effects {
            if !(preset_only || area.is_some() && only_area_beside) {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "use effects alone, or with area",
                ));
            }
            return Ok(
                json!({"effects": find_fx(words, area), "properties": find_properties(words)}),
            );
        }
        if area.is_some() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "area narrows effects; pass effects too",
            ));
        }
        if let Some(id) = &input.effect {
            if !preset_only {
                return Err(McpError::new("INVALID_ARGUMENTS", "use effect alone"));
            }
            if let Some(property) = clip_properties().find(|param| param.name == id.as_str()) {
                return Ok(json!({
                    "property": property.name,
                    "reference": kerf_lang::property_reference(property),
                }));
            }
            let fx = catalog().fx(id).ok_or_else(|| {
                let message = match kerf_model::catalog::retired(id) {
                    Some(_) => kerf_model::catalog::unknown("effect", id),
                    None => {
                        let names = catalog()
                            .all()
                            .map(|fx| fx.id.as_str())
                            .chain(clip_properties().map(|param| param.name.as_str()));
                        match kerf_model::catalog::nearest(id, names) {
                            Some(near) => format!("no effect `{id}` in the catalog; did you mean `{near}`?"),
                            None => format!("no effect `{id}` in the catalog; `effects` finds one by what it does"),
                        }
                    }
                };
                McpError::new("INVALID_ARGUMENTS", message)
            })?;
            return Ok(json!({
                "effect": fx.id.as_str(),
                "category": fx.category,
                "area": fx.area().name(),
                "engine": fx.engine.name(),
                "reference": kerf_lang::fx_reference(fx),
            }));
        }
        if let Some(file) = &input.file {
            if !preset_only {
                return Err(McpError::new("INVALID_ARGUMENTS", "use file alone"));
            }
            return self.look_at_file(file);
        }
        if input.preset.is_some() || input.presets {
            if !preset_only {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "use preset or presets alone",
                ));
            }
            return match input.preset {
                Some(file) => self.inspect_preset(&self.project_at(None)?.1, &file),
                None => self.list_presets(),
            };
        }
        if input.media.is_some() && input.sequence.is_some() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "`sequence` chooses program frames or what a sequence says; not with `media`",
            ));
        }
        if input.cost {
            if input.media.is_some() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "`cost` is of program frames, not with `media`",
                ));
            }
            if input.at.is_empty() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "`cost` is of the frames at `at`: pass the positions to read",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self.frame_costs(&project, input.sequence.as_deref(), revision, &input.at);
        }
        if !input.sync.is_empty() {
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self.line_up(&project, revision, &input.sync);
        }
        if input.track.is_some() && (input.media.is_some() || !input.at.is_empty()) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "track follows the program over `from`–`to`; not with `media` or `at`",
            ));
        }
        let ranged = input.transcript
            || input.sheet.is_some()
            || input.media.is_some()
            || input.track.is_some()
            || input.contrast;
        if !ranged && (input.from.is_some() || input.to.is_some()) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "from and to choose a range for `transcript`, `sheet`, `contrast`, or a media's onsets",
            ));
        }
        if let Some(count) = input.sheet {
            if !(2..=MAX_SHEET_FRAMES).contains(&count) {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("a sheet has 2 to {MAX_SHEET_FRAMES} frames"),
                ));
            }
            if input.transcript || !input.at.is_empty() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "pass one of `sheet`, `at`, or `transcript`",
                ));
            }
        }
        if input.media.is_none() && input.transcript && !input.at.is_empty() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "pass `at` for frames or `transcript` for what is said, not both",
            ));
        }
        if input.words && !input.transcript {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "words times a transcript; pass transcript: true",
            ));
        }
        if input.scopes
            && (input.at.is_empty()
                || input.media.is_some()
                || input.transcript
                || input.sheet.is_some())
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "scopes read the program at `at`; pass `at`, and not `media`, `transcript`, or `sheet`",
            ));
        }
        if input.facts && input.media.is_none() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "facts are of a media; pass `media`",
            ));
        }
        if input.contrast
            && (input.media.is_some()
                || !input.at.is_empty()
                || input.transcript
                || input.sheet.is_some()
                || input.track.is_some()
                || input.scopes)
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "contrast reads the sequence's text; pass only `from`, `to`, `sequence`, or `proposal`",
            ));
        }
        if input.at.len() > MAX_LOOK_FRAMES {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("look shows at most {MAX_LOOK_FRAMES} frames at a time"),
            ));
        }
        if input.media.is_none()
            && input.at.is_empty()
            && !input.transcript
            && !input.contrast
            && input.track.is_none()
            && input.sheet.is_none()
            && input.sequence.is_none()
            && input.proposal.is_none()
        {
            let (revision, document) = self.runtime.get_document(true)?;
            let state = self.runtime.get_state()?;
            let project = parse(&document).ok();
            let unplaced = project
                .as_ref()
                .map_or_else(Vec::new, |project| self.unplaced(project));
            let mut result = json!({
                "revision": revision,
                "document": document,
                "proposal": proposals_status(&state),
                "unplaced": unplaced,
            });
            if let Some(project) = project {
                let to_make = self.to_make(&project, false);
                if !to_make.is_empty() {
                    result["toMake"] = json!(to_make);
                }
            }
            // The models a recipe's `generate` can name: those the person has set up, each
            // with what their file says it takes. Nothing is run to learn it.
            match kerf_engine::recipe::model::Models::load() {
                Ok(models) => {
                    let listed = models_listed(&models);
                    if !listed.is_empty() {
                        result["models"] = json!(listed);
                    }
                }
                // A file that cannot be read would refuse a making too: the person mends it.
                Err(error) => result["modelsError"] = json!(error.to_string()),
            }
            return Ok(result);
        }
        let (revision, project) = self.project_at(input.proposal.as_deref())?;
        let default_width = if input.sheet.is_some() {
            DEFAULT_SHEET_TILE_WIDTH
        } else {
            DEFAULT_FRAME_WIDTH
        };
        let width = input.width.unwrap_or(default_width).clamp(64, 1920);
        let mut result = match input.media.as_deref() {
            None if input.contrast => self.program_contrast(&project, revision, &input)?,
            None if input.track.is_some() => self.program_track(&project, revision, &input)?,
            None if input.transcript => self.program_transcript(&project, revision, &input)?,
            None if input.sheet.is_some() => {
                self.program_sheet(&project, revision, &input, width)?
            }
            None if input.at.is_empty() => {
                let mut shown = json!({
                    "revision": revision,
                    "document": print_document(&project, catalog(), PrintOptions { annotate: true }),
                });
                // What it would have made, as `look` with no arguments says of the project:
                // of a proposal, with what would refuse the making once it is accepted.
                let to_make = self.to_make(&project, input.proposal.is_some());
                if !to_make.is_empty() {
                    shown["toMake"] = json!(to_make);
                }
                shown
            }
            None if input.scopes => {
                self.program_scopes(&project, input.sequence.as_deref(), revision, &input.at)?
            }
            None => self.program_frames(
                &project,
                input.sequence.as_deref(),
                revision,
                &input.at,
                width,
            )?,
            Some(media_id) => {
                let media = project.media(media_id).ok_or_else(|| {
                    McpError::new(
                        "UNKNOWN_MEDIA",
                        format!(
                            "no media `{media_id}` in the project{}",
                            self.in_pending(media_id, "proposal")
                        ),
                    )
                })?;
                if input.sheet.is_some() {
                    self.source_sheet(media, &input, width)?
                } else if input.at.is_empty() {
                    self.media_index(&project, media, &input)?
                } else {
                    self.source_frames(media, &input.at, width)?
                }
            }
        };
        if let Some(id) = input.proposal {
            result["proposal"] = json!(id);
        }
        Ok(result)
    }

    fn media_index(
        &self,
        project: &Project,
        media: &Media,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let clock = Clock::of_media(media);
        let mut uses = Vec::new();
        for sequence in &project.sequences {
            let at = Clock::of_sequence(sequence);
            for track in &sequence.tracks {
                for clip in &track.clips {
                    let Content::Media { media: id, .. } = &clip.content else {
                        continue;
                    };
                    if *id != media.id {
                        continue;
                    }
                    let (source_in, source_out) = clip.source_range().unwrap_or_default();
                    uses.push(json!({
                        "clip": clip.id.to_string(),
                        "sequence": sequence.id.to_string(),
                        "track": track.id.to_string(),
                        "at": at.print_position(clip.start),
                        "end": at.print_position(clip.end()),
                        "src": clock.print_position(source_in),
                        "out": clock.print_position(source_out),
                    }));
                }
            }
        }
        let path = resolve_project_source_path(&self.dir, &media.path);
        let mut result = json!({
            "media": media.id.to_string(),
            "path": media.path,
            "start": clock.print_position(Time::ZERO),
            "end": clock.print_position(media.duration),
            "uses": uses,
        });
        if let Some(takes) = self.takes(project, media) {
            result["recipe"] = json!(media.recipe);
            result["takes"] = takes;
        }
        let index = kerf_engine::media_index(
            &path,
            &DerivativeCache::new(&self.dir),
            &kerf_engine::CancellationToken::new(),
        );
        let index_for_facts = index.as_ref().ok().cloned();
        match index {
            Ok(index) => {
                let at = |seconds: f64| print_seconds(&clock, seconds);
                result["cuts"] = json!(index.cuts.iter().map(|cut| at(*cut)).collect::<Vec<_>>());
                result["pauses"] = json!(index
                    .pauses
                    .iter()
                    .map(|(from, to)| json!({"from": at(*from), "to": at(*to)}))
                    .collect::<Vec<_>>());
                result["loudest"] = json!(index
                    .peaks
                    .iter()
                    .map(|peak| json!({"at": at(peak.at), "level": format!("{} dB", peak.level)}))
                    .collect::<Vec<_>>());
                if let Some(tempo) = &index.tempo {
                    result["tempo"] = json!({
                        "bpm": tempo.bpm,
                        "firstBeat": at(tempo.first),
                        "confidence": tempo.confidence,
                    });
                }
                let (from, to) = self.media_range(media, &clock, input)?;
                let onsets: Vec<String> = index
                    .onsets
                    .iter()
                    .filter(|onset| (from..to).contains(*onset))
                    .map(|onset| at(*onset))
                    .collect();
                let ranged = input.from.is_some() || input.to.is_some();
                if !ranged && onsets.len() > MAX_INDEX_ONSETS {
                    result["onsets"] = json!(onsets[..MAX_INDEX_ONSETS]);
                    result["moreOnsets"] = json!(format!(
                        "{} more; pass from and to for those in a range",
                        onsets.len() - MAX_INDEX_ONSETS
                    ));
                } else {
                    result["onsets"] = json!(onsets);
                }
            }
            // A media a recipe makes has no file until it is made, which is no fault of its line.
            Err(_) if media.recipe.is_some() && !path.is_file() => {
                result["analysis"] = json!(format!(
                    "unavailable: `{}` is not made yet; a person makes it from its recipe with `kerf make {}`",
                    media.path, media.id
                ));
            }
            Err(error) => result["analysis"] = json!(format!("unavailable: {error}")),
        }
        if input.facts {
            let cache = DerivativeCache::new(&self.dir);
            let at = |seconds: f64| print_seconds(&clock, seconds);
            result["facts"] = match index_for_facts {
                Some(index) => crate::media_facts::sheet(&path, media, &index, &cache, &at),
                None => json!({"unmeasured": "its index could not be made"}),
            };
        }
        if input.transcript {
            result["transcript"] = self.transcript(&path, media, &clock, input)?;
        }
        Ok(result)
    }

    /// `from`–`to` of `media` in seconds, all of it by default.
    fn media_range(
        &self,
        media: &Media,
        clock: &Clock,
        input: &LookInput,
    ) -> Result<(f64, f64), McpError> {
        let from = match &input.from {
            Some(text) => position(clock, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => position(clock, text)?.min(media.duration),
            None => media.duration,
        };
        if from.is_negative() || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("choose a range inside media `{}`", media.id),
            ));
        }
        Ok((from.seconds(), to.seconds()))
    }

    /// A contact sheet of `media`: `input.sheet` frames spread over its
    /// range, from its proxy when one is made and wide enough.
    fn source_sheet(
        &self,
        media: &Media,
        input: &LookInput,
        width: u32,
    ) -> Result<Value, McpError> {
        if media.video.is_none() {
            return Err(McpError::new(
                "NO_VIDEO",
                format!("media `{}` has no picture", media.id),
            ));
        }
        let clock = Clock::of_media(media);
        let (from, to) = self.media_range(media, &clock, input)?;
        let count = input.sheet.unwrap_or(MAX_SHEET_FRAMES);
        let times: Vec<f64> = (0..count)
            .map(|index| {
                let time =
                    Time::from_seconds(from + (to - from) * (index as f64 + 0.5) / count as f64)
                        .unwrap_or(Time::ZERO);
                media.rate.round(time).seconds()
            })
            .collect();
        let grabbed = kerf_engine::cached_exact_frames(
            &self.viewable(media, width).to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &times,
            width,
        )
        .map_err(engine_error)?;
        let paths: Vec<String> = grabbed
            .frames
            .iter()
            .map(|frame| frame.path.clone())
            .collect();
        let sheet = Path::new(&paths[0]).with_file_name("sheet.jpg");
        if !sheet.is_file() {
            kerf_engine::create_contact_sheet(&paths, &sheet, SHEET_COLUMNS.min(count))
                .map_err(engine_error)?;
        }
        let mut result = json!({
            "media": media.id.to_string(),
            "frames": times.iter().map(|time| print_seconds(&clock, *time)).collect::<Vec<_>>(),
            "path": sheet,
        });
        inline_image(&mut result)?;
        Ok(result)
    }

    /// A contact sheet of the sequence: `input.sheet` program frames spread
    /// over its range.
    fn program_sheet(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
        width: u32,
    ) -> Result<Value, McpError> {
        let sequence = program(project, input.sequence.as_deref())?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let from = match &input.from {
            Some(text) => position(&clock, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => position(&clock, text)?.min(duration),
            None => duration,
        };
        if from.is_negative() || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("choose a range inside sequence `{}`", sequence.id),
            ));
        }
        let count = input.sheet.unwrap_or(MAX_SHEET_FRAMES);
        let span = (to - from).seconds();
        let mut times: Vec<Time> = (0..count)
            .map(|index| {
                let offset = Time::from_seconds(span * (index as f64 + 0.5) / count as f64)
                    .unwrap_or(Time::ZERO);
                sequence
                    .rate
                    .round(from + offset)
                    .min(duration - sequence.rate.frame_duration())
            })
            .collect();
        times.dedup();
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let capture = self.capture_frames(
            &resolved,
            sequence.id.as_str(),
            &times,
            width,
            Some(SHEET_COLUMNS.min(times.len())),
        )?;
        let mut result = json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "frames": times.iter().map(|time| clock.print_position(*time)).collect::<Vec<_>>(),
            "path": capture.sheet,
        });
        inline_image(&mut result)?;
        Ok(result)
    }

    /// The file to take frames of `media` from at `width`: its proxy when
    /// one is made and wide enough, or else the media itself.
    fn viewable(&self, media: &Media, width: u32) -> PathBuf {
        let path = resolve_project_source_path(&self.dir, &media.path);
        if width > kerf_engine::PROXY_WIDTH {
            return path;
        }
        kerf_engine::cached_proxy(&path, &DerivativeCache::new(&self.dir)).unwrap_or(path)
    }

    fn transcript(
        &self,
        path: &std::path::Path,
        media: &Media,
        clock: &Clock,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        if media.audio.is_none() {
            return Err(McpError::new(
                "NO_AUDIO",
                format!("media `{}` has no audio", media.id),
            ));
        }
        let from = match &input.from {
            Some(text) => position(clock, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => position(clock, text)?,
            None => media.duration,
        };
        let (from, to) = (from.seconds(), to.seconds().min(media.duration.seconds()));
        if to - from > kerf_engine::MAX_TRANSCRIPTION_RANGE_SECONDS + 1e-9 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "a transcript covers at most 15 minutes; choose a range with from and to",
            ));
        }
        let range = kerf_engine::TimeRange::new(from, to)
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
        let result = kerf_engine::transcribe_cached(
            &path.to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &kerf_engine::TranscriptionOptions::new(range),
            &kerf_engine::CancellationToken::new(),
            |_| {},
        )
        .map_err(engine_error)?;
        Ok(cues(&result.transcript, 0.0, clock, input.words))
    }

    /// What the sequence says from `from` to `to`, as it plays: its sound
    /// is rendered and transcribed. Times are sequence positions.
    fn program_transcript(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let sequence = program(project, input.sequence.as_deref())?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let from = match &input.from {
            Some(text) => position(&clock, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => position(&clock, text)?.min(duration),
            None => duration,
        };
        if from.is_negative() || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("choose a range inside sequence `{}`", sequence.id),
            ));
        }
        if (to - from).seconds() > kerf_engine::MAX_TRANSCRIPTION_RANGE_SECONDS + 1e-9 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "a transcript covers at most 15 minutes; choose a range with from and to",
            ));
        }
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        kerf_runtime::use_proxies(&mut resolved, &self.dir);
        let entry = self.program_entry(
            &resolved,
            sequence.id.as_str(),
            "program-sound",
            &json!({"from": from.flicks(), "to": to.flicks()}),
        )?;
        let sound = entry.path("sound.mp4");
        if !entry.is_complete(&["sound.mp4"]) {
            render_program::<fn(f64)>(
                &resolved,
                Some(sequence.id.as_str()),
                Some((from, to)),
                &RenderOptions {
                    output: sound.to_string_lossy().into_owned(),
                    draft: true,
                    force_software: true,
                    project_dir: Some(self.dir.clone()),
                    ..RenderOptions::default()
                },
                None,
            )
            .map_err(engine_error)?;
            entry.mark_complete(&["sound.mp4"]).map_err(engine_error)?;
        }
        // A program's sound is rendered for each revision listened to.
        let _ = DerivativeCache::new(&self.dir).keep_newest(&entry, KEPT_PROGRAM_SOUNDS);
        let range = kerf_engine::TimeRange::new(0.0, (to - from).seconds())
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
        let result = kerf_engine::transcribe_cached(
            &sound.to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &kerf_engine::TranscriptionOptions::new(range),
            &kerf_engine::CancellationToken::new(),
            |_| {},
        )
        .map_err(engine_error)?;
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "from": clock.print_position(from),
            "to": clock.print_position(to),
            "transcript": cues(&result.transcript, from.seconds(), &clock, input.words),
        }))
    }

    /// A cache entry for what `sequence` of `project`, whose media paths
    /// are resolved, renders to; refused when it cannot be rendered. Its
    /// key is the whole document, so each revision makes new ones: keep a
    /// few with [`DerivativeCache::keep_newest`].
    fn program_entry(
        &self,
        project: &Project,
        sequence: &str,
        kind: &str,
        options: &Value,
    ) -> Result<kerf_engine::derivatives::CacheEntry, McpError> {
        kerf_engine::render::renderable(project, Some(sequence))
            .map_err(|error| McpError::new("UNRENDERABLE", error.to_string()))?;
        let document = print_document(project, catalog(), PrintOptions::default());
        let identities = crate::qc::used_media(project)
            .map(|media| {
                SourceIdentity::from_path(std::path::Path::new(&media.path)).map_err(|error| {
                    McpError::new(
                        "MEDIA_UNAVAILABLE",
                        format!("media `{}` cannot be read: {error}", media.id),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut options = options.clone();
        options["sequence"] = json!(sequence);
        DerivativeCache::new(&self.dir)
            .project_entry(&json!(document), &identities, kind, &options)
            .map_err(engine_error)
    }

    fn source_frames(&self, media: &Media, at: &[String], width: u32) -> Result<Value, McpError> {
        if media.video.is_none() {
            return Err(McpError::new(
                "NO_VIDEO",
                format!("media `{}` has no picture", media.id),
            ));
        }
        let clock = Clock::of_media(media);
        let times = at
            .iter()
            .map(|text| {
                let time = position(&clock, text)?;
                if time.is_negative() || time >= media.duration {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        format!("`{text}` is outside media `{}`", media.id),
                    ));
                }
                Ok(time.seconds())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let grabbed = kerf_engine::cached_exact_frames(
            &self.viewable(media, width).to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &times,
            width,
        )
        .map_err(engine_error)?;
        let mut frames = Vec::new();
        for (text, frame) in at.iter().zip(grabbed.frames) {
            let mut value = json!({"at": text, "path": frame.path});
            inline_image(&mut value)?;
            frames.push(value);
        }
        Ok(json!({"media": media.id.to_string(), "frames": frames}))
    }

    fn program_frames(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        at: &[String],
        width: u32,
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let times = at
            .iter()
            .map(|text| {
                let time = position(&clock, text)?;
                if time.is_negative() || time >= duration {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        format!(
                            "`{text}` is outside sequence `{}`{}",
                            sequence.id,
                            self.any_pending("proposal")
                        ),
                    ));
                }
                Ok(time)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let capture = self.capture_frames(&resolved, sequence.id.as_str(), &times, width, None)?;
        let mut frames = Vec::new();
        for (text, path) in at.iter().zip(capture.frames) {
            let mut value = json!({"at": text, "path": path});
            inline_image(&mut value)?;
            frames.push(value);
        }
        Ok(json!({"sequence": sequence.id.to_string(), "revision": revision, "frames": frames}))
    }

    /// How recordings fall against each other by their sound, and a sequence with a camera on each
    /// track for them to be a multicam of.
    fn line_up(&self, project: &Project, revision: u64, ids: &[String]) -> Result<Value, McpError> {
        let invalid = |message: String| McpError::new("INVALID_ARGUMENTS", message);
        if ids.len() < 2 {
            return Err(invalid("`sync` lines up two or more media".to_owned()));
        }
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let mut media = Vec::with_capacity(ids.len());
        for id in ids {
            let found = resolved
                .media(id)
                .ok_or_else(|| invalid(format!("no media `{id}`")))?;
            if found.audio.is_none() {
                return Err(invalid(format!("`{id}` has no sound to line up by")));
            }
            media.push(found);
        }
        let paths: Vec<&std::path::Path> = media
            .iter()
            .map(|media| std::path::Path::new(&media.path))
            .collect();
        let found = kerf_engine::sync::align_sound(&paths)
            .map_err(|error| McpError::new("RENDER_FAILED", error.to_string()))?;
        let first = media[0];
        let rate = first.rate;
        // Times start with whichever began first.
        let earliest = found.iter().map(|a| a.offset).fold(0.0, f64::min);
        let frame_of = |seconds: f64| {
            rate.frames_round(
                kerf_model::Time::from_seconds(seconds).unwrap_or(kerf_model::Time::ZERO),
            )
        };
        let mut id = "multicam".to_owned();
        let mut number = 1;
        while project.sequence(&id).is_some() {
            number += 1;
            id = format!("multicam{number}");
        }
        let (width, height) = first
            .video
            .map_or((1920, 1080), |video| (video.width, video.height));
        let mut lines = vec![
            format!("sequence add {id} rate={rate} size={width}x{height} name=\"Multicam\""),
            format!("use {id}"),
        ];
        let mut rows = Vec::with_capacity(ids.len());
        for (index, (media, alignment)) in media.iter().zip(&found).enumerate() {
            let at = frame_of(alignment.offset - earliest);
            let camera = index + 1;
            let dur = rate.frames_round(media.duration).max(1);
            let mut targets = Vec::new();
            if media.video.is_some() {
                lines.push(format!("track add V{camera} video"));
                targets.push(format!("V{camera}"));
            }
            lines.push(format!("track add A{camera} audio"));
            targets.push(format!("A{camera}"));
            lines.push(format!(
                "overwrite media={} src=0f at={at}f dur={dur}f on={}",
                media.id,
                targets.join(",")
            ));
            rows.push(json!({
                "media": media.id.as_str(),
                "offset": (alignment.offset * 1000.0).round() / 1000.0,
                "at": format!("{at}f"),
                "confidence": (alignment.confidence * 100.0).round() / 100.0,
            }));
        }
        Ok(json!({
            "revision": revision,
            "recordings": rows,
            "sequence": id,
            "script": lines.join("\n"),
        }))
    }

    /// Where a point of the program's picture goes over `from`–`to`.
    fn program_track(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let Some(track) = &input.track else {
            return Err(McpError::new("INVALID_ARGUMENTS", "nothing to track"));
        };
        let sequence = program(project, input.sequence.as_deref())?;
        let clock = Clock::of_sequence(sequence);
        let (Some(from), Some(to)) = (&input.from, &input.to) else {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "track needs `from` and `to`",
            ));
        };
        let (from, to) = (position(&clock, from)?, position(&clock, to)?);
        let duration = sequence.duration();
        if from.is_negative() || to > duration || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("`from`–`to` is not a range of sequence `{}`", sequence.id),
            ));
        }
        let object = !track.inside.is_empty() || track.bbox.is_some();
        let point = match (track.x, track.y, track.corners, object, track.camera) {
            (Some(x), Some(y), None, false, false) => Some([x, y]),
            (None, None, Some(_), false, false)
            | (None, None, None, true, false)
            | (None, None, None, false, true) => None,
            _ => {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "give `x` and `y` to follow a point, `corners` to follow a surface, `inside` or `box` to follow an object, or `camera` to solve the camera",
                ))
            }
        };
        if let Some(fov) = track.fov {
            if !track.camera || !(1.0..=170.0).contains(&fov) {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "`fov` goes with `camera`, from 1 to 170 degrees",
                ));
            }
        }
        if !object && (!track.outside.is_empty() || track.model.is_some()) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "`outside` and `model` go with `inside` or `box`",
            ));
        }
        let within = |value: &f64| (0.0..=1.0).contains(value);
        let all_within = point
            .iter()
            .chain(track.corners.iter().flatten())
            .chain(&track.inside)
            .chain(&track.outside)
            .flatten()
            .chain(track.bbox.iter().flatten())
            .all(within);
        if !all_within {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "`x` and `y` run 0 to 1 across the frame",
            ));
        }
        let rate = sequence.rate;
        let frame = rate.frame_duration().flicks();
        let first = from.flicks().div_euclid(frame);
        let count = (to.flicks() - from.flicks()).div_euclid(frame).max(1);
        if count > MAX_TRACK_FRAMES {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("track follows at most {MAX_TRACK_FRAMES} frames at a time"),
            ));
        }
        let target = match &track.clip {
            Some(id) => Some(
                sequence
                    .tracks
                    .iter()
                    .flat_map(|track| &track.clips)
                    .find(|clip| clip.id.as_str() == id)
                    .ok_or_else(|| {
                        McpError::new(
                            "INVALID_ARGUMENTS",
                            format!("no clip `{id}` in the sequence"),
                        )
                    })?,
            ),
            None => None,
        };
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let scale = (SCOPE_WIDTH / f64::from(sequence.width.max(1))).min(1.0);
        let id = sequence.id.as_str();
        // One drawer for the run: its frames are decoded once through. One made for each
        // frame decoded each from its media's last keyframe again, and following a point for
        // five seconds of a clip with one keyframe took 70 s.
        let drawer = kerf_engine::still::drawer(&resolved, Some(id))
            .map_err(|error| McpError::new("RENDER_FAILED", error.to_string()))?;
        let read = |index: usize| {
            let at = rate.time_of(first + index as i64);
            let at = if at >= duration { to } else { at };
            let frame = drawer.frame(at, scale);
            Some((frame.width as usize, frame.height as usize, frame.rgba))
        };
        let drawn_width = f64::from(sequence.width.max(1)) * scale;
        // Times on the clip that will follow, or from `from` without one.
        let origin = target.map_or(first, |clip| clip.start.flicks().div_euclid(frame));
        if track.camera {
            // Up to `to`, and up to the last frame there is when that is the end of the sequence.
            let frames = if to >= duration { count } else { count + 1 } as usize;
            let solved = kerf_track::camera::solve(frames, track.fov, read);
            let mut result = json!({
                "sequence": sequence.id.to_string(),
                "revision": revision,
                "frames": frames,
            });
            match solved {
                Ok(shot) => {
                    solved_camera(&shot, sequence, target, (first, origin), scale, &mut result)
                }
                Err(why) => {
                    result["solved"] = json!(false);
                    result["reason"] = json!(why.to_string());
                }
            }
            return Ok(result);
        }
        if object {
            let model = match track.model.as_deref() {
                None | Some(kerf_segment::CLASSICAL) => None,
                Some(id) => Some(trained_model(id)?),
            };
            let hints = kerf_track::Hints {
                inside: track.inside.clone(),
                outside: track.outside.clone(),
                bbox: track.bbox,
            };
            let followed =
                kerf_track::follow_object(count as usize + 1, &hints, model.as_deref(), read);
            let kept =
                kerf_track::simplified_object(&followed, TRACK_TOLERANCE_PIXELS / drawn_width);
            let rounded = |value: f64| (value * 10_000.0).round() / 10_000.0;
            // How soft its edge is, in the pixels of what the mask goes on, and said only of an
            // object that has one.
            let pixels = picture_width(project, target).unwrap_or(f64::from(sequence.width));
            let softly = |frame: usize, corner: usize| {
                (followed.soft[frame][corner] * pixels * 10.0).round() / 10.0
            };
            let any_soft = followed
                .soft
                .iter()
                .flatten()
                .any(|soft| soft * pixels >= 0.05);
            let words: Vec<String> = kept
                .iter()
                .map(|index| {
                    let corners: Vec<String> = followed.outlines[*index]
                        .iter()
                        .enumerate()
                        .map(|(at, corner)| {
                            let (x, y) = (rounded(corner[0]), rounded(corner[1]));
                            if any_soft {
                                format!("{x},{y},{}", softly(*index, at))
                            } else {
                                format!("{x},{y}")
                            }
                        })
                        .collect();
                    format!("{}f:{}", first + *index as i64 - origin, corners.join(";"))
                })
                .collect();
            let outline = format!("{{{}}}", words.join(" "));
            let mut result = json!({
                "sequence": sequence.id.to_string(),
                "revision": revision,
                "frames": followed.outlines.len(),
                "lostAt": followed.lost_at.map(|index| format!("{}f", first + index as i64)),
                "keyframes": kept.len(),
                "area": followed.areas.last().map(|area| rounded(*area)),
                "outline": outline,
            });
            if let Some(clip) = target {
                result["script"] = json!(format!("mask {} add object points={outline}", clip.id));
            }
            return Ok(result);
        }
        if let Some(corners) = track.corners {
            let followed = kerf_track::follow_plane(count as usize + 1, corners, read);
            let kept =
                kerf_track::simplified_quads(&followed.quads, TRACK_TOLERANCE_PIXELS / drawn_width);
            let rounded = |value: f64| (value * 100_000.0).round() / 100_000.0;
            // One number of one corner over the kept frames, as keyframes.
            let channel = |corner: usize, axis: usize| {
                let words: Vec<String> = kept
                    .iter()
                    .map(|index| {
                        format!(
                            "{}f:{}",
                            first + *index as i64 - origin,
                            rounded(followed.quads[*index][corner][axis])
                        )
                    })
                    .collect();
                format!("{{{}}}", words.join(" "))
            };
            let words: Vec<String> = kept
                .iter()
                .map(|index| {
                    let corners: Vec<String> = followed.quads[*index]
                        .iter()
                        .map(|corner| format!("{},{}", rounded(corner[0]), rounded(corner[1])))
                        .collect();
                    format!("{}f:{}", first + *index as i64 - origin, corners.join(";"))
                })
                .collect();
            let mut result = json!({
                "sequence": sequence.id.to_string(),
                "revision": revision,
                "frames": followed.quads.len(),
                "lostAt": followed.lost_at.map(|index| format!("{}f", first + index as i64)),
                "quads": kept.iter().map(|index| json!({
                    "frame": first + *index as i64,
                    "corners": followed.quads[*index].map(|corner| corner.map(rounded)),
                    "agreeing": followed.agreeing[*index],
                })).collect::<Vec<_>>(),
                "outline": format!("{{{}}}", words.join(" ")),
            });
            if let Some(clip) = target {
                // Corner order, as the corner pin names them.
                let names = [
                    ("upper_left", 0),
                    ("upper_right", 1),
                    ("lower_right", 2),
                    ("lower_left", 3),
                ];
                let params: Vec<String> = names
                    .iter()
                    .map(|(name, corner)| {
                        format!(
                            "{name}_x={} {name}_y={}",
                            channel(*corner, 0),
                            channel(*corner, 1)
                        )
                    })
                    .collect();
                result["script"] = json!(format!(
                    "fx {} add distort.cornerpin@2 {}",
                    clip.id,
                    params.join(" ")
                ));
            }
            return Ok(result);
        }
        let point = point.ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "nothing to follow"))?;
        let followed = kerf_track::follow(count as usize + 1, point, read);
        let kept = kerf_track::simplified(&followed.points, TRACK_TOLERANCE_PIXELS / drawn_width);
        let keys = |value: &dyn Fn(usize) -> f64| {
            let words: Vec<String> = kept
                .iter()
                .map(|index| format!("{}f:{:.5}", first + *index as i64 - origin, value(*index)))
                .collect();
            format!("{{{}}}", words.join(" "))
        };
        let x = keys(&|index| followed.points[index].x - 0.5);
        let y = keys(&|index| followed.points[index].y - 0.5);
        let mut result = json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "frames": followed.points.len(),
            "lostAt": followed.lost_at.map(|index| format!("{}f", first + index as i64)),
            "points": kept.iter().map(|index| {
                let point = &followed.points[*index];
                json!({
                    "frame": first + *index as i64,
                    "x": (point.x * 10_000.0).round() / 10_000.0,
                    "y": (point.y * 10_000.0).round() / 10_000.0,
                    "confidence": (point.confidence * 100.0).round() / 100.0,
                })
            }).collect::<Vec<_>>(),
            "x": x,
            "y": y,
        });
        if let Some(clip) = target {
            result["script"] = json!(format!("set {} x={x} y={y}", clip.id));
        }
        Ok(result)
    }

    /// Which of the sequence's titles and captions are too faint against what is behind them.
    fn program_contrast(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let sequence = program(project, input.sequence.as_deref())?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let moment = |text: &Option<String>, otherwise: Time| match text {
            Some(text) => position(&clock, text),
            None => Ok(otherwise),
        };
        let (from, to) = (
            moment(&input.from, Time::ZERO)?,
            moment(&input.to, duration)?,
        );
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let mut findings = Vec::new();
        let (read, of) =
            crate::text_lint::contrast_of(&resolved, sequence, (from, to), |_| true, &mut findings);
        // The same frames say what the edge of the frame cuts off.
        let (clipped, findings): (Vec<_>, Vec<_>) = findings
            .into_iter()
            .partition(|finding| finding.code == "text.clipped");
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "read": read,
            "of": of,
            "clipped": clipped,
            "contrast": findings,
        }))
    }

    /// What scopes read of the program at each of `at`.
    fn program_scopes(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        at: &[String],
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        // Scopes read a few hundred samples across, so a smaller frame reads the same.
        let scale = (SCOPE_WIDTH / f64::from(sequence.width.max(1))).min(1.0);
        // An HDR sequence is read by the signal it masters to, and not what a monitor shows of it.
        let hdr = match sequence.color.curve() {
            kerf_color::Curve::Pq => Some(kerf_scopes::SignalCurve::Pq),
            kerf_color::Curve::Hlg => Some(kerf_scopes::SignalCurve::Hlg),
            _ => None,
        };
        let mut readings = Vec::new();
        for text in at {
            let time = position(&clock, text)?;
            if time.is_negative() || time >= duration {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("`{text}` is outside sequence `{}`", sequence.id),
                ));
            }
            let id = Some(sequence.id.as_str());
            let drawn = match hdr {
                Some(_) => kerf_engine::still::draw_signal(&resolved, id, time, scale),
                None => kerf_engine::still::draw_frame(&resolved, id, time, scale),
            };
            let frame = drawn.map_err(|error| McpError::new("RENDER_FAILED", error.to_string()))?;
            let (w, h) = (frame.width as usize, frame.height as usize);
            let reading = match hdr {
                Some(curve) => kerf_scopes::read_signal(&frame.rgba, w, h, curve),
                None => kerf_scopes::read(&frame.rgba, w, h),
            }
            .ok_or_else(|| McpError::new("RENDER_FAILED", "the frame came back empty"))?;
            readings.push(json!({"at": text, "scopes": reading}));
        }
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "readings": readings,
        }))
    }

    /// Render the program at each of `times` at full size and quality and
    /// take its frame there, at most `width` wide; with `columns`, also a
    /// contact sheet of them. `project` has its media paths resolved.
    /// Results are cached by document, media identity, and options.
    fn capture_frames(
        &self,
        project: &Project,
        sequence: &str,
        times: &[Time],
        width: u32,
        columns: Option<usize>,
    ) -> Result<Capture, McpError> {
        let entry = self.program_entry(
            project,
            sequence,
            "program-frames",
            &json!({
                "times": times.iter().map(|time| time.flicks()).collect::<Vec<_>>(),
                "maxWidth": width,
                "columns": columns,
            }),
        )?;
        let sequence = program(project, Some(sequence))?;
        let names: Vec<String> = (0..times.len())
            .map(kerf_engine::probe::frame_file_name)
            .collect();
        let mut required: Vec<&str> = names.iter().map(String::as_str).collect();
        if columns.is_some() {
            required.push("sheet.jpg");
        }
        if !entry.is_complete(&required) {
            let frame = sequence.rate.frame_duration();
            let duration = sequence.duration();
            // One frame long render of each time, so a frame never comes
            // from a draft's lower size or rate.
            let render = |index: usize| -> Result<(), McpError> {
                let start = times[index];
                let clip = entry.path(&format!("frame-{index:03}.mp4"));
                render_program::<fn(f64)>(
                    project,
                    Some(sequence.id.as_str()),
                    Some((start, (start + frame).min(duration))),
                    &RenderOptions {
                        output: clip.to_string_lossy().into_owned(),
                        force_software: true,
                        project_dir: Some(self.dir.clone()),
                        ..RenderOptions::default()
                    },
                    None,
                )
                .map_err(engine_error)?;
                let directory = entry.path(&format!("frame-{index:03}"));
                let grabbed = kerf_engine::probe::extract_exact_frames(
                    &clip.to_string_lossy(),
                    &[0.0],
                    &directory,
                    width,
                )
                .map_err(engine_error)?;
                fs::rename(&grabbed[0].path, entry.path(&names[index])).map_err(io_error)?;
                let _ = fs::remove_dir_all(directory);
                let _ = fs::remove_file(clip);
                Ok(())
            };
            let render = &render;
            thread::scope(|scope| {
                (0..times.len())
                    .map(|index| scope.spawn(move || render(index)))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .try_for_each(|handle| handle.join().expect("frame render panicked"))
            })?;
            if let Some(columns) = columns {
                let paths: Vec<String> = names
                    .iter()
                    .map(|name| entry.path(name).to_string_lossy().into_owned())
                    .collect();
                kerf_engine::probe::create_contact_sheet(
                    &paths,
                    &entry.path("sheet.jpg"),
                    columns.min(times.len()),
                )
                .map_err(engine_error)?;
            }
            entry.mark_complete(&required).map_err(engine_error)?;
        }
        // Every edit's preview captures two revisions.
        let _ = DerivativeCache::new(&self.dir).keep_newest(&entry, KEPT_PROGRAM_FRAMES);
        Ok(Capture {
            frames: names
                .iter()
                .map(|name| entry.path(name).to_string_lossy().into_owned())
                .collect(),
            sheet: columns.map(|_| entry.path("sheet.jpg").to_string_lossy().into_owned()),
        })
    }

    pub(super) fn edit(&self, arguments: Value) -> Result<Value, McpError> {
        // `revise` starts again from the project, so what its proposal added is not there either.
        let building_on = arguments.get("extend").is_some();
        self.run_edit(arguments).map_err(|mut error| {
            // What is not in the project may be in a proposal not built on.
            if !building_on && error.code == "SCRIPT_REFUSED" {
                let hint = ["no clip `", "unknown media `", "no clip or marker `"]
                    .iter()
                    .find_map(|before| {
                        let (_, rest) = error.message.split_once(before)?;
                        rest.split_once('`').map(|(name, _)| name.to_owned())
                    })
                    .map(|name| self.in_pending(&name, "extend"));
                if let Some(hint) = hint {
                    error.message.push_str(&hint);
                }
            }
            error
        })
    }

    fn run_edit(&self, arguments: Value) -> Result<Value, McpError> {
        let input: EditInput = input(arguments)?;
        if [
            input.script.is_some() || !input.recipes.is_empty(),
            input.preset.is_some(),
            input.save_preset.is_some(),
        ]
        .into_iter()
        .filter(|v| *v)
        .count()
            != 1
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "pass exactly one of script (with or without recipes), recipes, preset, or savePreset",
            ));
        }
        if let Some(file) = &input.save_preset {
            if input.extend.is_some()
                || input.revise.is_some()
                || !input.bindings.is_empty()
                || !input.texts.is_empty()
            {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "savePreset takes only sequence and name",
                ));
            }
            let (_, project) = self.project_at(None)?;
            return self.save_preset(
                &project,
                file,
                input.sequence.as_deref(),
                input.name.as_deref(),
            );
        }
        if input.name.is_some()
            || (input.script.is_some()
                && (input.sequence.is_some()
                    || !input.bindings.is_empty()
                    || !input.texts.is_empty()))
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "sequence, bindings and texts are for presets; name is for savePreset",
            ));
        }
        if input.extend.is_some() && input.revise.is_some() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "pass `extend` or `revise`, not both",
            ));
        }
        let replaced = input.extend.as_deref().or(input.revise.as_deref());
        let prior = match replaced {
            Some(id) => Some(self.pending(id)?),
            None => None,
        };
        // `extend` runs the script on what the proposal makes; `revise` on
        // the project.
        let (revision, before) = self.project_at(input.extend.as_deref())?;
        let source_script = match input.preset {
            Some(file) => self.preset_script(
                &before,
                &file,
                input.sequence.as_deref(),
                &input.bindings,
                &input.texts,
            )?,
            None => input.script.clone().unwrap_or_default(),
        };
        let facts = CachedFacts::new(&self.dir);
        let mut applied =
            apply_script_with(&before, catalog(), &facts, &source_script).map_err(refused)?;
        // A recipe the edit writes is kept beside the one before it, and the media that name it
        // are moved to it by lines added to the script, so that the proposal shows the move and
        // undo takes it back. The recipe itself is shown in the script, as comments.
        let recipes = self.recipe_versions(&input.recipes)?;
        let mut source_script = source_script;
        if !recipes.is_empty() {
            let (shown, moves) = recipe_lines(&recipes, &applied.project)?;
            source_script = format!("{shown}{}{moves}", with_newline(&source_script));
            applied =
                apply_script_with(&before, catalog(), &facts, &source_script).map_err(refused)?;
        }
        if print_document(&applied.project, catalog(), PrintOptions::default())
            == print_document(&before, catalog(), PrintOptions::default())
        {
            // A new recipe a media names already, which was not there: nothing in the document
            // changes, and nothing can be made of it until a person makes it.
            let written: Vec<&RecipeVersion> =
                recipes.iter().filter(|recipe| recipe.version.new).collect();
            if written.is_empty() {
                return Err(McpError::new(
                    "NO_CHANGE",
                    if recipes.is_empty() {
                        "the script changes nothing"
                    } else {
                        "the script changes nothing, and each recipe is what its file holds"
                    },
                ));
            }
            for recipe in &written {
                kerf_engine::recipe::write_version(&self.dir, &recipe.version, &recipe.text)
                    .map_err(engine_error)?;
            }
            return Ok(json!({
                "proposal": null,
                "status": "written",
                "recipes": recipes.iter().map(RecipeVersion::json).collect::<Vec<_>>(),
                "toMake": self.to_make(&applied.project, false),
            }));
        }
        // An extension runs the operations of the proposal it extends, which
        // read nothing that could have changed since, and then this script;
        // it keeps both scripts as they were written.
        let (script, written) = match prior.filter(|_| input.extend.is_some()) {
            Some((operations, source, _)) => {
                let operations = operations.trim_end();
                let script = format!("{operations}\n{source_script}");
                let (_, base) = self.project_at(None)?;
                let staged = apply_script_with(&base, catalog(), &facts, &script)
                    .map_err(|errors| refused(after(errors, operations.lines().count())))?;
                // Such as when the operations end on another sequence than
                // this script started on.
                if print_document(&staged.project, catalog(), PrintOptions::default())
                    != print_document(&applied.project, catalog(), PrintOptions::default())
                {
                    return Err(McpError::new(
                        "SCRIPT_REFUSED",
                        "after the operations of the proposal it extends, this script does \
                         something else than it did on its own; start it with `use <sequence>`",
                    ));
                }
                (
                    script,
                    Some(format!("{}\n{source_script}", source.trim_end())),
                )
            }
            None => (source_script, None),
        };
        // A revised or extended proposal becomes its next version, which
        // keeps the versions before it for the user to see.
        let author = self.author();
        let snapshot = self
            .runtime
            .stage_script_proposal_by(
                &operation_key("agent-edit"),
                revision,
                &script,
                written.as_deref(),
                &author,
                replaced,
            )
            .map_err(|error| match error.code.as_str() {
                "PROPOSAL_PENDING" => McpError::new(
                    "PROPOSAL_PENDING",
                    format!(
                        "{}; pass one's id as `revise` to replace it, or `extend` to build on it",
                        error.message
                    ),
                ),
                "PROPOSAL_NOT_PENDING" | "PROPOSAL_NOT_FOUND" => {
                    not_pending(replaced.unwrap_or_default())
                }
                _ => McpError::from(error),
            })?;
        // The runtime lists the proposal it staged first.
        let Some(staged) = snapshot
            .proposals
            .first()
            .filter(|entry| entry.status == "pending")
        else {
            return Err(McpError::new(
                "RUNTIME_ERROR",
                "the runtime did not keep the proposal",
            ));
        };
        let proposal_id = staged.proposal_id.clone();
        // Written once the proposal is staged, so that a refused edit leaves no file; the
        // document names a new version only once the proposal is accepted.
        for recipe in recipes.iter().filter(|recipe| recipe.version.new) {
            if let Err(error) =
                kerf_engine::recipe::write_version(&self.dir, &recipe.version, &recipe.text)
            {
                let _ = self.runtime.reject_proposal(&proposal_id);
                return Err(engine_error(error));
            }
        }
        let script = staged.script.clone();
        let document_diff = staged.document_diff.clone();
        let (version, supersedes) = (staged.version, staged.supersedes.clone());
        let (_, after_document) = self.runtime.proposal_documents(&proposal_id)?;
        let after_program = parse(&after_document)?;
        let mut before_program = before.clone();
        kerf_runtime::resolve_media_paths(&mut before_program, &self.dir);
        let check = crate::qc::check(&after_program, &self.dir).to_json();
        let preview = match changed_range(&before, &applied.project) {
            None => json!({"unavailable": "the edit changes nothing on the sequence"}),
            Some((sequence, from, to)) => self
                .preview(&sequence, from, to, &before_program, &after_program)
                .unwrap_or_else(|error| json!({"unavailable": error.message})),
        };
        let mut result = json!({
            "proposal": proposal_id,
            "version": version,
            "supersedes": supersedes,
            "author": author.name,
            "baseRevision": revision,
            "status": "awaiting_review",
            "script": script,
            "diff": document_diff,
            "check": check,
            "preview": preview,
        });
        if !recipes.is_empty() {
            result["recipes"] = json!(recipes.iter().map(RecipeVersion::json).collect::<Vec<_>>());
        }
        // What the person lets an agent's edit be without their review.
        match self.permissions() {
            Ok(permissions) if permissions.accepts_edits() => {
                let accepted = self.runtime.accept_proposal(
                    &proposal_id,
                    if author.kind == "mcp" { "mcp" } else { "agent" },
                    &operation_key("agent-accept"),
                    revision,
                );
                result["permissions"] = json!(permissions.mode.name());
                match accepted {
                    Ok(state) => {
                        result["status"] = json!("accepted");
                        result["revision"] = json!(state.revision);
                    }
                    Err(error) => {
                        result["notAccepted"] = json!(format!(
                            "{}; it waits for the person's review",
                            error.message
                        ));
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                result["permissionsError"] =
                    json!(format!("{error}: so it waits for the person's review"));
            }
        }
        // What a person would be asked to make, with what would refuse it: a recipe the edit
        // names is mended here, not after someone has tried to run it. A making is asked of
        // the project, so while this waits for review it says that the proposal comes first.
        let waiting = result["status"] == "awaiting_review";
        let to_make = self.to_make(&applied.project, waiting);
        if !to_make.is_empty() {
            result["toMake"] = json!(to_make);
        }
        Ok(result)
    }

    /// Where each recipe an edit writes is kept, read as a recipe first.
    fn recipe_versions(
        &self,
        recipes: &BTreeMap<String, String>,
    ) -> Result<Vec<RecipeVersion>, McpError> {
        recipes
            .iter()
            .map(|(asked, text)| {
                // A file ends with a line, as an editor writes one.
                let text = with_newline(text);
                let version = kerf_engine::recipe::version(&self.dir, asked, &text)
                    .map_err(|error| McpError::new("INVALID_RECIPE", error.to_string()))?;
                Ok(RecipeVersion {
                    asked: asked.clone(),
                    text,
                    version,
                })
            })
            .collect()
    }

    /// Contact sheets of what the edit changed in `from`–`to`, before and
    /// after: the frames either side of each cut it made or moved, and the
    /// middle of a clip it changed in place. `before` and `after` have their
    /// media paths resolved.
    fn preview(
        &self,
        sequence: &Sequence,
        from: Time,
        to: Time,
        before: &Project,
        after: &Project,
    ) -> Result<Value, McpError> {
        let clock = Clock::of_sequence(sequence);
        let mut result = json!({
            "sequence": sequence.id.to_string(),
            "from": clock.print_position(from),
            "to": clock.print_position(to),
        });
        for (side, project, other) in [("before", before, after), ("after", after, before)] {
            let times = preview_times(project, other, sequence.id.as_str());
            if times.is_empty() {
                result[side] = json!({"empty": true});
                continue;
            }
            let capture = self.capture_frames(
                project,
                sequence.id.as_str(),
                &times,
                PREVIEW_WIDTH,
                Some(PREVIEW_COLUMNS),
            )?;
            let mut sheet = json!({
                "frames": times.iter().map(|time| clock.print_position(*time)).collect::<Vec<_>>(),
                "path": capture.sheet,
            });
            inline_image(&mut sheet)?;
            result[side] = sheet;
        }
        Ok(result)
    }

    /// Start a render job of the project, or of a pending proposal, in the
    /// background and return it queued. It runs in this process, which
    /// waits for it or cancels it before exiting (see `finish_renders`).
    fn render_start(
        &self,
        draft: bool,
        range: Option<RenderRange>,
        proposal: Option<&str>,
        sequence: Option<&str>,
    ) -> Result<(RenderJob, PathBuf), McpError> {
        let (job, document, output) = self
            .runtime
            .start_render_range(
                if draft { "preview" } else { "full" },
                range,
                proposal,
                sequence,
            )
            .map_err(McpError::from)?;
        let runtime = self.runtime.clone();
        let worker_job = job.clone();
        let worker_output = output.clone();
        self.spawn_render(job.job_id.clone(), move || {
            let mut last_progress = -1.0_f64;
            let _ = kerf_runtime::run_render_job(
                &runtime,
                &worker_job,
                &document,
                &worker_output,
                draft,
                |progress| {
                    if progress < 1.0 && progress - last_progress < 0.01 {
                        return;
                    }
                    last_progress = progress;
                    let _ = runtime.update_job_with_progress(
                        &worker_job.job_id,
                        "running",
                        None,
                        None,
                        Some(progress),
                    );
                },
            );
        });
        Ok((job, output))
    }

    pub(super) fn render(&self, arguments: Value) -> Result<Value, McpError> {
        let input: RenderInput = input(arguments)?;
        if let Some(job) = input.job.as_deref() {
            if input.quality.is_some()
                || input.from.is_some()
                || input.to.is_some()
                || input.sequence.is_some()
                || input.proposal.is_some()
            {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "`job` reads a render; it takes no quality, range, sequence, or proposal",
                ));
            }
            let job = if input.cancel {
                self.runtime.cancel_render(job)?
            } else {
                self.runtime.render_job(job)?
            };
            return Ok(json!({
                "job": job.job_id,
                "status": job.status,
                "progress": job.progress,
                "output": job.output_path,
                "error": job.error,
            }));
        }
        if input.cancel {
            return Err(McpError::new("INVALID_ARGUMENTS", "cancel needs `job`"));
        }
        let draft = match input.quality.as_deref() {
            None | Some("draft") => true,
            Some("full") => false,
            Some(other) => {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("quality `{other}` is not draft or full"),
                ))
            }
        };
        let (_, project) = self.project_at(input.proposal.as_deref())?;
        let sequence = program(&project, input.sequence.as_deref())?;
        if !sequence.duration().is_positive() {
            return Err(McpError::new(
                "EMPTY_SEQUENCE",
                format!(
                    "sequence `{}` is empty{}",
                    sequence.id,
                    self.any_pending("proposal")
                ),
            ));
        }
        let mut range = None;
        if input.from.is_some() || input.to.is_some() {
            let clock = Clock::of_sequence(sequence);
            let from = match &input.from {
                Some(text) => position(&clock, text)?,
                None => Time::ZERO,
            };
            let to = match &input.to {
                Some(text) => position(&clock, text)?,
                None => sequence.duration(),
            };
            range = Some(kerf_runtime::RenderRange::new(
                from.seconds(),
                (to - from).seconds(),
            )?);
        }
        let (job, output) = self.render_start(
            draft,
            range,
            input.proposal.as_deref(),
            input.sequence.as_deref(),
        )?;
        let mut result = json!({
            "job": job.job_id,
            "status": job.status,
            "output": output,
            "sequence": sequence.id.to_string(),
            "revision": self.runtime.get_state()?.revision,
        });
        if let Some(id) = input.proposal {
            result["proposal"] = json!(id);
        }
        Ok(result)
    }
}

/// How many renders of a program's sound (each up to 15 minutes, with its
/// transcript) the cache keeps besides the one in use; older revisions'
/// make way.
const KEPT_PROGRAM_SOUNDS: usize = 4;
/// How many captures of a program's frames the cache keeps besides the one
/// in use.
const KEPT_PROGRAM_FRAMES: usize = 32;

/// A script refusal, with each line numbered as the agent wrote it.
/// The most catalog entries `look` with `effects` returns.
/// What `look` says of a solved shot: its camera as keyframes, how well it fits, and where to pin
/// things. `first` is the frame the range starts on and `origin` the one keyframes count from.
/// `scale` is how much smaller than the sequence's frames it was read.
fn solved_camera(
    shot: &kerf_track::camera::SolvedShot,
    sequence: &Sequence,
    target: Option<&Clip>,
    (first, origin): (i64, i64),
    scale: f64,
    result: &mut Value,
) {
    let kept = shot.simplified(TRACK_TOLERANCE_PIXELS);
    let rounded = |value: f64, places: i32| {
        let by = 10f64.powi(places);
        (value * by).round() / by
    };
    let keys = |value: &dyn Fn(&kerf_track::camera::SolvedCamera) -> f64, places: i32| {
        let words: Vec<String> = kept
            .iter()
            .map(|index| {
                let camera = &shot.cameras[*index];
                format!(
                    "{}f:{}",
                    first + camera.frame as i64 - origin,
                    rounded(value(camera), places)
                )
            })
            .collect();
        format!("{{{}}}", words.join(" "))
    };
    let fov = rounded(shot.fov, 3);
    let lens = if shot.zooms {
        keys(&|camera| camera.fov, 3)
    } else {
        fov.to_string()
    };
    let params = format!(
        "x={} y={} z={} pan={} tilt={} roll={} fov={lens}",
        keys(&|camera| camera.x, 5),
        keys(&|camera| camera.y, 5),
        keys(&|camera| camera.z, 5),
        keys(&|camera| camera.pan, 4),
        keys(&|camera| camera.tilt, 4),
        keys(&|camera| camera.roll, 4),
    );
    let script = match target {
        Some(clip) => format!("set {} {params}", clip.id),
        None => {
            // A new camera on a new track, which is on top and so is the one that sees.
            let free = |taken: &dyn Fn(&str) -> bool, stem: &str| {
                (1..)
                    .map(|index| format!("{stem}{index}"))
                    .find(|name| !taken(name))
                    .unwrap_or_default()
            };
            let on = free(&|name| sequence.track(name).is_some(), "V");
            let id = free(
                &|name| {
                    sequence
                        .tracks
                        .iter()
                        .any(|track| track.clip(name).is_some())
                },
                "camera",
            );
            let dur = shot.cameras.last().map_or(1, |camera| camera.frame + 1);
            format!(
                "track add {on} video\noverwrite gen=camera@1 at={first}f dur={dur}f on={on} as={id}\nset {id} {params}"
            )
        }
    };
    result["solved"] = json!(true);
    result["solvedFrames"] = json!(shot.cameras.len());
    result["lostAt"] = json!(shot
        .ended_at
        .map(|index| format!("{}f", first + index as i64)));
    result["fov"] = json!(fov);
    result["lensKnown"] = json!(shot.lens_known);
    result["zooms"] = json!(shot.zooms);
    if shot.zoom_or_push {
        result["zoomOrPush"] = json!(
            "This shot cannot tell a zoom from a push in: what is in the picture grows alike, which \
             either would do. It is written as the camera moving along its line with one lens, and \
             if the lens zoomed that way is not true."
        );
    }
    result["fit"] = json!({
        "error": rounded(shot.error / scale, 3),
        "worst": rounded(shot.worst / scale, 3),
        "agreeing": rounded(shot.agreeing, 3),
    });
    result["keyframes"] = json!(kept.len());
    result["points"] = json!(shot
        .spread_points(MOST_PINS)
        .iter()
        .map(|point| json!({
            "x": rounded(point.x, 5),
            "y": rounded(point.y, 5),
            "z": rounded(point.z, 5),
            "frame": first + point.frame as i64,
            "at": [rounded(point.seen_at[0], 4), rounded(point.seen_at[1], 4)],
            "lastFrame": first + point.last_frame as i64,
        }))
        .collect::<Vec<_>>());
    result["script"] = json!(script);
}

/// The most frames one `track` follows.
const MAX_TRACK_FRAMES: i64 = 1800;

/// The most places in the world a solved camera offers to pin to.
const MOST_PINS: usize = 24;

/// How far a followed point may stray from the keyframes it is written with, in pixels of the frame
/// it was followed on: finer than that is the tracker's own jitter.
/// How many pixels wide the picture of `clip` is, if it has one of its own.
fn picture_width(project: &kerf_model::Project, clip: Option<&kerf_model::Clip>) -> Option<f64> {
    let kerf_model::Content::Media { media, .. } = &clip?.content else {
        return None;
    };
    let media = project.media.iter().find(|found| found.id == *media)?;
    Some(f64::from(media.video.as_ref()?.width))
}

const TRACK_TOLERANCE_PIXELS: f64 = 0.5;

/// The width frames are drawn at for scopes to read.
const SCOPE_WIDTH: f64 = 960.0;

const MAX_FOUND_FX: usize = 20;

/// Catalog entries matching `words`, by how many of the words their id,
/// category, and doc contain, then in catalog order.
fn find_fx(words: &str, area: Option<kerf_model::Area>) -> Vec<Value> {
    let words: Vec<String> = words
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut found: Vec<(usize, &kerf_model::FxDef)> = catalog()
        .all()
        .filter(|fx| area.is_none_or(|area| fx.area() == area))
        .filter_map(|fx| {
            // What its parameters are for counts too: a light is what casts a shadow.
            let params: String = fx
                .params
                .iter()
                .map(|param| format!(" {} {}", param.name, param.doc))
                .collect();
            let haystack = format!("{} {} {}{params}", fx.id, fx.category, fx.doc).to_lowercase();
            let whole: Vec<&str> = haystack
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .collect();
            // A word found whole counts twice what one found inside another
            // does: `glow` over `glowing`.
            let score: usize = words
                .iter()
                .map(|word| {
                    if whole.contains(&word.as_str()) {
                        2
                    } else if haystack.contains(word.as_str()) {
                        1
                    } else {
                        0
                    }
                })
                .sum();
            (score > 0).then_some((score, fx))
        })
        .collect();
    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found
        .into_iter()
        .take(MAX_FOUND_FX)
        .map(|(_, fx)| {
            json!({
                "id": fx.id.as_str(),
                "kind": fx.kind.name(),
                "category": fx.category,
                "area": fx.area().name(),
                "doc": fx.doc,
            })
        })
        .collect()
}

/// The clip properties, of pictures and of sound.
fn clip_properties() -> impl Iterator<Item = &'static kerf_model::ParamDef> {
    [kerf_model::Domain::Video, kerf_model::Domain::Audio]
        .into_iter()
        .flat_map(|domain| catalog().props(domain).iter())
}

/// How many clip properties `effects` gives.
const MAX_FOUND_PROPERTIES: usize = 6;

/// The clip properties whose name or guidance match `words`, best first: what no effect does, such as a
/// `bevel`, is found the way an effect is.
fn find_properties(words: &str) -> Vec<Value> {
    let words: Vec<String> = words
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut found: Vec<(usize, &kerf_model::ParamDef)> = clip_properties()
        .filter_map(|param| {
            let haystack = format!("{} {}", param.name, param.doc).to_lowercase();
            let score = words
                .iter()
                .filter(|word| haystack.contains(word.as_str()))
                .count();
            (score > 0).then_some((score, param))
        })
        .collect();
    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found
        .into_iter()
        .take(MAX_FOUND_PROPERTIES)
        .map(|(_, param)| json!({"id": param.name, "reference": kerf_lang::property_reference(param)}))
        .collect()
}

/// The models set up, as `look` lists them: each by the name a `generate` line gives it,
/// with what the person's models file says it takes. `size` is written as a line writes it,
/// and `guide` is the `render` line that draws a guide for it, with no more than it reads.
fn models_listed(models: &kerf_engine::recipe::model::Models) -> Vec<Value> {
    models
        .names()
        .map(|name| {
            let mut listed = json!({"name": name});
            let Some(takes) = models.takes(name) else {
                return listed;
            };
            if let Some(about) = &takes.about {
                listed["about"] = json!(about);
            }
            let size = takes
                .size
                .map(|(width, height)| format!("{width}x{height}"));
            let rate = takes.rate.map(fps_text);
            if let Some(size) = &size {
                listed["size"] = json!(size);
            }
            if let Some(rate) = takes.rate {
                listed["rate"] = json!(rate);
            }
            if let Some(seconds) = takes.seconds {
                listed["seconds"] = json!(seconds);
            }
            if let Some(reads) = &takes.reads {
                listed["reads"] = json!(reads);
            }
            if !takes.more.is_empty() {
                listed["more"] = json!(takes.more);
            }
            for (name, value) in &takes.other {
                // What the file says under a name of its own never replaces one of these.
                if listed.get(name).is_none() && name != "guide" {
                    listed[name] = value.clone();
                }
            }
            if takes.follows_a_guide() == Some(true) && (size.is_some() || rate.is_some()) {
                let mut line = "render <sequence> {guide.mp4}".to_owned();
                if let Some(size) = size {
                    line.push_str(&format!(" size={size}"));
                }
                if let Some(rate) = rate {
                    line.push_str(&format!(" rate={rate}"));
                }
                listed["guide"] = json!(line);
            }
            listed
        })
        .collect()
}

/// Whether a file's extension says it is footage: what an import takes.
fn is_footage(path: &std::path::Path) -> bool {
    const FOOTAGE: [&str; 16] = [
        "mp4", "mov", "m4v", "mkv", "webm", "avi", "mxf", "mts", "wav", "mp3", "m4a", "aac",
        "flac", "png", "jpg", "jpeg",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| FOOTAGE.contains(&extension.to_lowercase().as_str()))
}

/// How many frames a file `seconds` long has at `fps`, as a making and an import count them
/// (`kerf-runtime`'s `media_from_probe`): its length to the nearest frame of its rate, and
/// never none. A container says a length a hair under its frames' as often as not, 4.062012 s
/// for 65 frames at 16 a second, and counted down that is a frame short of the file.
fn frames_of(seconds: f64, fps: f64) -> i64 {
    let counted = kerf_model::Rate::from_fps(fps)
        .ok()
        .zip(Time::from_seconds(seconds))
        .map(|(rate, length)| rate.frames_round(length));
    counted
        .unwrap_or_else(|| (seconds * fps).round() as i64)
        .max(1)
}

/// A frame rate as a document writes it.
fn fps_text(fps: f64) -> String {
    let whole = fps.round();
    if (fps - whole).abs() < 1e-3 {
        format!("{whole}")
    } else {
        format!("{:.3}", fps)
    }
}

fn refused(errors: Vec<kerf_model::Diagnostic>) -> McpError {
    McpError::new("SCRIPT_REFUSED", diagnostics(&errors))
}

/// `errors` of a script run after `lines` lines of other script, with
/// their lines counted in that script.
fn after(mut errors: Vec<kerf_model::Diagnostic>, lines: usize) -> Vec<kerf_model::Diagnostic> {
    for error in &mut errors {
        if let Some(span) = error.span.as_mut() {
            span.line = span.line.saturating_sub(lines);
        }
    }
    errors
}

/// The first sequence that differs between two projects, and the span of
/// it that differs: every clip added, removed, or changed, and the
/// transitions around them. A change to a track itself (enabled, muted)
/// spans the whole sequence.
fn changed_range(before: &Project, after: &Project) -> Option<(Sequence, Time, Time)> {
    after.sequences.iter().find_map(|sequence| {
        let (from, to) = changed_span(sequence, before.sequence(sequence.id.as_str()))?;
        Some((sequence.clone(), from, to))
    })
}

/// The span of `sequence` that differs from `old`, what it was.
fn changed_span(sequence: &Sequence, old: Option<&Sequence>) -> Option<(Time, Time)> {
    let mut span: Option<(Time, Time)> = None;
    let mut extend = |from: Time, to: Time| {
        span = Some(match span {
            None => (from, to),
            Some((start, end)) => (start.min(from), end.max(to)),
        });
    };
    let whole = sequence
        .duration()
        .max(old.map_or(Time::ZERO, Sequence::duration));
    for track in &sequence.tracks {
        let Some(previous) = old.and_then(|old| old.track(track.id.as_str())) else {
            extend(Time::ZERO, whole);
            continue;
        };
        if (track.enabled, track.muted, track.kind)
            != (previous.enabled, previous.muted, previous.kind)
        {
            extend(Time::ZERO, whole);
        }
        for clip in &track.clips {
            match previous.clip(clip.id.as_str()) {
                Some(was) if was == clip => {}
                Some(was) => {
                    extend(was.start, was.end());
                    extend(clip.start, clip.end());
                }
                None => extend(clip.start, clip.end()),
            }
        }
        for was in &previous.clips {
            if track.clip(was.id.as_str()).is_none() {
                extend(was.start, was.end());
            }
        }
        let transitions = track.transitions.iter().chain(&previous.transitions);
        for transition in transitions {
            let same = track.transition(transition.id.as_str())
                == previous.transition(transition.id.as_str());
            if same {
                continue;
            }
            for clip in [&transition.from, &transition.to] {
                if let Some(clip) = track.clip(clip.as_str()).or(previous.clip(clip.as_str())) {
                    extend(clip.start, clip.end());
                }
            }
        }
    }
    if let Some(old) = old {
        for track in &old.tracks {
            if sequence.track(track.id.as_str()).is_none() {
                extend(Time::ZERO, whole);
            }
        }
    }
    let (from, to) = span?;
    (to > from).then_some((from, to))
}

/// The positions in `sequence` that show what an edit changed on `side`, the
/// project before or after it, compared with `other`: the last frame
/// before and the first frame of every cut it made or moved, and the middle
/// frame of a clip it changed in place. The earliest come first.
fn preview_times(side: &Project, other: &Project, sequence: &str) -> Vec<Time> {
    let Some(sequence) = side.sequence(sequence) else {
        return Vec::new();
    };
    let old = other.sequence(sequence.id.as_str());
    let rate = sequence.rate;
    let mut times = Vec::new();
    let mut cut = |at: Time| times.extend([at - rate.frame_duration(), at]);
    let mut middles = Vec::new();
    for track in &sequence.tracks {
        let previous = old.and_then(|old| old.track(track.id.as_str()));
        for clip in &track.clips {
            match previous.and_then(|previous| previous.clip(clip.id.as_str())) {
                Some(was) if was == clip => {}
                Some(was) if (was.start, was.end()) == (clip.start, clip.end()) => {
                    let (first, last) =
                        (rate.frames_floor(clip.start), rate.frames_floor(clip.end()));
                    middles.push(rate.time_of(first + (last - first) / 2));
                }
                _ => {
                    cut(clip.start);
                    cut(clip.end());
                }
            }
        }
        let Some(previous) = previous else {
            continue;
        };
        for was in &previous.clips {
            if track.clip(was.id.as_str()).is_none() {
                cut(was.start);
            }
        }
        for transition in &track.transitions {
            if previous.transition(transition.id.as_str()) != Some(transition) {
                if let Some(clip) = track.clip(transition.from.as_str()) {
                    cut(clip.end());
                }
            }
        }
    }
    times.extend(middles);
    let duration = sequence.duration();
    times.retain(|time| !time.is_negative() && *time < duration);
    times.sort();
    times.dedup();
    times.truncate(PREVIEW_FRAMES);
    times
}

/// Frames of the program, and their contact sheet when one was asked for.
struct Capture {
    frames: Vec<String>,
    sheet: Option<String>,
}

fn resolve_project_source_path(dir: &Path, input: &str) -> PathBuf {
    let path = Path::new(input);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    }
}

/// Inline the image at `object.path` as `imageBase64`, with its `mediaType`.
fn inline_image(object: &mut Value) -> Result<(), McpError> {
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::new("HANDLER_FAILED", "artifact path is missing"))?
        .to_owned();
    let media_type = match Path::new(&path).extension().and_then(|ext| ext.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => {
            return Err(McpError::new(
                "HANDLER_FAILED",
                format!("artifact is not an image: {path}"),
            ))
        }
    };
    let metadata = fs::metadata(&path).map_err(io_error)?;
    if metadata.len() > MAX_INLINE_ARTIFACT_BYTES as u64 {
        return Err(McpError::new(
            "RESULT_TOO_LARGE",
            format!("inline artifact exceeds {MAX_INLINE_ARTIFACT_BYTES} bytes"),
        ));
    }
    let data = fs::read(path).map_err(io_error)?;
    object["imageBase64"] = json!(base64::engine::general_purpose::STANDARD.encode(data));
    object["mediaType"] = json!(media_type);
    Ok(())
}

fn io_error(error: io::Error) -> McpError {
    McpError::new("HANDLER_FAILED", error.to_string())
}

fn not_pending(id: &str) -> McpError {
    McpError::new(
        "NOT_PENDING",
        format!("proposal {id} is not waiting for review"),
    )
}

/// A recipe an edit writes: the file it was written as, its text, and where it is kept.
struct RecipeVersion {
    asked: String,
    text: String,
    version: kerf_engine::recipe::Version,
}

impl RecipeVersion {
    fn json(&self) -> Value {
        let mut value = json!({"recipe": self.asked, "keptAs": self.version.path});
        value["written"] = json!(self.version.new);
        if let Some(before) = &self.version.before {
            value["diff"] = json!(kerf_lang::diff_text(before, &self.text));
        }
        value
    }
}

/// The lines an edit's script gains for the recipes it writes: each recipe as comments, for the
/// person reviewing it to read, and a `media replace` for each media that names a version of it
/// other than the one that holds the text, which it moves to. A recipe written again that no media
/// names is refused: it would be a file the project never uses.
fn recipe_lines(
    recipes: &[RecipeVersion],
    project: &Project,
) -> Result<(String, String), McpError> {
    let (mut shown, mut moves) = (String::new(), String::new());
    for recipe in recipes {
        let path = &recipe.version.path;
        let heading = match (&recipe.version.before, recipe.version.new) {
            (None, true) => format!("# writes the recipe {path}:"),
            (None, false) => format!("# the recipe {path}, as it is:"),
            (Some(_), true) => format!("# writes the recipe {path}, after {}:", recipe.asked),
            (Some(_), false) => format!(
                "# the recipe {path}, as it was, in place of {}:",
                recipe.asked
            ),
        };
        shown.push_str(&heading);
        shown.push('\n');
        for line in recipe.text.lines() {
            shown.push_str(&format!("#   {line}\n"));
        }
        let named: Vec<&Media> = project
            .media
            .iter()
            .filter(|media| {
                media
                    .recipe
                    .as_deref()
                    .is_some_and(|named| kerf_engine::recipe::same_recipe(named, &recipe.asked))
            })
            .collect();
        if named.is_empty() && (recipe.version.before.is_some() || !recipe.version.new) {
            return Err(McpError::new(
                "INVALID_RECIPE",
                format!(
                    "no media names `{}`, which holds another recipe: name a new file, or the \
                     media whose recipe it is",
                    recipe.asked
                ),
            ));
        }
        for media in named
            .into_iter()
            .filter(|media| media.recipe.as_deref() != Some(path.as_str()))
        {
            let mut moved = media.clone();
            moved.recipe = Some(path.clone());
            moves.push_str(&kerf_lang::print_op(
                project,
                catalog(),
                &kerf_model::Op::ReplaceMedia(moved),
            ));
            moves.push('\n');
        }
    }
    Ok((shown, moves))
}

/// `text` ending with a line break, unless it is empty.
fn with_newline(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}

/// What would refuse a making, as an agent is told it: where a person's own models file is, is
/// theirs to be told and not an agent's, which is told which models there are.
pub(crate) fn cannot_said(
    cannot: String,
    models: Option<&kerf_engine::recipe::model::Models>,
) -> String {
    match (cannot.split_once(" is set up: "), models) {
        (Some((model, _)), Some(models)) if cannot.starts_with("no model `") => {
            let have: Vec<&str> = models.names().collect();
            format!(
                "{model} is set up: {}; a person sets one up in their own models file",
                if have.is_empty() {
                    "none is".to_owned()
                } else {
                    format!("the models set up are {}", have.join(", "))
                }
            )
        }
        _ => cannot,
    }
}

pub(crate) fn operation_key(kind: &str) -> String {
    let sequence = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{kind}-{}-{sequence}-{timestamp}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(text: &str) -> Project {
        parse(text).expect("document")
    }

    const DOCUMENT: &str = "kerf 1
project name=\"Test\"
media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000
sequence s1 rate=24 size=1920x1080
  track V1 video
    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00
    clip c2 media=m1 src=00:00:10:00 at=00:00:04:00 dur=00:00:04:00
  track A1 audio
";

    #[test]
    fn models_are_listed_with_what_each_takes() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("models.json");
        std::fs::write(
            &file,
            r#"{
                "vace": {
                    "run": ["kerf-comfy", "--workflow", "wan-vace", "{request}"],
                    "takes": {
                        "about": "follows a guide",
                        "size": [832, 480],
                        "rate": 16,
                        "seconds": 5.0625,
                        "reads": ["guide", "ref"],
                        "more": {"steps": 20},
                        "frames": {"most": 81, "step": 4},
                        "name": "another",
                        "guide": "run anything"
                    }
                },
                "ntsc": {"run": ["a", "{request}"], "takes": {"rate": 23.976, "reads": ["guide"]}},
                "words": {"run": ["b", "{request}"], "takes": {"size": [1024, 1024], "reads": []}},
                "dawn": {"run": ["c", "{request}"]}
            }"#,
        )
        .unwrap();
        let models = kerf_engine::recipe::model::Models::read(&file).unwrap();
        assert_eq!(
            json!(models_listed(&models)),
            json!([
                // One that has not said is its name alone.
                {"name": "dawn"},
                // A rate that is not whole is written as a line writes it.
                {"name": "ntsc", "rate": 23.976, "reads": ["guide"],
                 "guide": "render <sequence> {guide.mp4} rate=23.976"},
                {"name": "vace", "about": "follows a guide", "size": "832x480", "rate": 16.0,
                 "seconds": 5.0625, "reads": ["guide", "ref"], "more": {"steps": 20},
                 "frames": {"most": 81, "step": 4},
                 "guide": "render <sequence> {guide.mp4} size=832x480 rate=16"},
                // One that follows no guide is given no line to draw one.
                {"name": "words", "size": "1024x1024", "reads": []},
            ])
        );
    }

    #[test]
    fn a_files_frames_are_counted_as_a_making_counts_them() {
        // A take a server made: 65 frames at 16 a second, its container saying 4.062012 s.
        // Counted down, the line that chooses the take had it a frame short of the file.
        assert_eq!(frames_of(4.062012, 16.0), 65);
        assert_eq!(frames_of(4.0625, 16.0), 65);
        // A whole length is what it was.
        assert_eq!(frames_of(8.0, 30.0), 240);
        // 300 frames of 29.97 said a hair short, by the rate a document keeps for it.
        assert_eq!(frames_of(10.0099, 29.97), 300);
        // A file whose length was not read is still a frame long, as an import has it.
        assert_eq!(frames_of(0.0, 30.0), 1);
    }

    #[test]
    fn changed_range_covers_old_and_new_positions() {
        let before = project(DOCUMENT);
        let after = apply_script(&before, catalog(), "move c2 at=00:00:10:00")
            .unwrap()
            .project;
        let (sequence, from, to) = changed_range(&before, &after).unwrap();
        let clock = Clock::of_sequence(&sequence);
        assert_eq!(clock.print_position(from), "00:00:04:00");
        assert_eq!(clock.print_position(to), "00:00:14:00");

        let trimmed = apply_script(&before, catalog(), "set c1 opacity=0.5")
            .unwrap()
            .project;
        let (_, from, to) = changed_range(&before, &trimmed).unwrap();
        assert_eq!(
            (clock.print_position(from), clock.print_position(to)),
            ("00:00:00:00".into(), "00:00:04:00".into())
        );

        let muted = apply_script(&before, catalog(), "track set A1 muted=true")
            .unwrap()
            .project;
        let (_, from, to) = changed_range(&before, &muted).unwrap();
        assert_eq!((from, to), (Time::ZERO, before.sequences[0].duration()));

        let marked = apply_script(&before, catalog(), "marker add at=00:00:01:00 name=\"x\"")
            .unwrap()
            .project;
        assert!(changed_range(&before, &marked).is_none());
    }

    #[test]
    fn preview_shows_the_frames_around_each_changed_cut() {
        let before = project(DOCUMENT);
        let print = |project: &Project, times: Vec<Time>| {
            let clock = Clock::of_sequence(&project.sequences[0]);
            times
                .into_iter()
                .map(|time| clock.print_position(time))
                .collect::<Vec<_>>()
        };

        // A ripple delete at the head moves every later clip, but the
        // preview starts at the cut it made.
        let rippled = apply_script(&before, catalog(), "ripple-delete c1")
            .unwrap()
            .project;
        assert_eq!(
            print(&rippled, preview_times(&rippled, &before, "s1")),
            ["00:00:00:00", "00:00:03:23"]
        );
        assert_eq!(
            print(&before, preview_times(&before, &rippled, "s1")),
            ["00:00:00:00", "00:00:03:23", "00:00:04:00", "00:00:07:23"]
        );

        // A change in place shows the middle of the clip.
        let faded = apply_script(&before, catalog(), "set c2 opacity=0.5")
            .unwrap()
            .project;
        assert_eq!(
            print(&faded, preview_times(&faded, &before, "s1")),
            ["00:00:06:00"]
        );
    }

    #[test]
    fn context_text_is_bounded_at_line_boundaries() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(bounded_lines(text, 100, "…"), text);
        assert_eq!(bounded_lines(text, 9, "…"), "one\ntwo\n…\n");
    }

    #[test]
    fn prompt_carries_the_reference_and_editorial_guidance() {
        let prompt = system_prompt(&kerf_skills::bundled_skills());
        assert!(prompt.starts_with("You edit video in Kerf."));
        assert!(prompt.contains("# Kerf edit script"));
        assert!(prompt.contains("`dissolve@1`"));
        assert!(prompt.contains("## shorts"));
        for retired in [
            "tools.search",
            "timeline.apply",
            "skills.read",
            "media.probe",
        ] {
            assert!(!prompt.contains(retired), "{retired}");
        }
    }

    #[test]
    fn proposal_status_describes_each_lifecycle_state() {
        let pending = PendingProposal::Pending {
            proposal_id: "p1".into(),
            base_revision: 3,
            script: "trim c1 out -6f".into(),
            source: String::new(),
            document_diff: "-a\n+b\n".into(),
            freshness: ProposalFreshness::Stale,
        };
        let text = proposal_status(Some(&pending)).unwrap();
        assert!(text.contains("Proposal p1 (from revision 3)"));
        assert!(text.contains("revise it"));
        assert!(text.contains("```diff\n-a\n+b\n```"));
        let rejected = PendingProposal::Rejected {
            proposal_id: "p1".into(),
            base_revision: 3,
        };
        assert_eq!(
            proposal_status(Some(&rejected)).unwrap(),
            "Proposal p1 was rejected.\n"
        );
        assert!(proposal_status(None).is_none());
    }
}
