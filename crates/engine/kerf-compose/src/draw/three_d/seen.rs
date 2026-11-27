//! What the camera sees of a clip's picture in depth, found once for every pass of a frame that
//! shares it (`crate::draw::share`): how far along the camera's way the clip is at each pixel of
//! the output picture, and where in its picture each pixel reads and how much it takes there. Only
//! the picture's alpha says that, and a pass leaves a picture's alpha as it is, so each pass reads
//! its own picture by it, laid and lit as [`Looked::draw`] lays it, and comes out as it would drawn
//! alone.

use super::*;

/// What the camera sees of a clip's picture on an output picture of `size`.
pub(in crate::draw) struct Seen {
    /// How far along the camera's way the clip is at each pixel, as [`project_with_depth`] has it.
    pub(super) depth: Vec<f32>,
    rows: Rows,
    size: (usize, usize),
}

enum Rows {
    /// Of a flat picture, each row's pixels it is at, and where in its picture each reads.
    Flat(Vec<Vec<(u32, f32, f32)>>),
    /// Of a solid, what each row's pixels take of its picture.
    Solid(Vec<SolidRow>),
}

/// What a row of pixels takes of a solid's picture: each pixel it covers, by where its steps end
/// among the row's and by how much it is covered, and the steps.
#[derive(Default)]
struct SolidRow {
    pixels: Vec<(u32, u32, f32)>,
    steps: Vec<Step>,
}

impl Reading for SolidRow {
    fn step(&mut self, _: usize, step: &Step, _: &[f32; 4]) {
        self.steps.push(*step);
    }

    fn done(&mut self, x: usize, covered: f32) {
        self.pixels
            .push((x as u32, self.steps.len() as u32, covered));
    }
}

impl Seen {
    /// What the camera sees of the picture `looked` at, of a clip a solid `solid` deep at clip time
    /// `time`, or flat, on an output picture of `(w, h)`.
    pub(super) fn of(
        looked: &Looked,
        plan: &ClipPlan,
        time: Time,
        solid: f64,
        (w, h): (usize, usize),
    ) -> Self {
        let mut depth = vec![f32::INFINITY; w * h];
        let rows = depth.par_chunks_mut(w.max(1)).enumerate();
        let rows = if solid > 0.0 {
            let solid = looked.solid(plan, time, solid);
            Rows::Solid(
                rows.map(|(y, depth)| {
                    let mut row = SolidRow::default();
                    solid.walk_row(looked.scene, y, w, Some(depth), &mut row);
                    row
                })
                .collect(),
            )
        } else {
            Rows::Flat(
                rows.map(|(y, depth)| {
                    let mut row = Vec::new();
                    looked.walk_flat_row(y, w, Some(depth), |x, (u, v), _| {
                        row.push((x as u32, u, v));
                    });
                    row
                })
                .collect(),
            )
        };
        Self {
            depth,
            rows,
            size: (w, h),
        }
    }

    /// About how many bytes it holds.
    pub(in crate::draw) fn bytes(&self) -> usize {
        let rows = match &self.rows {
            Rows::Flat(rows) => rows
                .iter()
                .map(|row| row.len() * std::mem::size_of::<(u32, f32, f32)>())
                .sum(),
            Rows::Solid(rows) => rows
                .iter()
                .map(|row| {
                    row.pixels.len() * std::mem::size_of::<(u32, u32, f32)>()
                        + row.steps.len() * std::mem::size_of::<Step>()
                })
                .sum::<usize>(),
        };
        self.depth.len() * 4 + rows
    }

    /// The picture `looked` at, the clip `id`'s, laid where the camera sees it and lit by the lights
    /// of the scene it is looked at in: as [`Looked::draw`] lays it.
    pub(super) fn shade(&self, looked: &Looked, id: &Id) -> Image {
        let (w, h) = self.size;
        let mut out = Image::new(w, h);
        let (scene, source, plane) = (looked.scene, looked.source, &looked.plane);
        let rows = out.px.par_chunks_mut(w.max(1) * 4).enumerate();
        match &self.rows {
            Rows::Flat(seen) => rows.zip(seen.par_iter()).for_each(|((y, row), seen)| {
                for &(x, u, v) in seen {
                    let x = x as usize;
                    let pixel = source.sample_bilinear(u, v);
                    lay_flat(row, x, pixel, || {
                        if scene.lights.is_empty() {
                            return None;
                        }
                        // Where the ray meets the plane, found again as it was found.
                        scene
                            .meets(plane, (x as f64 + 0.5, y as f64 + 0.5))
                            .and_then(|(hit, _)| scene.lit(id, hit, plane.normal))
                    });
                }
            }),
            Rows::Solid(seen) => rows.zip(seen.par_iter()).for_each(|((_, row), seen)| {
                let mut laid = Laid {
                    scene,
                    id,
                    row,
                    sum: [0.0; 3],
                };
                let mut from = 0;
                for &(x, to, covered) in &seen.pixels {
                    let (x, to) = (x as usize, to as usize);
                    for step in &seen.steps[from..to] {
                        let pixel = source.sample_bilinear(step.at.0, step.at.1);
                        laid.step(x, step, &pixel);
                    }
                    laid.done(x, covered);
                    from = to;
                }
            }),
        }
        out
    }
}
