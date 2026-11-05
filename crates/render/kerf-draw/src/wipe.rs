//! The coverage of Kerf's `wipe` transition, as the graph draws it.
//!
//! The front is a line across the incoming picture, moving along `angle`
//! (degrees, 0 left to right, 90 top to bottom) in fractions of the
//! picture's width and height. At `progress` 0 nothing of the picture
//! shows and at 1 all of it; `softness` (a fraction of the way across)
//! fades the front.

use rayon::prelude::*;

/// How much of each pixel of a `w` × `h` incoming picture shows.
pub fn coverage(w: usize, h: usize, angle: f64, softness: f64, progress: f64) -> Vec<f32> {
    let (dy, dx) = angle.to_radians().sin_cos();
    let low = dx.min(0.0) + dy.min(0.0);
    let span = (dx.max(0.0) + dy.max(0.0) - low).max(1e-6);
    let soft = softness.max(0.001);
    let front = progress * (1.0 + soft);
    let mut out = vec![0.0f32; w * h];
    if w == 0 {
        return out;
    }
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let down = y as f64 / h as f64 * dy;
        for (x, covered) in row.iter_mut().enumerate() {
            let position = (x as f64 / w as f64 * dx + down - low) / span;
            *covered = ((front - position) / soft).clamp(0.0, 1.0) as f32;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wipe_runs_from_nothing_to_everything() {
        assert!(coverage(8, 4, 0.0, 0.0, 0.0).iter().all(|c| *c == 0.0));
        assert!(coverage(8, 4, 0.0, 0.0, 1.0).iter().all(|c| *c == 1.0));
        // Halfway across from the left: the left half shows.
        let half = coverage(8, 1, 0.0, 0.0, 0.5);
        assert_eq!(&half[..4], &[1.0; 4]);
        assert_eq!(&half[5..], &[0.0; 3]);
        // From the top down.
        let down = coverage(1, 8, 90.0, 0.0, 0.5);
        assert_eq!((down[1], down[6]), (1.0, 0.0));
    }
}
