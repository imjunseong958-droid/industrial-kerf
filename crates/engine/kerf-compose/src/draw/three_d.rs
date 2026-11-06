//! Clips placed in depth, seen through a camera and lit.
//!
//! A clip with `three_d=true` is still placed by its 2D geometry first; then `z`, `rotate_x`, and
//! `rotate_y` lift that rectangle into a world where x runs right, y down, and z away from the viewer,
//! in the sequence's pixels. The default camera sees the plane z = 0 exactly as the 2D picture, so
//! nothing moves until something is asked to. Each output pixel's ray meets the clip's plane where
//! the plane says, which needs no homography, gives the true perspective, and tells each pixel where
//! it is for the lights. Clips on adjacent 3D tracks draw far to near, and where two of them cross
//! the run is drawn pixel by pixel, nearest last, so each is cut where they meet. Shadows and depth of
//! field are still to come.

use super::*;

/// The field of view at which a clip at z = 0 is its 2D self, in degrees.
const DEFAULT_FOV: f64 = 40.0;
/// How near a camera sees, in frame heights.
const NEAR: f64 = 0.001;
/// How much nearer one plane must be than another, in pixels, to go in front of it: less than this
/// is the same place, as clips laid on one plane are, and they stack as their tracks do.
const SAME_DEPTH: f32 = 0.01;

type Vector = [f64; 3];

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn minus(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn times(a: Vector, by: f64) -> Vector {
    [a[0] * by, a[1] * by, a[2] * by]
}

fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: Vector) -> Vector {
    let length = dot(a, a).sqrt();
    if length > 1e-12 {
        times(a, 1.0 / length)
    } else {
        a
    }
}

/// Turn about x by `pitch` and then about y by `yaw`, in radians.
fn turned(a: Vector, pitch: f64, yaw: f64) -> Vector {
    let (s, c) = pitch.sin_cos();
    let [x, y, z] = a;
    let (y, z) = (y * c - z * s, y * s + z * c);
    let (s, c) = yaw.sin_cos();
    [x * c + z * s, y, -x * s + z * c]
}

/// A generator clip's parameter at sequence time `at`, or the catalog's default.
fn generated(catalog_def: Option<&FxDef>, clip: &Clip, name: &str, at: Time) -> Option<Value> {
    let Content::Generator { params, .. } = &clip.content else {
        return None;
    };
    match params.get(name) {
        Some(param) => Some(param.value_at(at - clip.start)),
        None => catalog_def
            .and_then(|def| def.find(name))
            .map(|def| def.default.clone()),
    }
}

fn is_generator(clip: &Clip, id: &str) -> bool {
    matches!(&clip.content, Content::Generator { fx, .. } if fx == id)
}

/// A light on the scene.
enum Light {
    Ambient([f32; 3]),
    /// A position, in output pixels.
    Point(Vector, [f32; 3]),
    /// The way it shines.
    Directional(Vector, [f32; 3]),
}

/// How a scene is seen and lit at a moment, in output pixels.
pub(super) struct Scene {
    eye: Vector,
    right: Vector,
    down: Vector,
    forward: Vector,
    focal: f64,
    center: (f64, f64),
    height: f64,
    lights: Vec<Light>,
}

impl Scene {
    /// The scene of `sequence` at `at`: its topmost camera, or the default one, and its lights.
    pub fn at(sequence: &SequencePlan, at: Time, (w, h): (usize, usize)) -> Self {
        let (width, height) = (w as f64, h as f64);
        let catalog = Catalog::builtin();
        let mut camera = None;
        let mut lights = Vec::new();
        for track in sequence.tracks.iter().filter(|track| track.shows) {
            for shown in track.shown.iter().filter(|shown| shown.shows(at)) {
                let clip = &shown.plan.clip;
                if is_generator(clip, "camera") {
                    camera = Some(clip);
                } else if is_generator(clip, "light") {
                    let def = catalog.fx("light");
                    let read = |name: &str| {
                        generated(def, clip, name, at).and_then(|value| value.as_number())
                    };
                    let intensity = read("intensity").unwrap_or(1.0) as f32;
                    let color = match generated(def, clip, "color", at) {
                        Some(Value::Color(color)) => color,
                        _ => Rgba::WHITE,
                    };
                    let linear = premultiplied(Rgba { a: 255, ..color });
                    let color = [
                        linear[0] * intensity,
                        linear[1] * intensity,
                        linear[2] * intensity,
                    ];
                    let own = |name: &str| number(&param(clip, name).value_at(at - clip.start));
                    let place = [own("x") * width, own("y") * height, own("z") * height];
                    lights.push(match generated(def, clip, "kind", at) {
                        Some(Value::Enum(kind)) if kind == "ambient" => Light::Ambient(color),
                        Some(Value::Enum(kind)) if kind == "directional" => {
                            // Straight on unless it says otherwise.
                            let way = if dot(place, place) > 0.0 {
                                unit(place)
                            } else {
                                [0.0, 0.0, 1.0]
                            };
                            Light::Directional(way, color)
                        }
                        _ => Light::Point(place, color),
                    });
                }
            }
        }
        let def = catalog.fx("camera");
        let read = |name: &str, otherwise: f64| {
            camera
                .and_then(|clip| generated(def, clip, name, at))
                .and_then(|value| value.as_number())
                .unwrap_or(otherwise)
        };
        let distance = |fov: f64| height / 2.0 / (fov.clamp(1.0, 170.0).to_radians() / 2.0).tan();
        let own = |name: &str| {
            camera.map_or(0.0, |clip| {
                number(&param(clip, name).value_at(at - clip.start))
            })
        };
        let eye = [
            own("x") * width,
            own("y") * height,
            own("z") * height - distance(DEFAULT_FOV),
        ];
        let (pitch, yaw, roll) = (
            read("tilt", 0.0).to_radians(),
            read("pan", 0.0).to_radians(),
            read("roll", 0.0).to_radians(),
        );
        let forward = turned([0.0, 0.0, 1.0], pitch, yaw);
        let level_right = turned([1.0, 0.0, 0.0], pitch, yaw);
        let level_down = cross(forward, level_right);
        let (s, c) = roll.sin_cos();
        let right = add(times(level_right, c), times(level_down, s));
        let down = add(times(level_down, c), times(level_right, -s));
        Self {
            eye,
            right,
            down,
            forward,
            focal: distance(read("fov", DEFAULT_FOV)),
            center: (width / 2.0, height / 2.0),
            height,
            lights,
        }
    }

    /// A point of the output picture on the plane z = 0, in the world.
    fn world(&self, x: f64, y: f64, z: f64) -> Vector {
        [x - self.center.0, y - self.center.1, z]
    }

    /// How far away a clip is, along the camera's way, for drawing far to near.
    pub fn depth(&self, plan: &ClipPlan, to_output: &Affine, time: Time) -> f64 {
        let (w, h) = (f64::from(plan.size.0), f64::from(plan.size.1));
        let center = to_output.apply(Vec2::new(w / 2.0, h / 2.0));
        let z = number(&param(&plan.clip, "z").value_at(time)) * self.height;
        dot(
            minus(self.world(center.x, center.y, z), self.eye),
            self.forward,
        )
    }

    fn lit(&self, at: Vector, normal: Vector) -> Option<[f32; 3]> {
        if self.lights.is_empty() {
            return None;
        }
        let mut sum = [0.0f32; 3];
        for light in &self.lights {
            let (color, facing) = match light {
                Light::Ambient(color) => (color, 1.0),
                Light::Point(place, color) => (color, dot(unit(minus(*place, at)), normal).abs()),
                Light::Directional(way, color) => (color, dot(*way, normal).abs()),
            };
            for (sum, channel) in sum.iter_mut().zip(color) {
                *sum += channel * facing as f32;
            }
        }
        Some(sum)
    }
}

/// A clip's plane in the world: where its picture's first pixel is, and how far each of its pixels
/// goes across and down.
pub(super) struct Plane {
    origin: Vector,
    across: Vector,
    down: Vector,
    /// `across` by `down`, and its square, for going from the world to the picture.
    perpendicular: Vector,
    area: f64,
    normal: Vector,
    middle: Vector,
}

impl Plane {
    /// The plane of a clip of `size` pixels at clip time `time`, which `to_output` takes to where its
    /// 2D geometry puts them.
    pub fn of(
        scene: &Scene,
        plan: &ClipPlan,
        size: (f64, f64),
        to_output: &Affine,
        time: Time,
    ) -> Self {
        let clip = &plan.clip;
        let read = |name: &str| number(&param(clip, name).value_at(time));
        let z = read("z") * scene.height;
        let (pitch, yaw) = (read("rotate_x").to_radians(), read("rotate_y").to_radians());
        let corner = |x: f64, y: f64| {
            let at = to_output.apply(Vec2::new(x, y));
            scene.world(at.x, at.y, z)
        };
        let (origin, across, down) = (
            corner(0.0, 0.0),
            minus(corner(1.0, 0.0), corner(0.0, 0.0)),
            minus(corner(0.0, 1.0), corner(0.0, 0.0)),
        );
        let middle = corner(size.0 / 2.0, size.1 / 2.0);
        let origin = add(middle, turned(minus(origin, middle), pitch, yaw));
        let (across, down) = (turned(across, pitch, yaw), turned(down, pitch, yaw));
        let perpendicular = cross(across, down);
        Self {
            origin,
            across,
            down,
            perpendicular,
            area: dot(perpendicular, perpendicular),
            normal: unit(perpendicular),
            middle,
        }
    }

    /// A point of the plane, in the picture's own terms: `u` across and `v` down, in its pixels.
    fn point(&self, u: f64, v: f64) -> Vector {
        add(self.origin, add(times(self.across, u), times(self.down, v)))
    }

    /// Where a point of the plane is in the picture's own terms, which need not be square.
    fn at(&self, hit: Vector) -> (f64, f64) {
        let from = minus(hit, self.origin);
        (
            dot(cross(from, self.down), self.perpendicular) / self.area,
            dot(cross(self.across, from), self.perpendicular) / self.area,
        )
    }

    /// Whether any edge of the rectangle `crop` of this plane goes through that of another.
    fn pierces(&self, crop: Rect, other: &Plane, other_crop: Rect) -> bool {
        let corners = [
            self.point(crop.x, crop.y),
            self.point(crop.right(), crop.y),
            self.point(crop.right(), crop.bottom()),
            self.point(crop.x, crop.bottom()),
        ];
        (0..4).any(|index| {
            let (a, b) = (corners[index], corners[(index + 1) % 4]);
            let side = |point: Vector| dot(minus(point, other.origin), other.normal);
            let (from, to) = (side(a), side(b));
            // On one side of it, or lying in it, an edge does not go through.
            if from * to >= 0.0 {
                return false;
            }
            let (u, v) = other.at(add(a, times(minus(b, a), from / (from - to))));
            u >= other_crop.x
                && u <= other_crop.right()
                && v >= other_crop.y
                && v <= other_crop.bottom()
        })
    }

    /// Whether the rectangles `crop` of this plane and `other_crop` of another cross. Two flat
    /// rectangles cross just where an edge of one goes through the other.
    pub fn crosses(&self, crop: Rect, other: &Plane, other_crop: Rect) -> bool {
        self.pierces(crop, other, other_crop) || other.pierces(other_crop, self, crop)
    }
}

/// `picture` as a clip in depth on an output picture of `(w, h)`, transparent around it. `to_output`
/// takes its pixels to where its 2D geometry puts them.
pub(super) fn project(
    scene: &Scene,
    plan: &ClipPlan,
    picture: &Image,
    placed: (&Affine, Option<Rect>),
    time: Time,
    size: (usize, usize),
) -> Image {
    projected(scene, plan, picture, placed, time, size, false).0
}

/// [`project`], with how far along the camera's way its plane is at each pixel, inside the picture
/// or out of it, in pixels: infinite where the plane is behind the camera or edge on to it.
pub(super) fn project_with_depth(
    scene: &Scene,
    plan: &ClipPlan,
    picture: &Image,
    placed: (&Affine, Option<Rect>),
    time: Time,
    size: (usize, usize),
) -> (Image, Vec<f32>) {
    projected(scene, plan, picture, placed, time, size, true)
}

fn projected(
    scene: &Scene,
    plan: &ClipPlan,
    picture: &Image,
    (to_output, crop): (&Affine, Option<Rect>),
    time: Time,
    (w, h): (usize, usize),
    with_depth: bool,
) -> (Image, Vec<f32>) {
    let (pw, ph) = (picture.w as f64, picture.h as f64);
    let plane = Plane::of(scene, plan, (pw, ph), to_output, time);
    // Far smaller than it was drawn, it reads from a smaller copy, or it would sparkle.
    let to_eye = dot(minus(plane.middle, scene.eye), scene.forward).max(1.0);
    let shrink = (to_eye / scene.focal)
        / (dot(plane.across, plane.across) * dot(plane.down, plane.down))
            .powf(0.25)
            .max(1e-6);
    let (source, by) = if shrink >= 2.0 {
        let by = 1.0 / shrink.min(16.0);
        let small = picture.transformed(
            ((pw * by).ceil() as usize).max(1),
            ((ph * by).ceil() as usize).max(1),
            &Affine::scale(by, by),
        );
        (std::borrow::Cow::Owned(small), by)
    } else {
        (std::borrow::Cow::Borrowed(picture), 1.0)
    };
    let mut out = Image::new(w, h);
    let crop = crop.unwrap_or_else(|| Rect::new(0.0, 0.0, pw, ph));
    let near = NEAR * scene.height;
    let draw_row = |y: usize, row: &mut [f32], mut depth: Option<&mut [f32]>| {
        for x in 0..w {
            let (sx, sy) = (
                x as f64 + 0.5 - scene.center.0,
                y as f64 + 0.5 - scene.center.1,
            );
            let ray = add(
                add(times(scene.right, sx), times(scene.down, sy)),
                times(scene.forward, scene.focal),
            );
            let toward = dot(ray, plane.normal);
            if toward.abs() < 1e-12 {
                continue;
            }
            let along = dot(minus(plane.origin, scene.eye), plane.normal) / toward;
            let ahead = along * dot(ray, scene.forward);
            if ahead < near {
                continue;
            }
            if let Some(depth) = depth.as_deref_mut() {
                depth[x] = ahead as f32;
            }
            let hit = add(scene.eye, times(ray, along));
            let (u, v) = plane.at(hit);
            if u < crop.x - 1.0
                || v < crop.y - 1.0
                || u > crop.right() + 1.0
                || v > crop.bottom() + 1.0
            {
                continue;
            }
            let inside_crop = u >= crop.x && v >= crop.y && u <= crop.right() && v <= crop.bottom();
            let mut pixel = source.sample_bilinear((u * by) as f32, (v * by) as f32);
            if !inside_crop {
                pixel = [0.0; 4];
            }
            if pixel[3] <= 0.0 {
                continue;
            }
            if let Some(light) = scene.lit(hit, plane.normal) {
                for (value, light) in pixel.iter_mut().zip(light) {
                    *value *= light;
                }
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&pixel);
        }
    };
    let row_size = w.max(1) * 4;
    if with_depth {
        let mut depth = vec![f32::INFINITY; w * h];
        out.px
            .par_chunks_mut(row_size)
            .zip(depth.par_chunks_mut(w.max(1)))
            .enumerate()
            .for_each(|(y, (row, depth))| draw_row(y, row, Some(depth)));
        (out, depth)
    } else {
        out.px
            .par_chunks_mut(row_size)
            .enumerate()
            .for_each(|(y, row)| draw_row(y, row, None));
        (out, Vec::new())
    }
}

/// A clip of a run that crosses, ready to go down.
pub(super) struct Layer {
    pub picture: Image,
    pub depth: Vec<f32>,
    pub opacity: f32,
    pub blend: Blend,
}

/// How much of a pixel the nearer of two planes covers in front of the farther: all of it, but for a
/// pixel the line where they meet goes through, where they share it. That is what keeps the line from
/// being jagged.
fn cover(far: &Layer, near: &Layer, (x, y): (usize, usize), (w, h): (usize, usize)) -> f32 {
    let gap = |x: usize, y: usize| far.depth[y * w + x] - near.depth[y * w + x];
    let here = gap(x, y);
    let change = |there: f32| {
        let change = (there - here).abs();
        if change.is_finite() {
            change
        } else {
            0.0
        }
    };
    let across = if x + 1 < w {
        gap(x + 1, y)
    } else if x > 0 {
        gap(x - 1, y)
    } else {
        here
    };
    let down = if y + 1 < h {
        gap(x, y + 1)
    } else if y > 0 {
        gap(x, y - 1)
    } else {
        here
    };
    let change = change(across) + change(down);
    if change < 1e-3 {
        return 1.0;
    }
    (0.5 + here.max(0.0) / change).min(1.0)
}

/// `layers`, far to near as a whole, over `canvas`: at each pixel in the order of their depth there,
/// the nearest last, each with its opacity and blend.
pub(super) fn composite_by_depth(canvas: &mut Image, layers: &[Layer]) {
    let (w, h) = (canvas.w, canvas.h);
    canvas
        .px
        .par_chunks_mut(w.max(1) * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let mut here: Vec<usize> = Vec::with_capacity(layers.len());
            for x in 0..w {
                let at = y * w + x;
                here.clear();
                for (index, layer) in layers.iter().enumerate() {
                    if layer.picture.px[at * 4 + 3] <= 0.0 {
                        continue;
                    }
                    // Past all that is farther, and at the same depth past those too, so that clips
                    // that do not differ stack as their tracks do.
                    let depth = layer.depth[at];
                    let place = here
                        .iter()
                        .position(|other| layers[*other].depth[at] < depth - SAME_DEPTH)
                        .unwrap_or(here.len());
                    here.insert(place, index);
                }
                for (place, index) in here.iter().enumerate() {
                    let layer = &layers[*index];
                    let shared = match place {
                        0 => 1.0,
                        _ => cover(&layers[here[place - 1]], layer, (x, y), (w, h)),
                    };
                    let picture = &layer.picture.px[at * 4..at * 4 + 4];
                    let by = layer.opacity * shared;
                    let source = [
                        picture[0] * by,
                        picture[1] * by,
                        picture[2] * by,
                        picture[3] * by,
                    ];
                    kerf_draw::blend::mix(&mut row[x * 4..x * 4 + 4], source, x, y, layer.blend);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane with its top left at `origin`, 100 pixels across along `across` and 100 down along `down`.
    fn plane(origin: Vector, across: Vector, down: Vector) -> Plane {
        let perpendicular = cross(across, down);
        Plane {
            origin,
            across,
            down,
            perpendicular,
            area: dot(perpendicular, perpendicular),
            normal: unit(perpendicular),
            middle: origin,
        }
    }

    const WHOLE: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 100.0,
    };

    #[test]
    fn rectangles_cross_only_where_they_truly_do() {
        let flat = plane([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        // Edge on through its middle, either way round.
        let through = plane([50.0, 0.0, -50.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        assert!(flat.crosses(WHOLE, &through, WHOLE));
        assert!(through.crosses(WHOLE, &flat, WHOLE));
        // Parallel, and on one plane, they do not.
        let parallel = plane([0.0, 0.0, 10.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!(!flat.crosses(WHOLE, &parallel, WHOLE));
        let same = plane([30.0, 30.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!(!flat.crosses(WHOLE, &same, WHOLE));
        // Their planes meet, but off the end of one of them.
        let beside = plane([150.0, 0.0, -50.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        assert!(!flat.crosses(WHOLE, &beside, WHOLE));
        // Stopping short of it.
        let short = plane([50.0, 0.0, -150.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        assert!(!flat.crosses(WHOLE, &short, WHOLE));
        // What is cropped away of it no longer reaches.
        let cropped = Rect {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 100.0,
        };
        assert!(!flat.crosses(WHOLE, &through, cropped));
        // A small one through the middle of a big one, with no edge of the big one going through it.
        let small = plane([40.0, 40.0, -5.0], [0.0, 0.0, 0.1], [0.0, 0.2, 0.0]);
        assert!(flat.crosses(WHOLE, &small, WHOLE));
    }
}
