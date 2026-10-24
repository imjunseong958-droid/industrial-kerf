//! A clip's masks cutting its picture, as EffectCraft's renderer cuts a
//! layer with its masks (`effectcraft-render`'s `masks::apply`) before its
//! effects run: each outline is filled, grown by its expansion, feathered,
//! inverted, weighed by its opacity, and combined with the masks before it
//! by its mode.

use effectcraft_geom::{vec2, Mat3};
use effectcraft_keyframe::ShapePath;
use effectcraft_path::{FillRule, Join};
use effectcraft_raster::{gaussian_blur, Image};
use kerf_model::{Catalog, Mask, MaskMode, MaskShape, Param, Time};
use std::sync::{Arc, Mutex};

/// A mask's outline on a picture of `width` × `height` pixels.
pub fn outline(shape: &MaskShape, width: f64, height: f64) -> ShapePath {
    let at = |x: f64, y: f64| [x * width, y * height];
    match shape {
        MaskShape::Rect([l, t, r, b]) => ShapePath::rect(
            at((l + r) / 2.0, (t + b) / 2.0),
            (r - l) * width,
            (b - t) * height,
        ),
        MaskShape::Ellipse([x, y, rx, ry]) => {
            ShapePath::ellipse(at(*x, *y), 2.0 * rx * width, 2.0 * ry * height)
        }
        MaskShape::Polygon { points, closed } => {
            let points: Vec<[f64; 2]> = points.iter().map(|[x, y]| at(*x, *y)).collect();
            ShapePath::polygon(&points, *closed)
        }
    }
}

/// Whether a mask cuts the picture. One that combines by `none`, or an
/// open outline, only gives effects a path.
pub fn cuts(mask: &Mask) -> bool {
    let closed = match &mask.shape {
        MaskShape::Polygon { closed, .. } => *closed,
        MaskShape::Rect(_) | MaskShape::Ellipse(_) => true,
    };
    mask.mode != MaskMode::None && closed
}

/// The masks of a clip that cut its picture.
pub(crate) struct Cut {
    masks: Vec<Mask>,
    layer_size: [f64; 2],
    /// The coverage last drawn, by picture size, while no mask animates.
    still: Option<Mutex<Option<Drawn>>>,
}

struct Drawn {
    size: (u32, u32),
    coverage: Arc<[f32]>,
}

impl Cut {
    pub fn new(masks: &[Mask], layer_size: [f64; 2]) -> Self {
        let masks: Vec<Mask> = masks.iter().filter(|mask| cuts(mask)).cloned().collect();
        let animated = masks.iter().any(|mask| {
            mask.params
                .values()
                .any(|param| matches!(param, Param::Animated(_)))
        });
        Self {
            masks,
            layer_size,
            still: (!animated).then(|| Mutex::new(None)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    /// Cut a premultiplied picture drawn at `scale` of the layer, at clip
    /// time `at`.
    pub fn apply(&self, image: &mut Image, scale: f64, at: Time) {
        if self.masks.is_empty() {
            return;
        }
        let size = (image.width, image.height);
        let Some(still) = &self.still else {
            image.mul_mask(&self.coverage(size, scale, at));
            return;
        };
        let mut drawn = still
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let coverage = match &*drawn {
            Some(drawn) if drawn.size == size => Arc::clone(&drawn.coverage),
            _ => {
                let coverage: Arc<[f32]> = self.coverage(size, scale, at).into();
                *drawn = Some(Drawn {
                    size,
                    coverage: Arc::clone(&coverage),
                });
                coverage
            }
        };
        drop(drawn);
        image.mul_mask(&coverage);
    }

    /// How much of each pixel the masks keep.
    fn coverage(&self, (width, height): (u32, u32), scale: f64, at: Time) -> Vec<f32> {
        let to_picture = Mat3::scale(vec2(scale, scale));
        // A first mask that subtracts takes from the whole picture.
        let first = self.masks.first().map(|mask| mask.mode);
        let start = if first == Some(MaskMode::Subtract) {
            1.0
        } else {
            0.0
        };
        let mut kept = vec![start; width as usize * height as usize];
        for (index, mask) in self.masks.iter().enumerate() {
            let number = |name: &str| {
                mask.params
                    .get(name)
                    .and_then(|param| param.value_at(at).as_number())
                    .or_else(|| {
                        Catalog::builtin()
                            .mask_prop(name)
                            .and_then(|def| def.default.as_number())
                    })
                    .unwrap_or(0.0)
            };
            let path = effectcraft_path::to_kurbo(&outline(
                &mask.shape,
                self.layer_size[0],
                self.layer_size[1],
            ));
            let expansion = number("expansion");
            let path = if expansion.abs() > 0.01 {
                effectcraft_path::offset::offset_path(&path, expansion, Join::Round, 4.0)
            } else {
                path
            };
            let mut coverage = effectcraft_path::fill_coverage(
                std::slice::from_ref(&path),
                &to_picture,
                width,
                height,
                FillRule::NonZero,
            );
            let feather = number("feather");
            if feather > 0.0 {
                let sigma = feather * scale / 2.0;
                let blurred = gaussian_blur(&coverage.to_image(), sigma, sigma, false);
                coverage = effectcraft_raster::Mask::from_alpha(&blurred);
            }
            let opacity = number("opacity") as f32;
            // The first mask has nothing to intersect with; it covers as
            // Add does.
            let mode = match mask.mode {
                MaskMode::Intersect | MaskMode::Darken if index == 0 => MaskMode::Add,
                mode => mode,
            };
            for (kept, &covered) in kept.iter_mut().zip(&coverage.data) {
                let covered = if mask.inverted {
                    1.0 - covered
                } else {
                    covered
                } * opacity;
                *kept = match mode {
                    MaskMode::Add => *kept + covered - *kept * covered,
                    MaskMode::Subtract => *kept * (1.0 - covered),
                    MaskMode::Intersect => *kept * covered,
                    MaskMode::Lighten => kept.max(covered),
                    MaskMode::Darken => kept.min(covered),
                    MaskMode::Difference => (*kept - covered).abs(),
                    MaskMode::None => *kept,
                };
            }
        }
        kept
    }
}
