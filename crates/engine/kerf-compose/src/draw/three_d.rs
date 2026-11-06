//! Clips placed in depth, seen through a camera and lit.
//!
//! A clip with `three_d=true` is still placed by its 2D geometry first; then `z`, `rotate_x`, and
//! `rotate_y` lift that rectangle into a world where x runs right, y down, and z away from the viewer,
//! in the sequence's pixels. The default camera sees the plane z = 0 exactly as the 2D picture, so
//! nothing moves until something is asked to. Each output pixel's ray meets the clip's plane where
//! the plane says, which needs no homography, gives the true perspective, and tells each pixel where
//! it is for the lights. Clips on adjacent 3D tracks draw far to near. Planes that cross each other
//! are not cut where they meet, and shadows and depth of field are still to come.

use super::*;

/// The field of view at which a clip at z = 0 is its 2D self, in degrees.
const DEFAULT_FOV: f64 = 40.0;
/// How near a camera sees, in frame heights.
const NEAR: f64 = 0.001;

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

/// `picture` as a clip in depth on an output picture of `(w, h)`, transparent around it. `to_output`
/// takes its pixels to where its 2D geometry puts them.
pub(super) fn project(
    scene: &Scene,
    plan: &ClipPlan,
    picture: &Image,
    (to_output, crop): (&Affine, Option<Rect>),
    time: Time,
    (w, h): (usize, usize),
) -> Image {
    let clip = &plan.clip;
    let read = |name: &str| number(&param(clip, name).value_at(time));
    let z = read("z") * scene.height;
    let (pitch, yaw) = (read("rotate_x").to_radians(), read("rotate_y").to_radians());
    // Its plane: where the picture's corner is, and how far each picture pixel goes across and down.
    let corner = |x: f64, y: f64| {
        let at = to_output.apply(Vec2::new(x, y));
        scene.world(at.x, at.y, z)
    };
    let (pw, ph) = (picture.w as f64, picture.h as f64);
    let (origin, across, down) = (
        corner(0.0, 0.0),
        minus(corner(1.0, 0.0), corner(0.0, 0.0)),
        minus(corner(0.0, 1.0), corner(0.0, 0.0)),
    );
    let middle = corner(pw / 2.0, ph / 2.0);
    let origin = add(middle, turned(minus(origin, middle), pitch, yaw));
    let (across, down) = (turned(across, pitch, yaw), turned(down, pitch, yaw));
    let plane = cross(across, down);
    let normal = unit(plane);
    let area = dot(plane, plane);
    // Far smaller than it was drawn, it reads from a smaller copy, or it would sparkle.
    let to_eye = dot(minus(middle, scene.eye), scene.forward).max(1.0);
    let shrink =
        (to_eye / scene.focal) / (dot(across, across) * dot(down, down)).powf(0.25).max(1e-6);
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
    out.px
        .par_chunks_mut(w.max(1) * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let (sx, sy) = (
                    x as f64 + 0.5 - scene.center.0,
                    y as f64 + 0.5 - scene.center.1,
                );
                let ray = add(
                    add(times(scene.right, sx), times(scene.down, sy)),
                    times(scene.forward, scene.focal),
                );
                let toward = dot(ray, normal);
                if toward.abs() < 1e-12 {
                    continue;
                }
                let along = dot(minus(origin, scene.eye), normal) / toward;
                if along * dot(ray, scene.forward) < near {
                    continue;
                }
                let hit = add(scene.eye, times(ray, along));
                let from = minus(hit, origin);
                // In the plane's own terms, which need not be square.
                let u = dot(cross(from, down), plane) / area;
                let v = dot(cross(across, from), plane) / area;
                if u < crop.x - 1.0
                    || v < crop.y - 1.0
                    || u > crop.right() + 1.0
                    || v > crop.bottom() + 1.0
                {
                    continue;
                }
                let inside_crop =
                    u >= crop.x && v >= crop.y && u <= crop.right() && v <= crop.bottom();
                let mut pixel = source.sample_bilinear((u * by) as f32, (v * by) as f32);
                if !inside_crop {
                    pixel = [0.0; 4];
                }
                if pixel[3] <= 0.0 {
                    continue;
                }
                if let Some(light) = scene.lit(hit, normal) {
                    for (value, light) in pixel.iter_mut().zip(light) {
                        *value *= light;
                    }
                }
                row[x * 4..x * 4 + 4].copy_from_slice(&pixel);
            }
        });
    out
}
