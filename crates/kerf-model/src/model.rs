//! The project model.
//!
//! Times are offsets. A clip's `start` counts from the first frame of its
//! sequence, `source_in` from the first frame of its media, and a keyframe's
//! `at` from the first frame of its clip. Timecode (`Media::start`,
//! `Sequence::start`) is added only when presenting or reading text.

use crate::catalog::{Catalog, Domain, ParamType};
use crate::id::{Id, IdGen};
use crate::time::{Rate, Ratio, Time};
use crate::value::{Param, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub type Params = BTreeMap<String, Param>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    pub name: String,
    pub media: Vec<Media>,
    pub sequences: Vec<Sequence>,
    /// Ids that named something which no longer exists. They are never
    /// allocated or claimed again, so a stale reference cannot silently
    /// address a different item.
    pub retired: BTreeSet<Id>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub id: Id,
    pub path: String,
    /// Picture rate, or the timecode rate of audio-only media.
    pub rate: Rate,
    pub drop_frame: bool,
    /// Timecode of the first frame.
    pub start: Time,
    pub duration: Time,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    pub reel: Option<String>,
}

impl Media {
    pub fn supports(&self, domain: Domain) -> bool {
        match domain {
            Domain::Video => self.video.is_some(),
            Domain::Audio => self.audio.is_some(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    /// The picture carries transparency, which compositing keeps.
    pub alpha: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    pub channels: u16,
    pub sample_rate: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub id: Id,
    pub name: String,
    pub rate: Rate,
    pub drop_frame: bool,
    pub width: u32,
    pub height: u32,
    pub sample_rate: u32,
    /// Timecode of the first frame, conventionally 01:00:00:00.
    pub start: Time,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
}

impl Sequence {
    /// The end of the last clip on any track.
    pub fn duration(&self) -> Time {
        self.tracks
            .iter()
            .filter_map(|track| track.clips.last())
            .map(Clip::end)
            .max()
            .unwrap_or(Time::ZERO)
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn marker(&self, id: &str) -> Option<&Marker> {
        self.markers.iter().find(|marker| marker.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

impl TrackKind {
    pub const ALL: [Self; 3] = [Self::Video, Self::Audio, Self::Subtitle];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Subtitle => "subtitle",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == text)
    }

    pub const fn domain(self) -> Option<Domain> {
        match self {
            Self::Video => Some(Domain::Video),
            Self::Audio => Some(Domain::Audio),
            Self::Subtitle => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: Id,
    pub kind: TrackKind,
    pub name: Option<String>,
    pub enabled: bool,
    pub locked: bool,
    pub muted: bool,
    /// Ordered by start and never overlapping.
    pub clips: Vec<Clip>,
    pub transitions: Vec<Transition>,
}

impl Track {
    pub fn new(id: Id, kind: TrackKind) -> Self {
        Self {
            id,
            kind,
            name: None,
            enabled: true,
            locked: false,
            muted: false,
            clips: Vec::new(),
            transitions: Vec::new(),
        }
    }

    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.id == id)
    }

    pub fn clip_index(&self, id: &str) -> Option<usize> {
        self.clips.iter().position(|clip| clip.id == id)
    }

    /// Whether `[from, to)` holds no clip other than those in `ignore`.
    pub fn is_free(&self, from: Time, to: Time, ignore: &[&Id]) -> bool {
        self.clips
            .iter()
            .filter(|clip| !ignore.contains(&&clip.id))
            .all(|clip| clip.end() <= from || clip.start >= to)
    }

    /// Insert keeping start order. The caller guarantees no overlap.
    pub fn insert_sorted(&mut self, clip: Clip) {
        let index = self
            .clips
            .partition_point(|other| other.start <= clip.start);
        self.clips.insert(index, clip);
    }

    pub fn transition(&self, id: &str) -> Option<&Transition> {
        self.transitions
            .iter()
            .find(|transition| transition.id == id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: Id,
    pub start: Time,
    pub duration: Time,
    pub content: Content,
    pub enabled: bool,
    pub name: Option<String>,
    pub link: Option<Id>,
    /// Intrinsic properties from the catalog; absent means default.
    pub props: Params,
    /// Masks on a picture clip, combined in order.
    pub masks: Vec<Mask>,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Media {
        media: Id,
        source_in: Time,
        speed: Ratio,
    },
    /// A nested sequence, as used for reels and compound clips.
    Sequence {
        sequence: Id,
        source_in: Time,
        speed: Ratio,
    },
    Generator {
        fx: Id,
        /// The generator's version, as documents name it (`title@1`).
        version: u32,
        params: Params,
    },
    Caption {
        text: String,
    },
}

impl Clip {
    pub fn new(id: Id, content: Content, start: Time, duration: Time) -> Self {
        Self {
            id,
            start,
            duration,
            content,
            enabled: true,
            name: None,
            link: None,
            props: Params::new(),
            masks: Vec::new(),
            effects: Vec::new(),
        }
    }

    pub fn mask(&self, name: &str) -> Option<&Mask> {
        self.masks.iter().find(|mask| mask.name == name)
    }

    pub fn end(&self) -> Time {
        self.start + self.duration
    }

    pub fn speed(&self) -> Ratio {
        match &self.content {
            Content::Media { speed, .. } | Content::Sequence { speed, .. } => *speed,
            Content::Generator { .. } | Content::Caption { .. } => Ratio::ONE,
        }
    }

    /// The source interval `[in, out)` for media and nested-sequence clips.
    pub fn source_range(&self) -> Option<(Time, Time)> {
        match &self.content {
            Content::Media {
                source_in, speed, ..
            }
            | Content::Sequence {
                source_in, speed, ..
            } => Some((*source_in, *source_in + speed.source_span(self.duration))),
            Content::Generator { .. } | Content::Caption { .. } => None,
        }
    }

    fn source_in_mut(&mut self) -> Option<&mut Time> {
        match &mut self.content {
            Content::Media { source_in, .. } | Content::Sequence { source_in, .. } => {
                Some(source_in)
            }
            Content::Generator { .. } | Content::Caption { .. } => None,
        }
    }

    /// Every parameter whose keyframes are relative to this clip.
    fn params_mut(&mut self) -> impl Iterator<Item = &mut Param> {
        let content = match &mut self.content {
            Content::Generator { params, .. } => Some(params.values_mut()),
            _ => None,
        };
        self.props
            .values_mut()
            .chain(self.effects.iter_mut().flat_map(|e| e.params.values_mut()))
            .chain(content.into_iter().flatten())
    }

    /// Move the first frame by `delta`: positive removes material from the
    /// head, negative reveals it. Source and keyframes follow the content.
    pub fn move_head(&mut self, delta: Time) {
        let speed = self.speed();
        self.start += delta;
        self.duration -= delta;
        if !speed.is_reverse() {
            if let Some(source_in) = self.source_in_mut() {
                *source_in += speed.source_span(delta);
            }
        }
        for param in self.params_mut() {
            *param = param.shifted(-delta);
        }
    }

    /// Move the frame after the last by `delta`: positive reveals material,
    /// negative removes it.
    pub fn move_tail(&mut self, delta: Time) {
        let speed = self.speed();
        self.duration += delta;
        if speed.is_reverse() {
            if let Some(source_in) = self.source_in_mut() {
                *source_in -= speed.source_span(delta);
            }
        }
    }

    /// Show source material `delta` of timeline later (slip).
    pub fn slip(&mut self, delta: Time) {
        let speed = self.speed();
        if let Some(source_in) = self.source_in_mut() {
            let span = speed.source_span(delta);
            *source_in += if speed.is_reverse() { -span } else { span };
        }
    }

    /// Split at clip-relative `offset`. `self` keeps the head and the tail
    /// is returned with `id`, sharing nothing but the content reference.
    pub fn split_off(&mut self, offset: Time, id: Id) -> Clip {
        let mut tail = self.clone();
        tail.id = id;
        tail.move_head(offset);
        let split_params = |head: &mut Clip, tail: &mut Clip| {
            for (name, param) in head.props.iter_mut() {
                let (left, right) = param.split(offset);
                *param = left;
                tail.props.insert(name.clone(), right);
            }
            for (head_fx, tail_fx) in head.effects.iter_mut().zip(tail.effects.iter_mut()) {
                for (name, param) in head_fx.params.iter_mut() {
                    let (left, right) = param.split(offset);
                    *param = left;
                    tail_fx.params.insert(name.clone(), right);
                }
            }
            if let (
                Content::Generator { params: head, .. },
                Content::Generator { params: tail, .. },
            ) = (&mut head.content, &mut tail.content)
            {
                for (name, param) in head.iter_mut() {
                    let (left, right) = param.split(offset);
                    *param = left;
                    tail.insert(name.clone(), right);
                }
            }
        };
        split_params(self, &mut tail);
        self.move_tail(-(self.duration - offset));
        tail
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub fx: Id,
    /// The effect's version, as documents name it (`blur.gaussian@1`).
    pub version: u32,
    pub enabled: bool,
    pub params: Params,
}

/// A mask on a clip's picture. It shows the picture inside its outline
/// (or, by its mode, outside it, or only where it overlaps the masks
/// before it), and effects can read its outline by name.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    /// Unique among the clip's masks.
    pub name: Id,
    pub shape: MaskShape,
    pub mode: MaskMode,
    pub inverted: bool,
    /// Feather, opacity, and expansion (`Catalog::mask_props`); absent
    /// means default.
    pub params: Params,
}

impl Mask {
    pub fn new(name: Id, shape: MaskShape) -> Self {
        Self {
            name,
            shape,
            mode: MaskMode::Add,
            inverted: false,
            params: Params::new(),
        }
    }
}

/// A mask's outline, in fractions of the clip's picture: 0, 0 is its top
/// left corner and 1, 1 its bottom right.
#[derive(Clone, Debug, PartialEq)]
pub enum MaskShape {
    /// Left, top, right, bottom.
    Rect([f64; 4]),
    /// Center x and y, then the radius across (a fraction of the width)
    /// and down (of the height).
    Ellipse([f64; 4]),
    /// Corners in order. An open outline cuts nothing; effects still read
    /// it as a path.
    Polygon { points: Vec<[f64; 2]>, closed: bool },
}

/// How a mask combines with the masks before it, as After Effects'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MaskMode {
    #[default]
    Add,
    Subtract,
    Intersect,
    Lighten,
    Darken,
    Difference,
    /// Cuts nothing; a path only effects read.
    None,
}

impl MaskMode {
    pub const ALL: [Self; 7] = [
        Self::Add,
        Self::Subtract,
        Self::Intersect,
        Self::Lighten,
        Self::Darken,
        Self::Difference,
        Self::None,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Subtract => "subtract",
            Self::Intersect => "intersect",
            Self::Lighten => "lighten",
            Self::Darken => "darken",
            Self::Difference => "difference",
            Self::None => "none",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == text)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    #[default]
    Center,
    /// Begins at the cut.
    Start,
    /// Ends at the cut.
    End,
}

impl Align {
    pub const ALL: [Self; 3] = [Self::Center, Self::Start, Self::End];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Start => "start",
            Self::End => "end",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|align| align.name() == text)
    }
}

/// A transition across the cut between two adjacent clips on one track.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub id: Id,
    pub from: Id,
    pub to: Id,
    pub fx: Id,
    /// The transition's version, as documents name it (`dissolve@1`).
    pub version: u32,
    pub duration: Time,
    pub align: Align,
    pub params: Params,
}

impl Transition {
    /// Portions before and after the cut. Centered transitions with an odd
    /// frame count put the extra frame after the cut.
    pub fn split_at_cut(&self, rate: Rate) -> (Time, Time) {
        match self.align {
            Align::Start => (Time::ZERO, self.duration),
            Align::End => (self.duration, Time::ZERO),
            Align::Center => {
                let before = rate.time_of(rate.frames_floor(self.duration) / 2);
                (before, self.duration - before)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkerColor {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Pink,
    White,
}

impl MarkerColor {
    pub const ALL: [Self; 9] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Cyan,
        Self::Blue,
        Self::Purple,
        Self::Pink,
        Self::White,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Cyan => "cyan",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Pink => "pink",
            Self::White => "white",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.name() == text)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub id: Id,
    pub at: Time,
    pub duration: Time,
    pub name: Option<String>,
    pub color: Option<MarkerColor>,
    pub note: Option<String>,
}

/// Where a clip lives: sequence, track, and clip indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipAt {
    pub sequence: usize,
    pub track: usize,
    pub clip: usize,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn media(&self, id: &str) -> Option<&Media> {
        self.media.iter().find(|media| media.id == id)
    }

    /// Make the paths a document keeps relative to its project absolute
    /// against `directory`: media files and the files clips' effects read
    /// (`ParamType::File`).
    pub fn resolve_paths(&mut self, catalog: &Catalog, directory: &Path) {
        let resolve = |path: &mut String| {
            if !path.is_empty() && Path::new(path.as_str()).is_relative() {
                *path = directory.join(path.as_str()).to_string_lossy().into_owned();
            }
        };
        for media in &mut self.media {
            resolve(&mut media.path);
        }
        let effects = self
            .sequences
            .iter_mut()
            .flat_map(|sequence| sequence.tracks.iter_mut())
            .flat_map(|track| track.clips.iter_mut())
            .flat_map(|clip| clip.effects.iter_mut());
        for effect in effects {
            let Some(fx) = catalog.fx(effect.fx.as_str()) else {
                continue;
            };
            for (name, param) in &mut effect.params {
                let file = fx
                    .find(name)
                    .is_some_and(|def| matches!(def.ty, ParamType::File(_)));
                if let (true, Param::Static(Value::Text(path))) = (file, param) {
                    resolve(path);
                }
            }
        }
    }

    pub fn sequence(&self, id: &str) -> Option<&Sequence> {
        self.sequences.iter().find(|sequence| sequence.id == id)
    }

    pub fn sequence_index(&self, id: &str) -> Option<usize> {
        self.sequences.iter().position(|sequence| sequence.id == id)
    }

    pub fn locate(&self, clip: &str) -> Option<ClipAt> {
        self.sequences.iter().enumerate().find_map(|(s, sequence)| {
            sequence.tracks.iter().enumerate().find_map(|(t, track)| {
                track.clip_index(clip).map(|c| ClipAt {
                    sequence: s,
                    track: t,
                    clip: c,
                })
            })
        })
    }

    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.locate(id).map(|at| self.clip_at(at))
    }

    pub fn clip_at(&self, at: ClipAt) -> &Clip {
        &self.sequences[at.sequence].tracks[at.track].clips[at.clip]
    }

    pub fn clip_at_mut(&mut self, at: ClipAt) -> &mut Clip {
        &mut self.sequences[at.sequence].tracks[at.track].clips[at.clip]
    }

    /// Every id in the project-wide namespace, including link groups.
    pub fn ids(&self) -> impl Iterator<Item = &Id> {
        let media = self.media.iter().map(|media| &media.id);
        let sequences = self.sequences.iter().flat_map(|sequence| {
            let markers = sequence.markers.iter().map(|marker| &marker.id);
            let tracks = sequence.tracks.iter().flat_map(|track| {
                let clips = track
                    .clips
                    .iter()
                    .flat_map(|clip| std::iter::once(&clip.id).chain(clip.link.as_ref()));
                clips.chain(track.transitions.iter().map(|transition| &transition.id))
            });
            std::iter::once(&sequence.id).chain(markers).chain(tracks)
        });
        media.chain(sequences)
    }

    /// An allocator that skips every live and retired id.
    pub fn id_gen(&self) -> IdGen {
        let mut ids = IdGen::new(self.ids());
        for id in &self.retired {
            ids.retire(id);
        }
        ids
    }

    /// Retire every id in `before` that this project no longer uses, and
    /// forget retirements of ids that are live again (as after an undo).
    pub fn retire_missing<'a>(&mut self, before: impl IntoIterator<Item = &'a Id>) {
        let live: std::collections::HashSet<Id> = self.ids().cloned().collect();
        for id in before {
            if !live.contains(id) {
                self.retired.insert(id.clone());
            }
        }
        self.retired.retain(|id| !live.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Interp, Keyframe, Value};

    fn frames(n: i64) -> Time {
        Rate::FPS_24.time_of(n)
    }

    fn media_clip(speed: &str) -> Clip {
        Clip::new(
            Id::new("c1").unwrap(),
            Content::Media {
                media: Id::new("m1").unwrap(),
                source_in: frames(100),
                speed: Ratio::parse(speed).unwrap(),
            },
            frames(10),
            frames(48),
        )
    }

    #[test]
    fn relative_media_and_effect_files_resolve_against_the_project() {
        let text = |value: &str| Param::Static(Value::Text(value.into()));
        let mut clip = media_clip("1");
        clip.effects = vec![
            Effect {
                fx: Id::new("color.lumetri").unwrap(),
                version: 2,
                enabled: true,
                params: [("look_lut".to_owned(), text("looks/warm.cube"))].into(),
            },
            // Text is not a file.
            Effect {
                fx: Id::new("text.simple").unwrap(),
                version: 1,
                enabled: true,
                params: [("text".to_owned(), text("warm"))].into(),
            },
        ];
        let mut track = Track::new(Id::new("V1").unwrap(), TrackKind::Video);
        track.clips.push(clip);
        let mut project = Project::new("p");
        project.media.push(Media {
            id: Id::new("m1").unwrap(),
            path: "clips/a.mp4".into(),
            rate: Rate::FPS_24,
            drop_frame: false,
            start: Time::ZERO,
            duration: frames(240),
            video: None,
            audio: None,
            reel: None,
        });
        project.sequences.push(Sequence {
            id: Id::new("s1").unwrap(),
            name: "s1".into(),
            rate: Rate::FPS_24,
            drop_frame: false,
            width: 320,
            height: 180,
            sample_rate: 48_000,
            start: Time::ZERO,
            tracks: vec![track],
            markers: Vec::new(),
        });
        let directory = Path::new("/projects/p");
        project.resolve_paths(Catalog::builtin(), directory);
        let joined = |path: &str| directory.join(path).to_string_lossy().into_owned();
        assert_eq!(project.media[0].path, joined("clips/a.mp4"));
        let effects = &project.sequences[0].tracks[0].clips[0].effects;
        assert_eq!(
            effects[0].params["look_lut"],
            text(&joined("looks/warm.cube"))
        );
        assert_eq!(effects[1].params["text"], text("warm"));
    }

    #[test]
    fn head_and_tail_trims_follow_the_source() {
        let mut clip = media_clip("1");
        clip.move_head(frames(8));
        assert_eq!((clip.start, clip.duration), (frames(18), frames(40)));
        assert_eq!(clip.source_range(), Some((frames(108), frames(148))));
        clip.move_tail(frames(-4));
        assert_eq!(clip.source_range(), Some((frames(108), frames(144))));
    }

    #[test]
    fn reverse_clips_trim_from_the_opposite_source_end() {
        let mut clip = media_clip("-2");
        assert_eq!(clip.source_range(), Some((frames(100), frames(196))));
        clip.move_head(frames(8));
        assert_eq!(clip.source_range(), Some((frames(100), frames(180))));
        clip.move_tail(frames(-4));
        assert_eq!(clip.source_range(), Some((frames(108), frames(180))));
    }

    #[test]
    fn split_keeps_source_continuity_and_animation() {
        let mut head = media_clip("1");
        head.props.insert(
            "opacity".into(),
            Param::Animated(vec![
                Keyframe {
                    at: frames(0),
                    value: Value::Number(0.0),
                    interp: Interp::Linear,
                },
                Keyframe {
                    at: frames(48),
                    value: Value::Number(1.0),
                    interp: Interp::Linear,
                },
            ]),
        );
        let tail = head.split_off(frames(12), Id::new("c2").unwrap());
        assert_eq!(head.end(), tail.start);
        assert_eq!(
            head.source_range().unwrap().1,
            tail.source_range().unwrap().0
        );
        assert_eq!(
            tail.props["opacity"].value_at(frames(0)),
            Value::Number(0.25)
        );
        assert_eq!(
            head.props["opacity"].value_at(frames(12)),
            Value::Number(0.25)
        );

        let mut reverse = media_clip("-1");
        let reverse_tail = reverse.split_off(frames(12), Id::new("c3").unwrap());
        assert_eq!(reverse.source_range(), Some((frames(136), frames(148))));
        assert_eq!(
            reverse_tail.source_range(),
            Some((frames(100), frames(136)))
        );
    }

    #[test]
    fn centered_transitions_put_the_odd_frame_after_the_cut() {
        let transition = Transition {
            id: Id::new("t1").unwrap(),
            from: Id::new("a").unwrap(),
            to: Id::new("b").unwrap(),
            fx: Id::new("dissolve").unwrap(),
            version: 1,
            duration: frames(13),
            align: Align::Center,
            params: Params::new(),
        };
        assert_eq!(
            transition.split_at_cut(Rate::FPS_24),
            (frames(6), frames(7))
        );
    }
}
