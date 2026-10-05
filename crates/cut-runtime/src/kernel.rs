//! The runtime's edits on the kernel model.
//!
//! The canonical project is a `cut_model::Project`. Every change reaches it
//! as one of three edits: an edit script, typed operations from the editor,
//! or a v2 command from a client that still speaks the v2 protocol. v2
//! commands with a kernel equivalent (trims, splits, moves, deletes, media
//! and clip insertion, markers, tracks) become operations, so linked sound
//! follows its picture. The rest go through the temporary v2 export: the
//! command runs on the exported project and the result is imported back with
//! the current model as the hint, so only what the command changed moves.

use crate::RuntimeError;
use cut_domain as v2;
use cut_interchange::v2::{export, exported_span, import_v2_with, kernel_track};
use cut_lang::{apply_script, parse_document, print_document, print_ops, PrintOptions};
use cut_model::{
    apply, Catalog, ClipChange, Content, Diagnostic, Edge, Id, Marker, MarkerColor, Media, NewClip,
    Op, Param, ParamType, Project, Ratio, Rejected, SequenceChange, Time, TrackChange, TrackKind,
    Value, VideoInfo,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::BTreeMap;

pub(crate) fn catalog() -> &'static Catalog {
    Catalog::builtin()
}

/// The canonical text of a project.
pub(crate) fn print(project: &Project) -> String {
    print_document(project, catalog(), PrintOptions::default())
}

/// The length of the program: the first sequence.
pub(crate) fn program_duration(project: &Project) -> Time {
    project
        .sequences
        .first()
        .map_or(Time::ZERO, cut_model::Sequence::duration)
}

/// `[start, end)` seconds of the first sequence as frame-aligned times.
pub(crate) fn time_range(
    project: &Project,
    start: f64,
    end: f64,
) -> Result<(Time, Time), RuntimeError> {
    let rate = project
        .sequences
        .first()
        .map(|sequence| sequence.rate)
        .ok_or_else(|| RuntimeError::new("INVALID_RENDER_RANGE", "the project has no sequence"))?;
    let time = |seconds: f64| {
        Time::from_seconds(seconds)
            .map(|time| rate.round(time))
            .ok_or_else(|| RuntimeError::new("INVALID_RENDER_RANGE", "the range is out of bounds"))
    };
    let (from, to) = (time(start)?, time(end)?);
    if to <= from {
        return Err(RuntimeError::new(
            "INVALID_RENDER_RANGE",
            "the range is shorter than a frame",
        ));
    }
    Ok((from, to))
}

/// Make relative media paths absolute against `directory`. Documents keep
/// paths relative to the project so it can move; renderers and players run
/// elsewhere and need them resolved.
pub fn resolve_media_paths(project: &mut Project, directory: &std::path::Path) {
    for media in &mut project.media {
        if std::path::Path::new(&media.path).is_relative() {
            media.path = directory.join(&media.path).to_string_lossy().into_owned();
        }
    }
}

/// The document of `project` with media paths resolved, for a renderer.
pub(crate) fn renderable(project: &Project, directory: &std::path::Path) -> String {
    let mut project = project.clone();
    resolve_media_paths(&mut project, directory);
    print(&project)
}

pub(crate) fn print_annotated(project: &Project) -> String {
    print_document(project, catalog(), PrintOptions { annotate: true })
}

pub(crate) fn parse(text: &str) -> Result<Project, RuntimeError> {
    parse_document(text, catalog())
        .map(|document| document.project)
        .map_err(|errors| diagnostics_error("INVALID_PROJECT", &errors))
}

pub(crate) fn diagnostics_error(code: &str, diagnostics: &[Diagnostic]) -> RuntimeError {
    let message: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
    RuntimeError::new(code, message.join("; "))
}

fn rejected_error(rejected: Rejected) -> RuntimeError {
    diagnostics_error("EDIT_REJECTED", &rejected.diagnostics)
}

fn domain_error(error: v2::DomainError) -> RuntimeError {
    RuntimeError::new(error.code, error.message)
}

/// The v2 project derived from the model for v2 commands, proposal field
/// diffs, and the v2 MCP catalog, until ADR 0006 phase 5. Every committed
/// project has one, so edits whose result cannot be exported are refused.
pub(crate) fn view(project: &Project) -> Result<v2::Project, RuntimeError> {
    export(project).map_err(|error| RuntimeError::new("V2_EXPORT_FAILED", error))
}

/// The v2 project of a committed project.
pub(crate) fn committed_view(project: &Project) -> v2::Project {
    view(project).expect("committed projects export to v2")
}

/// A new project: one empty sequence with the v2 defaults.
pub(crate) fn empty_project() -> Project {
    cut_interchange::v2::import_v2(&v2::empty_project())
        .expect("the empty v2 project imports")
        .project
}

/// Restore `target` (a snapshot from history) over `current`, keeping every
/// retired id retired and retiring ids that only `current` uses.
pub(crate) fn restore(target: &Project, current: &Project) -> Project {
    let mut restored = target.clone();
    restored.retired.extend(current.retired.iter().cloned());
    restored.retire_missing(current.ids());
    restored
}

/// One change requested of the runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Edit {
    /// A v2 command (`cut_domain::Command` wire JSON).
    Legacy {
        command: Json,
    },
    Script {
        script: String,
    },
    Ops {
        ops: Vec<EditOp>,
    },
}

/// The outcome of an edit: the new project and, when every step has a
/// kernel equivalent, the operations it ran as script.
#[derive(Clone, Debug)]
pub(crate) struct Edited {
    pub project: Project,
    pub script: Option<String>,
}

impl Edit {
    pub fn fingerprint(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn run(&self, project: &Project) -> Result<Edited, RuntimeError> {
        let edited = self.run_unchecked(project)?;
        view(&edited.project)?;
        Ok(edited)
    }

    fn run_unchecked(&self, project: &Project) -> Result<Edited, RuntimeError> {
        match self {
            Self::Legacy { command } => apply_legacy(project, command),
            Self::Script { script } => {
                let applied = apply_script(project, catalog(), script)
                    .map_err(|errors| diagnostics_error("EDIT_REJECTED", &errors))?;
                Ok(Edited {
                    script: Some(print_ops(project, catalog(), &applied.ops)),
                    project: applied.project,
                })
            }
            Self::Ops { ops } => {
                let ops = lower_ops(project, ops)?;
                apply_ops(project, &ops)
            }
        }
    }

    /// A one-line description for history when the edit has no script.
    pub fn label(&self) -> String {
        match self {
            Self::Legacy { command } => {
                let kind = command
                    .get("type")
                    .and_then(Json::as_str)
                    .unwrap_or("command");
                format!("# v2 {kind}")
            }
            Self::Script { .. } => "# script".into(),
            Self::Ops { .. } => "# operations".into(),
        }
    }
}

fn apply_ops(project: &Project, ops: &[Op]) -> Result<Edited, RuntimeError> {
    let (next, _) = apply(project, catalog(), ops).map_err(rejected_error)?;
    Ok(Edited {
        script: Some(print_ops(project, catalog(), ops)),
        project: next,
    })
}

// ---- v2 commands -------------------------------------------------------------

pub(crate) fn apply_legacy(project: &Project, command: &Json) -> Result<Edited, RuntimeError> {
    let typed = v2::parse_command(command).map_err(domain_error)?;
    let mut state = Edited {
        project: project.clone(),
        script: Some(String::new()),
    };
    legacy_step(&mut state, &typed)?;
    Ok(state)
}

fn legacy_step(state: &mut Edited, command: &v2::Command) -> Result<(), RuntimeError> {
    if let v2::Command::Batch { commands } = command {
        for command in commands {
            legacy_step(state, command)?;
        }
        return Ok(());
    }
    match translate(&state.project, command) {
        Some(ops) => {
            let edited = apply_ops(&state.project, &ops)?;
            state.project = edited.project;
            if let (Some(script), Some(more)) = (&mut state.script, edited.script) {
                script.push_str(&more);
            }
        }
        None => {
            state.project = bridge(&state.project, command)?;
            state.script = None;
        }
    }
    Ok(())
}

/// Run `command` on the v2 export and import the result with `project` as
/// the hint. Refuses commands whose effect the model cannot hold.
fn bridge(project: &Project, command: &v2::Command) -> Result<Project, RuntimeError> {
    let edited = v2::apply_command(&view(project)?, command).map_err(domain_error)?;
    let imported = import_v2_with(&edited, Some(project))
        .map_err(|errors| RuntimeError::new("EDIT_REJECTED", errors.join("; ")))?;
    if !imported.dropped.is_empty() {
        return Err(RuntimeError::new(
            "NO_EQUIVALENT",
            format!(
                "the project cannot hold this change: {}",
                imported.dropped.join("; ")
            ),
        ));
    }
    Ok(imported.project)
}

/// The kernel operations equivalent to a v2 command, or `None` when the
/// command has to go through the v2 export.
fn translate(project: &Project, command: &v2::Command) -> Option<Vec<Op>> {
    let sequence = project.sequences.first()?;
    let rate = sequence.rate;
    let seconds = |value: f64| Time::from_seconds(value.max(0.0)).map(|time| rate.round(time));
    let clip = |id: &str| {
        let id = Id::new(id).ok()?;
        let span = exported_span(project, id.as_str())?;
        Some((id, span))
    };
    let track_of = |clip: &Id| {
        sequence
            .tracks
            .iter()
            .find(|track| track.clip(clip.as_str()).is_some())
    };
    Some(match command {
        v2::Command::RemoveClip { clip_id, .. } => {
            let (clip, _) = clip(clip_id.as_str())?;
            vec![Op::Delete { clips: vec![clip] }]
        }
        v2::Command::TrimClip {
            clip_id,
            edge,
            timeline_frame,
        } => {
            let (clip, (start, end)) = clip(clip_id.as_str())?;
            let target = rate.time_of(*timeline_frame);
            let (edge, delta) = match edge {
                v2::TrimEdge::Leading => (Edge::In, target - start),
                v2::TrimEdge::Trailing => (Edge::Out, target - end),
            };
            vec![Op::Trim {
                clip,
                edge,
                delta,
                ripple: false,
            }]
        }
        v2::Command::SplitClip {
            clip_id,
            at_frame,
            new_clip_id,
        } => {
            let (clip, _) = clip(clip_id.as_str())?;
            vec![Op::Split {
                clip,
                at: rate.time_of(*at_frame),
                tail: Some(Id::new(new_clip_id.as_str()).ok()?),
            }]
        }
        v2::Command::PlaceClip {
            clip_id,
            timeline_frame,
            track_id,
        } => {
            let (clip, (start, _)) = clip(clip_id.as_str())?;
            let current = track_of(&clip)?;
            let track = match track_id {
                Some(track) => {
                    let track = sequence.track(kernel_track(track.as_str()))?;
                    (track.id != current.id).then(|| track.id.clone())
                }
                None => None,
            };
            vec![Op::Move {
                clip,
                track,
                delta: rate.time_of(*timeline_frame) - start,
            }]
        }
        v2::Command::AddMarker { marker, .. } => {
            let color = match &marker.color {
                Some(color) => Some(MarkerColor::parse(&color.to_lowercase())?),
                None => None,
            };
            vec![Op::AddMarker {
                sequence: sequence.id.clone(),
                id: Some(Id::new(marker.id.as_str()).ok()?),
                at: seconds(marker.time)?,
                duration: seconds(marker.duration.unwrap_or(0.0))?,
                name: marker.label.clone(),
                color,
                note: None,
            }]
        }
        v2::Command::RemoveMarker { marker_id } => {
            let marker: &Marker = sequence.marker(marker_id)?;
            vec![Op::RemoveMarker {
                id: marker.id.clone(),
            }]
        }
        v2::Command::SetCanvas { width, height } => {
            let size = |value: f64| {
                (value.is_finite() && value >= 2.0 && value <= f64::from(u16::MAX))
                    .then(|| value.round() as u32)
            };
            vec![Op::SetSequence {
                sequence: sequence.id.clone(),
                change: SequenceChange {
                    size: Some((size(*width)?, size(*height)?)),
                    ..SequenceChange::default()
                },
            }]
        }
        v2::Command::AddTrack { track, .. } => {
            if !track.clips.is_empty()
                || track.gaps.as_ref().is_some_and(|g| !g.is_empty())
                || track.transitions.as_ref().is_some_and(|t| !t.is_empty())
                || track.effects.as_ref().is_some_and(|e| !e.is_empty())
                || track.markers.as_ref().is_some_and(|m| !m.is_empty())
            {
                return None;
            }
            let id = Id::new(track.id.as_str()).ok()?;
            let kind = match track.kind {
                v2::TrackType::Audio => TrackKind::Audio,
                _ => TrackKind::Video,
            };
            let mut ops = vec![Op::AddTrack {
                sequence: sequence.id.clone(),
                id: id.clone(),
                kind,
                index: None,
            }];
            let change = TrackChange {
                name: track.name.clone().map(Some),
                enabled: track.enabled,
                locked: track.locked,
                muted: track.muted,
            };
            if change != TrackChange::default() {
                ops.push(Op::SetTrack {
                    sequence: sequence.id.clone(),
                    track: id,
                    change,
                });
            }
            ops
        }
        v2::Command::RemoveTrack { track_id } => {
            let base = sequence.track(kernel_track(track_id.as_str()))?;
            if base.id != track_id.as_str() {
                return None;
            }
            let mut ops = Vec::new();
            if !base.clips.is_empty() {
                ops.push(Op::Delete {
                    clips: base.clips.iter().map(|c| c.id.clone()).collect(),
                });
            }
            ops.push(Op::RemoveTrack {
                sequence: sequence.id.clone(),
                track: base.id.clone(),
            });
            ops
        }
        v2::Command::AddSource { source, .. } => {
            let duration = seconds(source.duration.filter(|d| *d > 0.0)?)?;
            vec![Op::AddMedia(Media {
                id: Id::new(source.id.as_str()).ok()?,
                path: source.path.clone(),
                rate,
                drop_frame: false,
                start: Time::ZERO,
                duration: duration.max(rate.frame_duration()),
                // Like the v2 import, media without a recorded frame size is
                // assumed to be picture at the sequence size.
                video: Some(match source.width.zip(source.height) {
                    Some((w, h)) => VideoInfo {
                        width: w.round() as u32,
                        height: h.round() as u32,
                    },
                    None => VideoInfo {
                        width: sequence.width,
                        height: sequence.height,
                    },
                }),
                audio: (source.has_audio == Some(true)).then_some(cut_model::AudioInfo {
                    channels: 2,
                    sample_rate: 48_000,
                }),
                reel: None,
            })]
        }
        v2::Command::RemoveSource { source_id } => {
            let media = project.media(source_id.as_str())?;
            vec![Op::RemoveMedia {
                media: media.id.clone(),
            }]
        }
        v2::Command::AddClip { track_id, clip, .. } => add_clip(project, track_id.as_str(), clip)?,
        _ => return None,
    })
}

/// A plain media clip added to a track. On a picture track, media with
/// sound also gets a linked clip on a free audio track, which is how v2
/// played a picture clip's embedded sound.
fn add_clip(project: &Project, track_id: &str, clip: &v2::Clip) -> Option<Vec<Op>> {
    let value = serde_json::to_value(clip).ok()?;
    let plain = value.as_object()?.keys().all(|key| {
        matches!(
            key.as_str(),
            "id" | "sourceId" | "in" | "out" | "timelineStart" | "duration"
        )
    });
    if !plain {
        return None;
    }
    let sequence = project.sequences.first()?;
    let rate = sequence.rate;
    let seconds = |value: f64| Time::from_seconds(value.max(0.0)).map(|time| rate.round(time));
    let track = sequence.track(kernel_track(track_id))?;
    if track.id != track_id || track.kind == TrackKind::Subtitle {
        return None;
    }
    let media = project.media(clip.source_id.as_ref()?.as_str())?;
    let at = seconds(clip.timeline_start)?;
    let duration = seconds(clip.timeline_duration()?)?.max(rate.frame_duration());
    let content = Content::Media {
        media: media.id.clone(),
        source_in: seconds(clip.source_in().unwrap_or(0.0))?,
        speed: Ratio::ONE,
    };
    let mut targets = vec![(
        track.id.clone(),
        NewClip {
            id: Some(Id::new(clip.id.as_str()).ok()?),
            ..NewClip::new(content.clone())
        },
    )];
    let mut ops = Vec::new();
    if track.kind == TrackKind::Video && media.audio.is_some() {
        let free = sequence.tracks.iter().find(|candidate| {
            candidate.kind == TrackKind::Audio
                && !candidate.locked
                && candidate.is_free(at, at + duration, &[])
        });
        let audio = match free {
            Some(track) => track.id.clone(),
            None => {
                let id = (1..)
                    .map(|n| format!("A{n}"))
                    .find(|name| sequence.track(name).is_none())
                    .and_then(|name| Id::new(name).ok())?;
                ops.push(Op::AddTrack {
                    sequence: sequence.id.clone(),
                    id: id.clone(),
                    kind: TrackKind::Audio,
                    index: None,
                });
                id
            }
        };
        targets.push((audio, NewClip::new(content)));
    }
    ops.push(Op::Overwrite {
        sequence: sequence.id.clone(),
        at,
        duration,
        targets,
    });
    Some(ops)
}

// ---- typed operations ----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrimSide {
    In,
    Out,
}

/// A typed edit for the editor. Amounts and positions count frames of the
/// sequence: `frames` is a signed amount, `at` a frame offset from the
/// sequence's first frame. `sourceIn` counts frames of the media. Generated
/// ids follow the language's rules; `ids`/`tail` name new clips explicitly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum EditOp {
    Trim {
        clip: String,
        edge: TrimSide,
        frames: i64,
        #[serde(default)]
        ripple: bool,
    },
    Roll {
        clip: String,
        frames: i64,
    },
    Slip {
        clip: String,
        frames: i64,
    },
    Slide {
        clip: String,
        frames: i64,
    },
    Move {
        clip: String,
        #[serde(default)]
        frames: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<String>,
    },
    Split {
        clip: String,
        at: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail: Option<String>,
    },
    Blade {
        at: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tracks: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    Delete {
        clips: Vec<String>,
    },
    RippleDelete {
        clips: Vec<String>,
    },
    /// Static property or generator parameter values; `null` restores the
    /// default.
    Set {
        clip: String,
        props: BTreeMap<String, Json>,
    },
    Speed {
        clip: String,
        speed: String,
    },
    Insert {
        media: String,
        source_in: i64,
        frames: i64,
        at: i64,
        tracks: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    Overwrite {
        media: String,
        source_in: i64,
        frames: i64,
        at: i64,
        tracks: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    AddMarker {
        at: i64,
        #[serde(default)]
        frames: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    RemoveMarker {
        id: String,
    },
    Link {
        clips: Vec<String>,
    },
    Unlink {
        clips: Vec<String>,
    },
    SetTrack {
        track: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locked: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
    },
}

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new("INVALID_EDIT", message)
}

fn edit_id(text: &str) -> Result<Id, RuntimeError> {
    Id::new(text).map_err(|error| invalid(error.to_string()))
}

fn edit_ids(texts: &[String]) -> Result<Vec<Id>, RuntimeError> {
    texts.iter().map(|text| edit_id(text)).collect()
}

/// Lower typed operations to kernel operations. Each one is read against the
/// project as the earlier ones leave it, so later operations may name clips
/// that earlier ones created.
pub(crate) fn lower_ops(project: &Project, ops: &[EditOp]) -> Result<Vec<Op>, RuntimeError> {
    let mut state = project.clone();
    let mut lowered = Vec::with_capacity(ops.len());
    for op in ops {
        let op = lower(&state, op)?;
        state = apply(&state, catalog(), std::slice::from_ref(&op))
            .map_err(rejected_error)?
            .0;
        lowered.push(op);
    }
    Ok(lowered)
}

fn lower(project: &Project, op: &EditOp) -> Result<Op, RuntimeError> {
    let clip_rate = |clip: &str| {
        project
            .locate(clip)
            .map(|at| project.sequences[at.sequence].rate)
            .ok_or_else(|| invalid(format!("no clip `{clip}`")))
    };
    let sequence = |id: &Option<String>| match id {
        Some(id) => project
            .sequence(id)
            .ok_or_else(|| invalid(format!("no sequence `{id}`"))),
        None => project
            .sequences
            .first()
            .ok_or_else(|| invalid("the project has no sequence")),
    };
    Ok(match op {
        EditOp::Trim {
            clip,
            edge,
            frames,
            ripple,
        } => Op::Trim {
            clip: edit_id(clip)?,
            edge: match edge {
                TrimSide::In => Edge::In,
                TrimSide::Out => Edge::Out,
            },
            delta: clip_rate(clip)?.time_of(*frames),
            ripple: *ripple,
        },
        EditOp::Roll { clip, frames } => Op::Roll {
            clip: edit_id(clip)?,
            delta: clip_rate(clip)?.time_of(*frames),
        },
        EditOp::Slip { clip, frames } => Op::Slip {
            clip: edit_id(clip)?,
            delta: clip_rate(clip)?.time_of(*frames),
        },
        EditOp::Slide { clip, frames } => Op::Slide {
            clip: edit_id(clip)?,
            delta: clip_rate(clip)?.time_of(*frames),
        },
        EditOp::Move {
            clip,
            frames,
            track,
        } => Op::Move {
            clip: edit_id(clip)?,
            track: track.as_deref().map(edit_id).transpose()?,
            delta: clip_rate(clip)?.time_of(*frames),
        },
        EditOp::Split { clip, at, tail } => Op::Split {
            clip: edit_id(clip)?,
            at: clip_rate(clip)?.time_of(*at),
            tail: tail.as_deref().map(edit_id).transpose()?,
        },
        EditOp::Blade {
            at,
            tracks,
            sequence: id,
        } => {
            let sequence = sequence(id)?;
            Op::Blade {
                sequence: sequence.id.clone(),
                at: sequence.rate.time_of(*at),
                tracks: tracks.as_deref().map(edit_ids).transpose()?,
            }
        }
        EditOp::Delete { clips } => Op::Delete {
            clips: edit_ids(clips)?,
        },
        EditOp::RippleDelete { clips } => Op::RippleDelete {
            clips: edit_ids(clips)?,
        },
        EditOp::Set { clip, props } => {
            let at = project
                .locate(clip)
                .ok_or_else(|| invalid(format!("no clip `{clip}`")))?;
            let target = project.clip_at(at);
            let domain = project.sequences[at.sequence].tracks[at.track]
                .kind
                .domain();
            let mut changes = Vec::new();
            for (name, value) in props {
                let def = domain
                    .and_then(|domain| catalog().prop(domain, name))
                    .or_else(|| match &target.content {
                        Content::Generator { fx, .. } => {
                            catalog().fx(fx.as_str()).and_then(|def| def.find(name))
                        }
                        _ => None,
                    })
                    .ok_or_else(|| invalid(format!("clip `{clip}` has no property `{name}`")))?;
                let param = match value {
                    Json::Null => None,
                    value => Some(Param::Static(json_value(&def.ty, value).ok_or_else(
                        || invalid(format!("`{name}` expects a {}", def.ty.name())),
                    )?)),
                };
                changes.push(ClipChange::Param(name.clone(), param));
            }
            Op::Set {
                clip: edit_id(clip)?,
                changes,
            }
        }
        EditOp::Speed { clip, speed } => Op::Speed {
            clip: edit_id(clip)?,
            speed: Ratio::parse(speed).map_err(|error| invalid(error.to_string()))?,
        },
        EditOp::Insert {
            media,
            source_in,
            frames,
            at,
            tracks,
            ids,
            sequence: id,
        }
        | EditOp::Overwrite {
            media,
            source_in,
            frames,
            at,
            tracks,
            ids,
            sequence: id,
        } => {
            let sequence = sequence(id)?;
            let found = project
                .media(media)
                .ok_or_else(|| invalid(format!("no media `{media}`")))?;
            if ids.len() > tracks.len() {
                return Err(invalid("give at most one id per track"));
            }
            let content = Content::Media {
                media: found.id.clone(),
                source_in: found.rate.time_of(*source_in),
                speed: Ratio::ONE,
            };
            let mut targets = Vec::new();
            for (index, track) in tracks.iter().enumerate() {
                let mut clip = NewClip::new(content.clone());
                clip.id = ids.get(index).map(|id| edit_id(id)).transpose()?;
                targets.push((edit_id(track)?, clip));
            }
            let (at, duration) = (sequence.rate.time_of(*at), sequence.rate.time_of(*frames));
            if matches!(op, EditOp::Insert { .. }) {
                Op::Insert {
                    sequence: sequence.id.clone(),
                    at,
                    duration,
                    targets,
                }
            } else {
                Op::Overwrite {
                    sequence: sequence.id.clone(),
                    at,
                    duration,
                    targets,
                }
            }
        }
        EditOp::AddMarker {
            at,
            frames,
            id,
            name,
            color,
            note,
            sequence: sequence_id,
        } => {
            let sequence = sequence(sequence_id)?;
            Op::AddMarker {
                sequence: sequence.id.clone(),
                id: id.as_deref().map(edit_id).transpose()?,
                at: sequence.rate.time_of(*at),
                duration: sequence.rate.time_of(*frames),
                name: name.clone(),
                color: color
                    .as_deref()
                    .map(|color| {
                        MarkerColor::parse(color)
                            .ok_or_else(|| invalid(format!("`{color}` is not a marker color")))
                    })
                    .transpose()?,
                note: note.clone(),
            }
        }
        EditOp::RemoveMarker { id } => Op::RemoveMarker { id: edit_id(id)? },
        EditOp::Link { clips } => Op::Link {
            clips: edit_ids(clips)?,
            group: None,
        },
        EditOp::Unlink { clips } => Op::Unlink {
            clips: edit_ids(clips)?,
        },
        EditOp::SetTrack {
            track,
            sequence: id,
            enabled,
            locked,
            muted,
        } => Op::SetTrack {
            sequence: sequence(id)?.id.clone(),
            track: edit_id(track)?,
            change: TrackChange {
                name: None,
                enabled: *enabled,
                locked: *locked,
                muted: *muted,
            },
        },
    })
}

fn json_value(ty: &ParamType, value: &Json) -> Option<Value> {
    Some(match (ty, value) {
        (ParamType::Number, Json::Number(n)) => Value::Number(n.as_f64()?),
        (ParamType::Bool, Json::Bool(b)) => Value::Bool(*b),
        (ParamType::Text, Json::String(s)) => Value::Text(s.clone()),
        (ParamType::Enum(_), Json::String(s)) => Value::Enum(s.clone()),
        (ParamType::Color, Json::String(s)) => Value::Color(cut_model::Rgba::parse(s)?),
        _ => return None,
    })
}
