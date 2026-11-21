//! What Kerf measures of the programme and its media, for the Finish and
//! Deliver trays, the bin and the Source rows:
//!
//! - `loudness {sequence?}`: the programme measured to EBU R128
//!   (`kerf_engine::loudness::measure_program`) on a thread of its own,
//!   `running` with its progress, then `ready` or `failed`. A measurement is
//!   kept for the document revision it was made of, so asking again before
//!   the next edit answers at once; a request while one runs is taken as
//!   soon as it ends.
//! - `media_info {media?}`: what each file says of itself
//!   (`kerf_engine::media_info`: codec, pixel format, colour, camera and
//!   lens tags), keyed by media id. It is also sent on its own for media new
//!   to the editor (after an import, a replace, a merge). It is a derived
//!   cache only and never enters the document.
//! - `media_index {media}`: where a file's picture cuts and its sound
//!   pauses, and its loudest moments (`kerf_engine::media_index`, from the
//!   project's derivative cache when kerfd's import already made it).
//! - `scope_signal {time, sequence?}`: an HDR sequence's frame at `time` as the signal its master gets,
//!   small, for the Finish tray's scopes to read through `Bridge::signal`, on a thread of its own:
//!   only the newest request waits for one that runs. `scope_signal {status}` says when it starts or
//!   stops being a sequence with one. An SDR sequence draws nothing here.

use crate::{emit, Shared};
use kerf_engine::derivatives::DerivativeCache;
use kerf_engine::{CancellationToken, EngineError, MediaIndex};
use kerf_model::Project;
use kerf_runtime::RuntimeClient;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

/// The analysis state the bridge's workers share.
#[derive(Default)]
pub(crate) struct Analysis {
    loudness: Mutex<LoudnessState>,
    /// Media already probed on their own, by id, with the path probed.
    probed: Mutex<HashMap<String, String>>,
    /// Media being indexed now; another request for one waits for it.
    indexing: Mutex<HashSet<String>>,
    /// Cancelled when the bridge closes, which stops every FFmpeg here.
    cancel: CancellationToken,
    /// What the scopes' frames are drawn with.
    signal: Mutex<SignalState>,
    /// The newest signal frame, until the editor takes it.
    pub(crate) signal_frame: Mutex<Option<SignalFrame>>,
}

/// A frame of an HDR sequence's signal: 16-bit little-endian RGBA in BT.2020.
pub(crate) struct SignalFrame {
    /// Shared, so that the editor takes it without a copy.
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub time: f64,
    /// 1 for PQ, 2 for HLG.
    pub curve: u32,
}

#[derive(Default)]
struct SignalState {
    busy: bool,
    /// A request that came while one ran: its time and sequence.
    next: Option<(f64, Option<String>)>,
    /// Whether the last request found a signal to read.
    was_hdr: bool,
    /// The project and sequence the compositor is for, which opening footage makes worth keeping.
    drawing: Option<(Project, Option<String>, kerf_compose::Compositor)>,
}

#[derive(Default)]
struct LoudnessState {
    busy: bool,
    /// A request that came while one ran: its sequence.
    next: Option<Option<String>>,
    /// The last measurement: the revision and sequence it is of, and the
    /// `ready` event.
    kept: Option<(u64, Option<String>, Value)>,
}

impl Analysis {
    /// Stop every measurement running; the bridge closed.
    pub(crate) fn close(&self) {
        self.cancel.cancel();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn with_sequence(mut event: Value, sequence: Option<&str>) -> Value {
    if let Some(sequence) = sequence {
        event["sequence"] = json!(sequence);
    }
    event
}

// ---------------------------------------------------------------- scopes

/// How wide a frame is drawn for the scopes to read: they take a 160 by 90 grab, at most twice over.
const SIGNAL_WIDTH: f64 = 480.0;

/// `scope_signal {time, sequence?}`.
pub(crate) fn scope_signal(
    shared: Arc<Shared>,
    tx: SyncSender<Value>,
    time: f64,
    sequence: Option<String>,
) {
    {
        let mut state = lock(&shared.analysis.signal);
        if state.busy {
            state.next = Some((time, sequence));
            return;
        }
        state.busy = true;
    }
    std::thread::spawn(move || {
        let (mut time, mut sequence) = (time, sequence);
        loop {
            draw_signal(&shared, &tx, time, sequence.as_deref());
            let mut state = lock(&shared.analysis.signal);
            match state.next.take() {
                Some(next) if !shared.closed.load(std::sync::atomic::Ordering::SeqCst) => {
                    (time, sequence) = next;
                }
                _ => {
                    state.busy = false;
                    return;
                }
            }
        }
    });
}

/// The curve of an HDR sequence's signal: 1 for PQ, 2 for HLG; none for SDR.
fn signal_curve(project: &Project, sequence: Option<&str>) -> Option<u32> {
    let sequence = match sequence {
        Some(id) => project.sequence(id),
        None => project.sequences.first(),
    }?;
    match sequence.color.curve() {
        kerf_color::Curve::Pq => Some(1),
        kerf_color::Curve::Hlg => Some(2),
        _ => None,
    }
}

fn draw_signal(shared: &Shared, tx: &SyncSender<Value>, time: f64, sequence: Option<&str>) {
    let Some(project) = lock(&shared.model).clone() else {
        return;
    };
    let curve = signal_curve(&project, sequence);
    let hdr = curve.is_some();
    let drawing = {
        let mut state = lock(&shared.analysis.signal);
        if state.was_hdr != hdr {
            state.was_hdr = hdr;
            let status = if hdr { "hdr" } else { "sdr" };
            emit(tx, json!({"type": "scope_signal", "status": status}));
        }
        state.drawing.take()
    };
    let Some(curve) = curve else {
        return;
    };
    let same = |(kept, kept_sequence, _): &(Project, Option<String>, kerf_compose::Compositor)| {
        *kept == project && kept_sequence.as_deref() == sequence
    };
    let drawing = drawing.filter(same).or_else(|| {
        let compositor = kerf_compose::Compositor::new(
            &project,
            kerf_model::Catalog::builtin(),
            &kerf_compose::ComposeOptions {
                sequence: sequence.map(str::to_owned),
                captions: true,
                ..Default::default()
            },
        )
        .ok()?;
        Some((project.clone(), sequence.map(str::to_owned), compositor))
    });
    let Some(drawing) = drawing else {
        return;
    };
    let width = match sequence {
        Some(id) => project.sequence(id),
        None => project.sequences.first(),
    }
    .map_or(SIGNAL_WIDTH, |found| f64::from(found.width.max(1)));
    let at = kerf_model::Time::from_seconds(time.max(0.0)).unwrap_or(kerf_model::Time::ZERO);
    let frame = drawing.2.frame16(at, (SIGNAL_WIDTH / width).min(1.0));
    *lock(&shared.analysis.signal_frame) = Some(SignalFrame {
        pixels: frame.rgba.into(),
        width: frame.width,
        height: frame.height,
        time,
        curve,
    });
    lock(&shared.analysis.signal).drawing = Some(drawing);
}

// ---------------------------------------------------------------- loudness

/// `loudness {sequence?}`.
pub(crate) fn loudness(shared: Arc<Shared>, tx: SyncSender<Value>, sequence: Option<String>) {
    {
        let mut state = lock(&shared.analysis.loudness);
        if state.busy {
            state.next = Some(sequence);
            return;
        }
        state.busy = true;
    }
    std::thread::spawn(move || {
        let mut sequence = sequence;
        loop {
            let event = measure(&shared, &tx, sequence.as_deref());
            emit(&tx, event);
            let mut state = lock(&shared.analysis.loudness);
            match state.next.take() {
                Some(next) if !shared.closed.load(std::sync::atomic::Ordering::SeqCst) => {
                    sequence = next;
                }
                _ => {
                    state.busy = false;
                    return;
                }
            }
        }
    });
}

/// Measure the project kerfd has now, or answer with the measurement kept
/// for its revision.
fn measure(shared: &Shared, tx: &SyncSender<Value>, sequence: Option<&str>) -> Value {
    let failed = |message: &str, revision: Option<u64>| {
        let mut event = json!({"type": "loudness", "status": "failed", "message": message});
        if let Some(revision) = revision {
            event["revision"] = json!(revision);
        }
        with_sequence(event, sequence)
    };
    let (revision, document) =
        match RuntimeClient::new(&shared.directory).and_then(|client| client.get_document(false)) {
            Ok(found) => found,
            Err(error) => return failed(&error.message, None),
        };
    if let Some((kept, on, event)) = &lock(&shared.analysis.loudness).kept {
        if *kept == revision && on.as_deref() == sequence {
            return event.clone();
        }
    }
    let mut project = match kerf_runtime::parse_document(&document) {
        Ok(project) => project,
        Err(error) => return failed(&error.message, Some(revision)),
    };
    if let Some(id) = sequence {
        if !project.sequences.iter().any(|candidate| candidate.id == id) {
            return failed(&format!("no sequence `{id}`"), Some(revision));
        }
    }
    kerf_runtime::resolve_media_paths(&mut project, &shared.directory);
    let running = |progress: f64| {
        with_sequence(
            json!({"type": "loudness", "status": "running", "progress": progress,
                "revision": revision}),
            sequence,
        )
    };
    emit(tx, running(0.0));
    let mut sent = 0;
    let measured = kerf_engine::loudness::measure_program_with_progress(
        &project,
        sequence,
        &shared.directory,
        &shared.analysis.cancel,
        |done| {
            let percent = (done.clamp(0.0, 1.0) * 100.0).floor() as u32;
            if percent > sent && percent < 100 {
                sent = percent;
                // Replaceable; never hold the measurement up.
                let _ = tx.try_send(running(f64::from(percent) / 100.0));
            }
        },
    );
    match measured {
        Ok(loudness) => {
            let event = ready(&loudness, revision, sequence);
            lock(&shared.analysis.loudness).kept =
                Some((revision, sequence.map(str::to_owned), event.clone()));
            event
        }
        Err(EngineError::Cancelled) => failed("The measurement was cancelled", Some(revision)),
        Err(error) => failed(&error.to_string(), Some(revision)),
    }
}

/// The `ready` event of a measurement: `Loudness::to_json` (integrated,
/// lra, truePeak, truePeakMax, shortTermMax, momentaryMax, series,
/// duration, target, limit; silence as null), plus where the loudest
/// windows end and the revision measured.
pub(crate) fn ready(
    loudness: &kerf_engine::Loudness,
    revision: u64,
    sequence: Option<&str>,
) -> Value {
    let mut event = loudness.to_json();
    event["type"] = json!("loudness");
    event["status"] = json!("ready");
    event["revision"] = json!(revision);
    event["shortTermMaxAt"] = json!(loudness.short_term_max_at());
    event["momentaryMaxAt"] = json!(loudness.momentary_max_at());
    with_sequence(event, sequence)
}

// -------------------------------------------------------------- media info

/// The project as edited, its media paths resolved; `None` before kerfd
/// sent one.
fn model(shared: &Shared) -> Option<Project> {
    lock(&shared.model).clone()
}

/// What `kerf_engine::media_info` says of `path`, as the event carries it.
fn probe(path: &str) -> Result<Value, String> {
    kerf_engine::media_info(path)
        .map_err(|error| error.to_string())
        .and_then(|info| serde_json::to_value(info).map_err(|error| error.to_string()))
}

/// `media_info {media?}`: every media of the project, or the one asked
/// for. Files that cannot be read are named in `errors`; the one media asked
/// for that cannot be read is an `error {command: media_info, media}`.
pub(crate) fn media_info(shared: Arc<Shared>, tx: SyncSender<Value>, media: Option<String>) {
    std::thread::spawn(move || {
        let refuse = |message: &str| {
            let mut event = json!({"type": "error", "message": message, "command": "media_info"});
            if let Some(media) = &media {
                event["media"] = json!(media);
            }
            emit(&tx, event);
        };
        let Some(project) = model(&shared) else {
            refuse("The project is not open yet");
            return;
        };
        let chosen: Vec<_> = project
            .media
            .iter()
            .filter(|found| media.as_deref().is_none_or(|id| found.id == id))
            .collect();
        if chosen.is_empty() {
            if let Some(id) = &media {
                refuse(&format!("no media `{id}`"));
                return;
            }
        }
        let mut infos = Map::new();
        let mut errors = Map::new();
        for found in chosen {
            match probe(&found.path) {
                Ok(info) => {
                    infos.insert(found.id.to_string(), info);
                }
                Err(message) => {
                    errors.insert(found.id.to_string(), json!(message));
                }
            }
        }
        if media.is_some() && infos.is_empty() {
            let message = errors.values().next().and_then(Value::as_str).unwrap_or("");
            refuse(message);
            return;
        }
        let mut event = json!({"type": "media_info", "media": infos});
        if !errors.is_empty() {
            event["errors"] = Value::Object(errors);
        }
        emit(&tx, event);
    });
}

/// Probe the media of `project` new to the editor (or whose file changed
/// name), and send what they say as `media_info`. Files that cannot be
/// read now (offline, not downloaded) are tried again with the next
/// change.
pub(crate) fn media_changed(shared: &Arc<Shared>, tx: &SyncSender<Value>, project: &Project) {
    let fresh: Vec<(String, String)> = {
        let probed = lock(&shared.analysis.probed);
        project
            .media
            .iter()
            .map(|media| (media.id.to_string(), media.path.clone()))
            .filter(|(id, path)| probed.get(id) != Some(path))
            .filter(|(_, path)| Path::new(path).is_file())
            .collect()
    };
    if fresh.is_empty() {
        return;
    }
    let (shared, tx) = (shared.clone(), tx.clone());
    std::thread::spawn(move || {
        let mut infos = Map::new();
        for (id, path) in fresh {
            if let Ok(info) = probe(&path) {
                lock(&shared.analysis.probed).insert(id.clone(), path);
                infos.insert(id, info);
            }
        }
        if !infos.is_empty() && !shared.closed.load(std::sync::atomic::Ordering::SeqCst) {
            emit(&tx, json!({"type": "media_info", "media": infos}));
        }
    });
}

// ------------------------------------------------------------- media index

/// The `media_index` event of `index`, for a file `duration` seconds long:
/// `cuts` (seconds, from the media's first frame), `ranges` (the shots the
/// cuts make, `{start, end}`), `silences` (`[start, end]` pauses),
/// `loudest` (seconds, in time order) and `peaks` (`{at, level}` in dBFS),
/// plus the sound's `onsets` and `tempo` when it has a steady beat.
pub(crate) fn index_event(media: &str, duration: f64, index: &MediaIndex) -> Value {
    let mut bounds = vec![0.0];
    bounds.extend(
        index
            .cuts
            .iter()
            .copied()
            .filter(|cut| *cut > 0.0 && *cut < duration),
    );
    bounds.push(duration);
    let ranges: Vec<Value> = bounds
        .windows(2)
        .filter(|pair| pair[1] > pair[0])
        .map(|pair| json!({"start": pair[0], "end": pair[1]}))
        .collect();
    let silences: Vec<Value> = index
        .pauses
        .iter()
        .map(|(start, end)| json!([start, end]))
        .collect();
    let mut loudest: Vec<f64> = index.peaks.iter().map(|peak| peak.at).collect();
    loudest.sort_by(f64::total_cmp);
    json!({
        "type": "media_index", "media": media, "duration": duration,
        "cuts": index.cuts, "ranges": ranges, "silences": silences, "loudest": loudest,
        "peaks": index.peaks, "onsets": index.onsets, "tempo": index.tempo,
    })
}

/// `media_index {media}`: from the project's cache, else made now. A
/// refusal is `error {command: media_index, media, message}`.
pub(crate) fn media_index(shared: Arc<Shared>, tx: SyncSender<Value>, media: String) {
    if !lock(&shared.analysis.indexing).insert(media.clone()) {
        // The one running answers.
        return;
    }
    std::thread::spawn(move || {
        let event = index(&shared, &media).unwrap_or_else(|message| {
            json!({"type": "error", "message": message, "command": "media_index", "media": media})
        });
        lock(&shared.analysis.indexing).remove(&media);
        emit(&tx, event);
    });
}

fn index(shared: &Shared, media: &str) -> Result<Value, String> {
    let project = model(shared).ok_or("The project is not open yet")?;
    let found = project
        .media
        .iter()
        .find(|found| found.id == media)
        .ok_or_else(|| format!("no media `{media}`"))?;
    let path = Path::new(&found.path);
    if !path.is_file() {
        return Err(format!("{} is offline", found.path));
    }
    let cache = DerivativeCache::new(&shared.directory);
    let index = match kerf_engine::cached_media_index(path, &cache) {
        Some(index) => index,
        None => kerf_engine::media_index(path, &cache, &shared.analysis.cancel)
            .map_err(|error| error.to_string())?,
    };
    Ok(index_event(media, found.duration.seconds(), &index))
}
