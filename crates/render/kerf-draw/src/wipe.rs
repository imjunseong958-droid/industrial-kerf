//! The coverage of Kerf's `wipe` transition, as the graph draws it.
//!
//! The front is a line across the incoming picture, moving along `angle`
//! (degrees, 0 left to right, 90 top to bottom) in fractions of the
//! picture's width and height. At `progress` 0 nothing of the picture
//! shows and at 1 all of it; `softness` (a fraction of the way across)
//! fades the front.

use crate::Rect;
use rayon::prelude::*;

/// How much of each pixel of a `w` × `h` incoming picture shows.
pub fn coverage(w: usize, h: usize, angle: f64, softness: f64, progress: f64) -> Vec<f32> {
    let whole = Rect::new(0.0, 0.0, w as f64, h as f64);
    coverage_within(w, h, whole, angle, softness, progress)
}

/// [`coverage`] of a `w` × `h` picture whose frame is `frame` of it, which the front goes across. Past
/// the frame it shows as the frame's edge nearest it does, so that nothing shows at the start and
/// all of it at the end.
pub fn coverage_within(
    w: usize,
    h: usize,
    frame: Rect,
    angle: f64,
    softness: f64,
    progress: f64,
) -> Vec<f32> {
    let (dy, dx) = angle.to_radians().sin_cos();
    let low = dx.min(0.0) + dy.min(0.0);
    let span = (dx.max(0.0) + dy.max(0.0) - low).max(1e-6);
    let soft = softness.max(0.001);
    let front = progress * (1.0 + soft);
    let mut out = vec![0.0f32; w * h];
    if w == 0 {
        return out;
    }
    let (fw, fh) = (frame.w.max(1e-9), frame.h.max(1e-9));
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let down = ((y as f64 - frame.y) / fh).clamp(0.0, 1.0) * dy;
        for (x, covered) in row.iter_mut().enumerate() {
            let across = ((x as f64 - frame.x) / fw).clamp(0.0, 1.0);
            let position = (across * dx + down - low) / span;
            *covered = ((front - position) / soft).clamp(0.0, 1.0) as f32;
        }
    });
    out
}
