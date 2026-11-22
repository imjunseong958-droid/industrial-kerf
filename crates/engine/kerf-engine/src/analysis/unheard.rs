//! Where a recording sounds like speech and its transcript has no words.
//!
//! A transcriber leaves things out: a sentence said quietly beside louder
//! ones, the start of a take, a whole speaker. Nothing in its words says
//! so. The sound it was given does: speech rises and falls with its
//! syllables, a few times a second, where room tone, hum and silence hold
//! still. So in each stretch of two seconds or more that the transcripts
//! of a file have no words in, the level of the speech band is followed,
//! and where syllables keep coming for two seconds or more it is said that
//! something may have been missed.
//!
//! It says where to listen, not what was said: music with a beat, clapping
//! or footsteps rise and fall as well, and read the same.

use crate::analysis::transcription::{
    cached_transcripts, wav_samples, Transcript, TRANSCRIPTION_SAMPLE_RATE,
    TRANSCRIPTION_SCHEMA_VERSION,
};
use crate::media::derivatives::DerivativeCache;
use serde::Serialize;
use std::path::Path;

/// How long the level is taken over, in seconds.
const WINDOW_SECONDS: f64 = 0.02;
/// The speech band, in hertz: what a telephone carries.
const BAND: (f64, f64) = (300.0, 3000.0);
/// How long a transcript must have no words for the sound there to be
/// looked at, in seconds. Whisper times a word to within a second or so
/// (`WORD_TIME_SLACK_SECONDS`), so a shorter gap may be a word's own sound.
const LEAST_GAP_SECONDS: f64 = 2.0;
/// How long syllables must keep coming for it to be said, in seconds.
const LEAST_TALK_SECONDS: f64 = 2.0;
/// A syllable: a peak of the level that stands this many decibels over the
/// lowest level within [`SYLLABLE_REACH_SECONDS`] to either side, and at
/// least [`SYLLABLE_APART_SECONDS`] after the one before.
const SYLLABLE_DB: f64 = 8.0;
const SYLLABLE_REACH_SECONDS: f64 = 0.3;
const SYLLABLE_APART_SECONDS: f64 = 0.1;
/// Talk: this many syllables or more, each within
/// [`TALK_APART_SECONDS`] of the one before.
const TALK_SYLLABLES: usize = 3;
const TALK_APART_SECONDS: f64 = 1.0;

/// A stretch of a file, in seconds from its first frame, that sounds like
/// speech and that no transcript of it has a word in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unheard {
    pub start: f64,
    pub end: f64,
}

/// The level of the speech band in each window of `samples`, in decibels.
fn levels(samples: &[i16]) -> Vec<f64> {
    let rate = f64::from(TRANSCRIPTION_SAMPLE_RATE);
    let pole = |hertz: f64| (-std::f64::consts::TAU * hertz / rate).exp();
    let (high, low) = (pole(BAND.0), pole(BAND.1));
    let window = (WINDOW_SECONDS * rate) as usize;
    let (mut passed, mut before, mut smooth) = (0.0f64, 0.0f64, 0.0f64);
    samples
        .chunks_exact(window)
        .map(|chunk| {
            let mut energy = 0.0;
            for sample in chunk {
                let sample = f64::from(*sample);
                // What is over the band's low edge, then what of that is under its high edge.
                passed = high * (passed + sample - before);
                before = sample;
                smooth += (1.0 - low) * (passed - smooth);
                energy += smooth * smooth;
            }
            10.0 * (energy / window as f64 + 1e-9).log10()
        })
        .collect()
}

/// The windows of `levels[from..to]` where a syllable peaks.
fn syllables(levels: &[f64], from: usize, to: usize) -> Vec<usize> {
    let to = to.min(levels.len());
    if to <= from + 2 {
        return Vec::new();
    }
    let part = &levels[from..to];
    // Over three windows, so that one window's flutter is not a peak.
    let smooth: Vec<f64> = (0..part.len())
        .map(|at| {
            let around = &part[at.saturating_sub(1)..(at + 2).min(part.len())];
            around.iter().sum::<f64>() / around.len() as f64
        })
        .collect();
    let reach = (SYLLABLE_REACH_SECONDS / WINDOW_SECONDS).round() as usize;
    let apart = (SYLLABLE_APART_SECONDS / WINDOW_SECONDS).round() as usize;
    let lowest = |values: &[f64]| values.iter().copied().fold(f64::INFINITY, f64::min);
    let mut peaks: Vec<usize> = Vec::new();
    for at in 1..smooth.len() - 1 {
        if smooth[at] < smooth[at - 1] || smooth[at] <= smooth[at + 1] {
            continue;
        }
        let before = lowest(&smooth[at.saturating_sub(reach)..=at]);
        let after = lowest(&smooth[at..(at + reach + 1).min(smooth.len())]);
        if smooth[at] - before >= SYLLABLE_DB
            && smooth[at] - after >= SYLLABLE_DB
            && peaks.last().is_none_or(|last| at - last >= apart)
        {
            peaks.push(at);
        }
    }
    peaks.into_iter().map(|at| at + from).collect()
}

/// The stretches where syllables keep coming, as windows from the first
/// of each to its last.
fn talk(syllables: &[usize]) -> Vec<(usize, usize)> {
    let apart = (TALK_APART_SECONDS / WINDOW_SECONDS).round() as usize;
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for at in syllables {
        match runs.last_mut() {
            Some(run) if at - run[run.len() - 1] <= apart => run.push(*at),
            _ => runs.push(vec![*at]),
        }
    }
    runs.into_iter()
        .filter(|run| run.len() >= TALK_SYLLABLES)
        .map(|run| (run[0], run[run.len() - 1]))
        .collect()
}

/// Where `samples`, the sound from `start` to `end` seconds of a file at
/// the transcription rate, sounds like speech in a stretch that `words`
/// (each from when to when, in seconds of the file) leave empty.
pub fn unheard_in(samples: &[i16], (start, end): (f64, f64), words: &[(f64, f64)]) -> Vec<Unheard> {
    let levels = levels(samples);
    let window = |seconds: f64| ((seconds - start) / WINDOW_SECONDS).max(0.0) as usize;
    let mut words: Vec<(f64, f64)> = words
        .iter()
        .copied()
        .filter(|(from, to)| *to > start && *from < end)
        .collect();
    words.sort_by(|a, b| a.0.total_cmp(&b.0));
    // The gaps: from where the last word so far ended to where the next starts.
    let mut found = Vec::new();
    let mut reached = start;
    for (from, to) in words.into_iter().chain([(end, end)]) {
        if from - reached >= LEAST_GAP_SECONDS {
            let talked = talk(&syllables(&levels, window(reached), window(from)));
            let seconds: f64 = talked
                .iter()
                .map(|(first, last)| (last - first) as f64 * WINDOW_SECONDS)
                .sum();
            if let (true, Some(first), Some(last)) =
                (seconds >= LEAST_TALK_SECONDS, talked.first(), talked.last())
            {
                found.push(Unheard {
                    start: (start + first.0 as f64 * WINDOW_SECONDS).max(reached),
                    end: (start + (last.1 + 1) as f64 * WINDOW_SECONDS).min(from),
                });
            }
        }
        reached = reached.max(to);
    }
    found
}

/// Where the file `src` sounds like speech and the transcripts already
/// made of it have no words, by the sound each transcript kept. Nothing is
/// transcribed, and a file with no transcript has nothing to say.
pub fn unheard_speech(src: &str, cache: &DerivativeCache) -> Vec<Unheard> {
    // The words of every transcript, joined where they meet: a moment two of them heard is
    // empty only if neither has a word there.
    let words: Vec<(f64, f64)> = cached_transcripts(src, cache)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|transcript| transcript.cues)
        .flat_map(|cue| cue.words)
        .map(|word| (word.start, word.end))
        .collect();
    let mut found: Vec<Unheard> = Vec::new();
    for entry in cache
        .source_entries(Path::new(src), "transcription")
        .unwrap_or_default()
    {
        if !entry.is_complete(&["transcript.json"]) {
            continue;
        }
        let Ok(transcript) = entry.read_json::<Transcript>("transcript.json") else {
            continue;
        };
        if transcript.schema_version != TRANSCRIPTION_SCHEMA_VERSION {
            continue;
        }
        let (start, end) = (transcript.range.start, transcript.range.end);
        let Some(samples) = wav_samples(&entry.path("audio.wav"), 0.0, end - start) else {
            continue;
        };
        found.extend(unheard_in(&samples, (start, end), &words));
    }
    // What two transcripts both kept the sound of is said once.
    found.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut merged: Vec<Unheard> = Vec::new();
    for stretch in found {
        match merged.last_mut() {
            Some(last) if stretch.start <= last.end => last.end = last.end.max(stretch.end),
            _ => merged.push(stretch),
        }
    }
    merged
}
