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
    apply, track_role, AudioInfo, Catalog, ClipChange, Content, Diagnostic, Domain, Edge, Effect,
    FxDef, FxKind, Id, Interp, Keyframe, MarkerChange, MarkerColor, Mask, MaskMode, MaskShape,
    Media, NewClip, Op, Param, ParamDef, ParamType, Project, Rate, Ratio, Rejected, Sequence,
    SequenceChange, Time, TrackChange, TrackKind, TrackRoleChange, Value,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

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

    /// Where the sound the transcripts kept reads as speech and they have no
    /// words. Nothing is transcribed.
    fn unheard(&self, media: &Media) -> Vec<(f64, f64)> {
        let path = self.directory.join(&media.path);
        let cache = kerf_engine::derivatives::DerivativeCache::new(&self.directory);
        kerf_engine::unheard_speech(&path.to_string_lossy(), &cache)
            .into_iter()
            .map(|stretch| (stretch.start, stretch.end))
            .collect()
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
///
/// Files wait their turn: a few workers prepare them one at a time each,
/// so importing a folder of clips does not start an FFmpeg for every one
/// at once. A file already waiting or being prepared is not queued again.
/// Each file is indexed first and goes back to the end of the queue for
/// its proxy, so the last file of a folder is indexed without waiting for
/// every proxy before it.
pub(crate) fn prepare_media(path: String, directory: PathBuf) {
    let path = PathBuf::from(path);
    let file = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let mut queue = media_queue();
    if !queue.pending.insert((directory.clone(), file.clone())) {
        return;
    }
    let cancellation = queue
        .cancellations
        .entry(directory.clone())
        .or_default()
        .clone();
    queue.waiting.push_back(MediaJob {
        // Counted before the request that asked for it answers, so an
        // idle kerfd never sees a moment with neither.
        _work: BackgroundWork::start(&directory),
        directory,
        file,
        path,
        cancellation,
        indexed: false,
    });
    if queue.workers >= media_workers() {
        return;
    }
    queue.workers += 1;
    let spawned = std::thread::Builder::new()
        .name("kerf-media".into())
        .spawn(prepare_queued_media);
    if spawned.is_err() {
        queue.workers -= 1;
        if queue.workers == 0 {
            // Nothing would ever take what waits, and it would keep kerfd
            // running; the work is done again when it is asked for.
            let waiting: Vec<MediaJob> = queue.waiting.drain(..).collect();
            for job in &waiting {
                queue.forget(job);
            }
            drop(queue);
            drop(waiting);
        }
    }
}

/// Stop preparing media for `directory`, waiting up to `wait` for the
/// FFmpeg running for it to be killed. kerfd calls this as it exits: its
/// sidecars run in process groups of their own, which would otherwise
/// outlive it. Media imported later is prepared afresh.
pub(crate) fn stop_media(directory: &Path, wait: std::time::Duration) {
    let dropped: VecDeque<MediaJob> = {
        let mut queue = media_queue();
        if let Some(cancellation) = queue.cancellations.remove(directory) {
            cancellation.cancel();
        }
        let (dropped, kept) = queue
            .waiting
            .drain(..)
            .partition(|job| job.directory == directory);
        queue.waiting = kept;
        for job in &dropped {
            queue.forget(job);
        }
        dropped
    };
    drop(dropped);
    let deadline = Instant::now() + wait;
    while media_queue().preparing(directory) && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// How many files are prepared at once. Each FFmpeg already uses several
/// cores.
fn media_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |cores| (cores.get() / 4).clamp(1, 2))
}

/// Prepare queued files until none is left.
fn prepare_queued_media() {
    loop {
        let mut job = {
            let mut queue = media_queue();
            let Some(job) = queue.waiting.pop_front() else {
                queue.workers -= 1;
                return;
            };
            job
        };
        // A panic in one file's work must not stop the queue, or what
        // waits behind it would keep kerfd running forever.
        let indexed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let cache = kerf_engine::derivatives::DerivativeCache::new(&job.directory);
            if job.indexed {
                let _ = kerf_engine::make_proxy(&job.path, &cache, &job.cancellation);
            } else {
                let _ = kerf_engine::media_index(&job.path, &cache, &job.cancellation);
            }
        }))
        .is_ok()
            && !job.indexed;
        let mut queue = media_queue();
        // Checked under the lock: stop_media cancels under it before it
        // drops what waits, so a job put back here is either dropped by it
        // or was never stopped.
        if indexed && !job.cancellation.is_cancelled() {
            job.indexed = true;
            queue.waiting.push_back(job);
        } else {
            queue.forget(&job);
            drop(queue);
            drop(job);
        }
    }
}

/// One file to prepare.
struct MediaJob {
    directory: PathBuf,
    /// The file as the import named it, which is how the cache knows it.
    path: PathBuf,
    /// The file itself, which keeps it from being queued twice.
    file: PathBuf,
    cancellation: kerf_engine::CancellationToken,
    /// Its index is made, and its proxy is next.
    indexed: bool,
    _work: BackgroundWork,
}

/// Files waiting to be prepared, and those being prepared.
static MEDIA_QUEUE: Mutex<MediaQueue> = Mutex::new(MediaQueue {
    waiting: VecDeque::new(),
    pending: BTreeSet::new(),
    cancellations: BTreeMap::new(),
    workers: 0,
});

struct MediaQueue {
    waiting: VecDeque<MediaJob>,
    /// Each project folder and file waiting or being prepared.
    pending: BTreeSet<(PathBuf, PathBuf)>,
    /// What stops each folder's work, while it has some.
    cancellations: BTreeMap<PathBuf, kerf_engine::CancellationToken>,
    workers: usize,
}

impl MediaQueue {
    /// `job` is no longer waiting or being prepared.
    fn forget(&mut self, job: &MediaJob) {
        self.pending
            .remove(&(job.directory.clone(), job.file.clone()));
        if !self.preparing(&job.directory) {
            self.cancellations.remove(&job.directory);
        }
    }

    fn preparing(&self, directory: &Path) -> bool {
        self.pending.iter().any(|(pending, _)| pending == directory)
    }
}

fn media_queue() -> MutexGuard<'static, MediaQueue> {
    // Every update leaves the queue consistent, and work runs outside the
    // lock, so a panic leaves nothing half-done in it.
    MEDIA_QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Work kerfd does for each project folder after the request that started
/// it has answered, so the daemon does not exit under it.
static BACKGROUND_WORK: Mutex<BTreeMap<PathBuf, BackgroundState>> = Mutex::new(BTreeMap::new());

#[derive(Default)]
struct BackgroundState {
    running: usize,
    finished: Option<Instant>,
}

fn background_state() -> MutexGuard<'static, BTreeMap<PathBuf, BackgroundState>> {
    // The map is consistent between statements, so a panic elsewhere
    // leaves nothing half-done in it.
    BACKGROUND_WORK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// One piece of background work for a project folder. It ends when
/// dropped, by a panic too.
pub(crate) struct BackgroundWork {
    directory: PathBuf,
}

impl BackgroundWork {
    pub(crate) fn start(directory: &Path) -> Self {
        background_state()
            .entry(directory.to_path_buf())
            .or_default()
            .running += 1;
        Self {
            directory: directory.to_path_buf(),
        }
    }
}

impl Drop for BackgroundWork {
    fn drop(&mut self) {
        if let Some(state) = background_state().get_mut(&self.directory) {
            state.running = state.running.saturating_sub(1);
            state.finished = Some(Instant::now());
        }
    }
}

/// Whether background work runs for `directory`, and when the last piece
/// finished.
pub(crate) fn background_work(directory: &Path) -> (bool, Option<Instant>) {
    background_state()
        .get(directory)
        .map_or((false, None), |state| (state.running > 0, state.finished))
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
            buses: Vec::new(),
        id: Id::new("s1").expect("valid id"),
        name: project.name.clone(),
        rate: Rate::FPS_30,
        drop_frame: false,
        width: 1920,
        height: 1080,
        sample_rate: 48_000,
        color: Default::default(),
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

/// The operation that adds the media file at `path` (described by `probe`
/// when it could be read), and its id, leaving the program as it is.
pub(crate) fn add_media(
    project: &Project,
    path: &str,
    probe: Option<&ProbeInfo>,
) -> Result<(Vec<Op>, Id), RuntimeError> {
    let sequence = project
        .sequences
        .first()
        .ok_or_else(|| RuntimeError::new("INVALID_IMPORT", "the project has no sequence"))?;
    let id = project.id_gen().next("m");
    let media = match probe.map(|probe| media_from_probe(id.clone(), path, probe, sequence.rate)) {
        Some(Ok(media)) => media,
        _ => assumed_media(id, path, probe, sequence)?,
    };
    let id = media.id.clone();
    Ok((vec![Op::AddMedia(media)], id))
}

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
    let mut ids = project.id_gen();
    let id = ids.next("m");
    let media = match probe.map(|probe| media_from_probe(id.clone(), path, probe, sequence.rate)) {
        Some(Ok(media)) => media,
        _ => assumed_media(id, path, probe, sequence)?,
    };
    let at = program_duration(project);
    let length = sequence
        .rate
        .round(media.duration)
        .max(sequence.rate.frame_duration());
    let content = Content::Media {
        media: media.id.clone(),
        source_in: Time::ZERO,
        speed: Ratio::ONE.into(),
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

/// The media `probe` found in the file at `path`, under `id`: its picture
/// rate (`rate` when it has no picture), size, sound, and length on whole
/// frames. Sound is recorded as stereo at 48 kHz, which is what playback
/// and renders make of it. A probe that found no length, or neither picture
/// nor sound, is refused rather than guessed.
pub(crate) fn media_from_probe(
    id: Id,
    path: &str,
    probe: &ProbeInfo,
    rate: Rate,
) -> Result<Media, RuntimeError> {
    let unreadable = |what: &str| RuntimeError::new("MEDIA_UNREADABLE", format!("{path} {what}"));
    let rate = probe
        .fps
        .and_then(|fps| Rate::from_fps(fps).ok())
        .unwrap_or(rate);
    let seconds = probe
        .duration
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or_else(|| unreadable("has no length that can be read"))?;
    let duration = Time::from_seconds(seconds)
        .filter(|time| time.in_range())
        .map(|time| rate.round(time).max(rate.frame_duration()))
        .ok_or_else(|| unreadable("is too long"))?;
    let video = probe
        .width
        .zip(probe.height)
        .filter(|(width, height)| *width > 0 && *height > 0)
        .map(|(width, height)| probe.video_info(width, height));
    let audio = (probe.has_audio == Some(true)).then_some(AudioInfo {
        channels: 2,
        sample_rate: 48_000,
    });
    if video.is_none() && audio.is_none() {
        return Err(unreadable("has neither picture nor sound"));
    }
    Ok(Media {
        id,
        path: path.to_owned(),
        rate,
        drop_frame: false,
        start: Time::ZERO,
        duration,
        video,
        audio,
        reel: None,
        recipe: None,
    })
}

/// What an import assumes of a file `probe` could not fully read: picture
/// at the sequence's size and rate unless it found sound, lasting one
/// second unless it found a length.
fn assumed_media(
    id: Id,
    path: &str,
    probe: Option<&ProbeInfo>,
    sequence: &Sequence,
) -> Result<Media, RuntimeError> {
    let mut assumed = probe.cloned().unwrap_or_default();
    if !assumed
        .duration
        .is_some_and(|seconds| seconds.is_finite() && seconds > 0.0)
    {
        assumed.duration = Some(1.0);
    }
    let sized = assumed
        .width
        .zip(assumed.height)
        .is_some_and(|(width, height)| width > 0 && height > 0);
    if !sized && assumed.has_audio != Some(true) {
        assumed.width = Some(sequence.width);
        assumed.height = Some(sequence.height);
        assumed.alpha = None;
    }
    media_from_probe(id, path, &assumed, sequence.rate)
}

/// Swap the file behind `media.id` for the one `media` describes. Every
/// clip keeps its timing; a file one no longer fits is refused with the
/// clips (and transitions) it would break.
pub(crate) fn replace_media(project: &Project, media: Media) -> Result<Edited, RuntimeError> {
    let (id, path) = (media.id.clone(), media.path.clone());
    let ops = [Op::ReplaceMedia(media)];
    let (next, _) = apply(project, catalog(), &ops).map_err(|rejected| {
        let reasons: Vec<String> = rejected
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        RuntimeError::new(
            "MEDIA_DOES_NOT_FIT",
            format!("{path} cannot replace media `{id}`: {}", reasons.join("; ")),
        )
    })?;
    Ok(Edited {
        script: print_ops(project, catalog(), &ops),
        project: next,
    })
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
    /// Property or generator parameter values: a static value, keyframes
    /// as `{"keys": [{"at": frames, "value": v, "interp": "linear"}]}` with
    /// `at` counted from the clip's first frame, or `null` to restore the
    /// default. `name` renames the clip; an empty one gives it back the
    /// name of what it shows.
    Set {
        clip: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        props: BTreeMap<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Speed {
        clip: String,
        speed: String,
    },
    /// Add an effect to the end of the clip's stack, or at `index`, with
    /// parameter values, static or keyframed; the rest stay at their
    /// defaults.
    FxAdd {
        clip: String,
        fx: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Turn the effect at `index` on or off, or set its parameters, static
    /// or keyframed; `null` restores a parameter's default.
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
    /// Place media or a generator at `at` on each of `tracks`, pushing
    /// what follows later by `frames`.
    Insert {
        /// The media to place, from `sourceIn` (frames of the media); or
        /// else `generator`, a generated picture such as a title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
        #[serde(default)]
        source_in: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorSpec>,
        frames: i64,
        at: i64,
        tracks: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
        /// `insert` or `overwrite` in place of the op's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
    /// Place media or a generator at `at` on each of `tracks` over what
    /// is there.
    Overwrite {
        /// The media to place, from `sourceIn` (frames of the media); or
        /// else `generator`, a generated picture such as a title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
        #[serde(default)]
        source_in: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorSpec>,
        frames: i64,
        at: i64,
        tracks: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
        /// `insert` or `overwrite` in place of the op's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
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
    /// Move, retime, rename, recolour, or annotate the marker `marker`:
    /// `at` is a frame offset from the sequence's first frame and `frames`
    /// its length, as `add-marker` counts them. An empty `name`, `note`, or
    /// `color` clears it; what is absent stays as it is.
    SetMarker {
        marker: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frames: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
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
    /// `set-track`: a track's switches, its role, and its sync lock. `role`
    /// is one of `video`, `titles`, `dialogue`, `music`, `effects` (any
    /// case) or other one-line text, and an empty one gives back the
    /// kind's own; `syncLocked` false keeps ripple edits on other tracks
    /// from moving the track.
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sync_locked: Option<bool>,
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
    /// A sequence's name, frame size, frame rate, and start timecode;
    /// what is absent stays as it is. `width` and `height` go together.
    /// `rate` (`{"num": 30, "den": 1}`, or text such as `"29.97"`) changes
    /// only while the sequence has no clips or markers, and the start
    /// keeps its timecode at the new rate. `start` is timecode text, with
    /// `;` before the frames for drop-frame.
    SetSequence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rate: Option<RateSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
}

/// Why `set-sequence` refuses a new frame rate for a sequence with content.
pub const RATE_NEEDS_EMPTY: &str = "Frame rate can change only while the sequence is empty";

/// A generated picture for `insert` and `overwrite`: a generator of the
/// catalog (`title`, `solid`, `adjustment`) and its parameter values,
/// static or keyframed as `set` takes them; the rest stay at their
/// defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratorSpec {
    pub fx: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Json>,
}

impl GeneratorSpec {
    /// The clip content it makes; keyframes count frames of `rate`.
    fn content(&self, rate: Rate) -> Result<Content, RuntimeError> {
        let def = catalog()
            .fx(&self.fx)
            .ok_or_else(|| invalid(format!("no generator `{}`", self.fx)))?;
        if def.kind != FxKind::Generator {
            return Err(invalid(format!(
                "`{}` is {} {}, not a generator",
                self.fx,
                if def.kind == FxKind::Effect {
                    "an"
                } else {
                    "a"
                },
                def.kind.name()
            )));
        }
        let params = fx_params(def, &self.params, Some(rate))?
            .into_iter()
            .filter_map(|(name, param)| param.map(|param| (name, param)))
            .collect();
        Ok(Content::Generator {
            fx: def.id.clone(),
            version: def.version,
            params,
        })
    }
}

/// A frame rate as the editor gives one: `{"num": 30000, "den": 1001}`,
/// text the language reads (`"29.97"`, `"24000/1001"`), or a number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RateSpec {
    Fraction { num: u32, den: u32 },
    Text(String),
    Fps(f64),
}

impl RateSpec {
    pub fn rate(&self) -> Result<Rate, RuntimeError> {
        match self {
            Self::Fraction { num, den } => Rate::new(*num, *den),
            Self::Text(text) => Rate::parse(text.trim()),
            Self::Fps(fps) => Rate::from_fps(*fps),
        }
        .map_err(|error| invalid(format!("frame rate: {error}")))
    }
}

/// A mask's outline, as the language writes it: at most one of `rect`
/// (left, top, right, bottom), `ellipse` (center x and y, then the radius
/// across and down), or `points` (corners in order, closed unless `closed`
/// is false, and `soft`, how soft the edge is at each, in pixels), in fractions of the clip's picture
/// with 0, 0 at its top left.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<Vec<f64>>,
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
        if (self.closed.is_some() || self.soft.is_some()) && self.points.is_none() {
            return Err(invalid("`closed` and `soft` go with `points`"));
        }
        Ok(match (self.rect, self.ellipse, &self.points) {
            (Some(rect), _, _) => Some(MaskShape::Rect(rect)),
            (_, Some(ellipse), _) => Some(MaskShape::Ellipse(ellipse)),
            (_, _, Some(points)) => Some(MaskShape::Polygon {
                points: points.clone(),
                closed: self.closed.unwrap_or(true),
                soft: self.soft.clone().unwrap_or_default(),
            }),
            _ => None,
        })
    }
}

/// `shape` for a mask being given new corners without a word of how soft they are, as dragging them
/// does: as soft as they were, if there are as many as there were.
fn kept_soft(
    project: &Project,
    clip: &str,
    name: &str,
    outline: &MaskOutline,
    shape: MaskShape,
) -> MaskShape {
    let was = project
        .sequences
        .iter()
        .flat_map(|sequence| &sequence.tracks)
        .flat_map(|track| &track.clips)
        .filter(|found| found.id.as_str() == clip)
        .flat_map(|found| &found.masks)
        .find(|mask| mask.name.as_str() == name);
    match (shape, was.map(|mask| &mask.shape), &outline.soft) {
        (
            MaskShape::Polygon { points, closed, .. },
            Some(MaskShape::Polygon {
                points: before,
                soft,
                ..
            }),
            None,
        ) if points.len() == before.len() => MaskShape::Polygon {
            points,
            closed,
            soft: soft.clone(),
        },
        (shape, _, _) => shape,
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
/// Keyframes count frames of `rate`, the rate of the clip's sequence.
fn mask_params(
    params: &BTreeMap<String, Json>,
    rate: Option<Rate>,
) -> Result<Vec<(String, Option<Param>)>, RuntimeError> {
    params
        .iter()
        .map(|(name, value)| {
            let def = catalog()
                .mask_prop(name)
                .ok_or_else(|| invalid(format!("a mask has no property `{name}`")))?;
            Ok((name.clone(), param_value(name, def, value, rate)?))
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
    let rate_of = |clip: &str| {
        project
            .locate(clip)
            .map(|at| project.sequences[at.sequence].rate)
    };
    let clip_rate = |clip: &str| rate_of(clip).ok_or_else(|| invalid(format!("no clip `{clip}`")));
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
        EditOp::Set { clip, props, name } => {
            let at = project
                .locate(clip)
                .ok_or_else(|| invalid(format!("no clip `{clip}`")))?;
            let target = project.clip_at(at);
            let rate = project.sequences[at.sequence].rate;
            let domain = project.sequences[at.sequence].tracks[at.track]
                .kind
                .domain();
            let mut changes = Vec::new();
            for (name, value) in props {
                let prop = domain.and_then(|domain| catalog().prop(domain, name));
                let def = prop
                    .or_else(|| match &target.content {
                        Content::Generator { fx, .. } => {
                            catalog().fx(fx.as_str()).and_then(|def| def.find(name))
                        }
                        _ => None,
                    })
                    .ok_or_else(|| invalid(format!("clip `{clip}` has no property `{name}`")))?;
                let param = param_value(name, def, value, Some(rate))?;
                if prop.is_some() && domain == Some(Domain::Audio) {
                    sound_keyframes(None, name, param.as_ref())?;
                }
                changes.push(ClipChange::Param(name.clone(), param));
            }
            if let Some(name) = name {
                let name = name.trim();
                changes.push(ClipChange::Name(
                    (!name.is_empty()).then(|| name.to_owned()),
                ));
            }
            Op::Set {
                clip: edit_id(clip)?,
                changes,
            }
        }
        EditOp::Speed { clip, speed } => Op::Speed {
            clip: edit_id(clip)?,
            speed: Ratio::parse(speed)
                .map_err(|error| invalid(error.to_string()))?
                .into(),
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
            let params = fx_params(def, params, rate_of(clip))?
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
                params: fx_params(def, params, rate_of(clip))?,
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
                    params: mask_params(params, rate_of(clip))?
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
            shape: outline
                .shape()?
                .map(|shape| kept_soft(project, clip, name, outline, shape)),
            mode: mask_mode(mode)?,
            inverted: *inverted,
            params: mask_params(params, rate_of(clip))?,
        },
        EditOp::MaskRemove { clip, name } => Op::MaskRemove {
            clip: edit_id(clip)?,
            name: edit_id(name)?,
        },
        EditOp::Insert {
            media,
            source_in,
            generator,
            frames,
            at,
            tracks,
            ids,
            sequence: id,
            mode,
        }
        | EditOp::Overwrite {
            media,
            source_in,
            generator,
            frames,
            at,
            tracks,
            ids,
            sequence: id,
            mode,
        } => {
            let sequence = sequence(id)?;
            if ids.len() > tracks.len() {
                return Err(invalid("give at most one id per track"));
            }
            let content = match (media, generator) {
                (Some(media), None) => {
                    let found = project
                        .media(media)
                        .ok_or_else(|| invalid(format!("no media `{media}`")))?;
                    Content::Media {
                        media: found.id.clone(),
                        source_in: found.rate.time_of(*source_in),
                        speed: Ratio::ONE.into(),
                    }
                }
                (None, Some(generator)) => {
                    if *source_in != 0 {
                        return Err(invalid("a generator has no source in point"));
                    }
                    generator.content(sequence.rate)?
                }
                (Some(_), Some(_)) => {
                    return Err(invalid("place media or a generator, not both"));
                }
                (None, None) => return Err(invalid("give the media or the generator to place")),
            };
            let overwrite = match mode.as_deref().map(str::trim) {
                None => matches!(op, EditOp::Overwrite { .. }),
                Some("insert") => false,
                Some("overwrite") => true,
                Some(other) => {
                    return Err(invalid(format!(
                        "`{other}` is not an edit mode; use insert or overwrite"
                    )))
                }
            };
            let mut targets = Vec::new();
            for (index, track) in tracks.iter().enumerate() {
                let mut clip = NewClip::new(content.clone());
                clip.id = ids.get(index).map(|id| edit_id(id)).transpose()?;
                targets.push((edit_id(track)?, clip));
            }
            let (at, duration) = (sequence.rate.time_of(*at), sequence.rate.time_of(*frames));
            if !overwrite {
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
        EditOp::SetMarker {
            marker,
            at,
            frames,
            name,
            note,
            color,
        } => {
            let sequence = project
                .sequences
                .iter()
                .find(|sequence| sequence.marker(marker).is_some())
                .ok_or_else(|| invalid(format!("no marker `{marker}`")))?;
            // Blank text clears; absent text leaves what is there.
            let text = |text: &Option<String>| {
                text.as_ref()
                    .map(|text| (!text.trim().is_empty()).then(|| text.clone()))
            };
            let color = match color.as_deref().map(str::trim) {
                None => None,
                Some("") => Some(None),
                Some(color) => {
                    Some(Some(MarkerColor::parse(color).ok_or_else(|| {
                        invalid(format!("`{color}` is not a marker color"))
                    })?))
                }
            };
            Op::SetMarker {
                id: edit_id(marker)?,
                change: MarkerChange {
                    at: at.map(|at| sequence.rate.time_of(at)),
                    duration: frames.map(|frames| sequence.rate.time_of(frames)),
                    name: text(name),
                    color,
                    note: text(note),
                },
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
            role,
            sync_locked,
        } => {
            let (sequence, track) = (sequence(id)?.id.clone(), edit_id(track)?);
            let switches = TrackChange {
                bus: None,
                name: None,
                enabled: *enabled,
                locked: *locked,
                muted: *muted,
            };
            let roles = TrackRoleChange {
                role: role
                    .as_deref()
                    .map(|role| track_role(role).map_err(invalid))
                    .transpose()?,
                sync_locked: *sync_locked,
            };
            let mut ops = Vec::new();
            // An op that names nothing to change still names the track,
            // which must exist, as `set-track` always has.
            if switches != TrackChange::default() || roles == TrackRoleChange::default() {
                ops.push(Op::SetTrack {
                    sequence: sequence.clone(),
                    track: track.clone(),
                    change: switches,
                });
            }
            if roles != TrackRoleChange::default() {
                ops.push(Op::SetTrackRole {
                    sequence,
                    track,
                    change: roles,
                });
            }
            return Ok(ops);
        }
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
            name,
            rate,
            start,
            sequence: id,
        } => {
            let target = sequence(id)?;
            let size = match (width, height) {
                (Some(width), Some(height)) => Some((*width, *height)),
                (None, None) => None,
                _ => return Err(invalid("give the width and the height together")),
            };
            let rate = rate
                .as_ref()
                .map(RateSpec::rate)
                .transpose()?
                .filter(|rate| *rate != target.rate);
            let empty = target.tracks.iter().all(|track| track.clips.is_empty())
                && target.markers.is_empty();
            if rate.is_some() && !empty {
                return Err(invalid(RATE_NEEDS_EMPTY));
            }
            let name = name
                .as_deref()
                .map(|name| match name.trim() {
                    "" => Err(invalid("a sequence needs a name")),
                    name => Ok(name.to_owned()),
                })
                .transpose()?;
            let start = start
                .as_deref()
                .map(|text| {
                    kerf_model::time::parse_timecode(text.trim(), rate.unwrap_or(target.rate))
                        .map_err(|error| invalid(format!("start timecode: {error}")))
                })
                .transpose()?;
            Op::SetSequence {
                sequence: target.id.clone(),
                change: SequenceChange {
                    name,
                    rate,
                    size,
                    sample_rate: None,
                    color: None,
                    start,
                },
            }
        }
    }])
}

/// An effect's parameters from JSON: a static value or keyframes each (see
/// `json_param`), or `None` (for `null`) to restore the default.
/// Keyframes count frames of `rate`, the rate of the clip's sequence.
fn fx_params(
    def: &FxDef,
    params: &BTreeMap<String, Json>,
    rate: Option<Rate>,
) -> Result<Vec<(String, Option<Param>)>, RuntimeError> {
    params
        .iter()
        .map(|(name, value)| {
            let param = def
                .find(name)
                .ok_or_else(|| invalid(format!("`{}` has no parameter `{name}`", def.id)))?;
            let value = param_value(name, param, value, rate)?;
            if def.domain == Domain::Audio {
                sound_keyframes(Some(def.id.as_str()), name, value.as_ref())?;
            }
            Ok((name.clone(), value))
        })
        .collect()
}

/// A property or parameter value a typed op gives: `None` for `null`, which
/// restores the default, else a static value or keyframes. `rate` is the
/// rate of the clip's sequence, or `None` when there is no such clip.
fn param_value(
    name: &str,
    def: &ParamDef,
    value: &Json,
    rate: Option<Rate>,
) -> Result<Option<Param>, RuntimeError> {
    if value.is_null() {
        return Ok(None);
    }
    let keyed = value.get("keys").is_some();
    if keyed && !def.animatable {
        return Err(invalid(format!("`{name}` cannot be animated")));
    }
    let param = match rate {
        Some(rate) => json_param(&def.ty, value, rate),
        None if keyed => return Err(invalid("keyframes need a clip to count frames on")),
        None => json_value(&def.ty, value).map(Param::Static),
    };
    param.map(Some).ok_or_else(|| {
        invalid(if keyed {
            format!(
                "`{name}` keyframes are {{\"keys\": [{{\"at\": <frame>, \"value\": <{}>, \
                 \"interp\": {}}}]}}, at least one, at whole frames",
                def.ty.name(),
                Interp::ALL.map(Interp::name).join("|"),
            )
        } else {
            format!("`{name}` expects a {}", def.ty.name())
        })
    })
}

/// Refuse keyframes on sound that playback and renders cannot follow. The
/// program (kerf-core) lowers keyframed clip gain to a volume expression;
/// it refuses keyframed pan and keyframed sound-effect parameters, so an
/// edit that makes them is refused here, with the reason, before it lands.
fn sound_keyframes(
    fx: Option<&str>,
    name: &str,
    param: Option<&Param>,
) -> Result<(), RuntimeError> {
    if !matches!(param, Some(Param::Animated(_))) {
        return Ok(());
    }
    match fx {
        None if name == "gain" => Ok(()),
        None => Err(invalid(format!(
            "`{name}` cannot be animated on sound: playback and renders follow keyframes on \
             gain only"
        ))),
        Some(fx) => Err(invalid(format!(
            "`{fx}` {name} cannot be animated: playback and renders do not follow keyframes on \
             sound effects"
        ))),
    }
}

/// The largest keyframe offset a typed op takes, in frames: well past any
/// clip, and far inside what a time holds.
const MAX_KEY_FRAMES: i64 = 1 << 30;

/// A parameter from JSON of type `ty`: a static value, or keyframes as
/// `{"keys": [{"at": frames, "value": v, "interp": "linear"}]}`. `at` counts
/// whole frames of `rate` (the clip's sequence) from the clip's first frame,
/// as the model keeps keyframes clip-relative; `interp` is `linear` (the
/// default), `hold`, `ease`, `ease-in`, `ease-out` or a curve's four numbers
/// (`0.25,0.1,0.75,0.9`), and says how the value moves toward the next key. Keys are put in time order; two at one frame
/// are refused when the edit is checked. `None` for anything else.
pub(crate) fn json_param(ty: &ParamType, value: &Json, rate: Rate) -> Option<Param> {
    let Some(keys) = value.as_object().and_then(|object| object.get("keys")) else {
        return json_value(ty, value).map(Param::Static);
    };
    if value.as_object().is_some_and(|object| object.len() != 1) {
        return None;
    }
    let mut keys = keys
        .as_array()?
        .iter()
        .map(|key| {
            let key = key.as_object()?;
            let at = key.get("at").and_then(whole_frames)?;
            let value = json_value(ty, key.get("value")?)?;
            let interp = match key.get("interp") {
                None | Some(Json::Null) => Interp::Linear,
                Some(Json::String(name)) => Interp::parse(name)?,
                Some(_) => return None,
            };
            Some(Keyframe {
                at: rate.time_of(at),
                value,
                interp,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    if keys.is_empty() {
        return None;
    }
    keys.sort_by_key(|key| key.at);
    Some(Param::Animated(keys))
}

/// A whole number of frames, within [`MAX_KEY_FRAMES`] either way.
fn whole_frames(value: &Json) -> Option<i64> {
    let frames = match value.as_i64() {
        Some(frames) => frames,
        None => {
            let frames = value.as_f64()?;
            if frames.fract() != 0.0 || frames.abs() > MAX_KEY_FRAMES as f64 {
                return None;
            }
            frames as i64
        }
    };
    (frames.abs() <= MAX_KEY_FRAMES).then_some(frames)
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
            | ParamType::Features
            // A time given as text names a marker to land on.
            | ParamType::Time,
            Json::String(s),
        ) => Value::Text(s.clone()),
        (ParamType::Enum(_), Json::String(s)) => Value::Enum(s.clone()),
        (ParamType::Color, Json::String(s)) => Value::Color(kerf_model::Rgba::parse(s)?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_take_the_curves_the_graph_editor_writes() {
        use kerf_model::{Interp, Param, ParamType, Rate};
        let keys = serde_json::json!({"keys": [
            {"at": 0, "value": 0.0, "interp": "0.25,0.1,0.75,0.9"},
            {"at": 24, "value": 1.0, "interp": "hold"},
        ]});
        let Some(Param::Animated(found)) = super::json_param(&ParamType::Number, &keys, Rate::FPS_24)
        else {
            panic!("took no keys");
        };
        assert_eq!(found[0].interp, Interp::parse("0.25,0.1,0.75,0.9").expect("a curve"));
        assert_eq!(found[1].interp, Interp::Hold);
        let wrong = serde_json::json!({"keys": [{"at": 0, "value": 0.0, "interp": "9,0,0,1"}]});
        assert!(super::json_param(&ParamType::Number, &wrong, Rate::FPS_24).is_none());
    }

    /// The Inspector sets a title's features as the line of text it shows them as.
    #[test]
    fn a_titles_features_come_as_text() {
        use kerf_model::{ParamType, Value};
        let features = serde_json::json!("tnum -liga");
        assert_eq!(super::json_value(&ParamType::Features, &features), Some(Value::Text("tnum -liga".into())));
        assert_eq!(super::json_value(&ParamType::Features, &serde_json::json!(1)), None);
    }

    use super::*;
    use serde_json::json;
    use std::time::Duration;

    /// A 24 fps sequence: c1 on V1 (0–4 s), V2 empty, and an empty
    /// second sequence `empty` starting at 01:00:00:00.
    fn project() -> Project {
        parse(
            "kerf 1\nproject name=\"Ops\"\n\n\
             media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080\n\n\
             sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00\n\
             \x20 track V2 video\n\
             \x20 track A1 audio\n\n\
             sequence empty name=\"Empty\" rate=24 size=1080x1920 audio=48000 start=01:00:00:00\n\
             \x20 track V1 video\n",
        )
        .unwrap()
    }

    /// The editor's ops as the bridge reads them, lowered on `project`.
    fn lowered(project: &Project, ops: Json) -> Result<Vec<Op>, RuntimeError> {
        let ops: Vec<EditOp> = serde_json::from_value(ops).unwrap();
        lower_ops(project, &ops)
    }

    fn refusal(project: &Project, ops: Json) -> String {
        let error = lowered(project, ops).unwrap_err();
        assert_eq!(error.code, "INVALID_EDIT", "{}", error.message);
        error.message
    }

    #[test]
    fn a_generator_insert_reads_as_the_editor_writes_it_and_places_a_title() {
        let project = project();
        let json = json!({"op": "insert", "generator": {"fx": "title", "params": {"text": "Day One"}},
                          "at": 24, "frames": 48, "tracks": ["V2"]});
        let op: EditOp = serde_json::from_value(json.clone()).unwrap();
        let EditOp::Insert {
            media: None,
            generator: Some(generator),
            source_in: 0,
            mode: None,
            ..
        } = &op
        else {
            panic!("a generator insert, got {op:?}");
        };
        assert_eq!(generator.fx, "title");
        // It writes back as it was read.
        assert_eq!(serde_json::to_value(&op).unwrap(), {
            let mut written = json.clone();
            written["sourceIn"] = json!(0);
            written
        });
        let ops = lowered(&project, json!([json])).unwrap();
        let [Op::Insert {
            at,
            duration,
            targets,
            ..
        }] = ops.as_slice()
        else {
            panic!("an insert, got {ops:?}");
        };
        assert_eq!(
            (*at, *duration),
            (Rate::FPS_24.time_of(24), Rate::FPS_24.time_of(48))
        );
        let [(track, clip)] = targets.as_slice() else {
            panic!("one target");
        };
        assert_eq!(track.as_str(), "V2");
        let Content::Generator {
            fx,
            version,
            params,
        } = &clip.content
        else {
            panic!("generator content, got {:?}", clip.content);
        };
        assert_eq!((fx.as_str(), *version), ("title", 1));
        assert_eq!(params["text"], Param::Static(Value::Text("Day One".into())));
        // A solid with no parameters keeps its defaults; overwrite and
        // `mode` place it over what is there.
        for json in [
            json!({"op": "overwrite", "generator": {"fx": "solid"}, "at": 0, "frames": 24, "tracks": ["V1"]}),
            json!({"op": "insert", "mode": "overwrite", "generator": {"fx": "solid"}, "at": 0,
                   "frames": 24, "tracks": ["V1"]}),
        ] {
            let ops = lowered(&project, json!([json])).unwrap();
            let [Op::Overwrite { targets, .. }] = ops.as_slice() else {
                panic!("an overwrite, got {ops:?}");
            };
            assert!(
                matches!(&targets[0].1.content, Content::Generator { fx, params, .. }
                if fx.as_str() == "solid" && params.is_empty())
            );
        }
        // Colours are text, as the Inspector writes them.
        let ops = lowered(
            &project,
            json!([{"op": "insert", "generator": {"fx": "solid", "params": {"color": "#ff0000"}},
                    "at": 0, "frames": 25, "tracks": ["V2"]}]),
        )
        .unwrap();
        let [Op::Insert { targets, .. }] = ops.as_slice() else {
            panic!("an insert, got {ops:?}");
        };
        assert!(
            matches!(&targets[0].1.content, Content::Generator { params, .. }
            if params.contains_key("color"))
        );
        let ops = lowered(
            &project,
            json!([{"op": "overwrite", "mode": "insert", "generator": {"fx": "title"}, "at": 0,
                    "frames": 24, "tracks": ["V2"]}]),
        )
        .unwrap();
        assert!(matches!(ops.as_slice(), [Op::Insert { .. }]));
    }

    #[test]
    fn generator_inserts_that_cannot_be_made_say_why() {
        let project = project();
        let place = |extra: Json| {
            let mut op = json!({"op": "insert", "at": 0, "frames": 24, "tracks": ["V2"]});
            op.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            json!([op])
        };
        assert_eq!(
            refusal(&project, place(json!({"generator": {"fx": "sparkle"}}))),
            "no generator `sparkle`"
        );
        assert_eq!(
            refusal(
                &project,
                place(json!({"generator": {"fx": "blur.gaussian"}}))
            ),
            "`blur.gaussian` is an effect, not a generator"
        );
        assert!(refusal(
            &project,
            place(json!({"generator": {"fx": "title", "params": {"glow": 1}}}))
        )
        .contains("glow"));
        assert_eq!(
            refusal(
                &project,
                place(json!({"media": "m1", "generator": {"fx": "title"}}))
            ),
            "place media or a generator, not both"
        );
        assert_eq!(
            refusal(&project, place(json!({}))),
            "give the media or the generator to place"
        );
        assert_eq!(
            refusal(
                &project,
                place(json!({"sourceIn": 12, "generator": {"fx": "title"}}))
            ),
            "a generator has no source in point"
        );
        assert_eq!(
            refusal(
                &project,
                place(json!({"mode": "replace", "generator": {"fx": "title"}}))
            ),
            "`replace` is not an edit mode; use insert or overwrite"
        );
        // A title on a sound track is refused by the kernel.
        let ops: Vec<EditOp> = serde_json::from_value(json!([{"op": "insert",
            "generator": {"fx": "title"}, "at": 0, "frames": 24, "tracks": ["A1"]}]))
        .unwrap();
        assert!(lower_ops(&project, &ops).is_err());
    }

    #[test]
    fn set_sequence_changes_the_rate_only_while_the_sequence_is_empty() {
        let project = project();
        // With a clip in it, the rate stays; the same rate is no change.
        assert_eq!(
            refusal(
                &project,
                json!([{"op": "set-sequence", "rate": {"num": 30, "den": 1}}])
            ),
            RATE_NEEDS_EMPTY
        );
        assert_eq!(
            RATE_NEEDS_EMPTY,
            "Frame rate can change only while the sequence is empty"
        );
        let same = lowered(
            &project,
            json!([{"op": "set-sequence", "rate": {"num": 24, "den": 1}, "name": "Cut"}]),
        )
        .unwrap();
        assert!(matches!(&same[..], [Op::SetSequence { change, .. }]
            if change.rate.is_none() && change.name.as_deref() == Some("Cut")));
        // Clips placed earlier in the same edit count.
        assert_eq!(
            refusal(
                &project,
                json!([
                    {"op": "insert", "generator": {"fx": "title"}, "at": 0, "frames": 24,
                     "tracks": ["V1"], "sequence": "empty"},
                    {"op": "set-sequence", "rate": "30", "sequence": "empty"}
                ])
            ),
            RATE_NEEDS_EMPTY
        );
        // Empty, it takes any spelling of a rate, and the start keeps its
        // timecode.
        for (rate, expected) in [
            (json!({"num": 30000, "den": 1001}), Rate::FPS_29_97),
            (json!("25"), Rate::FPS_25),
            (json!(23.976), Rate::FPS_23_976),
        ] {
            let ops = lowered(
                &project,
                json!([{"op": "set-sequence", "sequence": "empty", "rate": rate}]),
            )
            .unwrap();
            assert!(matches!(&ops[..], [Op::SetSequence { change, .. }]
                if change.rate == Some(expected) && change.start.is_none()));
            let (next, _) = apply(&project, catalog(), &ops).unwrap();
            let sequence = next.sequence("empty").unwrap();
            assert_eq!(sequence.rate, expected);
            assert_eq!(
                kerf_model::time::format_timecode(sequence.start, expected, false),
                "01:00:00:00"
            );
        }
        assert!(refusal(
            &project,
            json!([{"op": "set-sequence", "sequence": "empty", "rate": {"num": 0, "den": 1}}])
        )
        .starts_with("frame rate:"));
    }

    #[test]
    fn set_sequence_keeps_name_size_and_start_and_checks_them() {
        let project = project();
        let ops = lowered(
            &project,
            json!([{"op": "set-sequence", "sequence": "empty", "name": "  Vertical  ",
                    "width": 1080, "height": 1350, "rate": {"num": 30, "den": 1},
                    "start": "00:59:58:00"}]),
        )
        .unwrap();
        let (next, _) = apply(&project, catalog(), &ops).unwrap();
        let sequence = next.sequence("empty").unwrap();
        assert_eq!(sequence.name, "Vertical");
        assert_eq!((sequence.width, sequence.height), (1080, 1350));
        assert_eq!(sequence.rate, Rate::FPS_30);
        assert_eq!(sequence.start, Rate::FPS_30.time_of((3600 - 2) * 30));
        // The language writes what the op did.
        let script = print_ops(&project, catalog(), &ops);
        assert!(script.contains("name=\"Vertical\""), "{script}");
        assert!(script.contains("rate=30"), "{script}");
        assert!(script.contains("size=1080x1350"), "{script}");
        assert!(script.contains("start=00:59:58:00"), "{script}");
        assert_eq!(
            refusal(&project, json!([{"op": "set-sequence", "width": 1080}])),
            "give the width and the height together"
        );
        assert_eq!(
            refusal(&project, json!([{"op": "set-sequence", "name": " "}])),
            "a sequence needs a name"
        );
        assert!(
            refusal(&project, json!([{"op": "set-sequence", "start": "1:00"}]))
                .starts_with("start timecode:")
        );
        // A drop-frame start needs a rate that has drop-frame.
        assert!(refusal(
            &project,
            json!([{"op": "set-sequence", "start": "00:00:00;00"}])
        )
        .starts_with("start timecode:"));
    }

    fn running(directory: &Path) -> usize {
        background_state()
            .get(directory)
            .map_or(0, |state| state.running)
    }

    /// How much work `directory` still counts once its workers have let go
    /// of it. A worker forgets its file before its work stops counting, so
    /// an idle kerfd never sees a moment with neither.
    fn settled(directory: &Path) -> usize {
        let deadline = Instant::now() + Duration::from_secs(10);
        while running(directory) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        running(directory)
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kerf-media-{name}-{}", crate::unique_id()))
    }

    fn missing(directory: &Path, name: &str) -> String {
        directory.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn media_is_prepared_once_by_a_bounded_set_of_workers() {
        let directory = scratch("queue");
        prepare_media(missing(&directory, "twice.mov"), directory.clone());
        prepare_media(missing(&directory, "twice.mov"), directory.clone());
        // The file counts once, whether or not a worker has taken it yet.
        assert!(running(&directory) <= 1);
        for index in 0..8 {
            prepare_media(
                missing(&directory, &format!("{index}.mov")),
                directory.clone(),
            );
            assert!(media_queue().workers <= media_workers());
        }
        stop_media(&directory, Duration::from_secs(10));
        assert!(!media_queue().preparing(&directory));
        assert_eq!(settled(&directory), 0);
    }

}
