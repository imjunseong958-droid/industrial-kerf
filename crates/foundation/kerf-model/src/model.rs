//! The project model.
//!
//! Times are offsets. A clip's `start` counts from the first frame of its
//! sequence, `source_in` from the first frame of its media, and a keyframe's
//! `at` from the first frame of its clip. Timecode (`Media::start`,
//! `Sequence::start`) is added only when presenting or reading text.

use crate::catalog::{Catalog, Domain, ParamType};
use crate::id::{Id, IdGen};
use crate::speed::Speed;
use crate::time::{Rate, Time};
use crate::value::{Interp, Param, Value};
use kerf_color::ColorSpace;
use std::borrow::Cow;
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
    /// The text file the media's file was made from, where another program
    /// made it: a scene a renderer drew, or what a model was asked for. A
    /// path as the media's own is, and a label as the reel is: Kerf reads
    /// nothing from it.
    pub recipe: Option<String>,
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
    /// Bits per component the file holds.
    pub bits: u8,
    /// How its code values become light. Log footage rarely says so itself.
    pub color: ColorSpace,
}

impl Default for VideoInfo {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            alpha: false,
            bits: 8,
            color: ColorSpace::Rec709,
        }
    }
}

impl VideoInfo {
    /// Whether its code values mean something other than what 8-bit sRGB pictures mean, so that
    /// showing them as they are would be wrong: HDR, log, and wide gamut.
    pub fn needs_converting(&self) -> bool {
        !matches!(self.color, ColorSpace::Rec709 | ColorSpace::Srgb)
    }

    /// Whether drawing it should keep more than 8 bits of it.
    pub fn is_deep(&self) -> bool {
        self.bits > 8 || self.needs_converting()
    }
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
    /// What it is finished in: Rec.709, or an HDR signal.
    pub color: ColorSpace,
    /// Timecode of the first frame, conventionally 01:00:00:00.
    pub start: Time,
    /// Where sound tracks are mixed before the sequence's own mix.
    pub buses: Vec<Bus>,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
}

/// Sound tracks mixed together, then through gain, pan, and effects of their own, on to another bus
/// or the sequence's mix.
#[derive(Clone, Debug, PartialEq)]
pub struct Bus {
    pub id: Id,
    pub name: Option<String>,
    /// The bus it feeds; `None` for the sequence's mix.
    pub to: Option<Id>,
    /// Gain and pan (`Catalog::props(Domain::Audio)`), keyframed from the sequence's start.
    pub params: Params,
    pub effects: Vec<Effect>,
}

impl Bus {
    pub fn new(id: Id) -> Self {
        Self {
            id,
            name: None,
            to: None,
            params: Params::new(),
            effects: Vec::new(),
        }
    }
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

    pub fn bus(&self, id: &str) -> Option<&Bus> {
        self.buses.iter().find(|bus| bus.id == id)
    }

    /// Its buses with each before the one it feeds, as a mix needs them; ones feeding each other in
    /// a loop, which validation refuses, come last.
    pub fn buses_in_order(&self) -> Vec<&Bus> {
        let mut order: Vec<&Bus> = Vec::with_capacity(self.buses.len());
        let mut left: Vec<&Bus> = self.buses.iter().collect();
        while !left.is_empty() {
            let (ready, rest): (Vec<&Bus>, Vec<&Bus>) = left
                .iter()
                .partition(|bus| !left.iter().any(|other| other.to.as_ref() == Some(&bus.id)));
            if ready.is_empty() {
                order.extend(rest);
                break;
            }
            order.extend(ready);
            left = rest;
        }
        order
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
    /// What the track carries, for grouping, stems, and the timeline's
    /// role colours: one of [`TRACK_ROLES`] or free text. `None` means the
    /// kind's default ([`Track::role_or_default`]).
    pub role: Option<String>,
    /// Ripple edits on other tracks move this track's clips too, keeping it
    /// in sync with them. Off, the track holds still unless the edit is on
    /// it. Absent from a document means on.
    pub sync_locked: bool,
    /// The bus a sound track feeds; `None` for the sequence's mix.
    pub bus: Option<Id>,
    /// Ordered by start and never overlapping.
    pub clips: Vec<Clip>,
    pub transitions: Vec<Transition>,
}

/// The track roles every surface knows by name. A role may also be any
/// other word or phrase.
pub const TRACK_ROLES: [&str; 5] = ["video", "titles", "dialogue", "music", "effects"];

/// A role as a track keeps it: trimmed, a known role in its own spelling
/// whatever the case written, and `None` for nothing. `Err` for text that
/// is not one line.
pub fn track_role(text: &str) -> Result<Option<String>, &'static str> {
    let role = text.trim();
    if role.is_empty() {
        return Ok(None);
    }
    if role.chars().any(char::is_control) {
        return Err("a track role is one line of text");
    }
    Ok(Some(
        TRACK_ROLES
            .iter()
            .find(|known| known.eq_ignore_ascii_case(role))
            .map_or_else(|| role.to_owned(), |known| (*known).to_owned()),
    ))
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
            role: None,
            sync_locked: true,
            bus: None,
            clips: Vec::new(),
            transitions: Vec::new(),
        }
    }

    /// The track's role, or the one its kind implies: `video` for picture,
    /// `dialogue` for sound, and none for subtitles.
    pub fn role_or_default(&self) -> Option<&str> {
        self.role.as_deref().or(match self.kind {
            TrackKind::Video => Some("video"),
            TrackKind::Audio => Some("dialogue"),
            TrackKind::Subtitle => None,
        })
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
    /// The clip of the same sequence this one moves with: its position, scale, and rotation
    /// apply on top of this clip's own.
    pub parent: Option<Id>,
    /// What a clip of footage shows between the frames of its file.
    pub retime: Retime,
    /// On a clip of a nested sequence, the one track of it that shows or is heard, as a cut of a
    /// multicam sequence shows one camera.
    pub angle: Option<Id>,
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
        speed: Speed,
    },
    /// A nested sequence, as used for reels and compound clips.
    Sequence {
        sequence: Id,
        source_in: Time,
        speed: Speed,
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

/// What footage shows when its frames do not fall on the sequence's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Retime {
    /// The frame showing at that moment.
    #[default]
    Frames,
    /// The two frames around it, cross-faded.
    Blend,
    /// The two frames around it, moved along the motion between them and cross-faded.
    Flow,
}

impl Retime {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Blend => "blend",
            Self::Flow => "flow",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Frames, Self::Blend, Self::Flow]
            .into_iter()
            .find(|retime| retime.id() == id)
    }
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
            parent: None,
            retime: Retime::default(),
            angle: None,
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

    pub fn speed(&self) -> &Speed {
        match &self.content {
            Content::Media { speed, .. } | Content::Sequence { speed, .. } => speed,
            Content::Generator { .. } | Content::Caption { .. } => Speed::normal(),
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
            } => Some((*source_in, *source_in + speed.played(self.duration))),
            Content::Generator { .. } | Content::Caption { .. } => None,
        }
    }

    /// Where on its sequence this clip plays `source`, a time on its media or
    /// nested sequence: past either edge, where it would play it. `None` for
    /// generators and captions.
    pub fn played_at(&self, source: Time) -> Option<Time> {
        let (low, high) = self.source_range()?;
        let speed = self.speed();
        let offset = if speed.is_reverse() {
            high - source
        } else {
            source - low
        };
        Some(self.start + speed.time_for(offset))
    }

    /// What this clip plays at `time` on its sequence, a time on its media
    /// or nested sequence: past either edge, what it would play. The
    /// inverse of [`Clip::played_at`].
    pub fn source_at(&self, time: Time) -> Option<Time> {
        let (low, high) = self.source_range()?;
        let speed = self.speed();
        let span = speed.played(time - self.start);
        Some(if speed.is_reverse() {
            high - span
        } else {
            low + span
        })
    }

    /// How long `handle` of unused source lasts before the clip's first frame, at the speed it
    /// would play there.
    pub fn time_before(&self, handle: Time) -> Time {
        -self.speed().time_for(-handle)
    }

    /// How long `handle` of unused source lasts after the clip's last frame, at the speed it would
    /// play there.
    pub fn time_after(&self, handle: Time) -> Time {
        let speed = self.speed();
        match speed.constant() {
            Some(ratio) => ratio.timeline_span(handle),
            None => speed.time_for(speed.played(self.duration) + handle) - self.duration,
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
            .chain(self.masks.iter_mut().flat_map(|m| m.params.values_mut()))
            .chain(content.into_iter().flatten())
    }

    /// Move the first frame by `delta`: positive removes material from the
    /// head, negative reveals it. Source and keyframes follow the content.
    pub fn move_head(&mut self, delta: Time) {
        let played = self.speed().played(delta);
        let reverse = self.speed().is_reverse();
        self.start += delta;
        self.duration -= delta;
        if !reverse {
            if let Some(source_in) = self.source_in_mut() {
                *source_in += played;
            }
        }
        if let Content::Media { speed, .. } | Content::Sequence { speed, .. } = &mut self.content {
            speed.shift(-delta);
        }
        for param in self.params_mut() {
            *param = param.shifted(-delta);
        }
        for mask in &mut self.masks {
            mask.shape.shift(-delta);
        }
    }

    /// Move the frame after the last by `delta`: positive reveals material,
    /// negative removes it.
    pub fn move_tail(&mut self, delta: Time) {
        let speed = self.speed();
        let reverse = speed.is_reverse();
        let played = speed.played(self.duration + delta) - speed.played(self.duration);
        self.duration += delta;
        if reverse {
            if let Some(source_in) = self.source_in_mut() {
                *source_in -= played;
            }
        }
    }

    /// Show source material `delta` of timeline later (slip).
    pub fn slip(&mut self, delta: Time) {
        let span = self.speed().played(delta);
        let reverse = self.speed().is_reverse();
        if let Some(source_in) = self.source_in_mut() {
            *source_in += if reverse { -span } else { span };
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
    /// it as a path. `soft` says how many picture pixels wide the edge's fall-off is at each corner,
    /// over the mask's `feather`, going evenly from one to the next; empty is none.
    Polygon {
        points: Vec<[f64; 2]>,
        closed: bool,
        soft: Vec<f64>,
    },
    /// An outline that changes over the clip: outlines of one kind, with the same corners, at times
    /// relative to the clip's start. Before the first and after the last the nearest one holds.
    Moving(Vec<OutlineKey>),
}

/// An outline at a time relative to the start of its clip, and how it moves toward the next.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineKey {
    pub at: Time,
    pub shape: MaskShape,
    pub interp: Interp,
}

impl MaskShape {
    /// The outline at clip time `at`.
    pub fn at(&self, at: Time) -> Cow<'_, Self> {
        let Self::Moving(keys) = self else {
            return Cow::Borrowed(self);
        };
        let next = keys.partition_point(|key| key.at <= at);
        match (next.checked_sub(1).map(|i| &keys[i]), keys.get(next)) {
            (None, Some(first)) => Cow::Borrowed(&first.shape),
            (Some(last), None) => Cow::Borrowed(&last.shape),
            (Some(left), Some(right)) => {
                let across = (at - left.at).flicks() as f64 / (right.at - left.at).flicks() as f64;
                let along = left.interp.curve(across);
                let mix = |a: f64, b: f64| a + (b - a) * along;
                Cow::Owned(match (&left.shape, &right.shape) {
                    (Self::Rect(a), Self::Rect(b)) => {
                        Self::Rect(std::array::from_fn(|i| mix(a[i], b[i])))
                    }
                    (Self::Ellipse(a), Self::Ellipse(b)) => {
                        Self::Ellipse(std::array::from_fn(|i| mix(a[i], b[i])))
                    }
                    (
                        Self::Polygon {
                            points: a,
                            closed,
                            soft: from,
                        },
                        Self::Polygon {
                            points: b,
                            soft: to,
                            ..
                        },
                    ) if a.len() == b.len() => {
                        // A key that says nothing of it says 0.
                        let soft_at =
                            |soft: &[f64], corner: usize| soft.get(corner).copied().unwrap_or(0.0);
                        Self::Polygon {
                            points: a
                                .iter()
                                .zip(b)
                                .map(|(a, b)| [mix(a[0], b[0]), mix(a[1], b[1])])
                                .collect(),
                            closed: *closed,
                            soft: if from.is_empty() && to.is_empty() {
                                Vec::new()
                            } else {
                                (0..a.len())
                                    .map(|corner| mix(soft_at(from, corner), soft_at(to, corner)))
                                    .collect()
                            },
                        }
                    }
                    _ => left.shape.clone(),
                })
            }
            (None, None) => Cow::Owned(Self::Rect([0.0; 4])),
        }
    }

    /// Whether it encloses something: an open outline only gives effects a path.
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Polygon { closed, .. } => *closed,
            Self::Rect(_) | Self::Ellipse(_) => true,
            Self::Moving(keys) => keys.first().is_none_or(|key| key.shape.is_closed()),
        }
    }

    /// What is wrong with it, for a document to be told.
    pub fn problem(&self) -> Option<&'static str> {
        let fine = |values: &[f64]| values.iter().all(|value| value.is_finite());
        match self {
            Self::Rect(r) if !fine(r) || r[2] <= r[0] || r[3] <= r[1] => {
                Some("a rectangle needs its right past its left and its bottom below its top")
            }
            Self::Ellipse(e) if !fine(e) || e[2] <= 0.0 || e[3] <= 0.0 => {
                Some("an ellipse needs radii above 0")
            }
            Self::Polygon { points, closed, .. }
                if !fine(points.as_flattened()) || points.len() < if *closed { 3 } else { 2 } =>
            {
                Some("an outline needs 3 corners, or 2 points when open")
            }
            Self::Polygon {
                closed: false,
                soft,
                ..
            } if !soft.is_empty() => Some("an open outline has no edge to soften"),
            Self::Polygon { points, soft, .. }
                if !soft.is_empty()
                    && (soft.len() != points.len()
                        || soft.iter().any(|soft| !(0.0..=10_000.0).contains(soft))) =>
            {
                Some("a corner's softness is 0 to 10000 pixels, on every corner")
            }
            Self::Moving(keys) => {
                if keys.windows(2).any(|pair| pair[0].at >= pair[1].at) {
                    return Some("an outline's keyframes need increasing times");
                }
                let same =
                    keys.windows(2)
                        .all(|pair| match (&pair[0].shape, &pair[1].shape) {
                            (Self::Rect(_), Self::Rect(_))
                            | (Self::Ellipse(_), Self::Ellipse(_)) => true,
                            (
                                Self::Polygon {
                                    points: a,
                                    closed: x,
                                    ..
                                },
                                Self::Polygon {
                                    points: b,
                                    closed: y,
                                    ..
                                },
                            ) => a.len() == b.len() && x == y,
                            _ => false,
                        });
                if !same {
                    return Some(
                        "an outline's keyframes need one kind of outline, with the same corners",
                    );
                }
                if keys.iter().any(|key| matches!(key.shape, Self::Moving(_))) {
                    return Some("an outline's keyframes hold outlines, not more keyframes");
                }
                keys.iter().find_map(|key| key.shape.problem()).or(keys
                    .is_empty()
                    .then_some("an outline's keyframes need at least one"))
            }
            _ => None,
        }
    }

    /// Move its keyframes by `delta`, as when a clip's head is trimmed.
    fn shift(&mut self, delta: Time) {
        if let Self::Moving(keys) = self {
            for key in keys {
                key.at += delta;
            }
        }
    }
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

    /// What to say of a clip that is not there: its name, and a name it could be a slip for.
    pub fn no_clip(&self, id: &str) -> String {
        let names = self
            .sequences
            .iter()
            .flat_map(|sequence| &sequence.tracks)
            .flat_map(|track| &track.clips)
            .map(|clip| clip.id.as_str());
        match crate::catalog::nearest(id, names) {
            Some(near) => format!("no clip `{id}`; did you mean `{near}`?"),
            None => format!("no clip `{id}`"),
        }
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
    use crate::time::Ratio;
    use crate::value::{Keyframe, Value};

    fn frames(n: i64) -> Time {
        Rate::FPS_24.time_of(n)
    }

    fn media_clip(speed: &str) -> Clip {
        Clip::new(
            Id::new("c1").unwrap(),
            Content::Media {
                media: Id::new("m1").unwrap(),
                source_in: frames(100),
                speed: Ratio::parse(speed).unwrap().into(),
            },
            frames(10),
            frames(48),
        )
    }

    #[test]
    fn a_clip_plays_its_source_at_its_speed_and_direction() {
        // c1 plays source frames 100..148 from sequence frame 10.
        let forward = media_clip("1");
        assert_eq!(forward.played_at(frames(112)), Some(frames(22)));
        assert_eq!(forward.played_at(frames(90)), Some(frames(0)), "before it");
        let fast = media_clip("2");
        assert_eq!(fast.played_at(frames(124)), Some(frames(22)));
        // In reverse it starts on its source out.
        let reverse = media_clip("-1");
        assert_eq!(reverse.played_at(frames(148)), Some(frames(10)));
        assert_eq!(reverse.played_at(frames(136)), Some(frames(22)));
        for clip in [forward, fast, reverse] {
            for at in [0, 10, 22, 58, 70] {
                let source = clip.source_at(frames(at)).unwrap();
                assert_eq!(clip.played_at(source), Some(frames(at)), "{at}");
            }
        }
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
            recipe: None,
        });
        project.sequences.push(Sequence {
            buses: Vec::new(),
            id: Id::new("s1").unwrap(),
            name: "s1".into(),
            rate: Rate::FPS_24,
            drop_frame: false,
            width: 320,
            height: 180,
            sample_rate: 48_000,
            color: Default::default(),
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

    fn moving_rect() -> MaskShape {
        MaskShape::Moving(vec![
            OutlineKey {
                at: frames(0),
                shape: MaskShape::Rect([0.0, 0.0, 0.2, 0.2]),
                interp: Interp::Linear,
            },
            OutlineKey {
                at: frames(24),
                shape: MaskShape::Rect([0.5, 0.0, 0.7, 0.4]),
                interp: Interp::Hold,
            },
            OutlineKey {
                at: frames(48),
                shape: MaskShape::Rect([0.8, 0.8, 1.0, 1.0]),
                interp: Interp::Linear,
            },
        ])
    }

    #[test]
    fn an_outline_moves_between_its_keyframes() {
        let shape = moving_rect();
        let MaskShape::Rect(between) = shape.at(frames(12)).into_owned() else {
            panic!("a rectangle");
        };
        for (found, wanted) in between.iter().zip([0.25, 0.0, 0.45, 0.3]) {
            assert!((found - wanted).abs() < 1e-9, "{between:?}");
        }
        assert_eq!(*shape.at(frames(36)), MaskShape::Rect([0.5, 0.0, 0.7, 0.4]));
        assert_eq!(*shape.at(frames(-3)), MaskShape::Rect([0.0, 0.0, 0.2, 0.2]));
        assert_eq!(*shape.at(frames(60)), MaskShape::Rect([0.8, 0.8, 1.0, 1.0]));
        assert_eq!(shape.problem(), None);
    }

    #[test]
    fn softness_moves_with_the_corners_and_a_key_without_it_says_none() {
        let polygon = |soft: Vec<f64>| MaskShape::Polygon {
            points: vec![[0.0, 0.0], [0.1, 0.0], [0.0, 0.1]],
            closed: true,
            soft,
        };
        let key = |at: i64, shape: MaskShape| OutlineKey {
            at: frames(at),
            shape,
            interp: Interp::Linear,
        };
        let soft_at = |shape: &MaskShape, at: i64| match shape.at(frames(at)).into_owned() {
            MaskShape::Polygon { soft, .. } => soft,
            other => panic!("{other:?}"),
        };
        let both = MaskShape::Moving(vec![
            key(0, polygon(vec![0.0, 4.0, 8.0])),
            key(10, polygon(vec![10.0, 4.0, 0.0])),
        ]);
        assert_eq!(soft_at(&both, 5), [5.0, 4.0, 4.0]);
        let one = MaskShape::Moving(vec![
            key(0, polygon(Vec::new())),
            key(10, polygon(vec![10.0, 4.0, 0.0])),
        ]);
        assert_eq!(soft_at(&one, 5), [5.0, 2.0, 0.0]);
        let none = MaskShape::Moving(vec![
            key(0, polygon(Vec::new())),
            key(10, polygon(Vec::new())),
        ]);
        assert!(soft_at(&none, 5).is_empty());
        assert!(polygon(vec![1.0, 2.0, 3.0]).problem().is_none());
        assert!(polygon(vec![1.0, 2.0]).problem().is_some());
        assert!(polygon(vec![1.0, -2.0, 3.0]).problem().is_some());
        assert!(polygon(vec![1.0, f64::NAN, 3.0]).problem().is_some());
        let open = MaskShape::Polygon {
            points: vec![[0.0, 0.0], [0.1, 0.0]],
            closed: false,
            soft: vec![1.0, 1.0],
        };
        assert!(open.problem().is_some());
    }

    #[test]
    fn outlines_that_cannot_move_into_each_other_are_refused() {
        let key = |at: i64, shape: MaskShape| OutlineKey {
            at: frames(at),
            shape,
            interp: Interp::Linear,
        };
        let rect = MaskShape::Rect([0.0, 0.0, 0.5, 0.5]);
        let triangle = |extra: bool| MaskShape::Polygon {
            points: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]
                .into_iter()
                .chain(extra.then_some([0.0, 1.0]))
                .collect(),
            closed: true,
            soft: Vec::new(),
        };
        for bad in [
            vec![key(0, rect.clone()), key(0, rect.clone())],
            vec![key(0, rect.clone()), key(5, MaskShape::Ellipse([0.5; 4]))],
            vec![key(0, triangle(false)), key(5, triangle(true))],
            vec![key(0, MaskShape::Rect([0.5, 0.0, 0.1, 0.5]))],
            vec![],
        ] {
            assert!(
                MaskShape::Moving(bad.clone()).problem().is_some(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_mask_stays_on_the_picture_through_a_trim_and_a_split() {
        let mut whole = media_clip("1");
        let mut mask = Mask::new(Id::new("k1").unwrap(), moving_rect());
        mask.params.insert(
            "feather".into(),
            Param::Animated(vec![
                Keyframe {
                    at: frames(0),
                    value: Value::Number(0.0),
                    interp: Interp::Linear,
                },
                Keyframe {
                    at: frames(24),
                    value: Value::Number(24.0),
                    interp: Interp::Linear,
                },
            ]),
        );
        whole.masks.push(mask);
        let check = |clip: &Clip, sequence_frame: i64| {
            let mask = clip.mask("k1").unwrap();
            let at = frames(sequence_frame) - clip.start;
            let there = frames(sequence_frame) - whole.start;
            let whole = whole.mask("k1").unwrap();
            assert_eq!(mask.shape.at(at), whole.shape.at(there));
            assert_eq!(
                mask.params["feather"].value_at(at),
                whole.params["feather"].value_at(there)
            );
        };
        let mut trimmed = whole.clone();
        trimmed.move_head(frames(6));
        check(&trimmed, 20);
        let mut head = whole.clone();
        let tail = head.split_off(frames(12), Id::new("c2").unwrap());
        check(&head, 15);
        check(&tail, 30);
    }

    /// A clip from source frame 100 that slows from 2 to 1/2 over 48 frames, or the reverse of that.
    fn ramped(reverse: bool) -> Clip {
        let sign = if reverse { "-" } else { "" };
        let key = |at: i64, speed: &str| crate::speed::RampKey {
            at: frames(at),
            speed: Ratio::parse(&format!("{sign}{speed}")).unwrap(),
            interp: Interp::Linear,
        };
        let mut clip = media_clip("1");
        let Content::Media { speed, .. } = &mut clip.content else {
            unreachable!();
        };
        *speed = Speed::ramp(vec![key(0, "2"), key(48, "0.5")]).unwrap();
        clip
    }

    /// What each frame of `clips` plays, one after another on the sequence.
    fn plays(clips: &[&Clip], from: i64, to: i64) -> Vec<i64> {
        (from..to)
            .map(|frame| {
                let time = frames(frame);
                let clip = clips
                    .iter()
                    .find(|clip| clip.start <= time && time < clip.end())
                    .expect("a clip shows");
                clip.source_at(time).unwrap().flicks()
            })
            .collect()
    }

    fn close(a: &[i64], b: &[i64]) {
        assert_eq!(a.len(), b.len());
        for (frame, (a, b)) in a.iter().zip(b).enumerate() {
            assert!((a - b).abs() <= 2, "frame {frame}: {a} against {b}");
        }
    }

    #[test]
    fn a_split_ramp_plays_what_it_played_on_both_sides() {
        for reverse in [false, true] {
            let whole = ramped(reverse);
            let before = plays(&[&whole], 10, 58);
            for at in [1, 7, 12, 30, 47] {
                let mut head = whole.clone();
                let tail = head.split_off(frames(at), Id::new("c2").unwrap());
                assert_eq!(head.end(), tail.start);
                close(&plays(&[&head, &tail], 10, 58), &before);
                // Their sources meet.
                let (head_range, tail_range) =
                    (head.source_range().unwrap(), tail.source_range().unwrap());
                if reverse {
                    assert!((tail_range.1 - head_range.0).abs().flicks() <= 2);
                } else {
                    assert!((head_range.1 - tail_range.0).abs().flicks() <= 2);
                }
            }
        }
    }

    #[test]
    fn a_ramp_finds_its_way_back_from_a_source_time() {
        for reverse in [false, true] {
            let clip = ramped(reverse);
            for frame in [10, 22, 40, 57, 70, 2] {
                let time = frames(frame);
                let played = clip.played_at(clip.source_at(time).unwrap()).unwrap();
                assert!((played - time).abs().flicks() < 1_000, "frame {frame}");
            }
        }
    }

    #[test]
    fn a_ramp_measures_its_handles_at_the_speed_it_would_play_them() {
        let clip = ramped(false);
        // Twice as fast before its first key, half as fast after its last.
        assert_eq!(clip.time_before(frames(10)), frames(5));
        assert_eq!(clip.time_after(frames(10)), frames(20));
    }

    #[test]
    fn slipping_a_ramp_moves_the_source_and_not_the_ramp() {
        let mut clip = ramped(false);
        let before = clip.source_range().unwrap();
        clip.slip(frames(4));
        let moved = clip.speed().played(frames(4));
        assert_eq!(clip.source_range().unwrap().0, before.0 + moved);
        assert_eq!(clip.speed(), ramped(false).speed());
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
