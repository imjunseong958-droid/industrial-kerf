//! The agent harness on the kernel: `look`, `edit`, and `render`.
//!
//! An agent works from two pieces of context: the annotated `.cut` document
//! and the language reference. It looks at media and frames, changes the
//! project with one edit script, and renders. Each edit becomes a proposal
//! for the user, returned with its document diff, the check findings, and a
//! before/after preview of the range it changed.

use super::{
    add_inline_base64, engine_error, operation_key, resolve_project_source_path, CapturePayload,
    CutMcpRouter, McpError, ToolAnnotations,
};
use cut_engine::derivatives::{DerivativeCache, SourceIdentity};
use cut_engine::program::{compile_program, ProgramOptions};
use cut_engine::render::{render_program, RenderOptions};
use cut_lang::{apply_script, parse_document, print_document, reference, Clock, PrintOptions};
use cut_model::{Catalog, Content, Media, Project, Sequence, Time};
use cut_runtime::{PendingProposal, ProposalFreshness, RuntimeClient, RuntimeError};
use cut_skills::Skill;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The tools every agent and MCP client uses.
pub const TOOL_NAMES: &[&str] = &["look", "edit", "render"];

const MAX_LOOK_FRAMES: usize = 12;
const DEFAULT_FRAME_WIDTH: u32 = 960;
const PREVIEW_FRAMES: usize = 4;
const PREVIEW_WIDTH: u32 = 480;
/// The longest stretch a preview covers, starting at the first change.
const MAX_PREVIEW_SECONDS: f64 = 120.0;
/// The largest document carried as context; `look` returns all of it.
const MAX_CONTEXT_DOCUMENT_BYTES: usize = 96 * 1024;
const MAX_CONTEXT_DIFF_BYTES: usize = 8 * 1024;
const SCENE_THRESHOLD: f64 = 0.35;
const PAUSE_NOISE_DB: f64 = -35.0;
const PAUSE_SECONDS: f64 = 0.35;

const GUIDE: &str = "You edit video in Cut. The project is a `.cut` text \
document, annotated with each clip's end and source out. Change it with edit \
scripts in the language below.

- `look` shows what is there: with no arguments the document; with `media` \
that media's index (where it is used, its scene cuts and pauses, and what is \
said on request); with `at` frames — source frames of a media, or program \
frames of the sequence.
- `edit` runs one script. It becomes a proposal for the user, returned with \
the document diff, check findings, and a before/after preview of the range \
it changed. The project does not change until the user accepts it. Replace \
a pending proposal by passing its id as `revise`.
- `render` renders the sequence, or a range of it, to a file.

Work like an editor. Look at the footage before choosing ranges, make each \
change one script, and look at the preview before saying it is done. Base \
every time, word, and frame on something you saw or heard; never invent \
them. A refused script names the line and the clip that stopped it.";

/// Fixed instructions for an agent: the guide, the language reference, and
/// the editorial guidance of each skill.
pub fn system_prompt(skills: &[Skill]) -> String {
    cut_skills::compose_system_prompt(
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
        "# Current project (revision {revision})\n\n```cut\n{}```\n",
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
    Some(match name {
        "look" => (
            "See the project and its footage. No arguments: the annotated document and \
             proposal state. `media`: that media's index — where it is used, scene cuts, \
             pauses, and with `transcript` what is said. `at`: frames, as source frames \
             of `media` or program frames of the sequence.",
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
                    "transcript": {"type": "boolean", "description": "With `media`: include what is said (local transcription, up to 15 minutes per call)."},
                    "from": {"type": "string", "description": format!("With `transcript`: the first media position to transcribe. {time}")},
                    "to": {"type": "string", "description": "With `transcript`: the media position to stop at."},
                    "width": {"type": "integer", "minimum": 64, "maximum": 1920, "description": "Frame width in pixels (default 960)."}
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
             document diff, check findings, and a before/after preview of the changed \
             range. The project is unchanged until the user accepts it.",
            json!({
                "type": "object",
                "properties": {
                    "script": {"type": "string", "minLength": 1, "description": "One operation per line, applied in order and atomically."},
                    "revise": {"type": "string", "description": "The id of a pending proposal this one replaces."}
                },
                "required": ["script"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
            },
        ),
        "render" => (
            "Render the sequence, or the range `from`–`to`, to a file. Returns a job; \
             pass `job` to read its progress and output, with `cancel` to stop it.",
            json!({
                "type": "object",
                "properties": {
                    "quality": {"type": "string", "enum": ["draft", "full"], "description": "Default draft."},
                    "from": {"type": "string", "description": format!("Sequence position. {time}")},
                    "to": {"type": "string", "description": "Sequence position."},
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
    media: Option<String>,
    #[serde(default)]
    at: Vec<String>,
    #[serde(default)]
    transcript: bool,
    from: Option<String>,
    to: Option<String>,
    width: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditInput {
    script: String,
    revise: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderInput {
    quality: Option<String>,
    from: Option<String>,
    to: Option<String>,
    job: Option<String>,
    #[serde(default)]
    cancel: bool,
}

/// Scene cuts and pauses of one media file, in seconds from its first frame.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Analysis {
    cuts: Vec<f64>,
    pauses: Vec<(f64, f64)>,
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

fn diagnostics(errors: &[cut_model::Diagnostic]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The sequence the renderer plays: the first one.
fn program(project: &Project) -> Result<&Sequence, McpError> {
    project
        .sequences
        .first()
        .ok_or_else(|| McpError::new("NO_SEQUENCE", "the project has no sequence"))
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

impl CutMcpRouter {
    pub(super) fn look(&self, arguments: Value) -> Result<Value, McpError> {
        let input: LookInput = input(arguments)?;
        if input.media.is_none() && input.at.is_empty() {
            if input.transcript || input.from.is_some() || input.to.is_some() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "transcript, from, and to need `media`",
                ));
            }
            let (revision, document) = self.runtime.get_document(true)?;
            let state = self.runtime.get_state()?;
            return Ok(json!({
                "revision": revision,
                "document": document,
                "proposal": proposal_status(state.pending_proposal.as_ref()),
            }));
        }
        if input.at.len() > MAX_LOOK_FRAMES {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("look shows at most {MAX_LOOK_FRAMES} frames at a time"),
            ));
        }
        let (revision, document) = self.runtime.get_document(false)?;
        let project = parse(&document)?;
        let width = input.width.unwrap_or(DEFAULT_FRAME_WIDTH).clamp(64, 1920);
        let Some(media_id) = input.media.as_deref() else {
            return self.program_frames(&project, revision, &input.at, width);
        };
        let media = project.media(media_id).ok_or_else(|| {
            McpError::new(
                "UNKNOWN_MEDIA",
                format!("no media `{media_id}` in the project"),
            )
        })?;
        if input.at.is_empty() {
            self.media_index(&project, media, &input)
        } else {
            self.source_frames(media, &input.at, width)
        }
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
        match self.analysis(&path, media) {
            Ok(analysis) => {
                result["cuts"] = json!(analysis
                    .cuts
                    .iter()
                    .map(|cut| print_seconds(&clock, *cut))
                    .collect::<Vec<_>>());
                result["pauses"] = json!(analysis
                    .pauses
                    .iter()
                    .map(|(from, to)| json!({
                        "from": print_seconds(&clock, *from),
                        "to": print_seconds(&clock, *to),
                    }))
                    .collect::<Vec<_>>());
            }
            Err(error) => result["analysis"] = json!(format!("unavailable: {}", error.message)),
        }
        if input.transcript {
            result["transcript"] = self.transcript(&path, media, &clock, input)?;
        } else if input.from.is_some() || input.to.is_some() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "from and to choose a transcript range; pass transcript: true",
            ));
        }
        Ok(result)
    }

    /// Scene cuts and pauses, cached beside the media's other derivatives.
    fn analysis(&self, path: &std::path::Path, media: &Media) -> Result<Analysis, McpError> {
        let cache = DerivativeCache::new(&self.dir);
        let entry = cache
            .source_entry(
                path,
                "media-index",
                &json!({
                    "version": 1,
                    "sceneThreshold": SCENE_THRESHOLD,
                    "pauseNoiseDb": PAUSE_NOISE_DB,
                    "pauseSeconds": PAUSE_SECONDS,
                }),
            )
            .map_err(engine_error)?;
        if entry.is_complete(&["index.json"]) {
            return entry.read_json("index.json").map_err(engine_error);
        }
        let source = path.to_string_lossy();
        let mut analysis = Analysis::default();
        if media.video.is_some() {
            analysis.cuts = cut_engine::detect_scenes(&source, SCENE_THRESHOLD)
                .map_err(engine_error)?
                .into_iter()
                .map(|scene| scene.start)
                .filter(|start| *start > 0.0)
                .collect();
        }
        if media.audio.is_some() {
            let end = media.duration.seconds();
            analysis.pauses =
                cut_engine::probe::silence_detect(&source, PAUSE_NOISE_DB, PAUSE_SECONDS)
                    .map_err(engine_error)?
                    .into_iter()
                    .map(|pause| (pause.start, pause.end.unwrap_or(end)))
                    .collect();
        }
        entry
            .write_json("index.json", &analysis)
            .map_err(engine_error)?;
        entry.mark_complete(&["index.json"]).map_err(engine_error)?;
        Ok(analysis)
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
        if to - from > cut_engine::MAX_TRANSCRIPTION_RANGE_SECONDS + 1e-9 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "a transcript covers at most 15 minutes; choose a range with from and to",
            ));
        }
        let range = cut_engine::TimeRange::new(from, to)
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
        let result = cut_engine::transcribe_cached(
            &path.to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &cut_engine::TranscriptionOptions::new(range),
            &cut_engine::CancellationToken::new(),
            |_| {},
        )
        .map_err(engine_error)?;
        Ok(json!(result
            .transcript
            .cues
            .iter()
            .map(|cue| json!({
                "from": print_seconds(clock, cue.start),
                "to": print_seconds(clock, cue.end),
                "text": cue.text.trim(),
            }))
            .collect::<Vec<_>>()))
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
        let path = resolve_project_source_path(&self.dir, &media.path);
        let grabbed = cut_engine::cached_exact_frames(
            &path.to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &times,
            width,
        )
        .map_err(engine_error)?;
        let mut frames = Vec::new();
        for (text, frame) in at.iter().zip(grabbed.frames) {
            let mut value = json!({"at": text, "path": frame.path});
            add_inline_base64(&mut value, "path", "imageBase64")?;
            frames.push(value);
        }
        Ok(json!({"media": media.id.to_string(), "frames": frames}))
    }

    fn program_frames(
        &self,
        project: &Project,
        revision: u64,
        at: &[String],
        width: u32,
    ) -> Result<Value, McpError> {
        let sequence = program(project)?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let mut resolved = project.clone();
        cut_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let mut frames = Vec::new();
        for text in at {
            let time = position(&clock, text)?;
            if time.is_negative() || time >= duration {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("`{text}` is outside sequence `{}`", sequence.id),
                ));
            }
            let end = (time + sequence.rate.frame_duration()).min(duration);
            let capture =
                self.capture_program(&resolved, time, end, &[time.seconds()], width, 1, "look")?;
            let mut value = json!({"at": text, "path": capture["frames"][0]["path"]});
            add_inline_base64(&mut value, "path", "imageBase64")?;
            frames.push(value);
        }
        Ok(json!({"sequence": sequence.id.to_string(), "revision": revision, "frames": frames}))
    }

    /// Render `[start, end)` of the program as a draft and extract frames at
    /// `times` (program seconds) with a contact sheet of them. `project`
    /// has its media paths resolved. Results are cached by document, media
    /// identity, and options.
    #[allow(clippy::too_many_arguments)]
    fn capture_program(
        &self,
        project: &Project,
        start: Time,
        end: Time,
        times: &[f64],
        width: u32,
        columns: usize,
        kind: &str,
    ) -> Result<Value, McpError> {
        let range = Some((start, end));
        compile_program(
            project,
            catalog(),
            &ProgramOptions {
                range,
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
        let options = json!({
            "start": start.seconds(),
            "end": end.seconds(),
            "times": times,
            "maxWidth": width,
            "columns": columns,
        });
        let entry = DerivativeCache::new(&self.dir)
            .project_entry(
                &json!(document),
                &identities,
                &format!("program-{kind}"),
                &options,
            )
            .map_err(engine_error)?;
        let frame_names: Vec<String> = (0..times.len())
            .map(|index| format!("frame-{index:03}.png"))
            .collect();
        let mut required = vec!["capture.mp4", "capture.png", "capture.json"];
        required.extend(frame_names.iter().map(String::as_str));
        let cached = entry.is_complete(&required);
        let payload = if cached {
            entry
                .read_json::<CapturePayload>("capture.json")
                .map_err(engine_error)?
        } else {
            let output = entry.path("capture.mp4");
            render_program::<fn(f64)>(
                project,
                None,
                range,
                &RenderOptions {
                    output: output.to_string_lossy().into_owned(),
                    draft: true,
                    force_software: true,
                    ..RenderOptions::default()
                },
                None,
            )
            .map_err(engine_error)?;
            let relative: Vec<f64> = times.iter().map(|time| time - start.seconds()).collect();
            let frames = cut_engine::probe::extract_exact_frames(
                &output.to_string_lossy(),
                &relative,
                &entry.directory,
                width,
            )
            .map_err(engine_error)?;
            let paths: Vec<String> = frames.iter().map(|frame| frame.path.clone()).collect();
            cut_engine::probe::create_contact_sheet(&paths, &entry.path("capture.png"), columns)
                .map_err(engine_error)?;
            let payload = CapturePayload {
                start: start.seconds(),
                end: end.seconds(),
                duration: (end - start).seconds(),
                path: entry.path("capture.png").to_string_lossy().into_owned(),
                media_type: "image/png".into(),
                frames: frames
                    .into_iter()
                    .zip(times)
                    .map(|(frame, time)| {
                        json!({"time": time, "path": frame.path, "mediaType": frame.media_type})
                    })
                    .collect(),
            };
            entry
                .write_json("capture.json", &payload)
                .map_err(engine_error)?;
            entry.mark_complete(&required).map_err(engine_error)?;
            payload
        };
        let mut result = serde_json::to_value(&payload)
            .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
        result["cached"] = json!(cached);
        Ok(result)
    }

    pub(super) fn edit(&self, arguments: Value) -> Result<Value, McpError> {
        let input: EditInput = input(arguments)?;
        if let Some(revise) = input.revise.as_deref() {
            match self.runtime.get_state()?.pending_proposal {
                Some(PendingProposal::Pending { proposal_id, .. }) if proposal_id == revise => {
                    self.runtime.reject_proposal(revise)?;
                }
                _ => {
                    return Err(McpError::new(
                        "NOT_PENDING",
                        format!("proposal {revise} is not waiting for review"),
                    ))
                }
            }
        }
        let (revision, document) = self.runtime.get_document(false)?;
        let before = parse(&document)?;
        let applied = apply_script(&before, catalog(), &input.script)
            .map_err(|errors| McpError::new("SCRIPT_REFUSED", diagnostics(&errors)))?;
        if print_document(&applied.project, catalog(), PrintOptions::default()) == document {
            return Err(McpError::new("NO_CHANGE", "the script changes nothing"));
        }
        let snapshot = self
            .runtime
            .stage_script_proposal(
                &operation_key("agent-edit"),
                revision,
                &input.script,
                Vec::new(),
            )
            .map_err(|error| match error.code.as_str() {
                "PROPOSAL_PENDING" => McpError::new(
                    "PROPOSAL_PENDING",
                    "another proposal is waiting for the user's review; pass its id as \
                     `revise` to replace it",
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
        let (before_document, after_document) = self.runtime.proposal_documents(&proposal_id)?;
        let (before_program, after_program) = (parse(&before_document)?, parse(&after_document)?);
        let check = check(&after_program);
        let preview = match changed_range(&before, &applied.project) {
            None => json!({"unavailable": "the edit changes nothing on the sequence"}),
            Some((sequence, from, to)) => self
                .preview(&sequence, from, to, &before_program, &after_program)
                .unwrap_or_else(|error| json!({"unavailable": error.message})),
        };
        Ok(json!({
            "proposal": proposal_id,
            "baseRevision": revision,
            "status": "waiting for the user's review",
            "script": script,
            "diff": document_diff,
            "check": check,
            "preview": preview,
        }))
    }

    /// Contact sheets of the changed range before and after the edit.
    /// `before` and `after` have their media paths resolved.
    fn preview(
        &self,
        sequence: &Sequence,
        from: Time,
        to: Time,
        before: &Project,
        after: &Project,
    ) -> Result<Value, McpError> {
        let clock = Clock::of_sequence(sequence);
        let longest = Time::from_seconds(MAX_PREVIEW_SECONDS).unwrap_or(to);
        let end = to.min(from + longest);
        let mut result = json!({
            "from": clock.print_position(from),
            "to": clock.print_position(end),
        });
        for (side, project) in [("before", before), ("after", after)] {
            let end = end.min(program(project)?.duration());
            if end <= from {
                result[side] = json!({"empty": true});
                continue;
            }
            let span = (end - from).seconds();
            let times: Vec<f64> = (0..PREVIEW_FRAMES)
                .map(|index| from.seconds() + span * (index as f64 + 0.5) / PREVIEW_FRAMES as f64)
                .collect();
            let capture = self.capture_program(
                project,
                from,
                end,
                &times,
                PREVIEW_WIDTH,
                PREVIEW_FRAMES,
                side,
            )?;
            let mut sheet = json!({"path": capture["path"]});
            add_inline_base64(&mut sheet, "path", "imageBase64")?;
            result[side] = sheet;
        }
        Ok(result)
    }

    pub(super) fn render(&self, arguments: Value) -> Result<Value, McpError> {
        let input: RenderInput = input(arguments)?;
        if let Some(job) = input.job.as_deref() {
            if input.quality.is_some() || input.from.is_some() || input.to.is_some() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "`job` reads a render; it takes no quality or range",
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
        let mut range = None;
        if input.from.is_some() || input.to.is_some() {
            let (_, document) = self.runtime.get_document(false)?;
            let project = parse(&document)?;
            let sequence = program(&project)?;
            let clock = Clock::of_sequence(sequence);
            let from = match &input.from {
                Some(text) => position(&clock, text)?,
                None => Time::ZERO,
            };
            let to = match &input.to {
                Some(text) => position(&clock, text)?,
                None => sequence.duration(),
            };
            range = Some(cut_runtime::RenderRange::new(
                from.seconds(),
                (to - from).seconds(),
            )?);
        }
        let started = self.render_start(draft, range)?;
        Ok(json!({
            "job": started["jobId"],
            "status": started["status"],
            "output": started["output"],
            "revision": started["revision"],
        }))
    }
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
/// read, features the renderer cannot lower, and the model's warnings.
/// `project` has its media paths resolved.
fn check(project: &Project) -> Value {
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
    if let Err(error) = compile_program(project, catalog(), &ProgramOptions::default()) {
        findings.push(json!({
            "severity": "error",
            "code": "unrenderable",
            "entity": error.entity(),
            "message": error.to_string(),
        }));
    }
    for warning in cut_model::validate(project, catalog()) {
        findings.push(json!({
            "severity": if warning.is_error() { "error" } else { "warning" },
            "code": warning.code,
            "entity": warning.entity.as_ref().map(|id| id.as_str()),
            "message": warning.message,
        }));
    }
    let valid = !findings
        .iter()
        .any(|finding| finding["severity"] == "error");
    json!({"valid": valid, "findings": findings})
}

/// The span of the program that differs between two projects: every clip
/// added, removed, or changed, and the transitions around them. A change to
/// a track itself (enabled, muted) spans the whole sequence.
fn changed_range(before: &Project, after: &Project) -> Option<(Sequence, Time, Time)> {
    let sequence = after.sequences.first()?;
    let old = before.sequence(sequence.id.as_str());
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
    (to > from).then(|| (sequence.clone(), from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(text: &str) -> Project {
        parse(text).expect("document")
    }

    const DOCUMENT: &str = "cut 1
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
    fn context_text_is_bounded_at_line_boundaries() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(bounded_lines(text, 100, "…"), text);
        assert_eq!(bounded_lines(text, 9, "…"), "one\ntwo\n…\n");
    }

    #[test]
    fn prompt_carries_the_reference_and_editorial_guidance() {
        let prompt = system_prompt(&cut_skills::bundled_skills());
        assert!(prompt.starts_with("You edit video in Cut."));
        assert!(prompt.contains("# Cut edit script"));
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
            observation: None,
            observations: None,
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
