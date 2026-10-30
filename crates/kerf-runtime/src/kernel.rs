//! The runtime's edits on the kernel model.
//!
//! The canonical project is a `kerf_model::Project`. Every change reaches it
//! as an edit script or as typed operations from the editor, and is
//! journaled as the script of the operations it ran.

use crate::RuntimeError;
use kerf_engine::ProbeInfo;
use kerf_lang::{
    apply_script_with, parse_document, print_document, print_ops, Breaks, Facts, PrintOptions,
    Speech,
};
use kerf_model::{
    apply, AudioInfo, Catalog, ClipChange, Content, Diagnostic, Edge, Effect, FxDef, Id,
    MarkerColor, Mask, MaskMode, MaskShape, Media, NewClip, Op, Param, ParamType, Project, Rate,
    Ratio, Rejected, Sequence, SequenceChange, Time, TrackChange, TrackKind, Value, VideoInfo,
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
        .map_or(Time::ZERO, kerf_model::Sequence::duration)
}

/// The sequence a render plays: `id`, or the first.
pub(crate) fn render_sequence<'a>(
    project: &'a Project,
    id: Option<&str>,
) -> Result<&'a kerf_model::Sequence, RuntimeError> {
    match id {
        Some(id) => project
            .sequence(id)
            .ok_or_else(|| RuntimeError::new("UNKNOWN_SEQUENCE", format!("no sequence `{id}`"))),
        None => project.sequences.first().ok_or_else(|| {
            RuntimeError::new("INVALID_RENDER_RANGE", "the project has no sequence")
        }),
    }
}

/// `[start, end)` seconds of `sequence` as frame-aligned times.
pub(crate) fn time_range(
    sequence: &kerf_model::Sequence,
    start: f64,
    end: f64,
) -> Result<(Time, Time), RuntimeError> {
    let rate = sequence.rate;
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

/// Make relative paths absolute against `directory`: media and the files
/// effects read, such as LUTs. Documents keep paths relative to the
/// project so it can move; renderers and players run elsewhere and need
/// them resolved.
pub fn resolve_media_paths(project: &mut Project, directory: &std::path::Path) {
    project.resolve_paths(catalog(), directory);
}

/// Point media at their proxies where `directory`'s cache has one. Paths
/// must already be resolved. For drafts: a proxy is smaller, frame for
/// frame.
pub fn use_proxies(project: &mut Project, directory: &std::path::Path) {
    let cache = kerf_engine::derivatives::DerivativeCache::new(directory);
    for media in &mut project.media {
        if let Some(proxy) = kerf_engine::cached_proxy(std::path::Path::new(&media.path), &cache) {
            media.path = proxy.to_string_lossy().into_owned();
        }
    }
}

/// What analysis already made of a project's media, read from the
/// project's cache when a script line asks. Nothing is analysed here.
pub struct CachedFacts {
    directory: std::path::PathBuf,
}

impl CachedFacts {
    pub fn new(directory: impl Into<std::path::PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
}

impl Facts for CachedFacts {
    /// The words transcripts already made of `media` heard, and the ranges
    /// they cover. Nothing is transcribed.
    fn speech(&self, media: &Media) -> Speech {
        let path = self.directory.join(&media.path);
        let cache = kerf_engine::derivatives::DerivativeCache::new(&self.directory);
        let transcripts =
            kerf_engine::cached_transcripts(&path.to_string_lossy(), &cache).unwrap_or_default();
        Speech {
            covered: transcripts
                .iter()
                .map(|transcript| (transcript.range.start, transcript.range.end))
                .collect(),
            words: transcripts
                .into_iter()
                .flat_map(|transcript| transcript.cues)
                .flat_map(|cue| cue.words)
                .map(|word| kerf_lang::Word {
                    start: word.start,
                    end: word.end,
                    text: word.text.trim().to_owned(),
                })
                .collect(),
        }
    }

    /// Where the media index made on import found pauses and cuts. Nothing
    /// is indexed here.
    fn breaks(&self, media: &Media) -> Option<Breaks> {
        let path = self.directory.join(&media.path);
        let cache = kerf_engine::derivatives::DerivativeCache::new(&self.directory);
        let index = kerf_engine::cached_media_index(&path, &cache)?;
        Some(Breaks {
            pauses: index.pauses,
            cuts: index.cuts,
        })
    }
}

/// Index `path` and make its proxy, in the background, so agents and
/// drafts find them ready. Failures leave nothing behind; the work is
/// done again when it is asked for.
pub(crate) fn prepare_media(path: String, directory: std::path::PathBuf) {
    std::thread::spawn(move || {
        let cache = kerf_engine::derivatives::DerivativeCache::new(&directory);
        let cancellation = kerf_engine::CancellationToken::new();
        let path = std::path::Path::new(&path);
        let _ = kerf_engine::media_index(path, &cache, &cancellation);
        let _ = kerf_engine::make_proxy(path, &cache, &cancellation);
    });
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

/// A new project: one empty 1920x1080, 30 fps sequence.
pub(crate) fn empty_project() -> Project {
    let mut project = Project::new("Untitled project");
    project.sequences.push(Sequence {
        id: Id::new("s1").expect("valid id"),
        name: project.name.clone(),
        rate: Rate::FPS_30,
        drop_frame: false,
        width: 1920,
        height: 1080,
        sample_rate: 48_000,
        start: Time::ZERO,
        tracks: Vec::new(),
        markers: Vec::new(),
    });
    project
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
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Edit {
    Script { script: String },
    Ops { ops: Vec<EditOp> },
}

/// The outcome of an edit: the new project and the operations it ran as
/// script.
#[derive(Clone, Debug)]
pub(crate) struct Edited {
    pub project: Project,
    pub script: String,
}

impl Edit {
    pub fn fingerprint(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Run the edit on `project`; script lines that read analysis read it
    /// from `facts`.
    pub fn run(&self, project: &Project, facts: &dyn Facts) -> Result<Edited, RuntimeError> {
        match self {
            Self::Script { script } => {
                let applied = apply_script_with(project, catalog(), facts, script)
                    .map_err(|errors| diagnostics_error("EDIT_REJECTED", &errors))?;
                Ok(Edited {
                    script: print_ops(project, catalog(), &applied.ops),
                    project: applied.project,
                })
            }
            Self::Ops { ops } => {
                let ops = lower_ops(project, ops)?;
                apply_ops(project, &ops)
            }
        }
    }
}

pub(crate) fn apply_ops(project: &Project, ops: &[Op]) -> Result<Edited, RuntimeError> {
    let (next, _) = apply(project, catalog(), ops).map_err(rejected_error)?;
    Ok(Edited {
        script: print_ops(project, catalog(), ops),
        project: next,
    })
}

// ---- media import -------------------------------------------------------------

/// The operations that add the media file at `path` (described by `probe`
/// when it could be read) and cut all of it in at the end of the program.
/// Picture goes on the first video track and sound, linked to it, on the
/// first audio track free there; missing tracks are added. Returns the
/// operations and the ids of the media and of its first clip.
pub(crate) fn import_media(
    project: &Project,
    path: &str,
    probe: Option<&ProbeInfo>,
) -> Result<(Vec<Op>, Id, Id), RuntimeError> {
    let sequence = project
        .sequences
        .first()
        .ok_or_else(|| RuntimeError::new("INVALID_IMPORT", "the project has no sequence"))?;
    let rate = probe
        .and_then(|info| info.fps)
        .and_then(|fps| Rate::from_fps(fps).ok())
        .unwrap_or(sequence.rate);
    let has_audio = probe.and_then(|info| info.has_audio) == Some(true);
    let video = match probe.and_then(|info| info.width.zip(info.height)) {
        Some((width, height)) => Some(VideoInfo {
            width,
            height,
            alpha: probe.and_then(|info| info.alpha) == Some(true),
        }),
        // Unprobed media is assumed to be picture at the sequence size.
        None if !has_audio => Some(VideoInfo {
            width: sequence.width,
            height: sequence.height,
            alpha: false,
        }),
        None => None,
    };
    // Unprobed media is assumed to last one second.
    let seconds = probe
        .and_then(|info| info.duration)
        .filter(|duration| *duration > 0.0)
        .unwrap_or(1.0);
    let duration = Time::from_seconds(seconds)
        .map(|time| rate.round(time).max(rate.frame_duration()))
        .ok_or_else(|| RuntimeError::new("INVALID_IMPORT", "the media is too long"))?;
    let mut ids = project.id_gen();
    let media = Media {
        id: ids.next("m"),
        path: path.to_owned(),
        rate,
        drop_frame: false,
        start: Time::ZERO,
        duration,
        video,
        audio: has_audio.then_some(AudioInfo {
            channels: 2,
            sample_rate: 48_000,
        }),
        reel: None,
    };
    let at = program_duration(project);
    let length = sequence
        .rate
        .round(duration)
        .max(sequence.rate.frame_duration());
    let content = Content::Media {
        media: media.id.clone(),
        source_in: Time::ZERO,
        speed: Ratio::ONE,
    };
    let clip = ids.next("c");
    let mut ops = vec![Op::AddMedia(media.clone())];
    let track_for = |kind: TrackKind, ops: &mut Vec<Op>| {
        let found = sequence.tracks.iter().find(|track| {
            track.kind == kind
                && !track.locked
                && (kind == TrackKind::Video || track.is_free(at, at + length, &[]))
        });
        match found {
            Some(track) => track.id.clone(),
            None => {
                let id = new_track_id(sequence, kind);
                ops.push(Op::AddTrack {
                    sequence: sequence.id.clone(),
                    id: id.clone(),
                    kind,
                    index: None,
                });
                id
            }
        }
    };
    let mut targets = Vec::new();
    for (kind, present) in [
        (TrackKind::Video, media.video.is_some()),
        (TrackKind::Audio, media.audio.is_some()),
    ] {
        if present {
            let track = track_for(kind, &mut ops);
            let mut placed = NewClip::new(content.clone());
            if targets.is_empty() {
                placed.id = Some(clip.clone());
            }
            targets.push((track, placed));
        }
    }
    ops.push(Op::Overwrite {
        sequence: sequence.id.clone(),
        at,
        duration: length,
        targets,
    });
    Ok((ops, media.id, clip))
}

/// The first free `V<n>`, `A<n>`, or `S<n>` track id of `sequence`.
fn new_track_id(sequence: &Sequence, kind: TrackKind) -> Id {
    let prefix = match kind {
        TrackKind::Video => "V",
        TrackKind::Audio => "A",
        TrackKind::Subtitle => "S",
    };
    (1..)
        .map(|n| format!("{prefix}{n}"))
        .find(|name| sequence.track(name).is_none())
        .and_then(|name| Id::new(name).ok())
        .expect("a free track id")
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
    /// Add an effect to the end of the clip's stack, or at `index`, with
    /// static parameter values; the rest stay at their defaults.
    FxAdd {
        clip: String,
        fx: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Turn the effect at `index` on or off, or set its parameters; `null`
    /// restores a parameter's default.
    FxSet {
        clip: String,
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
    },
    FxRemove {
        clip: String,
        index: usize,
    },
    /// Add a mask to a picture clip, after its others or at `index`. Its
    /// name is the first of `k1`, `k2`, … the clip does not use unless
    /// `name` gives one; `params` sets feather, opacity, and expansion.
    MaskAdd {
        clip: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(flatten)]
        outline: MaskOutline,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inverted: Option<bool>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Replace a mask's outline, or change its mode, inversion, or
    /// properties; `null` restores a property's default.
    MaskSet {
        clip: String,
        name: String,
        #[serde(flatten)]
        outline: MaskOutline,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inverted: Option<bool>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
    },
    MaskRemove {
        clip: String,
        name: String,
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
    /// An empty track after the others of its kind (`video`, `audio`, or
    /// `subtitle`), with a generated id unless `id` names it.
    AddTrack {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// The frame size of a sequence.
    SetSequence {
        width: u32,
        height: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
}

/// A mask's outline, as the language writes it: at most one of `rect`
/// (left, top, right, bottom), `ellipse` (center x and y, then the radius
/// across and down), or `points` (corners in order, closed unless `closed`
/// is false), in fractions of the clip's picture with 0, 0 at its top left.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MaskOutline {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ellipse: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
}

impl MaskOutline {
    /// The outline it gives, or `None` when it gives none.
    fn shape(&self) -> Result<Option<MaskShape>, RuntimeError> {
        let given = [
            self.rect.is_some(),
            self.ellipse.is_some(),
            self.points.is_some(),
        ];
        if given.into_iter().filter(|given| *given).count() > 1 {
            return Err(invalid("a mask has one outline: rect, ellipse, or points"));
        }
        if self.closed.is_some() && self.points.is_none() {
            return Err(invalid("`closed` goes with `points`"));
        }
        Ok(match (self.rect, self.ellipse, &self.points) {
            (Some(rect), _, _) => Some(MaskShape::Rect(rect)),
            (_, Some(ellipse), _) => Some(MaskShape::Ellipse(ellipse)),
            (_, _, Some(points)) => Some(MaskShape::Polygon {
                points: points.clone(),
                closed: self.closed.unwrap_or(true),
            }),
            _ => None,
        })
    }
}

fn mask_mode(mode: &Option<String>) -> Result<Option<MaskMode>, RuntimeError> {
    mode.as_deref()
        .map(|mode| {
            MaskMode::parse(mode).ok_or_else(|| invalid(format!("`{mode}` is not a mask mode")))
        })
        .transpose()
}

/// A mask's properties from JSON, as `fx_params` reads an effect's.
fn mask_params(
    params: &BTreeMap<String, Json>,
) -> Result<Vec<(String, Option<Param>)>, RuntimeError> {
    params
        .iter()
        .map(|(name, value)| {
            let def = catalog()
                .mask_prop(name)
                .ok_or_else(|| invalid(format!("a mask has no property `{name}`")))?;
            let value = match value {
                Json::Null => None,
                value => Some(Param::Static(json_value(&def.ty, value).ok_or_else(
                    || invalid(format!("`{name}` expects a {}", def.ty.name())),
                )?)),
            };
            Ok((name.clone(), value))
        })
        .collect()
}

/// The first of `k1`, `k2`, … that none of the clip's masks is named.
fn new_mask_name(project: &Project, clip: &str) -> Result<Id, RuntimeError> {
    let target = project
        .clip(clip)
        .ok_or_else(|| invalid(format!("no clip `{clip}`")))?;
    let name = (1..)
        .map(|n| format!("k{n}"))
        .find(|name| target.mask(name).is_none())
        .expect("a free mask name");
    edit_id(&name)
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
        let ops = lower(&state, op)?;
        state = apply(&state, catalog(), &ops).map_err(rejected_error)?.0;
        lowered.extend(ops);
    }
    Ok(lowered)
}

fn lower(project: &Project, op: &EditOp) -> Result<Vec<Op>, RuntimeError> {
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
    Ok(vec![match op {
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
        EditOp::FxAdd {
            clip,
            fx,
            params,
            index,
        } => {
            let def = catalog()
                .fx(fx)
                .ok_or_else(|| invalid(format!("no effect `{fx}`")))?;
            let params = fx_params(def, params)?
                .into_iter()
                .filter_map(|(name, param)| param.map(|param| (name, param)))
                .collect();
            Op::FxAdd {
                clip: edit_id(clip)?,
                effect: Effect {
                    fx: def.id.clone(),
                    version: def.version,
                    enabled: true,
                    params,
                },
                index: *index,
            }
        }
        EditOp::FxSet {
            clip,
            index,
            enabled,
            params,
        } => {
            let effect = project
                .clip(clip)
                .and_then(|found| found.effects.get(*index))
                .ok_or_else(|| invalid(format!("clip `{clip}` has no effect {index}")))?;
            let def = catalog()
                .fx(effect.fx.as_str())
                .ok_or_else(|| invalid(format!("no effect `{}`", effect.fx)))?;
            Op::FxSet {
                clip: edit_id(clip)?,
                index: *index,
                enabled: *enabled,
                params: fx_params(def, params)?,
            }
        }
        EditOp::FxRemove { clip, index } => Op::FxRemove {
            clip: edit_id(clip)?,
            index: *index,
        },
        // Outlines and values are checked with the rest of the project.
        EditOp::MaskAdd {
            clip,
            name,
            outline,
            mode,
            inverted,
            params,
            index,
        } => {
            let name = match name {
                Some(name) => edit_id(name)?,
                None => new_mask_name(project, clip)?,
            };
            let shape = outline
                .shape()?
                .ok_or_else(|| invalid("a mask needs rect, ellipse, or points"))?;
            Op::MaskAdd {
                clip: edit_id(clip)?,
                mask: Mask {
                    name,
                    shape,
                    mode: mask_mode(mode)?.unwrap_or_default(),
                    inverted: inverted.unwrap_or(false),
                    params: mask_params(params)?
                        .into_iter()
                        .filter_map(|(name, param)| param.map(|param| (name, param)))
                        .collect(),
                },
                index: *index,
            }
        }
        EditOp::MaskSet {
            clip,
            name,
            outline,
            mode,
            inverted,
            params,
        } => Op::MaskSet {
            clip: edit_id(clip)?,
            name: edit_id(name)?,
            shape: outline.shape()?,
            mode: mask_mode(mode)?,
            inverted: *inverted,
            params: mask_params(params)?,
        },
        EditOp::MaskRemove { clip, name } => Op::MaskRemove {
            clip: edit_id(clip)?,
            name: edit_id(name)?,
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
        EditOp::AddTrack {
            kind,
            id,
            name,
            sequence: sequence_id,
        } => {
            let sequence = sequence(sequence_id)?;
            let kind = TrackKind::parse(kind)
                .ok_or_else(|| invalid(format!("`{kind}` is not a track kind")))?;
            let id = match id {
                Some(id) => edit_id(id)?,
                None => new_track_id(sequence, kind),
            };
            let mut ops = vec![Op::AddTrack {
                sequence: sequence.id.clone(),
                id: id.clone(),
                kind,
                index: None,
            }];
            if let Some(name) = name {
                ops.push(Op::SetTrack {
                    sequence: sequence.id.clone(),
                    track: id,
                    change: TrackChange {
                        name: Some(Some(name.clone())),
                        ..TrackChange::default()
                    },
                });
            }
            return Ok(ops);
        }
        EditOp::SetSequence {
            width,
            height,
            sequence: id,
        } => Op::SetSequence {
            sequence: sequence(id)?.id.clone(),
            change: SequenceChange {
                size: Some((*width, *height)),
                ..SequenceChange::default()
            },
        },
    }])
}

/// An effect's parameters from JSON: a static value each, or `None` (for
/// `null`) to restore the default.
fn fx_params(
    def: &FxDef,
    params: &BTreeMap<String, Json>,
) -> Result<Vec<(String, Option<Param>)>, RuntimeError> {
    params
        .iter()
        .map(|(name, value)| {
            let param = def
                .find(name)
                .ok_or_else(|| invalid(format!("`{}` has no parameter `{name}`", def.id)))?;
            let value = match value {
                Json::Null => None,
                value => Some(Param::Static(json_value(&param.ty, value).ok_or_else(
                    || invalid(format!("`{name}` expects a {}", param.ty.name())),
                )?)),
            };
            Ok((name.clone(), value))
        })
        .collect()
}

fn json_value(ty: &ParamType, value: &Json) -> Option<Value> {
    Some(match (ty, value) {
        (ParamType::Number | ParamType::Time, Json::Number(n)) => Value::Number(n.as_f64()?),
        (ParamType::Bool, Json::Bool(b)) => Value::Bool(*b),
        (
            ParamType::Text
            | ParamType::File(_)
            | ParamType::Track
            | ParamType::Sound
            | ParamType::Mask
            // A time given as text names a marker to land on.
            | ParamType::Time,
            Json::String(s),
        ) => Value::Text(s.clone()),
        (ParamType::Enum(_), Json::String(s)) => Value::Enum(s.clone()),
        (ParamType::Color, Json::String(s)) => Value::Color(kerf_model::Rgba::parse(s)?),
        _ => return None,
    })
}
