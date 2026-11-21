//! What shows of each clip on a frame: where it is, how much of the frame
//! it has, and what is in front of it.
//!
//! A scene written as text, clips in depth under a camera, can be wrong in
//! ways its text does not show: a floor drawn over what stands on it, a
//! subject gone past the frame's edge, a wall with its back to the camera.
//! This reads a frame as an eye would, without one. It draws the frame's ids
//! pass ([`kerf_compose::Pass::Ids`]), which says which clip each pixel is
//! of, and then each clip by itself, the others made clear, which says where
//! that clip would be with nothing before it. What the two differ by is what
//! hides it.
//!
//! A pixel is not always of one clip. Along an edge two share it, and a clip
//! of thin strokes, a title or a post far off, may have no pixel to itself on
//! a frame this small: each is part its color and part the color of what is
//! behind it. So a clip is read by how much of each pixel it has. Drawn
//! alone, that is how much of its color the pixel is lit with. On the frame,
//! it is by the order the clips on the pixel are in there, which is the order
//! that makes the pixel's color when each is laid over those behind it. A
//! stroke over a floor is then before the floor, by as much of the pixel as
//! the stroke covers, and is not hidden by it. A color tells a few clips'
//! order and not many's: where more than five that let light through share a
//! pixel, the order is the nearest of those tried ([`MOST_ORDERS`]).
//!
//! It is of the clips of one sequence, a nested sequence being one clip, on a
//! small frame, so it reads to about a hundredth of the frame. Every clip that
//! draws is read, whichever track it is on. That is a frame drawn for each,
//! and they are drawn side by side, as many at once as there are cores, so a
//! scene of some tens of clips is read in a fraction of a second.

use crate::error::Result;
use kerf_compose::{ComposeOptions, Compositor, Frame, Pass};
use kerf_model::{Catalog, Param, Project, Time, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The height of the frames read, in pixels, where the sequence is as tall.
const HEIGHT: f64 = 216.0;

/// How far a byte of a pixel may be from a clip's color and still be that clip's.
const NEAR: u8 = 6;

/// How little of what is behind gets through the clips laid on a pixel for it
/// to be left out of the pixel.
const THROUGH: f32 = 1.0 / 64.0;

/// The most orders tried for the clips on one pixel, which is enough for
/// every order of five that let light through over one that lets none.
const MOST_ORDERS: usize = 512;

/// The most clips read on a frame. A scene blocked out by hand has some tens,
/// and each is a small frame to draw and to hold: this is there against a
/// frame of many hundreds, and where there are more the highest tracks' are
/// the ones read, a scene being stacked from what is behind up to its subject.
/// Fewer are read where the ids pass has not a color for each, which is in a
/// program of more than [`kerf_compose::ID_COLORS`] clips: see
/// [`Showing::unread`].
pub const MOST_CLIPS: usize = 256;

/// The most frames drawn at once, each being some megabytes while it is drawn.
const MOST_AT_ONCE: usize = 8;

/// What shows of one clip on a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub clip: String,
    /// Where it would be with nothing before it: left, top, right, bottom,
    /// in shares of the frame. `None` when none of it is in the frame.
    pub at: Option<[f64; 4]>,
    /// The share of the frame it would have with nothing before it.
    pub area: f64,
    /// The share of the frame it has, as the frame is drawn.
    pub shows: f64,
    /// The share of it that other clips hide.
    pub hidden: f64,
    /// Which clips hide it, and the share of it each hides, the most first.
    pub by: Vec<(String, f64)>,
    /// The edges of the frame it reaches: `left`, `top`, `right`, `bottom`.
    /// A clip that reaches one may go on past it.
    pub edges: Vec<&'static str>,
    /// How opaque it is, where that is not wholly: a clip that lets what is
    /// behind it through has that much less of each pixel it is on, and so of
    /// the frame.
    pub opacity: Option<f64>,
}

/// What shows on a frame of a sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct Showing {
    /// The clips that draw at that moment, from the lowest track up.
    pub clips: Vec<Shown>,
    /// The share of the frame no clip that is read has.
    pub empty: f64,
    /// Clips left out, from the lowest track up: those past [`MOST_CLIPS`],
    /// counting from the highest track down, and any whose color in the ids
    /// pass is also that of a clip on a higher track, which a frame cannot
    /// tell from it: the pass's colors come round after
    /// [`kerf_compose::ID_COLORS`] clips of a program. They are left out of
    /// the frame that is read as well, so what is said of the others is as if
    /// these were not there.
    pub unread: Vec<String>,
}

fn compositor(project: &Project, sequence: &str) -> Result<Compositor> {
    Ok(Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(sequence.to_owned()),
            pass: Pass::Ids,
            ..ComposeOptions::default()
        },
    )?)
}

/// `project` with the clips of its sequence `id` that `which` picks made clear:
/// they draw nothing, and still move what is parented to them.
fn with_clear(project: &Project, id: &str, which: impl Fn(&kerf_model::Clip) -> bool) -> Project {
    let mut cleared = project.clone();
    let own = cleared
        .sequences
        .iter_mut()
        .find(|sequence| sequence.id == id)
        .expect("the sequence is there");
    for clip in own
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.clips)
        .filter(|clip| which(clip))
    {
        clip.props
            .insert("opacity".to_owned(), Param::Static(Value::Number(0.0)));
    }
    cleared
}

fn rounded(share: f64) -> f64 {
    (share * 1000.0).round() / 1000.0
}

/// A pixel's color as light, which is what a pixel that clips share is a sum of.
fn light(color: &[u8]) -> [f32; 3] {
    let table = kerf_color::srgb_u8_to_linear_table();
    std::array::from_fn(|channel| table[usize::from(color[channel])])
}

/// Whether a pixel is `color`.
fn is(pixel: &[u8], color: &[u8; 3]) -> bool {
    color.iter().zip(pixel).all(|(a, b)| a.abs_diff(*b) <= NEAR)
}

/// Whether a pixel is of no clip.
fn dark(pixel: &[u8]) -> bool {
    pixel[..3].iter().all(|channel| *channel <= NEAR)
}

/// One clip drawn alone, the others clear.
struct Alone {
    /// Its place among the colors.
    place: usize,
    /// Its color as light.
    light: [f32; 3],
    /// How much of each pixel it reaches it would have with nothing before
    /// it, from 0 to 1, row by row. It has none of any other pixel, and they
    /// are not kept: a scene is mostly of clips that are small on the frame.
    cover: Vec<f32>,
    /// The pixels it reaches: left, top, right, bottom. `None` when it has none.
    reach: Option<[usize; 4]>,
}

impl Alone {
    /// `frame` read as the clip of `color`, which is all it holds.
    fn read(frame: &Frame, place: usize, color: &[u8; 3]) -> Self {
        let own = light(color);
        let whole: f32 = own.iter().map(|channel| channel * channel).sum();
        let w = (frame.width as usize).max(1);
        let mut reach: Option<[usize; 4]> = None;
        let cover: Vec<f32> = frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .map(|(index, pixel)| {
                if dark(pixel) {
                    return 0.0;
                }
                let (x, y) = (index % w, index / w);
                let [left, top, right, bottom] = reach.get_or_insert([x, y, x + 1, y + 1]);
                *left = (*left).min(x);
                *top = (*top).min(y);
                *right = (*right).max(x + 1);
                *bottom = (*bottom).max(y + 1);
                if is(pixel, color) {
                    return 1.0;
                }
                // Part its color and part nothing: as much of its color as is there.
                let along: f32 = light(pixel).iter().zip(own).map(|(a, b)| a * b).sum();
                (along / whole).clamp(0.0, 1.0)
            })
            .collect();
        let cover = match reach {
            Some([left, top, right, bottom]) => cover
                .chunks_exact(w)
                .take(bottom)
                .skip(top)
                .flat_map(|row| &row[left..right])
                .copied()
                .collect(),
            None => Vec::new(),
        };
        Self {
            place,
            light: own,
            cover,
            reach,
        }
    }

    /// How much of the pixel at `x` across and `y` down it would have with
    /// nothing before it.
    fn cover(&self, x: usize, y: usize) -> f32 {
        let Some([left, top, right, bottom]) = self.reach else {
            return 0.0;
        };
        if x < left || x >= right || y < top || y >= bottom {
            return 0.0;
        }
        self.cover[(y - top) * (right - left) + x - left]
    }
}

/// The clips on one pixel, and the order they are in there.
struct Order<'a> {
    alone: &'a [Alone],
    /// The clips that would be on the pixel, by their place in `alone`, each
    /// with how much of the pixel it would have.
    here: &'a [(usize, f32)],
    /// The pixel as the frame has it, as the root of its light, in which one
    /// byte is as far from the next for a dark pixel as for a bright one.
    pixel: [f32; 3],
    tried: usize,
    laid: Vec<usize>,
    /// How far the nearest order yet is from the pixel, and that order.
    best: (f32, Vec<usize>),
}

impl<'a> Order<'a> {
    /// The order the clips `here` are in on a pixel of `color`, the nearest
    /// first, by their places in `here`: of their orders, the one that comes
    /// nearest that color when each clip is laid over those behind it. A clip
    /// behind those that let nothing through is left out, being in no order
    /// there. The orders are tried from `here` as it is given, which matters
    /// where there are more than are tried: the highest track first is the
    /// likeliest.
    fn of(alone: &'a [Alone], here: &'a [(usize, f32)], color: &[u8]) -> Vec<usize> {
        let mut order = Order {
            alone,
            here,
            pixel: light(color).map(f32::sqrt),
            tried: 0,
            laid: Vec::with_capacity(here.len()),
            best: (f32::INFINITY, Vec::new()),
        };
        order.lay([0.0; 3], 1.0);
        order.best.1
    }

    /// Tries every order of the clips not yet laid, behind those that are:
    /// those have made `light` of the pixel, and let `through` of what is
    /// behind them.
    fn lay(&mut self, light: [f32; 3], through: f32) {
        if through < THROUGH || self.laid.len() == self.here.len() {
            self.tried += 1;
            let miss: f32 = light
                .iter()
                .zip(self.pixel)
                .map(|(made, is)| (made.sqrt() - is).powi(2))
                .sum();
            if miss < self.best.0 {
                self.best = (miss, self.laid.clone());
            }
            return;
        }
        for next in 0..self.here.len() {
            if self.tried >= MOST_ORDERS {
                return;
            }
            if self.laid.contains(&next) {
                continue;
            }
            let (clip, cover) = self.here[next];
            let share = through * cover;
            let own = self.alone[clip].light;
            self.laid.push(next);
            self.lay(
                std::array::from_fn(|channel| light[channel] + share * own[channel]),
                through - share,
            );
            self.laid.pop();
        }
    }
}

/// What shows of each clip of `sequence` (the first when `None`) of `project`
/// at sequence time `at`. Media is read from where the project's paths say,
/// so resolve them first.
pub fn showing(project: &Project, sequence: Option<&str>, at: Time) -> Result<Showing> {
    let target = crate::program::program_sequence(project, sequence)?;
    let id = target.id.to_string();
    let whole = compositor(project, &id)?;
    let colors = whole.ids();
    let scale = (HEIGHT / f64::from(target.height.max(1))).min(1.0);

    // The clips of the sequence that draw now, from the lowest track up, each
    // with its place among the colors.
    let drawing: Vec<(&kerf_model::Clip, usize)> = target
        .tracks
        .iter()
        .filter(|track| track.enabled)
        .flat_map(|track| &track.clips)
        .filter(|clip| clip.enabled && clip.start <= at && at < clip.end())
        .filter_map(|clip| {
            let place = colors
                .iter()
                .position(|(told, _)| *told == clip.id.as_str());
            place.map(|place| (clip, place))
        })
        .collect();
    // Those read, taken from the highest track down: as many as are read at
    // most, each with a color no clip above it has. Two of one color are one
    // clip to a frame, so the lower is left out, of the frame as well.
    let (mut read, mut unread) = (Vec::new(), Vec::new());
    for (clip, place) in drawing.into_iter().rev() {
        let its_own = read
            .iter()
            .all(|(_, taken): &(_, usize)| !is(&colors[*taken].1, &colors[place].1));
        if read.len() < MOST_CLIPS && its_own {
            read.push((clip, place));
        } else {
            unread.push(clip);
        }
    }
    read.reverse();
    unread.reverse();
    let whole = if unread.is_empty() {
        whole
    } else {
        let out = |clip: &kerf_model::Clip| unread.iter().any(|left| left.id == clip.id);
        compositor(&with_clear(project, &id, out), &id)?
    };

    let one = |(clip, place): &(&kerf_model::Clip, usize)| -> Result<Alone> {
        // The frame with the others clear: they still move what is parented to them.
        let others_clear = with_clear(project, &id, |other| other.id != clip.id);
        let there = compositor(&others_clear, &id)?.frame(at, scale);
        Ok(Alone::read(&there, *place, &colors[*place].1))
    };
    // Each clip alone is a frame of its own that waits on no other, so they are
    // drawn side by side, and beside the frame of them all, which the first
    // drawer starts with. A drawer is a thread that is a pool of one: a
    // frame's own loops then run on the thread that draws it, and a frame is
    // one core's work. Not tasks of the pool those loops share, where a thread
    // that waits on a loop takes up another frame while the one it is in is
    // half drawn, and would wait for good on a lock that one holds, of what
    // is kept between frames. And not plain threads, from which every loop of
    // a small frame is handed to that pool and waited for.
    let drawers = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .min(MOST_AT_ONCE)
        .min(read.len() + 1);
    let next = AtomicUsize::new(0);
    let draw = |first: bool| {
        let frame = first.then(|| whole.frame(at, scale));
        let mut alone = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(clip) = read.get(index) else {
                return (frame, alone);
            };
            alone.push((index, one(clip)));
        }
    };
    type Drawn = (Option<Frame>, Vec<(usize, Result<Alone>)>);
    let drawn: Vec<Drawn> = std::thread::scope(|scope| {
        let draw = &draw;
        let drawers: Vec<_> = (0..drawers)
            .map(|drawer| {
                scope.spawn(move || {
                    let draw = || draw(drawer == 0);
                    match rayon::ThreadPoolBuilder::new().num_threads(1).build() {
                        Ok(of_one) => of_one.install(draw),
                        Err(_) => draw(),
                    }
                })
            })
            .collect();
        drawers
            .into_iter()
            .map(|drawer| {
                drawer
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    let (mut frame, mut alone) = (None, Vec::with_capacity(read.len()));
    for (whole, clips) in drawn {
        frame = frame.or(whole);
        alone.extend(clips);
    }
    let frame = frame.expect("the first drawer drew it");
    // In the clips' order, whichever drawer drew each.
    alone.sort_by_key(|(index, _)| *index);
    let alone = alone
        .into_iter()
        .map(|(_, alone)| alone)
        .collect::<Result<Vec<Alone>>>()?;
    let pixels = (frame.width * frame.height).max(1) as usize;
    let empty = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| dark(*pixel))
        .count();

    // How much of the frame's pixels each clip has, and how much of what it
    // would have each other clip takes by being before it, in pixels.
    let mut seen = vec![0.0f64; alone.len()];
    let mut hidden_by = vec![vec![0.0f64; colors.len()]; alone.len()];
    let mut here: Vec<(usize, f32)> = Vec::with_capacity(alone.len());
    let mut before: Vec<(usize, f32)> = Vec::with_capacity(alone.len());
    let across = (frame.width as usize).max(1);
    for (index, pixel) in frame.rgba.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (index % across, index / across);
        // The highest track first, which is the nearest where no depth says otherwise.
        here.clear();
        here.extend(alone.iter().enumerate().rev().filter_map(|(clip, alone)| {
            let cover = alone.cover(x, y);
            (cover > 0.0).then_some((clip, cover))
        }));
        if here.is_empty() {
            continue;
        }
        // A pixel of one clip's color is that clip's, where that clip is, and
        // all else there is behind it. A clip that is read before any other
        // of that color: no two that are read share one, and a clip elsewhere
        // in the program may.
        let front = alone
            .iter()
            .find(|alone| alone.cover(x, y) > 0.0 && is(pixel, &colors[alone.place].1))
            .map(|alone| alone.place)
            .or_else(|| {
                colors
                    .iter()
                    .position(|(_, color)| is(pixel, color))
                    .filter(|front| alone.iter().all(|alone| alone.place != *front))
            });
        if let Some(front) = front {
            for (clip, cover) in &here {
                if alone[*clip].place == front {
                    seen[*clip] += f64::from(*cover);
                } else {
                    hidden_by[*clip][front] += f64::from(*cover);
                }
            }
            continue;
        }
        // A pixel of no one color is shared: each clip on it has what those
        // before it let through, and they hide the rest of it.
        let order = Order::of(&alone, &here, pixel);
        let mut hide = |clip: usize, cover: f32, before: &[(usize, f32)]| {
            for (place, share) in before {
                hidden_by[clip][*place] += f64::from(cover * share);
            }
        };
        let mut through = 1.0f32;
        before.clear();
        for laid in &order {
            let (clip, cover) = here[*laid];
            seen[clip] += f64::from(cover * through);
            hide(clip, cover, &before);
            before.push((alone[clip].place, cover * through));
            through -= cover * through;
        }
        for (at, (clip, cover)) in here.iter().enumerate() {
            if !order.contains(&at) {
                hide(*clip, *cover, &before);
            }
        }
    }

    let (w, h) = (frame.width.max(1) as f64, frame.height.max(1) as f64);
    let mut clips = Vec::with_capacity(read.len());
    for ((((clip, _), alone), seen), hidden_by) in read.iter().zip(&alone).zip(seen).zip(hidden_by)
    {
        let area: f64 = alone.cover.iter().map(|cover| f64::from(*cover)).sum();
        let share = |count: f64| rounded(count / pixels as f64);
        let of_it = |count: f64| if area > 0.0 { rounded(count / area) } else { 0.0 };
        let mut by: Vec<(String, f64)> = hidden_by
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0.0 && **count * 50.0 >= area)
            .map(|(other, count)| (colors[other].0.clone(), of_it(*count)))
            .collect();
        by.sort_by(|a, b| b.1.total_cmp(&a.1));
        let opacity = clip
            .props
            .get("opacity")
            .and_then(|opacity| opacity.value_at(at - clip.start).as_number())
            .filter(|opacity| *opacity < 0.999);
        let mut edges = Vec::new();
        if let Some([left, top, right, bottom]) = alone.reach {
            for (reached, name) in [
                (left == 0, "left"),
                (top == 0, "top"),
                (right == frame.width as usize, "right"),
                (bottom == frame.height as usize, "bottom"),
            ] {
                if reached {
                    edges.push(name);
                }
            }
        }
        clips.push(Shown {
            clip: clip.id.to_string(),
            at: alone.reach.map(|[left, top, right, bottom]| {
                [
                    rounded(left as f64 / w),
                    rounded(top as f64 / h),
                    rounded(right as f64 / w),
                    rounded(bottom as f64 / h),
                ]
            }),
            area: share(area),
            shows: share(seen),
            hidden: of_it(hidden_by.iter().sum()),
            by,
            edges,
            opacity,
        });
    }
    Ok(Showing {
        clips,
        empty: rounded(empty as f64 / pixels as f64),
        unread: unread.iter().map(|clip| clip.id.to_string()).collect(),
    })
}
