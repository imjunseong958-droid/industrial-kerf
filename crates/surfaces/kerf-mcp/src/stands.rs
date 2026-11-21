//! What each clip in depth stands on, as `look` with `stands` says it.
//!
//! [`kerf_engine::stands`] measures it; this is how it is told: an entry for each clip, or
//! for each run of frames a clip floats, is sunk, or is in the air on, with the line that
//! sets it down where one does. The line is one for the whole of a clip, the keys that keep
//! it down on every frame, so it is said once, with the first run it mends, however many
//! runs a thing that walks has. A clip that rests on every frame read is one entry with
//! what it is on, and the ground is named once and has none.

use kerf_engine::stands::{How, Run, Standing, Stood, LEANS, TOUCH};
use kerf_model::{Project, Time};
use serde_json::{json, Value};

/// A distance, to a thousandth of the frame's height.
fn place(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0 + 0.0
}

/// A share, to a hundredth.
fn share(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// What `look` answers under `stands`.
pub(crate) fn said(standing: &Standing) -> Value {
    let clips: Vec<Value> = standing.clips.iter().flat_map(entries).collect();
    let mut said = json!({"frames": standing.frames.len()});
    if standing.every > 1 {
        said["every"] = json!(standing.every);
    }
    if let Some(ground) = &standing.ground {
        said["ground"] = json!(ground);
    }
    said["clips"] = json!(clips);
    said["notMeasured"] = json!(standing.not_measured);
    said
}

/// The entries of one clip: one where it rests throughout, and else one for each run of
/// frames it does not rest on.
fn entries(stood: &Stood) -> Vec<Value> {
    let runs: Vec<&Run> = stood
        .runs
        .iter()
        .filter(|run| run.how != How::Ground)
        .collect();
    let found: Vec<&Run> = runs
        .iter()
        .copied()
        .filter(|run| run.how != How::Rests)
        .collect();
    // Where it leaves the ground, the line brings it up where it is sunk and no more.
    let leaves = runs.iter().any(|run| run.how == How::Air);
    let told = if found.is_empty() { &runs } else { &found };
    // The line, and why there is none, are the clip's: said with the first run they mend.
    let mut line_said = false;
    let mut entries: Vec<Value> = told
        .iter()
        .map(|run| {
            let mut entry = json!({"clip": stood.clip});
            let frames = json!([run.frames.0, run.frames.1]);
            if let Some(on) = run.on.as_ref().filter(|_| run.how != How::Air) {
                entry["on"] = json!(on);
            }
            let mended = match run.how {
                How::Floats(by) => {
                    entry["floats"] = json!(place(by));
                    !leaves
                }
                How::Sunk { deep, share: of } => {
                    entry["sunk"] = json!(place(deep));
                    entry["share"] = json!(share(of));
                    true
                }
                How::Air => {
                    entry["air"] = json!(true);
                    if let (Some(over), Some(up)) = (&run.on, run.up) {
                        entry["over"] = json!(over);
                        entry["up"] = json!(place(up));
                    }
                    false
                }
                How::Rests | How::Ground => false,
            };
            if run.how != How::Air && run.slope >= LEANS {
                entry["slope"] = json!((run.slope * 10.0).round() / 10.0);
                if run.open > TOUCH {
                    entry["gap"] = json!(place(run.open));
                }
            }
            if matches!(run.how, How::Floats(_) | How::Sunk { .. }) {
                entry["frames"] = frames;
                entry["worst"] = json!(run.worst);
            } else if runs.len() > 1 {
                entry["frames"] = frames;
            }
            if mended && !line_said {
                line_said = true;
                if let Some(script) = &stood.script {
                    entry["script"] = json!(script);
                }
                if let Some(cannot) = &stood.cannot {
                    entry["cannot"] = json!(cannot);
                }
            }
            entry
        })
        .collect();
    if !stood.through.is_empty() {
        let through: Vec<Value> = stood
            .through
            .iter()
            .map(|through| {
                json!({
                    "clip": through.clip,
                    "deep": place(through.deep),
                    "share": share(through.share),
                    "frames": [through.frames.0, through.frames.1],
                    "worst": through.worst,
                })
            })
            .collect();
        // Said once, with the first of what is said of it, or alone for the ground.
        if entries.is_empty() {
            entries.push(json!({"clip": stood.clip}));
        }
        entries[0]["through"] = json!(through);
    }
    entries
}

/// What is found of sequence `sequence` of `project` on the first, the middle, and the last
/// frame a `render` step draws of it, from `from` seconds in and for `seconds` where it
/// says: a note for one about to have it made, or none where nothing floats, is sunk, or
/// goes through another. Three frames of a scene cost under a millisecond to read, where
/// drawing its guide costs seconds and the making a minute and more.
pub(crate) fn note(
    project: &Project,
    sequence: &str,
    (from, seconds): (Option<f64>, Option<f64>),
) -> Option<String> {
    let drawn = project.sequence(sequence)?;
    if !drawn
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .any(kerf_model::Clip::in_depth)
    {
        return None;
    }
    let rate = drawn.rate;
    let frames_in = |seconds: f64| Time::from_seconds(seconds).map(|time| rate.frames_round(time));
    let count = rate.frames_round(drawn.duration());
    if count <= 0 {
        return None;
    }
    let first = from.and_then(frames_in).unwrap_or(0).clamp(0, count - 1);
    let last = seconds
        .and_then(frames_in)
        .map_or(count - 1, |long| first + long - 1)
        .clamp(first, count - 1);
    let mut frames = vec![first, (first + last) / 2, last];
    frames.dedup();
    let times: Vec<Time> = frames.iter().map(|frame| rate.time_of(*frame)).collect();
    let standing = kerf_engine::stands::standing(project, Some(sequence), &times).ok()?;
    let mut found = Vec::new();
    for stood in &standing.clips {
        let leaves = stood.runs.iter().any(|run| run.how == How::Air);
        let worst = |wanted: fn(&How) -> Option<f64>| {
            stood
                .runs
                .iter()
                .filter_map(|run| Some((wanted(&run.how)?, run.on.as_deref()?)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
        };
        // One that leaves the ground floats on its way up and down, which is not said.
        if let Some((by, on)) = worst(|how| match how {
            How::Floats(by) => Some(*by),
            _ => None,
        })
        .filter(|_| !leaves)
        {
            found.push(format!("`{}` floats {} over `{on}`", stood.clip, place(by)));
        }
        if let Some((deep, on)) = worst(|how| match how {
            How::Sunk { deep, .. } => Some(*deep),
            _ => None,
        }) {
            found.push(format!("`{}` is sunk {} in `{on}`", stood.clip, place(deep)));
        }
        let mut through: Vec<&str> = stood.through.iter().map(|through| through.clip.as_str()).collect();
        through.dedup();
        for other in through {
            found.push(format!("`{}` goes through `{other}`", stood.clip));
        }
    }
    if found.is_empty() {
        return None;
    }
    let read = match frames.len() {
        1 => "on its frame",
        2 => "on its first and last frame",
        _ => "on its first, middle, and last frame",
    };
    Some(format!(
        "in sequence `{sequence}`, {read}, {}, in frame heights: `look` with `stands` and `sequence` says on which frames, and gives the line that sets a clip down; what is meant to be off the ground is left as it is",
        found.join(", ")
    ))
}
