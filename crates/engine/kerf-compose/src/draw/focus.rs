//! Depth of field for a clip in depth: a gather blur by each pixel's circle of confusion.
//!
//! How it is done is carried from EffectCraft's `render/three_d/bokeh.rs`: the picture is blurred at a
//! few radii spanning those it needs, and each pixel blends the two around its own, so a plane turned
//! away from the camera goes softer along its length. Kerf's iris is round, with none of EffectCraft's
//! shapes or highlights. Each clip is blurred alone, before it goes down, so a sharp clip keeps its
//! edges against a soft one behind it. A wide blur gathers at a lower resolution, since it has no
//! detail to lose, which keeps what a frame costs near constant in the radius. Past the frame's edge a
//! picture reads as its edge, or a clip that fills the frame would fade towards it. The GPU does the
//! same in `kerf-gpu/src/focus.wgsl`, from what is here.

use kerf_draw::Image;
use rayon::prelude::*;

/// The most radii a clip is blurred at.
const MOST_LEVELS: usize = 7;
/// How far apart two radii may be, as a ratio, for what is between them to look like either.
const LEVEL_RATIO: f32 = 1.6;
/// A radius this small, in pixels, does nothing to see.
pub(super) const NO_BLUR: f32 = 0.3;
/// A blur gathers at a resolution that leaves it this many pixels of radius, or more.
const GATHERED_RADIUS: f32 = 4.0;
/// Gathered small and grown again, a blur reaches some way past its radius: this much of it, and more.
pub(super) const MARGIN: (f32, f32) = (1.9, 2.0);

/// The radius in pixels of the circle of confusion at `depth`, from the eye: nothing at the focus,
/// and approaching `aperture` far away. EffectCraft's `Dof::coc`, halved to a radius.
pub(super) fn radius_at(depth: f32, focus: f32, aperture: f32, most: f32) -> f32 {
    if !depth.is_finite() || depth <= 0.0 {
        return most;
    }
    (aperture * (depth - focus).abs() / depth).min(most)
}

/// The radii to blur at for those in `radius`: from the least to the most, at no more than
/// [`LEVEL_RATIO`] apart where [`MOST_LEVELS`] allows. EffectCraft's `blur_levels`, spaced by ratio, as
/// a radius of 2 is further from 1 than one of 40 is from 39.
pub(super) fn levels(least: f32, most: f32) -> Vec<f32> {
    if most - least < 0.75 {
        return vec![(least + most) / 2.0];
    }
    // The bottom one is the picture itself when some of it is in focus.
    let (bottom, base) = if least < NO_BLUR {
        (
            Some(0.0),
            NO_BLUR.max(most / LEVEL_RATIO.powi(MOST_LEVELS as i32 - 2)),
        )
    } else {
        (None, least)
    };
    let room = MOST_LEVELS - usize::from(bottom.is_some());
    let steps = ((most / base).ln() / LEVEL_RATIO.ln()).ceil() as usize;
    let count = (steps + 1).clamp(2, room);
    bottom
        .into_iter()
        .chain((0..count).map(|index| base * (most / base).powf(index as f32 / (count - 1) as f32)))
        .collect()
}

/// How much smaller a blur of `radius` is gathered.
pub(super) fn gathered_by(radius: f32) -> usize {
    ((radius / GATHERED_RADIUS).floor() as usize).max(1)
}

/// How far a blur reaches past what it has, whose largest radius there is `reach`.
pub(super) fn margin(reach: f32) -> usize {
    (reach * MARGIN.0).ceil() as usize + MARGIN.1 as usize
}

/// `picture` shrunk by a whole `by`, each pixel the mean of those it covers.
fn shrunk(picture: &Image, by: usize) -> Image {
    let (w, h) = (picture.w.div_ceil(by), picture.h.div_ceil(by));
    let mut out = Image::new(w, h);
    out.px
        .par_chunks_mut(w.max(1) * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let mut sum = [0.0f32; 4];
                let mut count = 0.0f32;
                for sy in y * by..((y + 1) * by).min(picture.h) {
                    for sx in x * by..((x + 1) * by).min(picture.w) {
                        let at = (sy * picture.w + sx) * 4;
                        for (sum, value) in sum.iter_mut().zip(&picture.px[at..at + 4]) {
                            *sum += value;
                        }
                        count += 1.0;
                    }
                }
                for (out, sum) in row[x * 4..x * 4 + 4].iter_mut().zip(sum) {
                    *out = sum / count;
                }
            }
        });
    out
}

/// Each row's running sums, with `room` more either side where the row reads as its end pixel:
/// `sums[room + x]` is the sum of its pixels before `x`, and so its difference from another is what
/// lies between them, wherever they are.
fn running_sums(picture: &Image, room: usize) -> Vec<[f32; 4]> {
    let (w, h) = (picture.w, picture.h);
    let stride = w + 2 * room + 1;
    let mut sums = vec![[0.0f32; 4]; stride * h];
    sums.par_chunks_mut(stride)
        .zip(picture.px.par_chunks(w.max(1) * 4))
        .for_each(|(sums, row)| {
            let first = [row[0], row[1], row[2], row[3]];
            let last = [
                row[(w - 1) * 4],
                row[(w - 1) * 4 + 1],
                row[(w - 1) * 4 + 2],
                row[(w - 1) * 4 + 3],
            ];
            let mut sum = [0.0f32; 4];
            for index in (0..room).rev() {
                for (sum, value) in sum.iter_mut().zip(first) {
                    *sum -= value;
                }
                sums[index] = sum;
            }
            sum = [0.0; 4];
            for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
                for (sum, value) in sum.iter_mut().zip(pixel) {
                    *sum += value;
                }
                sums[room + x + 1] = sum;
            }
            for past in &mut sums[room + w + 1..] {
                for (sum, value) in sum.iter_mut().zip(last) {
                    *sum += value;
                }
                *past = sum;
            }
        });
    sums
}

/// `picture` gathered through a round iris of `radius` pixels, over the rows and columns in `rows`
/// and `columns`, from running sums of its rows, so a pixel costs the kernel's height and not its area.
/// Across a row, the ends of each of the kernel's spans are a fixed way along from the pixel, which
/// leaves a loop with no branches in it.
fn gathered(
    picture: &Image,
    radius: f32,
    rows: std::ops::RangeInclusive<usize>,
    columns: std::ops::RangeInclusive<usize>,
) -> Image {
    if radius < NO_BLUR {
        return picture.clone();
    }
    let (w, h) = (picture.w, picture.h);
    let reach = radius.ceil() as i32;
    let room = reach as usize + 2;
    let stride = w + 2 * room + 1;
    let sums = running_sums(picture, room);
    // Half of each row of the disc: a row's middle stands for the whole of it.
    let spans: Vec<(i32, f32)> = (-reach..=reach)
        .filter_map(|dy| {
            let half = radius * radius - (dy * dy) as f32;
            (half >= 0.0).then(|| (dy, half.sqrt() + 0.5))
        })
        .collect();
    let area: f32 = spans.iter().map(|(_, half)| 2.0 * half).sum();
    let (from, to) = (*columns.start(), *columns.end());
    let mut out = Image::new(w, h);
    out.px
        .par_chunks_mut(w.max(1) * 4)
        .enumerate()
        .skip(*rows.start())
        .take(rows.end() + 1 - rows.start())
        .for_each(|(y, row)| {
            let row = &mut row[from * 4..(to + 1) * 4];
            for (dy, half) in &spans {
                let line = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                let sums = &sums[line * stride..(line + 1) * stride];
                // Where a span's ends are from the first column, in whole pixels and the rest.
                let end = |along: f32| {
                    let at = along + (from + room) as f32;
                    let whole = at.floor();
                    (whole as usize, at - whole)
                };
                let (high, high_part) = end(0.5 + half);
                let (low, low_part) = end(0.5 - half);
                for (x, out) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let (a, b) = (&sums[high + x], &sums[high + x + 1]);
                    let (c, d) = (&sums[low + x], &sums[low + x + 1]);
                    for k in 0..4 {
                        out[k] +=
                            a[k] + (b[k] - a[k]) * high_part - c[k] - (d[k] - c[k]) * low_part;
                    }
                }
            }
            for value in row.iter_mut() {
                *value /= area;
            }
        });
    out
}

/// `picture`, with each pixel blurred by its own `radius` in pixels.
pub(super) fn blurred(picture: &Image, radius: &[f32]) -> Image {
    let (w, h) = (picture.w, picture.h);
    // What it needs is what is near what it has: the rest of the frame has nothing to gather, and its
    // radii, on a plane going off behind the camera, may be anything.
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0, 0);
    let mut reach = 0.0f32;
    for y in 0..h {
        for x in 0..w {
            if picture.px[(y * w + x) * 4 + 3] > 0.0 {
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
                reach = reach.max(radius[y * w + x]);
            }
        }
    }
    if right < left {
        return Image::new(w, h);
    }
    let margin = margin(reach);
    let (left, top) = (left.saturating_sub(margin), top.saturating_sub(margin));
    let (right, bottom) = ((right + margin).min(w - 1), (bottom + margin).min(h - 1));
    let (mut least, mut most) = (f32::MAX, 0.0f32);
    for y in top..=bottom {
        for value in &radius[y * w + left..=y * w + right] {
            least = least.min(*value);
            most = most.max(*value);
        }
    }
    if most < NO_BLUR {
        return picture.clone();
    }
    let levels = levels(least, most);
    let drawn: Vec<(Image, f32)> = levels
        .iter()
        .map(|radius| {
            let by = gathered_by(*radius);
            let small = |at: usize| at / by;
            if by == 1 {
                (gathered(picture, *radius, top..=bottom, left..=right), 1.0)
            } else {
                let shrunk = shrunk(picture, by);
                // A row or column more than it takes, for reading between pixels when it grows again.
                let rows = small(top).saturating_sub(1)..=(small(bottom) + 1).min(shrunk.h - 1);
                let columns = small(left).saturating_sub(1)..=(small(right) + 1).min(shrunk.w - 1);
                (
                    gathered(&shrunk, radius / by as f32, rows, columns),
                    by as f32,
                )
            }
        })
        .collect();
    let read = |level: usize, x: usize, y: usize| {
        let (image, by) = &drawn[level];
        image.sample_bilinear_clamped((x as f32 + 0.5) / by, (y as f32 + 0.5) / by)
    };
    let mut out = Image::new(w, h);
    out.px
        .par_chunks_mut(w.max(1) * 4)
        .enumerate()
        .skip(top)
        .take(bottom + 1 - top)
        .for_each(|(y, row)| {
            for x in left..=right {
                let own = radius[y * w + x];
                let next = levels.partition_point(|level| *level < own);
                let pixel = if next == 0 {
                    read(0, x, y)
                } else if next == levels.len() {
                    read(levels.len() - 1, x, y)
                } else {
                    let (low, high) = (levels[next - 1], levels[next]);
                    let along = (own - low) / (high - low);
                    let (a, b) = (read(next - 1, x, y), read(next, x, y));
                    [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * along)
                };
                row[x * 4..x * 4 + 4].copy_from_slice(&pixel);
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(size: usize) -> Image {
        let mut image = Image::new(size, size);
        let middle = (size / 2 * size + size / 2) * 4;
        image.px[middle..middle + 4].copy_from_slice(&[1.0; 4]);
        image
    }

    fn lit(image: &Image, x: usize, y: usize) -> bool {
        image.px[(y * image.w + x) * 4 + 3] > 1e-4
    }

    #[test]
    fn no_radius_leaves_it_be() {
        let picture = dot(21);
        assert_eq!(blurred(&picture, &vec![0.0; 21 * 21]).px, picture.px);
    }

    #[test]
    fn each_pixel_goes_by_its_own_radius() {
        // A line across the picture, whose left end is sharp and whose right end is not.
        let (w, h) = (101, 41);
        let mut picture = Image::new(w, h);
        for x in 0..w {
            picture.px[(20 * w + x) * 4 + 3] = 1.0;
        }
        let radius: Vec<f32> = (0..w * h).map(|at| (at % w) as f32 * 0.16).collect();
        let out = blurred(&picture, &radius);
        let spread = |x: usize| (0..h).filter(|y| lit(&out, x, *y)).count();
        assert!(spread(2) <= 3, "{}", spread(2));
        assert!(spread(50) > spread(10), "{} {}", spread(50), spread(10));
        assert!(spread(95) > spread(50), "{} {}", spread(95), spread(50));
    }

    #[test]
    fn past_the_edge_it_reads_as_the_edge() {
        let mut picture = Image::new(40, 40);
        picture.px.fill(0.5);
        let out = blurred(&picture, &vec![9.0; 1600]);
        for at in [0, 39, 39 * 40, 1599, 820] {
            assert!(
                (out.px[at * 4] - 0.5).abs() < 0.01,
                "{at}: {}",
                out.px[at * 4]
            );
        }
    }

    #[test]
    fn the_radius_is_nothing_at_the_focus_and_grows_away_from_it() {
        assert_eq!(radius_at(300.0, 300.0, 20.0, 100.0), 0.0);
        assert!((radius_at(600.0, 300.0, 20.0, 100.0) - 10.0).abs() < 1e-3);
        assert!((radius_at(150.0, 300.0, 20.0, 100.0) - 20.0).abs() < 1e-3);
        assert_eq!(radius_at(f32::INFINITY, 300.0, 20.0, 50.0), 50.0);
        assert_eq!(radius_at(1.0, 300.0, 20.0, 50.0), 50.0);
    }

    #[test]
    fn the_levels_span_what_is_asked() {
        assert_eq!(levels(4.0, 4.0), [4.0]);
        let some = levels(1.0, 40.0);
        assert_eq!((some[0], *some.last().unwrap()), (1.0, 40.0));
        assert!(some.len() <= MOST_LEVELS);
        let from_nothing = levels(0.0, 40.0);
        assert_eq!(from_nothing[0], 0.0);
        assert!((*from_nothing.last().unwrap() - 40.0).abs() < 1e-3);
        assert!(from_nothing.len() <= MOST_LEVELS);
        assert!(from_nothing.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
