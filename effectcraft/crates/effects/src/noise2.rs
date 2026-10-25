//! Noise & Grain effects, batch 2: Match Grain (grain statistics measured from another layer
//! and synthesised onto this one), Median (Legacy) and Curl Noise (divergence-free flow
//! distortion from the curl of a fractal noise potential).

use effectcraft_raster::Image;
use rayon::prelude::*;

use crate::util::{Plane, gauss_plane, unpremul};

// ---------------------------------------------------------------- Match Grain

/// Grain statistics of an image: per-channel standard deviation of the high-pass residual
/// (over opaque pixels) and a grain size from the lag-1 autocorrelation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GrainStats {
    pub std: [f32; 3],
    pub size: f32,
}

/// Grain statistics over the pixels `mask` selects (all opaque pixels without one).
pub(crate) fn grain_stats_masked(img: &Image, mask: Option<&[bool]>) -> GrainStats {
    let sel = |i: usize| mask.is_none_or(|m| m[i]);
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 3 || h < 3 {
        return GrainStats::default();
    }
    let straight: Vec<([f32; 3], f32)> = img.data.par_iter().map(|&p| unpremul(p)).collect();
    let mut std = [0.0f32; 3];
    let mut rho_num = 0.0f64;
    let mut rho_den = 0.0f64;
    for (k, s) in std.iter_mut().enumerate() {
        let pl = Plane { w, h, data: straight.iter().map(|(c, _)| c[k]).collect() };
        let low = gauss_plane(&pl, 1.5, 1.5);
        let hp: Vec<f32> = pl.data.iter().zip(low.data.iter()).map(|(a, b)| a - b).collect();
        let (mut sum, mut sq, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for (i, v) in hp.iter().enumerate() {
            if straight[i].1 > 0.5 && sel(i) {
                sum += *v as f64;
                sq += (*v as f64) * (*v as f64);
                n += 1.0;
            }
        }
        if n > 1.0 {
            let mean = sum / n;
            *s = ((sq / n - mean * mean).max(0.0)).sqrt() as f32;
        }
        for y in 0..h {
            for x in 0..w - 1 {
                let i = y * w + x;
                if straight[i].1 > 0.5 && straight[i + 1].1 > 0.5 && sel(i) && sel(i + 1) {
                    rho_num += (hp[i] * hp[i + 1]) as f64;
                    rho_den += (hp[i] * hp[i]) as f64;
                }
            }
        }
    }
    let rho = if rho_den > 1e-12 { (rho_num / rho_den).clamp(0.0, 0.95) } else { 0.0 };
    GrainStats { std, size: (1.0 / (1.0 - rho as f32)).clamp(0.5, 20.0) * 0.6 }
}

// ---------------------------------------------------------------- Median (Legacy)

// ---------------------------------------------------------------- Curl Noise
