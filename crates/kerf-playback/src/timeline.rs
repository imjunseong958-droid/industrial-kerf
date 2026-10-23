use crate::PlaybackError;
use kerf_core::preview::{FindingSeverity, PreviewMode, PreviewPlan, PreviewRange, PreviewReason};
use kerf_core::program::{compile_program, program_sequence, ProgramError, ProgramOptions};
use kerf_model::{
    Catalog, Clip, Content, Domain, Param, Project, Sequence, Time, TrackKind, Value,
};
use std::collections::HashMap;
use std::path::PathBuf;

/// Nested sequences deeper than this are not played natively.
const MAX_NESTING: usize = 16;

/// Whether a clip shows picture or plays sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipKind {
    Video,
    Audio,
}

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
    pub track_kind: ClipKind,
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

    /// Map a source timestamp back into this clip's timeline interval.
    pub fn timeline_time_for_source(&self, source_time: f64) -> f64 {
        let speed = if self.speed.is_finite() && self.speed > 0.0 {
            self.speed
        } else {
            1.0
        };
        let time = self.timeline_start + (source_time - self.source_in) / speed;
        time.clamp(self.timeline_start, self.timeline_end())
    }
}

/// A clip together with its resolved source.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedClip {
    pub clip: PlaybackClip,
    pub source: PlaybackSource,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipIndexEntry {
    clip_index: usize,
    start: f64,
    end: f64,
}

impl ResolvedClip {
    pub fn timeline_end(&self) -> f64 {
        self.clip.timeline_end()
    }

    pub fn source_time_at(&self, timeline_time: f64) -> f64 {
        self.clip.source_time_at(timeline_time)
    }
}

/// Immutable playback plan derived from a `kerf-model` project.
///
/// The plan is deliberately separate from the project: it contains only the
/// resolved media data required by a decoder and can be moved to a worker
/// thread without giving that worker ownership of project state. It plays
/// the first sequence as cuts: the topmost picture and every sound clip.
/// [`preview_plan`] says when that is not the whole program.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackTimeline {
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    sources: Vec<PlaybackSource>,
    clips: Vec<PlaybackClip>,
    captions: Vec<CaptionCue>,
    pub subtitles: Vec<PathBuf>,
    source_indices: HashMap<String, usize>,
    caption_prefix_max_end: Vec<f64>,
    clip_index: Vec<ClipIndexEntry>,
    clip_prefix_max_end: Vec<f64>,
}

impl PlaybackTimeline {
    /// Sources resolved by this immutable playback plan.
    pub fn sources(&self) -> &[PlaybackSource] {
        &self.sources
    }

    /// Clips in their original project order, which defines overlap priority.
    pub fn clips(&self) -> &[PlaybackClip] {
        &self.clips
    }

    /// Inline captions sorted by timeline start.
    pub fn captions(&self) -> &[CaptionCue] {
        &self.captions
    }
    /// Build a playback plan from the first sequence of a validated project.
    /// A project without a sequence plays nothing.
    pub fn from_project(project: &Project) -> Result<Self, PlaybackError> {
        let sequence = project.sequences.first();
        let sources: Vec<PlaybackSource> = project
            .media
            .iter()
            .map(|media| PlaybackSource {
                id: media.id.to_string(),
                path: PathBuf::from(&media.path),
                duration: Some(media.duration.seconds()),
                has_audio: media.audio.is_some(),
                subtitles: Vec::new(),
            })
            .collect();
        let mut flattener = Flattener {
            project,
            clips: Vec::new(),
            captions: Vec::new(),
            depth: 0,
        };
        let duration = sequence.map_or(Time::ZERO, Sequence::duration);
        if let Some(sequence) = sequence {
            flattener.sequence(sequence, Time::ZERO, duration, Time::ZERO, false);
        }
        let Flattener {
            clips,
            mut captions,
            ..
        } = flattener;
        captions.sort_by(|left, right| left.start.total_cmp(&right.start));

        let caption_prefix_max_end = captions
            .iter()
            .scan(f64::NEG_INFINITY, |max_end, cue| {
                *max_end = (*max_end).max(cue.end);
                Some(*max_end)
            })
            .collect();
        let source_indices = sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source.id.clone(), index))
            .collect();
        let mut clip_index: Vec<_> = clips
            .iter()
            .enumerate()
            .map(|(clip_index, clip)| ClipIndexEntry {
                clip_index,
                start: clip.timeline_start,
                end: clip.timeline_end(),
            })
            .collect();
        clip_index.sort_by(|left, right| {
            left.start
                .total_cmp(&right.start)
                .then(left.clip_index.cmp(&right.clip_index))
        });
        let clip_prefix_max_end = clip_index
            .iter()
            .scan(f64::NEG_INFINITY, |max_end, entry| {
                *max_end = (*max_end).max(entry.end);
                Some(*max_end)
            })
            .collect();
        let (width, height) = sequence.map_or((1920, 1080), kerf_core::program::canvas);
        Ok(Self {
            fps: sequence.map_or(30.0, |sequence| sequence.rate.fps()),
            width,
            height,
            duration: duration.seconds(),
            sources,
            clips,
            captions,
            subtitles: Vec::new(),
            source_indices,
            caption_prefix_max_end,
            clip_index,
            clip_prefix_max_end,
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
                clip.track_kind == ClipKind::Audio
                    && !clip.track_muted
                    && clip.contains(timeline_time)
            })
            .filter_map(|clip| {
                self.source_indices
                    .get(&clip.source_id)
                    .and_then(|index| self.sources.get(*index))
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
        let entry = self
            .indexed_entries_at(timeline_time)?
            .filter(|entry| self.clips[entry.clip_index].track_kind == ClipKind::Video)
            .max_by_key(|entry| entry.clip_index)?;
        let clip = &self.clips[entry.clip_index];
        let source = self
            .source_indices
            .get(&clip.source_id)
            .and_then(|index| self.sources.get(*index))?;
        Some((clip, source))
    }

    pub(crate) fn video_at_indices(&self, timeline_time: f64) -> Option<(usize, usize)> {
        let entry = self
            .indexed_entries_at(timeline_time)?
            .filter(|entry| self.clips[entry.clip_index].track_kind == ClipKind::Video)
            .max_by_key(|entry| entry.clip_index)?;
        let source_index = self
            .source_indices
            .get(&self.clips[entry.clip_index].source_id)
            .copied()?;
        Some((entry.clip_index, source_index))
    }

    pub(crate) fn audio_at_indices(&self, timeline_time: f64) -> Option<(usize, usize)> {
        self.indexed_entries_at(timeline_time)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let clip = &self.clips[entry.clip_index];
                if clip.track_kind != ClipKind::Audio || clip.track_muted {
                    return None;
                }
                let source_index = self.source_indices.get(&clip.source_id).copied()?;
                self.sources[source_index]
                    .has_audio
                    .then_some((entry.clip_index, source_index))
            })
            .min_by_key(|(clip_index, _)| *clip_index)
    }

    pub fn next_video_start_after(&self, timeline_time: f64) -> Option<f64> {
        if !timeline_time.is_finite() {
            return None;
        }
        let first = self
            .clip_index
            .partition_point(|entry| entry.start <= timeline_time);
        self.clip_index[first..]
            .iter()
            .find(|entry| self.clips[entry.clip_index].track_kind == ClipKind::Video)
            .map(|entry| entry.start)
    }

    fn indexed_entries_at(
        &self,
        timeline_time: f64,
    ) -> Option<impl Iterator<Item = &ClipIndexEntry>> {
        if !timeline_time.is_finite() {
            return None;
        }
        let end = self
            .clip_index
            .partition_point(|entry| entry.start <= timeline_time);
        let first =
            self.clip_prefix_max_end[..end].partition_point(|max_end| *max_end <= timeline_time);
        Some(
            self.clip_index[first..end]
                .iter()
                .filter(move |entry| entry.end > timeline_time),
        )
    }

    pub fn captions_at(&self, timeline_time: f64) -> Vec<CaptionCue> {
        if !timeline_time.is_finite() {
            return Vec::new();
        }
        let first = self
            .caption_prefix_max_end
            .partition_point(|end| *end <= timeline_time);
        let end = self
            .captions
            .partition_point(|cue| cue.start <= timeline_time);
        self.captions[first.min(end)..end]
            .iter()
            .filter(|cue| timeline_time >= cue.start && timeline_time < cue.end)
            .cloned()
            .collect()
    }
}

/// Lays the clips of a sequence, and of the sequences nested in it, on the
/// program timeline.
struct Flattener<'a> {
    project: &'a Project,
    clips: Vec<PlaybackClip>,
    captions: Vec<CaptionCue>,
    depth: usize,
}

impl Flattener<'_> {
    /// Add `[from, to)` of `sequence`, starting at program time `at`.
    fn sequence(&mut self, sequence: &Sequence, from: Time, to: Time, at: Time, muted: bool) {
        if self.depth > MAX_NESTING {
            return;
        }
        let place = |time: Time| (time - from + at).seconds();
        for track in &sequence.tracks {
            if !track.enabled {
                continue;
            }
            for clip in track.clips.iter().filter(|clip| clip.enabled) {
                let (lo, hi) = (clip.start.max(from), clip.end().min(to));
                if lo >= hi {
                    continue;
                }
                let speed = clip.speed();
                let Some((source_in, _)) = clip.source_range() else {
                    if let (TrackKind::Subtitle, Content::Caption { text }) =
                        (track.kind, &clip.content)
                    {
                        if !text.trim().is_empty() {
                            self.captions.push(CaptionCue {
                                start: place(lo),
                                end: place(hi),
                                text: text.trim().to_owned(),
                            });
                        }
                    }
                    continue;
                };
                if speed.is_reverse() {
                    continue;
                }
                let source_at = |time: Time| source_in + speed.source_span(time - clip.start);
                match &clip.content {
                    Content::Media { media, .. } => {
                        let kind = match track.kind {
                            TrackKind::Video => ClipKind::Video,
                            TrackKind::Audio => ClipKind::Audio,
                            TrackKind::Subtitle => continue,
                        };
                        let gain = match clip.props.get("gain") {
                            Some(Param::Static(Value::Number(gain))) => *gain,
                            _ => 0.0,
                        };
                        self.clips.push(PlaybackClip {
                            id: clip.id.to_string(),
                            track_id: track.id.to_string(),
                            track_kind: kind,
                            source_id: media.to_string(),
                            timeline_start: place(lo),
                            duration: (hi - lo).seconds(),
                            source_in: source_at(lo).seconds(),
                            source_out: source_at(hi).seconds(),
                            speed: speed.value(),
                            volume: 10f64.powf(gain / 20.0) as f32,
                            track_muted: muted || track.muted,
                        });
                    }
                    Content::Sequence { sequence: id, .. } if speed.value() == 1.0 => {
                        if let Some(nested) = self.project.sequence(id.as_str()) {
                            self.depth += 1;
                            self.sequence(
                                nested,
                                source_at(lo),
                                source_at(hi),
                                lo - from + at,
                                muted || track.muted,
                            );
                            self.depth -= 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Whether native playback shows the first sequence exactly as a render
/// would. Anything it cannot show is a reason to play a rendered preview
/// instead; anything the renderer cannot render makes both unavailable.
pub fn preview_plan(project: &Project) -> PreviewPlan {
    let mut reasons = Vec::new();
    let mut unavailable = false;
    match program_sequence(project, None) {
        Ok(sequence) if sequence.duration().is_positive() => {
            if let Err(error) =
                compile_program(project, Catalog::builtin(), &ProgramOptions::default())
            {
                unavailable = true;
                reasons.push(reason(
                    "compiler_rejected_feature",
                    error.entity(),
                    None,
                    error.to_string(),
                    FindingSeverity::Error,
                ));
            }
            NativeCheck {
                project,
                reasons: &mut reasons,
                depth: 0,
            }
            .sequence(sequence, Time::ZERO);
        }
        Ok(_) => {}
        Err(ProgramError::Invalid(message) | ProgramError::Unsupported { what: message, .. }) => {
            unavailable = true;
            reasons.push(reason(
                "compiler_rejected_feature",
                None,
                None,
                message,
                FindingSeverity::Error,
            ));
        }
    }
    let reasons_truncated = reasons.len() > kerf_core::preview::MAX_PREVIEW_REASONS;
    reasons.truncate(kerf_core::preview::MAX_PREVIEW_REASONS);
    let mode = if unavailable {
        PreviewMode::Unavailable
    } else if reasons.is_empty() {
        PreviewMode::NativeExact
    } else {
        PreviewMode::RenderedFallback
    };
    PreviewPlan {
        mode,
        range: None,
        reasons,
        reasons_truncated,
    }
}

fn reason(
    code: &str,
    entity: Option<&str>,
    range: Option<PreviewRange>,
    message: String,
    severity: FindingSeverity,
) -> PreviewReason {
    PreviewReason {
        code: code.into(),
        entity_path: entity.map_or_else(|| "/".into(), |id| format!("/{id}")),
        entity_id: entity.map(str::to_owned),
        time_range: range,
        message,
        severity,
    }
}

struct NativeCheck<'a> {
    project: &'a Project,
    reasons: &'a mut Vec<PreviewReason>,
    depth: usize,
}

impl NativeCheck<'_> {
    fn sequence(&mut self, sequence: &Sequence, at: Time) {
        if self.depth > MAX_NESTING {
            return;
        }
        for track in sequence.tracks.iter().filter(|track| track.enabled) {
            for transition in &track.transitions {
                self.push(&transition.id, None, "a transition");
            }
            for clip in track.clips.iter().filter(|clip| clip.enabled) {
                let range =
                    PreviewRange::new((clip.start + at).seconds(), (clip.end() + at).seconds());
                if let Some(what) = self.unsupported(track.kind, clip) {
                    self.push(&clip.id, range, what);
                }
                if let Content::Sequence { sequence: id, .. } = &clip.content {
                    if let Some(nested) = self.project.sequence(id.as_str()) {
                        self.depth += 1;
                        let source_in = clip.source_range().map_or(Time::ZERO, |(start, _)| start);
                        self.sequence(nested, clip.start + at - source_in);
                        self.depth -= 1;
                    }
                }
            }
        }
    }

    fn unsupported(&self, kind: TrackKind, clip: &Clip) -> Option<&'static str> {
        if clip.effects.iter().any(|effect| effect.enabled) {
            return Some("effects");
        }
        match &clip.content {
            Content::Generator { .. } => return Some("a generator"),
            Content::Caption { .. } => return Some("captions"),
            Content::Sequence { .. } if clip.speed().value() != 1.0 => {
                return Some("a speed change on a nested sequence")
            }
            Content::Media { .. } | Content::Sequence { .. } => {}
        }
        if clip.speed().is_reverse() {
            return Some("reverse playback");
        }
        let domain = match kind {
            TrackKind::Video => Domain::Video,
            TrackKind::Audio => Domain::Audio,
            TrackKind::Subtitle => return None,
        };
        let catalog = Catalog::builtin();
        clip.props.iter().find_map(|(name, param)| {
            let default = &catalog.prop(domain, name)?.default;
            let native = match param {
                Param::Static(value) => value == default || name == "gain",
                Param::Animated(_) => false,
            };
            (!native).then_some(match domain {
                Domain::Video => "a picture property (position, scale, crop, opacity, or blend)",
                Domain::Audio => "an animated gain or a pan",
            })
        })
    }

    fn push(&mut self, entity: &kerf_model::Id, range: Option<PreviewRange>, what: &str) {
        self.reasons.push(reason(
            "live_offline_capability_mismatch",
            Some(entity.as_str()),
            range,
            format!("`{entity}` uses {what}, which native playback does not show"),
            FindingSeverity::Warning,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{preview_plan, ClipKind, PlaybackClip, PlaybackTimeline};
    use kerf_core::preview::PreviewMode;
    use kerf_lang::parse_document;
    use kerf_model::{Catalog, Project};

    fn project(body: &str) -> Project {
        let text = format!(
            "kerf 1\nproject name=\"Playback test\"\n\
             media source-a path=\"/tmp/source-a.mp4\" rate=30 dur=00:00:10:00 video=320x180\n\
             media source-b path=\"/tmp/source-b.mp4\" rate=30 dur=00:00:10:00 video=320x180 audio=2ch@48000\n\
             sequence s1 rate=30 size=320x180 audio=48000\n{body}"
        );
        parse_document(&text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project
    }

    #[test]
    fn maps_speed_and_placement_into_source_time() {
        let project = project(
            "  track V1 video\n    clip clip-a media=source-a src=3s at=2s dur=2s speed=2\n",
        );
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        let resolved = timeline.video_at(2.5).expect("active clip");
        assert_eq!(resolved.clip.source_time_at(2.0), 3.0);
        assert_eq!(resolved.clip.source_time_at(2.5), 4.0);
        assert!((resolved.clip.source_time_at(3.99) - 6.98).abs() < 1e-9);
        assert!(timeline.video_at(4.0).is_none());
        assert_eq!(timeline.duration, 4.0);
        assert_eq!(
            (timeline.width, timeline.height, timeline.fps),
            (320, 180, 30.0)
        );
    }

    #[test]
    fn caption_index_preserves_overlaps_and_exclusive_boundaries() {
        let project = project(
            "  track S1 subtitle\n    clip c1 caption=\" long \" at=0s dur=10s\n\
             \x20 track S2 subtitle\n    clip c2 caption=\"short\" at=2s dur=2s\n\
             \x20   clip c3 caption=\"next\" at=4s dur=2s\n",
        );
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        assert_eq!(timeline.captions_at(2.0).len(), 2);
        assert_eq!(
            timeline
                .captions_at(4.0)
                .iter()
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>(),
            ["long", "next"]
        );
        assert!(timeline.captions_at(10.0).is_empty());
        assert!(timeline.captions_at(f64::NAN).is_empty());
    }

    #[test]
    fn sound_plays_from_audio_tracks_only() {
        let project = project(
            "  track V1 video muted=true\n\
             \x20   clip video-a media=source-b src=0s at=0s dur=4s\n\
             \x20   clip video-c media=source-b src=0s at=6s dur=1s\n\
             \x20 track V2 video\n\
             \x20   clip video-b media=source-b src=0s at=2s dur=2s\n\
             \x20 track A1 audio\n\
             \x20   clip audio-a media=source-b src=0s at=1s dur=4s\n\
             \x20     set gain=-6\n\
             \x20   clip audio-b media=source-b src=0s at=5s dur=1s\n",
        );
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        assert_eq!(timeline.video_at(2.5).expect("overlap").clip.id, "video-b");
        assert_eq!(
            timeline.video_at(6.5).expect("later clip").clip.id,
            "video-c"
        );
        assert!(timeline.video_at(4.0).is_none());
        assert_eq!(timeline.next_video_start_after(4.0), Some(6.0));
        assert_eq!(
            timeline.video_at_indices(2.5).map(|(_, source)| source),
            Some(1)
        );
        let (clip, _) = timeline.audio_at_indices(2.5).expect("sound");
        assert_eq!(timeline.clips()[clip].id, "audio-a");
        assert!((timeline.clips()[clip].volume - 0.501).abs() < 0.001);
        assert_eq!(timeline.audio_at(2.5).len(), 1);
        assert!(timeline.video_at(f64::NAN).is_none());
        assert_eq!(timeline.next_video_start_after(f64::NAN), None);
    }

    #[test]
    fn nested_sequences_play_their_clips_in_place() {
        let text = "kerf 1\nproject name=\"n\"\n\
             media m path=\"/tmp/m.mp4\" rate=30 dur=00:00:10:00 video=320x180 audio=2ch@48000\n\
             sequence main rate=30 size=320x180 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip n1 seq=inner src=1s at=2s dur=2s\n\
             sequence inner rate=30 size=320x180 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip i1 media=m src=5s at=0s dur=2s\n\
             \x20   clip i2 media=m src=8s at=2s dur=2s\n";
        let project = parse_document(text, Catalog::builtin()).unwrap().project;
        let timeline = PlaybackTimeline::from_project(&project).expect("timeline");
        let ids: Vec<_> = timeline
            .clips()
            .iter()
            .map(|clip| clip.id.as_str())
            .collect();
        assert_eq!(ids, ["i1", "i2"]);
        let first = &timeline.clips()[0];
        assert_eq!(
            (first.timeline_start, first.duration, first.source_in),
            (2.0, 1.0, 6.0)
        );
        let second = &timeline.clips()[1];
        assert_eq!(
            (second.timeline_start, second.duration, second.source_in),
            (3.0, 1.0, 8.0)
        );
        assert_eq!(preview_plan(&project).mode, PreviewMode::NativeExact);
    }

    #[test]
    fn preview_plan_asks_for_a_render_when_playback_would_differ() {
        let plain = project("  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n");
        assert_eq!(preview_plan(&plain).mode, PreviewMode::NativeExact);
        let cropped = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      set crop_l=0.5\n",
        );
        let plan = preview_plan(&cropped);
        assert_eq!(plan.mode, PreviewMode::RenderedFallback);
        assert_eq!(plan.reasons[0].entity_id.as_deref(), Some("c1"));
        let blended = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      set blend=screen\n",
        );
        assert_eq!(preview_plan(&blended).mode, PreviewMode::Unavailable);
        let gained = project(
            "  track A1 audio\n    clip a1 media=source-b src=0s at=0s dur=2s\n      set gain=-3\n",
        );
        assert_eq!(preview_plan(&gained).mode, PreviewMode::NativeExact);
    }

    #[test]
    fn source_time_maps_back_onto_the_clip_timeline() {
        let clip = PlaybackClip {
            id: "clip".into(),
            track_id: "video-1".into(),
            track_kind: ClipKind::Video,
            source_id: "source-a".into(),
            timeline_start: 2.0,
            duration: 4.0,
            source_in: 1.0,
            source_out: 5.0,
            speed: 2.0,
            volume: 1.0,
            track_muted: false,
        };
        assert!((clip.timeline_time_for_source(1.0) - 2.0).abs() < 1e-9);
        assert!((clip.timeline_time_for_source(5.0) - 4.0).abs() < 1e-9);
        assert!((clip.source_time_at(clip.timeline_time_for_source(3.0)) - 3.0).abs() < 1e-9);
    }
}
