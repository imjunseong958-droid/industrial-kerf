//! What each clip in depth stands on, as `look` with `stands` says it.
//!
//! [`kerf_engine::stands`] measures it; this is how it is told: an entry for each clip, or
//! for each run of frames a clip floats, is sunk, or is in the air on, with the line that
//! sets it down where one does. A clip that rests on every frame read is one entry with what
//! it is on, and the ground is named once and has none.

use kerf_engine::stands::{How, Run, Standing, Stood, LEANS, TOUCH};
use kerf_model::{Content, Param, Project, Time};
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
            if mended {
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
    // As a pass sees it: a clip that says it is in depth, or a mesh, which is whether or not.
    let in_depth = |clip: &kerf_model::Clip| {
        matches!(clip.props.get("three_d"), Some(Param::Static(kerf_model::Value::Bool(true))))
            || matches!(&clip.content, Content::Generator { fx, .. } if fx.as_str() == "mesh")
    };
    if !drawn.tracks.iter().flat_map(|track| &track.clips).any(in_depth) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_engine::stands::Through;

    fn run(how: How, on: Option<&str>, frames: (i64, i64)) -> Run {
        Run {
            how,
            on: on.map(str::to_owned),
            slope: 0.0,
            open: 0.0,
            up: None,
            frames,
            worst: frames.0,
        }
    }

    fn stood(clip: &str, runs: Vec<Run>) -> Stood {
        Stood {
            clip: clip.to_owned(),
            runs,
            through: Vec::new(),
            script: None,
            cannot: None,
        }
    }

    #[test]
    fn a_clip_is_one_entry_where_it_rests_and_one_for_each_run_it_does_not() {
        let mut car = stood("car", vec![run(How::Rests, Some("road"), (0, 64))]);
        (car.runs[0].slope, car.runs[0].open) = (6.0004, 0.0421);
        let mut crate_ = stood(
            "crate",
            vec![
                run(How::Rests, Some("road"), (0, 9)),
                run(How::Floats(0.0312), Some("road"), (10, 30)),
                run(How::Rests, Some("road"), (31, 64)),
            ],
        );
        crate_.runs[1].worst = 12;
        crate_.script = Some("set crate y=0.412".to_owned());
        let mut post = stood(
            "post",
            vec![run(How::Sunk { deep: 0.0204, share: 0.6 }, Some("road"), (0, 64))],
        );
        post.cannot = Some("a parent turns it".to_owned());
        // A ball thrown: in the air, down, and into the road. Its line brings it up out of
        // the road, and is not said of the frames it is on its way down.
        let mut ball = stood(
            "ball",
            vec![
                run(How::Air, Some("road"), (0, 40)),
                run(How::Floats(0.05), Some("road"), (41, 50)),
                run(How::Sunk { deep: 0.01, share: 1.0 }, Some("road"), (51, 64)),
            ],
        );
        ball.runs[0].up = Some(0.4004);
        ball.script = Some("set ball y={0f:0 50f:0.9 64f:0.9}".to_owned());
        let mut sign = stood("sign", vec![run(How::Rests, Some("road"), (0, 64))]);
        sign.through = vec![Through {
            clip: "wall".to_owned(),
            deep: 0.0496,
            share: 0.3,
            frames: (0, 64),
            worst: 3,
        }];
        let mut road = stood("road", vec![run(How::Ground, None, (0, 64))]);
        let said = said(&Standing {
            frames: (0..65).collect(),
            every: 1,
            ground: Some("road".to_owned()),
            clips: vec![road.clone(), car, crate_, post, ball, sign],
            not_measured: vec!["a mesh into another mesh".to_owned()],
        });
        assert_eq!(
            said,
            json!({
                "frames": 65,
                "ground": "road",
                "clips": [
                    {"clip": "car", "on": "road", "slope": 6.0, "gap": 0.042},
                    {"clip": "crate", "on": "road", "floats": 0.031, "frames": [10, 30], "worst": 12, "script": "set crate y=0.412"},
                    {"clip": "post", "on": "road", "sunk": 0.02, "share": 0.6, "frames": [0, 64], "worst": 0, "cannot": "a parent turns it"},
                    {"clip": "ball", "air": true, "over": "road", "up": 0.4, "frames": [0, 40]},
                    {"clip": "ball", "on": "road", "floats": 0.05, "frames": [41, 50], "worst": 41},
                    {"clip": "ball", "on": "road", "sunk": 0.01, "share": 1.0, "frames": [51, 64], "worst": 51, "script": "set ball y={0f:0 50f:0.9 64f:0.9}"},
                    {"clip": "sign", "on": "road", "through": [{"clip": "wall", "deep": 0.05, "share": 0.3, "frames": [0, 64], "worst": 3}]},
                ],
                "notMeasured": ["a mesh into another mesh"],
            })
        );
        // The ground has an entry only for what it goes through, and a frame in every few
        // read is said.
        road.through = vec![Through {
            clip: "pier".to_owned(),
            deep: 0.2,
            share: 0.5,
            frames: (0, 8),
            worst: 4,
        }];
        let said = super::said(&Standing {
            frames: vec![0, 4, 8],
            every: 4,
            ground: Some("road".to_owned()),
            clips: vec![road],
            not_measured: Vec::new(),
        });
        assert_eq!(
            said,
            json!({
                "frames": 3,
                "every": 4,
                "ground": "road",
                "clips": [{"clip": "road", "through": [{"clip": "pier", "deep": 0.2, "share": 0.5, "frames": [0, 8], "worst": 4}]}],
                "notMeasured": [],
            })
        );
    }
}
