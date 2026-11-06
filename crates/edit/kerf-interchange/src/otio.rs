//! The timeline the OTIO bridge exchanges: the program sequence of a
//! `kerf-model` project in OpenTimelineIO's terms.
//!
//! Positions and durations count frames of the timeline `rate`. Media times
//! count frames of the media `rate` and include its timecode `start`, as
//! OTIO source ranges do. Each track lists its clips in order with each
//! transition between the two clips it joins; time between clips is a gap.

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
    #[serde(default)]
    pub media: Vec<WireMedia>,
    #[serde(default)]
    pub tracks: Vec<WireTrack>,
    #[serde(default)]
    pub markers: Vec<WireMarker>,
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
}

/// The program sequence of `project` as a timeline, and what it leaves out.
pub fn to_timeline(
    project: &Project,
) -> Result<(Timeline, Vec<InterchangeIssue>), InterchangeError> {
    let sequence = project.sequences.first().ok_or_else(|| {
        InterchangeError::new("INTERCHANGE_PROJECT_INVALID", "the project has no sequence")
    })?;
    let rate = sequence.rate;
    let mut issues = Vec::new();
    let mut loss = |code: &str, message: String, path: &str| {
        issues.push(issue(code, message, IssueSeverity::Loss, Some(path)));
    };
    if project.sequences.len() > 1 {
        loss(
            "sequences_not_exported",
            "only the first sequence was exported".into(),
            "sequences",
        );
    }
    loss(
        "canvas_not_exported",
        format!(
            "the {}x{} frame size has no OTIO equivalent",
            sequence.width, sequence.height
        ),
        sequence.id.as_str(),
    );
    let media_index: HashMap<&Id, usize> = project
        .media
        .iter()
        .enumerate()
        .map(|(index, media)| (&media.id, index))
        .collect();
    let media = project
        .media
        .iter()
        .map(|media| WireMedia {
            id: Some(media.id.to_string()),
            path: media.path.clone(),
            rate: media.rate.fps(),
            start: media.rate.frames_round(media.start),
            duration: Some(media.rate.frames_round(media.duration)),
        })
        .collect();
    let mut tracks = Vec::new();
    for track in &sequence.tracks {
        let kind = match track.kind {
            TrackKind::Video => WireKind::Video,
            TrackKind::Audio => WireKind::Audio,
            TrackKind::Subtitle => {
                loss(
                    "captions_not_exported",
                    "caption tracks have no OTIO equivalent".into(),
                    track.id.as_str(),
                );
                continue;
            }
        };
        if track.locked || track.muted {
            loss(
                "track_state_not_exported",
                "track lock and mute were not exported".into(),
                track.id.as_str(),
            );
        }
        let mut items = Vec::new();
        for clip in &track.clips {
            let Content::Media {
                media: id,
                source_in,
                speed,
            } = &clip.content
            else {
                loss(
                    "content_not_exported",
                    "only media clips are exported; this clip became a gap".into(),
                    clip.id.as_str(),
                );
                continue;
            };
            if !speed.is_one() {
                loss(
                    "speed_not_exported",
                    format!("speed {speed} was not exported"),
                    clip.id.as_str(),
                );
            }
            if !clip.props.is_empty() || !clip.effects.is_empty() {
                loss(
                    "properties_not_exported",
                    "clip properties and effects were not exported".into(),
                    clip.id.as_str(),
                );
            }
            if let Some(transition) = track.transitions.iter().find(|t| t.to == clip.id) {
                let (before, after) = transition.split_at_cut(rate);
                if !matches!(transition.fx.as_str(), "dissolve" | "crossfade")
                    || !transition.params.is_empty()
                {
                    loss(
                        "transition_not_exported",
                        format!("`{}` was exported as a dissolve", transition.fx),
                        transition.id.as_str(),
                    );
                }
                items.push(Item::Transition {
                    id: Some(transition.id.to_string()),
                    before: rate.frames_round(before),
                    after: rate.frames_round(after),
                });
            }
            let source = &project.media[media_index[id]];
            items.push(Item::Clip {
                id: Some(clip.id.to_string()),
                name: clip.name.clone(),
                media: media_index[id],
                source_in: source.rate.frames_round(source.start + *source_in),
                start: rate.frames_round(clip.start),
                duration: rate.frames_round(clip.duration),
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
    let markers = sequence
        .markers
        .iter()
        .map(|marker| WireMarker {
            id: Some(marker.id.to_string()),
            at: rate.frames_round(marker.at),
            duration: rate.frames_round(marker.duration),
            name: marker.name.clone(),
            color: marker.color.map(|color| color.name().to_owned()),
        })
        .collect();
    Ok((
        Timeline {
            name: project.name.clone(),
            rate: rate.fps(),
            start: rate.frames_round(sequence.start),
            media,
            tracks,
            markers,
        },
        issues,
    ))
}

/// A one-sequence project from a timeline, and what the conversion changed.
pub fn from_timeline(
    timeline: &Timeline,
) -> Result<(Project, Vec<InterchangeIssue>), InterchangeError> {
    let invalid = |message: String| InterchangeError::new("INTERCHANGE_OUTPUT_INVALID", message);
    let rate = Rate::from_fps(timeline.rate).map_err(|e| invalid(format!("timeline rate: {e}")))?;
    let (width, height) = (1920, 1080);
    let mut issues = vec![issue(
        "canvas_assumed",
        format!("OTIO has no frame size; the sequence is {width}x{height}"),
        IssueSeverity::Warning,
        None,
    )];
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
    let mut sequence = Sequence {
        buses: Vec::new(),
        id: claim(None, "s"),
        name: if timeline.name.is_empty() {
            "Main".into()
        } else {
            timeline.name.clone()
        },
        rate,
        drop_frame: false,
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
                } => {
                    let source = media.get(*index).ok_or_else(|| {
                        invalid(format!("a clip names media {index}, which does not exist"))
                    })?;
                    let mut clip = Clip::new(
                        claim(id.as_deref(), "c"),
                        Content::Media {
                            media: source.id.clone(),
                            source_in: source.rate.time_of(*source_in) - source.start,
                            speed: Ratio::ONE.into(),
                        },
                        rate.time_of(*start),
                        rate.time_of(*duration),
                    );
                    clip.name.clone_from(name);
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
            note: None,
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

    #[test]
    fn what_otio_cannot_hold_is_reported() {
        let mut project = project();
        let sequence = &mut project.sequences[0];
        sequence.tracks[0].clips[0].content = Content::Caption { text: "hi".into() };
        sequence
            .tracks
            .push(Track::new(Id::new("S1").unwrap(), TrackKind::Subtitle));
        let (timeline, issues) = to_timeline(&project).unwrap();
        let codes: Vec<&str> = issues.iter().map(|i| i.code.as_str()).collect();
        assert!(codes.contains(&"content_not_exported"), "{codes:?}");
        assert!(codes.contains(&"captions_not_exported"), "{codes:?}");
        assert_eq!(timeline.tracks.len(), 2);
    }
}
