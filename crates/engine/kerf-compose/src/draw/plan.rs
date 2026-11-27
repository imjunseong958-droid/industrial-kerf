//! What a program is planned into before a frame is drawn ([`super::builder`] plans it): each
//! sequence with its tracks, sounds, captions, and the clips others move with, each track's clips
//! as it shows them with how they enter, its dips and the transitions that mix two clips, and each
//! clip's source and all about it that does not change over time.

use super::*;

pub(super) struct SequencePlan {
    pub(super) name: String,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rate: Rate,
    /// Timecode of the first frame.
    pub(super) start: Time,
    pub(super) drop_frame: bool,
    pub(super) duration: Time,
    pub(super) tracks: Vec<TrackPlan>,
    /// Its picture tracks' ids, from the lowest, for effects that read one.
    pub(super) track_ids: Vec<String>,
    /// Its sound tracks, for effects that draw their sound, and their ids
    /// in order.
    pub(super) sounds: Vec<SoundPlan>,
    pub(super) sound_ids: Vec<String>,
    pub(super) captions: Vec<CaptionPlan>,
    /// The clips other clips move with.
    pub(super) rigs: HashMap<Id, Rig>,
}

pub(super) struct SoundPlan {
    pub(super) id: String,
    pub(super) clips: Vec<SoundClip>,
}

/// A media clip's sound on a sound track.
pub(super) struct SoundClip {
    pub(super) path: String,
    /// How its sequence's time maps onto the media's.
    pub(super) map: TimeMap<'static>,
}

pub(super) struct CaptionPlan {
    pub(super) text: String,
    pub(super) from: Time,
    pub(super) to: Time,
}

pub(super) struct TrackPlan {
    pub(super) id: Id,
    /// Whether it shows; a hidden track is drawn only for effects that read
    /// it.
    pub(super) shows: bool,
    pub(super) shown: Vec<Shown>,
    pub(super) dips: Vec<Dip>,
    pub(super) pairs: Vec<Pair>,
}

/// A transition that mixes two shown clips' pictures across their cut.
pub(super) struct Pair {
    /// The clips it leaves and enters, by their place in the track's shown;
    /// the same clip when only one of them shows.
    pub(super) from: usize,
    pub(super) to: usize,
    /// Only the clip it enters shows: it enters from nothing.
    pub(super) from_nothing: bool,
    pub(super) def: FxDef,
    pub(super) params: Params,
    pub(super) start: Time,
    pub(super) end: Time,
}

/// A clip as its track shows it: its span with transition handles, and
/// how it enters.
pub(super) struct Shown {
    pub(super) plan: ClipPlan,
    pub(super) from: Time,
    pub(super) to: Time,
    pub(super) reveal: Option<Entrance>,
}

/// How a clip enters over the one before it, and from when to when.
pub(super) type Entrance = (Reveal, Time, Time);

#[derive(Clone, Copy, Debug)]
pub(super) enum Reveal {
    Dissolve,
    Wipe {
        angle: f64,
        softness: f64,
    },
    /// Over the outgoing clip as it is: a look's transition draws it.
    Over,
}

/// A dip's color between the two clips of its cut.
pub(super) struct Dip {
    pub(super) color: Rgba,
    pub(super) from: Time,
    pub(super) to: Time,
}

/// A clip other clips move with: what it takes to work out how it has moved them.
pub(super) struct Rig {
    pub(super) clip: Clip,
    /// The source's size in its own pixels.
    pub(super) size: (u32, u32),
    pub(super) fit: bool,
    pub(super) path: Option<MotionPath>,
    /// Its `z`, `rotate_x`, and `rotate_y` are its place in depth, which its children in depth
    /// take with it: a clip in depth, or a null. On a flat clip that draws they are nothing, to
    /// itself or to its children, and a camera and a light are turned by numbers of their own.
    pub(super) in_depth: bool,
    pub(super) digest: Option<u64>,
}

impl SequencePlan {
    /// The clips `clip` moves with, from the one it names outward.
    pub(super) fn parents_of<'a>(&'a self, clip: &'a Clip) -> impl Iterator<Item = &'a Rig> {
        let mut parent = clip.parent.as_ref();
        std::iter::from_fn(move || {
            let rig = parent.and_then(|id| self.rigs.get(id))?;
            parent = rig.clip.parent.as_ref();
            Some(rig)
        })
    }

    /// How the parents of `clip` take it at sequence time `at`, the one it names first, in one
    /// pass over them ([`Taken::by`]): each moves it across the canvas, and with it where the
    /// parents in depth before it hold what they hold.
    pub(super) fn taken_by_parents(&self, clip: &Clip, at: Time) -> Taken {
        let canvas = (f64::from(self.width), f64::from(self.height));
        let parents = self.parents_of(clip);
        Taken::by(parents.map(|rig| rig.at(rig.clip.time_at(at))), canvas)
    }

    /// How the parents of `clip` have moved it across the canvas at sequence time `at`, in
    /// canvas pixels: all that a flat clip takes of them.
    pub(super) fn moved_by_parents(&self, clip: &Clip, at: Time) -> Affine {
        self.taken_by_parents(clip, at).moved
    }
}

impl Rig {
    /// Its numbers at its clip time `time`, as kerf-geom works out from them how it has moved
    /// what moves with it.
    pub(super) fn at(&self, time: Time) -> Parent {
        let depth = || {
            ["z", "rotate_x", "rotate_y"]
                .map(|name| number(&param(&self.clip, name).value_at(time)))
        };
        Parent {
            geometry: geometry_numbers(&self.clip, self.path.as_ref(), time),
            size: pixels(self.size),
            fit: self.fit,
            depth: self.in_depth.then(depth),
        }
    }
}

pub(super) struct ClipPlan {
    pub(super) clip: Clip,
    /// Its name, or its media's file or nested sequence's name.
    pub(super) name: String,
    pub(super) source: Source,
    /// The source's size in its own pixels.
    pub(super) size: (u32, u32),
    /// Fitted to the canvas, keeping its aspect, as media and nested
    /// sequences are.
    pub(super) fit: bool,
    /// Padded to the canvas with black, as the graph pads it.
    pub(super) letterboxed: bool,
    /// Its source's picture may be clear in places, as a generator's is and media with alpha.
    pub(super) alpha: bool,
    /// A frame showing it alone is its media's picture as decoded.
    pub(super) plain: bool,
    /// Any of the properties that place it animate.
    pub(super) animated: bool,
    /// Placed in depth, seen by the sequence's camera.
    pub(super) three_d: bool,
    /// A model films it (`filmed=true`): where its sequence is drawn as a clip of another, it
    /// leaves its place clear.
    pub(super) filmed: bool,
    /// The most of its size it is ever shown at, when under 1: its frames
    /// are decoded at that size.
    pub(super) shown: f64,
    pub(super) blend: Blend,
    pub(super) effects: Vec<Effect>,
    pub(super) path: Option<MotionPath>,
    /// A number for everything about the clip that does not change over time, wherever it starts.
    /// `None` when it draws on something the document does not hold: another track, a sound, a file.
    pub(super) digest: Option<u64>,
    /// Its effects read the sequence's time or name, as a burn-in does.
    pub(super) reads_sequence_time: bool,
}

pub(super) enum Source {
    Media {
        picture: usize,
    },
    Nested {
        sequence: usize,
        /// How the clip's own time maps onto the nested sequence's.
        map: TimeMap<'static>,
        /// The one track of it to show.
        angle: Option<Id>,
    },
    Solid(Rgba),
    Title(TitlePlan),
    /// A sound track drawn.
    Sound(SoundShow),
    /// A generator a crate draws (`crate::domains`).
    Drawn(&'static dyn PictureSource),
    Adjustment,
    /// A null: it draws nothing, and other clips move with it.
    Nothing,
    /// A mesh: it has no picture of its own, and is drawn in depth from its triangles
    /// (`three_d::mesh`).
    Mesh,
}

pub(super) struct TitlePlan {
    pub(super) text: String,
    pub(super) font: String,
    /// The face of `font`, from `style` and `weight` ([`kerf_model::title_face`]).
    pub(super) style: String,
    /// In frame heights.
    pub(super) size: f64,
    pub(super) color: Rgba,
    /// An edge around its letters, and how far it reaches past them, in frame heights.
    pub(super) outline: Option<(Rgba, f64)>,
    pub(super) justify: Justify,
    /// Where a variable font's axes are set, by tag, as they are given.
    pub(super) axes: Vec<([u8; 4], Param)>,
    /// The font's OpenType features it turns on or off, as `features` names them.
    pub(super) features: Vec<([u8; 4], u32)>,
    /// How it comes in, when it does.
    pub(super) reveal: Option<Box<Reveals>>,
}

/// A title's reveal, as its parameters say.
pub(super) struct Reveals {
    pub(super) reveal: Param,
    pub(super) by_word: bool,
    pub(super) soft: Param,
    /// Across and down, in frame heights.
    pub(super) from: [Param; 2],
    pub(super) scale: Param,
    /// In degrees.
    pub(super) rotation: Param,
    pub(super) opacity: Param,
}

impl TrackPlan {
    /// Whether it is drawn: as it says, unless a clip holding its sequence names one track, which
    /// is drawn even when hidden, as the angles of a multicam sequence may be.
    pub(super) fn drawn(&self, angle: Option<&Id>) -> bool {
        match angle {
            Some(angle) => self.id == *angle,
            None => self.shows,
        }
    }

    /// Whether it puts nothing on the canvas at `at`: no dip, and no clip but those that draw
    /// nothing.
    pub(super) fn idle(&self, at: Time) -> bool {
        !self.dips.iter().any(|dip| dip.shows(at))
            && self
                .shown
                .iter()
                .filter(|shown| shown.shows(at))
                .all(|shown| matches!(shown.plan.source, Source::Nothing))
    }

    /// The transition mixing its clip `index` at `at`.
    pub(super) fn pair_at(&self, index: usize, at: Time) -> Option<&Pair> {
        self.pairs.iter().find(|pair| {
            pair.start <= at && at < pair.end && (pair.from == index || pair.to == index)
        })
    }

    /// The one clip in depth it shows at `at`, drawing a picture of its own: not in a transition or a
    /// dip, whose mixes are of whole pictures, which cannot be cut.
    pub(super) fn lone_in_depth(&self, at: Time) -> Option<&Shown> {
        let mut showing = self
            .shown
            .iter()
            .enumerate()
            .filter(|(_, shown)| shown.shows(at));
        let (index, shown) = showing.next()?;
        let mixed = self.dips.iter().any(|dip| dip.shows(at)) || self.pair_at(index, at).is_some();
        let drawn = !matches!(shown.plan.source, Source::Nothing | Source::Adjustment);
        (showing.next().is_none() && shown.plan.three_d && drawn && !mixed).then_some(shown)
    }
}

impl Shown {
    pub(super) fn shows(&self, at: Time) -> bool {
        self.from <= at && at < self.to
    }
}

/// Leave out of `sequence` every clip that draws and that no model films, so that only those
/// one films (`filmed=true`) are drawn, as [`kerf_model::Sequence::only_filmed`] has it: its
/// cameras, its lights, and what its clips move with draw nothing, and stay, and so does a
/// clip on a hidden track, which draws nothing itself and is there for the light or the
/// effect that reads it. Refused where it has no clip that draws which a model films.
pub(super) fn only_filmed(sequence: &mut SequencePlan, id: &Id) -> Result<()> {
    let mut any = false;
    for shown in sequence
        .tracks
        .iter_mut()
        .filter(|track| track.shows)
        .flat_map(|track| &mut track.shown)
    {
        if matches!(shown.plan.source, Source::Nothing) {
            continue;
        }
        if shown.plan.filmed {
            any = true;
        } else {
            // It shows at no time, and is in its place for the transitions that name it.
            shown.to = shown.from;
        }
    }
    if any {
        Ok(())
    } else {
        Err(ComposeError::Invalid(format!(
            "only the clips a model films are to be drawn, and sequence `{id}` has none: a clip says so with `filmed=true`"
        )))
    }
}

impl Dip {
    pub(super) fn shows(&self, at: Time) -> bool {
        self.from <= at && at < self.to
    }
}

impl CaptionPlan {
    pub(super) fn shows(&self, at: Time) -> bool {
        self.from <= at && at < self.to
    }
}
