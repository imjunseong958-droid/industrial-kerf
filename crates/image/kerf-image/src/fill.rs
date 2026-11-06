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
//!   overlapping matches, and that repeats over a pyramid, coarse to fine.
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
    sat: HoleSat,
}

impl Pm<'_> {
    fn valid_source(&self, x: i64, y: i64) -> bool {
        let r = self.r as i64;
        x >= r
            && y >= r
            && x + r < self.w as i64
            && y + r < self.h as i64
            && self.sat.count(
                (x - r) as usize,
                (y - r) as usize,
                (x + r + 1) as usize,
                (y + r + 1) as usize,
            ) == 0
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

/// One Wexler/PatchMatch level: refine `img` (hole pixels) with `em` EM rounds, starting from the
/// nearest-neighbour field `nnf` (per pixel, source centre; `None` = random).
fn pm_level(
    l: &mut Level,
    nnf: &mut Vec<Option<[i32; 2]>>,
    em: usize,
    pm_iters: usize,
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
    let sat = HoleSat::new(&l.hole, w, h);
    // Sources: centres of fully known patches.
    let sources: Vec<[i32; 2]> = {
        let pm = Pm {
            w,
            h,
            r,
            img: &l.img,
            sat: HoleSat::new(&l.hole, w, h),
        };
        (0..w * h)
            .filter(|&i| pm.valid_source((i % w) as i64, (i / w) as i64))
            .map(|i| [(i % w) as i32, (i / w) as i32])
            .collect()
    };
    if sources.is_empty() {
        return;
    }
    nnf.resize(w * h, None);
    for e in 0..em {
        let pm = Pm {
            w,
            h,
            r,
            img: &l.img,
            sat: HoleSat {
                w: sat.w,
                s: sat.s.clone(),
            },
        };
        // Initialise / validate.
        let mut field: Vec<([i32; 2], f32)> = vec![([0, 0], f32::INFINITY); w * h];
        targets
            .par_iter()
            .map(|&i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let s = nnf[i]
                    .filter(|s| pm.valid_source(s[0] as i64, s[1] as i64))
                    .unwrap_or_else(|| {
                        sources[rnd(seed ^ e as u32, i as u32, 0) as usize % sources.len()]
                    });
                (
                    i,
                    (s, pm.dist(x, y, s[0] as i64, s[1] as i64, f32::INFINITY)),
                )
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
        let ri = r as i64;
        let new: Vec<(usize, [f32; 4])> = (0..w * h)
            .into_par_iter()
            .filter(|&i| l.hole[i])
            .map(|i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let mut acc = [0.0f32; 4];
                let mut wsum = 0.0f32;
                for dy in -ri..=ri {
                    for dx in -ri..=ri {
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
        pm_level(&mut l, &mut nnf, 2, 4, patch, seed);
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
            let (coarse_img, cw, ch, coarse_nnf) = {
                let c = &levels[k + 1];
                (c.img.clone(), c.w, c.h, nnf.clone())
            };
            let l = &mut levels[k];
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
        let em = if k == top { 8 } else { 4 };
        pm_level(
            &mut levels[k],
            &mut nnf,
            em,
            5,
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
    fn membrane_matches_a_linear_ramp() {
        let (w, h) = (40u32, 30u32);
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = x as f32 / w as f32;
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        let hole = rect_hole(w, h, 10, 8, 14);
        let f = membrane_fill(&cut(&img, &hole), &hole);
        assert!(
            psnr(&f, &img, Some(&hole)) > 40.0,
            "{}",
            psnr(&f, &img, Some(&hole))
        );
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
