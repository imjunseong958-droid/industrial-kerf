//! Sparse feature tracking (KLT), clean-room from the published methods:
//!
//! - **corners**: the Shi–Tomasi criterion (Shi & Tomasi, "Good Features to Track", CVPR 1994):
//!   the smaller eigenvalue of the gradient structure tensor summed over a small window, with
//!   grid-based non-maximum suppression and a minimum distance between features;
//! - **tracking**: iterative Lucas–Kanade translation estimation (Lucas & Kanade 1981) run coarse
//!   to fine over an image pyramid (the pyramidal scheme described in Bouguet's 2000 technical
//!   note "Pyramidal Implementation of the Lucas Kanade Feature Tracker"), with a
//!   **forward–backward** consistency check (Kalal et al., "Forward-Backward Error", ICPR 2010) to
//!   drop features that were lost or occluded.
//!
//! Frames are analysed in luma at a reduced resolution ([`GrayPyramid::from_image`] takes an
//! integer box-downsampling factor). Coordinates follow [`Plane`]: pixel `i` covers `[i, i + 1)`.

use kerf_raster::Image;
use rayon::prelude::*;

use crate::plane::Plane;

/// A luma pyramid with gradients per level.
#[derive(Clone, Debug, Default)]
pub struct GrayPyramid {
    pub levels: Vec<Plane>,
    pub grads: Vec<(Plane, Plane)>,
}

/// Luma of `img` averaged over `factor × factor` blocks (straight colour over premultiplied
/// alpha is ignored: tracking looks at the visible pixels).
pub fn luma_plane(img: &Image, factor: usize) -> Plane {
    let f = factor.max(1);
    let w = (img.width as usize).div_ceil(f).max(1);
    let h = (img.height as usize).div_ceil(f).max(1);
    let mut p = Plane::new(w, h, 1);
    if img.is_empty() {
        return p;
    }
    p.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut s = 0.0f32;
            let mut n = 0.0f32;
            for yy in y * f..((y + 1) * f).min(img.height as usize) {
                for xx in x * f..((x + 1) * f).min(img.width as usize) {
                    let px = img.data[yy * img.width as usize + xx];
                    s += 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
                    n += 1.0;
                }
            }
            *o = if n > 0.0 { s / n } else { 0.0 };
        }
    });
    p
}

impl GrayPyramid {
    /// A pyramid of `img` downsampled by `factor`, with `levels` extra half-resolution levels.
    pub fn from_image(img: &Image, factor: usize, levels: usize) -> GrayPyramid {
        Self::from_plane(luma_plane(img, factor), levels)
    }

    pub fn from_plane(base: Plane, levels: usize) -> GrayPyramid {
        let mut lv = vec![base];
        for _ in 0..levels {
            let Some(last) = lv.last() else { break };
            if last.w < 16 || last.h < 16 {
                break;
            }
            let d = last.downsample();
            lv.push(d);
        }
        let grads = lv.par_iter().map(Plane::gradients).collect();
        GrayPyramid { levels: lv, grads }
    }

    pub fn width(&self) -> usize {
        self.levels.first().map(|p| p.w).unwrap_or(0)
    }
    pub fn height(&self) -> usize {
        self.levels.first().map(|p| p.h).unwrap_or(0)
    }
}

/// Lucas–Kanade options.
#[derive(Clone, Copy, Debug)]
pub struct LkOpts {
    /// Window radius (the window is `2r + 1` square).
    pub radius: i32,
    pub iterations: usize,
    /// Stop when the update is below this (pixels).
    pub epsilon: f64,
    /// Reject windows whose structure tensor's smaller eigenvalue (per pixel) is below this.
    pub min_eigen: f64,
    /// Forward–backward error limit (level-0 pixels); 0 = no check.
    pub fb_max: f64,
}

impl Default for LkOpts {
    fn default() -> Self {
        LkOpts { radius: 5, iterations: 20, epsilon: 0.01, min_eigen: 1e-5, fb_max: 1.0 }
    }
}

fn lk_point(a: &GrayPyramid, b: &GrayPyramid, p: [f64; 2], guess: [f64; 2], o: &LkOpts) -> Option<[f64; 2]> {
    let top = a.levels.len().min(b.levels.len()).checked_sub(1)?;
    let r = o.radius;
    let n = ((2 * r + 1) * (2 * r + 1)) as f64;
    let s0 = (1u32 << top) as f64;
    let mut g = [guess[0] / s0, guess[1] / s0];
    let mut win = vec![0.0f32; ((2 * r + 1) * (2 * r + 1)) as usize];
    let mut wgx = win.clone();
    let mut wgy = win.clone();
    let mut v1 = [0.0f32];
    for l in (0..=top).rev() {
        let s = (1u32 << l) as f64;
        let (ia, ib) = (&a.levels[l], &b.levels[l]);
        let (gx, gy) = &a.grads[l];
        let c = [p[0] / s, p[1] / s];
        if c[0] < -1.0 || c[1] < -1.0 || c[0] > ia.w as f64 + 1.0 || c[1] > ia.h as f64 + 1.0 {
            return None;
        }
        let (mut gxx, mut gxy, mut gyy) = (0.0f64, 0.0f64, 0.0f64);
        let mut k = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (c[0] + dx as f64, c[1] + dy as f64);
                ia.sample(x, y, &mut v1);
                win[k] = v1[0];
                gx.sample(x, y, &mut v1);
                wgx[k] = v1[0];
                gy.sample(x, y, &mut v1);
                wgy[k] = v1[0];
                let (ix, iy) = (wgx[k] as f64, wgy[k] as f64);
                gxx += ix * ix;
                gxy += ix * iy;
                gyy += iy * iy;
                k += 1;
            }
        }
        let det = gxx * gyy - gxy * gxy;
        let tr = 0.5 * (gxx + gyy);
        let mineig = (tr - (0.25 * (gxx - gyy).powi(2) + gxy * gxy).sqrt()) / n;
        if mineig < o.min_eigen || det.abs() < 1e-18 {
            return None;
        }
        let mut v = [0.0f64; 2];
        for _ in 0..o.iterations {
            let (mut bx, mut by) = (0.0f64, 0.0f64);
            let mut k = 0;
            for dy in -r..=r {
                for dx in -r..=r {
                    ib.sample(c[0] + g[0] + v[0] + dx as f64, c[1] + g[1] + v[1] + dy as f64, &mut v1);
                    let diff = (win[k] - v1[0]) as f64;
                    bx += diff * wgx[k] as f64;
                    by += diff * wgy[k] as f64;
                    k += 1;
                }
            }
            let nx = (gyy * bx - gxy * by) / det;
            let ny = (gxx * by - gxy * bx) / det;
            v[0] += nx;
            v[1] += ny;
            if !(v[0].is_finite() && v[1].is_finite()) {
                return None;
            }
            if nx * nx + ny * ny < o.epsilon * o.epsilon {
                break;
            }
        }
        if l > 0 {
            g = [2.0 * (g[0] + v[0]), 2.0 * (g[1] + v[1])];
        } else {
            g = [g[0] + v[0], g[1] + v[1]];
        }
    }
    let q = [p[0] + g[0], p[1] + g[1]];
    let l0 = &b.levels[0];
    if q[0] < 0.0 || q[1] < 0.0 || q[0] > l0.w as f64 || q[1] > l0.h as f64 {
        return None;
    }
    Some(q)
}

/// Track level-0 points from pyramid `a` to pyramid `b` (with optional per-point initial
/// displacement guesses). `None` for lost points (untrackable window, left the frame, or failed
/// the forward–backward check).
pub fn track(a: &GrayPyramid, b: &GrayPyramid, pts: &[[f64; 2]], guesses: Option<&[[f64; 2]]>, o: &LkOpts) -> Vec<Option<[f64; 2]>> {
    pts.par_iter()
        .enumerate()
        .map(|(i, p)| {
            let g = guesses.and_then(|g| g.get(i).copied()).unwrap_or([0.0; 2]);
            let q = lk_point(a, b, *p, g, o)?;
            if o.fb_max > 0.0 {
                let back = lk_point(b, a, q, [p[0] - q[0], p[1] - q[1]], o)?;
                if (back[0] - p[0]).powi(2) + (back[1] - p[1]).powi(2) > o.fb_max * o.fb_max {
                    return None;
                }
            }
            Some(q)
        })
        .collect()
}

/// Integer downsampling factor so the longer side is at most `max_side` pixels.
pub fn analysis_factor(w: u32, h: u32, max_side: u32) -> usize {
    let long = w.max(h).max(1);
    long.div_ceil(max_side.max(16)).max(1) as usize
}
