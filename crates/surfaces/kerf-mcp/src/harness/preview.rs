//! What an edit's preview shows: the span of the sequence it changed, and contact sheets of
//! the frames that show what it changed there, before and after.

use super::inline_image;
use crate::{KerfMcpRouter, McpError};
use kerf_lang::Clock;
use kerf_model::{Catalog, Clip, Content, Project, Sequence, Time};
use serde_json::{json, Value};

/// A preview shows the frame that shows best each picture generator an edit
/// added or changed, then, as many as fit, the frames either side of the
/// first cuts it made or moved, two cuts to a row.
const PREVIEW_FRAMES: usize = 8;
/// The most frames of a generator read for the one that shows it best: a
/// longer one is read at frames spread through it.
const MOST_SHOWN_READS: i64 = 600;
const PREVIEW_COLUMNS: usize = 4;
const PREVIEW_WIDTH: u32 = 480;

impl KerfMcpRouter {
    /// Contact sheets of what the edit changed in `from`–`to`, before and
    /// after: the frames [`preview_times`] picks on each side. `before` and
    /// `after` have their media paths resolved.
    pub(super) fn preview(
        &self,
        sequence: &Sequence,
        from: Time,
        to: Time,
        before: &Project,
        after: &Project,
    ) -> Result<Value, McpError> {
        let clock = Clock::of_sequence(sequence);
        let mut result = json!({
            "sequence": sequence.id.to_string(),
            "from": clock.print_position(from),
            "to": clock.print_position(to),
        });
        for (side, project, other) in [("before", before, after), ("after", after, before)] {
            let times = preview_times(project, other, sequence.id.as_str());
            if times.is_empty() {
                result[side] = json!({"empty": true});
                continue;
            }
            let capture = self.capture_frames(
                project,
                sequence.id.as_str(),
                &times,
                PREVIEW_WIDTH,
                Some(PREVIEW_COLUMNS),
            )?;
            let mut sheet = json!({
                "frames": times.iter().map(|time| clock.print_position(*time)).collect::<Vec<_>>(),
                "path": capture.sheet,
            });
            inline_image(&mut sheet)?;
            result[side] = sheet;
        }
        Ok(result)
    }
}

/// The first sequence that differs between two projects, and the span of
/// it that differs: every clip added, removed, or changed, and the
/// transitions around them. A change to a track itself (enabled, muted)
/// spans the whole sequence.
pub(super) fn changed_range(before: &Project, after: &Project) -> Option<(Sequence, Time, Time)> {
    after.sequences.iter().find_map(|sequence| {
        let (from, to) = changed_span(sequence, before.sequence(sequence.id.as_str()))?;
        Some((sequence.clone(), from, to))
    })
}

/// The span of `sequence` that differs from `old`, what it was.
fn changed_span(sequence: &Sequence, old: Option<&Sequence>) -> Option<(Time, Time)> {
    let mut span: Option<(Time, Time)> = None;
    let mut extend = |from: Time, to: Time| {
        span = Some(match span {
            None => (from, to),
            Some((start, end)) => (start.min(from), end.max(to)),
        });
    };
    let whole = sequence
        .duration()
        .max(old.map_or(Time::ZERO, Sequence::duration));
    for track in &sequence.tracks {
        let Some(previous) = old.and_then(|old| old.track(track.id.as_str())) else {
            extend(Time::ZERO, whole);
            continue;
        };
        if (track.enabled, track.muted, track.kind)
            != (previous.enabled, previous.muted, previous.kind)
        {
            extend(Time::ZERO, whole);
        }
        for clip in &track.clips {
            match previous.clip(clip.id.as_str()) {
                Some(was) if was == clip => {}
                Some(was) => {
                    extend(was.start, was.end());
                    extend(clip.start, clip.end());
                }
                None => extend(clip.start, clip.end()),
            }
        }
        for was in &previous.clips {
            if track.clip(was.id.as_str()).is_none() {
                extend(was.start, was.end());
            }
        }
        let transitions = track.transitions.iter().chain(&previous.transitions);
        for transition in transitions {
            let same = track.transition(transition.id.as_str())
                == previous.transition(transition.id.as_str());
            if same {
                continue;
            }
            for clip in [&transition.from, &transition.to] {
                if let Some(clip) = track.clip(clip.as_str()).or(previous.clip(clip.as_str())) {
                    extend(clip.start, clip.end());
                }
            }
        }
    }
    if let Some(old) = old {
        for track in &old.tracks {
            if sequence.track(track.id.as_str()).is_none() {
                extend(Time::ZERO, whole);
            }
        }
    }
    let (from, to) = span?;
    (to > from).then_some((from, to))
}

/// Whether something of `clip` changes as it plays: a property, an effect's
/// parameter or its generator's keyed or worked out from its time.
fn moves(clip: &Clip) -> bool {
    let animated = |params: &kerf_model::Params| {
        params
            .values()
            .any(|param| !matches!(param, kerf_model::Param::Static(_)))
    };
    animated(&clip.props)
        || clip.effects.iter().any(|effect| animated(&effect.params))
        || matches!(&clip.content, Content::Generator { params, .. } if animated(params))
}

/// Whether `clip` is a generator that draws a picture of its own: a title,
/// a shape, particles, a solid, a mesh. A camera, a light, a null, a signal
/// and an adjustment show nothing of themselves.
fn draws(clip: &Clip) -> bool {
    matches!(
        &clip.content,
        Content::Generator { fx, .. }
            if !matches!(
                fx.as_str(),
                "adjustment" | "camera" | "light" | "null" | kerf_model::SIGNAL
            )
    )
}

/// How much of the generator `clip` shows at `at`, clip-relative, from 0 to
/// 1: its opacity, how far a title has revealed itself and a shape drawn its
/// stroke on, and, out of depth, how much of the frame its picture covers by
/// its scale, crop and place. Growing past the frame shows no more of it, so
/// a pop's overshoot shows it as much as where it settles.
fn shows(clip: &Clip, at: Time) -> f64 {
    let Content::Generator { fx, params, .. } = &clip.content else {
        return 1.0;
    };
    let read = |params: &kerf_model::Params, name: &str, default: f64| {
        params
            .get(name)
            .and_then(|param| param.value_at(at).as_number())
            .unwrap_or(default)
    };
    let prop = |name: &str, default: f64| read(&clip.props, name, default);
    let param = |name: &str, default: f64| read(params, name, default);
    let unit = |value: f64| value.clamp(0.0, 1.0);
    let mut shows = unit(prop("opacity", 1.0));
    match fx.as_str() {
        "title" => shows *= unit(param("reveal", 1.0)),
        "shape" if param("stroke_width", 0.0) > 0.0 => {
            shows *= unit(param("trim_end", 1.0) - param("trim_start", 0.0));
        }
        _ => {}
    }
    if clip.in_depth() {
        return shows;
    }
    // A generator's picture is the frame's: how much of the frame it covers
    // across and down, in frame widths and heights about its middle.
    let covers = |scale: f64, (low, high): (f64, f64), place: f64| {
        let (low, high) = (unit(low), unit(high));
        if low + high >= 1.0 {
            return 0.0;
        }
        let scale = scale.abs();
        let (from, to) = (place + scale * (low - 0.5), place + scale * (0.5 - high));
        (to.min(0.5) - from.max(-0.5)).max(0.0)
    };
    let scale = prop("scale", 1.0);
    let crop = |low: &str, high: &str| (prop(low, 0.0), prop(high, 0.0));
    shows
        * covers(
            scale * prop("scale_x", 1.0),
            crop("crop_l", "crop_r"),
            prop("x", 0.0),
        )
        * covers(
            scale * prop("scale_y", 1.0),
            crop("crop_t", "crop_b"),
            prop("y", 0.0),
        )
}

/// The frame of the picture generator `clip` that shows it most, by
/// [`shows`]: of those that show it as much, the nearest its middle, so a
/// still one is seen at its middle and one that reveals and fades out where
/// it has come in.
fn shown_most(clip: &Clip, rate: kerf_model::Rate) -> Time {
    let (first, last) = (rate.frames_floor(clip.start), rate.frames_floor(clip.end()));
    let middle = first + (last - first) / 2;
    let step = ((last - first) / MOST_SHOWN_READS + 1) as usize;
    let read: Vec<(i64, f64)> = (first..last)
        .step_by(step)
        .chain([middle])
        .map(|frame| (frame, shows(clip, rate.time_of(frame) - clip.start)))
        .collect();
    let most = read.iter().map(|(_, shows)| *shows).fold(0.0, f64::max);
    let frame = read
        .into_iter()
        .filter(|(_, shows)| *shows >= most - 1e-3)
        .min_by_key(|(frame, _)| (frame - middle).abs())
        .map_or(middle, |(frame, _)| frame);
    rate.time_of(frame)
}

/// The positions in `sequence` that show what an edit changed on `side`, the
/// project before or after it, compared with `other`: the last frame
/// before and the first frame of every cut it made or moved; the middle
/// frame of a clip it changed in place, with its first and last when the
/// clip moves; a quarter, half and three quarters of the way through a
/// transition it added or changed; and, in place of its middle, the frame
/// that shows most of a picture generator it added or changed.
/// [`PREVIEW_FRAMES`] of them at most, in order: the generators' frames, as a
/// title's first and last frame may show none of it, and the earliest of the
/// rest.
fn preview_times(side: &Project, other: &Project, sequence: &str) -> Vec<Time> {
    let Some(sequence) = side.sequence(sequence) else {
        return Vec::new();
    };
    let old = other.sequence(sequence.id.as_str());
    let rate = sequence.rate;
    // How much a generator shows is of what is drawn: a look as the keys it
    // stands for. One that does not expand is read as it is.
    let drawn = kerf_model::look::expand(side, Catalog::builtin()).ok();
    let drawn = drawn
        .as_ref()
        .and_then(|drawn| drawn.sequence(sequence.id.as_str()));
    let mut times = Vec::new();
    let mut cut = |at: Time| times.extend([at - rate.frame_duration(), at]);
    let mut middles = Vec::new();
    let mut most = Vec::new();
    for track in &sequence.tracks {
        let previous = old.and_then(|old| old.track(track.id.as_str()));
        let drawn = drawn.and_then(|drawn| drawn.track(track.id.as_str()));
        let mut show_most = |clip: &Clip| {
            let drawn = drawn.and_then(|drawn| drawn.clip(clip.id.as_str()));
            most.push(shown_most(drawn.unwrap_or(clip), rate));
        };
        for clip in &track.clips {
            match previous.and_then(|previous| previous.clip(clip.id.as_str())) {
                Some(was) if was == clip => {}
                Some(was) if (was.start, was.end()) == (clip.start, clip.end()) => {
                    let (first, last) =
                        (rate.frames_floor(clip.start), rate.frames_floor(clip.end()));
                    if draws(clip) {
                        show_most(clip);
                    } else {
                        middles.push(rate.time_of(first + (last - first) / 2));
                    }
                    // One frame says nothing of a move: where it starts and
                    // where it ends, either side of the middle.
                    if moves(clip) || moves(was) {
                        middles.extend([rate.time_of(first), rate.time_of(last - 1)]);
                    }
                }
                _ => {
                    cut(clip.start);
                    cut(clip.end());
                    if draws(clip) {
                        show_most(clip);
                    }
                }
            }
        }
        let Some(previous) = previous else {
            continue;
        };
        for was in &previous.clips {
            if track.clip(was.id.as_str()).is_none() {
                cut(was.start);
            }
        }
        for transition in &track.transitions {
            if previous.transition(transition.id.as_str()) != Some(transition) {
                if let Some(clip) = track.clip(transition.from.as_str()) {
                    // The two frames at its cut are both its middle: three
                    // through it show what it does.
                    let (before, _) = transition.split_at_cut(rate);
                    let first = rate.frames_floor(clip.end() - before);
                    let frames = rate.frames_floor(transition.duration).max(1);
                    middles.extend(
                        [1, 2, 3].map(|quarter| rate.time_of(first + frames * quarter / 4)),
                    );
                }
            }
        }
    }
    times.extend(middles);
    let duration = sequence.duration();
    let shown = |mut times: Vec<Time>| {
        times.retain(|time| !time.is_negative() && *time < duration);
        times.sort();
        times.dedup();
        times
    };
    let mut most = shown(most);
    most.truncate(PREVIEW_FRAMES);
    let room = PREVIEW_FRAMES - most.len();
    let rest: Vec<Time> = shown(times)
        .into_iter()
        .filter(|time| !most.contains(time))
        .take(room)
        .collect();
    most.extend(rest);
    most.sort();
    most
}
