//! A signal's keys from what a sound does: a model is told when with a
//! `gen=signal@1` clip, and where the moments are a sound's,
//! the beats of a track or a ball on a floor, they are read from it and not
//! written by ear.
//!
//! An agent reads them with `look` (`signal`) and a person asks for them in
//! the signal's card of a shot's graph, from the one reading here.

use kerf_engine::derivatives::DerivativeCache;
use kerf_model::{Content, Project, Rate, Time};
use std::path::Path;

/// How many frames a signal holds at 1 from each onset, unless told: long
/// enough to reach a model that makes two frames for a sequence's three
/// whole.
pub const HOLD: i64 = 3;

/// The keys of a signal that is 1 for `hold` frames from each of `onsets`
/// and 0 between, as `set <clip> value=…` takes them, with how many of the
/// onsets fall in the clip. `onsets` are times on the sequence, and the clip
/// runs from `start` for `duration`; its keys are from its own first frame.
/// Onsets nearer each other than `hold` run together.
pub fn pulses(
    onsets: &[Time],
    (start, duration): (Time, Time),
    rate: Rate,
    hold: i64,
) -> (String, usize) {
    let length = rate.frames_round(duration);
    let mut frames: Vec<i64> = onsets
        .iter()
        .map(|onset| rate.frames_round(*onset - start))
        .filter(|frame| (0..length).contains(frame))
        .collect();
    frames.sort_unstable();
    frames.dedup();
    // Each run of 1: from a frame, to the frame it is 0 again on.
    let mut runs: Vec<(i64, i64)> = Vec::new();
    for frame in &frames {
        match runs.last_mut() {
            Some((_, end)) if *frame <= *end => *end = (*frame + hold).max(*end),
            _ => runs.push((*frame, *frame + hold)),
        }
    }
    let mut keys = Vec::new();
    if runs.first().is_none_or(|(from, _)| *from > 0) {
        keys.push("0f:0:hold".to_owned());
    }
    for (from, to) in runs {
        keys.push(format!("{from}f:1:hold"));
        if to < length {
            keys.push(format!("{to}f:0:hold"));
        }
    }
    (format!("{{{}}}", keys.join(" ")), frames.len())
}

/// A signal's keys read from a sound: what an edit sets its clip's value to, and how many
/// of the sound's onsets it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromSound {
    /// The keys, as `set <clip> value=…` takes them.
    pub keys: String,
    /// How many frames each onset is held at 1.
    pub hold: i64,
    /// The onsets that fall in the signal's clip, and those the sound plays outside it.
    pub placed: usize,
    pub outside: usize,
}

impl FromSound {
    /// The line that, in an edit, has the signal clip `clip` be it.
    pub fn script(&self, clip: &str) -> String {
        format!("set {clip} {}={}", kerf_model::SIGNAL_VALUE, self.keys)
    }
}

/// The keys of signal clip `clip` from the onsets of the sound clip `of` plays, each held at
/// 1 for `hold` frames ([`HOLD`] unless said): both clips of one sequence of `project`, kept
/// in `directory`. What is wrong is said: a clip that is no signal, one that plays no
/// media, a sound with no onsets, none of them where the signal's clip is.
pub fn from_sound(
    project: &Project,
    directory: &Path,
    of: &str,
    clip: &str,
    hold: Option<i64>,
) -> Result<FromSound, String> {
    let located = |id: &str| project.locate(id).ok_or_else(|| project.no_clip(id));
    let (of_at, clip_at) = (located(of)?, located(clip)?);
    let (sound, signal) = (project.clip_at(of_at), project.clip_at(clip_at));
    if !matches!(&signal.content, Content::Generator { fx, .. } if fx.as_str() == kerf_model::SIGNAL)
    {
        return Err(format!(
            "`{clip}` is not a signal: a signal is a `gen={}@1` clip",
            kerf_model::SIGNAL
        ));
    }
    if of_at.sequence != clip_at.sequence {
        return Err(format!(
            "`{of}` and `{clip}` are in different sequences: a signal's time is its own sequence's"
        ));
    }
    let media = match &sound.content {
        Content::Media { media, .. } => project.media(media.as_str()),
        _ => None,
    }
    .ok_or_else(|| {
        format!("`{of}` plays no media: `of` is a clip of music or of footage with sound")
    })?;
    let hold = hold.unwrap_or(HOLD);
    if !(1..=240).contains(&hold) {
        return Err(format!("`hold` is frames, from 1 to 240, not {hold}"));
    }
    // A path the project's folder is the start of, as a document writes one.
    let file = if Path::new(&media.path).is_absolute() {
        Path::new(&media.path).to_path_buf()
    } else {
        directory.join(&media.path)
    };
    let index = kerf_engine::media_index(
        &file,
        &DerivativeCache::new(directory),
        &kerf_engine::CancellationToken::new(),
    )
    .map_err(|error| error.to_string())?;
    // The onsets the clip plays, where on the sequence it plays them.
    let heard: Vec<Time> = index
        .onsets
        .iter()
        .filter_map(|seconds| Time::from_seconds(*seconds))
        .filter_map(|source| sound.played_at(source))
        .filter(|at| sound.start <= *at && *at < sound.end())
        .collect();
    if heard.is_empty() {
        return Err(format!(
            "`{of}` plays no sound onsets of `{}`: nothing to place a signal on",
            media.id
        ));
    }
    let rate = project.sequences[clip_at.sequence].rate;
    let (keys, placed) = pulses(&heard, (signal.start, signal.duration), rate, hold);
    if placed == 0 {
        return Err(format!(
            "none of the {} onsets `{of}` plays fall in `{clip}`: have the signal clip run where the sound does",
            heard.len()
        ));
    }
    Ok(FromSound {
        keys,
        hold,
        placed,
        outside: heard.len() - placed,
    })
}
