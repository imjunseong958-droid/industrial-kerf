use crate::PlaybackError;
use kerf_compose::{ComposeOptions, Compositor};
use kerf_core::preview::{FindingSeverity, PreviewMode, PreviewPlan, PreviewRange, PreviewReason};
use kerf_core::program::{
    compile_program, program_sequence, program_sound, ProgramError, ProgramOptions, ProgramSound,
};
use kerf_model::{
    Catalog, Clip, Content, Domain, Effect, Engine, Mask, Param, ParamType, Project, Sequence,
    Time, TrackKind, Value,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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
    /// Raster effects run on each picture, in stack order; empty for sound.
    pub effects: Vec<Effect>,
    /// Masks that cut each picture before its effects run; empty for sound.
    pub masks: Vec<Mask>,
    /// The picture size the effects measure against: the media's.
    pub layer: (u32, u32),
    /// The clip time at `timeline_start`, where its keyframes are read.
    pub clip_offset: f64,
    /// The seed of its random effects, as a bake uses.
    pub seed: u32,
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
/// the first sequence: the topmost picture as cuts, and the sound as a
/// render mixes it. [`preview_plan`] says when that is not the whole
/// program.
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
    sounds: Arc<[ProgramSound]>,
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

    /// The program's sound clip by clip, as a render mixes it.
    pub(crate) fn sounds(&self) -> Arc<[ProgramSound]> {
        Arc::clone(&self.sounds)
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
        // A program a render cannot mix plays no sound; the preview plan
        // says why.
        let sounds = program_sound(project, Catalog::builtin(), None).unwrap_or_default();
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
            sounds: sounds.into(),
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

/// Whether an effect is a raster effect, which playback runs itself.
fn raster(effect: &Effect) -> bool {
    Catalog::builtin()
        .fx(effect.fx.as_str())
        .is_some_and(|fx| fx.engine == Engine::Raster)
}

/// Raster effect categories playback cannot run on the one picture it holds,
/// as keys and mattes make parts of it transparent.
const NOT_LIVE: &[&str] = &["Keying", "Matte"];

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
                        let video = self
                            .project
                            .media(media.as_str())
                            .and_then(|media| media.video)
                            .filter(|_| kind == ClipKind::Video);
                        let (effects, masks) = match video {
                            Some(_) => (
                                clip.effects
                                    .iter()
                                    .filter(|effect| effect.enabled && raster(effect))
                                    .cloned()
                                    .collect(),
                                clip.masks.clone(),
                            ),
                            None => (Vec::new(), Vec::new()),
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
                            effects,
                            masks,
                            layer: video.map_or((0, 0), |video| (video.width, video.height)),
                            clip_offset: (lo - clip.start).seconds(),
                            seed: kerf_core::raster::seed(clip.id.as_str()),
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

/// How the viewer plays the first sequence: natively when that shows it
/// exactly as a render would, otherwise with the compositor drawing the
/// picture live. Its reasons then say what playback or an export does not
/// match yet: sound live playback leaves out, and sound an export cannot
/// mix. Nothing plays when the compositor cannot draw the program, which no
/// render can then either.
pub fn preview_plan(project: &Project) -> PreviewPlan {
    let sequence = match program_sequence(project, None) {
        Ok(sequence) => sequence,
        Err(ProgramError::Invalid(message) | ProgramError::Unsupported { what: message, .. }) => {
            return plan(
                PreviewMode::Unavailable,
                vec![reason(
                    "compiler_rejected_feature",
                    None,
                    None,
                    message,
                    FindingSeverity::Error,
                )],
            );
        }
    };
    if !sequence.duration().is_positive() {
        return plan(PreviewMode::NativeExact, Vec::new());
    }
    // A render mixes the sound with the graph and draws the picture with
    // the compositor.
    let unmixed = compile_program(
        project,
        Catalog::builtin(),
        &ProgramOptions {
            audio_only: true,
            ..ProgramOptions::default()
        },
    )
    .err()
    .map(|error| {
        reason(
            "compiler_rejected_feature",
            error.entity(),
            None,
            error.to_string(),
            FindingSeverity::Error,
        )
    });
    let drawn = Compositor::new(project, Catalog::builtin(), &ComposeOptions::default());
    let mut check = NativeCheck {
        project,
        picture: Vec::new(),
        sound: Vec::new(),
        shown: Vec::new(),
        depth: 0,
    };
    check.sequence(sequence, Time::ZERO, (Time::ZERO, sequence.duration()));
    let NativeCheck { picture, sound, .. } = check;
    match drawn {
        Ok(_) if unmixed.is_none() && picture.is_empty() && sound.is_empty() => {
            plan(PreviewMode::NativeExact, Vec::new())
        }
        Ok(_) => {
            let mut reasons = sound;
            reasons.extend(unmixed.map(|reason| PreviewReason {
                severity: FindingSeverity::Warning,
                ..reason
            }));
            plan(PreviewMode::Composited, reasons)
        }
        Err(error) => {
            let mut reasons = picture;
            reasons.extend(sound);
            reasons.push(reason(
                "compositor_rejected_feature",
                error.entity(),
                None,
                error.to_string(),
                FindingSeverity::Error,
            ));
            reasons.extend(unmixed);
            plan(PreviewMode::Unavailable, reasons)
        }
    }
}

fn plan(mode: PreviewMode, mut reasons: Vec<PreviewReason>) -> PreviewPlan {
    let reasons_truncated = reasons.len() > kerf_core::preview::MAX_PREVIEW_REASONS;
    reasons.truncate(kerf_core::preview::MAX_PREVIEW_REASONS);
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

/// What native playback does not show of a sequence, picture and sound
/// apart.
struct NativeCheck<'a> {
    project: &'a Project,
    picture: Vec<PreviewReason>,
    sound: Vec<PreviewReason>,
    /// When the pictures checked so far show, in program time: what shows
    /// beneath the clips of later tracks.
    shown: Vec<(Time, Time)>,
    depth: usize,
}

impl NativeCheck<'_> {
    /// Check `sequence` placed at program time `at`, where it shows during
    /// `window`.
    fn sequence(&mut self, sequence: &Sequence, at: Time, window: (Time, Time)) {
        if self.depth > MAX_NESTING {
            return;
        }
        for track in sequence.tracks.iter().filter(|track| track.enabled) {
            let sound = track.kind == TrackKind::Audio;
            // Sound, crossfades too, plays as a render mixes it.
            if !sound {
                for transition in &track.transitions {
                    self.push(&transition.id, None, "a transition", false);
                }
            }
            for clip in track.clips.iter().filter(|clip| clip.enabled) {
                let range =
                    PreviewRange::new((clip.start + at).seconds(), (clip.end() + at).seconds());
                let shows = (
                    (clip.start + at).max(window.0),
                    (clip.end() + at).min(window.1),
                );
                if let Some(what) = self.unsupported(track.kind, clip) {
                    self.push(&clip.id, range, what, sound);
                } else if track.kind == TrackKind::Video
                    && !clip.masks.is_empty()
                    && self.covers(shows)
                {
                    // Native playback shows one picture at a time: black,
                    // not the tracks below, where masks leave it clear.
                    self.push(&clip.id, range, "a mask over another picture", false);
                }
                if track.kind == TrackKind::Video && shows.0 < shows.1 && self.has_picture(clip) {
                    self.shown.push(shows);
                }
                if let Content::Sequence { sequence: id, .. } = &clip.content {
                    if let Some(nested) = self.project.sequence(id.as_str()) {
                        self.depth += 1;
                        let source_in = clip.source_range().map_or(Time::ZERO, |(start, _)| start);
                        self.sequence(nested, clip.start + at - source_in, shows);
                        self.depth -= 1;
                    }
                }
            }
        }
    }

    /// Whether a picture checked so far shows during any of `(from, to)`.
    fn covers(&self, (from, to): (Time, Time)) -> bool {
        self.shown
            .iter()
            .any(|&(start, end)| start < to && from < end)
    }

    /// Whether a clip on a picture track draws a picture of its own; a
    /// nested sequence's are its clips'.
    fn has_picture(&self, clip: &Clip) -> bool {
        match &clip.content {
            Content::Media { media, .. } => self
                .project
                .media(media.as_str())
                .is_some_and(|media| media.video.is_some()),
            Content::Generator { .. } | Content::Caption { .. } => true,
            Content::Sequence { .. } => false,
        }
    }

    fn unsupported(&self, kind: TrackKind, clip: &Clip) -> Option<&'static str> {
        // Sound plays as a render mixes it, but for a nested sequence at
        // another speed: a render changes the speed of its mix as a whole.
        if kind == TrackKind::Audio {
            let nested = matches!(clip.content, Content::Sequence { .. });
            return (nested && clip.speed().value() != 1.0)
                .then_some("a speed change on a nested sequence");
        }
        // Raster effects on a media picture run live, unless they make it
        // transparent, read its other frames, read another track, or read
        // the clip's masks, which only the compositor hands them.
        let live = |effect: &Effect| {
            kind == TrackKind::Video
                && matches!(clip.content, Content::Media { .. })
                && Catalog::builtin().fx(effect.fx.as_str()).is_some_and(|fx| {
                    let reads_a_track = fx.params.iter().any(|param| {
                        matches!(param.ty, ParamType::Track | ParamType::Sound)
                            && matches!(effect.params.get(&param.name), Some(Param::Static(Value::Text(track))) if !track.is_empty())
                    });
                    let reads_a_mask = !clip.masks.is_empty()
                        && fx.params.iter().any(|param| param.ty == ParamType::Mask);
                    fx.engine == Engine::Raster
                        && !NOT_LIVE.contains(&fx.category.as_str())
                        && !kerf_fx::reads_other_frames(effect.fx.as_str())
                        && !reads_a_track
                        && !reads_a_mask
                })
        };
        if clip
            .effects
            .iter()
            .any(|effect| effect.enabled && !live(effect))
        {
            return Some("effects");
        }
        // Masks cut a media picture live, but not a nested sequence's.
        if matches!(clip.content, Content::Sequence { .. }) && clip.masks.iter().any(kerf_fx::cuts)
        {
            return Some("masks on a nested sequence");
        }
        match &clip.content {
            Content::Media { media, .. }
                if self
                    .project
                    .media(media.as_str())
                    .and_then(|media| media.video)
                    .is_some_and(|video| video.alpha) =>
            {
                return Some("a picture with transparency")
            }
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
        if kind != TrackKind::Video {
            return None;
        }
        let catalog = Catalog::builtin();
        clip.props.iter().find_map(|(name, param)| {
            let default = &catalog.prop(Domain::Video, name)?.default;
            let native = matches!(param, Param::Static(value) if value == default);
            (!native).then_some("a picture property (position, scale, crop, opacity, or blend)")
        })
    }

    fn push(
        &mut self,
        entity: &kerf_model::Id,
        range: Option<PreviewRange>,
        what: &str,
        sound: bool,
    ) {
        let (reasons, message) = if sound {
            (
                &mut self.sound,
                format!("`{entity}` uses {what}, whose sound only a render plays exactly"),
            )
        } else {
            (
                &mut self.picture,
                format!("`{entity}` uses {what}, which native playback does not show"),
            )
        };
        reasons.push(reason(
            "live_offline_capability_mismatch",
            Some(entity.as_str()),
            range,
            message,
            FindingSeverity::Warning,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{preview_plan, ClipKind, NativeCheck, PlaybackClip, PlaybackTimeline};
    use kerf_core::preview::PreviewMode;
    use kerf_lang::parse_document;
    use kerf_model::{Catalog, Project, Time};

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
        let sound = timeline.audio_at(2.5);
        assert_eq!(sound.len(), 1);
        assert_eq!(sound[0].clip.id, "audio-a");
        assert!((sound[0].clip.volume - 0.501).abs() < 0.001);
        let sounds = timeline.sounds();
        assert_eq!(
            sounds
                .iter()
                .map(|sound| sound.clip.as_str())
                .collect::<Vec<_>>(),
            ["audio-a", "audio-b"]
        );
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
    fn preview_plan_draws_what_native_playback_cannot() {
        let plain = project("  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n");
        assert_eq!(preview_plan(&plain).mode, PreviewMode::NativeExact);
        let cropped = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      set crop_l=0.5\n",
        );
        // The compositor draws it, so nothing is left to say.
        let plan = preview_plan(&cropped);
        assert_eq!(plan.mode, PreviewMode::Composited);
        assert!(plan.reasons.is_empty(), "{:?}", plan.reasons);
        // The compositor blends, live and in a render.
        let blended = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      set blend=screen\n",
        );
        let plan = preview_plan(&blended);
        assert_eq!(plan.mode, PreviewMode::Composited);
        assert!(plan.reasons.is_empty(), "{:?}", plan.reasons);
        // A raster effect runs on the picture playback holds; one that reads
        // the clip's other frames needs the compositor.
        let saturated = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      fx color.huesaturation\n",
        );
        assert_eq!(preview_plan(&saturated).mode, PreviewMode::NativeExact);
        // So do FilmCraft's effects, which Kerf's renderer draws.
        let inverted = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      fx channel.invert\n",
        );
        assert_eq!(preview_plan(&inverted).mode, PreviewMode::Composited);
        // So does an effect that draws a sound track.
        let drawn = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      fx generate.audiowaveform audioLayer=A1\n  \
             track A1 audio\n    clip a1 media=source-b src=0s at=0s dur=2s\n",
        );
        assert_eq!(preview_plan(&drawn).mode, PreviewMode::Composited);
        // So does a fix that compares neighbouring frames.
        let steadied = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      fx distort.rollingshutterrepair\n",
        );
        assert_eq!(preview_plan(&steadied).mode, PreviewMode::Composited);
        // So does one that reads another track.
        let matted = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      fx key.trackmatte matte=V2\n  \
             track V2 video\n    clip c2 media=source-a src=0s at=0s dur=2s\n",
        );
        assert_eq!(preview_plan(&matted).mode, PreviewMode::Composited);
        let echoed = project(
            "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n      fx time.echo\n",
        );
        let plan = preview_plan(&echoed);
        assert_eq!(plan.mode, PreviewMode::Composited);
        assert!(plan.reasons.is_empty(), "{:?}", plan.reasons);
        let gained = project(
            "  track A1 audio\n    clip a1 media=source-b src=0s at=0s dur=2s\n      set gain=-3\n",
        );
        assert_eq!(preview_plan(&gained).mode, PreviewMode::NativeExact);
        // A crossfade plays live as a render mixes it.
        let crossfaded = project(
            "  track A1 audio\n    clip a1 media=source-b src=1s at=0s dur=2s\n    \
             clip a2 media=source-b src=4s at=2s dur=2s\n    \
             transition x1 from=a1 to=a2 fx=crossfade@1 dur=10f\n",
        );
        let plan = preview_plan(&crossfaded);
        assert_eq!(plan.mode, PreviewMode::NativeExact, "{:?}", plan.reasons);
        // Neither the compositor nor a render plays a transition out of an
        // adjustment layer.
        let adjusted = project(
            "  track V1 video\n    clip a1 gen=adjustment@1 at=0s dur=2s\n    \
             clip g1 gen=solid@1 at=2s dur=2s\n    \
             transition x1 from=a1 to=g1 fx=dissolve@1 dur=10f\n",
        );
        assert_eq!(preview_plan(&adjusted).mode, PreviewMode::Unavailable);
    }

    /// What native playback leaves out of the picture of the first
    /// sequence, which the compositor then draws.
    fn unshown(project: &Project) -> Vec<String> {
        let sequence = &project.sequences[0];
        let mut check = NativeCheck {
            project,
            picture: Vec::new(),
            sound: Vec::new(),
            shown: Vec::new(),
            depth: 0,
        };
        check.sequence(sequence, Time::ZERO, (Time::ZERO, sequence.duration()));
        check
            .picture
            .into_iter()
            .map(|reason| reason.message)
            .collect()
    }

    #[test]
    fn masks_cut_the_picture_live() {
        let clip = "  track V1 video\n    clip c1 media=source-a src=0s at=0s dur=2s\n";
        let native = |body: &str| {
            let plan = preview_plan(&project(body));
            assert_eq!(
                plan.mode,
                PreviewMode::NativeExact,
                "{body}: {:?}",
                plan.reasons
            );
        };
        native(&format!("{clip}      mask k1 rect=0.1,0.1,0.9,0.9\n"));
        native(&format!(
            "{clip}      mask k1 ellipse=0.5,0.5,0.3,0.4 feather={{0f:0 30f:12}} expansion=-2\n      \
             mask k2 points=\"0.2,0.2 0.8,0.3 0.5,0.9\" mode=subtract inverted=true opacity=0.5\n      \
             mask k3 rect=0,0,0.6,1 mode=intersect\n      \
             fx color.huesaturation\n"
        ));
        // A path for effects cuts nothing.
        native(&format!(
            "{clip}      mask k1 rect=0.1,0.1,0.9,0.9 mode=none\n"
        ));
        // Masked clips one after another.
        native(&format!(
            "{clip}      mask k1 rect=0.1,0.1,0.9,0.9\n    \
             clip c2 media=source-a src=0s at=2s dur=2s\n      mask k1 rect=0.2,0.2,0.8,0.8\n"
        ));

        let composited = |body: &str, what: &str| {
            let project = project(body);
            assert_eq!(
                preview_plan(&project).mode,
                PreviewMode::Composited,
                "{body}"
            );
            let unshown = unshown(&project);
            assert!(
                unshown.iter().any(|reason| reason.contains(what)),
                "{body}: {unshown:?}"
            );
        };
        // Native playback shows black where a mask leaves the picture
        // clear; the compositor shows the track below.
        composited(
            &format!(
                "{clip}  track V2 video\n    clip c2 media=source-a src=0s at=1s dur=2s\n      \
                 mask k1 rect=0.1,0.1,0.9,0.9\n"
            ),
            "`c2` uses a mask over another picture",
        );
        // An effect that reads a mask needs the compositor to hand it over.
        composited(
            &format!("{clip}      mask k1 rect=0.1,0.1,0.9,0.9 mode=none\n      fx generate.audiowaveform path=k1\n"),
            "`c1` uses effects",
        );
    }

    #[test]
    fn masks_over_nested_sequences_need_the_compositor() {
        let nested = |outer: &str| -> Project {
            let text = format!(
                "kerf 1\nproject name=\"n\"\n\
                 media m path=\"/tmp/m.mp4\" rate=30 dur=00:00:10:00 video=320x180\n\
                 sequence main rate=30 size=320x180 audio=48000\n\
                 \x20 track V1 video\n\
                 \x20   clip n1 seq=inner src=0s at=0s dur=2s\n{outer}\
                 sequence inner rate=30 size=320x180 audio=48000\n\
                 \x20 track V1 video\n\
                 \x20   clip i1 media=m src=0s at=0s dur=2s\n\
                 \x20   clip i2 media=m src=0s at=4s dur=2s\n\
                 \x20 track V2 video\n\
                 \x20   clip i3 media=m src=0s at=3s dur=2s\n\
                 \x20     mask k1 rect=0.1,0.1,0.9,0.9\n"
            );
            parse_document(&text, Catalog::builtin()).unwrap().project
        };
        // The nested clip shows only i1, and nothing is beneath i3 there.
        let project = nested("");
        let plan = preview_plan(&project);
        assert_eq!(plan.mode, PreviewMode::NativeExact, "{:?}", plan.reasons);
        // A mask on the nested clip cuts the nested picture as a whole.
        let project = nested("\x20     mask k1 rect=0.1,0.1,0.9,0.9\n");
        assert_eq!(preview_plan(&project).mode, PreviewMode::Composited);
        assert_eq!(
            unshown(&project),
            ["`n1` uses masks on a nested sequence, which native playback does not show"]
        );
        // Over the nested clip, a masked clip shows it beneath.
        let project = nested(
            "\x20 track V2 video\n\
             \x20   clip c2 media=m src=0s at=1s dur=1s\n\
             \x20     mask k1 ellipse=0.5,0.5,0.4,0.4\n",
        );
        assert_eq!(preview_plan(&project).mode, PreviewMode::Composited);
        assert_eq!(
            unshown(&project),
            ["`c2` uses a mask over another picture, which native playback does not show"]
        );
    }

    #[test]
    fn sound_plays_live_as_a_render_mixes_it() {
        let mixed = project(
            "  track A1 audio\n    clip d1 media=source-b src=0s at=0s dur=4s\n\
             \x20     set gain={0f:-12 15f:0:ease} pan=0.3\n\
             \x20     fx audio.eq3 low=3\n\
             \x20 track A2 audio\n    clip m1 media=source-b src=6s at=1s dur=4s speed=-1\n",
        );
        let plan = preview_plan(&mixed);
        assert_eq!(plan.mode, PreviewMode::NativeExact, "{:?}", plan.reasons);
        // Sound nested at another speed is not.
        let text = "kerf 1\nproject name=\"n\"\n\
             media m path=\"/tmp/m.mp4\" rate=30 dur=00:00:10:00 video=320x180 audio=2ch@48000\n\
             sequence main rate=30 size=320x180 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip n1 seq=inner src=0s at=0s dur=2s speed=2\n\
             sequence inner rate=30 size=320x180 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip i1 media=m src=0s at=0s dur=4s\n";
        let project = parse_document(text, Catalog::builtin()).unwrap().project;
        let plan = preview_plan(&project);
        assert_eq!(plan.mode, PreviewMode::Composited);
        assert_eq!(plan.reasons.len(), 1, "{:?}", plan.reasons);
        assert_eq!(plan.reasons[0].entity_id.as_deref(), Some("n1"));
        assert!(
            plan.reasons[0]
                .message
                .contains("a speed change on a nested sequence, whose sound only a render"),
            "{:?}",
            plan.reasons
        );
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
            effects: Vec::new(),
            masks: Vec::new(),
            layer: (0, 0),
            clip_offset: 0.0,
            seed: 0,
        };
        assert!((clip.timeline_time_for_source(1.0) - 2.0).abs() < 1e-9);
        assert!((clip.timeline_time_for_source(5.0) - 4.0).abs() < 1e-9);
        assert!((clip.source_time_at(clip.timeline_time_for_source(3.0)) - 3.0).abs() < 1e-9);
    }
}
