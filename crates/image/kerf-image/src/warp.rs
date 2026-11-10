//! A mesh warp, for liquify work: a mesh over the picture, dragged by moves in the document, and the
//! picture drawn on the mesh wherever it moved. Each move drags in small steps, so a mesh that moves
//! only by a small part of its radius at a time does not fold over itself, and what was at a move's
//! start ends exactly at its end.

use kerf_draw::Image;
use rayon::prelude::*;

/// The most a move is broken into.
const MOST_STEPS: usize = 4096;
/// The most columns the mesh has.
const MOST_COLUMNS: f32 = 1024.0;
/// Rows of the picture drawn together.
const BAND: usize = 32;

/// One small part of a drag: where it is centered, and how far it goes.
#[derive(Clone, Copy)]
struct Step {
    at: [f32; 2],
    by: [f32; 2],
}

/// The steps of `chain`, as far along it as `amount` says.
fn steps_of(chain: &[[f32; 2]], amount: f32, radius: f32) -> Vec<Step> {
    let length: f32 = chain
        .windows(2)
        .map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
        .sum();
    let mut left = length * amount.clamp(0.0, 1.0);
    if left <= 0.0 {
        return Vec::new();
    }
    let size = (radius / 4.0).max(length / MOST_STEPS as f32);
    let mut steps = Vec::new();
    for pair in chain.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let span = (b[0] - a[0]).hypot(b[1] - a[1]);
        if span <= 0.0 {
            continue;
        }
        let going = span.min(left);
        let count = (going / size).ceil().max(1.0) as usize;
        let along = going / span / count as f32;
        let by = [(b[0] - a[0]) * along, (b[1] - a[1]) * along];
        for step in 0..count {
            let at = [a[0] + by[0] * step as f32, a[1] + by[1] * step as f32];
            steps.push(Step { at, by });
        }
        left -= going;
        if left <= 0.0 {
            break;
        }
    }
    steps
}

/// A mesh that drags: its corners, their rows after each other.
struct Mesh {
    columns: usize,
    rows: usize,
    /// Across and down of one cell.
    cell: [f32; 2],
    corners: Vec<[f32; 2]>,
}

impl Mesh {
    /// A mesh over a picture of `(w, h)`, its cells a good deal smaller than `radius`.
    fn new((w, h): (usize, usize), radius: f32) -> Self {
        let (w, h) = (w as f32, h as f32);
        let wanted = (radius / 8.0).max(w / MOST_COLUMNS).max(2.0);
        let columns = (w / wanted).ceil().max(1.0) as usize;
        let rows = (h / wanted).ceil().max(1.0) as usize;
        let cell = [w / columns as f32, h / rows as f32];
        let corners = (0..=rows)
            .flat_map(|row| {
                (0..=columns).map(move |column| [column as f32 * cell[0], row as f32 * cell[1]])
            })
            .collect();
        Self {
            columns,
            rows,
            cell,
            corners,
        }
    }

    /// Where a corner started.
    fn rest(&self, index: usize) -> [f32; 2] {
        let across = self.columns + 1;
        [
            (index % across) as f32 * self.cell[0],
            (index / across) as f32 * self.cell[1],
        ]
    }

    /// Follow `steps`, each moving what is around it by less the further it is, then keep the mesh's
    /// edges on the picture's, so that no hole opens at them.
    fn drag(&mut self, steps: &[Step], radius: f32) {
        let square = radius * radius;
        self.corners.par_iter_mut().for_each(|corner| {
            for step in steps {
                let (dx, dy) = (corner[0] - step.at[0], corner[1] - step.at[1]);
                let near = dx * dx + dy * dy;
                if near < square {
                    let fall = 1.0 - near / square;
                    let weight = fall * fall;
                    corner[0] += step.by[0] * weight;
                    corner[1] += step.by[1] * weight;
                }
            }
        });
        let (columns, rows, cell) = (self.columns, self.rows, self.cell);
        let across = columns + 1;
        for (index, corner) in self.corners.iter_mut().enumerate() {
            let (column, row) = (index % across, index / across);
            if column == 0 || column == columns {
                corner[0] = column as f32 * cell[0];
            }
            if row == 0 || row == rows {
                corner[1] = row as f32 * cell[1];
            }
        }
    }

    /// The triangles of the cells that moved, each corner with where it started.
    fn moved_triangles(&self) -> Vec<[([f32; 2], [f32; 2]); 3]> {
        const STILL: f32 = 1e-4;
        let across = self.columns + 1;
        let held = |index: usize| {
            let (corner, rest) = (self.corners[index], self.rest(index));
            (corner[0] - rest[0]).abs() < STILL && (corner[1] - rest[1]).abs() < STILL
        };
        let mut triangles = Vec::new();
        for row in 0..self.rows {
            for column in 0..self.columns {
                let a = row * across + column;
                let (b, c, d) = (a + 1, a + across, a + across + 1);
                if held(a) && held(b) && held(c) && held(d) {
                    continue;
                }
                let vertex = |index: usize| (self.corners[index], self.rest(index));
                triangles.push([vertex(a), vertex(b), vertex(c)]);
                triangles.push([vertex(b), vertex(d), vertex(c)]);
            }
        }
        triangles
    }
}

/// Draw a triangle of `source` into the rows of a band starting at row `first`.
fn draw_triangle(
    band: &mut [f32],
    first: usize,
    w: usize,
    source: &Image,
    triangle: &[([f32; 2], [f32; 2]); 3],
) {
    let [(a, sa), (b, sb), (c, sc)] = *triangle;
    let widen = |p: [f32; 2]| [f64::from(p[0]), f64::from(p[1])];
    let (a, b, c) = (widen(a), widen(b), widen(c));
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if area.abs() < 1e-9 {
        return;
    }
    let rows = band.len() / (w * 4);
    let low = |one: f64, two: f64, three: f64| one.min(two).min(three).floor().max(0.0) as usize;
    let high = |one: f64, two: f64, three: f64, most: usize| {
        (one.max(two).max(three).ceil().max(0.0) as usize).min(most)
    };
    let (x0, x1) = (low(a[0], b[0], c[0]), high(a[0], b[0], c[0], w));
    let (y0, y1) = (
        low(a[1], b[1], c[1]).max(first),
        high(a[1], b[1], c[1], first + rows),
    );
    // Pixels on an edge the triangles share go to both, which agree on them, so that none is missed.
    const ON_EDGE: f64 = -1e-7;
    for y in y0..y1 {
        let py = y as f64 + 0.5;
        for x in x0..x1 {
            let px = x as f64 + 0.5;
            let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
            let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < ON_EDGE || w1 < ON_EDGE || w2 < ON_EDGE {
                continue;
            }
            let (w0, w1, w2) = (w0 as f32, w1 as f32, w2 as f32);
            let sx = w0 * sa[0] + w1 * sb[0] + w2 * sc[0];
            let sy = w0 * sa[1] + w1 * sb[1] + w2 * sc[1];
            let start = ((y - first) * w + x) * 4;
            band[start..start + 4].copy_from_slice(&source.sample_bilinear_clamped(sx, sy));
        }
    }
}

/// Warp `picture` by `chains` of points in its pixels, each dragging what is at its first point
/// through the rest, and `radius` around it, `amount` of the way along.
pub fn warp(picture: &mut Image, chains: &[Vec<[f32; 2]>], radius: f32, amount: f32) {
    if radius <= 0.0 || picture.w == 0 || picture.h == 0 {
        return;
    }
    let steps: Vec<Step> = chains
        .iter()
        .flat_map(|chain| steps_of(chain, amount, radius))
        .collect();
    if steps.is_empty() {
        return;
    }
    let mut mesh = Mesh::new((picture.w, picture.h), radius);
    mesh.drag(&steps, radius);
    let triangles = mesh.moved_triangles();
    if triangles.is_empty() {
        return;
    }
    let source = picture.clone();
    let w = picture.w;
    picture
        .px
        .par_chunks_mut(w * 4 * BAND)
        .enumerate()
        .for_each(|(index, band)| {
            let first = index * BAND;
            let last = (first + BAND) as f32;
            for triangle in &triangles {
                let ys = [triangle[0].0[1], triangle[1].0[1], triangle[2].0[1]];
                let (low, high) = (ys[0].min(ys[1]).min(ys[2]), ys[0].max(ys[1]).max(ys[2]));
                if high >= first as f32 && low <= last {
                    draw_triangle(band, first, w, &source, triangle);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAY: [f32; 4] = [0.3, 0.3, 0.3, 1.0];

    fn with_dot(w: usize, h: usize, at: [usize; 2], size: usize) -> Image {
        let mut picture = Image::filled(w, h, GRAY);
        for y in at[1] - size..at[1] + size {
            for x in at[0] - size..at[0] + size {
                let start = (y * w + x) * 4;
                picture.px[start..start + 4].copy_from_slice(&[1.0; 4]);
            }
        }
        picture
    }

    #[test]
    fn nothing_moves_at_no_amount_or_outside_the_radius() {
        let original = with_dot(200, 120, [60, 60], 3);
        let mut still = original.clone();
        warp(&mut still, &[vec![[60.0, 60.0], [100.0, 75.0]]], 50.0, 0.0);
        assert_eq!(still, original);
        let mut picture = with_dot(200, 120, [20, 20], 3);
        let before = picture.clone();
        warp(
            &mut picture,
            &[vec![[140.0, 80.0], [150.0, 80.0]]],
            20.0,
            1.0,
        );
        for y in 0..120 {
            for x in 0..200 {
                if (x as f32 - 145.0).hypot(y as f32 - 80.0) > 40.0 {
                    assert_eq!(picture.get(x, y), before.get(x, y), "{x},{y}");
                }
            }
        }
    }

    #[test]
    fn a_gradient_stays_in_order_so_the_mesh_has_not_folded() {
        let (w, h) = (160, 90);
        let mut picture = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let start = (y * w + x) * 4;
                picture.px[start..start + 4].copy_from_slice(&[x as f32 / w as f32, 0.0, 0.0, 1.0]);
            }
        }
        // A long drag along the rows, with a small radius.
        warp(
            &mut picture,
            &[vec![[40.0, 45.0], [120.0, 45.0]]],
            20.0,
            1.0,
        );
        for x in 1..w {
            let (here, before) = (picture.get(x, 45)[0], picture.get(x - 1, 45)[0]);
            assert!(here >= before - 1e-3, "{x}: {before} then {here}");
        }
    }

    #[test]
    fn its_cells_are_fine_enough_for_a_small_radius_and_not_too_many() {
        let small = Mesh::new((3840, 2160), 12.0);
        assert!(
            small.columns <= 1024 && small.cell[0] < 4.0,
            "{}",
            small.columns
        );
        let large = Mesh::new((3840, 2160), 400.0);
        assert!(large.columns < 100, "{}", large.columns);
        let steps = steps_of(&[[0.0, 0.0], [1.0e6, 0.0]], 1.0, 1.0);
        assert!(steps.len() <= MOST_STEPS + 1);
    }
}
