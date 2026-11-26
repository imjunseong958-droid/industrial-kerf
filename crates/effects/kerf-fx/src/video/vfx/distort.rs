//! Distort and Transform effects (geometry).

use crate::video::project::EffectInstance;
use rayon::prelude::*;

use super::*;

/// Mosaic (and Mosaic (Legacy)): solid blocks; Softness blends towards a smooth interpolation of
/// the block colours.
pub fn mosaic(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let bx = fv(e, "horizontal", cx).max(1.0) as usize;
    let by = fv(e, "vertical", cx).max(1.0) as usize;
    let soft = (fv(e, "softness", cx) / 100.0).clamp(0.0, 1.0);
    let (w, h) = (img.w, img.h);
    let bw = (w as f32 / bx as f32).max(1.0);
    let bh = (h as f32 / by as f32).max(1.0);
    let (nx, ny) = (
        (w as f32 / bw).ceil() as usize,
        (h as f32 / bh).ceil() as usize,
    );
    let mut small = Image::new(nx.max(1), ny.max(1));
    for j in 0..small.h {
        for i in 0..small.w {
            let p = img.sample_bilinear_clamped((i as f32 + 0.5) * bw, (j as f32 + 0.5) * bh);
            let k = (j * small.w + i) * 4;
            small.px[k..k + 4].copy_from_slice(&p);
        }
    }
    img.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let (u, v) = ((x as f32 + 0.5) / bw, (y as f32 + 0.5) / bh);
                // tile index as in the original Mosaic (pixel x belongs to tile floor(x / bw))
                let (ti, tj) = ((x as f32 / bw) as usize, (y as f32 / bh) as usize);
                let sharp = small.get(ti.min(small.w - 1), tj.min(small.h - 1));
                let p = if soft > 0.0 {
                    let sm = small.sample_bilinear_clamped(u, v);
                    [0, 1, 2, 3].map(|k| sharp[k] + (sm[k] - sharp[k]) * soft)
                } else {
                    sharp
                };
                row[x * 4..x * 4 + 4].copy_from_slice(&p);
            }
        });
}
