//! The timeline an OTIO file holds: one sequence of a `kerf-model` project
//! in OpenTimelineIO's terms. `otio_json` writes and reads it as an `.otio`
//! file; the Python bridge exchanges the same form for every other adapter.
//!
//! Positions and durations count frames of the timeline `rate`. Media times
//! count frames of the media `rate` and include its timecode `start`, as
//! OTIO source ranges do. Each track lists its clips in order with each
//! transition between the two clips it joins; time between clips is a gap.
//!
//! What OTIO has no field for and Kerf needs to read its own file back (ids,
//! the sequence's name and frame size) travels in `metadata["kerf"]`. Another
//! editor does not read it, so the frame size is still reported as not
//! carried.

use crate::{issue, InterchangeError, InterchangeIssue, IssueSeverity};
use kerf_model::{
    validate, Align, AudioInfo, Catalog, Clip, Content, Diagnostic, Id, IdGen, Marker, MarkerColor,
    Media, Params, Project, Rate, Ratio, Sequence, Track, TrackKind, Transition, VideoInfo,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    #[serde(default)]
    pub name: String,
    pub rate: f64,
    #[serde(default)]
    pub start: i64,
    /// The sequence as Kerf knows it, in `metadata["kerf"]` of the timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<WireSequence>,
    #[serde(default)]
    pub media: Vec<WireMedia>,
    #[serde(default)]
    pub tracks: Vec<WireTrack>,
    #[serde(default)]
    pub markers: Vec<WireMarker>,
}

/// What a sequence is besides its timeline, which OTIO has no field for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSequence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Its timecode counts drop-frame.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drop_frame: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireMedia {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub path: String,
    pub rate: f64,
    #[serde(default)]
    pub start: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireTrack {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub kind: WireKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub items: Vec<Item>,
}

fn enabled() -> bool {
    true
}

fn is_enabled(enabled: &bool) -> bool {
    *enabled
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireKind {
    Video,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Item {
    Clip {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Index into `media`.
        media: usize,
        #[serde(rename = "sourceIn")]
        source_in: i64,
        start: i64,
        duration: i64,
        /// How fast it plays its media, forward: OTIO's `LinearTimeWarp`.
        /// Absent at normal speed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speed: Option<f64>,
        #[serde(default = "enabled", skip_serializing_if = "is_enabled")]
        enabled: bool,
    },
    /// Frames taken from either side of the cut.
    Transition {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        before: i64,
        after: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireMarker {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub at: i64,
    #[serde(default)]
    pub duration: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// OTIO's marker `comment`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The program sequence of `project` as a timeline, and what it leaves out.
pub fn to_timeline(
    project: &Project,
) -> Result<(Timeline, Vec<InterchangeIssue>), InterchangeError> {
    let sequence = project.sequences.first().ok_or_else(|| {
        InterchangeError::new("INTERCHANGE_PROJECT_INVALID", "the project has no sequence")
    })?;
    Ok(sequence_timeline(project, sequence))
}

/// `sequence` of `project` as a timeline, and what it leaves out.
pub fn sequence_timeline(
    project: &Project,
    sequence: &Sequence,
) -> (Timeline, Vec<InterchangeIssue>) {
    let rate = sequence.rate;
    let mut issues = Vec::new();
    if project.sequences.len() > 1 {
        issues.push(lost(
            "sequences_not_exported",
            format!("only sequence `{}` was exported", sequence.id),
            "sequences",
        ));
    }
    issues.push(lost(
        "canvas_not_exported",
        format!(
            "the {}x{} frame size has no OTIO equivalent",
            sequence.width, sequence.height
        ),
        sequence.id.as_str(),
    ));
    if sequence.drop_frame {
        issues.push(lost(
            "drop_frame_not_exported",
            "OTIO does not say that timecode counts drop-frame".into(),
            sequence.id.as_str(),
        ));
    }
    sequence_losses(sequence, &mut issues);
    // The media the sequence plays, in the order it first plays each: all a
    // file holds of them is what its clips name.
    let mut media: Vec<WireMedia> = Vec::new();
    let mut media_index: HashMap<&Id, usize> = HashMap::new();
    let mut tracks = Vec::new();
    for track in &sequence.tracks {
        let kind = match track.kind {
            TrackKind::Video => WireKind::Video,
            TrackKind::Audio => WireKind::Audio,
            TrackKind::Subtitle => {
                issues.push(captions_lost(track, "OTIO"));
                continue;
            }
        };
        if track.locked || track.muted {
            issues.push(lost(
                "track_state_not_exported",
                "track lock and mute were not exported".into(),
                track.id.as_str(),
            ));
        }
        // The clips that are written, by id: a transition needs both of its own.
        let written = |id: &Id| {
            track.clip(id.as_str()).is_some_and(|clip| {
                matches!(&clip.content, Content::Media { media, .. } if project.media(media.as_str()).is_some())
            })
        };
        for transition in &track.transitions {
            if !(written(&transition.from) && written(&transition.to)) {
                issues.push(lost(
                    "transition_not_exported",
                    format!(
                        "`{}` joins a clip that was not exported, and was dropped",
                        transition.fx
                    ),
                    transition.id.as_str(),
                ));
            }
        }
        let mut items = Vec::new();
        for clip in &track.clips {
            let Content::Media {
                media: id,
                source_in,
                ..
            } = &clip.content
            else {
                issues.push(content_lost(clip));
                continue;
            };
            let Some(source) = project.media(id.as_str()) else {
                issues.push(content_lost(clip));
                continue;
            };
            let index = *media_index.entry(&source.id).or_insert_with(|| {
                media.push(WireMedia {
                    id: Some(source.id.to_string()),
                    path: source.path.clone(),
                    rate: source.rate.fps(),
                    start: source.rate.frames_round(source.start),
                    duration: Some(source.rate.frames_round(source.duration)),
                });
                media.len() - 1
            });
            let speed = carried_speed(clip, &mut issues);
            clip_losses(clip, &mut issues);
            if let Some(transition) = track
                .transitions
                .iter()
                .find(|t| t.to == clip.id && written(&t.from))
            {
                let (before, after) = transition.split_at_cut(rate);
                if !is_dissolve(transition) {
                    issues.push(lost(
                        "transition_not_exported",
                        format!("`{}` was exported as a dissolve", transition.fx),
                        transition.id.as_str(),
                    ));
                }
                items.push(Item::Transition {
                    id: Some(transition.id.to_string()),
                    before: rate.frames_round(before),
                    after: rate.frames_round(after),
                });
            }
            items.push(Item::Clip {
                id: Some(clip.id.to_string()),
                name: clip.name.clone(),
                media: index,
                source_in: source.rate.frames_round(source.start + *source_in),
                start: rate.frames_round(clip.start),
                duration: rate.frames_round(clip.duration),
                speed: (speed != Ratio::ONE).then(|| speed.value()),
                enabled: clip.enabled,
            });
        }
        tracks.push(WireTrack {
            id: Some(track.id.to_string()),
            kind,
            name: track.name.clone(),
            enabled: track.enabled,
            items,
        });
    }
    let unused: Vec<&str> = project
        .media
        .iter()
        .filter(|media| !media_index.contains_key(&media.id))
        .map(|media| media.id.as_str())
        .collect();
    if !unused.is_empty() {
        issues.push(lost(
            "media_not_exported",
            format!(
                "media the sequence does not play were not exported: {}",
                unused.join(", ")
            ),
            "media",
        ));
    }
    let markers = sequence
        .markers
        .iter()
        .map(|marker| WireMarker {
            id: Some(marker.id.to_string()),
            at: rate.frames_round(marker.at),
            duration: rate.frames_round(marker.duration),
            name: marker.name.clone(),
            color: marker.color.map(|color| color.name().to_owned()),
            note: marker.note.clone(),
        })
        .collect();
    let timeline = Timeline {
        name: project.name.clone(),
        rate: rate.fps(),
        start: rate.frames_round(sequence.start),
        sequence: Some(WireSequence {
            id: Some(sequence.id.to_string()),
            name: Some(sequence.name.clone()),
            width: Some(sequence.width),
            height: Some(sequence.height),
            drop_frame: sequence.drop_frame,
        }),
        media,
        tracks,
        markers,
    };
    (timeline, issues)
}

pub(crate) fn lost(code: &str, message: String, path: &str) -> InterchangeIssue {
    issue(code, message, IssueSeverity::Loss, Some(path))
}

/// Whether another editor's cross dissolve is what `transition` is.
pub(crate) fn is_dissolve(transition: &Transition) -> bool {
    matches!(transition.fx.as_str(), "dissolve" | "crossfade") && transition.params.is_empty()
}

/// What a caption track is to a file that takes none of it.
pub(crate) fn captions_lost(track: &Track, format: &str) -> InterchangeIssue {
    lost(
        "captions_not_exported",
        format!(
            "the {} caption(s) of this track were not exported to {format}; export them as an .srt or .vtt file",
            track.clips.len()
        ),
        track.id.as_str(),
    )
}

/// A clip that plays no media file: a generator, a nested sequence, a caption.
pub(crate) fn content_lost(clip: &Clip) -> InterchangeIssue {
    let what = match &clip.content {
        Content::Generator { fx, .. } => format!("the `{fx}` generator"),
        Content::Sequence { sequence, .. } => format!("the nested sequence `{sequence}`"),
        Content::Caption { .. } => "a caption".to_owned(),
        Content::Media { media, .. } => format!("media `{media}`, which the project does not have,"),
    };
    lost(
        "content_not_exported",
        format!("only media clips are exported; {what} became a gap"),
        clip.id.as_str(),
    )
}

/// What of a media clip no timeline file here carries: how it is drawn and heard.
pub(crate) fn clip_losses(clip: &Clip, issues: &mut Vec<InterchangeIssue>) {
    let mut parts = Vec::new();
    if !clip.props.is_empty() {
        let names: Vec<&str> = clip.props.keys().map(String::as_str).collect();
        parts.push(format!("properties ({})", names.join(", ")));
    }
    if !clip.effects.is_empty() {
        let names: Vec<&str> = clip.effects.iter().map(|effect| effect.fx.as_str()).collect();
        parts.push(format!("effects ({})", names.join(", ")));
    }
    if !clip.masks.is_empty() {
        parts.push(format!("{} mask(s)", clip.masks.len()));
    }
    if !parts.is_empty() {
        issues.push(lost(
            "properties_not_exported",
            format!("not exported: {}", parts.join("; ")),
            clip.id.as_str(),
        ));
    }
}

/// What of a sequence as a whole no timeline file here carries.
pub(crate) fn sequence_losses(sequence: &Sequence, issues: &mut Vec<InterchangeIssue>) {
    let clips = || sequence.tracks.iter().flat_map(|track| &track.clips);
    if clips().any(|clip| clip.link.is_some()) {
        issues.push(lost(
            "links_not_exported",
            "clip links were not exported: picture and sound arrive as separate clips".into(),
            sequence.id.as_str(),
        ));
    }
    if clips().any(|clip| clip.parent.is_some()) {
        issues.push(lost(
            "parents_not_exported",
            "clips that follow a parent clip arrive where they are without it".into(),
            sequence.id.as_str(),
        ));
    }
    if sequence.color != Default::default() {
        issues.push(lost(
            "color_space_not_exported",
            format!(
                "the sequence's color space, {}, was not exported: the other editor gives the timeline its own",
                sequence.color.id()
            ),
            sequence.id.as_str(),
        ));
    }
    if !sequence.buses.is_empty() || sequence.tracks.iter().any(|track| track.bus.is_some()) {
        issues.push(lost(
            "mix_not_exported",
            "the mix buses and what is on them were not exported".into(),
            sequence.id.as_str(),
        ));
    }
}

/// The one forward speed a timeline file plays `clip` at, with what that
/// leaves out: a ramp becomes its speed overall, as in an EDL, and reverse
/// plays the same source forward.
pub(crate) fn carried_speed(clip: &Clip, issues: &mut Vec<InterchangeIssue>) -> Ratio {
    let speed = clip.speed();
    let forward = match speed.constant() {
        Some(ratio) => Ratio::new(ratio.num().abs(), ratio.den()).unwrap_or(Ratio::ONE),
        None => {
            let overall = ratio_of(speed.average(clip.duration)).unwrap_or(Ratio::ONE);
            issues.push(lost(
                "speed_ramp_not_exported",
                format!("a speed ramp was exported as its speed overall, {overall}"),
                clip.id.as_str(),
            ));
            overall
        }
    };
    if speed.is_reverse() {
        issues.push(lost(
            "reverse_not_exported",
            "reverse was not exported: the clip plays the same source forward".into(),
            clip.id.as_str(),
        ));
    }
    forward
}

/// The ratio Kerf holds nearest `value`, with a denominator up to 1000:
/// `None` outside 1% to 10,000%.
pub(crate) fn ratio_of(value: f64) -> Option<Ratio> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    // Continued fractions: each step is the best ratio of its size.
    let (mut num, mut den) = (1_i64, 0_i64);
    let (mut last_num, mut last_den) = (0_i64, 1_i64);
    let mut rest = value;
    for _ in 0..32 {
        let whole = rest.floor();
        let next_num = whole as i64 * num + last_num;
        let next_den = whole as i64 * den + last_den;
        if next_den > 1000 || next_num > i64::from(i32::MAX) {
            break;
        }
        (last_num, last_den, num, den) = (num, den, next_num, next_den);
        let fraction = rest - whole;
        if fraction < 1e-9 {
            break;
        }
        rest = 1.0 / fraction;
    }
    if den == 0 {
        return None;
    }
    Ratio::new(num as i32, den as u32).ok()
}

/// A one-sequence project from a timeline, and what the conversion changed.
pub fn from_timeline(
    timeline: &Timeline,
) -> Result<(Project, Vec<InterchangeIssue>), InterchangeError> {
    let invalid = |message: String| InterchangeError::new("INTERCHANGE_OUTPUT_INVALID", message);
    let rate = Rate::from_fps(timeline.rate).map_err(|e| invalid(format!("timeline rate: {e}")))?;
    let mut issues = Vec::new();
    let wire = timeline.sequence.clone().unwrap_or_default();
    let (width, height) = match (wire.width, wire.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => (width, height),
        _ => {
            issues.push(issue(
                "canvas_assumed",
                "OTIO has no frame size; the sequence is 1920x1080",
                IssueSeverity::Warning,
                None,
            ));
            (1920, 1080)
        }
    };
    let mut ids = IdGen::default();
    let mut claim = |wanted: Option<&str>, prefix: &str| {
        wanted
            .and_then(|text| Id::new(text).ok())
            .filter(|id| ids.reserve(id))
            .unwrap_or_else(|| ids.next(prefix))
    };
    let used_on = |kind: WireKind, index: usize| {
        timeline.tracks.iter().any(|track| {
            track.kind == kind
                && track
                    .items
                    .iter()
                    .any(|item| matches!(item, Item::Clip { media, .. } if *media == index))
        })
    };
    let mut media = Vec::new();
    for (index, item) in timeline.media.iter().enumerate() {
        let media_rate = Rate::from_fps(item.rate).unwrap_or(rate);
        let on_audio = used_on(WireKind::Audio, index);
        let video = (used_on(WireKind::Video, index) || !on_audio).then_some(VideoInfo {
            width,
            height,
            alpha: false,
            ..Default::default()
        });
        media.push(Media {
            id: claim(item.id.as_deref(), "m"),
            path: item.path.clone(),
            rate: media_rate,
            drop_frame: false,
            start: media_rate.time_of(item.start),
            duration: media_rate.time_of(item.duration.unwrap_or(0).max(1)),
            video,
            audio: on_audio.then_some(AudioInfo {
                channels: 2,
                sample_rate: 48_000,
            }),
            reel: None,
            recipe: None,
            steps: Vec::new(),
            sync: None,
        });
    }
    if !media.is_empty() {
        issues.push(issue(
            "media_assumed",
            "media formats are assumed until the media is probed",
            IssueSeverity::Warning,
            None,
        ));
    }
    let named = |name: Option<String>| name.filter(|name| !name.is_empty());
    let mut sequence = Sequence {
        buses: Vec::new(),
        id: claim(wire.id.as_deref(), "s"),
        name: named(wire.name)
            .or_else(|| named(Some(timeline.name.clone())))
            .unwrap_or_else(|| "Main".into()),
        rate,
        drop_frame: wire.drop_frame && rate.supports_drop_frame(),
        width,
        height,
        sample_rate: 48_000,
        color: Default::default(),
        start: rate.time_of(timeline.start),
        tracks: Vec::new(),
        markers: Vec::new(),
    };
    for wire in &timeline.tracks {
        let kind = match wire.kind {
            WireKind::Video => TrackKind::Video,
            WireKind::Audio => TrackKind::Audio,
        };
        let prefix = if kind == TrackKind::Video { "V" } else { "A" };
        let mut track = Track::new(claim(wire.id.as_deref(), prefix), kind);
        track.name.clone_from(&wire.name);
        track.enabled = wire.enabled;
        let mut pending: Option<(Option<&str>, i64, i64)> = None;
        for item in &wire.items {
            match item {
                Item::Transition { id, before, after } => {
                    pending = Some((id.as_deref(), *before, *after));
                }
                Item::Clip {
                    id,
                    name,
                    media: index,
                    source_in,
                    start,
                    duration,
                    speed,
                    enabled,
                } => {
                    let source = media.get(*index).ok_or_else(|| {
                        invalid(format!("a clip names media {index}, which does not exist"))
                    })?;
                    let id = claim(id.as_deref(), "c");
                    let speed = match speed {
                        None => Ratio::ONE,
                        Some(value) => ratio_of(*value).unwrap_or_else(|| {
                            issues.push(issue(
                                "speed_dropped",
                                format!("a speed of {value} is outside what a clip plays at; it plays at 1"),
                                IssueSeverity::Loss,
                                Some(id.as_str()),
                            ));
                            Ratio::ONE
                        }),
                    };
                    let mut clip = Clip::new(
                        id,
                        Content::Media {
                            media: source.id.clone(),
                            source_in: source.rate.time_of(*source_in) - source.start,
                            speed: speed.into(),
                        },
                        rate.time_of(*start),
                        rate.time_of(*duration),
                    );
                    clip.name.clone_from(name);
                    clip.enabled = *enabled;
                    if let Some((id, before, after)) = pending.take() {
                        match track.clips.last() {
                            Some(from) if from.end() == clip.start => {
                                let fx = if kind == TrackKind::Video {
                                    "dissolve"
                                } else {
                                    "crossfade"
                                };
                                let transition = Transition {
                                    id: claim(id, "t"),
                                    from: from.id.clone(),
                                    to: clip.id.clone(),
                                    fx: Id::new(fx).expect("catalog id"),
                                    version: Catalog::builtin()
                                        .fx(fx)
                                        .expect("a built-in transition")
                                        .version,
                                    duration: rate.time_of(before + after),
                                    align: match (before, after) {
                                        (0, _) => Align::Start,
                                        (_, 0) => Align::End,
                                        _ => Align::Center,
                                    },
                                    params: Params::new(),
                                };
                                if transition.split_at_cut(rate)
                                    != (rate.time_of(before), rate.time_of(after))
                                {
                                    issues.push(issue(
                                        "transition_centered",
                                        "an off-center transition was centered on its cut",
                                        IssueSeverity::Warning,
                                        Some(transition.id.as_str()),
                                    ));
                                }
                                track.transitions.push(transition);
                            }
                            _ => issues.push(issue(
                                "transition_dropped",
                                "a transition not between two adjacent clips was dropped",
                                IssueSeverity::Loss,
                                Some(clip.id.as_str()),
                            )),
                        }
                    }
                    track.insert_sorted(clip);
                }
            }
        }
        sequence.tracks.push(track);
    }
    for wire in &timeline.markers {
        let color = wire
            .color
            .as_deref()
            .and_then(|c| MarkerColor::parse(&c.to_lowercase()));
        if wire.color.is_some() && color.is_none() {
            issues.push(issue(
                "marker_color_dropped",
                "a marker color is not in the marker palette",
                IssueSeverity::Loss,
                wire.id.as_deref(),
            ));
        }
        sequence.markers.push(Marker {
            id: claim(wire.id.as_deref(), "mk"),
            at: rate.time_of(wire.at),
            duration: rate.time_of(wire.duration),
            name: wire.name.clone(),
            color,
            note: wire.note.clone().filter(|note| !note.is_empty()),
        });
    }
    sequence.markers.sort_by_key(|marker| marker.at);
    let project = Project {
        name: timeline.name.clone(),
        media,
        sequences: vec![sequence],
        retired: Default::default(),
    };
    let errors: Vec<String> = validate(&project, Catalog::builtin())
        .into_iter()
        .filter(Diagnostic::is_error)
        .map(|diagnostic| diagnostic.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(invalid(format!(
            "the OTIO timeline is not a valid project: {}",
            errors.join("; ")
        )));
    }
    Ok((project, issues))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_lang::{parse_document, print_document, PrintOptions};

    const DOCUMENT: &str = r#"kerf 1
project name="Short"

media m1 path="/media/a.mov" rate=23.976 tc=01:00:00:00 dur=00:01:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Short" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=01:00:10:00 at=01:00:00:00 dur=00:00:02:00
    clip c2 media=m1 src=01:00:20:00 at=01:00:02:00 dur=00:00:03:00
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
  track A1 audio
    clip a1 media=m1 src=01:00:10:00 at=01:00:00:00 dur=00:00:02:00
    clip a2 media=m1 src=01:00:20:00 at=01:00:03:00 dur=00:00:02:00
  marker mk1 at=01:00:01:00 name="beat" color=red
"#;

    fn project() -> Project {
        parse_document(DOCUMENT, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project
    }

    #[test]
    fn timelines_carry_cuts_handles_and_markers_exactly() {
        let (timeline, issues) = to_timeline(&project()).unwrap();
        let video = &timeline.tracks[0].items;
        assert_eq!(
            video[1],
            Item::Transition {
                id: Some("t1".into()),
                before: 6,
                after: 6,
            }
        );
        assert!(matches!(
            video[2],
            Item::Clip {
                source_in,
                start: 48,
                duration: 72,
                ..
            } if source_in == 86_400 + 480
        ));
        assert!(issues.iter().all(|i| i.code == "canvas_not_exported"));

        let (imported, _) = from_timeline(&timeline).unwrap();
        let text = print_document(&imported, Catalog::builtin(), PrintOptions::default());
        assert_eq!(text, DOCUMENT);
    }

}
