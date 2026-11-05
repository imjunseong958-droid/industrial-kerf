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

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_engine::{Loudness, Peak, Tempo};
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    fn ffmpeg() -> bool {
        Command::new(kerf_engine::ffmpeg::resolve_ffmpeg_bin())
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    /// A two-second clip whose picture cuts from black to white at 1 s, with
    /// a sine tone over its second half.
    fn clip(path: &Path) -> bool {
        Command::new(kerf_engine::ffmpeg::resolve_ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "color=black:s=160x90:r=24:d=1"])
            .args(["-f", "lavfi", "-i", "color=white:s=160x90:r=24:d=1"])
            .args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo:d=1"])
            .args(["-f", "lavfi", "-i", "sine=f=1000:r=48000:d=1"])
            .args([
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0[v];[2:a][3:a]concat=n=2:v=0:a=1,aformat=channel_layouts=stereo[a]",
                "-map", "[v]", "-map", "[a]", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                "-c:a", "pcm_s16le",
            ])
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    fn project_with(directory: &Path, file: &str) -> Project {
        let mut project = kerf_runtime::parse_document(&format!(
            "kerf 1\nproject name=\"Tone\"\n\
             media m1 path=\"{file}\" rate=24 dur=48f video=160x90 audio=2ch@48000\n\
             sequence s1 rate=24 size=160x90 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=48f\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=m1 src=0f at=0f dur=48f\n"
        ))
        .unwrap();
        kerf_runtime::resolve_media_paths(&mut project, directory);
        project
    }

    #[test]
    fn analysis_commands_read_as_the_contract_says() {
        assert!(matches!(
            serde_json::from_value(json!({"type": "loudness"})).unwrap(),
            crate::Command::Loudness { sequence: None }
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "loudness", "sequence": "s2"})).unwrap(),
            crate::Command::Loudness { sequence: Some(s) } if s == "s2"
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "media_info"})).unwrap(),
            crate::Command::MediaInfo { media: None }
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "media_info", "media": "m1"})).unwrap(),
            crate::Command::MediaInfo { media: Some(m) } if m == "m1"
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "media_index", "media": "m1"})).unwrap(),
            crate::Command::MediaIndex { media } if media == "m1"
        ));
        assert!(serde_json::from_value::<crate::Command>(json!({"type": "media_index"})).is_err());
    }

    #[test]
    fn the_index_event_names_shots_pauses_and_loud_moments() {
        let index = MediaIndex {
            cuts: vec![1.5, 4.0, 9.5],
            pauses: vec![(0.0, 0.4), (6.0, 6.8)],
            peaks: vec![
                Peak {
                    at: 7.0,
                    level: -3.0,
                },
                Peak {
                    at: 2.0,
                    level: -6.0,
                },
            ],
            onsets: vec![0.5, 2.0],
            tempo: Some(Tempo {
                bpm: 120.0,
                first: 0.5,
                confidence: 0.4,
            }),
        };
        let event = index_event("m1", 8.0, &index);
        assert_eq!(event["type"], "media_index");
        assert_eq!(event["media"], "m1");
        assert_eq!(event["cuts"], json!([1.5, 4.0, 9.5]));
        // A cut past the end makes no shot.
        assert_eq!(
            event["ranges"],
            json!([{"start": 0.0, "end": 1.5}, {"start": 1.5, "end": 4.0}, {"start": 4.0, "end": 8.0}])
        );
        assert_eq!(event["silences"], json!([[0.0, 0.4], [6.0, 6.8]]));
        assert_eq!(event["loudest"], json!([2.0, 7.0]));
        assert_eq!(event["peaks"][0], json!({"at": 7.0, "level": -3.0}));
        assert_eq!(event["tempo"]["bpm"], 120.0);
        // One shot when nothing cuts.
        let still = index_event("m2", 3.0, &MediaIndex::default());
        assert_eq!(still["ranges"], json!([{"start": 0.0, "end": 3.0}]));
        assert_eq!(still["tempo"], Value::Null);
    }

    #[test]
    fn the_ready_event_is_the_engines_measurement() {
        let loudness = Loudness {
            integrated: -17.8,
            lra: 4.2,
            true_peak: [-1.4, -1.6],
            true_peak_max: -1.4,
            short_term_max: -12.0,
            momentary_max: f64::NEG_INFINITY,
            series: vec![
                (0.1, f64::NEG_INFINITY, f64::NEG_INFINITY),
                (0.2, -20.0, -12.0),
            ],
            duration: 0.2,
        };
        let event = ready(&loudness, 7, Some("s1"));
        assert_eq!(event["type"], "loudness");
        assert_eq!(event["status"], "ready");
        assert_eq!(event["integrated"], -17.8);
        assert_eq!(event["truePeak"], json!([-1.4, -1.6]));
        assert_eq!(event["momentaryMax"], Value::Null, "silence is null");
        assert_eq!(event["series"][0], json!([0.1, null, null]));
        assert_eq!(
            (event["target"].clone(), event["limit"].clone()),
            (json!(-23.0), json!(-1.0))
        );
        assert_eq!(event["revision"], 7);
        assert_eq!(event["sequence"], "s1");
        assert_eq!(event["shortTermMaxAt"], 0.2);
    }

    #[test]
    fn media_info_names_what_each_file_holds() {
        if !ffmpeg() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("tone.mov");
        assert!(clip(&file));
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let project = project_with(directory.path(), "tone.mov");
        *shared.model.lock().unwrap() = Some(project.clone());
        let (tx, rx) = mpsc::sync_channel(16);
        media_info(shared.clone(), tx.clone(), None);
        let event = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(event["type"], "media_info");
        let info = &event["media"]["m1"];
        assert_eq!(info["codec"], "h264");
        assert_eq!(info["pixFmt"], "yuv420p");
        assert_eq!(info["bitDepth"], 8);
        assert_eq!(info["audioCodec"], "pcm_s16le");
        assert_eq!(info["audioChannels"], 2);
        // Unknown media are refused by name.
        media_info(shared.clone(), tx.clone(), Some("nope".into()));
        let refused = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(refused["command"], "media_info");
        assert_eq!(refused["media"], "nope");
        // New media are probed on their own, once.
        media_changed(&shared, &tx, &project);
        let event = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(event["media"]["m1"].is_object(), "{event}");
        media_changed(&shared, &tx, &project);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn media_index_finds_the_cut_and_the_pause() {
        if !ffmpeg() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("tone.mov");
        assert!(clip(&file));
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        *shared.model.lock().unwrap() = Some(project_with(directory.path(), "tone.mov"));
        let (tx, rx) = mpsc::sync_channel(16);
        media_index(shared.clone(), tx.clone(), "m1".into());
        let event = rx.recv_timeout(Duration::from_secs(60)).unwrap();
        assert_eq!(event["type"], "media_index", "{event}");
        let cuts = event["cuts"].as_array().unwrap();
        assert_eq!(cuts.len(), 1, "{event}");
        assert!((cuts[0].as_f64().unwrap() - 1.0).abs() < 0.1, "{event}");
        assert_eq!(event["ranges"].as_array().unwrap().len(), 2, "{event}");
        let silence = &event["silences"][0];
        assert!(
            silence[0].as_f64().unwrap() < 0.1 && silence[1].as_f64().unwrap() > 0.8,
            "{event}"
        );
        // The second time it comes from the cache.
        media_index(shared.clone(), tx.clone(), "m1".into());
        let again = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(again["cuts"], event["cuts"]);
        media_index(shared, tx, "gone".into());
        let refused = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(refused["command"], "media_index");
        assert_eq!(refused["media"], "gone");
    }

    #[test]
    fn the_loudness_of_a_generated_programme_is_measured() {
        if !ffmpeg() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("tone.mov");
        assert!(clip(&file));
        let project = project_with(directory.path(), "tone.mov");
        let measured = kerf_engine::loudness::measure_program(
            &project,
            None,
            directory.path(),
            &CancellationToken::new(),
        )
        .unwrap();
        let event = ready(&measured, 3, None);
        let integrated = event["integrated"].as_f64().expect("a tone has a loudness");
        assert!(integrated < -3.0 && integrated > -40.0, "{event}");
        assert!(event["truePeak"][0].as_f64().unwrap() < 0.5, "{event}");
        assert!(!event["series"].as_array().unwrap().is_empty());
        assert!(event.get("sequence").is_none());
    }
}
