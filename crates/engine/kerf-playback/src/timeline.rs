use crate::PlaybackError;
use kerf_compose::{ComposeOptions, Compositor};
use kerf_core::preview::{FindingSeverity, PreviewMode, PreviewPlan, PreviewRange, PreviewReason};
use kerf_core::program::{
    compile_program, program_sequence, program_sound, ProgramError, ProgramOptions, ProgramSound,
};
use kerf_model::{
    Catalog, Clip, Content, Domain, Effect, Id, Mask, Param, ParamType, Project, Rate, Sequence,
    Time, TimeChain, TimeMap, TrackKind, Value,
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
    /// The rate its frames are counted at.
    pub rate: Rate,
    pub has_audio: bool,
    pub subtitles: Vec<PathBuf>,
}

/// A source-backed clip normalized for playback: the part of a media clip the program plays,
/// where it plays it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackClip {
    pub id: String,
    pub track_id: String,
    pub track_kind: ClipKind,
    pub source_id: String,
    pub timeline_start: f64,
    pub duration: f64,
    pub volume: f32,
    pub track_muted: bool,
    /// Raster effects run on each picture, in stack order; empty for sound.
    pub effects: Vec<Effect>,
    /// Masks that cut each picture before its effects run; empty for sound.
    pub masks: Vec<Mask>,
    /// The picture size the effects measure against: the media's.
    pub layer: (u32, u32),
    /// The seed of its random effects, as a bake uses.
    pub seed: u32,
    /// The part of the program it plays over, `[from, to)`.
    span: (Time, Time),
    /// How the program's time maps onto the media's: down the clips holding the sequence the
    /// clip is in, and by the clip itself.
    plays: TimeChain<'static>,
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

    /// The second of its media the clip plays at `timeline_time`, kept within the part of the
    /// program it plays over.
    pub fn source_time_at(&self, timeline_time: f64) -> f64 {
        let at = Time::from_seconds(timeline_time).unwrap_or(self.span.0);
        self.plays
            .source(at.clamp(self.span.0, self.span.1))
            .seconds()
    }

    /// The frame of its media, at the media's `rate`, the clip shows at `at` of the program:
    /// the one the compositor draws there.
    pub fn source_frame_at(&self, at: Time, rate: Rate) -> i64 {
        self.plays.frame(at, rate).max(0)
    }

    /// Map a source timestamp back into this clip's timeline interval.
    pub fn timeline_time_for_source(&self, source_time: f64) -> f64 {
        Time::from_seconds(source_time)
            .map_or(self.timeline_start, |source| {
                self.plays.played_at(source).seconds()
            })
            .clamp(self.timeline_start, self.timeline_end())
    }

    /// The clip's own time at `at` of the program, where its keyframes are read.
    pub(crate) fn clip_time_at(&self, at: Time) -> Time {
        self.plays.clip().time(self.plays.own(at))
    }

    /// The clip's own time where it plays second `source_time` of its media.
    pub(crate) fn clip_time_of_source(&self, source_time: f64) -> Time {
        let own = self.plays.clip();
        Time::from_seconds(source_time).map_or(Time::ZERO, |source| own.time(own.played_at(source)))
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

impl ResolvedClip {}

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
    /// The rate `fps` is of, which the program's frames are counted at.
    rate: Rate,
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
    /// The project as it plays (looks expanded), for a solo to remix.
    program: Arc<Project>,
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

    /// The program's frame showing at `seconds`, as the compositor counts it when it draws
    /// the picture ([`crate::drawn`]).
    pub(crate) fn frame_at(&self, seconds: f64) -> i64 {
        let frames = seconds * f64::from(self.rate.num()) / f64::from(self.rate.den());
        ((frames + 1e-6).floor() as i64).max(0)
    }

    /// The rate the program's frames are counted at.
    pub(crate) fn rate(&self) -> Rate {
        self.rate
    }

    /// The program's sound clip by clip, as a render mixes it.
    pub(crate) fn sounds(&self) -> Arc<[ProgramSound]> {
        Arc::clone(&self.sounds)
    }

    /// The program's sound with only the audio tracks `solo` heard; the
    /// whole mix when `solo` names none of them (see
    /// [`crate::mix::soloed_sounds`]).
    pub(crate) fn soloed_sounds(&self, solo: &[Id]) -> Arc<[ProgramSound]> {
        if solo.is_empty() {
            return self.sounds();
        }
        crate::mix::soloed_sounds(&self.program, solo).map_or_else(|| self.sounds(), Into::into)
    }

    /// Build a playback plan from the first sequence of a validated project.
    /// A project without a sequence plays nothing.
    pub fn from_project(project: &Project) -> Result<Self, PlaybackError> {
        // Looks play as the built-ins they stand for.
        let expanded = kerf_model::look::expand(project, Catalog::builtin())
            .map_err(PlaybackError::InvalidProject)?;
        let project = &expanded;
        let sequence = project.sequences.first();
        let sources: Vec<PlaybackSource> = project
            .media
            .iter()
            .map(|media| PlaybackSource {
                id: media.id.to_string(),
                path: PathBuf::from(&media.path),
                duration: Some(media.duration.seconds()),
                rate: media.rate,
                has_audio: media.audio.is_some(),
                subtitles: Vec::new(),
            })
            .collect();
        let mut flattener = Flattener {
            project,
            clips: Vec::new(),
            captions: Vec::new(),
            holders: Vec::new(),
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
        // A clip that ramps plays sound made for it.
        let heard = kerf_compose::ramps::with_sound(project, |_, _, _| None);
        let sounds = program_sound(&heard, Catalog::builtin(), None).unwrap_or_default();
        let program = heard.into_owned();
        let rate = sequence.map_or(Rate::FPS_30, |sequence| sequence.rate);
        Ok(Self {
            fps: rate.fps(),
            rate,
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
            program: Arc::new(program),
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
    /// workers use this on every frame.
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
    kerf_fx::is_raster(effect.fx.as_str())
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
    /// The clips holding the sequence being laid, from the program's in, each with the rate
    /// of the sequence it holds.
    holders: Vec<(TimeMap<'static>, Rate)>,
}

impl Flattener<'_> {
    /// Add `[from, to)` of `sequence`, starting at program time `at`.
    fn sequence(&mut self, sequence: &Sequence, from: Time, to: Time, at: Time, muted: bool) {
        if self.holders.len() > MAX_NESTING {
            return;
        }
        let place = |time: Time| time - from + at;
        for track in &sequence.tracks {
            if !track.enabled {
                continue;
            }
            for clip in track.clips.iter().filter(|clip| clip.enabled) {
                let (lo, hi) = (clip.start.max(from), clip.end().min(to));
                if lo >= hi {
                    continue;
                }
                let Some(map) = clip.time_map() else {
                    if let (TrackKind::Subtitle, Content::Caption { text }) =
                        (track.kind, &clip.content)
                        && !text.trim().is_empty()
                    {
                        self.captions.push(CaptionCue {
                            start: place(lo).seconds(),
                            end: place(hi).seconds(),
                            text: text.trim().to_owned(),
                        });
                    }
                    continue;
                };
                if map.is_reverse() {
                    continue;
                }
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
                            timeline_start: place(lo).seconds(),
                            duration: (hi - lo).seconds(),
                            volume: 10f64.powf(gain / 20.0) as f32,
                            track_muted: muted || track.muted,
                            effects,
                            masks,
                            layer: video.map_or((0, 0), |video| (video.width, video.height)),
                            seed: kerf_core::raster::seed(clip.id.as_str()),
                            span: (place(lo), place(hi)),
                            plays: TimeChain::new(self.holders.clone(), map.into_owned()),
                        });
                    }
                    Content::Sequence { sequence: id, .. } if map.speed().is_one() => {
                        if let Some(nested) = self.project.sequence(id.as_str()) {
                            let (inner_from, inner_to) = map.window(lo, hi);
                            self.holders.push((map.into_owned(), nested.rate));
                            self.sequence(
                                nested,
                                inner_from,
                                inner_to,
                                place(lo),
                                muted || track.muted,
                            );
                            self.holders.pop();
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
    let expanded = match kerf_model::look::expand(project, Catalog::builtin()) {
        Ok(expanded) => expanded,
        Err(message) => {
            return plan(
                PreviewMode::Unavailable,
                vec![reason(
                    "compiler_rejected_feature",
                    None,
                    None,
                    message,
                    FindingSeverity::Error,
                )],
            )
        }
    };
    let project = &expanded;
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
                if let Content::Sequence { sequence: id, .. } = &clip.content
                    && let Some(nested) = self.project.sequence(id.as_str())
                {
                    self.depth += 1;
                    // Where the nested sequence's own start falls in the program.
                    let starts = clip.played_at(Time::ZERO).unwrap_or(clip.start);
                    self.sequence(nested, starts + at, shows);
                    self.depth -= 1;
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
            return (nested && !clip.speed().is_one())
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
                    kerf_fx::is_raster(effect.fx.as_str())
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
        if clip.angle.is_some() {
            return Some("a multicam angle");
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
            Content::Media { media, .. }
                if self
                    .project
                    .media(media.as_str())
                    .and_then(|media| media.video)
                    .is_some_and(|video| video.needs_converting()) =>
            {
                return Some("footage in another color space")
            }
            Content::Generator { .. } => return Some("a generator"),
            Content::Caption { .. } => return Some("captions"),
            Content::Sequence { .. } if !clip.speed().is_one() => {
                return Some("a speed change on a nested sequence")
            }
            Content::Media { .. } | Content::Sequence { .. } => {}
        }
        if clip.speed().is_reverse() {
            return Some("reverse playback");
        }
        if clip.speed().constant().is_none() {
            return Some("a speed ramp");
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
