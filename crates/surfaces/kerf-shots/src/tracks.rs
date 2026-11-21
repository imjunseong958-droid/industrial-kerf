//! What a shot hands a model, read from footage: where a thing goes
//! (`moves=<clip>`), as a clip that follows a point of the picture, and the
//! matte of a thing (`render … matte=<clip>`), as a mask that holds to it as
//! it moves and changes shape.
//!
//! A person asks for either in a shot's graph, on a picture of the clip's
//! first frame: a point for the clip to follow, or points on what to cut out.
//! An agent follows the same with `look` (`track`) and runs the line it is
//! given with `edit`. Both write the keys the one way, here.

use kerf_engine::still::Frame;
use kerf_model::{ClipAt, Project};
use kerf_track::{Followed, FollowedObject, Hints};

/// The most frames followed at a time.
pub const MOST_FRAMES: i64 = 1800;

/// How far a followed point may stray from the keyframes it is written with, in pixels of the frame
/// it was followed on: finer than that is the tracker's own jitter.
pub const POINT_TOLERANCE: f64 = 0.5;

/// How far an object's outline may leave a straight way between two keyframes, in pixels of the
/// frame it was cut in. A cut is of whole pixels, so its edge is known to half a pixel either way:
/// one that goes 3.2 pixels a frame is cut 3, 3, 3, 3 and 4 pixels on, 0.8 off a straight way, and
/// at half a pixel four frames of every five were keyframes.
pub const OUTLINE_TOLERANCE: f64 = 1.0;

/// The widest a frame is drawn to be followed on.
pub const WIDTH: f64 = 960.0;

/// The most of the frame a thing may cover for a clip to follow its middle.
pub const MOST_OF_FRAME: f64 = 0.2;

/// The name of the mask that cuts a followed object out of its clip.
pub const MASK: &str = "object";

/// A followed point as keyframes of a clip's `x` and `y`, from the middle of the frame as a
/// clip's are, with the frames kept. Frame `index` of what was followed is written as frame
/// `index + offset` of the clip, and `width` is how many pixels wide it was followed on.
pub fn point_keys(followed: &Followed, offset: i64, width: f64) -> (Vec<usize>, String, String) {
    let kept = kerf_track::simplified(&followed.points, POINT_TOLERANCE / width);
    let keys = |value: &dyn Fn(usize) -> f64| {
        let words: Vec<String> = kept
            .iter()
            .map(|index| format!("{}f:{:.5}", *index as i64 + offset, value(*index)))
            .collect();
        format!("{{{}}}", words.join(" "))
    };
    let x = keys(&|index| followed.points[index].x - 0.5);
    let y = keys(&|index| followed.points[index].y - 0.5);
    (kept, x, y)
}

/// A followed object as the keyframes of a mask's `points=`.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    /// The object, with only the corners that shape it.
    pub followed: FollowedObject,
    /// The frames of it that are keyframes.
    pub kept: Vec<usize>,
    /// The keyframes, as `mask <clip> add <name> points=` takes them.
    pub points: String,
}

/// The outline of `followed` as few corners as shape it, the same ones in every keyframe as a
/// mask's must be, and then the frames those corners need: a square followed by forty corners
/// through four seconds was 95 keyframes of forty, 48 KB of outline. `offset` and `width` are as
/// for [`point_keys`]; `pixels` is how wide the picture the mask goes on is, which how soft an
/// edge is is said in, and said only of an object that has one.
pub fn outline(followed: &FollowedObject, offset: i64, width: f64, pixels: f64) -> Outline {
    let followed = followed.with_corners(&kerf_track::simplified_corners(
        followed,
        POINT_TOLERANCE / width,
    ));
    let kept = kerf_track::simplified_object(&followed, OUTLINE_TOLERANCE / width);
    let rounded = |value: f64| (value * 10_000.0).round() / 10_000.0;
    let softly =
        |frame: usize, corner: usize| (followed.soft[frame][corner] * pixels * 10.0).round() / 10.0;
    let any_soft = followed
        .soft
        .iter()
        .flatten()
        .any(|soft| soft * pixels >= 0.05);
    let words: Vec<String> = kept
        .iter()
        .map(|index| {
            let corners: Vec<String> = followed.outlines[*index]
                .iter()
                .enumerate()
                .map(|(at, corner)| {
                    let (x, y) = (rounded(corner[0]), rounded(corner[1]));
                    if any_soft {
                        format!("{x},{y},{}", softly(*index, at))
                    } else {
                        format!("{x},{y}")
                    }
                })
                .collect();
            format!("{}f:{}", *index as i64 + offset, corners.join(";"))
        })
        .collect();
    Outline {
        points: format!("{{{}}}", words.join(" ")),
        followed,
        kept,
    }
}

/// How many pixels wide the picture of the clip at `at` is: its media's, or its sequence's.
fn picture_width(project: &Project, at: ClipAt) -> f64 {
    let own = match &project.clip_at(at).content {
        kerf_model::Content::Media { media, .. } => project
            .media(media.as_str())
            .and_then(|media| media.video.as_ref())
            .map(|video| f64::from(video.width)),
        _ => None,
    };
    own.unwrap_or(f64::from(project.sequences[at.sequence].width))
}

/// What a clip is read from footage for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum For {
    /// Where it goes: it follows a point of the picture under it. The clip itself is not drawn
    /// while the point is followed, or what was followed would be the clip.
    Moves,
    /// Its matte: a mask cuts out of it what it shows. The mask it has from an earlier cut is
    /// not on it while it is cut again, and no other clip of its sequence is drawn.
    Matte,
}

/// What was read for a clip: the line that sets it, to run as an edit of the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub script: String,
    /// How many frames it was followed through, and how many of them are keyframes.
    pub frames: usize,
    pub keyframes: usize,
    /// The frame of the clip it was lost on, where it was.
    pub lost_at: Option<usize>,
    /// Whether what a clip follows is the middle of a thing cut out, and not a point of it.
    pub as_a_thing: bool,
}

impl Read {
    /// What to tell whoever asked, beside the edit: a thing lost before the clip ends is not
    /// followed to it.
    pub fn says(&self) -> String {
        let what = if self.as_a_thing {
            "the middle of the thing there, "
        } else {
            ""
        };
        match self.lost_at {
            Some(frame) => format!(
                "{what}followed through {} frames and lost on frame {frame}: the rest of the clip is as it was left there",
                self.frames
            ),
            None => format!(
                "{what}followed through {} frames, {} of them keyframes",
                self.frames, self.keyframes
            ),
        }
    }
}

/// The frames of clip `clip` as its sequence shows them for `what`. The project is one whose
/// media say where their files are (`kerf_runtime::resolve_media_paths`), here and below.
struct Seen {
    drawer: kerf_engine::still::Compositor,
    at: ClipAt,
    /// The clip's first frame on its sequence, how many frames it runs for, and what its
    /// sequence is drawn at.
    first: i64,
    count: usize,
    scale: f64,
    rate: kerf_model::Rate,
}

impl Seen {
    fn of(project: &Project, clip: &str, what: For) -> Result<Self, String> {
        let at = project.locate(clip).ok_or_else(|| project.no_clip(clip))?;
        let mut seen = project.clone();
        let found = seen.clip_at_mut(at);
        match what {
            For::Moves => found.enabled = false,
            For::Matte => {
                found.masks.retain(|mask| mask.name.as_str() != MASK);
                // Its own picture, with nothing of the sequence over it: the mask is the clip's.
                let id = found.id.clone();
                for other in seen.sequences[at.sequence]
                    .tracks
                    .iter_mut()
                    .flat_map(|track| &mut track.clips)
                    .filter(|other| other.id != id)
                {
                    other.enabled = false;
                }
            }
        }
        let sequence = &project.sequences[at.sequence];
        let found = project.clip_at(at);
        let rate = sequence.rate;
        let frame = rate.frame_duration().flicks();
        let first = found.start.flicks().div_euclid(frame);
        let count = (found.duration.flicks().div_euclid(frame)).max(1);
        if count > MOST_FRAMES {
            return Err(format!(
                "`{clip}` runs for {count} frames, and at most {MOST_FRAMES} are followed at a time: cut it where the shot ends"
            ));
        }
        let drawer = kerf_engine::still::drawer(&seen, Some(sequence.id.as_str()))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            drawer,
            at,
            first,
            count: count as usize,
            scale: (WIDTH / f64::from(sequence.width.max(1))).min(1.0),
            rate,
        })
    }

    fn frame(&self, index: usize) -> Frame {
        self.drawer
            .frame(self.rate.time_of(self.first + index as i64), self.scale)
    }

    fn read(&self) -> impl FnMut(usize) -> Option<(usize, usize, Vec<u8>)> + '_ {
        |index| {
            let frame = self.frame(index);
            Some((frame.width as usize, frame.height as usize, frame.rgba))
        }
    }

    fn width(&self, project: &Project) -> f64 {
        f64::from(project.sequences[self.at.sequence].width.max(1)) * self.scale
    }
}

/// The picture a person points at for `what`: the sequence of `clip` on the clip's first frame,
/// as it is followed.
pub fn picture(project: &Project, clip: &str, what: For) -> Result<Frame, String> {
    Ok(Seen::of(project, clip, what)?.frame(0))
}

fn within(points: &[[f64; 2]]) -> Result<(), String> {
    if points
        .iter()
        .flatten()
        .all(|value| (0.0..=1.0).contains(value))
    {
        Ok(())
    } else {
        Err("a point runs 0 to 1 across the frame and down it".to_owned())
    }
}

/// Have `clip` follow the point of its sequence's picture that is at `point` on the clip's first
/// frame, 0 to 1 across the frame and down it, for as long as the clip runs: its `x` and `y`,
/// keyed. What reads where the clip goes (`moves=<clip>`) then reads where that point went.
pub fn moves(project: &Project, clip: &str, point: [f64; 2]) -> Result<Read, String> {
    within(&[point])?;
    let seen = Seen::of(project, clip, For::Moves)?;
    let mut followed = kerf_track::follow(seen.count, point, seen.read());
    let mut as_a_thing = false;
    // A point is followed by the detail around it, and a thing of one colour, a ball, has none:
    // where the point cannot be followed at all, the thing it is on is cut out and followed by
    // its middle. A point that was followed and then lost is kept as far as it went: what is
    // cut out there is as likely the wall behind it.
    if followed.points.len() < 2 {
        let cut = kerf_track::follow_object(
            seen.count,
            &Hints {
                inside: vec![point],
                ..Hints::default()
            },
            seen.read(),
        );
        // A floor or a wall is cut out as readily as a ball, and its middle is no place of
        // anything: only what is small in the frame is a thing that goes somewhere.
        if cut.areas.iter().all(|area| *area <= MOST_OF_FRAME) {
            followed = middles(&cut);
            as_a_thing = true;
        }
    }
    if followed.points.len() < 2 {
        return Err(format!(
            "nothing to follow at {:.3}, {:.3} on the first frame of `{clip}`: point at something with detail, such as a corner or a mark, or at a thing that stands out from what is behind it",
            point[0], point[1]
        ));
    }
    let (kept, x, y) = point_keys(&followed, 0, seen.width(project));
    Ok(Read {
        script: format!("set {clip} x={x} y={y}"),
        frames: followed.points.len(),
        keyframes: kept.len(),
        lost_at: followed.lost_at,
        as_a_thing,
    })
}

/// A followed object as a followed point: the middle of the box around it on each frame, up to
/// the frame it is somewhere else altogether on. What is cut out by colour can let go of a thing
/// and take up another of its colour, and a thing does not go further in a frame than it is
/// wide and tall together.
fn middles(object: &FollowedObject) -> Followed {
    let mut followed = Followed {
        points: Vec::new(),
        lost_at: object.lost_at,
    };
    let mut before: Option<([f64; 2], f64)> = None;
    for (index, outline) in object.outlines.iter().enumerate() {
        let over = |axis: usize| {
            outline
                .iter()
                .map(|corner| corner[axis])
                .fold((f64::MAX, f64::MIN), |(low, high), value| {
                    (low.min(value), high.max(value))
                })
        };
        let ((left, right), (top, bottom)) = (over(0), over(1));
        let middle = [(left + right) / 2.0, (top + bottom) / 2.0];
        let reach = (right - left) + (bottom - top);
        let jumped = before.is_some_and(|(was, reached)| {
            (middle[0] - was[0]).hypot(middle[1] - was[1]) > reached.max(reach)
        });
        if outline.is_empty() || jumped {
            followed.lost_at = Some(index);
            break;
        }
        before = Some((middle, reach));
        followed.points.push(kerf_track::Point {
            x: middle[0],
            y: middle[1],
            confidence: 1.0,
        });
    }
    followed
}

/// Cut out of `clip` the thing its sequence's picture shows at `inside` on the clip's first
/// frame, and not at `outside`, as a mask that holds to it for as long as the clip runs. What
/// draws the clip's matte (`render … matte=<clip>`) then draws that thing in white. It goes by
/// colour and edges; an agent can pick a trained model with `look`.
pub fn matte(
    project: &Project,
    clip: &str,
    inside: &[[f64; 2]],
    outside: &[[f64; 2]],
) -> Result<Read, String> {
    if inside.is_empty() {
        return Err("point at what to cut out: at least one point on it".to_owned());
    }
    within(inside)?;
    within(outside)?;
    let seen = Seen::of(project, clip, For::Matte)?;
    let hints = Hints {
        inside: inside.to_vec(),
        outside: outside.to_vec(),
        bbox: None,
    };
    let followed = kerf_track::follow_object(seen.count, &hints, seen.read());
    if followed.outlines.is_empty() {
        return Err(format!(
            "nothing was cut out at those points on the first frame of `{clip}`: point at the thing itself, and off it at what is taken for it"
        ));
    }
    let cut = outline(
        &followed,
        0,
        seen.width(project),
        picture_width(project, seen.at),
    );
    // The mask it has from a cut before is set anew; any other mask of the clip stays.
    let verb = if project
        .clip_at(seen.at)
        .masks
        .iter()
        .any(|mask| mask.name.as_str() == MASK)
    {
        "set"
    } else {
        "add"
    };
    Ok(Read {
        script: format!("mask {clip} {verb} {MASK} points={}", cut.points),
        frames: cut.followed.outlines.len(),
        keyframes: cut.kept.len(),
        lost_at: cut.followed.lost_at,
        as_a_thing: false,
    })
}
