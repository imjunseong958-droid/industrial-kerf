//! A signal's keys from what a sound does: a model is told when with a
//! `gen=signal@1` clip (`docs/shots.md`), and where the moments are a sound's,
//! the beats of a track or a ball on a floor, they are read from it and not
//! written by ear.

use kerf_model::{Rate, Time};

/// How many frames a signal holds at 1 from each onset, unless told: long
/// enough to reach a model that makes two frames for a sequence's three
/// whole.
pub(crate) const HOLD: i64 = 3;

/// The keys of a signal that is 1 for `hold` frames from each of `onsets`
/// and 0 between, as `set <clip> value=…` takes them, with how many of the
/// onsets fall in the clip. `onsets` are times on the sequence, and the clip
/// runs from `start` for `duration`; its keys are from its own first frame.
/// Onsets nearer each other than `hold` run together.
pub(crate) fn pulses(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(frames: &[i64]) -> Vec<Time> {
        frames
            .iter()
            .map(|frame| Rate::FPS_24.time_of(*frame))
            .collect()
    }

    #[test]
    fn each_onset_is_a_run_of_one_and_near_ones_run_together() {
        let clip = (Rate::FPS_24.time_of(0), Rate::FPS_24.time_of(96));
        let (keys, placed) = pulses(&at(&[12, 36, 37, 60]), clip, Rate::FPS_24, 3);
        assert_eq!(
            keys,
            "{0f:0:hold 12f:1:hold 15f:0:hold 36f:1:hold 40f:0:hold 60f:1:hold 63f:0:hold}"
        );
        assert_eq!(placed, 4);
        // One on the clip's first frame starts it at 1, and one at its end holds to the end.
        let (keys, placed) = pulses(&at(&[0, 95]), clip, Rate::FPS_24, 3);
        assert_eq!(keys, "{0f:1:hold 3f:0:hold 95f:1:hold}");
        assert_eq!(placed, 2);
    }

    #[test]
    fn keys_are_from_the_clips_own_first_frame_and_only_of_what_falls_in_it() {
        // A clip from frame 24 for 48: an onset before it and one after it are not its.
        let clip = (Rate::FPS_24.time_of(24), Rate::FPS_24.time_of(48));
        let (keys, placed) = pulses(&at(&[10, 30, 80]), clip, Rate::FPS_24, 2);
        assert_eq!(keys, "{0f:0:hold 6f:1:hold 8f:0:hold}");
        assert_eq!(placed, 1);
        // None in it is a signal that is 0 throughout.
        let (keys, placed) = pulses(&at(&[10]), clip, Rate::FPS_24, 2);
        assert_eq!((keys.as_str(), placed), ("{0f:0:hold}", 0));
    }
}
