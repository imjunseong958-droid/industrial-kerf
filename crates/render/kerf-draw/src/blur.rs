//! Gaussian blur by three box passes an axis, as FilmCraft's renderer
//! blurs (`effects.rs`): the same work for any radius.

use crate::image::Image;
use rayon::prelude::*;

/// Blur `image` by a Gaussian of standard deviation `sigma_x` across and
/// `sigma_y` down, in pixels. With `repeat_edge` the picture's edge pixels
/// carry on past it; without, the picture fades into transparency there.
pub fn gaussian(image: &mut Image, sigma_x: f32, sigma_y: f32, repeat_edge: bool) {
    if image.w == 0 || image.h == 0 {
        return;
    }
    let (across, down) = gaussian_radii(image.w, image.h, sigma_x, sigma_y);
    for radius in across {
        box_rows(&mut image.px, image.w, radius, repeat_edge);
    }
    if !down.is_empty() {
        let mut turned = transpose(image);
        for radius in down {
            box_rows(&mut turned.px, turned.w, radius, repeat_edge);
        }
        *image = transpose(&turned);
    }
}

/// The box radii [`gaussian`] passes a `w` × `h` picture through, across and
/// down.
pub fn gaussian_radii(w: usize, h: usize, sigma_x: f32, sigma_y: f32) -> (Vec<usize>, Vec<usize>) {
    // Past a few picture sizes every radius blurs to the same flat picture.
    let cap = |sigma: f32, length: usize| {
        if sigma.is_finite() {
            sigma.min(length.max(8) as f32 * 2.0)
        } else {
            0.0
        }
    };
    let radii = |sigma: f32| {
        if sigma > 0.3 {
            boxes_for_gauss(sigma, 3)
        } else {
            Vec::new()
        }
    };
    (radii(cap(sigma_x, w)), radii(cap(sigma_y, h)))
}

/// Box radii for an `n`-pass box blur close to a Gaussian of `sigma`
/// (Kovesi, after Wells).
fn boxes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    let ideal = ((12.0 * sigma * sigma / n as f32) + 1.0).sqrt();
    let mut lower = ideal.floor() as i32;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let upper = lower + 2;
    let n = n as i32;
    let m_ideal = (12.0 * sigma * sigma
        - (n * lower * lower) as f32
        - 4.0 * n as f32 * lower as f32
        - 3.0 * n as f32)
        / (-4.0 * lower as f32 - 4.0);
    let m = m_ideal.round() as i32;
    (0..n)
        .map(|i| (((if i < m { lower } else { upper }) - 1) / 2).max(0) as usize)
        .collect()
}

fn box_rows(px: &mut [f32], w: usize, r: usize, repeat: bool) {
    if r == 0 {
        return;
    }
    px.par_chunks_mut(w * 4).for_each_init(
        || vec![0f32; w * 4],
        |row_copy, row| {
            row_copy.copy_from_slice(row);
            let inverse = 1.0 / (2 * r + 1) as f32;
            let fetch = |i: isize| -> usize {
                if repeat {
                    i.clamp(0, w as isize - 1) as usize
                } else {
                    i as usize
                }
            };
            let mut sum = [0f32; 4];
            for i in -(r as isize)..=(r as isize) {
                if repeat || (i >= 0 && (i as usize) < w) {
                    let j = fetch(i);
                    for k in 0..4 {
                        sum[k] += row_copy[j * 4 + k];
                    }
                }
            }
            for x in 0..w {
                for k in 0..4 {
                    row[x * 4 + k] = sum[k] * inverse;
                }
                let leaving = x as isize - r as isize;
                let entering = x as isize + r as isize + 1;
                if repeat || leaving >= 0 {
                    let j = fetch(leaving);
                    for k in 0..4 {
                        sum[k] -= row_copy[j * 4 + k];
                    }
                }
                if repeat || (entering as usize) < w {
                    let j = fetch(entering);
                    for k in 0..4 {
                        sum[k] += row_copy[j * 4 + k];
                    }
                }
            }
        },
    );
}

fn transpose(image: &Image) -> Image {
    let (w, h) = (image.w, image.h);
    let mut out = Image::new(h, w);
    const BLOCK: usize = 32;
    out.px
        .par_chunks_mut(h * 4 * BLOCK)
        .enumerate()
        .for_each(|(block, chunk)| {
            let x0 = block * BLOCK;
            let rows = chunk.len() / (h * 4);
            for y in 0..h {
                for dx in 0..rows {
                    let from = (y * w + x0 + dx) * 4;
                    let to = (dx * h + y) * 4;
                    chunk[to..to + 4].copy_from_slice(&image.px[from..from + 4]);
                }
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blurring_keeps_the_light_and_spreads_it() {
        let mut image = Image::new(41, 41);
        let center = (20 * 41 + 20) * 4;
        image.px[center..center + 4].copy_from_slice(&[1.0; 4]);
        gaussian(&mut image, 3.0, 3.0, false);
        let total: f32 = image.px.as_chunks::<4>().0.iter().map(|pixel| pixel[3]).sum();
        assert!((total - 1.0).abs() < 1e-4, "{total}");
        assert!(image.get(20, 20)[3] < 0.1 && image.get(23, 20)[3] > 0.0);
    }

    #[test]
    fn repeated_edges_keep_a_flat_picture_flat() {
        let mut image = Image::filled(16, 9, [0.25, 0.5, 0.75, 1.0]);
        gaussian(&mut image, 5.0, 5.0, true);
        for pixel in image.px.as_chunks::<4>().0 {
            assert!((pixel[1] - 0.5).abs() < 1e-5);
        }
    }
}
