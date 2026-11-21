//! Where the eye is either side of a cut: the thing it was on when one shot ends, and the
//! thing it is given when the next starts.
//!
//! A shot a model makes is made by itself. Where its subject is on the frame as it ends,
//! how large, and which way it was going are nobody's, and a cut puts that beside another
//! shot's. The sound of a cut is read from what analysis kept (`look` with `cuts`); this is
//! its picture.
//!
//! The document has what it takes. A shot's media says how it is made: a `render` step
//! draws its guide from a sequence, and a `generate` step names the clip of that sequence
//! that is its subject (`moves=`). So a cut between two shots is read from their guides,
//! for what drawing a small frame costs and before either is made. Two readings are given
//! side by side:
//!
//! - the **guide**'s: the box around the subject in the guide's ids pass
//!   ([`kerf_compose::Pass::Ids`]), as `look` with `shows` reads a clip. It is what was
//!   asked for.
//! - the **take**'s, where the shot has a file: the subject found in the take's own
//!   picture where the guide has it, as [`crate::placed`] finds it, and in the guide's own
//!   picture the same way. How far apart those two are is how far the take is from its
//!   guide, with what a tracker's box adds to an ids pass's in neither, and the take's box
//!   is the guide's moved by that.
//!
//! A box gets from a guide to the program's frame by the way the frame it is on does. The
//! frame of its media a clip shows at the cut is the compositor's to say, its trim and its
//! speed being in it; that frame's time is a time of the guide's sequence, so a guide at
//! another rate than its sequence, or than the program, is read at the right moment. The
//! guide's frame is the take's picture where the two are one shape, and where they are not
//! the picture fills the take's size and what is over is cut off evenly, as a `render`
//! step draws it. From the take's picture to the program's frame is the placing the
//! compositor draws the clip by ([`kerf_compose::Compositor::lands`]).
//!
//! What that does not carry is said and not guessed: a clip in depth, a mask, an effect
//! that may move a picture, a crop that cuts the subject.
//!
//! Nothing here is a verdict. A hard cut jumps on purpose, and these are the numbers it
//! jumps by.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::error::Result;
use crate::placed::{around, box_of, looked_for};
use kerf_compose::{ComposeOptions, Compositor, Frame, Landing, Pass};
use kerf_model::making::Step;
use kerf_model::{Catalog, Clip, Content, Media, Project, Rate, Time, TrackKind};

/// The height of the frames read, in pixels, where a guide's sequence is as tall: what
/// [`crate::placed`] reads, a subject being a small part of a frame.
const HEIGHT: f64 = 360.0;

/// A cut whose subject jumps less than this, in frame heights, is counted as held.
///
/// A picture 1080 high is made to be seen from three times its height, where a tenth of
/// that height is 1.9 degrees: about what an eye sees sharply without moving. Nearer than
/// that the eye is still on its subject after the cut; further, it has to go and find it.
/// It is 36 times what a box's middle is read to.
pub const HELD: f64 = 0.1;

/// How long either side of a cut the way a subject goes is read over, in seconds.
pub const WINDOW: f64 = 0.25;

/// A subject going across slower than this, in frame heights a second, is still.
///
/// The middle of a box is read to a pixel of a frame 360 high, so over a quarter of a
/// second its speed is known to 0.011 of a frame height a second. This is nine times
/// that, and what goes this slowly takes ten seconds to cross a frame's height.
pub const STILL: f64 = 0.1;

/// The most cuts read at once.
pub const MOST_CUTS: usize = 64;

/// What is found in a take under this much of the size the same finding gives in the
/// guide's own picture, by its sides, is not taken for the subject. A take is looked at
/// within the guide's box and half of it again each way, so nothing found is over twice
/// that size; this is as far the other way and a little more.
const LEAST: f64 = 0.4;

/// The categories of the catalog whose effects leave a picture where it is and hide none
/// of it: a grade, a blur, grain, a finish, a burn-in. Any other may move it or cut it.
const IN_PLACE: [&str; 6] = [
    "Color Correction",
    "Blur & Sharpen",
    "Channel",
    "Noise & Grain",
    "Stylize",
    "Text",
];

/// Where a subject is on the program's frame either side of a cut, by one reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    /// Its box on the last frame before the cut: left, top, right, bottom, in shares of
    /// the frame.
    pub out: [f64; 4],
    /// Its box on the first frame after it.
    pub into: [f64; 4],
    /// How far the middle of the second is from the middle of the first, in frame heights.
    pub jump: f64,
    /// The same across the frame, with its direction: above 0 where the second is to the
    /// right.
    pub across: f64,
    /// The same down the frame: above 0 where the second is lower.
    pub down: f64,
    /// How large the second is beside the first, by their sides summed: 1 the same, above
    /// 1 where the subject is larger after the cut.
    pub size: f64,
}

impl Seen {
    /// What is between `out` and `into`, on a frame `aspect` times as wide as it is tall.
    fn between(out: [f64; 4], into: [f64; 4], aspect: f64) -> Self {
        let middle = |of: &[f64; 4]| [(of[0] + of[2]) / 2.0, (of[1] + of[3]) / 2.0];
        let sides = |of: &[f64; 4]| (of[2] - of[0]) * aspect + (of[3] - of[1]);
        let (from, to) = (middle(&out), middle(&into));
        let (across, down) = ((to[0] - from[0]) * aspect, to[1] - from[1]);
        Self {
            out,
            into,
            jump: across.hypot(down),
            across,
            down,
            size: sides(&into) / sides(&out).max(1e-9),
        }
    }

    /// Whether the eye is still on its subject after the cut: it jumps less than [`HELD`].
    pub fn held(&self) -> bool {
        self.jump < HELD
    }
}

/// Which way a subject goes across the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Going {
    Left,
    Right,
    /// Slower than [`STILL`].
    Still,
}

impl Going {
    /// By how fast it goes across, in frame heights a second, above 0 to the right.
    fn of(across: f64) -> Self {
        if across > STILL {
            Self::Right
        } else if across < -STILL {
            Self::Left
        } else {
            Self::Still
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Still => "still",
        }
    }
}

/// What was read of a cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    /// What the eye is on in each shot: the clip of its guide that is its subject.
    pub on: [String; 2],
    /// Where the guides have the two subjects.
    pub guide: Seen,
    /// Which way each goes, over [`WINDOW`] before the cut and after it: none where a clip
    /// is one frame long, or its guide does not show the subject that far from the cut.
    pub going: Option<[Going; 2]>,
    /// Where the takes have them: none while either shot has no file, and why where a
    /// file has nothing to read.
    pub take: Option<std::result::Result<Seen, String>>,
}

impl Read {
    /// Whether the subject goes one way before the cut and the other after it.
    pub fn reversed(&self) -> bool {
        matches!(
            self.going,
            Some([Going::Left, Going::Right] | [Going::Right, Going::Left])
        )
    }
}

/// A cut of a picture track: one clip ends on the frame the next starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// The frame the second clip starts on.
    pub at: Time,
    /// The clip that ends, and the one that starts.
    pub from: String,
    pub to: String,
    /// The transition across it, by its id. What is read is the two clips at the cut, and
    /// not how the transition mixes them.
    pub transition: Option<String>,
    /// The clips of tracks above that draw there, which may hide it or move it: they are
    /// not read.
    pub under: Vec<String>,
    /// What was read of it, or why it was not.
    pub read: std::result::Result<Read, String>,
}

/// The cuts of a sequence's picture, in the order they play.
#[derive(Clone, Debug, PartialEq)]
pub struct Eye {
    pub cuts: Vec<Cut>,
    /// How many more there are than [`MOST_CUTS`], which were not read.
    pub unread: usize,
}

/// Where a take's picture comes from: frame `frame` of a media, from 0, as wide and tall
/// as asked, four bytes a pixel, row by row. `Ok(None)` is a shot with no file yet.
pub type Takes<'t> =
    dyn FnMut(&Media, i64, (u32, u32)) -> std::result::Result<Option<Vec<u8>>, String> + 't;

/// A take's picture from its media's file, for [`eye`]: none where the file is not there,
/// which is a shot not made yet. The media's path is where the file is, so resolve the
/// project's paths first.
pub fn file_picture(
    media: &Media,
    frame: i64,
    size: (u32, u32),
) -> std::result::Result<Option<Vec<u8>>, String> {
    let file = Path::new(&media.path);
    if !file.is_file() {
        return Ok(None);
    }
    // Half a frame before it starts: the first frame from there on is the one.
    let seconds = (frame as f64 - 0.5) / media.rate.fps();
    crate::index::small_color_frame(file, seconds, size)
        .map(Some)
        .map_err(|error| format!("`{}`: {error}", media.id))
}

/// A guide being read: the sequence a shot's `render` step draws.
struct Guide {
    ids: Compositor,
    /// Its own picture, drawn only where a take is read beside it.
    picture: Option<Compositor>,
    scale: f64,
    duration: Time,
}

/// A clip of the program that plays a shot, and what its guide says the eye is on.
struct Shot<'a> {
    clip: &'a Clip,
    media: &'a Media,
    /// The guide's sequence, and the clip of it that is the subject.
    sequence: String,
    subject: String,
    /// The subject's color in the guide's ids pass.
    color: [u8; 3],
    /// Where in the guide's sequence the media starts.
    from: Time,
    /// How many times the take's picture the guide's frame is, across and down: 1 and 1
    /// where they are one shape, and above 1 on the side whose ends were cut off.
    grown: (f64, f64),
}

/// A `render` step of a shot: the sequence it draws, the size it draws it at where it
/// names one, and where in the sequence it starts, in seconds.
struct Drawn<'a> {
    sequence: &'a str,
    size: Option<(u32, u32)>,
    from: f64,
}

/// A shot's subject on one frame of the program.
struct Spot {
    landing: Landing,
    /// The moment of the guide's sequence the frame is of, and its ids pass then.
    when: Time,
    ids: Frame,
    /// The subject's box in the take's picture, by the guide, and on the program's frame.
    in_take: [f64; 4],
    on_frame: [f64; 4],
}

/// What of `around` is within 0 to 1 each way: none where none of it is.
fn within(around: [f64; 4]) -> Option<[f64; 4]> {
    let within = [
        around[0].max(0.0),
        around[1].max(0.0),
        around[2].min(1.0),
        around[3].min(1.0),
    ];
    (within[2] > within[0] && within[3] > within[1]).then_some(within)
}

/// Whether `next` plays on from where `clip` stops, as the two halves of a split do: no
/// cut is seen between them.
fn plays_on(clip: &Clip, next: &Clip) -> bool {
    match (&clip.content, &next.content) {
        (Content::Media { media, speed, .. }, Content::Media { media: other, .. }) => {
            media == other
                && !speed.is_reverse()
                && clip.speed() == next.speed()
                && clip.source_range().map(|(_, out)| out)
                    == next.source_range().map(|(into, _)| into)
        }
        _ => false,
    }
}

/// Whether effect `id` leaves a picture where it is.
fn in_place(id: &str) -> bool {
    Catalog::builtin()
        .fx(id)
        .is_some_and(|fx| IN_PLACE.contains(&fx.category.as_str()))
}

/// Where the subject's box `of`, in shares of the take's picture, is on the program's
/// frame, as `landing` puts the picture down.
fn on_frame(
    landing: &Landing,
    shot: &Shot,
    of: [f64; 4],
) -> std::result::Result<[f64; 4], String> {
    const EDGE: f64 = 1e-9;
    let kept = landing.kept;
    if of[0] < kept[0] - EDGE
        || of[1] < kept[1] - EDGE
        || of[2] > kept[2] + EDGE
        || of[3] > kept[3] + EDGE
    {
        return Err(format!(
            "the crop of `{}` cuts `{}`",
            shot.clip.id, shot.subject
        ));
    }
    // A picture that is turned has its box's corners elsewhere: the box around them.
    let mut around = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for corner in [[of[0], of[1]], [of[2], of[1]], [of[2], of[3]], [of[0], of[3]]] {
        let [x, y] = landing.on_frame(corner);
        around = [
            around[0].min(x),
            around[1].min(y),
            around[2].max(x),
            around[3].max(y),
        ];
    }
    within(around).ok_or_else(|| {
        format!(
            "`{}` puts `{}` out of the frame",
            shot.clip.id, shot.subject
        )
    })
}

/// A take's picture and the same part of its guide's own, to find the subject in.
struct Beside {
    /// How wide and tall both are, in pixels.
    size: (usize, usize),
    /// The subject's box in them, by the guide's ids pass.
    seed: crate::placed::Box,
    /// Four bytes a pixel, row by row.
    guide: Vec<u8>,
    take: Vec<u8>,
}

fn nothing_to_follow(shot: &Shot) -> String {
    format!(
        "`{}` has nothing to follow where the guide has `{}`",
        shot.media.id, shot.subject
    )
}

/// Where the take of `shot` has its subject on the program's frame, on the frame `spot` is
/// of: the guide's box, moved by how far the subject found in the take is from the subject
/// found the same way in the guide's own picture.
fn taken(shot: &Shot, spot: &Spot, pictures: &Beside) -> std::result::Result<[f64; 4], String> {
    let (w, h) = pictures.size;
    let size = (w as f64, h as f64);
    let hints = looked_for(&pictures.seed, size);
    let found = |picture: &Vec<u8>| {
        kerf_track::follow_object(1, &hints, |_| Some((w, h, picture.clone())))
            .outlines
            .first()
            .and_then(|outline| around(outline, size))
    };
    let (asked, there) = rayon::join(|| found(&pictures.guide), || found(&pictures.take));
    let asked = asked.ok_or_else(|| {
        format!(
            "`{}` cannot be told from what is around it in the guide's own picture, so nothing says where the take has it",
            shot.subject
        )
    })?;
    let there = there.ok_or_else(|| nothing_to_follow(shot))?;
    // Where a take has nothing of the subject, what is cut out of it there is a speck.
    let sides = |of: &[f64; 4]| (of[2] - of[0]) + (of[3] - of[1]);
    if sides(&there) < sides(&asked) * LEAST {
        return Err(nothing_to_follow(shot));
    }
    let guide = spot.in_take;
    let middle = [
        (guide[0] + guide[2]) / 2.0 + ((there[0] + there[2]) - (asked[0] + asked[2])) / 2.0 / size.0,
        (guide[1] + guide[3]) / 2.0 + ((there[1] + there[3]) - (asked[1] + asked[3])) / 2.0 / size.1,
    ];
    let half = [
        (guide[2] - guide[0]) / 2.0 * (there[2] - there[0]) / (asked[2] - asked[0]),
        (guide[3] - guide[1]) / 2.0 * (there[3] - there[1]) / (asked[3] - asked[1]),
    ];
    let moved = within([
        middle[0] - half[0],
        middle[1] - half[1],
        middle[0] + half[0],
        middle[1] + half[1],
    ])
    .ok_or_else(|| {
        format!(
            "`{}` has `{}` out of its picture",
            shot.media.id, shot.subject
        )
    })?;
    on_frame(&spot.landing, shot, moved)
}

struct Reader<'a> {
    project: &'a Project,
    subjects: &'a BTreeMap<String, String>,
    program: Compositor,
    /// How many times as wide as it is tall the program's frame is.
    aspect: f64,
    guides: HashMap<String, Guide>,
}

impl<'a> Reader<'a> {
    fn drawing(&self, sequence: &str, pass: Pass) -> std::result::Result<Compositor, String> {
        Compositor::new(
            self.project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: Some(sequence.to_owned()),
                pass,
                ..ComposeOptions::default()
            },
        )
        .map_err(|error| format!("the guide `{sequence}` cannot be drawn: {error}"))
    }

    fn guide(&mut self, sequence: &str) -> std::result::Result<&Guide, String> {
        if !self.guides.contains_key(sequence) {
            let ids = self.drawing(sequence, Pass::Ids)?;
            let scale = (HEIGHT / f64::from(ids.size().1.max(1))).min(1.0);
            let duration = ids.duration();
            self.guides.insert(
                sequence.to_owned(),
                Guide {
                    ids,
                    picture: None,
                    scale,
                    duration,
                },
            );
        }
        Ok(&self.guides[sequence])
    }

    /// The shot `clip` plays and what its guide says the eye is on, or why nothing does.
    fn shot(&mut self, clip: &'a Clip) -> std::result::Result<Shot<'a>, String> {
        let id = clip.id.as_str();
        let project = self.project;
        let media = match &clip.content {
            Content::Media { media, .. } => project
                .media(media.as_str())
                .ok_or_else(|| format!("`{id}` plays a media the project does not have"))?,
            Content::Sequence { sequence, .. } => {
                return Err(format!(
                    "`{id}` plays the sequence `{sequence}`, not a shot with a guide"
                ))
            }
            Content::Generator { fx, .. } => {
                return Err(format!("`{id}` is a `gen={fx}` clip, not a shot with a guide"))
            }
            Content::Caption { .. } => return Err(format!("`{id}` is a caption")),
        };
        let name = media.id.as_str();
        if media.steps.is_empty() {
            return Err(format!("`{id}` plays footage with no guide"));
        }
        let drawn: Vec<Drawn> = media
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::Render {
                    sequence,
                    size,
                    from,
                    ..
                } => Some(Drawn {
                    sequence,
                    size: *size,
                    from: from.unwrap_or(0.0),
                }),
                _ => None,
            })
            .collect();
        if drawn.is_empty() {
            return Err(format!(
                "`{name}`, which `{id}` plays, is made with no `render` step, so it has no guide"
            ));
        }
        let subject = match self.subjects.get(name) {
            Some(given) => given.clone(),
            None => media
                .steps
                .iter()
                .rev()
                .find_map(|step| match step {
                    Step::Generate(ask) => ask.moves.first().cloned(),
                    _ => None,
                })
                .ok_or_else(|| {
                    format!(
                        "nothing says what the eye is on in `{name}`, which `{id}` plays: its `generate` step names no `moves=<clip>`, and `subjects` gives none"
                    )
                })?,
        };
        let place = project
            .locate(&subject)
            .ok_or_else(|| format!("the subject of `{name}`: {}", project.no_clip(&subject)))?;
        let sequence = project.sequences[place.sequence].id.to_string();
        // A longer shot is drawn a part at a time, and starts where its first part does.
        let of_it = || drawn.iter().filter(|drawn| drawn.sequence == sequence);
        let Some(Drawn { size, .. }) = of_it().next() else {
            return Err(format!(
                "`{subject}` is a clip of `{sequence}`, which no `render` step of `{name}` draws"
            ));
        };
        let from = of_it().map(|drawn| drawn.from).fold(f64::INFINITY, f64::min);
        let from = Time::from_seconds(from).unwrap_or(Time::ZERO);
        let Some(video) = media.video else {
            return Err(format!("`{name}`, which `{id}` plays, has no picture"));
        };
        let guide = self.guide(&sequence)?;
        let color = guide
            .ids
            .ids()
            .into_iter()
            .find(|(drawn, _)| *drawn == subject)
            .map(|(_, color)| color)
            .ok_or_else(|| {
                format!(
                    "`{subject}` draws nothing in `{sequence}`, so it is nowhere on the frame: a camera, a light, a null, and a signal are not looked at"
                )
            })?;
        // The sequence fills its guide's size, and the guide the take's, what is over cut
        // off evenly each time: one scale, about the middle.
        let pair = |(wide, tall): (u32, u32)| (f64::from(wide.max(1)), f64::from(tall.max(1)));
        let canvas = pair(guide.ids.size());
        let drawn_at = size.map_or(canvas, pair);
        let take = pair((video.width, video.height));
        let fills = |of: (f64, f64), size: (f64, f64)| (size.0 / of.0).max(size.1 / of.1);
        let scale = fills(canvas, drawn_at) * fills(drawn_at, take);
        Ok(Shot {
            clip,
            media,
            sequence,
            subject,
            color,
            from,
            grown: (canvas.0 * scale / take.0, canvas.1 * scale / take.1),
        })
    }

    /// Where `shot`'s subject is on the program's frame at program time `at`, by its
    /// guide: `end` says which end of the clip that is, for what is said where it is not
    /// shown.
    fn spot(&self, shot: &Shot, at: Time, end: &str) -> std::result::Result<Spot, String> {
        let (clip, subject) = (shot.clip.id.as_str(), shot.subject.as_str());
        let landing = self
            .program
            .lands(clip, at)
            .map_err(|why| format!("where `{clip}` puts its picture is not read: {why}"))?;
        if landing.opacity <= 0.0 {
            return Err(format!("`{clip}` is clear where it {end}, and shows nothing"));
        }
        if landing.masked {
            return Err(format!("`{clip}` has a mask, which may cut `{subject}`"));
        }
        if let Some(effect) = landing.effects.iter().find(|id| !in_place(id)) {
            return Err(format!(
                "`{clip}` has `{effect}` on it, which may move or cut its picture"
            ));
        }
        let guide = &self.guides[&shot.sequence];
        // By time and not by a frame's number: a guide's rate is its own.
        let when = shot.from + shot.media.rate.time_of(landing.frame);
        if when >= guide.duration {
            return Err(format!(
                "`{clip}` plays `{}` past where its guide `{}` ends",
                shot.media.id, shot.sequence
            ));
        }
        let ids = guide.ids.frame(when, guide.scale);
        let (wide, tall) = (f64::from(ids.width), f64::from(ids.height));
        let Some((seen, _)) = box_of(&ids.rgba, ids.width as usize, shot.color) else {
            return Err(format!(
                "the guide `{}` does not show `{subject}` where `{clip}` {end}",
                shot.sequence
            ));
        };
        let across = |x: f64| 0.5 + (x / wide - 0.5) * shot.grown.0;
        let down = |y: f64| 0.5 + (y / tall - 0.5) * shot.grown.1;
        let in_take = within([across(seen[0]), down(seen[1]), across(seen[2]), down(seen[3])])
            .ok_or_else(|| {
                format!(
                    "`{}` is another shape than its guide `{}`, and `{subject}` is in what was cut off",
                    shot.media.id, shot.sequence
                )
            })?;
        let on_frame = on_frame(&landing, shot, in_take)?;
        Ok(Spot {
            landing,
            when,
            ids,
            in_take,
            on_frame,
        })
    }

    /// The take of `shot` beside its guide's own picture, on the frame `spot` is of: none
    /// where the shot has no file.
    fn beside(
        &mut self,
        shot: &Shot,
        spot: &Spot,
        takes: &mut Takes,
    ) -> Option<std::result::Result<Beside, String>> {
        let (wide, tall) = (spot.ids.width as usize, spot.ids.height as usize);
        // The part of the guide's frame the take's picture is of.
        let part = |whole: usize, grown: f64| {
            ((whole as f64 / grown).round() as usize).clamp(1, whole)
        };
        let (w, h) = (part(wide, shot.grown.0), part(tall, shot.grown.1));
        let take = match takes(shot.media, spot.landing.frame, (w as u32, h as u32)) {
            Ok(Some(take)) => take,
            Ok(None) => return None,
            Err(why) => return Some(Err(why)),
        };
        Some(self.paired(shot, spot, take, (w, h)))
    }

    /// `take`, a picture `w` by `h`, with the same part of the guide's own picture and
    /// where the guide's ids pass has the subject in it.
    fn paired(
        &mut self,
        shot: &Shot,
        spot: &Spot,
        take: Vec<u8>,
        (w, h): (usize, usize),
    ) -> std::result::Result<Beside, String> {
        if take.len() != w * h * 4 {
            return Err(format!(
                "the picture read of `{}` is not {w} by {h}",
                shot.media.id
            ));
        }
        let (wide, tall) = (spot.ids.width as usize, spot.ids.height as usize);
        let (left, top) = ((wide - w) / 2, (tall - h) / 2);
        let part = |rgba: &[u8]| -> Vec<u8> {
            (top..top + h)
                .flat_map(|y| &rgba[(y * wide + left) * 4..(y * wide + left + w) * 4])
                .copied()
                .collect()
        };
        if self.guides[&shot.sequence].picture.is_none() {
            let picture = self.drawing(&shot.sequence, Pass::Picture)?;
            if let Some(guide) = self.guides.get_mut(&shot.sequence) {
                guide.picture = Some(picture);
            }
        }
        let guide = &self.guides[&shot.sequence];
        let drawn = guide
            .picture
            .as_ref()
            .expect("it was made above")
            .frame(spot.when, guide.scale);
        if (drawn.width, drawn.height) != (spot.ids.width, spot.ids.height) {
            return Err(format!(
                "the guide `{}` draws its picture at another size than its ids pass",
                shot.sequence
            ));
        }
        let (seed, _) = box_of(&part(&spot.ids.rgba), w, shot.color)
            .ok_or_else(|| nothing_to_follow(shot))?;
        Ok(Beside {
            size: (w, h),
            seed,
            guide: part(&drawn.rgba),
            take,
        })
    }

    /// What is read of the cut at `at` from `leaves` to `enters`, clips of a sequence at
    /// `rate`.
    /// What is read of the cut at `at` from `leaves` to `enters`, clips of a sequence at
    /// `rate`.
    fn read(
        &mut self,
        (leaves, enters): (&'a Clip, &'a Clip),
        at: Time,
        rate: Rate,
        takes: &mut Takes,
    ) -> std::result::Result<Read, String> {
        let (first, second) = (self.shot(leaves)?, self.shot(enters)?);
        let frame = rate.frame_duration();
        let last = at - frame;
        let out = self.spot(&first, last, "ends")?;
        let into = self.spot(&second, at, "starts")?;
        let guide = Seen::between(out.on_frame, into.on_frame, self.aspect);
        // Which way each goes, from where it is a quarter of a second off the cut.
        let window = rate.time_of(((WINDOW * rate.fps()).round() as i64).max(1));
        let before = (last - window).max(leaves.start);
        let after = (at + window).min(enters.end() - frame);
        let earlier = (before < last)
            .then(|| self.spot(&first, before, "ends").ok())
            .flatten();
        let later = (after > at)
            .then(|| self.spot(&second, after, "starts").ok())
            .flatten();
        let across = |from: &Spot, to: &Spot, over: Time| {
            let middle = |spot: &Spot| (spot.on_frame[0] + spot.on_frame[2]) / 2.0;
            (middle(to) - middle(from)) * self.aspect / over.seconds()
        };
        let going = match (&earlier, &later) {
            (Some(earlier), Some(later)) => Some([
                Going::of(across(earlier, &out, last - before)),
                Going::of(across(&into, later, after - at)),
            ]),
            _ => None,
        };
        // The takes, where both shots have a file. Finding a subject in a picture is most
        // of what that costs, and the four findings of a cut wait on nothing of each other.
        let pictures = match self.beside(&first, &out, takes) {
            None => None,
            Some(one) => self.beside(&second, &into, takes).map(|other| (one, other)),
        };
        let aspect = self.aspect;
        let take = pictures.map(|(one, other)| {
            let (one, other) = (one?, other?);
            let (leaves, enters) = rayon::join(
                || taken(&first, &out, &one),
                || taken(&second, &into, &other),
            );
            Ok(Seen::between(leaves?, enters?, aspect))
        });
        Ok(Read {
            on: [first.subject, second.subject],
            guide,
            going,
            take,
        })
    }
}

/// Where the eye is either side of each cut of the picture of `sequence` (the first when
/// `None`) of `project`: those within `range`, from its start up to its end, or all of
/// them. `subjects` says what the eye is on in a shot whose steps do not, or in place of
/// what they say: a clip of its guide's sequence, by the shot's media. `takes` gives the
/// pictures of the shots that have a file ([`file_picture`]). Media is read from where the
/// project's paths say, so resolve them first.
pub fn eye(
    project: &Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
    subjects: &BTreeMap<String, String>,
    takes: &mut Takes,
) -> Result<Eye> {
    let target = crate::program::program_sequence(project, sequence)?;
    let program = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(target.id.to_string()),
            ..ComposeOptions::default()
        },
    )?;
    let (wide, tall) = program.size();
    let mut reader = Reader {
        project,
        subjects,
        program,
        aspect: f64::from(wide) / f64::from(tall.max(1)),
        guides: HashMap::new(),
    };
    let frame = target.rate.frame_duration();
    // A camera, a light, a null, and a signal draw nothing: no picture ends or starts
    // with one, and none is over a cut.
    let draws = |clip: &Clip| {
        !matches!(
            &clip.content,
            Content::Generator { fx, .. }
                if matches!(fx.as_str(), "null" | "camera" | "light" | kerf_model::SIGNAL)
        )
    };
    // Every cut, with the track it is on: from the lowest track up, then as they play.
    let mut found: Vec<(Time, usize, &Clip, &Clip)> = Vec::new();
    for (index, track) in target.tracks.iter().enumerate() {
        if track.kind != TrackKind::Video || !track.enabled {
            continue;
        }
        for pair in track.clips.windows(2) {
            let (leaves, enters) = (&pair[0], &pair[1]);
            let at = enters.start;
            if leaves.enabled
                && enters.enabled
                && draws(leaves)
                && draws(enters)
                && leaves.end() == at
                && !plays_on(leaves, enters)
                && range.is_none_or(|(from, to)| from <= at && at < to)
            {
                found.push((at, index, leaves, enters));
            }
        }
    }
    found.sort_by_key(|(at, ..)| *at);
    let unread = found.len().saturating_sub(MOST_CUTS);
    found.truncate(MOST_CUTS);
    let cuts = found
        .into_iter()
        .map(|(at, index, leaves, enters)| {
            let track = &target.tracks[index];
            let under = target.tracks[index + 1..]
                .iter()
                .filter(|above| above.kind == TrackKind::Video && above.enabled)
                .flat_map(|above| &above.clips)
                .filter(|clip| clip.enabled && draws(clip))
                .filter(|clip| clip.start <= at && at - frame < clip.end())
                .map(|clip| clip.id.to_string())
                .collect();
            Cut {
                at,
                from: leaves.id.to_string(),
                to: enters.id.to_string(),
                transition: track
                    .transitions
                    .iter()
                    .find(|transition| transition.from == leaves.id && transition.to == enters.id)
                    .map(|transition| transition.fx.to_string()),
                under,
                read: reader.read((leaves, enters), at, target.rate, takes),
            }
        })
        .collect();
    Ok(Eye { cuts, unread })
}
