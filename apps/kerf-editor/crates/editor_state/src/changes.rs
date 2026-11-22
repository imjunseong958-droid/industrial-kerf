//! What a proposal changes on the program, clip by clip, and the change
//! reel that shows it.

use crate::model::{self, ClipShow, Timeline, TrackShow};
use std::collections::HashSet;

/// A clip a proposal changes, from its timeline against the program's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipChange {
    /// The clip's id, or the transition's.
    pub clip: String,
    /// A transition between clips, not a clip.
    pub transition: bool,
    /// What the clip is called.
    pub name: String,
    /// The track it ends up on (or was on, removed), as it is called.
    pub track: String,
    pub added: bool,
    pub removed: bool,
    /// Its start changed.
    pub moved: bool,
    /// Its length changed.
    pub trimmed: bool,
    pub effects: bool,
    /// Its text, generator or source in changed.
    pub properties: bool,
    pub retracked: bool,
    pub toggled: bool,
    pub retimed: bool,
    pub relinked: bool,
    pub old_start: f64,
    pub old_end: f64,
    pub new_start: f64,
    pub new_end: f64,
}

impl ClipChange {
    /// Where on the program it lies, before and after.
    pub fn span(&self) -> (f64, f64) {
        if self.added {
            (self.new_start, self.new_end)
        } else if self.removed {
            (self.old_start, self.old_end)
        } else {
            (
                self.old_start.min(self.new_start),
                self.old_end.max(self.new_end),
            )
        }
    }

    /// Its row on a card: the clip and its track, where it goes, and by
    /// how much ("Logo · V2", "→ 00:00:26:12", "+17f").
    pub fn row(&self, fps: f64) -> (String, String, String) {
        let name = if self.transition {
            "Transition"
        } else {
            &self.name
        };
        let name = if self.track.is_empty() {
            name.to_owned()
        } else {
            format!("{name} · {}", self.track)
        };
        let frames = |seconds: f64| (seconds * fps).round() as i64;
        let (target, shift) = if self.removed {
            ("removed".to_owned(), String::new())
        } else if self.added {
            (
                format!("→ {}", model::timecode(self.new_start, fps)),
                "new".to_owned(),
            )
        } else if self.moved {
            (
                format!("→ {}", model::timecode(self.new_start, fps)),
                signed_frames(frames(self.new_start - self.old_start)),
            )
        } else if self.trimmed {
            (
                format!("→ {}", model::timecode(self.new_end, fps)),
                signed_frames(frames(
                    (self.new_end - self.new_start) - (self.old_end - self.old_start),
                )),
            )
        } else {
            let what: Vec<&str> = [
                (self.effects, "effects"),
                (self.properties, "properties"),
                (self.toggled, "enabled"),
                (self.retimed, "speed"),
                (self.relinked, "link"),
                (self.retracked, "track"),
            ]
            .into_iter()
            .filter_map(|(changed, word)| changed.then_some(word))
            .collect();
            (what.join(", "), String::new())
        };
        (name, target, shift)
    }
}

/// One stretch of a change reel: where it plays on the program as it is
/// (`before`) and on what the proposal makes (`after`), and what it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segment {
    pub before: f64,
    pub before_end: f64,
    pub after: f64,
    pub after_end: f64,
    pub title: String,
}

impl Segment {
    pub fn length(&self) -> f64 {
        self.after_end - self.after
    }
}

/// What a change does, in words: "Trim sea", "Add title", "Transition".
pub fn change_title(change: &ClipChange) -> String {
    let name = if change.name.is_empty() {
        &change.clip
    } else {
        &change.name
    };
    if change.transition {
        "Transition".to_owned()
    } else if change.added {
        format!("Add {name}")
    } else if change.removed {
        format!("Remove {name}")
    } else if change.trimmed {
        format!("Trim {name}")
    } else if change.moved || change.retracked {
        format!("Move {name}")
    } else if change.effects {
        format!("Effects on {name}")
    } else {
        format!("Change {name}")
    }
}

/// The change reel of `changes`: each change with `handle` seconds either
/// side, in order on the proposal, those that touch merged ("Trim sea and
/// 2 more"), each side's ends kept to its length (`before_length` the
/// program's, `after_length` the proposal's). What a clamp leaves nothing
/// of goes.
pub fn reel_plan(
    changes: &[ClipChange],
    before_length: f64,
    after_length: f64,
    handle: f64,
) -> Vec<Segment> {
    let mut raw: Vec<Segment> = changes
        .iter()
        .map(|change| Segment {
            // What is gone has no place on the proposal but where it was,
            // and what is new none on the program but where it lands.
            after: if change.removed {
                change.old_start
            } else {
                change.new_start
            },
            after_end: if change.removed {
                change.old_start
            } else {
                change.new_end
            },
            before: if change.added {
                change.new_start
            } else {
                change.old_start
            },
            before_end: if change.added {
                change.new_start
            } else {
                change.old_end
            },
            title: change_title(change),
        })
        .collect();
    raw.sort_by(|a, b| a.after.total_cmp(&b.after));
    let mut out: Vec<Segment> = Vec::new();
    let mut merged = 0;
    let close = |out: &mut Vec<Segment>, merged: &mut usize| {
        if let (Some(last), true) = (out.last_mut(), *merged > 0) {
            last.title = format!("{} and {} more", last.title, merged);
        }
        *merged = 0;
    };
    for mut segment in raw {
        segment.after = (segment.after - handle).max(0.0);
        segment.after_end = (segment.after_end + handle).min(after_length.max(0.0));
        segment.before = (segment.before - handle).max(0.0);
        segment.before_end = (segment.before_end + handle).min(before_length.max(0.0));
        if let Some(last) = out
            .last_mut()
            .filter(|last| segment.after <= last.after_end)
        {
            last.after_end = last.after_end.max(segment.after_end);
            last.before = last.before.min(segment.before);
            last.before_end = last.before_end.max(segment.before_end);
            merged += 1;
            continue;
        }
        close(&mut out, &mut merged);
        out.push(segment);
    }
    close(&mut out, &mut merged);
    out.retain(|segment| segment.length() > 0.0);
    out
}

/// How long a reel takes to watch, in seconds.
pub fn reel_total(segments: &[Segment]) -> f64 {
    segments.iter().map(Segment::length).sum()
}

/// Frames as a shift: "+17f", "−3f", "0f".
pub fn signed_frames(frames: i64) -> String {
    match frames {
        0 => "0f".to_owned(),
        frames if frames < 0 => format!("−{}f", -frames),
        frames => format!("+{frames}f"),
    }
}

/// The clips `proposed` changes against `program`, by clip id: those it
/// adds, in its order, those it changes, then those it removes.
pub fn clip_changes(program: &Timeline, proposed: &Timeline) -> Vec<ClipChange> {
    const NEAR: f64 = 1e-4;
    type Placed<'a> = (&'a model::Track, &'a model::Clip);
    fn placed(timeline: &Timeline) -> Vec<Placed<'_>> {
        timeline
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
            .collect()
    }
    let before = placed(program);
    let after = placed(proposed);
    let mut changes = Vec::new();
    for (track, clip) in &after {
        let mut change = ClipChange {
            clip: clip.id.clone(),
            name: clip.label().to_owned(),
            track: track.label().to_owned(),
            new_start: clip.timeline_start,
            new_end: clip.end(),
            ..ClipChange::default()
        };
        let Some((was_track, was)) = before.iter().find(|(_, was)| was.id == clip.id) else {
            change.added = true;
            changes.push(change);
            continue;
        };
        change.old_start = was.timeline_start;
        change.old_end = was.end();
        change.moved = (clip.timeline_start - was.timeline_start).abs() > NEAR;
        change.trimmed = (clip.duration - was.duration).abs() > NEAR;
        change.effects = clip.effects != was.effects;
        change.properties = clip.text != was.text
            || clip.props != was.props
            || clip.fx != was.fx
            || clip.name != was.name
            || clip.source_id != was.source_id
            || clip
                .source_in
                .zip(was.source_in)
                .is_some_and(|(now, then)| (now - then).abs() > NEAR);
        change.retracked = track.id != was_track.id;
        change.toggled = clip.enabled != was.enabled;
        change.retimed = clip.speed != was.speed;
        change.relinked = clip.link != was.link;
        if change.moved
            || change.trimmed
            || change.effects
            || change.properties
            || change.retracked
            || change.toggled
            || change.retimed
            || change.relinked
        {
            changes.push(change);
        }
    }
    for (track, clip) in &before {
        if after.iter().any(|(_, now)| now.id == clip.id) {
            continue;
        }
        changes.push(ClipChange {
            clip: clip.id.clone(),
            name: clip.label().to_owned(),
            track: track.label().to_owned(),
            removed: true,
            old_start: clip.timeline_start,
            old_end: clip.end(),
            ..ClipChange::default()
        });
    }
    changes.extend(transition_changes(program, proposed));
    changes
}

/// The transitions `proposed` adds, moves, cuts, restyles or takes off
/// against `program`, by id: a crossfade is a change as a clip is.
fn transition_changes(program: &Timeline, proposed: &Timeline) -> Vec<ClipChange> {
    const NEAR: f64 = 1e-4;
    fn fades(timeline: &Timeline) -> Vec<(&model::Track, &model::Transition)> {
        let mut seen = HashSet::new();
        timeline
            .tracks
            .iter()
            .flat_map(|track| track.transitions.iter().map(move |fade| (track, fade)))
            .filter(|(_, fade)| !fade.id.is_empty() && seen.insert(fade.id.clone()))
            .collect()
    }
    let before = fades(program);
    let after = fades(proposed);
    let mut changes = Vec::new();
    for (track, fade) in &after {
        let mut change = ClipChange {
            clip: fade.id.clone(),
            transition: true,
            name: "Transition".to_owned(),
            track: track.label().to_owned(),
            new_start: fade.start,
            new_end: fade.end,
            ..ClipChange::default()
        };
        let Some((was_track, was)) = before.iter().find(|(_, was)| was.id == fade.id) else {
            change.added = true;
            changes.push(change);
            continue;
        };
        change.old_start = was.start;
        change.old_end = was.end;
        change.moved = (fade.start - was.start).abs() > NEAR;
        change.trimmed = ((fade.end - fade.start) - (was.end - was.start)).abs() > NEAR;
        change.properties = fade.from != was.from || fade.to != was.to || fade.fx != was.fx;
        change.retracked = track.id != was_track.id;
        if change.moved || change.trimmed || change.properties || change.retracked {
            changes.push(change);
        }
    }
    for (track, fade) in &before {
        if after.iter().any(|(_, now)| now.id == fade.id) {
            continue;
        }
        changes.push(ClipChange {
            clip: fade.id.clone(),
            transition: true,
            name: "Transition".to_owned(),
            track: track.label().to_owned(),
            removed: true,
            old_start: fade.start,
            old_end: fade.end,
            ..ClipChange::default()
        });
    }
    changes
}
