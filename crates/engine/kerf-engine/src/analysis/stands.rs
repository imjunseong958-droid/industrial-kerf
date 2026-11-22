//! What each clip in depth stands on, measured in the world with nothing drawn.
//!
//! [`crate::analysis::shows`] reads a frame as an eye would: where a clip is, and what hides it. It
//! does not say that a crate hangs a hundredth of the frame over the road, or that a car's
//! far wheels are in it, which a depth pass shows a model all the same: a model draws what a
//! guide shows of a thing, and leaves out what a floor hides of it. This reads what a frame
//! is drawn from ([`kerf_compose::Compositor::in_depth`]): each clip's plane, the box of one
//! with a `depth`, a mesh's triangles where its file's animation has them. It is in frame
//! heights, down being the way `y` grows, and no pixel is read, so a sequence of any size
//! says the same.
//!
//! A clip's feet are its lowest corners. A line straight down through each meets the
//! surfaces of the other clips, and how far off the first of them is says how the clip
//! stands: on it, over it, in it, or with nothing under it. A clip is on what the first of
//! its feet to come down touches. So one whose foot is level, on a floor that leans, rests
//! on the side the floor is highest under and is open under the other, which is said with
//! the lean. One that comes down on two clips at once, an axle across two wheels, is on the
//! one of the lower track: which of two as near is the nearer is in the last places of
//! their numbers, and is the other on the next frame of a thing that moves. A surface that
//! goes through a clip is one it is sunk in; one over its top is a roof, or something that
//! stands on it, but for the ground, the largest clip with nothing under it, which a clip
//! wholly under is sunk in however deep. What stands on a clip may reach a little way into
//! it, a ball set on a ball, a roof on its walls: the underside of that is inside the lower
//! clip and is no floor through it, so of a thing whose surface closes round it the side a
//! line comes out by is not one to stand on or be sunk in ([`comes_to`]). And an edge of a
//! clip that goes through another's surface, a box through a wall, is said of the two.
//!
//! What is said is what is so, and not that it is wrong: a ball thrown is in the air. The
//! line that sets a clip down is its own `y`, which the one who asked runs or does not.

use crate::error::Result;
use kerf_compose::{ComposeOptions, Compositor, Form, InDepth, World};
use kerf_model::{Catalog, Project, Time};
use rayon::prelude::*;

/// How near a foot is to a surface to be on it, in frame heights: a pixel and a half of a
/// guide 480 high, six times what a `y` written to [`PLACES`] can be off by, and more than
/// the rule a floor was laid by before this could be asked is off by where it was given
/// (`a_touch_is_more_than_a_written_y_can_be_off_by`).
pub const TOUCH: f64 = 0.003;

/// How many places after the point a line's `y` is written to, where no parent scales it.
pub const PLACES: i32 = 3;

/// The most frames of a sequence read: a shot a model makes, and some more. Of a longer
/// one, one frame in every few is.
pub const MOST_FRAMES: usize = 256;

/// How much of a clip's height its feet are within, from its lowest corner up.
const FOOT: f64 = 0.02;

/// The most feet read of a clip. Of more, the lowest in each part of its foot is, this many
/// parts across and as many back.
const MOST_FEET: usize = 256;
const ACROSS: usize = 16;

/// The share of its feet under a surface from which a clip is in it. Fewer are a corner of
/// a mesh that dips, and are passed over where the rest of it stands on something.
const IN: f64 = 0.1;

/// How far toward its clip's middle a foot is moved to ask whether it is inside another
/// clip: a line down the very edge of a box set on a box as wide, or through the very top
/// of a ball, runs along surfaces and through corners, where what it meets is in the last
/// places of its numbers. This is a twenty-thousandth of a pixel of a guide 480 high.
const INWARD: f64 = 1e-7;

/// How far past a surface a line is taken up again to find the next one: over what the
/// last places of a number come to, and under anything a thing is thick by.
const PAST: f64 = 1e-9;

/// The most surfaces of one clip a line is followed through: a mesh folded on itself more
/// often than this is read as far as that.
const MOST_CROSSED: usize = 64;

/// The least a clip looks under itself for something to stand on, where it is flatter than
/// this: past its own height, or this, it is in the air.
const LEAST_REACH: f64 = 0.05;

/// The lean of a surface from which it is worth saying, in degrees.
pub const LEANS: f64 = 0.5;

/// How near its lowest a foot is to be level with it. A ball's feet are not, by its own
/// shape, and what a leaning floor leaves open is said of a foot that is flat.
const LEVEL: f64 = TOUCH / 10.0;

/// From how far over a foot the ground is looked for: further than any scene goes.
const FAR: f64 = 1e3;

const DOWN: [f64; 3] = [0.0, 1.0, 0.0];

/// How much nearer a surface has to be than another to be the nearer: two as far off but
/// for the last places of their numbers are as near, and a clip is on the lower track's.
/// It is a thousandth of the least a line's `y` can put two surfaces apart by, so those
/// written apart are apart, and millions of times what the last places come to.
const NEARER: f64 = 1e-6;

/// How much further off a frame has to be to be a run's worst: what holds still is as far
/// off on every frame, but for the last places of its numbers, and its first frame is said.
const WORSE: f64 = 1e-6;

/// How a clip stands on a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum How {
    /// Its lowest foot is on a surface, and none of its feet is in one.
    Rests,
    /// All of its feet are over a surface: by how far it would come down to rest.
    Floats(f64),
    /// Some of its feet are under a surface it goes through: how far the deepest is, and
    /// the share of them that are.
    Sunk { deep: f64, share: f64 },
    /// Nothing is under it within its own height.
    Air,
    /// Nothing is under it at all, and it is the largest such: what the rest stands on.
    Ground,
}

impl How {
    fn kind(&self) -> u8 {
        match self {
            Self::Rests => 0,
            Self::Floats(_) => 1,
            Self::Sunk { .. } => 2,
            Self::Air => 3,
            Self::Ground => 4,
        }
    }

    /// How far it is from resting, to say which frame of a run is the worst.
    fn off(&self) -> f64 {
        match self {
            Self::Floats(by) => *by,
            Self::Sunk { deep, .. } => *deep,
            _ => 0.0,
        }
    }
}

/// A run of the frames read on which a clip stands one way on one thing.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// As on its worst frame.
    pub how: How,
    /// The clip under its feet: the one its lowest foot is on, or would come down on, and of
    /// two as near the one of the lower track.
    pub on: Option<String>,
    /// How far from level the surface under it leans, in degrees, at the most.
    pub slope: f64,
    /// The most room under those of its feet that are level with its lowest, once that one
    /// is down: what a surface that leans leaves open under a clip that does not.
    pub open: f64,
    /// For a clip in the air, how far up it is from what is under it, at the least.
    pub up: Option<f64>,
    /// The first and the last frame of it, from the sequence's first as 0.
    pub frames: (i64, i64),
    /// The frame it is furthest from resting on.
    pub worst: i64,
}

/// A run of the frames read on which a clip goes through another.
#[derive(Clone, Debug, PartialEq)]
pub struct Through {
    /// The clip whose surface one of its edges goes through.
    pub clip: String,
    /// How far past that surface the end of that edge is, on the side it has less of.
    pub deep: f64,
    /// The share of its corners that are on that side.
    pub share: f64,
    pub frames: (i64, i64),
    /// The frame it is deepest on.
    pub worst: i64,
}

/// How one clip stands over the frames read.
#[derive(Clone, Debug, PartialEq)]
pub struct Stood {
    pub clip: String,
    pub runs: Vec<Run>,
    pub through: Vec<Through>,
    /// The line that sets it down where it floats or is sunk: its own `y`, as one number or
    /// as the fewest keys that keep it down on every frame read.
    pub script: Option<String>,
    /// Why no line does, where one is wanted.
    pub cannot: Option<String>,
}

/// How the clips in depth of a sequence stand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Standing {
    /// The frames read, from the sequence's first as 0.
    pub frames: Vec<i64>,
    /// One frame in this many was read: 1 but for a sequence longer than [`MOST_FRAMES`].
    pub every: usize,
    /// What the rest stands on, on most of the frames read.
    pub ground: Option<String>,
    /// From the lowest track up.
    pub clips: Vec<Stood>,
    /// What of this scene is not measured, said so that it is not read as being right.
    pub not_measured: Vec<String>,
}

/// One clip on one frame.
#[derive(Clone, Debug)]
struct Stand {
    how: How,
    on: Option<String>,
    slope: f64,
    open: f64,
    /// How far down it has to go to rest, up being under 0, where something is under it.
    drop: Option<f64>,
    fall: Option<f64>,
    through: Vec<(String, f64, f64)>,
}

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A clip with the box around it and its feet.
struct Body<'a> {
    of: &'a InDepth,
    least: [f64; 3],
    most: [f64; 3],
    feet: Vec<[f64; 3]>,
    /// The middle of its feet, where that is a place of it: of a thing that bulges nowhere
    /// inward, a box, what is between its lowest corners is its underside.
    middle: Option<[f64; 3]>,
}

/// Where a line straight down from `from` first comes to a surface of `other` that is one
/// to stand on or be sunk in: how far along, and the way it faces.
///
/// A flat clip's surface is one, from either side, and so is each surface of a mesh that
/// is open, a set rebuilt from a shot. A thing whose surface closes round it has an inside,
/// and a line that starts in it comes out by its underside, which nothing stands on: a ball
/// set on another and a little way into it has its underside in the lower one, and the
/// lower one is not sunk in it. So where `from` is inside such a thing, the surface the
/// line leaves it by is passed, and what it comes to is the next. `inward` is `from` moved
/// a hair toward the middle of the clip whose foot it is over, by which inside is asked:
/// what the line itself meets is told as it always was.
fn comes_to(from: [f64; 3], inward: [f64; 3], other: &Body) -> Option<(f64, [f64; 3])> {
    let first = other.of.meets(from, DOWN, f64::INFINITY)?;
    // A line from over all of it goes in by the first surface it meets and out by the
    // next, so one that has met an odd number of them by `from` is inside it there.
    let inside = other.of.closed && other.least[1] < from[1] && {
        let mut at = [inward[0], other.least[1] - 1.0, inward[2]];
        let mut met = 0;
        while met < MOST_CROSSED {
            let Some((along, _)) = other.of.meets(at, DOWN, from[1] - at[1]) else {
                break;
            };
            met += 1;
            at[1] += along + PAST;
        }
        met % 2 == 1
    };
    if !inside {
        return Some(first);
    }
    let past = first.0 + PAST;
    let (along, facing) =
        other
            .of
            .meets([from[0], from[1] + past, from[2]], DOWN, f64::INFINITY)?;
    Some((past + along, facing))
}

impl<'a> Body<'a> {
    fn new(of: &'a InDepth) -> Option<Self> {
        let (mut least, mut most) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for corner in &of.corners {
            for axis in 0..3 {
                least[axis] = least[axis].min(corner[axis]);
                most[axis] = most[axis].max(corner[axis]);
            }
        }
        if !(0..3).all(|axis| least[axis].is_finite() && most[axis].is_finite()) {
            return None;
        }
        let mut body = Self {
            of,
            least,
            most,
            feet: Vec::new(),
            middle: None,
        };
        body.feet = body.find_feet();
        if of.convex && body.feet.len() > 1 {
            let count = body.feet.len() as f64;
            body.middle = Some(
                [0, 1, 2].map(|axis| body.feet.iter().map(|foot| foot[axis]).sum::<f64>() / count),
            );
        }
        Some(body)
    }

    fn height(&self) -> f64 {
        self.most[1] - self.least[1]
    }

    /// Its lowest corners, each once: all of them, or of a mesh that has thousands there the
    /// lowest in each part of its foot, which spread over it say where it stands.
    fn find_feet(&self) -> Vec<[f64; 3]> {
        let from = self.most[1] - (self.height() * FOOT).max(TOUCH);
        let mut low: Vec<[f64; 3]> = self
            .of
            .corners
            .iter()
            .copied()
            .filter(|corner| corner[1] >= from)
            .collect();
        // A corner is told again for each side it is a corner of.
        low.sort_by(|a, b| {
            a[0].total_cmp(&b[0])
                .then(a[2].total_cmp(&b[2]))
                .then(a[1].total_cmp(&b[1]))
        });
        low.dedup();
        if low.len() <= MOST_FEET {
            return low;
        }
        let (mut least, mut most) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for corner in &low {
            for (axis, of) in [0, 2].into_iter().enumerate() {
                least[axis] = least[axis].min(corner[of]);
                most[axis] = most[axis].max(corner[of]);
            }
        }
        let mut kept: Vec<Option<[f64; 3]>> = vec![None; ACROSS * ACROSS];
        for corner in low {
            let place = |axis: usize, of: usize| {
                let wide = most[axis] - least[axis];
                if wide <= 0.0 {
                    return 0;
                }
                (((corner[of] - least[axis]) / wide * ACROSS as f64) as usize).min(ACROSS - 1)
            };
            let cell = &mut kept[place(1, 2) * ACROSS + place(0, 0)];
            if cell.is_none_or(|lowest| corner[1] > lowest[1]) {
                *cell = Some(corner);
            }
        }
        kept.into_iter().flatten().collect()
    }

    fn area(&self) -> f64 {
        (self.most[0] - self.least[0]) * (self.most[2] - self.least[2])
    }

    /// Whether the boxes around the two are in each other by more than a touch.
    fn near(&self, other: &Body) -> bool {
        (0..3).all(|axis| {
            self.least[axis] < other.most[axis] - TOUCH
                && other.least[axis] < self.most[axis] - TOUCH
        })
    }
}

/// What is read of one frame.
struct Read {
    /// How each clip of its world stands, in the world's order.
    stands: Vec<Option<Stand>>,
    /// The pairs of meshes near enough to be in each other, which is not measured.
    meshes: Vec<(String, String)>,
}

/// How each clip of `world` stands.
fn read(world: &World) -> Read {
    let bodies: Vec<Option<Body>> = world.clips.iter().map(Body::new).collect();
    let all = |ground: Option<usize>| -> Vec<Option<Stand>> {
        bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                body.as_ref()
                    .map(|body| stand(index, body, &bodies, ground))
            })
            .collect()
    };
    let mut stands = all(None);
    // What nothing is under is in the air, but for the largest such, which is the ground.
    let ground = bodies
        .iter()
        .enumerate()
        .filter_map(|(index, body)| Some((index, body.as_ref()?)))
        .filter(|(index, _)| {
            stands[*index]
                .as_ref()
                .is_some_and(|stand| stand.how == How::Air && stand.on.is_none())
        })
        .max_by(|(_, a), (_, b)| a.area().total_cmp(&b.area()))
        .map(|(index, _)| index);
    // Under the ground is in it, however deep: read again, now that it is known which it is.
    if ground.is_some_and(|ground| {
        bodies[ground]
            .as_ref()
            .is_some_and(|body| body.of.form != Form::Mesh)
    }) {
        stands = all(ground);
    }
    if let Some(stand) = ground.and_then(|ground| stands[ground].as_mut()) {
        *stand = Stand {
            how: How::Ground,
            on: None,
            slope: 0.0,
            open: 0.0,
            drop: None,
            fall: stand.fall,
            through: Vec::new(),
        };
    }
    let mut meshes = Vec::new();
    for (index, body) in bodies.iter().enumerate() {
        let Some(body) = body else {
            continue;
        };
        let mut edges = None;
        for (other_index, other) in bodies.iter().enumerate() {
            let Some(other) = other.as_ref().filter(|_| other_index != index) else {
                continue;
            };
            if !body.near(other) {
                continue;
            }
            // What a clip stands on or is sunk in is said of its feet.
            let stands_on = |a: usize, b: &Body| {
                stands[a]
                    .as_ref()
                    .is_some_and(|stand| stand.on.as_deref() == Some(b.of.clip.as_str()))
            };
            if stands_on(index, other) || stands_on(other_index, body) {
                continue;
            }
            if body.of.form == Form::Mesh && other.of.form == Form::Mesh {
                if index < other_index {
                    meshes.push((body.of.clip.clone(), other.of.clip.clone()));
                }
                continue;
            }
            let edges = edges.get_or_insert_with(|| body.of.edges());
            if let Some((deep, share)) = through(body, edges, other)
                && let Some(stand) = stands[index].as_mut()
            {
                stand.through.push((other.of.clip.clone(), deep, share));
            }
        }
    }
    Read { stands, meshes }
}

/// How `body`, the one at `index` of `bodies`, stands on the others, `ground` being the one
/// of them that is the ground, where that is known and is no mesh.
///
/// It stands by its feet, its lowest corners. A thing that bulges nowhere inward has its
/// underside between them, and what is under the middle of that is under the thing as
/// much as what is under a corner: a roof on walls it is wider than has its corners over
/// the floor and its middle on the walls. So a thing whose corners are down on nothing,
/// over a floor or in the air, is read with that one foot more where its middle comes
/// down on something first. One whose corners are down is down: what is under its middle
/// then goes through it, which is said of its edges.
fn stand(index: usize, body: &Body, bodies: &[Option<Body>], ground: Option<usize>) -> Stand {
    let by_corners = stand_by(index, body, &body.feet, bodies, ground);
    let (Some(middle), How::Air | How::Floats(_)) = (body.middle, by_corners.how) else {
        return by_corners;
    };
    let mut feet = body.feet.clone();
    feet.push(middle);
    let by_all = stand_by(index, body, &feet, bodies, ground);
    match (by_corners.drop, by_all.drop) {
        (None, Some(_)) => by_all,
        (Some(corners), Some(all)) if all < corners - TOUCH => by_all,
        _ => by_corners,
    }
}

/// How `body` stands by `feet`, places of its underside.
fn stand_by(
    index: usize,
    body: &Body,
    feet: &[[f64; 3]],
    bodies: &[Option<Body>],
    ground: Option<usize>,
) -> Stand {
    let height = body.height();
    // From as high as the clip is: a surface that goes through it is one it is sunk in, and
    // one over its top is a roof, or something that stands on it. The ground is no roof, so
    // a clip wholly under it is sunk in it; a mesh that is the ground may have one.
    let lift = (height - TOUCH).max(TOUCH);
    // Each foot with something under it: how far off that is, whose it is, and how it faces.
    let mut under: Vec<(usize, f64, usize, [f64; 3])> = Vec::new();
    let middle = [0, 2].map(|axis| (body.least[axis] + body.most[axis]) / 2.0);
    for (foot_index, foot) in feet.iter().enumerate() {
        let mut first: Option<(f64, usize, [f64; 3])> = None;
        // The foot a hair toward its clip's middle, to ask what it is inside of.
        let toward = [middle[0] - foot[0], middle[1] - foot[2]];
        let far = toward[0].hypot(toward[1]);
        let hair = if far > INWARD { INWARD / far } else { 0.0 };
        for (other_index, other) in bodies.iter().enumerate() {
            let Some(other) = other.as_ref().filter(|_| other_index != index) else {
                continue;
            };
            let lift = if ground == Some(other_index) {
                FAR
            } else {
                lift
            };
            let from = [foot[0], foot[1] - lift, foot[2]];
            let beside = [0, 2]
                .into_iter()
                .any(|axis| foot[axis] < other.least[axis] || foot[axis] > other.most[axis]);
            if beside || other.most[1] < from[1] {
                continue;
            }
            let inward = [
                from[0] + toward[0] * hair,
                from[1],
                from[2] + toward[1] * hair,
            ];
            if let Some((along, facing)) = comes_to(from, inward, other) {
                let gap = along - lift;
                // The others are asked from the lowest track up, and one as near is no nearer.
                if first.is_none_or(|(least, ..)| gap < least - NEARER) {
                    first = Some((gap, other_index, facing));
                }
            }
        }
        if let Some((gap, other, facing)) = first {
            under.push((foot_index, gap, other, facing));
        }
    }
    let within = under.iter().filter(|(_, gap, ..)| *gap < -TOUCH).count();
    let share = within as f64 / feet.len().max(1) as f64;
    // A few feet in a surface are a corner that dips, where the others are on something or
    // over it. Where none is, they are all it stands by: a brim on a head it is wider than,
    // a table's top on its one leg.
    let reach = height.max(LEAST_REACH);
    let stands_by_others = under
        .iter()
        .any(|(_, gap, ..)| (-TOUCH..=reach).contains(gap));
    let sunk = within > 0 && (share >= IN || !stands_by_others);
    // The feet it stands by: all of them where it is sunk, and else those in no surface.
    let stood_by = |gap: f64| sunk || gap >= -TOUCH;
    // How far off the foot that is down first is, or with the clip sunk the one that is deepest.
    let drop = under
        .iter()
        .map(|(_, gap, ..)| *gap)
        .filter(|gap| stood_by(*gap))
        .min_by(f64::total_cmp);
    // What is under that foot: of feet as near, the first over the lowest track's clip.
    let first = drop.and_then(|drop| {
        under
            .iter()
            .filter(|(_, gap, ..)| stood_by(*gap) && *gap <= drop + NEARER)
            .min_by_key(|(_, _, other, _)| *other)
    });
    let (Some(drop), Some(&(_, _, on, facing))) = (drop, first) else {
        return Stand {
            how: How::Air,
            on: None,
            slope: 0.0,
            open: 0.0,
            drop: None,
            fall: body.of.fall,
            through: Vec::new(),
        };
    };
    let how = if sunk {
        How::Sunk { deep: -drop, share }
    } else if drop <= TOUCH {
        How::Rests
    } else if drop <= reach {
        How::Floats(drop)
    } else {
        How::Air
    };
    // Of its feet level with its lowest, over the same clip: the one with the most under it.
    let open = under
        .iter()
        .filter(|(foot, _, other, _)| *other == on && feet[*foot][1] >= body.most[1] - LEVEL)
        .map(|(_, gap, ..)| gap - drop)
        .fold(0.0, f64::max);
    Stand {
        how,
        on: bodies[on].as_ref().map(|other| other.of.clip.clone()),
        slope: facing[1].abs().min(1.0).acos().to_degrees(),
        open,
        drop: Some(drop),
        fall: body.of.fall,
        through: Vec::new(),
    }
}

/// How far `body` goes through `other`, where one of its `edges` goes through the other's
/// surface: how far past it the nearer end of that edge is, and the share of the body's
/// corners on that side of it.
fn through(body: &Body, edges: &[[[f64; 3]; 2]], other: &Body) -> Option<(f64, f64)> {
    // The deepest: how far, a point of the surface there, how it faces, and which side.
    let mut deepest: Option<(f64, [f64; 3], [f64; 3], f64)> = None;
    for [from, to] in edges {
        let along = minus(*to, *from);
        let length = dot(along, along).sqrt();
        if length <= 2.0 * TOUCH {
            continue;
        }
        let way = along.map(|side| side / length);
        let Some((met, facing)) = other.of.meets(*from, way, length) else {
            continue;
        };
        // How far each end is from the surface, straight off it.
        let lean = dot(way, facing);
        let (before, after) = (met * lean.abs(), (length - met) * lean.abs());
        let deep = before.min(after);
        if deep <= TOUCH || deepest.is_some_and(|(most, ..)| deep <= most) {
            continue;
        }
        let at = [0, 1, 2].map(|axis| from[axis] + way[axis] * met);
        // The end it has less of is before the surface, or past it.
        let side = if before <= after { -lean } else { lean };
        deepest = Some((deep, at, facing, side.signum()));
    }
    let (deep, at, facing, side) = deepest?;
    let past = body
        .of
        .corners
        .iter()
        .filter(|corner| dot(minus(**corner, at), facing) * side > TOUCH)
        .count();
    Some((deep, past as f64 / body.of.corners.len().max(1) as f64))
}

/// A number as a line writes it, to `places`: no zeros after it, and no `-0`.
fn plain(number: f64, places: i32) -> String {
    let by = 10f64.powi(places);
    let rounded = (number * by).round() / by + 0.0;
    let text = format!("{rounded:.*}", places.max(0) as usize);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// How the clips in depth of `sequence` (the first when `None`) of `project` stand: on the
/// frames at the sequence times `at`, or with none on every frame of it, one in every few
/// of more than [`MOST_FRAMES`]. A mesh's file is read from where the project's paths say,
/// so resolve them first.
pub fn standing(project: &Project, sequence: Option<&str>, at: &[Time]) -> Result<Standing> {
    let target = crate::program::program_sequence(project, sequence)?;
    let compositor = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(target.id.to_string()),
            ..ComposeOptions::default()
        },
    )?;
    let rate = target.rate;
    let count = rate.frames_round(target.duration()).max(0) as usize;
    let (frames, every): (Vec<i64>, usize) = if at.is_empty() {
        let every = count.div_ceil(MOST_FRAMES).max(1);
        ((0..count as i64).step_by(every).collect(), every)
    } else {
        (at.iter().map(|time| rate.frames_floor(*time)).collect(), 1)
    };
    let worlds: Vec<World> = frames
        .par_iter()
        .map(|frame| compositor.in_depth(rate.time_of(*frame)))
        .collect();
    let read: Vec<Read> = worlds.par_iter().map(read).collect();

    // Each clip by its place in the sequence, with how it stands on each frame it is on.
    let order: Vec<&kerf_model::Clip> = target
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .collect();
    let mut by_clip: Vec<Vec<(i64, &Stand)>> = vec![Vec::new(); order.len()];
    let mut grounds: Vec<(String, usize)> = Vec::new();
    for ((frame, world), read) in frames.iter().zip(&worlds).zip(&read) {
        for (clip, stand) in world.clips.iter().zip(&read.stands) {
            let (Some(stand), Some(place)) = (
                stand,
                order.iter().position(|told| told.id.as_str() == clip.clip),
            ) else {
                continue;
            };
            by_clip[place].push((*frame, stand));
            if stand.how == How::Ground {
                match grounds.iter_mut().find(|(told, _)| *told == clip.clip) {
                    Some((_, frames)) => *frames += 1,
                    None => grounds.push((clip.clip.clone(), 1)),
                }
            }
        }
    }
    let evenly = at.is_empty();
    let clips = order
        .iter()
        .zip(&by_clip)
        .filter(|(_, stands)| !stands.is_empty())
        .map(|(clip, stands)| {
            let (script, cannot) = set_down(clip, rate, stands, evenly);
            Stood {
                clip: clip.id.to_string(),
                runs: runs(stands),
                through: throughs(stands),
                script,
                cannot,
            }
        })
        .collect();

    let mut not_measured = Vec::new();
    let named = |clips: &mut dyn Iterator<Item = &String>| {
        let mut names: Vec<&String> = Vec::new();
        for clip in clips {
            if !names.contains(&clip) {
                names.push(clip);
            }
        }
        names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut pairs: Vec<&(String, String)> = Vec::new();
    for pair in read.iter().flat_map(|read| &read.meshes) {
        if !pairs.contains(&pair) {
            pairs.push(pair);
        }
    }
    for (one, other) in pairs {
        not_measured.push(format!(
            "whether `{one}` and `{other}` go into each other: a mesh into another mesh"
        ));
    }
    let within = named(
        &mut worlds
            .iter()
            .flat_map(|world| &world.clips)
            .filter(|clip| clip.form != Form::Mesh && !clip.filled)
            .map(|clip| &clip.clip),
    );
    if !within.is_empty() {
        not_measured.push(format!(
            "the outline of {within} within the box around it: it is measured as that box"
        ));
    }
    let unread = named(&mut worlds.iter().flat_map(|world| &world.unread));
    if !unread.is_empty() {
        not_measured.push(format!(
            "where {unread} stands: its outline is its picture's own, as a title's, a nested sequence's, a masked clip's, and that of media with alpha are"
        ));
    }
    let flat = named(&mut worlds.iter().flat_map(|world| &world.flat));
    if !flat.is_empty() {
        not_measured.push(format!("{flat}, out of depth: it is nowhere in the scene"));
    }
    grounds.sort_by_key(|(_, frames)| std::cmp::Reverse(*frames));
    Ok(Standing {
        frames,
        every,
        ground: grounds.into_iter().next().map(|(clip, _)| clip),
        clips,
        not_measured,
    })
}

/// The runs of frames on which a clip stands one way on one thing.
fn runs(stands: &[(i64, &Stand)]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (frame, stand) in stands {
        let up = (stand.how == How::Air).then_some(stand.drop).flatten();
        match runs
            .last_mut()
            .filter(|run| run.how.kind() == stand.how.kind() && run.on == stand.on)
        {
            Some(run) => {
                run.frames.1 = *frame;
                run.slope = run.slope.max(stand.slope);
                run.open = run.open.max(stand.open);
                run.up = match (run.up, up) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                if stand.how.off() > run.how.off() + WORSE {
                    run.how = stand.how;
                    run.worst = *frame;
                }
            }
            None => runs.push(Run {
                how: stand.how,
                on: stand.on.clone(),
                slope: stand.slope,
                open: stand.open,
                up,
                frames: (*frame, *frame),
                worst: *frame,
            }),
        }
    }
    runs
}

/// The runs of frames on which a clip goes through each other clip.
fn throughs(stands: &[(i64, &Stand)]) -> Vec<Through> {
    let mut found: Vec<Through> = Vec::new();
    // The runs still going on the frame before, by their place in `found`.
    let mut going: Vec<usize> = Vec::new();
    for (frame, stand) in stands {
        let mut still = Vec::new();
        for (clip, deep, share) in &stand.through {
            match going.iter().find(|index| found[**index].clip == *clip) {
                Some(index) => {
                    let run = &mut found[*index];
                    run.frames.1 = *frame;
                    if *deep > run.deep + WORSE {
                        (run.deep, run.share, run.worst) = (*deep, *share, *frame);
                    }
                    still.push(*index);
                }
                None => {
                    found.push(Through {
                        clip: clip.clone(),
                        deep: *deep,
                        share: *share,
                        frames: (*frame, *frame),
                        worst: *frame,
                    });
                    still.push(found.len() - 1);
                }
            }
        }
        going = still;
    }
    found
}

/// The line that sets `clip` down on the frames it floats or is sunk on, or why none does.
/// `evenly` says the frames read are a frame, or as many each time, apart.
///
/// A clip that is in the air on some frame has left the ground on purpose, a ball thrown or
/// a jump: it is brought up where it is sunk and left where it floats, on its way up or
/// down. One that is never in the air is set down on every frame.
fn set_down(
    clip: &kerf_model::Clip,
    rate: kerf_model::Rate,
    stands: &[(i64, &Stand)],
    evenly: bool,
) -> (Option<String>, Option<String>) {
    let leaves = stands.iter().any(|(_, stand)| stand.how == How::Air);
    let wanted = |stand: &Stand| match stand.how {
        How::Sunk { .. } => true,
        How::Floats(_) => !leaves,
        _ => false,
    };
    if !stands.iter().any(|(_, stand)| wanted(stand)) {
        return (None, None);
    }
    if stands
        .iter()
        .any(|(_, stand)| wanted(stand) && stand.fall.is_none())
    {
        return (
            None,
            Some("a parent turns it, so its own `y` moves it across or back as well as down: set it down by its parent, or by its own `x`, `y`, and `z` together".to_owned()),
        );
    }
    let own = clip.props.get("y");
    let now = |frame: i64| {
        own.and_then(|own| own.value_at(rate.time_of(frame) - clip.start).as_number())
            .unwrap_or(0.0)
    };
    // Where its own `y` would be for it to rest, on each frame something is under it
    // within its reach, and where it is on the others.
    let mut most_fall = 1.0f64;
    let rests: Vec<f64> = stands
        .iter()
        .map(|(frame, stand)| match (stand.drop, stand.fall, stand.how) {
            (_, _, How::Air | How::Ground) | (None, ..) | (_, None, _) => now(*frame),
            (_, _, How::Floats(_) | How::Rests) if leaves => now(*frame),
            (Some(drop), Some(fall), _) => {
                most_fall = most_fall.max(fall.abs());
                now(*frame) + drop / fall
            }
        })
        .collect();
    // To as many places as leave it within a quarter of a touch, however a parent scales it.
    let places = PLACES + most_fall.log10().ceil().max(0.0) as i32;
    let near = TOUCH / 4.0 / most_fall;
    if rests.iter().all(|rest| (rest - rests[0]).abs() <= near) {
        // One number, the one that holds on the frame it is furthest off on.
        let worst = stands
            .iter()
            .zip(&rests)
            .max_by(|a, b| a.0 .1.how.off().total_cmp(&b.0 .1.how.off()))
            .map_or(rests[0], |(_, rest)| *rest);
        return (
            Some(format!("set {} y={}", clip.id, plain(worst, places))),
            None,
        );
    }
    if !evenly {
        return (
            None,
            Some("what sets it down is another `y` on each frame read: read the sequence with no `at`, for the keys that keep it down on every frame".to_owned()),
        );
    }
    // The fewest keys whose lines come within that of every frame's.
    let points: Vec<kerf_track::Point> = rests
        .iter()
        .map(|rest| kerf_track::Point {
            x: *rest,
            y: 0.0,
            confidence: 1.0,
        })
        .collect();
    let first = rate.frames_round(clip.start);
    let keys: Vec<String> = kerf_track::simplified(&points, near)
        .into_iter()
        .map(|index| {
            format!(
                "{}f:{}",
                stands[index].0 - first,
                plain(rests[index], places)
            )
        })
        .collect();
    (
        Some(format!("set {} y={{{}}}", clip.id, keys.join(" "))),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_is_more_than_a_written_y_can_be_off_by() {
        // A line's `y` is written to three places, so it is within half a thousandth of what
        // was meant, and a clip set down by one has to read as down, with room for the keys
        // of one that moves to be a quarter of a touch off as well: a touch is six times
        // that half thousandth.
        let written = 0.5 * 10f64.powi(-PLACES);
        assert!(
            TOUCH >= 4.0 * written && written + TOUCH / 4.0 < TOUCH,
            "{TOUCH} {written}"
        );
        // What is said to float can be seen to: a touch is over a pixel of the smallest
        // guide a model here is handed, 480 high, and under two.
        assert!((1.0..2.0).contains(&(TOUCH * 480.0)));
        // The skill laid a floor tipped back 6 degrees and said it is 0.1 lower for each
        // frame height nearer, 1.0 where `z` is 1.6. It is 0.1051 lower, so 1.0022 there,
        // and a foot put down by that rule is on the floor.
        let floor = 0.75 + (4.0 - 1.6) * 6f64.to_radians().tan();
        assert!(
            (floor - 1.0).abs() < TOUCH && (floor - 1.0).abs() > 0.002,
            "{floor}"
        );
        assert_eq!(plain(0.41249, PLACES), "0.412");
        assert_eq!(plain(-0.0001, PLACES), "0");
        assert_eq!(plain(1.0, PLACES), "1");
        assert_eq!(plain(-0.0507, PLACES), "-0.051");
    }
}
