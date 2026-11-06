//! The sound of a clip that changes speed.
//!
//! FFmpeg's `atempo` takes one tempo. Sending it timed changes drifts, by nearly half a second over
//! a few seconds when measured, so a ramp's sound is made here, by waveform-similarity overlap-add:
//! each grain of output is read from where the ramp says the source is at that moment, shifted by
//! up to [`SEARCH`] to where it lines up best with the grain before. It never drifts, since each
//! grain looks up its place afresh, and it keeps its pitch, as a clip at one speed does.
//!
//! The result is a WAV file, and the project given back plays it at speed 1 in the clip's place, so
//! a render's graph and playback's mixer mix it as they would any media. Files are named by what
//! they depend on, and an edit that leaves a clip alone finds its sound already made.

use crate::sound::{Sounds, RATE};
use kerf_core::raster::handles;
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{AudioInfo, Clip, Content, Id, IdGen, Media, Project, Speed, Time, TrackKind};
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

/// Samples in a grain: about 43 ms at 48 kHz.
const WINDOW: usize = 2048;
/// Grains overlap by half, which the window adds back up to exactly 1.
const HOP: usize = WINDOW / 2;
/// How far a grain may move from its place to line up: 10 ms, the most it can be out of sync.
const SEARCH: i64 = 480;

/// How much made sound stays on disk.
const KEPT_BYTES: u64 = 2 << 30;

/// Sound of a stretch of a nested sequence: stereo samples, and the frame of `RATE` they start at.
pub type Read = (Arc<Vec<f32>>, i64);

/// `project` with the sound of every clip on a sound track that ramps made into a file of its own.
/// `nested` gives the sound of `[from, to)` of a nested sequence; playback, which leaves out nested
/// sequences at another speed, gives none. A clip whose sound cannot be read or kept stays as it was
/// and plays at its speed overall.
pub fn with_sound<'a>(
    project: &'a Project,
    mut nested: impl FnMut(&Id, Time, Time) -> Option<Read>,
) -> Cow<'a, Project> {
    let mut made: Option<Project> = None;
    let mut ids: Option<IdGen> = None;
    let sounds = Sounds::default();
    for (s, sequence) in project.sequences.iter().enumerate() {
        for (t, track) in sequence.tracks.iter().enumerate() {
            if track.kind != TrackKind::Audio {
                continue;
            }
            for (c, clip) in track.clips.iter().enumerate() {
                if clip.speed().constant().is_some() {
                    continue;
                }
                let (pre, post) = handles(track, clip);
                let Some((path, length)) =
                    sound_of(project, clip, (pre, post), &sounds, &mut nested)
                else {
                    continue;
                };
                let ids =
                    ids.get_or_insert_with(|| IdGen::new(project.media.iter().map(|m| &m.id)));
                let id = ids.next("ramp");
                let out = made.get_or_insert_with(|| project.clone());
                out.media.push(Media {
                    id: id.clone(),
                    path,
                    rate: sequence.rate,
                    drop_frame: false,
                    start: Time::ZERO,
                    duration: length,
                    video: None,
                    audio: Some(AudioInfo {
                        channels: 2,
                        sample_rate: RATE,
                    }),
                    reel: None,
                });
                out.sequences[s].tracks[t].clips[c].content = Content::Media {
                    media: id,
                    source_in: pre,
                    speed: Speed::ONE,
                };
            }
        }
    }
    made.map_or(Cow::Borrowed(project), Cow::Owned)
}

/// Where a clip's sound was made and how long it is, handles included.
fn sound_of(
    project: &Project,
    clip: &Clip,
    (pre, post): (Time, Time),
    sounds: &Sounds,
    nested: &mut impl FnMut(&Id, Time, Time) -> Option<Read>,
) -> Option<(String, Time)> {
    let speed = clip.speed();
    let (source_in, _) = clip.source_range()?;
    let span = pre + clip.duration + post;
    let frames = usize::try_from(span.flicks() / (FLICKS_PER_SECOND / i64::from(RATE))).ok()?;
    let length = Time::from_flicks(frames as i64 * (FLICKS_PER_SECOND / i64::from(RATE)));
    let mut hash = DefaultHasher::new();
    format!("{speed:?}|{source_in:?}|{:?}|{span:?}", clip.duration).hash(&mut hash);
    match &clip.content {
        Content::Media { media, .. } => {
            let media = project.media(media.as_str())?;
            media.audio.as_ref()?;
            let file = std::fs::metadata(&media.path).ok()?;
            (media.path.as_str(), file.len(), file.modified().ok()).hash(&mut hash);
        }
        Content::Sequence { sequence, .. } => {
            let nested = project.sequence(sequence.as_str())?;
            format!("{nested:?}|{:?}", project.media).hash(&mut hash);
        }
        Content::Generator { .. } | Content::Caption { .. } => return None,
    }
    let folder = std::env::temp_dir().join("kerf-ramped-sound");
    std::fs::create_dir_all(&folder).ok()?;
    let path = folder.join(format!("{:016x}.wav", hash.finish()));
    if !path.is_file() {
        let (from, to) = (speed.played(-pre), speed.played(clip.duration + post));
        let (low, high) = if speed.is_reverse() {
            let out = source_in + speed.played(clip.duration);
            (out - to, out - from)
        } else {
            (source_in + from, source_in + to)
        };
        let (read, first) = match &clip.content {
            Content::Media { media, .. } => (sounds.of(&project.media(media.as_str())?.path)?, 0),
            Content::Sequence { sequence, .. } => nested(sequence, low, high)?,
            _ => return None,
        };
        let mut input = stretch_of(&read, first, low, high);
        if speed.is_reverse() {
            reverse_frames(&mut input);
        }
        let start = speed.played_seconds(-pre.seconds());
        let position = |frame: usize| {
            let at = frame as f64 / f64::from(RATE) - pre.seconds();
            (speed.played_seconds(at) - start) * f64::from(RATE)
        };
        write_wav(&path, &stretch(&input, frames, position))?;
        keep_within(&folder, KEPT_BYTES);
    }
    Some((path.to_string_lossy().into_owned(), length))
}

/// `[low, high)` of a sound starting at frame `first`, silent where the sound has nothing.
fn stretch_of(sound: &[f32], first: i64, low: Time, high: Time) -> Vec<f32> {
    let frame = |time: Time| (time.seconds() * f64::from(RATE)).round() as i64;
    let (from, to) = (frame(low), frame(high));
    let mut out = vec![0.0; 2 * usize::try_from(to - from).unwrap_or(0)];
    for (index, sample) in out.iter_mut().enumerate() {
        let at = 2 * (from - first) + index as i64;
        if let Some(found) = usize::try_from(at).ok().and_then(|at| sound.get(at)) {
            *sample = *found;
        }
    }
    out
}

fn reverse_frames(samples: &mut [f32]) {
    samples.reverse();
    for frame in samples.chunks_exact_mut(2) {
        frame.swap(0, 1);
    }
}

/// `frames` of stereo output from stereo `input`, where `position` says which frame of the input
/// plays at each frame of the output.
pub(crate) fn stretch(input: &[f32], frames: usize, position: impl Fn(usize) -> f64) -> Vec<f32> {
    let mono: Vec<f32> = input
        .chunks_exact(2)
        .map(|frame| (frame[0] + frame[1]) / 2.0)
        .collect();
    let window: Vec<f32> = (0..WINDOW)
        .map(|at| {
            let turn = at as f64 / WINDOW as f64 * std::f64::consts::TAU;
            (0.5 - 0.5 * turn.cos()) as f32
        })
        .collect();
    let mut out = vec![0.0; frames * 2];
    let mut previous: Option<i64> = None;
    // The first grain starts half a window early, so the first samples are covered twice over too.
    let mut at = -(HOP as i64);
    while at < frames as i64 {
        let centre = (at + (WINDOW / 2) as i64).clamp(0, frames.saturating_sub(1) as i64);
        let wanted = position(centre as usize).round() as i64 - (WINDOW / 2) as i64;
        let start = match previous {
            Some(previous) => line_up(&mono, previous + HOP as i64, wanted),
            None => wanted,
        };
        for (offset, weight) in window.iter().enumerate() {
            let Some(into) = usize::try_from(at + offset as i64)
                .ok()
                .filter(|i| *i < frames)
            else {
                continue;
            };
            let Some(from) = usize::try_from(start + offset as i64)
                .ok()
                .filter(|from| *from < mono.len())
            else {
                continue;
            };
            out[into * 2] += input[from * 2] * weight;
            out[into * 2 + 1] += input[from * 2 + 1] * weight;
        }
        previous = Some(start);
        at += HOP as i64;
    }
    out
}

/// The start within [`SEARCH`] of `wanted` that lines up best with what follows the last grain,
/// `natural`. Searched coarsely over every eighth start and then finely around the best.
fn line_up(mono: &[f32], natural: i64, wanted: i64) -> i64 {
    // Playing on is as good as it gets, and as a speed of 1 it is the file itself.
    if (natural - wanted).abs() <= 1 {
        return natural;
    }
    let score = |start: i64, stride: usize| -> f64 {
        let (mut product, mut energy) = (0.0f64, 0.0f64);
        for offset in (0..HOP).step_by(stride) {
            let read = |base: i64| {
                usize::try_from(base + offset as i64)
                    .ok()
                    .and_then(|at| mono.get(at))
                    .copied()
                    .unwrap_or(0.0)
            };
            let (a, b) = (f64::from(read(natural)), f64::from(read(start)));
            product += a * b;
            energy += b * b;
        }
        if energy > 1e-9 {
            product / energy.sqrt()
        } else {
            f64::NEG_INFINITY
        }
    };
    let mut best = (wanted, f64::NEG_INFINITY);
    let mut offset = -SEARCH;
    while offset <= SEARCH {
        let found = score(wanted + offset, 4);
        if found > best.1 {
            best = (wanted + offset, found);
        }
        offset += 8;
    }
    if best.1 == f64::NEG_INFINITY {
        return wanted;
    }
    let around = best.0;
    let mut finest = (around, f64::NEG_INFINITY);
    for start in around - 7..=around + 7 {
        let found = score(start, 2);
        if found > finest.1 {
            finest = (start, found);
        }
    }
    finest.0.clamp(wanted - SEARCH, wanted + SEARCH)
}

/// A stereo WAV of 32-bit floats, written whole under a name no reader looks for yet.
fn write_wav(path: &std::path::Path, samples: &[f32]) -> Option<()> {
    let bytes = (samples.len() * 4) as u32;
    let mut out = Vec::with_capacity(samples.len() * 4 + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 8).to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    let partial: PathBuf = path.with_extension("partial");
    std::fs::write(&partial, out).ok()?;
    std::fs::rename(&partial, path).ok()
}

/// Delete the oldest files of `folder` past `budget`.
fn keep_within(folder: &std::path::Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    files.sort();
    let mut total: u64 = files.iter().map(|(_, bytes, _)| bytes).sum();
    for (_, bytes, path) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= bytes;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = RATE as f64;

    fn sine(frames: usize, hertz: f64) -> Vec<f32> {
        (0..frames)
            .flat_map(|at| {
                let value = (at as f64 / R * hertz * std::f64::consts::TAU).sin() as f32;
                [value, value]
            })
            .collect()
    }

    /// Bursts of tone a tenth of a second long, every half second of the input.
    fn bursts(frames: usize) -> Vec<f32> {
        let tone = sine(frames, 1000.0);
        tone.chunks_exact(2)
            .enumerate()
            .flat_map(|(at, frame)| {
                let on = (at as f64 / R) % 0.5 < 0.1;
                [
                    frame[0] * f32::from(u8::from(on)),
                    frame[1] * f32::from(u8::from(on)),
                ]
            })
            .collect()
    }

    /// Where each burst of output starts, in seconds.
    fn onsets(samples: &[f32]) -> Vec<f64> {
        let block = 96;
        let loud: Vec<bool> = samples
            .chunks(block * 2)
            .map(|chunk| chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32 > 0.05)
            .collect();
        // A burst may flicker where its grains meet; a new one follows a fifth of a second of quiet.
        let quiet = (0.2 * R) as usize / block;
        loud.iter()
            .enumerate()
            .filter(|(at, on)| **on && !loud[at.saturating_sub(quiet)..*at].contains(&true))
            .map(|(at, _)| (at * block) as f64 / R)
            .collect()
    }

    fn crossings_per_second(samples: &[f32]) -> f64 {
        let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f64 / (left.len() as f64 / R)
    }

    fn rms_of(samples: &[f32]) -> f64 {
        (samples.iter().map(|s| f64::from(s * s)).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn at_speed_one_the_sound_comes_back_exactly() {
        let input = sine(48_000, 440.0);
        let out = stretch(&input, 48_000, |at| at as f64);
        let middle = WINDOW * 2..out.len() - WINDOW * 2;
        for (made, was) in out[middle.clone()].iter().zip(&input[middle]) {
            assert!((made - was).abs() < 1e-3, "{made} against {was}");
        }
    }

    #[test]
    fn slowing_down_keeps_the_pitch_and_the_level() {
        for speed in [0.25, 0.5, 0.8, 1.5, 3.0] {
            let input = sine(4 * 48_000, 440.0);
            let frames = (48_000.0 / speed * 0.9) as usize;
            let out = stretch(&input, frames.min(96_000), |at| at as f64 * speed);
            let body = &out[WINDOW * 2..out.len() - WINDOW * 2];
            let pitch = crossings_per_second(body);
            assert!((pitch - 440.0).abs() < 4.0, "at {speed}: {pitch} Hz");
            let level = rms_of(body);
            assert!((level - 0.707).abs() < 0.05, "at {speed}: level {level}");
        }
    }

    #[test]
    fn a_ramp_lands_every_burst_where_the_ramp_says() {
        // From 1 down to 1/4 over the first four seconds, then 1/4.
        let speed = |t: f64| {
            let within = t.min(4.0);
            within - 0.75 * within * within / 8.0 + (t - within).max(0.0) * 0.25
        };
        let input = bursts(5 * 48_000);
        let out = stretch(&input, 16 * 48_000, |at| speed(at as f64 / R) * R);
        let found = onsets(&out);
        let mut checked = 0;
        for (index, onset) in found.iter().enumerate() {
            let source = index as f64 * 0.5;
            // When the ramp reaches that source.
            let (mut low, mut high) = (0.0, 20.0);
            for _ in 0..40 {
                let middle = (low + high) / 2.0;
                if speed(middle) < source {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            assert!(
                (onset - low).abs() < 0.06,
                "burst {index}: heard at {onset}, the ramp says {low}"
            );
            checked += 1;
        }
        assert!(checked >= 8, "only {checked} bursts: {found:?}");
    }

    #[test]
    fn reversing_the_frames_swaps_nothing_else() {
        let mut samples = vec![1.0, 2.0, 3.0, 4.0];
        reverse_frames(&mut samples);
        assert_eq!(samples, [3.0, 4.0, 1.0, 2.0]);
    }

    #[test]
    fn a_wav_reads_as_its_samples() {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("a.wav");
        write_wav(&path, &[0.5, -0.5, 0.25, -0.25]).expect("written");
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 16);
        assert_eq!(&bytes[44..48], &0.5f32.to_le_bytes());
    }

    #[test]
    fn the_oldest_files_go_past_the_budget() {
        let folder = tempfile::tempdir().expect("a folder");
        for name in ["a", "b", "c"] {
            std::fs::write(folder.path().join(name), [0u8; 100]).expect("written");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        keep_within(folder.path(), 150);
        let left: Vec<_> = std::fs::read_dir(folder.path())
            .expect("reads")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left, ["c"]);
    }
}
