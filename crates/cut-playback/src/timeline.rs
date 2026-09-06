use crate::PlaybackError;
use cut_core::evaluation::EvaluationPlan;
use cut_domain::{Clip, Overlay, Project, Source, TrackType};
use std::collections::HashMap;
use std::path::PathBuf;

/// An inline caption cue in timeline coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Source metadata needed by the playback backend.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackSource {
    pub id: String,
    pub path: PathBuf,
    pub duration: Option<f64>,
    pub has_audio: bool,
    pub subtitles: Vec<PathBuf>,
}

/// A source-backed clip normalized for playback.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackClip {
    pub id: String,
    pub track_id: String,
    pub track_kind: TrackType,
    pub source_id: String,
    pub timeline_start: f64,
    pub duration: f64,
    pub source_in: f64,
    pub source_out: f64,
    pub speed: f64,
    pub volume: f32,
    pub track_muted: bool,
}

impl PlaybackClip {
    pub fn timeline_end(&self) -> f64 {
        self.timeline_start + self.duration
    }

    pub fn contains(&self, timeline_time: f64) -> bool {
        timeline_time.is_finite()
            && timeline_time >= self.timeline_start
            && timeline_time < self.timeline_end()
    }

    /// Map a timeline timestamp into the source interval, including speed.
    pub fn source_time_at(&self, timeline_time: f64) -> f64 {
        let elapsed = (timeline_time - self.timeline_start).max(0.0);
        (self.source_in + elapsed * self.speed).min(self.source_out)
    }
}

/// A clip together with its resolved source.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedClip {
    pub clip: PlaybackClip,
    pub source: PlaybackSource,
}

impl ResolvedClip {
    pub fn timeline_end(&self) -> f64 {
        self.clip.timeline_end()
    }

    pub fn source_time_at(&self, timeline_time: f64) -> f64 {
        self.clip.source_time_at(timeline_time)
    }
}

/// Immutable playback plan derived from the canonical typed project.
///
/// The plan is deliberately separate from `cut-domain::Project`: it contains
/// only the resolved media data required by a decoder and can be moved to a
/// worker thread without giving that worker ownership of project state.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackTimeline {
    /// Shared immutable timing interpretation also used by offline export.
    pub evaluation: EvaluationPlan,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub sources: Vec<PlaybackSource>,
    pub clips: Vec<PlaybackClip>,
    pub captions: Vec<CaptionCue>,
    pub subtitles: Vec<PathBuf>,
}

impl PlaybackTimeline {
    /// Build a playback plan from a validated typed project.
    pub fn from_project(project: &Project) -> Result<Self, PlaybackError> {
        cut_domain::validate(project)
            .map_err(|error| PlaybackError::InvalidProject(error.to_string()))?;
        let project_value = serde_json::to_value(project)
            .map_err(|error| PlaybackError::InvalidProject(error.to_string()))?;
        let evaluation = EvaluationPlan::from_project_json(&project_value)
            .map_err(PlaybackError::InvalidProject)?;

        let (width, height) = canvas_size(project);
        let mut sources = Vec::with_capacity(project.sources.len());
        for source in &project.sources {
            sources.push(playback_source(source));
        }
        let source_by_id: HashMap<&str, &PlaybackSource> = sources
            .iter()
            .map(|source| (source.id.as_str(), source))
            .collect();

        let mut captions = Vec::new();
        let mut clips = Vec::new();
        for track in &project.tracks {
            if track.kind == TrackType::Title {
                for clip in &track.clips {
                    let Some(text) = clip.text.as_deref().filter(|text| !text.trim().is_empty())
                    else {
                        continue;
                    };
                    let Some(duration) = clip.timeline_duration() else {
                        continue;
                    };
                    captions.push(CaptionCue {
                        start: clip.timeline_start,
                        end: clip.timeline_start + duration,
                        text: text.trim().to_owned(),
                    });
                }
            }
            if !matches!(track.kind, TrackType::Video | TrackType::Audio) {
                continue;
            }
            let enabled = track.enabled.unwrap_or(true);
            if !enabled {
                continue;
            }
            for clip in &track.clips {
                let Some(source_id) = clip.source_id.as_ref() else {
                    continue;
                };
                let Some(source) = source_by_id.get(source_id.as_str()).copied() else {
                    return Err(PlaybackError::InvalidProject(format!(
                        "clip {} references unknown source {}",
                        clip.id, source_id
                    )));
                };
                let Some(normalized) = normalize_clip(
                    clip,
                    track.id.as_str(),
                    track.kind,
                    track.muted.unwrap_or(false),
                    source,
                ) else {
                    continue;
                };
                clips.push(normalized);
            }
        }

        for overlay in &project.overlays {
            if let Overlay::Captions(overlay) = overlay {
                let offset = overlay.start.unwrap_or(0.0);
                let overlay_end = overlay
                    .duration
                    .filter(|duration| duration.is_finite() && *duration > 0.0)
                    .map(|duration| offset + duration);
                for cue in &overlay.cues {
                    let start = cue.start + offset;
                    let end = cue.end + offset;
                    let end = overlay_end.map_or(end, |limit| end.min(limit));
                    if start.is_finite()
                        && end.is_finite()
                        && end > start
                        && !cue.text.trim().is_empty()
                    {
                        captions.push(CaptionCue {
                            start,
                            end,
                            text: cue.text.trim().to_owned(),
                        });
                    }
                }
            }
        }
        captions.sort_by(|left, right| {
            left.start
                .partial_cmp(&right.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let duration = project.duration().max(
            clips
                .iter()
                .map(PlaybackClip::timeline_end)
                .fold(0.0, f64::max),
        );
        let subtitles = project
            .overlays
            .iter()
            .filter_map(|overlay| match overlay {
                Overlay::Subtitles(overlay) => Some(PathBuf::from(&overlay.path)),
                _ => None,
            })
            .collect();
        Ok(Self {
            evaluation,
            fps: project.fps,
            width,
            height,
            duration,
            sources,
            clips,
            captions,
            subtitles,
        })
    }

    pub fn video_at(&self, timeline_time: f64) -> Option<ResolvedClip> {
        self.video_at_ref(timeline_time)
            .map(|(clip, source)| ResolvedClip {
                clip: clip.clone(),
                source: source.clone(),
            })
    }

    pub fn audio_at(&self, timeline_time: f64) -> Vec<ResolvedClip> {
        self.clips
            .iter()
            .filter(|clip| {
                matches!(clip.track_kind, TrackType::Audio | TrackType::Video)
                    && !clip.track_muted
                    && clip.contains(timeline_time)
            })
            .filter_map(|clip| {
                self.sources
                    .iter()
                    .find(|source| source.id == clip.source_id)
                    .cloned()
                    .map(|source| ResolvedClip {
                        clip: clip.clone(),
                        source,
                    })
            })
            .filter(|resolved| resolved.source.has_audio)
            .collect()
    }

    /// Resolve the active video clip without cloning media metadata. Decoder
    /// workers use this on every frame; the owned `video_at` API remains for
    /// callers that need to retain the result.
    pub(crate) fn video_at_ref(
        &self,
        timeline_time: f64,
    ) -> Option<(&PlaybackClip, &PlaybackSource)> {
        self.clips
            .iter()
            .rev()
            .filter(|clip| clip.track_kind == TrackType::Video)
            .filter(|clip| clip.contains(timeline_time))
            .find_map(|clip| {
                self.sources
                    .iter()
                    .find(|source| source.id == clip.source_id)
                    .map(|source| (clip, source))
            })
    }

    pub(crate) fn video_at_indices(&self, timeline_time: f64) -> Option<(usize, usize)> {
        self.clips
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, clip)| clip.track_kind == TrackType::Video && clip.contains(timeline_time))
            .find_map(|(clip_index, clip)| {
                self.sources
                    .iter()
                    .position(|source| source.id == clip.source_id)
                    .map(|source_index| (clip_index, source_index))
            })
    }

    pub(crate) fn audio_at_indices(&self, timeline_time: f64) -> Option<(usize, usize)> {
        self.clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| {
                matches!(clip.track_kind, TrackType::Audio | TrackType::Video)
                    && !clip.track_muted
                    && clip.contains(timeline_time)
            })
            .find_map(|(clip_index, clip)| {
                self.sources
                    .iter()
                    .position(|source| source.id == clip.source_id && source.has_audio)
                    .map(|source_index| (clip_index, source_index))
            })
    }

    pub fn next_video_start_after(&self, timeline_time: f64) -> Option<f64> {
        self.clips
            .iter()
            .filter(|clip| {
                clip.track_kind == TrackType::Video && clip.timeline_start > timeline_time
            })
            .map(|clip| clip.timeline_start)
            .min_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
    }

    pub fn captions_at(&self, timeline_time: f64) -> Vec<CaptionCue> {
        self.captions
            .iter()
            .filter(|cue| timeline_time >= cue.start && timeline_time < cue.end)
            .cloned()
            .collect()
    }
}

fn playback_source(source: &Source) -> PlaybackSource {
    let subtitles = source
        .subs
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    PlaybackSource {
        id: source.id.to_string(),
        path: PathBuf::from(&source.path),
        duration: source.duration,
        has_audio: source.has_audio.unwrap_or(true),
        subtitles,
    }
}

fn normalize_clip(
    clip: &Clip,
    track_id: &str,
    track_kind: TrackType,
    track_muted: bool,
    source: &PlaybackSource,
) -> Option<PlaybackClip> {
    let speed = clip.speed.unwrap_or(1.0);
    if !speed.is_finite() || speed <= 0.0 {
        return None;
    }
    let source_in = clip.source_in.unwrap_or(0.0);
    let duration = clip
        .timeline_duration()
        .or_else(|| {
            source
                .duration
                .map(|duration| (duration - source_in) / speed)
        })
        .filter(|duration| duration.is_finite() && *duration > 0.0)?;
    let source_end_from_duration = source_in + duration * speed;
    let source_out = clip
        .source_out
        .unwrap_or(source_end_from_duration)
        .min(source_end_from_duration);
    if !clip.timeline_start.is_finite()
        || clip.timeline_start < 0.0
        || !source_in.is_finite()
        || source_in < 0.0
        || !source_out.is_finite()
        || source_out <= source_in
    {
        return None;
    }
    Some(PlaybackClip {
        id: clip.id.to_string(),
        track_id: track_id.to_owned(),
        track_kind,
        source_id: source.id.clone(),
        timeline_start: clip.timeline_start,
        duration: duration.min((source_out - source_in) / speed),
        source_in,
        source_out,
        speed,
        volume: clip.volume.unwrap_or(1.0).clamp(0.0, 1.0) as f32,
        track_muted,
    })
}

fn canvas_size(project: &Project) -> (u32, u32) {
    if let (Some(width), Some(height)) = (project.width, project.height) {
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            return (dimension(width), dimension(height));
        }
    }

    let mut largest = None;
    for source in &project.sources {
        let (Some(width), Some(height)) = (source.width, source.height) else {
            continue;
        };
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            let area = width * height;
            if largest.is_none_or(|(largest_area, _, _)| area > largest_area) {
                largest = Some((area, width, height));
            }
        }
    }
    largest
        .map(|(_, width, height)| (dimension(width), dimension(height)))
        .unwrap_or((1920, 1080))
}

fn dimension(value: f64) -> u32 {
    value.round().clamp(2.0, u32::MAX as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::PlaybackTimeline;
    use cut_core::capability::{capability, CapabilitySupport, EditCapability};
    use cut_domain::parse_project;
    use serde_json::json;

    fn project(extra: serde_json::Value) -> cut_domain::Project {
        let mut value = json!({
            "version": 2,
            "name": "Playback test",
            "fps": 30,
            "width": 320,
            "height": 180,
            "sources": [{
                "id": "source-a",
                "path": "/tmp/source-a.mp4",
                "duration": 10,
                "hasAudio": false
            }],
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": []
            }],
            "overlays": [],
            "gaps": [],
            "transitions": [],
            "effects": [],
            "markers": []
        });
        if let (Some(root), Some(extra_root)) = (value.as_object_mut(), extra.as_object()) {
            for (key, item) in extra_root {
                root.insert(key.clone(), item.clone());
            }
        }
        parse_project(&value).expect("valid playback project")
    }

    #[test]
    fn maps_speed_and_placement_into_source_time() {
        let project = project(json!({
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": [{
                    "id": "clip-a",
                    "sourceId": "source-a",
                    "in": 3,
                    "out": 7,
                    "timelineStart": 2,
                    "speed": 2
                }]
            }]
        }));
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        let resolved = timeline.video_at(2.5).expect("active clip");
        assert_eq!(resolved.clip.source_time_at(2.0), 3.0);
        assert_eq!(resolved.clip.source_time_at(2.5), 4.0);
        assert_eq!(resolved.clip.source_time_at(3.99), 6.98);
        assert!(timeline.video_at(4.0).is_none());
        assert_eq!(timeline.duration, 4.0);
    }

    #[test]
    fn explicit_duration_limits_the_declared_source_out() {
        let project = project(json!({
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": [{
                    "id": "clip-a",
                    "sourceId": "source-a",
                    "in": 3,
                    "out": 7,
                    "duration": 0.5,
                    "timelineStart": 0,
                    "speed": 2
                }]
            }]
        }));
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        let clip = timeline.video_at(0.49).expect("active clip").clip;
        assert_eq!(clip.source_out, 4.0);
        assert_eq!(clip.duration, 0.5);
    }

    #[test]
    fn source_duration_fills_a_clip_without_explicit_in_out() {
        let project = project(json!({
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": [{
                    "id": "clip-a",
                    "sourceId": "source-a",
                    "timelineStart": 1
                }]
            }]
        }));
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        let clip = timeline.video_at(5.0).expect("active clip").clip;
        assert_eq!(clip.source_in, 0.0);
        assert_eq!(clip.source_out, 10.0);
        assert_eq!(clip.duration, 10.0);
        assert_eq!(timeline.duration, 11.0);
        assert!(timeline.audio_at(5.0).is_empty());
    }

    #[test]
    fn caption_offsets_are_timeline_relative_and_end_is_exclusive() {
        let project = project(json!({
            "overlays": [{
                "id": "captions-1",
                "type": "captions",
                "start": 2,
                "duration": 3,
                "cues": [{"start": 0.5, "end": 2, "text": " hello "}]
            }]
        }));
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        assert_eq!(timeline.captions_at(2.5)[0].text, "hello");
        assert!(timeline.captions_at(4.5).is_empty());
        assert!(timeline.captions_at(4.0).is_empty());
    }

    #[test]
    fn live_capability_contract_matches_the_playback_projection() {
        for feature in [
            EditCapability::Placement,
            EditCapability::SourceRange,
            EditCapability::Speed,
            EditCapability::Volume,
        ] {
            assert_eq!(capability(feature).live, CapabilitySupport::Exact);
        }
        for feature in [
            EditCapability::Crop,
            EditCapability::Transform,
            EditCapability::Opacity,
            EditCapability::LayeredVideo,
            EditCapability::AudioMixing,
            EditCapability::Effects,
            EditCapability::Keyframes,
            EditCapability::Transitions,
        ] {
            assert_eq!(capability(feature).live, CapabilitySupport::Unsupported);
        }
    }
}
