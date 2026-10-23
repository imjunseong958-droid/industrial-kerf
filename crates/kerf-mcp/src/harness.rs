//! The agent harness on the kernel: `look`, `edit`, and `render`.
//!
//! An agent works from two pieces of context: the annotated `.kerf` document
//! and the language reference. It looks at media and frames, changes the
//! project with one edit script, and renders. Each edit becomes a proposal
//! for the user, returned with its document diff, the check findings, and a
//! before/after preview of the range it changed.

use super::{engine_error, KerfMcpRouter, McpError, ToolAnnotations};
use base64::Engine;
use kerf_engine::derivatives::{DerivativeCache, SourceIdentity};
use kerf_engine::program::{compile_program, ProgramOptions};
use kerf_engine::render::{render_program, RenderOptions};
use kerf_lang::{apply_script, parse_document, print_document, reference, Clock, PrintOptions};
use kerf_model::{Catalog, Content, Media, Project, Sequence, Time};
use kerf_runtime::{
    PendingProposal, ProposalFreshness, RenderJob, RenderRange, RuntimeClient, RuntimeError,
};
use kerf_skills::Skill;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
const MAX_INLINE_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

/// The tools every agent and MCP client uses.
pub const TOOL_NAMES: &[&str] = &["look", "edit", "render"];

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
/// Words a caption holds unless `captions … words=<n>` says otherwise.
const DEFAULT_CAPTION_WORDS: usize = 4;
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
With `proposal` it shows what a pending proposal would make: its document, \
its frames, and what it says.
- `edit` runs one script. It becomes a proposal for the user, returned with \
the document diff, check findings, and a before/after preview: the frames \
either side of each cut it made or moved, and the middle of a clip it changed \
in place. The project does not change until the user accepts it. Pass a \
pending proposal's id as `extend` to run more lines on top of it, or as \
`revise` to replace it.
- `render` renders the sequence, or a range of it, to a file; with \
  `proposal`, what that proposal would make.

Reusable presets are portable .kerfpreset files. `look` with `presets: true` \
lists project presets; with `preset: \"file.kerfpreset\"` it shows the pattern, \
text slots, and each media slot with the clips that use it and the imported \
media that fit it (`fits`, with the latest `from` frame as `latestFrame`). \
`edit` with `savePreset: \"file.kerfpreset\"` \
saves the accepted sequence (optional `sequence` and `name`), without changing \
the project. To reuse one, `edit` with `preset`, `bindings` mapping each slot \
to {media: \"imported-media-id\", from: \"0f\"}, and optional `texts` mapping \
template clip ids to new text. This replaces the chosen sequence (the first \
by default) as a normal proposal. Every slot must be bound; its minimum \
duration includes transition handles. Inspect the preset before binding it.

In an edit script, `captions <clip> on=<subtitle track> [words=<n>]` captions \
what the clip says, timed to its speech, from a transcript of its media made \
earlier with `look`; the proposal's script shows the lines it became.

Check findings include what an editor would catch: black frames, picture that \
leaves the frame's edges black, flash frames, and sound cut inside a word \
(known once a transcript of that media has been made).

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
    if let Some(status) = proposal_status(state.pending_proposal.as_ref()) {
        text.push('\n');
        text.push_str(&status);
    }
    Ok(text)
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
             is said (`words` times each word). `sheet`: a contact sheet of a range. \
             `at`: frames, as source frames of `media` or program frames of the \
             sequence. `transcript` without `media`: what the sequence says. \
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
                    "sheet": {"type": "integer", "minimum": 2, "maximum": MAX_SHEET_FRAMES, "description": "A contact sheet of this many frames spread evenly over `from`–`to` (all of it by default) as one image, four to a row: of `media`, or else of the sequence. Scan with a sheet, then look `at` the moments that matter."},
                    "from": {"type": "string", "description": format!("The start of the range for `transcript`, `sheet`, or a media's onsets. Positions of `media`, or else of the sequence. {time}")},
                    "to": {"type": "string", "description": "The end of that range."},
                    "width": {"type": "integer", "minimum": 64, "maximum": 1920, "description": "Frame width in pixels (default 960; 320 for each frame of a sheet)."},
                    "sequence": {"type": "string", "description": sequence},
                    "proposal": {"type": "string", "description": proposal},
                    "preset": {"type": "string", "description": "Inspect a .kerfpreset file: pattern, text slots, and media slots with the imported media that fit each. Use alone."},
                    "presets": {"type": "boolean", "description": "List .kerfpreset files in the project root and .kerf/presets. Use alone with true."},
                    "effect": {"type": "string", "description": "An effect, transition, or generator id: its parameters, as the reference lists them. Raster effects are listed by id only. Use alone."}
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
             around each cut it changed. The project is unchanged until the user accepts it. `extend` builds \
             on a pending proposal; `revise` replaces it. `savePreset` saves a portable \
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
                        "from": {"type": "string", "description": "Source start, including handles; default 0f."}
                    }, "required": ["media"], "additionalProperties": false}},
                    "texts": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Template caption/title clip id to replacement text."},
                    "extend": {"type": "string", "description": "The id of a pending proposal to build on: the script runs on what it makes, the two become one proposal, and the preview shows what this script changed."},
                    "revise": {"type": "string", "description": "The id of a pending proposal this one replaces; the script runs on the project."}
                },
                "oneOf": [{"required": ["script"]}, {"required": ["preset"]}, {"required": ["savePreset"]}],
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
             of what a pending `proposal` would make. Returns a job; pass `job` to read its \
             progress and output, with `cancel` to stop it.",
            json!({
                "type": "object",
                "properties": {
                    "quality": {"type": "string", "enum": ["draft", "full"], "description": "Default draft."},
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
    media: Option<String>,
    #[serde(default)]
    at: Vec<String>,
    #[serde(default)]
    transcript: bool,
    #[serde(default)]
    words: bool,
    sheet: Option<usize>,
    from: Option<String>,
    to: Option<String>,
    width: Option<u32>,
    sequence: Option<String>,
    proposal: Option<String>,
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

fn input<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, McpError> {
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
    /// The script and base revision of `id`, the proposal waiting for review.
    fn pending(&self, id: &str) -> Result<(String, u64), McpError> {
        match self.runtime.get_state()?.pending_proposal {
            Some(PendingProposal::Pending {
                proposal_id,
                script,
                base_revision,
                ..
            }) if proposal_id == id => Ok((script, base_revision)),
            _ => Err(McpError::new(
                "NOT_PENDING",
                format!("proposal {id} is not waiting for review"),
            )),
        }
    }

    /// The current revision and the project, or what the pending
    /// `proposal` would make of it.
    fn project_at(&self, proposal: Option<&str>) -> Result<(u64, Project), McpError> {
        let (revision, document) = self.runtime.get_document(false)?;
        let project = parse(&document)?;
        let Some(id) = proposal else {
            return Ok((revision, project));
        };
        let (script, base_revision) = self.pending(id)?;
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

    pub(super) fn look(&self, arguments: Value) -> Result<Value, McpError> {
        let preset_only = arguments.as_object().is_some_and(|o| o.len() == 1);
        let input: LookInput = input(arguments)?;
        if let Some(id) = &input.effect {
            if !preset_only {
                return Err(McpError::new("INVALID_ARGUMENTS", "use effect alone"));
            }
            let fx = catalog().fx(id).ok_or_else(|| {
                McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("no effect `{id}` in the catalog"),
                )
            })?;
            return Ok(json!({
                "effect": fx.id.as_str(),
                "category": fx.category,
                "engine": fx.engine.name(),
                "reference": kerf_lang::fx_reference(fx),
            }));
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
        let ranged = input.transcript || input.sheet.is_some() || input.media.is_some();
        if !ranged && (input.from.is_some() || input.to.is_some()) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "from and to choose a range for `transcript`, `sheet`, or a media's onsets",
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
        if input.at.len() > MAX_LOOK_FRAMES {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("look shows at most {MAX_LOOK_FRAMES} frames at a time"),
            ));
        }
        if input.media.is_none()
            && input.at.is_empty()
            && !input.transcript
            && input.sheet.is_none()
            && input.sequence.is_none()
            && input.proposal.is_none()
        {
            let (revision, document) = self.runtime.get_document(true)?;
            let state = self.runtime.get_state()?;
            return Ok(json!({
                "revision": revision,
                "document": document,
                "proposal": proposal_status(state.pending_proposal.as_ref()),
            }));
        }
        let (revision, project) = self.project_at(input.proposal.as_deref())?;
        let default_width = if input.sheet.is_some() {
            DEFAULT_SHEET_TILE_WIDTH
        } else {
            DEFAULT_FRAME_WIDTH
        };
        let width = input.width.unwrap_or(default_width).clamp(64, 1920);
        let mut result = match input.media.as_deref() {
            None if input.transcript => self.program_transcript(&project, revision, &input)?,
            None if input.sheet.is_some() => {
                self.program_sheet(&project, revision, &input, width)?
            }
            None if input.at.is_empty() => json!({
                "revision": revision,
                "document": print_document(&project, catalog(), PrintOptions { annotate: true }),
            }),
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
                        format!("no media `{media_id}` in the project"),
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
        let index = kerf_engine::media_index(
            &path,
            &DerivativeCache::new(&self.dir),
            &kerf_engine::CancellationToken::new(),
        );
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
            Err(error) => result["analysis"] = json!(format!("unavailable: {error}")),
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
    /// are resolved, renders to; refused when it cannot be rendered.
    fn program_entry(
        &self,
        project: &Project,
        sequence: &str,
        kind: &str,
        options: &Value,
    ) -> Result<kerf_engine::derivatives::CacheEntry, McpError> {
        let (planned, ..) = kerf_engine::raster::plan_bakes(
            project,
            Some(sequence),
            &DerivativeCache::new(&self.dir),
        )
        .map_err(engine_error)?;
        compile_program(
            &planned.project,
            catalog(),
            &ProgramOptions {
                sequence: Some(sequence.to_owned()),
                ..ProgramOptions::default()
            },
        )
        .map_err(|error| McpError::new("UNRENDERABLE", error.to_string()))?;
        let document = print_document(project, catalog(), PrintOptions::default());
        let identities = used_media(project)
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
                        format!("`{text}` is outside sequence `{}`", sequence.id),
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
        Ok(Capture {
            frames: names
                .iter()
                .map(|name| entry.path(name).to_string_lossy().into_owned())
                .collect(),
            sheet: columns.map(|_| entry.path("sheet.jpg").to_string_lossy().into_owned()),
        })
    }

    pub(super) fn edit(&self, arguments: Value) -> Result<Value, McpError> {
        let input: EditInput = input(arguments)?;
        if [
            input.script.is_some(),
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
                "pass exactly one of script, preset, or savePreset",
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
            Some(id) => Some(self.pending(id)?.0),
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
            None => input.script.expect("checked script input"),
        };
        let (expanded, origins) = self.expand_captions(&before, &source_script)?;
        let applied = apply_script(&before, catalog(), &expanded)
            .map_err(|errors| refused(errors, &origins))?;
        if print_document(&applied.project, catalog(), PrintOptions::default())
            == print_document(&before, catalog(), PrintOptions::default())
        {
            return Err(McpError::new("NO_CHANGE", "the script changes nothing"));
        }
        let script = match prior.filter(|_| input.extend.is_some()) {
            Some(prior) => format!("{}\n{expanded}", prior.trim_end()),
            None => expanded,
        };
        if let Some(id) = replaced {
            self.runtime.reject_proposal(id)?;
        }
        let snapshot = self
            .runtime
            .stage_script_proposal(&operation_key("agent-edit"), revision, &script)
            .map_err(|error| match error.code.as_str() {
                "PROPOSAL_PENDING" => McpError::new(
                    "PROPOSAL_PENDING",
                    "another proposal is waiting for the user's review; pass its id as \
                     `extend` to build on it or as `revise` to replace it",
                ),
                _ => McpError::from(error),
            })?;
        let Some(PendingProposal::Pending {
            proposal_id,
            script,
            document_diff,
            ..
        }) = snapshot.pending_proposal
        else {
            return Err(McpError::new(
                "RUNTIME_ERROR",
                "the runtime did not keep the proposal",
            ));
        };
        let (_, after_document) = self.runtime.proposal_documents(&proposal_id)?;
        let after_program = parse(&after_document)?;
        let mut before_program = before.clone();
        kerf_runtime::resolve_media_paths(&mut before_program, &self.dir);
        let check = check(&after_program, &self.dir);
        let preview = match changed_range(&before, &applied.project) {
            None => json!({"unavailable": "the edit changes nothing on the sequence"}),
            Some((sequence, from, to)) => self
                .preview(&sequence, from, to, &before_program, &after_program)
                .unwrap_or_else(|error| json!({"unavailable": error.message})),
        };
        Ok(json!({
            "proposal": proposal_id,
            "baseRevision": revision,
            "status": "awaiting_review",
            "script": script,
            "diff": document_diff,
            "check": check,
            "preview": preview,
        }))
    }

    /// `script` with each `captions <clip> on=<track> [words=<n>]` line
    /// replaced by the lines that caption what the clip says, from
    /// transcripts of its media already made; and the line of `script`
    /// each line came from.
    fn expand_captions(
        &self,
        project: &Project,
        script: &str,
    ) -> Result<(String, Vec<usize>), McpError> {
        let mut lines: Vec<String> = Vec::new();
        let mut origins = Vec::new();
        for (index, line) in script.lines().enumerate() {
            let number = index + 1;
            let mut words = line.split_whitespace();
            if words.next() != Some("captions") {
                lines.push(line.to_owned());
                origins.push(number);
                continue;
            }
            let error = |message: String| {
                McpError::new(
                    "SCRIPT_REFUSED",
                    format!("error[captions] {number}:1: {message}"),
                )
            };
            let clip = words
                .next()
                .ok_or_else(|| error("captions needs a clip".into()))?;
            let (mut track, mut most) = (None, DEFAULT_CAPTION_WORDS);
            for field in words {
                match field.split_once('=') {
                    Some(("on", id)) => track = Some(id),
                    Some(("words", count)) => {
                        most = count
                            .parse()
                            .ok()
                            .filter(|count| *count > 0)
                            .ok_or_else(|| error(format!("`{count}` is not a word count")))?;
                    }
                    _ => {
                        return Err(error(format!(
                            "unexpected `{field}`; captions takes on=<subtitle track> and words=<n>"
                        )))
                    }
                }
            }
            let track = track.ok_or_else(|| error("captions needs on=<subtitle track>".into()))?;
            // The clip as the lines before this one leave it.
            let so_far = apply_script(project, catalog(), &lines.join("\n"))
                .map_err(|errors| refused(errors, &origins))?
                .project;
            let media = so_far
                .locate(clip)
                .map(|at| &so_far.clip_at(at).content)
                .and_then(|content| match content {
                    Content::Media { media, .. } => so_far.media(media.as_str()),
                    _ => None,
                })
                .ok_or_else(|| error(format!("no media clip `{clip}`")))?;
            let (spoken, covered) = known_speech(&self.dir, media);
            let (source_in, source_out) = so_far
                .clip_at(so_far.locate(clip).expect("located above"))
                .source_range()
                .unwrap_or_default();
            if !covers(&covered, source_in.seconds(), source_out.seconds()) {
                let clock = Clock::of_media(media);
                return Err(error(format!(
                    "no transcript of `{}` covers `{clip}` ({}–{}); look at it with transcript, from, and to first",
                    media.id,
                    clock.print_position(source_in),
                    clock.print_position(source_out),
                )));
            }
            let captions =
                kerf_lang::caption_script(&so_far, clip, track, &spoken, most).map_err(error)?;
            for caption in captions.lines() {
                lines.push(caption.to_owned());
                origins.push(number);
            }
        }
        Ok((lines.join("\n"), origins))
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
    /// background and return it queued.
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
        thread::spawn(move || {
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

/// A script refusal, with each line numbered as the agent wrote it.
fn refused(errors: Vec<kerf_model::Diagnostic>, origins: &[usize]) -> McpError {
    let errors: Vec<_> = errors
        .into_iter()
        .map(|mut error| {
            if let Some(span) = error.span.as_mut() {
                span.line = origins.get(span.line - 1).copied().unwrap_or(span.line);
            }
            error
        })
        .collect();
    McpError::new("SCRIPT_REFUSED", diagnostics(&errors))
}

/// The words of `media` that transcripts already made know, in seconds of
/// the media, and the ranges those transcripts cover. Nothing is
/// transcribed.
fn known_speech(dir: &Path, media: &Media) -> (Vec<kerf_lang::Word>, Vec<(f64, f64)>) {
    let path = resolve_project_source_path(dir, &media.path);
    let transcripts =
        kerf_engine::cached_transcripts(&path.to_string_lossy(), &DerivativeCache::new(dir))
            .unwrap_or_default();
    let ranges = transcripts
        .iter()
        .map(|transcript| (transcript.range.start, transcript.range.end))
        .collect();
    let words = transcripts
        .into_iter()
        .flat_map(|transcript| transcript.cues)
        .flat_map(|cue| cue.words)
        .map(|word| kerf_lang::Word {
            start: word.start,
            end: word.end,
            text: word.text.trim().to_owned(),
        })
        .collect();
    (words, ranges)
}

/// Whether `ranges` together cover `from`–`to`, give or take a frame.
fn covers(ranges: &[(f64, f64)], from: f64, to: f64) -> bool {
    let mut ranges = ranges.to_vec();
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reached = from;
    for (start, end) in ranges {
        if start > reached + 0.05 {
            break;
        }
        reached = reached.max(end);
    }
    reached + 0.05 >= to
}

/// Media that some clip uses.
fn used_media(project: &Project) -> impl Iterator<Item = &Media> {
    project.media.iter().filter(|media| {
        project.sequences.iter().any(|sequence| {
            sequence.tracks.iter().any(|track| {
                track.clips.iter().any(|clip| {
                    matches!(&clip.content, Content::Media { media: id, .. } if *id == media.id)
                })
            })
        })
    })
}

/// What stands between a project and its render: media that cannot be
/// read, features the renderer cannot lower, and the model's warnings;
/// and what an editor would catch in the cut. `project` has its media paths
/// resolved; `dir` is the project, whose transcripts show where words are.
fn check(project: &Project, dir: &Path) -> Value {
    let mut findings = Vec::new();
    for media in used_media(project) {
        if !std::path::Path::new(&media.path).is_file() {
            findings.push(json!({
                "severity": "error",
                "code": "missing_media",
                "entity": media.id.as_str(),
                "message": format!("media `{}` is missing: {}", media.id, media.path),
            }));
        }
    }
    for sequence in &project.sequences {
        // What a render compiles: raster effects baked, wherever they go.
        let planned =
            kerf_engine::raster::plan(project, catalog(), Some(sequence.id.as_str()), |_| {
                Ok::<_, std::convert::Infallible>("baked.mkv".to_owned())
            })
            .unwrap_or_else(|never| match never {});
        let options = ProgramOptions {
            sequence: Some(sequence.id.to_string()),
            ..ProgramOptions::default()
        };
        if let Err(error) = compile_program(&planned.project, catalog(), &options) {
            findings.push(json!({
                "severity": "error",
                "code": "unrenderable",
                "entity": error.entity().unwrap_or(sequence.id.as_str()),
                "message": error.to_string(),
            }));
        }
    }
    for warning in kerf_model::validate(project, catalog()) {
        findings.push(json!({
            "severity": if warning.is_error() { "error" } else { "warning" },
            "code": warning.code,
            "entity": warning.entity.as_ref().map(|id| id.as_str()),
            "message": warning.message,
        }));
    }
    findings.extend(crate::lint::findings(project, |media| {
        known_speech(dir, media)
            .0
            .into_iter()
            .map(|word| (word.start, word.end, word.text))
            .collect()
    }));
    let valid = !findings
        .iter()
        .any(|finding| finding["severity"] == "error");
    json!({"valid": valid, "findings": findings})
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

fn operation_key(kind: &str) -> String {
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
        assert!(prompt.contains("`dissolve`"));
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
