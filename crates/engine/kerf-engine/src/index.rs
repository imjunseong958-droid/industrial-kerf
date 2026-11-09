//! A media file's index: where its picture cuts, where its sound pauses,
//! its loudest moments, the onsets of its sound (hits, beats, the starts of
//! words), and its tempo when it has a steady one.
//!
//! One FFmpeg pass reads the file: scene detection on a small copy of the
//! picture, and silence detection and a loudness envelope on a mono
//! low-rate copy of the sound. The rest is computed from the envelope.
//! Indexes are cached with the media's other derivatives.

use crate::derivatives::DerivativeCache;
use crate::error::{EngineError, Result};
use crate::ffmpeg::{resolve_ffmpeg_bin, video_decode_prefix};
use crate::probe::{info, parse_scene_times, parse_silencedetect};
use crate::sidecar::{self, CancellationToken};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::Path;

const SCENES: &str = "scenes.txt";
const ENVELOPE: &str = "envelope.txt";
const SCENE_THRESHOLD: f64 = 0.35;
const SCENE_WIDTH: u32 = 160;
const PAUSE_NOISE_DB: f64 = -35.0;
const PAUSE_SECONDS: f64 = 0.35;
const SAMPLE_RATE: u32 = 8_000;
/// The loudness envelope has one value per hop.
const HOP_SECONDS: f64 = 0.01;
/// Quieter than this counts as silence.
const FLOOR_DB: f64 = -80.0;
/// Loudest moments: at most this many, this far apart, this far above the
/// median loudness.
const MAX_PEAKS: usize = 16;
const PEAK_SPACING_SECONDS: f64 = 3.0;
const PEAK_OVER_MEDIAN_DB: f64 = 6.0;
const PEAK_SMOOTHING_SECONDS: f64 = 1.0;
/// An onset is a rise of at least this much over two hops, louder than
/// `ONSET_FLOOR_DB`, and at least `ONSET_SPACING_SECONDS` after the last.
const ONSET_RISE_DB: f64 = 6.0;
const ONSET_FLOOR_DB: f64 = -50.0;
const ONSET_SPACING_SECONDS: f64 = 0.1;
/// Tempos from 60 to 200 beats a minute are looked for; one is reported
/// when the onsets repeat at least this regularly.
const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 200.0;
const MIN_TEMPO_CONFIDENCE: f64 = 0.2;
const MIN_TEMPO_ONSETS: usize = 8;

/// One of the loudest moments, in seconds, and its loudness in dBFS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Peak {
    pub at: f64,
    pub level: f64,
}

/// A steady beat: beats a minute, the first beat in seconds, and how
/// regular the onsets are (0 to 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tempo {
    pub bpm: f64,
    pub first: f64,
    pub confidence: f64,
}

/// What [`media_index`] finds, in seconds from the media's first frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaIndex {
    pub cuts: Vec<f64>,
    pub pauses: Vec<(f64, f64)>,
    pub peaks: Vec<Peak>,
    pub onsets: Vec<f64>,
    pub tempo: Option<Tempo>,
}

fn index_entry(src: &Path, cache: &DerivativeCache) -> Result<crate::derivatives::CacheEntry> {
    cache.source_entry(
        src,
        "media-index",
        &json!({
            "version": 2,
            "sceneThreshold": SCENE_THRESHOLD,
            "sceneWidth": SCENE_WIDTH,
            "pauseNoiseDb": PAUSE_NOISE_DB,
            "pauseSeconds": PAUSE_SECONDS,
            "sampleRate": SAMPLE_RATE,
            "hopSeconds": HOP_SECONDS,
        }),
    )
}

/// The index of `src` when the cache already has it; nothing is made.
pub fn cached_media_index(src: &Path, cache: &DerivativeCache) -> Option<MediaIndex> {
    let entry = index_entry(src, cache).ok()?;
    if entry.is_complete(&["index.json"]) {
        entry.read_json("index.json").ok()
    } else {
        None
    }
}

/// The index of `src`, from the cache or made now.
pub fn media_index(
    src: &Path,
    cache: &DerivativeCache,
    cancellation: &CancellationToken,
) -> Result<MediaIndex> {
    let entry = index_entry(src, cache)?;
    if entry.is_complete(&["index.json"]) {
        return entry.read_json("index.json");
    }
    let source = src.to_string_lossy();
    let probe = info(&source)?;
    let has_video = probe.width.is_some();
    let has_audio = probe.has_audio == Some(true);
    let scenes = entry.path(SCENES);
    let envelope = entry.path(ENVELOPE);
    let mut graph = Vec::new();
    let mut outputs = Vec::new();
    if has_video {
        // The first frame always passes, so a single shot still gives the
        // output a frame; a cut at 0 is not reported.
        graph.push(format!(
            "[0:v:0]scale={SCENE_WIDTH}:-2,select='eq(n,0)+gt(scene,{SCENE_THRESHOLD})',metadata=mode=print:file={SCENES}[v]"
        ));
        outputs.extend(["-map", "[v]", "-f", "null", "-"]);
    }
    if has_audio {
        let hop = (f64::from(SAMPLE_RATE) * HOP_SECONDS).round() as u32;
        graph.push(format!(
            "[0:a:0]aresample={SAMPLE_RATE},aformat=sample_fmts=flt:channel_layouts=mono,\
             silencedetect=noise={PAUSE_NOISE_DB}dB:d={PAUSE_SECONDS},\
             asetnsamples=n={hop}:p=0,astats=metadata=1:reset=1,\
             ametadata=mode=print:key=lavfi.astats.Overall.RMS_level:file={ENVELOPE}[a]"
        ));
        outputs.extend(["-map", "[a]", "-f", "null", "-"]);
    }
    let mut index = MediaIndex::default();
    if !graph.is_empty() {
        // FFmpeg writes the metadata files into the entry, named plainly so
        // no path needs escaping inside the filter graph.
        let mut command = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin());
        command
            .current_dir(&entry.directory)
            .args(video_decode_prefix())
            .args(["-hide_banner", "-nostats", "-i"])
            .arg(src.canonicalize()?)
            .args(["-filter_complex", &graph.join(";")])
            .args(outputs);
        let mut stderr = String::new();
        let finished = sidecar::run(command, cancellation, |line| {
            if line.contains("silence_") {
                stderr.push_str(line);
                stderr.push('\n');
            }
        })?;
        if !finished.status.success() {
            return Err(EngineError::msg(format!(
                "media analysis failed: {}",
                finished.stderr.trim()
            )));
        }
        if has_video {
            index.cuts = parse_scene_times(&fs::read_to_string(&scenes)?)
                .into_iter()
                .map(|scene| scene.start)
                .filter(|start| *start > 0.0)
                .collect();
        }
        if has_audio {
            let end = probe.duration.unwrap_or(0.0);
            index.pauses = parse_silencedetect(&stderr)
                .into_iter()
                .map(|pause| (pause.start, pause.end.unwrap_or(end)))
                .collect();
            let levels = parse_envelope(&fs::read_to_string(&envelope)?);
            index.peaks = peaks(&levels);
            index.onsets = onsets(&levels);
            index.tempo = tempo(&levels, index.onsets.len());
        }
        let _ = fs::remove_file(&scenes);
        let _ = fs::remove_file(&envelope);
    }
    entry.write_json("index.json", &index)?;
    entry.mark_complete(&["index.json"])?;
    Ok(index)
}

/// Every frame of `rate` a second of the picture of `src` from its start, as `width` by `height` bytes
/// of grey, going to `each` with how far in it is in seconds. One FFmpeg pass decodes it as it goes, so
/// that a long file takes no more memory than a short one.
pub fn small_grey_frames(
    src: &Path,
    rate: u32,
    size: (u32, u32),
    each: impl FnMut(f64, &[u8]),
) -> Result<()> {
    grey_frames_with(resolve_ffmpeg_bin(), src, rate, size, each)
}

/// [`small_grey_frames`] with the FFmpeg at `ffmpeg`.
fn grey_frames_with(
    ffmpeg: &str,
    src: &Path,
    rate: u32,
    (width, height): (u32, u32),
    mut each: impl FnMut(f64, &[u8]),
) -> Result<()> {
    let mut child = crate::ffmpeg::hidden_command(ffmpeg)
        .args(video_decode_prefix())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(src)
        .args(["-map", "0:v:0", "-vf"])
        .arg(format!("fps={rate},scale={width}:{height},format=gray"))
        .args(["-f", "rawvideo", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    // Read as it writes: an FFmpeg that says more than a pipe holds (a decoder failing on every
    // frame, as VideoToolbox does on 4:4:4) would otherwise wait on its stderr while this waits on
    // its stdout, and neither would end.
    let errors = crate::raster::drain_stderr(&mut child);
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut grey = vec![0u8; (width * height) as usize];
    let mut frame = 0u64;
    while std::io::Read::read_exact(&mut stdout, &mut grey).is_ok() {
        each(frame as f64 / f64::from(rate), &grey);
        frame += 1;
    }
    drop(stdout);
    let status = child.wait()?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() {
        return Err(EngineError::msg(format!(
            "the picture could not be read: {}",
            errors.trim()
        )));
    }
    Ok(())
}

/// One loudness per hop, in dBFS, from `ametadata` print output.
fn parse_envelope(text: &str) -> Vec<f64> {
    text.lines()
        .filter_map(|line| line.strip_prefix("lavfi.astats.Overall.RMS_level="))
        .map(|value| {
            value
                .trim()
                .parse::<f64>()
                .unwrap_or(FLOOR_DB)
                .max(FLOOR_DB)
        })
        .collect()
}

fn hops(seconds: f64) -> usize {
    (seconds / HOP_SECONDS).round() as usize
}

fn at(hop: usize) -> f64 {
    (hop as f64 * HOP_SECONDS * 1000.0).round() / 1000.0
}

/// The loudest moments of a smoothed envelope, in time order.
fn peaks(levels: &[f64]) -> Vec<Peak> {
    if levels.is_empty() {
        return Vec::new();
    }
    let power: Vec<f64> = levels.iter().map(|db| 10f64.powf(db / 10.0)).collect();
    let half = hops(PEAK_SMOOTHING_SECONDS) / 2;
    let mut prefix = vec![0.0];
    for value in &power {
        prefix.push(prefix.last().unwrap() + value);
    }
    let smooth: Vec<f64> = (0..power.len())
        .map(|i| {
            let (from, to) = (i.saturating_sub(half), (i + half + 1).min(power.len()));
            let mean = (prefix[to] - prefix[from]) / (to - from) as f64;
            (10.0 * mean.log10()).max(FLOOR_DB)
        })
        .collect();
    let mut sorted = smooth.clone();
    sorted.sort_by(f64::total_cmp);
    let floor = sorted[sorted.len() / 2] + PEAK_OVER_MEDIAN_DB;
    let mut candidates: Vec<usize> = (0..smooth.len())
        .filter(|&i| smooth[i] >= floor)
        .filter(|&i| {
            (i == 0 || smooth[i] >= smooth[i - 1])
                && (i + 1 == smooth.len() || smooth[i] > smooth[i + 1])
        })
        .collect();
    candidates.sort_by(|a, b| smooth[*b].total_cmp(&smooth[*a]));
    let spacing = hops(PEAK_SPACING_SECONDS);
    let mut chosen: Vec<usize> = Vec::new();
    for candidate in candidates {
        if chosen.len() == MAX_PEAKS {
            break;
        }
        if chosen.iter().all(|&c| c.abs_diff(candidate) >= spacing) {
            chosen.push(candidate);
        }
    }
    chosen.sort_unstable();
    chosen
        .into_iter()
        .map(|i| Peak {
            at: at(i),
            level: (smooth[i] * 10.0).round() / 10.0,
        })
        .collect()
}

/// How much louder each hop is than two hops before; before the start is
/// silence.
fn rises(levels: &[f64]) -> Vec<f64> {
    (0..levels.len())
        .map(|i| {
            let before = if i < 2 { FLOOR_DB } else { levels[i - 2] };
            (levels[i] - before).max(0.0)
        })
        .collect()
}

fn onsets(levels: &[f64]) -> Vec<f64> {
    let rise = rises(levels);
    let spacing = hops(ONSET_SPACING_SECONDS);
    let mut onsets: Vec<usize> = Vec::new();
    for i in 0..rise.len() {
        let local = rise[i.saturating_sub(2)..(i + 3).min(rise.len())]
            .iter()
            .all(|value| *value <= rise[i]);
        if rise[i] >= ONSET_RISE_DB
            && levels[i] >= ONSET_FLOOR_DB
            && local
            && onsets.last().is_none_or(|last| i - last >= spacing)
        {
            onsets.push(i);
        }
    }
    onsets.into_iter().map(at).collect()
}

/// The beat period that the rises repeat at most strongly, when they
/// repeat strongly enough.
fn tempo(levels: &[f64], onsets: usize) -> Option<Tempo> {
    if onsets < MIN_TEMPO_ONSETS {
        return None;
    }
    let rise = rises(levels);
    let mean = rise.iter().sum::<f64>() / rise.len() as f64;
    let centered: Vec<f64> = rise.iter().map(|value| value - mean).collect();
    let correlation = |lag: usize| -> f64 {
        centered
            .iter()
            .zip(&centered[lag..])
            .map(|(a, b)| a * b)
            .sum()
    };
    let energy = correlation(0);
    let (shortest, longest) = (hops(60.0 / MAX_BPM), hops(60.0 / MIN_BPM));
    if energy <= 0.0 || centered.len() <= longest + 1 {
        return None;
    }
    let scores: Vec<f64> = (shortest - 1..=longest + 1).map(correlation).collect();
    let best = (1..scores.len() - 1).max_by(|a, b| scores[*a].total_cmp(&scores[*b]))?;
    let confidence = scores[best] / energy;
    if confidence < MIN_TEMPO_CONFIDENCE {
        return None;
    }
    // Refine the period between hops with a parabola through the peak.
    let (before, here, after) = (scores[best - 1], scores[best], scores[best + 1]);
    let curvature = before - 2.0 * here + after;
    let shift = if curvature < 0.0 {
        0.5 * (before - after) / curvature
    } else {
        0.0
    };
    let period = (best + shortest - 1) as f64 + shift;
    let lag = period.round() as usize;
    let first = (0..lag)
        .max_by(|a, b| {
            let strength = |offset: usize| rise.iter().skip(offset).step_by(lag).sum::<f64>();
            strength(*a).total_cmp(&strength(*b))
        })
        .unwrap_or(0);
    Some(Tempo {
        bpm: (60.0 / (period * HOP_SECONDS) * 10.0).round() / 10.0,
        first: at(first),
        confidence: (confidence * 100.0).round() / 100.0,
    })
}

#[cfg(test)]
mod tests {

    /// An FFmpeg that fills its stderr before the frames it is read for: it must still be read to
    /// its end, where it once left both waiting for ever.
    #[cfg(unix)]
    #[test]
    fn frames_are_read_from_an_ffmpeg_that_fills_its_stderr() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("a folder");
        let fake = dir.path().join("ffmpeg");
        // 4000 lines of errors, far more than a pipe holds, then two 4x2 grey frames.
        std::fs::write(
            &fake,
            "#!/bin/sh\ni=0\nwhile [ $i -lt 4000 ]; do echo \"hardware accelerator failed to decode picture $i\" >&2; i=$((i+1)); done\nprintf '0123456789abcdef'\n",
        )
        .expect("the fake");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("executable");
        let (send, receive) = std::sync::mpsc::channel();
        let ffmpeg = fake.to_string_lossy().into_owned();
        std::thread::spawn(move || {
            let mut frames = 0;
            let result = grey_frames_with(&ffmpeg, Path::new("in.mp4"), 2, (4, 2), |_, _| frames += 1);
            let _ = send.send((result.is_ok(), frames));
        });
        let finished = receive.recv_timeout(std::time::Duration::from_secs(20));
        assert_eq!(finished, Ok((true, 2)), "the read never finished, or lost frames");
    }
    use super::*;

    /// An envelope of clicks every `period` seconds over quiet.
    fn clicks(period: f64, seconds: f64) -> Vec<f64> {
        let every = hops(period);
        (0..hops(seconds))
            .map(|i| if i % every < 3 { -12.0 } else { -60.0 })
            .collect()
    }

    #[test]
    fn clicks_have_onsets_and_a_tempo() {
        let levels = clicks(0.5, 10.0);
        let found = onsets(&levels);
        assert_eq!(found.len(), 20, "{found:?}");
        assert_eq!(found[..2], [0.0, 0.5]);
        let tempo = tempo(&levels, found.len()).unwrap();
        assert!((tempo.bpm - 120.0).abs() < 1.0, "{tempo:?}");
        assert!(tempo.first < 0.02, "{tempo:?}");
    }

    #[test]
    fn a_steady_tone_has_no_tempo() {
        let mut levels = vec![-60.0; 50];
        levels.extend(vec![-20.0; 950]);
        assert_eq!(onsets(&levels).len(), 1);
        assert!(tempo(&levels, 1).is_none());
    }

    #[test]
    fn peaks_are_the_loudest_moments_far_enough_apart() {
        let mut levels = vec![-40.0; 3000];
        for (start, level) in [(500, -10.0), (600, -12.0), (2000, -15.0)] {
            for value in &mut levels[start..start + 50] {
                *value = level;
            }
        }
        let found = peaks(&levels);
        let times: Vec<f64> = found.iter().map(|peak| peak.at).collect();
        // 6.0s is within three seconds of the louder 5.0s.
        assert_eq!(times.len(), 2, "{found:?}");
        assert!(
            (times[0] - 5.25).abs() < 0.3 && (times[1] - 20.25).abs() < 0.3,
            "{found:?}"
        );
    }

    #[test]
    fn envelopes_parse_silence_as_the_floor() {
        let text = "frame:0    pts:0       pts_time:0\nlavfi.astats.Overall.RMS_level=-inf\nframe:1    pts:80      pts_time:0.01\nlavfi.astats.Overall.RMS_level=-23.5\n";
        assert_eq!(parse_envelope(text), [FLOOR_DB, -23.5]);
    }
}
