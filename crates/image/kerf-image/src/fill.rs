//! Filling a hole in a picture from the picture itself. Carried from
//! EffectCraft's raster crate, written there from the published methods:
//!
//! - **Membrane (harmonic) fill**: the hole takes the smoothest values that
//!   match its boundary (Laplace's equation with Dirichlet boundary values),
//!   solved coarse to fine with known-weighted pyramids and Jacobi relaxation.
//! - **PatchMatch** (Barnes, Shechtman, Finkelstein, Goldman 2009)
//!   nearest-neighbour fields inside the completion EM loop of Wexler,
//!   Shechtman and Irani (2007): every patch overlapping the hole is matched
//!   to a fully known patch, hole pixels become the weighted vote of the
//!   overlapping matches, and that repeats over a pyramid, coarse to fine. A level with too many
//!   patches to search takes over what the level above found, which is most of the cost of a big
//!   hole gone and little of the result: see [`Effort`].
//! - **Flow-guided propagation** (the propagation stage of flow-guided video inpainting, e.g. Xu et
//!   al. 2019 and Huang et al. 2016): each missing pixel follows the optical flow, completed inside
//!   the hole from the motion around it, to a frame where that point of the scene shows.

use kerf_raster::flow::{block_flow, Flow};
use kerf_raster::Image;
use rayon::prelude::*;

#[inline]
fn rnd(seed: u32, a: u32, b: u32) -> u32 {
    let mut h =
        seed.wrapping_mul(0x9E37_79B9) ^ a.wrapping_mul(0x85EB_CA6B) ^ b.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^ (h >> 16)
}

/// Grow a hole mask by `r` pixels (square structuring element, separable).
pub fn dilate(hole: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    if r == 0 {
        return hole.to_vec();
    }
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            tmp[y * w + x] = (lo..=hi).any(|k| hole[y * w + k]);
        }
    }
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let lo = y.saturating_sub(r);
            let hi = (y + r).min(h - 1);
            out[y * w + x] = (lo..=hi).any(|k| tmp[k * w + x]);
        }
    }
    out
}

// ------------------------------------------------------------------ membrane

/// Harmonic fill of `data` (`w × h`, 4 channels) where `known` is false.
pub(crate) fn membrane_core(data: &mut [[f32; 4]], known: &[bool], w: usize, h: usize) {
    if !known.iter().any(|k| !*k) {
        return;
    }
    if !known.iter().any(|k| *k) {
        return;
    }
    // Coarse level: known-weighted 2×2 averages.
    if w > 8 && h > 8 {
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut cd = vec![[0.0f32; 4]; cw * ch];
        let mut ck = vec![false; cw * ch];
        for y in 0..ch {
            for x in 0..cw {
                let mut s = [0.0f32; 4];
                let mut n = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (fx, fy) = (2 * x + dx, 2 * y + dy);
                    if fx < w && fy < h && known[fy * w + fx] {
                        let p = data[fy * w + fx];
                        for c in 0..4 {
                            s[c] += p[c];
                        }
                        n += 1.0;
                    }
                }
                if n > 0.0 {
                    cd[y * cw + x] = s.map(|v| v / n);
                    ck[y * cw + x] = true;
                }
            }
        }
        membrane_core(&mut cd, &ck, cw, ch);
        for y in 0..h {
            for x in 0..w {
                if !known[y * w + x] {
                    data[y * w + x] = cd[(y / 2).min(ch - 1) * cw + (x / 2).min(cw - 1)];
                }
            }
        }
    }
    // Jacobi relaxation of the unknown pixels (in place, Gauss–Seidel order).
    let iters = if w * h <= 4096 { 200 } else { 40 };
    let unknown: Vec<usize> = (0..w * h).filter(|&i| !known[i]).collect();
    for _ in 0..iters {
        let mut delta = 0.0f32;
        for &i in &unknown {
            let (x, y) = (i % w, i / w);
            let mut s = [0.0f32; 4];
            let mut n = 0.0;
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < w && ny < h {
                    let p = data[ny * w + nx];
                    for c in 0..4 {
                        s[c] += p[c];
                    }
                    n += 1.0;
                }
            }
            let v = s.map(|v| v / n);
            delta = delta.max((v[0] - data[i][0]).abs());
            data[i] = v;
        }
        if delta < 1e-5 {
            break;
        }
    }
}

/// Membrane fill: `img` with the `hole` pixels replaced by the harmonic interpolation of their
/// border.
pub fn membrane_fill(img: &Image, hole: &[bool]) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = img.clone();
    let known: Vec<bool> = hole.iter().map(|h| !*h).collect();
    membrane_core(&mut out.data, &known, w, h);
    out
}

// ------------------------------------------------------------------ PatchMatch

struct Level {
    w: usize,
    h: usize,
    img: Vec<[f32; 4]>,
    hole: Vec<bool>,
}

fn downsample(l: &Level) -> Level {
    let (w, h) = (l.w.div_ceil(2), l.h.div_ceil(2));
    let mut img = vec![[0.0f32; 4]; w * h];
    let mut hole = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = [0.0f32; 4];
            let mut n = 0.0;
            let mut any_hole = false;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (fx, fy) = ((2 * x + dx).min(l.w - 1), (2 * y + dy).min(l.h - 1));
                let i = fy * l.w + fx;
                if l.hole[i] {
                    any_hole = true;
                } else {
                    for c in 0..4 {
                        s[c] += l.img[i][c];
                    }
                    n += 1.0;
                }
            }
            hole[y * w + x] = any_hole;
            if n > 0.0 {
                img[y * w + x] = s.map(|v| v / n);
            }
        }
    }
    Level { w, h, img, hole }
}

/// Summed-area table of hole pixels, for "is this patch fully known" queries.
struct HoleSat {
    w: usize,
    s: Vec<u32>,
}

impl HoleSat {
    fn new(hole: &[bool], w: usize, h: usize) -> HoleSat {
        let sw = w + 1;
        let mut s = vec![0u32; sw * (h + 1)];
        for y in 0..h {
            for x in 0..w {
                s[(y + 1) * sw + x + 1] =
                    hole[y * w + x] as u32 + s[y * sw + x + 1] + s[(y + 1) * sw + x]
                        - s[y * sw + x];
            }
        }
        HoleSat { w: sw, s }
    }
    /// Hole pixels in [x0, x1) × [y0, y1).
    fn count(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> u32 {
        self.s[y1 * self.w + x1] + self.s[y0 * self.w + x0]
            - self.s[y0 * self.w + x1]
            - self.s[y1 * self.w + x0]
    }
}

struct Pm<'a> {
    w: usize,
    h: usize,
    r: usize,
    img: &'a [[f32; 4]],
    /// For each pixel, whether the patch around it is whole and fully known.
    valid: &'a [bool],
}

impl Pm<'_> {
    fn valid_source(&self, x: i64, y: i64) -> bool {
        x >= 0
            && y >= 0
            && x < self.w as i64
            && y < self.h as i64
            && self.valid[y as usize * self.w + x as usize]
    }
    /// SSD between the patch at target (tx, ty) (clipped to the image) and source (sx, sy).
    fn dist(&self, tx: i64, ty: i64, sx: i64, sy: i64, cap: f32) -> f32 {
        let r = self.r as i64;
        let mut d = 0.0f32;
        let mut n = 0.0f32;
        for dy in -r..=r {
            let (ty2, sy2) = (ty + dy, sy + dy);
            if ty2 < 0 || ty2 >= self.h as i64 {
                continue;
            }
            for dx in -r..=r {
                let tx2 = tx + dx;
                if tx2 < 0 || tx2 >= self.w as i64 {
                    continue;
                }
                let a = self.img[ty2 as usize * self.w + tx2 as usize];
                let b = self.img[sy2 as usize * self.w + (sx + dx) as usize];
                for c in 0..4 {
                    let e = a[c] - b[c];
                    d += e * e;
                }
                n += 1.0;
            }
            if d > cap * n.max(1.0) {
                return f32::INFINITY;
            }
        }
        d / n.max(1.0)
    }
}

/// How hard a level of the pyramid works.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Effort {
    /// `rounds` of searching `sweeps` times for each patch's match and voting with all it overlaps.
    Search { rounds: usize, sweeps: usize },
    /// Take the matches of the level above and copy from them. Where they agree, as they do across
    /// most of a fill, a search here finds what it was given; where they do not, a vote over a whole
    /// patch only averages away the picture's grain, so this one reaches just one pixel around.
    Copy,
}

/// The most patches a level searches, unless it is coarse enough to need to.
const MOST_SEARCHED: usize = 64_000;

/// Whatever the hole's size, a level whose shorter side is no more than this is searched: a patch
/// must see a picture this coarse for its matches to hold the structure of the scene when copied.
const COARSE_ENOUGH: usize = 540;

/// What a level `w` × `h` with this many patches touching the hole does. `rounds` and `sweeps` are
/// what it does when it searches.
fn effort(patches: usize, w: usize, h: usize, rounds: usize, sweeps: usize) -> Effort {
    if patches <= MOST_SEARCHED || w.min(h) <= COARSE_ENOUGH {
        Effort::Search { rounds, sweeps }
    } else {
        Effort::Copy
    }
}

/// One Wexler/PatchMatch level: refine `img` (hole pixels) with what `effort` says, given how many
/// patches it has, starting from the nearest-neighbour field `nnf` (per pixel, source centre;
/// `None` = random).
fn pm_level(
    l: &mut Level,
    nnf: &mut Vec<Option<[i32; 2]>>,
    effort: impl FnOnce(usize) -> Effort,
    patch: usize,
    seed: u32,
) {
    let (w, h) = (l.w, l.h);
    let r = (patch / 2).min((w.min(h).saturating_sub(1)) / 2).max(1);
    // Target centres: pixels whose patch touches the hole.
    let near = dilate(&l.hole, w, h, r);
    let targets: Vec<usize> = (0..w * h).filter(|&i| near[i]).collect();
    if targets.is_empty() {
        return;
    }
    let valid: Vec<bool> = {
        let sat = HoleSat::new(&l.hole, w, h);
        (0..w * h)
            .into_par_iter()
            .map(|i| {
                let (x, y) = (i % w, i / w);
                x >= r
                    && y >= r
                    && x + r < w
                    && y + r < h
                    && sat.count(x - r, y - r, x + r + 1, y + r + 1) == 0
            })
            .collect()
    };
    let effort = effort(targets.len());
    let searching = matches!(effort, Effort::Search { .. });
    // Sources: centres of fully known patches, which only a search picks among.
    let sources: Vec<[i32; 2]> = if searching {
        let sources: Vec<[i32; 2]> = (0..w * h)
            .filter(|&i| valid[i])
            .map(|i| [(i % w) as i32, (i / w) as i32])
            .collect();
        if sources.is_empty() {
            return;
        }
        sources
    } else {
        Vec::new()
    };
    let (rounds, pm_iters, vote) = match effort {
        Effort::Search { rounds, sweeps } => (rounds, sweeps, r as i64),
        Effort::Copy => (1, 0, 1),
    };
    nnf.resize(w * h, None);
    for e in 0..rounds {
        let pm = Pm {
            w,
            h,
            r,
            img: &l.img,
            valid: &valid,
        };
        // Initialise / validate. A copy with no match to take leaves the pixel as it came from the
        // level above.
        let mut field: Vec<([i32; 2], f32)> = vec![([0, 0], f32::INFINITY); w * h];
        targets
            .par_iter()
            .map(|&i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let s = nnf[i]
                    .filter(|s| pm.valid_source(s[0] as i64, s[1] as i64))
                    .or_else(|| {
                        searching.then(|| {
                            sources[rnd(seed ^ e as u32, i as u32, 0) as usize % sources.len()]
                        })
                    });
                let matched = s.map_or(([0, 0], f32::INFINITY), |s| {
                    (s, pm.dist(x, y, s[0] as i64, s[1] as i64, f32::INFINITY))
                });
                (i, matched)
            })
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|(i, v)| field[i] = v);
        let is_target = &near;
        for it in 0..pm_iters {
            let fwd = it % 2 == 0;
            let snap = field.clone();
            let rows: Vec<Vec<(usize, ([i32; 2], f32))>> = (0..h)
                .into_par_iter()
                .map(|y| {
                    let mut row: Vec<([i32; 2], f32)> = snap[y * w..(y + 1) * w].to_vec();
                    let mut out = vec![];
                    let xs: Box<dyn Iterator<Item = usize>> = if fwd {
                        Box::new(0..w)
                    } else {
                        Box::new((0..w).rev())
                    };
                    for x in xs {
                        let i = y * w + x;
                        if !is_target[i] {
                            continue;
                        }
                        let (mut best, mut bd) = row[x];
                        let (xi, yi) = (x as i64, y as i64);
                        let try_c = |c: [i32; 2], best: &mut [i32; 2], bd: &mut f32| {
                            if c != *best && pm.valid_source(c[0] as i64, c[1] as i64) {
                                let d = pm.dist(xi, yi, c[0] as i64, c[1] as i64, *bd);
                                if d < *bd {
                                    *bd = d;
                                    *best = c;
                                }
                            }
                        };
                        // Propagation from the scan-order neighbours.
                        let step: i64 = if fwd { -1 } else { 1 };
                        let nx = xi + step;
                        if nx >= 0 && nx < w as i64 && is_target[y * w + nx as usize] {
                            let s = row[nx as usize].0;
                            try_c([s[0] - step as i32, s[1]], &mut best, &mut bd);
                        }
                        let ny = yi + step;
                        if ny >= 0 && ny < h as i64 && is_target[ny as usize * w + x] {
                            let s = snap[ny as usize * w + x].0;
                            try_c([s[0], s[1] - step as i32], &mut best, &mut bd);
                        }
                        // Random search around the best match, halving the radius.
                        let mut rad = w.max(h) as i64;
                        let mut k = 0u32;
                        while rad >= 1 {
                            let a = rnd(
                                seed.wrapping_add(it as u32 * 977 + e as u32 * 31),
                                i as u32,
                                k,
                            );
                            let ox = (a % (2 * rad as u32 + 1)) as i64 - rad;
                            let oy = ((a >> 16) % (2 * rad as u32 + 1)) as i64 - rad;
                            try_c(
                                [(best[0] as i64 + ox) as i32, (best[1] as i64 + oy) as i32],
                                &mut best,
                                &mut bd,
                            );
                            rad /= 2;
                            k += 1;
                        }
                        // And a random source anywhere (helps escape bad starts).
                        let any = sources[rnd(seed ^ 0xABCD, i as u32, it as u32 + e as u32 * 64)
                            as usize
                            % sources.len()];
                        try_c(any, &mut best, &mut bd);
                        row[x] = (best, bd);
                        out.push((i, (best, bd)));
                    }
                    out
                })
                .collect();
            for row in rows {
                for (i, v) in row {
                    field[i] = v;
                }
            }
        }
        // Vote.
        let mut ds: Vec<f32> = targets
            .iter()
            .map(|&i| field[i].1)
            .filter(|d| d.is_finite())
            .collect();
        ds.sort_by(f32::total_cmp);
        let sigma2 = ds.get(ds.len() * 3 / 4).copied().unwrap_or(1.0).max(1e-5);
        let new: Vec<(usize, [f32; 4])> = (0..w * h)
            .into_par_iter()
            .filter(|&i| l.hole[i])
            .map(|i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let mut acc = [0.0f32; 4];
                let mut wsum = 0.0f32;
                for dy in -vote..=vote {
                    for dx in -vote..=vote {
                        let (px, py) = (x + dx, y + dy);
                        if px < 0 || py < 0 || px >= w as i64 || py >= h as i64 {
                            continue;
                        }
                        let (s, d) = field[py as usize * w + px as usize];
                        if !d.is_finite() {
                            continue;
                        }
                        let (sx, sy) = (s[0] as i64 - dx, s[1] as i64 - dy);
                        let wt = (-d / (2.0 * sigma2)).exp();
                        let v = l.img[sy as usize * w + sx as usize];
                        for c in 0..4 {
                            acc[c] += v[c] * wt;
                        }
                        wsum += wt;
                    }
                }
                (
                    i,
                    if wsum > 0.0 {
                        acc.map(|v| v / wsum)
                    } else {
                        l.img[i]
                    },
                )
            })
            .collect();
        for (i, v) in new {
            l.img[i] = v;
        }
        for &i in &targets {
            nnf[i] = Some(field[i].0);
        }
    }
}

/// Fill the `hole` of `img` with PatchMatch synthesis (multi-scale Wexler EM). `init`: a starting
/// guess for the hole (e.g. the previous frame's fill warped here); with it only the finest
/// scale is refined.
pub fn patch_fill(
    img: &Image,
    hole: &[bool],
    init: Option<&Image>,
    patch: usize,
    seed: u32,
) -> Image {
    patch_fill_by(img, hole, init, patch, seed, effort)
}

/// [`patch_fill`] with what each level does given by `plan`, which [`effort`] is; the tests search
/// every level with it, to see what copying from the level above costs.
fn patch_fill_by(
    img: &Image,
    hole: &[bool],
    init: Option<&Image>,
    patch: usize,
    seed: u32,
    plan: fn(usize, usize, usize, usize, usize) -> Effort,
) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    if !hole.iter().any(|b| *b) || w == 0 || h == 0 {
        return img.clone();
    }
    let patch = patch.max(3) | 1;
    let fine = Level {
        w,
        h,
        img: img.data.clone(),
        hole: hole.to_vec(),
    };
    if let Some(init) = init {
        let mut l = fine;
        for (i, v) in l.img.iter_mut().enumerate() {
            if l.hole[i] {
                *v = init.data[i];
            }
        }
        let mut nnf = vec![];
        let refine = |_| Effort::Search {
            rounds: 2,
            sweeps: 4,
        };
        pm_level(&mut l, &mut nnf, refine, patch, seed);
        return Image {
            width: img.width,
            height: img.height,
            data: l.img,
        };
    }
    // Pyramid: stop while the image is still a few patches across.
    let mut levels = vec![fine];
    while levels.len() < 6 {
        let Some(last) = levels.last() else { break };
        if last.w.min(last.h) / 2 < patch * 4 || !last.hole.iter().any(|b| *b) {
            break;
        }
        let d = downsample(last);
        // Keep at least one fully known patch.
        if !d.hole.iter().any(|b| !*b) {
            break;
        }
        levels.push(d);
    }
    // Coarsest: membrane start.
    let top = levels.len() - 1;
    {
        let l = &mut levels[top];
        let known: Vec<bool> = l.hole.iter().map(|b| !*b).collect();
        membrane_core(&mut l.img, &known, l.w, l.h);
    }
    let mut nnf: Vec<Option<[i32; 2]>> = vec![];
    for k in (0..=top).rev() {
        if k < top {
            // Upsample the coarser result into this level's hole and the NNF (×2).
            let (fine, coarse) = levels.split_at_mut(k + 1);
            let (l, c) = (&mut fine[k], &coarse[0]);
            let (coarse_img, cw, ch) = (&c.img, c.w, c.h);
            let coarse_nnf = std::mem::take(&mut nnf);
            for y in 0..l.h {
                for x in 0..l.w {
                    let i = y * l.w + x;
                    if l.hole[i] {
                        // Bilinear sample of the coarse image.
                        let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (cw - 1) as f32);
                        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (ch - 1) as f32);
                        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
                        let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(ch - 1));
                        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                        let g = |xx: usize, yy: usize| coarse_img[yy * cw + xx];
                        let mut v = [0.0f32; 4];
                        for c in 0..4 {
                            let top = g(x0, y0)[c] + (g(x1, y0)[c] - g(x0, y0)[c]) * tx;
                            let bot = g(x0, y1)[c] + (g(x1, y1)[c] - g(x0, y1)[c]) * tx;
                            v[c] = top + (bot - top) * ty;
                        }
                        l.img[i] = v;
                    }
                }
            }
            nnf = (0..l.w * l.h)
                .map(|i| {
                    let (x, y) = (i % l.w, i / l.w);
                    let ci = (y / 2).min(ch - 1) * cw + (x / 2).min(cw - 1);
                    coarse_nnf
                        .get(ci)
                        .copied()
                        .flatten()
                        .map(|s| [s[0] * 2 + (x % 2) as i32, s[1] * 2 + (y % 2) as i32])
                })
                .collect();
        }
        let (lw, lh) = (levels[k].w, levels[k].h);
        let level = |patches| {
            if k == top {
                Effort::Search {
                    rounds: 8,
                    sweeps: 5,
                }
            } else {
                plan(patches, lw, lh, 4, 5)
            }
        };
        pm_level(
            &mut levels[k],
            &mut nnf,
            level,
            patch,
            seed.wrapping_add(k as u32 * 101),
        );
    }
    let l = levels.swap_remove(0);
    Image {
        width: img.width,
        height: img.height,
        data: l.img,
    }
}

/// Block flow from `a` to `b` with the tiles that touch either hole replaced by the membrane
/// interpolation of the surrounding motion (flow completion).
fn completed_flow(a: &Image, b: &Image, hole_a: &[bool], hole_b: &[bool]) -> Flow {
    let mut f = block_flow(a, b, 8, 6);
    if f.v.is_empty() {
        return f;
    }
    let (w, bs) = (a.width as usize, f.block as usize);
    let mut known = vec![true; f.v.len()];
    for (i, k) in known.iter_mut().enumerate() {
        let (c, r) = (i % f.cols, i / f.cols);
        // A tile is unreliable when any pixel within it (and a block's margin) is a hole.
        let (x0, y0) = (
            (c * bs).saturating_sub(bs / 2),
            (r * bs).saturating_sub(bs / 2),
        );
        let (x1, y1) = (
            ((c + 1) * bs + bs / 2).min(w),
            ((r + 1) * bs + bs / 2).min(a.height as usize),
        );
        'tile: for y in y0..y1 {
            for x in x0..x1 {
                if hole_a[y * w + x] || hole_b[y * w + x] {
                    *k = false;
                    break 'tile;
                }
            }
        }
    }
    if known.iter().all(|k| !*k) {
        f.v.iter_mut().for_each(|v| *v = [0.0; 2]);
        return f;
    }
    let mut d: Vec<[f32; 4]> = f.v.iter().map(|v| [v[0], v[1], 0.0, 0.0]).collect();
    membrane_core(&mut d, &known, f.cols, f.rows);
    for (v, x) in f.v.iter_mut().zip(d) {
        *v = [x[0], x[1]];
    }
    f
}

/// Trace pixel (x, y) of frame `t` through `flows` (`step` = +1 forward with `fwd`, −1 backward
/// with `bwd`) to the nearest frame where it is visible: (sampled colour, frames travelled).
fn trace(
    t: usize,
    x: f64,
    y: f64,
    step: i64,
    fwd: &[Flow],
    bwd: &[Flow],
    known: &dyn Fn(usize, i64, i64) -> bool,
    frames: &[&Image],
    max: usize,
) -> Option<([f32; 4], usize)> {
    let n = frames.len();
    let (w, h) = (frames[0].width as f64, frames[0].height as f64);
    let (mut px, mut py) = (x, y);
    let mut k = t as i64;
    for d in 1..=max {
        let v = if step > 0 {
            if k as usize + 1 >= n {
                return None;
            }
            fwd[k as usize].at(px, py)
        } else {
            if k == 0 {
                return None;
            }
            bwd[k as usize - 1].at(px, py)
        };
        px += v[0] as f64;
        py += v[1] as f64;
        k += step;
        if px < 0.0 || py < 0.0 || px >= w || py >= h {
            return None;
        }
        // Visible when the bilinear support is known.
        let (x0, y0) = ((px - 0.5).floor() as i64, (py - 0.5).floor() as i64);
        let ok = [(x0, y0), (x0 + 1, y0), (x0, y0 + 1), (x0 + 1, y0 + 1)]
            .iter()
            .all(|&(xx, yy)| {
                known(
                    k as usize,
                    xx.clamp(0, w as i64 - 1),
                    yy.clamp(0, h as i64 - 1),
                )
            });
        if ok {
            return Some((frames[k as usize].sample_bilinear_clamped(px, py), d));
        }
    }
    None
}

/// Fill a run of frames from each other: what a frame's hole hides that another frame shows comes
/// from that frame along the optical flow, and what no frame shows is synthesized once and carried
/// along the flow, so that it holds still from frame to frame. Frames and holes go together, one to
/// a frame, all the same size.
pub fn fill_frames(frames: &[Image], holes: &[Vec<bool>], patch: usize, seed: u32) -> Vec<Image> {
    let n = frames.len();
    if n == 0 {
        return Vec::new();
    }
    let (w, h) = (frames[0].width as usize, frames[0].height as usize);
    // Flows between membrane-filled neighbours, completed inside the holes.
    let pre: Vec<Image> = frames
        .par_iter()
        .zip(holes)
        .map(|(frame, hole)| membrane_fill(frame, hole))
        .collect();
    let pairs: Vec<(Flow, Flow)> = (0..n.saturating_sub(1))
        .into_par_iter()
        .map(|i| {
            (
                completed_flow(&pre[i], &pre[i + 1], &holes[i], &holes[i + 1]),
                completed_flow(&pre[i + 1], &pre[i], &holes[i + 1], &holes[i]),
            )
        })
        .collect();
    let (fwd, bwd): (Vec<Flow>, Vec<Flow>) = pairs.into_iter().unzip();
    // Propagation.
    let refs: Vec<&Image> = frames.iter().collect();
    let known = |k: usize, x: i64, y: i64| !holes[k][y as usize * w + x as usize];
    let propagated: Vec<(Image, Vec<bool>)> = (0..n)
        .into_par_iter()
        .map(|t| {
            let mut image = frames[t].clone();
            let mut left = holes[t].clone();
            let filled: Vec<(usize, [f32; 4])> = (0..w * h)
                .filter(|&i| holes[t][i])
                .filter_map(|i| {
                    let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
                    let ahead = trace(t, x, y, 1, &fwd, &bwd, &known, &refs, n);
                    let behind = trace(t, x, y, -1, &fwd, &bwd, &known, &refs, n);
                    let color = match (ahead, behind) {
                        (Some((a, da)), Some((c, dc))) => {
                            let (wa, wc) = (1.0 / da as f32, 1.0 / dc as f32);
                            std::array::from_fn(|k| (a[k] * wa + c[k] * wc) / (wa + wc))
                        }
                        (Some((a, _)), None) | (None, Some((a, _))) => a,
                        _ => return None,
                    };
                    Some((i, color))
                })
                .collect();
            for (i, color) in filled {
                image.data[i] = color;
                left[i] = false;
            }
            (image, left)
        })
        .collect();
    // Synthesis of what no frame shows, each frame starting from the one before it.
    let mut out: Vec<Image> = Vec::with_capacity(n);
    for t in 0..n {
        let (image, left) = &propagated[t];
        if !left.contains(&true) {
            out.push(image.clone());
            continue;
        }
        let Some(previous) = t.checked_sub(1).and_then(|before| out.get(before)) else {
            out.push(patch_fill(image, left, None, patch, seed));
            continue;
        };
        let mut warped = image.clone();
        for i in (0..w * h).filter(|&i| left[i]) {
            let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            let v = bwd[t - 1].at(x, y);
            warped.data[i] = previous.sample_bilinear_clamped(x + v[0] as f64, y + v[1] as f64);
        }
        out.push(patch_fill(image, left, Some(&warped), patch, seed));
    }
    out.into_iter()
        .zip(holes)
        .map(|(image, hole)| opaque_fill(image, hole))
        .collect()
}

/// Filled pixels are opaque.
pub fn opaque_fill(mut img: Image, hole: &[bool]) -> Image {
    for (p, hl) in img.data.iter_mut().zip(hole) {
        if *hl && p[3] > 1e-6 {
            let a = p[3];
            *p = [p[0] / a, p[1] / a, p[2] / a, 1.0];
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_raster::hash_noise;

    /// Peak signal-to-noise ratio (dB) of the RGB of `a` against `b` over `region` (all pixels when
    /// `None`), for values in 0…1.
    pub fn psnr(a: &Image, b: &Image, region: Option<&[bool]>) -> f64 {
        let (mut se, mut n) = (0.0f64, 0.0f64);
        for i in 0..a.data.len().min(b.data.len()) {
            if region.is_some_and(|r| !r[i]) {
                continue;
            }
            for c in 0..3 {
                let d = (a.data[i][c] - b.data[i][c]) as f64;
                se += d * d;
            }
            n += 3.0;
        }
        if se <= 0.0 || n == 0.0 {
            return 99.0;
        }
        10.0 * (1.0 / (se / n)).log10()
    }

    /// A textured background (periodic stripes + blocky noise), shifted by `dx`.
    fn background(w: u32, h: u32, dx: f32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 + dx;
                let s = 0.5 + 0.3 * (fx * std::f32::consts::TAU / 10.0).sin();
                let n = hash_noise(
                    ((fx.floor() as i64).rem_euclid(20) / 4) as u32,
                    (y % 20) / 4,
                    3,
                );
                img.set(x, y, [s, 0.3 + 0.4 * n, 0.6 - 0.3 * s * n, 1.0]);
            }
        }
        img
    }

    fn cut(img: &Image, hole: &[bool]) -> Image {
        let mut o = img.clone();
        for (p, h) in o.data.iter_mut().zip(hole) {
            if *h {
                *p = [0.0; 4];
            }
        }
        o
    }

    fn rect_hole(w: u32, h: u32, x0: u32, y0: u32, s: u32) -> Vec<bool> {
        (0..w * h)
            .map(|i| (x0..x0 + s).contains(&(i % w)) && (y0..y0 + s).contains(&(i / w)))
            .collect()
    }

    #[test]
    fn patchmatch_beats_the_membrane_on_texture() {
        let (w, h) = (96u32, 72u32);
        let gt = background(w, h, 0.0);
        let hole = rect_hole(w, h, 40, 28, 16);
        let src = cut(&gt, &hole);
        let mem = membrane_fill(&src, &hole);
        let pm = patch_fill(&src, &hole, None, 7, 1);
        let (pm_db, mem_db) = (psnr(&pm, &gt, Some(&hole)), psnr(&mem, &gt, Some(&hole)));
        assert!(
            pm_db > mem_db + 3.0 && pm_db > 20.0,
            "PatchMatch {pm_db:.1} dB vs membrane {mem_db:.1} dB"
        );
        // Known pixels are untouched.
        for i in 0..hole.len() {
            if !hole[i] {
                assert_eq!(pm.data[i], gt.data[i]);
            }
        }
    }

    #[test]
    fn a_level_searches_while_it_can_and_copies_after() {
        let search = Effort::Search {
            rounds: 4,
            sweeps: 5,
        };
        // A box 60% wide at 4K: 3 million patches, 757,000, 192,000, 49,000. The first level at
        // 960 x 540 is searched, for all it has, and the two after it copy.
        assert_eq!(effort(3_007_620, 3840, 2160, 4, 5), Effort::Copy);
        assert_eq!(effort(757_332, 1920, 1080, 4, 5), Effort::Copy);
        assert_eq!(effort(192_060, 960, 540, 4, 5), search);
        // At 1080p, the same.
        assert_eq!(effort(83_000, 1920, 1080, 4, 5), Effort::Copy);
        assert_eq!(effort(23_000, 960, 540, 4, 5), search);
        // A small one is searched all the way down, and draws as it did.
        assert_eq!(effort(23_000, 1920, 1080, 4, 5), search);
        assert_eq!(effort(MOST_SEARCHED, 3840, 2160, 4, 5), search);
        assert_eq!(effort(MOST_SEARCHED + 1, 3840, 2160, 4, 5), Effort::Copy);
    }

    /// `background` with film's grain on it, repeating every 40 pixels so that what the hole hides is
    /// there to be found.
    fn grainy(w: u32, h: u32) -> Image {
        let mut img = background(w, h, 0.0);
        for y in 0..h {
            for x in 0..w {
                let n = hash_noise(x % 40, y % 40, 11) * 0.08;
                let mut p = img.get(i64::from(x), i64::from(y));
                for v in &mut p[..3] {
                    *v += n;
                }
                img.set(x, y, p);
            }
        }
        img
    }

    /// The energy of each pixel off the mean of its neighbours: what a vote averages away.
    fn fine_detail(img: &Image, region: impl Fn(usize) -> bool) -> f64 {
        let (w, h) = (img.width as usize, img.height as usize);
        let (mut sum, mut n) = (0.0, 0.0);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                if !region(y * w + x) {
                    continue;
                }
                let around = (img.data[y * w + x - 1][1]
                    + img.data[y * w + x + 1][1]
                    + img.data[(y - 1) * w + x][1]
                    + img.data[(y + 1) * w + x][1])
                    / 4.0;
                sum += f64::from(img.data[y * w + x][1] - around).powi(2);
                n += 1.0;
            }
        }
        sum / f64::max(n, 1.0)
    }

    #[test]
    fn a_hole_too_big_to_search_finds_the_scene_and_keeps_its_grain() {
        // Big enough that the finest level has too many patches to search, and copies.
        let (w, h) = (1000u32, 560u32);
        let hole = rect_hole(w, h, 350, 130, 300);
        assert!(hole.iter().filter(|b| **b).count() > MOST_SEARCHED);
        let gt = grainy(w, h);
        let filled = patch_fill(&cut(&gt, &hole), &hole, None, 7, 1);
        let db = psnr(&filled, &gt, Some(&hole));
        assert!(db > 30.0, "{db:.1} dB");
        // Known pixels are untouched.
        for i in 0..hole.len() {
            if !hole[i] {
                assert_eq!(filled.data[i], gt.data[i]);
            }
        }
        // Grain on a soft picture, with nothing to find it by: matches disagree, and a vote over a
        // whole patch keeps 64% of what is around it, where this keeps 80%.
        let mut gt = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let n = hash_noise(x, y, 11) * 0.08;
                let v = 0.3 + 0.3 * (x as f32 / 170.0).sin() * (y as f32 / 90.0).cos() + n;
                gt.set(x, y, [v, 0.8 - v, 0.5, 1.0]);
            }
        }
        let filled = patch_fill(&cut(&gt, &hole), &hole, None, 7, 1);
        let inside = fine_detail(&filled, |i| hole[i]);
        let outside = fine_detail(&gt, |i| !hole[i]);
        assert!(inside > outside * 0.72, "{inside} of {outside}");
    }

    /// A frame of the real footage `scripts/fetch-real-footage.sh` fetches, its code values 0 to 1, or
    /// `None` when `KERF_REAL_FOOTAGE` does not say where it is.
    fn footage(name: &str) -> Option<Image> {
        let path = std::path::Path::new(&std::env::var_os("KERF_REAL_FOOTAGE")?).join(name);
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb48le", "pipe:"])
            .output()
            .expect("ffmpeg");
        assert!(decoded.status.success(), "{}", path.display());
        let header = std::fs::read(&path).expect("the frame");
        let size = |at: usize| u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
        let mut img = Image::new(size(16), size(20));
        for (px, rgb) in img.data.iter_mut().zip(decoded.stdout.as_chunks::<6>().0.iter()) {
            let channel = |c: usize| f32::from(u16::from_le_bytes([rgb[c], rgb[c + 1]])) / 65535.0;
            *px = [channel(0), channel(2), channel(4), 1.0];
        }
        Some(img)
    }

    /// Fill on real footage, where what the hole hid is known: a locked-off frame of a freeway, a box
    /// over the stack of tires, the skyline, and the wall, as Kerf fills it and with every level
    /// searched. `KERF_REAL_FOOTAGE=… cargo test --release -p kerf-image real_footage -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "needs scripts/fetch-real-footage.sh"]
    fn real_footage_fill() {
        let search_everywhere = |_: usize, _: usize, _: usize, rounds: usize, sweeps: usize| Effort::Search { rounds, sweeps };
        for size in ["1080", "4k"] {
            let Some(truth) = footage(&format!("traffic/{size}/2874.png")) else {
                eprintln!("KERF_REAL_FOOTAGE is not set");
                return;
            };
            let (w, h) = (truth.width as usize, truth.height as usize);
            for (name, [x0, y0, x1, y1]) in [
                ("tires", [0.70, 0.70, 0.80, 0.88]),
                ("skyline", [0.40, 0.30, 0.50, 0.45]),
                ("wall", [0.30, 0.91, 0.45, 0.99]),
                ("tires and road, 30%", [0.45, 0.45, 0.90, 0.95]),
            ] {
                let hole: Vec<bool> = (0..w * h)
                    .map(|i| {
                        let (x, y) = ((i % w) as f64 / w as f64, (i / w) as f64 / h as f64);
                        (x0..x1).contains(&x) && (y0..y1).contains(&y)
                    })
                    .collect();
                // The picture around it, a box half as wide again, for its detail.
                let (mx, my) = ((x1 - x0) / 4.0, (y1 - y0) / 4.0);
                let around = |i: usize| {
                    let (x, y) = ((i % w) as f64 / w as f64, (i / w) as f64 / h as f64);
                    !hole[i] && (x0 - mx..x1 + mx).contains(&x) && (y0 - my..y1 + my).contains(&y)
                };
                let cut_out = cut(&truth, &hole);
                let start = std::time::Instant::now();
                let kerf = patch_fill(&cut_out, &hole, None, 7, 1);
                let took = start.elapsed().as_secs_f64();
                let searched = patch_fill_by(&cut_out, &hole, None, 7, 1, search_everywhere);
                let detail = |img: &Image| fine_detail(img, |i| hole[i]) / fine_detail(&truth, around);
                println!(
                    "fill {size} {name}: {:.2} dB ({took:.1} s), searched {:.2} dB; detail {:.0}% of around, \
                     searched {:.0}%, truth {:.0}%; {}",
                    psnr(&kerf, &truth, Some(&hole)),
                    psnr(&searched, &truth, Some(&hole)),
                    detail(&kerf) * 100.0,
                    detail(&searched) * 100.0,
                    detail(&truth) * 100.0,
                    if kerf.data == searched.data { "the same bytes" } else { "copied below the search" },
                );
                // As close as the search, and keeping at least what it keeps.
                assert!(psnr(&kerf, &truth, Some(&hole)) > psnr(&searched, &truth, Some(&hole)) - 0.1, "{size} {name}");
                assert!(detail(&kerf) >= detail(&searched) * 0.95, "{size} {name}");
            }
        }
    }

    #[test]
    fn filled_pixels_are_opaque() {
        let mut image = Image::filled(2, 1, [0.25, 0.25, 0.25, 0.5]);
        image.data[1] = [0.25, 0.25, 0.25, 0.5];
        let filled = opaque_fill(image, &[true, false]);
        assert_eq!(filled.data[0], [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(filled.data[1], [0.25, 0.25, 0.25, 0.5]);
    }

    /// A moving object removed from a (static or panning) textured background.
    fn sequence(n: usize, pan: f32) -> (Vec<Image>, Vec<Vec<bool>>, Vec<Image>) {
        let (w, h) = (96u32, 64u32);
        let mut frames = vec![];
        let mut holes = vec![];
        let mut gts = vec![];
        for t in 0..n {
            let gt = background(w, h, t as f32 * pan);
            let hole = rect_hole(w, h, 10 + 8 * t as u32, 22, 18);
            frames.push(cut(&gt, &hole));
            holes.push(hole);
            gts.push(gt);
        }
        (frames, holes, gts)
    }

    #[test]
    fn object_fill_recovers_background_and_is_stable() {
        for pan in [0.0, 1.0] {
            let (frames, holes, gts) = sequence(8, pan);
            let out = fill_frames(&frames, &holes, 7, 1);
            for t in 0..out.len() {
                let db = psnr(&out[t], &gts[t], Some(&holes[t]));
                assert!(db > 26.0, "frame {t} pan {pan}: {db:.1} dB");
                assert_eq!(out[t].data[0][3], 1.0);
            }
            if pan == 0.0 {
                // Temporal consistency: on a static background the filled pixels agree between
                // frames.
                for t in 1..out.len() {
                    let both: Vec<bool> = holes[t]
                        .iter()
                        .zip(&holes[t - 1])
                        .map(|(a, b)| *a || *b)
                        .collect();
                    let db = psnr(&out[t], &out[t - 1], Some(&both));
                    assert!(db > 30.0, "frames {}–{t} differ: {db:.1} dB", t - 1);
                }
            }
        }
    }
}
