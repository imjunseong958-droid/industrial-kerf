//! Cutting an object out of a picture and following it, as a moving outline.
//!
//! The first frame is an interactive graph cut (Boykov & Jolly 2001) with iterated colour models
//! (GrabCut, Rother et al. 2004): what is known in or out is a hard constraint, every other pixel
//! gets a data cost from the colour models and neighbours a contrast-sensitive smoothness cost, and
//! the minimum s-t cut ([`maxflow`]) is the segmentation. Each next frame warps the matte by optical
//! flow and cuts again in a band around its edge (in the spirit of Video SnapCut, Bai et al. 2009). A
//! subject that went further than that band is first found again by its own pixels ([`moved_by`]), where
//! the flow has lost it ([`flow_lost`]), and the flow then only says what changed of it, with the band
//! wider the further it went.
//! A trained model ([`kerf_segment::MaskModel`]) is optional and swappable: its foreground
//! probability becomes a strong prior for the same cut, so hints stay hard and edges still snap to
//! colour, and one that disagrees with the flow is ignored for that frame.
//!
//! Carried from EffectCraft's `track/src/roto`. Kerf keeps no matte of its own: what this finds is
//! written as a mask whose outline has keyframes, which merges, reads, and edits like any other, with
//! how soft its edge is at each corner, measured from the picture ([`soft`]).

pub mod gmm;
pub mod matting;
pub mod maxflow;
mod shutter;
mod soft;

use gmm::Gmm;
use kerf_raster::flow::{block_flow, Flow};
use kerf_raster::Image;
use kerf_segment::{MaskModel, Prompt};
use maxflow::{Graph, Segment};
use rayon::prelude::*;

/// Corners in an outline. Every frame's has this many, as a moving mask needs.
pub const OUTLINE_CORNERS: usize = 40;

/// Hard constraints: 1 = foreground, 2 = background, 0 = free.
const HARD_FG: u8 = 1;
const HARD_BG: u8 = 2;

/// Graph-cut energy weights.
const GAMMA: f64 = 50.0;
const PRIOR: f64 = 1.0;
/// A trained model's matte as a prior: it outweighs the colour models.
const MODEL_PRIOR: f64 = 4.0;
/// Fixed-point scale of the energies.
const K: f64 = 16.0;
const INF: i64 = 1 << 40;

const NB: [(i64, i64, bool); 8] = [
    (1, 0, true),
    (0, 1, true),
    (1, 1, true),
    (-1, 1, true),
    (-1, 0, false),
    (0, -1, false),
    (-1, -1, false),
    (1, -1, false),
];

const COMPONENTS: usize = 5;
const MAX_SAMPLES: usize = 24_000;

fn beta(img: &[[f32; 3]], free: &[bool], w: usize, h: usize) -> f64 {
    let (mut s, mut n) = (0.0f64, 0.0f64);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !free[i] {
                continue;
            }
            let c = img[i];
            for (dx, dy) in [(1usize, 0usize), (0, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < w && ny < h {
                    let q = img[ny * w + nx];
                    s += ((c[0] - q[0]).powi(2) + (c[1] - q[1]).powi(2) + (c[2] - q[2]).powi(2)) as f64;
                    n += 1.0;
                }
            }
        }
    }
    if s <= 1e-12 { 1.0 } else { n / (2.0 * s) }
}

/// One min-cut over the `free` pixels; others keep `labels`. `prior`: P(foreground) per pixel
/// and how much it weighs against the colour models.
fn cut(img: &[[f32; 3]], w: usize, h: usize, labels: &mut [u8], free: &[bool], hard: &[u8], fg: &Gmm, bg: &Gmm, prior: Option<(&[f32], f64)>) {
    let mut idx = vec![u32::MAX; w * h];
    let mut nodes = vec![];
    for i in 0..w * h {
        if free[i] {
            idx[i] = nodes.len() as u32;
            nodes.push(i);
        }
    }
    if nodes.is_empty() {
        return;
    }
    let b = beta(img, free, w, h);
    let costs: Vec<(f64, f64)> = nodes
        .par_iter()
        .map(|&i| {
            let c = img[i];
            let (mut cf, mut cb) = (fg.cost(c), bg.cost(c));
            if let Some((p, wt)) = prior {
                let pf = (p[i] as f64).clamp(0.02, 0.98);
                cf += wt * -pf.ln();
                cb += wt * -(1.0 - pf).ln();
            }
            (cf, cb)
        })
        .collect();
    let mut g = Graph::new(nodes.len(), nodes.len() * 4);
    for (n, &i) in nodes.iter().enumerate() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let (cf, cb) = costs[n];
        let m = cf.min(cb);
        let (mut src, mut snk) = (((cb - m) * K).round() as i64, ((cf - m) * K).round() as i64);
        match hard[i] {
            HARD_FG => (src, snk) = (INF, 0),
            HARD_BG => (src, snk) = (0, INF),
            _ => {}
        }
        g.add_tweights(n, src, snk);
        let c = img[i];
        for (dx, dy, fwd) in NB {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            let q = img[j];
            let d2 = ((c[0] - q[0]).powi(2) + (c[1] - q[1]).powi(2) + (c[2] - q[2]).powi(2)) as f64;
            let dist = if dx != 0 && dy != 0 { std::f64::consts::SQRT_2 } else { 1.0 };
            let wt = ((GAMMA / dist * (-b * d2).exp()) * K).round() as i32;
            if wt <= 0 {
                continue;
            }
            if idx[j] == u32::MAX {
                // Fixed neighbour: disagreeing with it costs the edge.
                if labels[j] != 0 {
                    g.add_tweights(n, wt as i64, 0);
                } else {
                    g.add_tweights(n, 0, wt as i64);
                }
            } else if fwd {
                g.add_edge(n, idx[j] as usize, wt, wt);
            }
        }
    }
    g.maxflow();
    for (n, &i) in nodes.iter().enumerate() {
        labels[i] = (g.segment(n) == Segment::Source) as u8;
    }
}

/// Colour samples of the pixels where `sel` holds.
fn samples(img: &[[f32; 3]], sel: impl Fn(usize) -> bool) -> Vec<[f32; 3]> {
    img.iter().enumerate().filter(|(i, _)| sel(*i)).map(|(_, c)| *c).collect()
}

fn fit_models(img: &[[f32; 3]], labels: &[u8]) -> (Gmm, Gmm) {
    let fg = samples(img, |i| labels[i] != 0);
    let bg = samples(img, |i| labels[i] == 0);
    (Gmm::fit(&fg, COMPONENTS, MAX_SAMPLES), Gmm::fit(&bg, COMPONENTS, MAX_SAMPLES))
}

/// Keep the foreground components (8-connected) that touch `seeds`; fill small background
/// holes that touch neither the frame edge nor a background seed.
fn clean(labels: &mut [u8], seeds: &[bool], bg_seeds: &[bool], w: usize, h: usize) {
    let n = w * h;
    let mut comp = vec![u32::MAX; n];
    let mut stack = vec![];
    let mut fg_area = 0usize;
    for start in 0..n {
        if comp[start] != u32::MAX {
            continue;
        }
        let lab = labels[start];
        let id = start as u32;
        comp[start] = id;
        stack.push(start);
        let mut members = vec![];
        let (mut seeded, mut edge, mut bgs) = (false, false, false);
        while let Some(i) = stack.pop() {
            members.push(i);
            seeded |= seeds[i];
            bgs |= bg_seeds[i];
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            edge |= x == 0 || y == 0 || x == w as i64 - 1 || y == h as i64 - 1;
            for (dx, dy, _) in NB {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if comp[j] == u32::MAX && labels[j] == lab {
                    comp[j] = id;
                    stack.push(j);
                }
            }
        }
        if lab != 0 {
            if seeded {
                fg_area += members.len();
            } else {
                for i in members {
                    labels[i] = 0;
                }
            }
        } else if !edge && !bgs {
            // Remember holes as negative candidates (resolved below once fg_area is known).
            for i in members {
                comp[i] = u32::MAX - 1;
            }
        }
    }
    // Fill holes smaller than 2 % of the foreground.
    let mut seen = vec![false; n];
    for start in 0..n {
        if comp[start] != u32::MAX - 1 || seen[start] || labels[start] != 0 {
            continue;
        }
        let mut members = vec![start];
        seen[start] = true;
        let mut k = 0;
        while k < members.len() {
            let i = members[k];
            k += 1;
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            for (dx, dy, _) in NB {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if !seen[j] && comp[j] == u32::MAX - 1 && labels[j] == 0 {
                    seen[j] = true;
                    members.push(j);
                }
            }
        }
        // A hole is enclosed by foreground that survived the component filter.
        let enclosed = members.iter().all(|&i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            NB.iter().all(|(dx, dy, _)| {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    return false;
                }
                let j = ny as usize * w + nx as usize;
                labels[j] != 0 || comp[j] == u32::MAX - 1
            })
        });
        if enclosed && (members.len() as f64) < 0.02 * fg_area as f64 {
            for i in members {
                labels[i] = 1;
            }
        }
    }
}

/// What is known of the object on the first frame, in fractions of the frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hints {
    /// Points on it.
    pub inside: Vec<[f64; 2]>,
    /// Points off it.
    pub outside: Vec<[f64; 2]>,
    /// A box around it, left, top, right, bottom: nothing outside it is the object.
    pub bbox: Option<[f64; 4]>,
}

/// An object's way through the frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FollowedObject {
    /// Its outline in fractions of the frame, one a frame, up to where it was lost, each of
    /// [`OUTLINE_CORNERS`] corners that move one for one from the frame before.
    pub outlines: Vec<Vec<[f64; 2]>>,
    /// How wide its edge's fall-off is at each of those corners, one a frame, as fractions of the
    /// frame's width: 0 where it is hard.
    pub soft: Vec<Vec<f64>>,
    /// How much of the frame it covers, one a frame.
    pub areas: Vec<f64>,
    /// The frame it could no longer be found in.
    pub lost_at: Option<usize>,
}

/// A frame as straight colour, and as the flow reads it.
struct Picture {
    width: usize,
    height: usize,
    rgb: Vec<[f32; 3]>,
    image: Image,
}

impl Picture {
    fn of(width: usize, height: usize, rgba: &[u8]) -> Self {
        let rgb: Vec<[f32; 3]> = rgba
            .as_chunks::<4>().0.iter()
            .map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0))
            .collect();
        let image = Image {
            width: width as u32,
            height: height as u32,
            data: rgb.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect(),
        };
        Self {
            width,
            height,
            rgb,
            image,
        }
    }
}

/// Pixels within `radius` of any of `points`, in pixels.
fn discs(points: &[[f64; 2]], radius: f64, w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for point in points {
        let (x0, x1) = (
            (point[0] - radius).floor().max(0.0) as usize,
            ((point[0] + radius).ceil().max(0.0) as usize).min(w),
        );
        let (y0, y1) = (
            (point[1] - radius).floor().max(0.0) as usize,
            ((point[1] + radius).ceil().max(0.0) as usize).min(h),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f64 + 0.5 - point[0], y as f64 + 0.5 - point[1]);
                if dx.hypot(dy) <= radius {
                    out[y * w + x] = true;
                }
            }
        }
    }
    out
}

/// The first frame's matte from its hints, and the colour models that made it.
fn first_matte(
    picture: &Picture,
    hints: &Hints,
    model: Option<&dyn MaskModel>,
) -> Option<(Vec<u8>, Gmm, Gmm)> {
    let (w, h) = (picture.width, picture.height);
    let pixel = |point: &[f64; 2]| [point[0] * w as f64, point[1] * h as f64];
    let mut inside: Vec<[f64; 2]> = hints.inside.iter().map(pixel).collect();
    let outside: Vec<[f64; 2]> = hints.outside.iter().map(pixel).collect();
    let bbox = hints.bbox.map(|b| {
        [
            b[0] * w as f64,
            b[1] * h as f64,
            b[2] * w as f64,
            b[3] * h as f64,
        ]
    });
    // A box alone says the object is around its middle.
    if let (true, Some(b)) = (inside.is_empty(), bbox) {
        inside.push([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]);
    }
    if inside.is_empty() {
        return None;
    }
    let radius = (w.max(h) as f64 / 80.0).max(2.0);
    let fgs = discs(&inside, radius, w, h);
    let mut bgs = discs(&outside, radius, w, h);
    if let Some(b) = bbox {
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                if px < b[0] || px > b[2] || py < b[1] || py > b[3] {
                    bgs[y * w + x] = true;
                }
            }
        }
    }
    let hard: Vec<u8> = fgs
        .iter()
        .zip(&bgs)
        .map(|(f, b)| {
            if *b {
                HARD_BG
            } else if *f {
                HARD_FG
            } else {
                0
            }
        })
        .collect();
    let prior = model.and_then(|model| {
        let prompt = Prompt {
            points: inside
                .iter()
                .map(|p| ([p[0] as f32, p[1] as f32], true))
                .chain(outside.iter().map(|p| ([p[0] as f32, p[1] as f32], false)))
                .collect(),
            bbox: bbox.map(|b| b.map(|v| v as f32)),
            mask: None,
        };
        model
            .segment(&picture.rgb, w, h, &prompt)
            .ok()
            .filter(|soft| soft.len() == w * h)
    });
    let rgb = &picture.rgb;
    let mut fg_samples = samples(rgb, |i| hard[i] == HARD_FG);
    let mut bg_samples = samples(rgb, |i| hard[i] == HARD_BG);
    if let Some(soft) = &prior {
        fg_samples.extend(samples(rgb, |i| hard[i] == 0 && soft[i] >= 0.5));
        bg_samples.extend(samples(rgb, |i| hard[i] == 0 && soft[i] < 0.5));
    }
    if bg_samples.len() < 64 {
        // No word of what it is not: what is far from what it is.
        let distance = matting::distance(&fgs, w, h);
        let far = (4.0 * radius).max(0.25 * w.max(h) as f64) as f32;
        bg_samples = samples(rgb, |i| distance[i] > far);
        if bg_samples.is_empty() {
            bg_samples = samples(rgb, |i| hard[i] != HARD_FG);
        }
    }
    let (mut fg, mut bg) = (
        Gmm::fit(&fg_samples, COMPONENTS, MAX_SAMPLES),
        Gmm::fit(&bg_samples, COMPONENTS, MAX_SAMPLES),
    );
    let mut labels: Vec<u8> = match &prior {
        Some(soft) => soft.iter().map(|v| u8::from(*v >= 0.5)).collect(),
        None => fgs.iter().map(|v| u8::from(*v)).collect(),
    };
    for (label, known) in labels.iter_mut().zip(&hard) {
        if *known != 0 {
            *label = u8::from(*known == HARD_FG);
        }
    }
    let free = vec![true; w * h];
    let weighed = prior.as_deref().map(|soft| (soft, MODEL_PRIOR));
    for round in 0..3 {
        cut(rgb, w, h, &mut labels, &free, &hard, &fg, &bg, weighed);
        if round < 2 {
            let (a, b) = fit_models(rgb, &labels);
            if !a.is_empty() && !b.is_empty() {
                (fg, bg) = (a, b);
            }
        }
    }
    clean(&mut labels, &fgs, &bgs, w, h);
    labels.contains(&1).then_some((labels, fg, bg))
}

/// The most a picture is searched at the coarsest, across the longer of its sides.
const COARSEST: usize = 96;
/// How many of a subject's pixels stand for it at any size.
const MOST_READ: usize = 2500;
/// How much better than where it was it must match to have gone anywhere.
const CLEARLY_BETTER: f64 = 0.85;
/// How many bands a subject may go in a frame for the flow alone to keep up, as measured: all of it so
/// far at two, at any size.
const FLOW_KEEPS_UP: f64 = 2.0;
/// How many bands a band may grow to, with how far a subject went.
const MOST_BANDS: f64 = 3.0;

/// A picture at `by` of its size each way, each pixel the mean of those it covers.
fn smaller(rgb: &[[f32; 3]], (w, h): (usize, usize), by: usize) -> (Vec<[f32; 3]>, usize, usize) {
    let (sw, sh) = (w.div_ceil(by), h.div_ceil(by));
    let mut sums = vec![([0.0f32; 3], 0u32); sw * sh];
    for (index, color) in rgb.iter().enumerate() {
        let at = (index / w) / by * sw + (index % w) / by;
        let (sum, count) = &mut sums[at];
        for k in 0..3 {
            sum[k] += color[k];
        }
        *count += 1;
    }
    let out = sums
        .into_iter()
        .map(|(sum, count)| sum.map(|value| value / count.max(1) as f32))
        .collect();
    (out, sw, sh)
}

/// How far the subject in `matte` went from `previous` to `next`, in whole pixels, found by how well its
/// own pixels match anywhere in the frame, and closer at each of a few sizes. Nothing where it matches
/// no better there than where it was, or where it is all but the same.
fn moved_by(previous: &Picture, matte: &[u8], next: &Picture) -> [i64; 2] {
    let (w, h) = (next.width, next.height);
    let mut by = 1;
    while w.max(h) / by > COARSEST {
        by *= 2;
    }
    let mut best = [0i64; 2];
    let mut level = by;
    let mut coarsest = true;
    let mut where_it_was = f64::INFINITY;
    let mut best_cost = f64::INFINITY;
    while level >= 1 {
        let (from, sw, sh) = smaller(&previous.rgb, (w, h), level);
        let (to, _, _) = smaller(&next.rgb, (w, h), level);
        // Its pixels at this size, well inside what is known of it, as few as stand for it.
        let on: Vec<(i64, i64)> = (0..sw * sh)
            .filter(|at| {
                let (x, y) = ((at % sw) * level, (at / sw) * level);
                matte[(y + level / 2).min(h - 1) * w + (x + level / 2).min(w - 1)] != 0
            })
            .map(|at| ((at % sw) as i64, (at / sw) as i64))
            .collect();
        let step = (on.len() / MOST_READ).max(1);
        let read: Vec<(i64, i64)> = on.iter().copied().step_by(step).collect();
        if read.is_empty() {
            return [0, 0];
        }
        let cost = |dx: i64, dy: i64| -> f64 {
            let (mut sum, mut count) = (0.0f64, 0usize);
            for (x, y) in &read {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= sw as i64 || ny >= sh as i64 {
                    continue;
                }
                let a = from[*y as usize * sw + *x as usize];
                let b = to[ny as usize * sw + nx as usize];
                sum += f64::from(
                    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs(),
                );
                count += 1;
            }
            // Mostly out of the frame says nothing.
            if count * 2 < read.len() {
                f64::INFINITY
            } else {
                sum / count as f64
            }
        };
        let (reach_x, reach_y, around) = if coarsest {
            (sw as i64, sh as i64, [0i64, 0i64])
        } else {
            (2, 2, [best[0] / level as i64, best[1] / level as i64])
        };
        let tried: Vec<(i64, i64)> = (-reach_y..=reach_y)
            .flat_map(|dy| (-reach_x..=reach_x).map(move |dx| (dx, dy)))
            .collect();
        let (dx, dy, least) = tried
            .par_iter()
            .map(|(dx, dy)| {
                let (dx, dy) = (around[0] + dx, around[1] + dy);
                (dx, dy, cost(dx, dy))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap_or((0, 0, f64::INFINITY));
        best = [dx * level as i64, dy * level as i64];
        best_cost = least;
        where_it_was = cost(0, 0);
        coarsest = false;
        level /= 2;
    }
    if best_cost < where_it_was * CLEARLY_BETTER {
        best
    } else {
        [0, 0]
    }
}

/// The band to cut in, in pixels, for a frame of `(w, h)` whose subject went `moved`: wider the further
/// it went, since what lets it go that far blurs and turns it too.
fn band_of((w, h): (usize, usize), moved: [i64; 2]) -> f64 {
    let base = (w.max(h) as f64 / 40.0).max(4.0);
    let went = (moved[0] as f64).hypot(moved[1] as f64);
    (went / 4.0).clamp(base, base * MOST_BANDS)
}

/// A picture's pixels moved by `by`, its edge carrying on past it.
fn shifted_image(image: &Image, by: [i64; 2]) -> Image {
    let (w, h) = (image.width as i64, image.height as i64);
    let data = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let (fx, fy) = ((x - by[0]).clamp(0, w - 1), (y - by[1]).clamp(0, h - 1));
            image.data[(fy * w + fx) as usize]
        })
        .collect();
    Image {
        width: image.width,
        height: image.height,
        data,
    }
}

/// How unlike what the flow says it came from a frame's pixels may be, on average under the matte, the
/// three channels added up, before the subject is looked for elsewhere. A subject the flow keeps up with
/// comes to 0.01 to 0.05 on textured footage, turning or growing fast or under a shutter to 0.14, and one
/// it has lost to 0.09 to 0.7, since what its matte sits on then came from where the subject was.
const FLOW_LOST: f64 = 0.08;

/// How many pixels are read for that: a few thousand say it as well as all of them.
const FLOW_READ: usize = 2500;

/// Whether the flow has lost the subject in `matte`, going by how unlike what it says each pixel under
/// the matte came from that pixel is. It takes what a flow already found and less than a millisecond.
fn flow_lost(previous: &Picture, matte: &[u8], next: &Picture, flow: &Flow) -> bool {
    let (w, h) = (next.width, next.height);
    let stride = (((w * h) as f64 / (FLOW_READ * 4) as f64).sqrt() as usize).max(1);
    let (mut sum, mut count) = (0.0f64, 0usize);
    for y in (0..h).step_by(stride) {
        for x in (0..w).step_by(stride) {
            let v = flow.at(x as f64 + 0.5, y as f64 + 0.5);
            let from_x = (x as f64 + 0.5 + f64::from(v[0])).clamp(0.0, w as f64 - 1.0) as usize;
            let from_y = (y as f64 + 0.5 + f64::from(v[1])).clamp(0.0, h as f64 - 1.0) as usize;
            let from = from_y * w + from_x;
            if matte[from] == 0 {
                continue;
            }
            let (a, b) = (next.rgb[y * w + x], previous.rgb[from]);
            sum += f64::from((a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs());
            count += 1;
        }
    }
    count > 0 && sum / count as f64 > FLOW_LOST
}

/// The matte warped from the frame before, whose subject went `moved`.
fn warped(previous: &Picture, matte: &[u8], next: &Picture, moved: [i64; 2]) -> Vec<f32> {
    let shifted;
    let from = if moved == [0, 0] {
        &previous.image
    } else {
        shifted = shifted_image(&previous.image, moved);
        &shifted
    };
    let flow = block_flow(&next.image, from, 8, 4);
    warped_by(&flow, matte, (next.width, next.height), moved)
}

/// [`warped`], with its flow already found.
fn warped_by(flow: &Flow, matte: &[u8], (w, h): (usize, usize), moved: [i64; 2]) -> Vec<f32> {
    let source: Vec<f32> = matte.iter().map(|v| f32::from(*v)).collect();
    // With the picture moved to where the subject went, what stayed behind has moved by that much, and
    // a block on the subject's edge may go with that rather than with the subject. Going by such a block
    // reads the matte from where the subject was, which drags it further than a cut in a band can bring
    // it back, so what is nearer to that than to the subject says nothing.
    let (went_x, went_y) = (moved[0] as f64, moved[1] as f64);
    (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            let v = flow.at(x, y);
            let (vx, vy) = (f64::from(v[0]), f64::from(v[1]));
            let (vx, vy) = if vx.hypot(vy) > (vx - went_x).hypot(vy - went_y) {
                (0.0, 0.0)
            } else {
                (vx, vy)
            };
            matting::sample(&source, w, h, x + vx - moved[0] as f64, y + vy - moved[1] as f64)
        })
        .collect()
}

/// Pixels deep inside a soft matte, more than `radius` from its edge, or else the matte itself.
fn core_of(soft: &[f32], w: usize, h: usize, radius: f64) -> Vec<bool> {
    let binary: Vec<u8> = soft.iter().map(|v| u8::from(*v >= 0.5)).collect();
    let signed = matting::signed_distance(&binary, w, h);
    let core: Vec<bool> = signed.iter().map(|d| f64::from(*d) > radius).collect();
    if core.contains(&true) {
        core
    } else {
        binary.iter().map(|v| *v != 0).collect()
    }
}

/// How much two mattes agree, intersection over union.
fn iou(a: &[u8], b: &[u8]) -> f64 {
    let (mut both, mut either) = (0usize, 0usize);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x != 0, *y != 0);
        both += usize::from(x && y);
        either += usize::from(x || y);
    }
    if either == 0 {
        1.0
    } else {
        both as f64 / either as f64
    }
}

/// The box around a soft matte, and points deep inside it, for a model's prompt.
fn prompt_from(soft: &[f32], w: usize, h: usize) -> Option<Prompt> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, value) in soft.iter().enumerate() {
        if *value >= 0.5 {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    if x0 > x1 {
        return None;
    }
    let (mx, my) = (
        ((x1 - x0) as f32 * 0.05).max(4.0),
        ((y1 - y0) as f32 * 0.05).max(4.0),
    );
    let binary: Vec<u8> = soft.iter().map(|v| u8::from(*v >= 0.5)).collect();
    let signed = matting::signed_distance(&binary, w, h);
    let deepest = (0..w * h).max_by(|a, b| signed[*a].total_cmp(&signed[*b]))?;
    Some(Prompt {
        points: vec![(
            [(deepest % w) as f32 + 0.5, (deepest / w) as f32 + 0.5],
            true,
        )],
        bbox: Some([
            (x0 as f32 - mx).max(0.0),
            (y0 as f32 - my).max(0.0),
            (x1 as f32 + 1.0 + mx).min(w as f32),
            (y1 as f32 + 1.0 + my).min(h as f32),
        ]),
        mask: Some(soft.to_vec()),
    })
}

/// The next frame's matte from the one before, and how far its subject went, where that is further than
/// a band: the flow keeps up with less, as it has done, and is left to. The search for it (8 of 48 ms a
/// frame at 640x360) only runs where the subject was going fast last frame or the flow has lost it.
fn next_matte(
    previous: &Picture,
    matte: &[u8],
    next: &Picture,
    (fg, bg): (&Gmm, &Gmm),
    model: Option<&dyn MaskModel>,
    was_fast: bool,
) -> (Vec<u8>, [i64; 2]) {
    let (w, h) = (next.width, next.height);
    let base = (w.max(h) as f64 / 40.0).max(4.0);
    let search = || {
        let went = moved_by(previous, matte, next);
        if (went[0] as f64).hypot(went[1] as f64) > base * FLOW_KEEPS_UP {
            went
        } else {
            [0, 0]
        }
    };
    let (moved, soft) = if was_fast {
        let moved = search();
        (moved, warped(previous, matte, next, moved))
    } else {
        let flow = block_flow(&next.image, &previous.image, 8, 4);
        let moved = if flow_lost(previous, matte, next, &flow) {
            search()
        } else {
            [0, 0]
        };
        if moved == [0, 0] {
            (moved, warped_by(&flow, matte, (w, h), moved))
        } else {
            (moved, warped(previous, matte, next, moved))
        }
    };
    let reach = band_of((w, h), moved);
    // A warped matte is uncertain by about the band it is searched in.
    let blurred = matting::gauss(&soft, w, h, (reach / 4.0).max(1.0));
    let from_model = model.and_then(|model| {
        let prompt = prompt_from(&soft, w, h)?;
        let found = model.segment(&next.rgb, w, h, &prompt).ok()?;
        (found.len() == w * h).then_some(found)
    });
    let binary = |values: &[f32]| -> Vec<u8> { values.iter().map(|v| u8::from(*v >= 0.5)).collect() };
    let flowed = binary(&soft);
    let agreed = from_model.filter(|found| iou(&binary(found), &flowed) >= 0.5);
    let mut labels = flowed;
    let signed = matting::signed_distance(&labels, w, h);
    let free: Vec<bool> = signed.iter().map(|d| f64::from(d.abs()) <= reach).collect();
    let hard = vec![0u8; w * h];
    let prior = match &agreed {
        Some(found) => (found.as_slice(), MODEL_PRIOR),
        None => (blurred.as_slice(), PRIOR),
    };
    cut(&next.rgb, w, h, &mut labels, &free, &hard, fg, bg, Some(prior));
    let seeds = core_of(&soft, w, h, reach);
    clean(&mut labels, &seeds, &vec![false; w * h], w, h);
    (labels, moved)
}

/// The edge of the largest piece of a matte, walked around with the piece on one side, as points
/// on pixel corners.
fn edge_of(matte: &[u8], w: usize, h: usize) -> Vec<[f64; 2]> {
    // Its largest piece.
    let mut piece = vec![0u32; w * h];
    let (mut best, mut best_size, mut next) = (0u32, 0usize, 0u32);
    for start in 0..w * h {
        if matte[start] == 0 || piece[start] != 0 {
            continue;
        }
        next += 1;
        piece[start] = next;
        let mut stack = vec![start];
        let mut size = 0;
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if matte[j] != 0 && piece[j] == 0 {
                    piece[j] = next;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        if size > best_size {
            (best, best_size) = (next, size);
        }
    }
    if best_size == 0 {
        return Vec::new();
    }
    let on = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && x < w as i64
            && y < h as i64
            && piece[y as usize * w + x as usize] == best
    };
    // The topmost, then leftmost, pixel: its top left corner is on the outer edge.
    let start = piece
        .iter()
        .position(|value| *value == best)
        .unwrap_or_default();
    let (sx, sy) = ((start % w) as i64, (start / w) as i64);
    // Walking along pixel edges with the piece on the right. Directions: 0 right, 1 down, 2 left,
    // 3 up; `at` is a corner.
    let (mut at, mut heading) = ((sx, sy), 0usize);
    let mut edge = Vec::new();
    let limit = 4 * w * h + 4;
    for _ in 0..limit {
        edge.push([at.0 as f64, at.1 as f64]);
        at = match heading {
            0 => (at.0 + 1, at.1),
            1 => (at.0, at.1 + 1),
            2 => (at.0 - 1, at.1),
            _ => (at.0, at.1 - 1),
        };
        if at == (sx, sy) && heading == 3 {
            break;
        }
        // The two pixels ahead of this corner: to the right of the way and to the left of it.
        let (ahead_right, ahead_left) = match heading {
            0 => (on(at.0, at.1), on(at.0, at.1 - 1)),
            1 => (on(at.0 - 1, at.1), on(at.0, at.1)),
            2 => (on(at.0 - 1, at.1 - 1), on(at.0 - 1, at.1)),
            _ => (on(at.0, at.1 - 1), on(at.0 - 1, at.1 - 1)),
        };
        heading = if ahead_left {
            (heading + 3) % 4
        } else if ahead_right {
            heading
        } else {
            (heading + 1) % 4
        };
    }
    edge
}

/// `count` corners evenly spaced along a closed edge, smoothed so that it does not follow every pixel
/// step, starting from `start` of the way around.
fn resampled(edge: &[[f64; 2]], count: usize, start: f64) -> Vec<[f64; 2]> {
    let n = edge.len();
    if n < 3 {
        return Vec::new();
    }
    // A moving average, about as wide as the spacing asks for, but not so wide as to round a corner away.
    let reach = (n / count / 4).clamp(1, 8);
    let smooth: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            let mut sum = [0.0, 0.0];
            for k in 0..=2 * reach {
                let p = edge[(i + n + k - reach) % n];
                sum[0] += p[0];
                sum[1] += p[1];
            }
            let spread = (2 * reach + 1) as f64;
            [sum[0] / spread, sum[1] / spread]
        })
        .collect();
    let mut lengths = Vec::with_capacity(n + 1);
    lengths.push(0.0);
    for i in 0..n {
        let (a, b) = (smooth[i], smooth[(i + 1) % n]);
        lengths.push(lengths[i] + (b[0] - a[0]).hypot(b[1] - a[1]));
    }
    let total = lengths[n];
    if total <= 0.0 {
        return Vec::new();
    }
    (0..count)
        .map(|k| {
            let at = ((start + k as f64 / count as f64).rem_euclid(1.0)) * total;
            let after = lengths.partition_point(|length| *length <= at).clamp(1, n);
            let (from, to) = (lengths[after - 1], lengths[after]);
            let along = if to > from { (at - from) / (to - from) } else { 0.0 };
            let (a, b) = (smooth[after - 1], smooth[after % n]);
            [a[0] + (b[0] - a[0]) * along, a[1] + (b[1] - a[1]) * along]
        })
        .collect()
}

/// The start around `edge` whose corners lie nearest to the corners of the frame before, so that each
/// moves to its own place rather than sliding around the outline.
fn aligned(edge: &[[f64; 2]], before: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let tries = before.len() * 4;
    let mut best = (f64::INFINITY, Vec::new());
    for attempt in 0..tries {
        let corners = resampled(edge, before.len(), attempt as f64 / tries as f64);
        if corners.len() != before.len() {
            return Vec::new();
        }
        let cost: f64 = corners
            .iter()
            .zip(before)
            .map(|(a, b)| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2))
            .sum();
        if cost < best.0 {
            best = (cost, corners);
        }
    }
    best.1
}

/// Follow what `hints` point at through `count` frames that `frame` gives as `(width, height, rgba)`,
/// with a trained model if one is given. It stops at the frame the object is lost in, and at one
/// `frame` cannot give.
pub fn follow_object(
    count: usize,
    hints: &Hints,
    model: Option<&dyn MaskModel>,
    frame: impl FnMut(usize) -> Option<(usize, usize, Vec<u8>)>,
) -> FollowedObject {
    let traced = trace(count, hints, model, frame);
    match shutter::fit(&traced.raw, &traced.across, traced.shortest_blur()) {
        Some(shutter) => traced.under_a_shutter(shutter),
        None => traced.by_the_measure(),
    }
}

/// What following an object finds, before saying where its edge is.
struct Traced {
    followed: FollowedObject,
    /// The outline of the cut in each frame, in pixels.
    raw: Vec<Vec<[f64; 2]>>,
    /// What the edge was measured to be at each of its corners.
    across: Vec<Vec<soft::Across>>,
    width: usize,
    height: usize,
}

impl Traced {
    /// How long a blur must be to be gone by: a band and a half, below which the measure does as well or
    /// better, as a disk 128 across under a blur of 12 pixels comes within 0.052 of the true alpha by it
    /// and 0.077 by the model.
    fn shortest_blur(&self) -> f64 {
        (self.width.max(self.height) as f64 / 40.0).max(4.0) * 1.5
    }

    fn fraction(&self, corners: &[[f64; 2]]) -> Vec<[f64; 2]> {
        corners.iter().map(|c| [c[0] / self.width as f64, c[1] / self.height as f64]).collect()
    }

    /// An edge's middle is where the fall-off says, which a cut through a soft edge only comes near.
    fn by_the_measure(mut self) -> FollowedObject {
        for (corners, across) in self.raw.iter().zip(soft::settled(&self.across)) {
            let moved: Vec<[f64; 2]> = corners
                .iter()
                .zip(soft::normals(corners))
                .zip(&across)
                .map(|((at, out), edge)| [at[0] + out[0] * edge.out, at[1] + out[1] * edge.out])
                .collect();
            self.followed.outlines.push(self.fraction(&moved));
            self.followed
                .soft
                .push(across.iter().map(|edge| edge.wide / self.width as f64).collect());
        }
        self.followed
    }

    /// An edge's middle is where a shutter puts it, given how far the cut was from there.
    fn under_a_shutter(mut self, shutter: shutter::Shutter) -> FollowedObject {
        let (width, height) = (self.width, self.height);
        for ((corners, go), measured) in self.raw.iter().zip(shutter::going(&self.raw)).zip(&self.across) {
            let frame = shutter.frame(corners, measured, go);
            let half = shutter.half_covered(corners, &frame, width, height);
            let moved = aligned(&edge_of(&half, width, height), corners);
            // Nothing is left where a blur is longer than the subject is wide: it stays as it was cut.
            let moved = if moved.is_empty() { corners.clone() } else { moved };
            self.followed.soft.push(shutter.widths(&moved, &frame).iter().map(|wide| wide / width as f64).collect());
            self.followed.outlines.push(self.fraction(&moved));
        }
        self.followed
    }
}

/// Cut out what `hints` point at in each of `count` frames.
fn trace(
    count: usize,
    hints: &Hints,
    model: Option<&dyn MaskModel>,
    mut frame: impl FnMut(usize) -> Option<(usize, usize, Vec<u8>)>,
) -> Traced {
    let mut followed = FollowedObject::default();
    let nothing = |followed| Traced { followed, raw: Vec::new(), across: Vec::new(), width: 0, height: 0 };
    let Some((width, height, rgba)) = frame(0) else {
        return nothing(followed);
    };
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return nothing(followed);
    }
    let (w, h) = (width as f64, height as f64);
    let mut picture = Picture::of(width, height, &rgba);
    let Some((mut matte, fg, bg)) = first_matte(&picture, hints, model) else {
        followed.lost_at = Some(0);
        return nothing(followed);
    };
    let mut corners = resampled(&edge_of(&matte, width, height), OUTLINE_CORNERS, 0.0);
    if corners.is_empty() {
        followed.lost_at = Some(0);
        return nothing(followed);
    }
    let area_of = |matte: &[u8]| matte.iter().filter(|v| **v != 0).count() as f64 / (w * h);
    let mut area = area_of(&matte);
    let mut raw = vec![corners.clone()];
    let mut across = vec![soft::measured(&picture, &matte, &corners)];
    followed.areas.push(area);
    let mut lost_at = None;
    let mut was_fast = false;
    for index in 1..count {
        let Some((frame_w, frame_h, rgba)) = frame(index) else {
            lost_at = Some(index);
            break;
        };
        if (frame_w, frame_h) != (width, height) {
            lost_at = Some(index);
            break;
        }
        let next = Picture::of(width, height, &rgba);
        let (labels, went) = next_matte(&picture, &matte, &next, (&fg, &bg), model, was_fast);
        was_fast = went != [0, 0];
        let now = area_of(&labels);
        // Its corners go with it, and are matched where it went.
        let before: Vec<[f64; 2]> = corners
            .iter()
            .map(|at| [at[0] + went[0] as f64, at[1] + went[1] as f64])
            .collect();
        let moved = aligned(&edge_of(&labels, width, height), &before);
        // A cut that loses the object or spills over it is no longer following it.
        if moved.is_empty() || now < area * 0.5 || now > area * 2.0 {
            lost_at = Some(index);
            break;
        }
        corners = moved;
        area = now;
        matte = labels;
        picture = next;
        raw.push(corners.clone());
        across.push(soft::measured(&picture, &matte, &corners));
        followed.areas.push(area);
    }
    followed.lost_at = lost_at;
    Traced { followed, raw, across, width, height }
}

/// The frames of a followed outline that shape its way, within `tolerance` of the frame's width:
/// what to write as keyframes.
pub fn simplified_outlines(outlines: &[Vec<[f64; 2]>], tolerance: f64) -> Vec<usize> {
    let Some(first) = outlines.first() else {
        return Vec::new();
    };
    let mut kept = std::collections::BTreeSet::new();
    for corner in 0..first.len() {
        let points: Vec<crate::Point> = outlines
            .iter()
            .map(|outline| crate::Point {
                x: outline[corner][0],
                y: outline[corner][1],
                confidence: 1.0,
            })
            .collect();
        kept.extend(crate::simplified(&points, tolerance));
    }
    kept.into_iter().collect()
}

/// [`simplified_outlines`] of a followed object, its edge's softness included: that matters less than
/// where its corners are, as a blur is forgiving of its width, so it has four times the room.
pub fn simplified_object(followed: &FollowedObject, tolerance: f64) -> Vec<usize> {
    let mut kept: std::collections::BTreeSet<usize> =
        simplified_outlines(&followed.outlines, tolerance)
            .into_iter()
            .collect();
    for corner in 0..followed.soft.first().map_or(0, Vec::len) {
        let widths: Vec<crate::Point> = followed
            .soft
            .iter()
            .map(|soft| crate::Point {
                x: soft[corner],
                y: 0.0,
                confidence: 1.0,
            })
            .collect();
        kept.extend(crate::simplified(&widths, tolerance * 4.0));
    }
    kept.into_iter().collect()
}

#[cfg(test)]
mod tests;
