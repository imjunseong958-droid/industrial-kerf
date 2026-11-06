//! A fact sheet for a piece of media: what can be measured of it with no trained model, kept with its
//! other derivatives so that only the first look pays for it.
//!
//! The index says where the picture cuts and where the sound pauses. This adds what the camera does in
//! each shot (`kerf_track::shot`), and how loud the sound is by R128, and says in as many words what it
//! leaves out, so an agent does not take silence for a finding.

use kerf_engine::derivatives::DerivativeCache;
use kerf_engine::{CancellationToken, MediaIndex};
use kerf_model::Media;
use kerf_track::shot::{motion_over, step_between, Motion, Small, Step};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// How many frames a second the camera is read at.
const RATE: u32 = 4;
/// How wide a frame is read, in pixels.
const WIDTH: u32 = 320;
/// How near to a cut, in seconds, a step may not start or end: frames are read at about these times.
const NEAR_A_CUT: f64 = 0.05;

/// What cannot be measured here, and why, as the sheet says it.
const NOT_MEASURED: [(&str, &str); 4] = [
    (
        "faces",
        "not looked for: finding them takes a trained model, and none is bundled",
    ),
    (
        "subject",
        "not looked for: what a shot is about takes a trained model; give `track` points on it to follow it",
    ),
    (
        "onScreenText",
        "not looked for: without a trained model it cannot be found reliably, and foliage, fences, and fabric read as text",
    ),
    (
        "motionBetweenSubjectAndCamera",
        "a pan is told from a move sideways, and a zoom from a push in, only by `track` with `camera`",
    ),
];

/// What the camera did in one shot.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Shot {
    from: f64,
    to: f64,
    camera: Vec<String>,
    pan: f64,
    tilt: f64,
    zoom: f64,
    roll: f64,
    shake: f64,
    agreeing: f64,
}

/// What is measured and kept.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Measured {
    shots: Vec<Shot>,
    loudness: Option<Sound>,
}

/// How loud the sound is, or nothing for what has none.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Sound {
    /// In LUFS, or `None` for silence.
    integrated: Option<f64>,
    range: f64,
    /// In dBTP, or `None` for silence.
    true_peak: Option<f64>,
}

/// The shots between `cuts`, on a file `length` seconds long, and which of the steps between frames
/// `seconds` apart belong to each.
fn shots_of(steps: &[(f64, Option<Step>)], cuts: &[f64], length: f64, seconds: f64) -> Vec<Shot> {
    let mut edges = vec![0.0];
    edges.extend(
        cuts.iter()
            .copied()
            .filter(|cut| *cut > 0.0 && *cut < length),
    );
    edges.push(length);
    edges
        .windows(2)
        .map(|edge| {
            let (from, to) = (edge[0], edge[1]);
            let within: Vec<Option<Step>> = steps
                .iter()
                .filter(|(at, _)| *at >= from - NEAR_A_CUT && *at + seconds <= to + NEAR_A_CUT)
                .filter(|(at, _)| {
                    !cuts
                        .iter()
                        .any(|cut| *at < *cut + NEAR_A_CUT && *at + seconds > *cut - NEAR_A_CUT)
                })
                .map(|(_, step)| *step)
                .collect();
            let Motion {
                camera,
                pan,
                tilt,
                zoom,
                roll,
                shake,
                agreeing,
            } = motion_over(&within, seconds);
            let place = |value: f64| (value * 10_000.0).round() / 10_000.0;
            Shot {
                from,
                to,
                camera: camera.into_iter().map(str::to_owned).collect(),
                pan: place(pan),
                tilt: place(tilt),
                zoom: place(zoom),
                roll: place(roll),
                shake: place(shake),
                agreeing: place(agreeing),
            }
        })
        .collect()
}

/// Reads what the sheet keeps.
fn measure(src: &Path, media: &Media, index: &MediaIndex) -> Result<Measured, String> {
    let mut measured = Measured::default();
    if let Some(video) = &media.video {
        let height = ((f64::from(WIDTH) * f64::from(video.height) / f64::from(video.width.max(1)))
            .round() as u32
            / 2
            * 2)
        .max(2);
        let seconds = 1.0 / f64::from(RATE);
        let mut before: Option<Small> = None;
        let mut steps = Vec::new();
        kerf_engine::small_grey_frames(src, RATE, (WIDTH, height), |at, grey| {
            let now = Small::of(WIDTH, height, grey);
            if let Some(before) = &before {
                steps.push((at - seconds, step_between(before, &now)));
            }
            before = Some(now);
        })
        .map_err(|error| error.to_string())?;
        measured.shots = shots_of(&steps, &index.cuts, media.duration.seconds(), seconds);
    }
    if media.audio.is_some() {
        let loudness = kerf_engine::measure_file(src, &CancellationToken::new())
            .map_err(|error| error.to_string())?;
        let level = |value: f64| value.is_finite().then_some((value * 10.0).round() / 10.0);
        measured.loudness = Some(Sound {
            integrated: level(loudness.integrated),
            range: (loudness.lra * 10.0).round() / 10.0,
            true_peak: level(loudness.true_peak_max),
        });
    }
    Ok(measured)
}

/// What is measured of `media` at `src`, from the cache or found now.
fn measured(
    src: &Path,
    media: &Media,
    index: &MediaIndex,
    cache: &DerivativeCache,
) -> Result<Measured, String> {
    let entry = cache
        .source_entry(
            src,
            "media-facts",
            &json!({"version": 1, "rate": RATE, "width": WIDTH}),
        )
        .map_err(|error| error.to_string())?;
    if entry.is_complete(&["facts.json"]) {
        if let Ok(kept) = entry.read_json("facts.json") {
            return Ok(kept);
        }
    }
    let found = measure(src, media, index)?;
    entry
        .write_json("facts.json", &found)
        .and_then(|()| entry.mark_complete(&["facts.json"]))
        .map_err(|error| error.to_string())?;
    Ok(found)
}

/// The fact sheet of `media` at `src`, with `at` printing a time in it.
pub(crate) fn sheet(
    src: &Path,
    media: &Media,
    index: &MediaIndex,
    cache: &DerivativeCache,
    at: &dyn Fn(f64) -> String,
) -> Value {
    let not_measured: serde_json::Map<String, Value> = NOT_MEASURED
        .iter()
        .map(|(what, why)| ((*what).to_owned(), json!(why)))
        .collect();
    let mut sheet = json!({
        "seconds": media.duration.seconds(),
        "notMeasured": not_measured,
    });
    if let Some(video) = &media.video {
        sheet["picture"] =
            json!({"width": video.width, "height": video.height, "rate": media.rate.to_string()});
    }
    if let Some(audio) = &media.audio {
        sheet["sound"] = json!({"channels": audio.channels, "rate": audio.sample_rate});
    }
    sheet["cuts"] = json!(index.cuts.len());
    sheet["pauses"] = json!({
        "count": index.pauses.len(),
        "seconds": (index.pauses.iter().map(|(from, to)| to - from).sum::<f64>() * 100.0).round() / 100.0 + 0.0,
    });
    match measured(src, media, index, cache) {
        Ok(measured) => {
            if media.video.is_some() {
                sheet["shots"] = json!(measured
                    .shots
                    .iter()
                    .map(|shot| json!({
                        "from": at(shot.from),
                        "to": at(shot.to),
                        "camera": shot.camera,
                        "pan": shot.pan,
                        "tilt": shot.tilt,
                        "zoom": shot.zoom,
                        "roll": shot.roll,
                        "shake": shot.shake,
                        "agreeing": shot.agreeing,
                    }))
                    .collect::<Vec<_>>());
            }
            if let Some(sound) = measured.loudness {
                sheet["loudness"] = json!({
                    "integrated": sound.integrated,
                    "range": sound.range,
                    "truePeak": sound.true_peak,
                });
            }
        }
        Err(why) => sheet["unmeasured"] = json!(why),
    }
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panning(at: f64, across: f64) -> (f64, Option<Step>) {
        (
            at,
            Some(Step {
                across,
                agreeing: 1.0,
                ..Step::default()
            }),
        )
    }

    #[test]
    fn a_step_across_a_cut_belongs_to_no_shot() {
        // Half a second of one thing and half a second of another, with a wild step across the cut.
        let steps = [
            panning(0.0, -0.02),
            panning(0.25, -0.02),
            panning(0.5, 0.9),
            panning(0.75, 0.02),
            panning(1.0, 0.02),
        ];
        let shots = shots_of(&steps, &[0.625], 1.25, 0.25);
        assert_eq!(shots.len(), 2, "{shots:?}");
        assert_eq!(shots[0].camera, ["pan right"], "{shots:?}");
        assert_eq!(shots[1].camera, ["pan left"], "{shots:?}");
        assert!((shots[0].pan - 0.08).abs() < 1e-6, "{shots:?}");
        assert!((shots[1].pan + 0.08).abs() < 1e-6, "{shots:?}");
    }

    #[test]
    fn no_cuts_is_one_shot_and_nothing_read_says_so() {
        let shots = shots_of(&[(0.0, None), (0.25, None)], &[], 0.5, 0.25);
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].camera, ["unknown"]);
        assert_eq!((shots[0].from, shots[0].to), (0.0, 0.5));
    }
}
