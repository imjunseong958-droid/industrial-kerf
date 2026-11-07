//! Clips placed in depth, seen through a camera and lit.
//!
//! A clip with `three_d=true` is still placed by its 2D geometry first; then `z`, `rotate_x`, and
//! `rotate_y` lift that rectangle into a world where x runs right, y down, and z away from the viewer,
//! in the sequence's pixels. The default camera sees the plane z = 0 exactly as the 2D picture, so
//! nothing moves until something is asked to. Each output pixel's ray meets the clip's plane where
//! the plane says, which needs no homography, gives the true perspective, and tells each pixel where
//! it is for the lights. Clips on adjacent 3D tracks draw far to near, and where two of them cross
//! the run is drawn pixel by pixel, nearest last, so each is cut where they meet. A light with a
//! `shadow` sends a ray from each pixel to itself, which every other clip in depth may block by as
//! much as it is opaque there; with a `size`, a fixed spiral of rays across it, so that its edges are
//! soft and hold still. A camera with an `aperture` blurs each clip by how far its plane is from what
//! it focuses on at each pixel ([`super::focus`]).

use super::*;

/// The field of view at which a clip at z = 0 is its 2D self, in degrees.
const DEFAULT_FOV: f64 = 40.0;
/// How near a camera sees, in frame heights.
const NEAR: f64 = 0.001;
/// The widest a clip is blurred, in frame heights: as near the camera as can be, a blur has no end.
const MOST_BLUR: f64 = 0.1;
/// How much nearer one plane must be than another, in pixels, to go in front of it: less than this
/// is the same place, as clips laid on one plane are, and they stack as their tracks do.
const SAME_DEPTH: f32 = 0.01;
/// How many rays go to a light with a `size`. Each reads as much of what is in its way as its share of
/// the light covers, so that they join up in a smooth edge and not one with this many steps.
const RAYS: usize = 32;

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

/// A light on the scene, with how much what is in its way takes of it, from 0 to 1, and how wide it
/// is: 0 for a single ray.
enum Light {
    Ambient([f32; 3]),
    /// A position, in output pixels, and the radius of the disc it is, in the same.
    Point(Vector, [f32; 3], f32, f64),
    /// The way it shines, and the tangent of half the angle it fills.
    Directional(Vector, [f32; 3], f32, f64),
}

/// How far across each ray's share of a light of radius 1 is: the side of a square of its area.
fn share() -> f64 {
    (std::f64::consts::PI / RAYS as f64).sqrt()
}

/// Where on a light of radius 1 each of its rays goes, the same at every pixel and on every frame, so
/// that nothing shimmers: a spiral by the golden angle, whose rays are evenly spread however an edge
/// crosses them, where those on a grid would all go at once at an edge along it.
fn spiral() -> &'static [(f64, f64); RAYS] {
    static SPIRAL: std::sync::OnceLock<[(f64, f64); RAYS]> = std::sync::OnceLock::new();
    SPIRAL.get_or_init(|| {
        let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
        std::array::from_fn(|ray| {
            let out = ((ray as f64 + 0.5) / RAYS as f64).sqrt();
            let (s, c) = (ray as f64 * golden).sin_cos();
            (out * c, out * s)
        })
    })
}

/// Two ways square to `way` and to each other, a unit long, which turn smoothly as `way` does: the
/// picture's own across and down for a way straight back along z, as a light in front of a clip has,
/// and only for a way straight on along z is there no telling.
fn square_to(way: Vector) -> (Vector, Vector) {
    let [x, y, z] = times(way, -1.0);
    if z < -0.999_999 {
        return ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    }
    let by = -1.0 / (1.0 + z);
    let both = x * y * by;
    ([1.0 + x * x * by, both, -x], [both, 1.0 + y * y * by, -y])
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
    /// What may be in a light's way, when one says it casts shadows ([`Scene::casts_shadows`]).
    pub casters: Vec<Caster>,
    /// How far along the camera's way is in focus, in pixels, and how far what is far from that may
    /// spread: 0 for all of it sharp.
    focus: f64,
    aperture: f64,
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
                    let shadow = read("shadow").unwrap_or(0.0) as f32;
                    let size = read("size").unwrap_or(0.0);
                    lights.push(match generated(def, clip, "kind", at) {
                        Some(Value::Enum(kind)) if kind == "ambient" => Light::Ambient(color),
                        Some(Value::Enum(kind)) if kind == "directional" => {
                            // Straight on unless it says otherwise.
                            let way = if dot(place, place) > 0.0 {
                                unit(place)
                            } else {
                                [0.0, 0.0, 1.0]
                            };
                            let spread = (size.to_radians() / 2.0).tan();
                            Light::Directional(way, color, shadow, spread)
                        }
                        _ => Light::Point(place, color, shadow, size * height / 2.0),
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
        // Where the camera's own line meets the plane at that `z`, or straight across from it on a
        // camera looking along the plane.
        let sharp = read("focus", 0.0) * height;
        let focus = if forward[2].abs() > 1e-6 {
            (sharp - eye[2]) / forward[2]
        } else {
            dot(minus([0.0, 0.0, sharp], eye), forward)
        };
        Self {
            eye,
            right,
            down,
            forward,
            focal: distance(read("fov", DEFAULT_FOV)),
            center: (width / 2.0, height / 2.0),
            height,
            lights,
            casters: Vec::new(),
            focus,
            aperture: read("aperture", 0.0) * height,
        }
    }

    /// Where the ray through the place `(x, y)` of the output picture meets `plane`, and how far along
    /// the camera's way that is: nowhere if it runs along the plane, or meets it no nearer than the
    /// camera sees or behind it.
    #[inline]
    pub(super) fn meets(&self, plane: &Plane, (x, y): (f64, f64)) -> Option<(Vector, f64)> {
        let ray = add(
            add(
                times(self.right, x - self.center.0),
                times(self.down, y - self.center.1),
            ),
            times(self.forward, self.focal),
        );
        let toward = dot(ray, plane.normal);
        if toward.abs() < 1e-12 {
            return None;
        }
        let along = dot(minus(plane.origin, self.eye), plane.normal) / toward;
        let ahead = along * dot(ray, self.forward);
        if ahead < NEAR * self.height {
            return None;
        }
        Some((add(self.eye, times(ray, along)), ahead))
    }

    /// How far each pixel of a plane spreads, from its depths along the camera's way.
    fn blur_radii(&self, depth: &[f32]) -> Vec<f32> {
        let (focus, aperture) = (self.focus as f32, self.aperture as f32);
        let most = (self.height * MOST_BLUR) as f32;
        depth
            .iter()
            .map(|depth| super::focus::radius_at(*depth, focus, aperture, most))
            .collect()
    }

    /// Whether any light here is blocked by what is in its way, which takes the pictures of every
    /// clip in depth to find out.
    pub fn casts_shadows(&self) -> bool {
        self.lights.iter().any(|light| {
            matches!(
                light,
                Light::Point(_, _, shadow, _) | Light::Directional(_, _, shadow, _) if *shadow > 0.0
            )
        })
    }

    /// Whether a light here sends many rays, so that it pays to know what is opaque.
    fn is_wide(&self) -> bool {
        self.lights.iter().any(|light| {
            matches!(
                light,
                Light::Point(_, _, _, wide) | Light::Directional(_, _, _, wide) if *wide > 0.0
            )
        })
    }

    /// How much of a light gets to `at` on clip `receiver`, going along `way`, a unit long, for
    /// `reach`, with `shadow` of it blocked by what is in its way: all of it where there is nothing.
    fn gets_through(
        &self,
        receiver: &Id,
        at: Vector,
        (way, reach): (Vector, f64),
        shadow: f32,
    ) -> f32 {
        if shadow <= 0.0 {
            return 1.0;
        }
        let through = self
            .casters
            .iter()
            .filter(|caster| caster.clip != *receiver)
            .map(|caster| caster.lets_through(at, way, reach))
            .product::<f32>();
        1.0 - shadow * (1.0 - through)
    }

    /// [`Self::gets_through`] for a light with a `size`, whose middle lies along `way`: how much gets
    /// to `at` along [`RAYS`] rays, `ray` being the way and the reach of the one that ends that far off
    /// the middle. From here the light goes `spread` to either side of it for each pixel it is away.
    /// The four rays around all of it say which clips none of them can meet.
    fn gets_through_across(
        &self,
        receiver: &Id,
        (at, way): (Vector, Vector),
        (radius, spread): (f64, f64),
        shadow: f32,
        ray: impl Fn(Vector) -> (Vector, f64),
    ) -> f32 {
        if shadow <= 0.0 {
            return 1.0;
        }
        let (side, other) = square_to(way);
        let off = |(a, b): (f64, f64), by: f64| ray(add(times(side, a * by), times(other, b * by)));
        let out = radius * (1.0 + share() / 2.0);
        let around = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|at| off(at, out));
        let grows = spread * share() / 2.0;
        let mut rays = None;
        let mut through = [1.0f32; RAYS];
        let mut all_alike = 1.0;
        for caster in self
            .casters
            .iter()
            .filter(|caster| caster.clip != *receiver)
        {
            match caster.takes(at, &around) {
                Takes::Nothing => continue,
                Takes::All(lets) => all_alike *= lets,
                Takes::Some => {
                    let rays = rays.get_or_insert_with(|| spiral().map(|at| off(at, radius)));
                    for (through, (way, reach)) in through.iter_mut().zip(rays.iter()) {
                        *through *= caster.lets_some_through(at, (*way, *reach), grows);
                    }
                }
            }
            if all_alike <= 0.0 {
                return 1.0 - shadow;
            }
        }
        let through = all_alike * through.iter().sum::<f32>() / RAYS as f32;
        1.0 - shadow * (1.0 - through)
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

    fn lit(&self, receiver: &Id, at: Vector, normal: Vector) -> Option<[f32; 3]> {
        if self.lights.is_empty() {
            return None;
        }
        let mut sum = [0.0f32; 3];
        for light in &self.lights {
            let (color, facing, through) = match light {
                Light::Ambient(color) => (color, 1.0, 1.0),
                Light::Point(place, color, shadow, radius) => {
                    let to = minus(*place, at);
                    let way = unit(to);
                    let reach = dot(to, to).sqrt();
                    let through = if *radius > 0.0 {
                        let ray = |off| {
                            let to = add(to, off);
                            let reach = dot(to, to).sqrt();
                            (times(to, 1.0 / reach.max(1e-12)), reach)
                        };
                        let wide = (*radius, *radius / reach.max(1e-12));
                        self.gets_through_across(receiver, (at, way), wide, *shadow, ray)
                    } else {
                        self.gets_through(receiver, at, (way, reach), *shadow)
                    };
                    (color, dot(way, normal).abs(), through)
                }
                Light::Directional(way, color, shadow, spread) => {
                    let back = times(*way, -1.0);
                    let through = if *spread > 0.0 {
                        let ray = |off| (unit(add(back, off)), f64::INFINITY);
                        let wide = (*spread, *spread);
                        self.gets_through_across(receiver, (at, back), wide, *shadow, ray)
                    } else {
                        self.gets_through(receiver, at, (back, f64::INFINITY), *shadow)
                    };
                    (color, dot(*way, normal).abs(), through)
                }
            };
            for (sum, channel) in sum.iter_mut().zip(color) {
                *sum += channel * facing as f32 * through;
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
    pub(super) fn at(&self, hit: Vector) -> (f64, f64) {
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

/// What a clip does to the rays from a pixel to a light with a size.
enum Takes {
    Nothing,
    /// Each ray by the alpha its share of the light covers.
    Some,
    /// All of them are behind it, and it lets this much of each through.
    All(f32),
}

/// What a picture holds, for the many rays of a light with a size, which read it over an area.
enum Held {
    /// Opaque all over, as a shot is, so that only its rectangle matters.
    Solid,
    /// Its alpha in 255ths, summed from the top left to each pixel, with a row and a column before the
    /// first. What any rectangle of it holds is four of these apart, wrapped as they may be.
    Summed(Vec<u32>),
}

impl Held {
    fn of(picture: &Image) -> Self {
        if picture.px.par_chunks_exact(4).all(|pixel| pixel[3] >= 1.0) {
            return Self::Solid;
        }
        let (w, h) = (picture.w, picture.h);
        let stride = w + 1;
        let mut table = vec![0u32; stride * (h + 1)];
        table[stride..]
            .par_chunks_mut(stride)
            .enumerate()
            .for_each(|(y, row)| {
                let mut line = 0u32;
                for x in 0..w {
                    let alpha = picture.px[(y * w + x) * 4 + 3];
                    line += (alpha * 255.0 + 0.5) as u32;
                    row[x + 1] = line;
                }
            });
        for y in 1..h {
            let (above, row) = table.split_at_mut(stride * (y + 1));
            for (here, above) in row[..stride].iter_mut().zip(&above[stride * y..]) {
                *here = here.wrapping_add(*above);
            }
        }
        Self::Summed(table)
    }
}

/// A clip in depth, as what may stand in a light's way.
pub(super) struct Caster {
    clip: Id,
    plane: Plane,
    picture: Arc<Image>,
    crop: Rect,
    opacity: f32,
    /// Only for a light with a size.
    held: Held,
    /// What goes from the world to `u` and to `v`: [`Plane::at`], worked out ahead for many rays.
    to_u: Vector,
    to_v: Vector,
    /// Which way its picture goes across and down, a unit long, and how many of its pixels a pixel of
    /// the world is along each.
    across: (Vector, f64),
    down: (Vector, f64),
}

impl Caster {
    /// `picture`, as much of it as `crop` leaves, is where `to_output` puts it, on a clip at clip time
    /// `time`.
    pub fn new(
        scene: &Scene,
        plan: &ClipPlan,
        picture: Arc<Image>,
        (to_output, crop): (&Affine, Option<Rect>),
        opacity: f32,
        time: Time,
    ) -> Self {
        let (pw, ph) = (picture.w as f64, picture.h as f64);
        Self::on(
            plan.clip.id.clone(),
            Plane::of(scene, plan, (pw, ph), to_output, time),
            crop.unwrap_or_else(|| Rect::new(0.0, 0.0, pw, ph)),
            if scene.is_wide() {
                Held::of(&picture)
            } else {
                Held::Solid
            },
            (picture, opacity),
        )
    }

    fn on(
        clip: Id,
        plane: Plane,
        crop: Rect,
        held: Held,
        (picture, opacity): (Arc<Image>, f32),
    ) -> Self {
        let to_u = times(cross(plane.down, plane.perpendicular), 1.0 / plane.area);
        let to_v = times(cross(plane.perpendicular, plane.across), 1.0 / plane.area);
        let per = |way: Vector| (unit(way), 1.0 / dot(way, way).sqrt().max(1e-12));
        let (across, down) = (per(plane.across), per(plane.down));
        Self {
            clip,
            plane,
            picture,
            crop,
            opacity,
            held,
            to_u,
            to_v,
            across,
            down,
        }
    }

    /// What this clip does to the rays from `from` to a light that the rays `around` go around. Those
    /// between them cross its plane within where they do, as long as all of them do, and not at all if
    /// none of them does.
    fn takes(&self, from: Vector, around: &[(Vector, f64); 4]) -> Takes {
        let start = dot(minus(self.plane.origin, from), self.plane.normal);
        let (mut left, mut top) = (f64::INFINITY, f64::INFINITY);
        let (mut right, mut bottom) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut crossing = 0;
        for (way, reach) in around {
            let along = start / dot(*way, self.plane.normal);
            if along > 0.0 && along.is_finite() && along <= *reach {
                crossing += 1;
                let (u, v) = self.plane.at(add(from, times(*way, along)));
                (left, right) = (left.min(u), right.max(u));
                (top, bottom) = (top.min(v), bottom.max(v));
            }
        }
        if crossing == 0 {
            return Takes::Nothing;
        }
        if crossing < 4 {
            return Takes::Some;
        }
        let crop = self.crop;
        let (x0, x1) = (left.max(crop.x), right.min(crop.right()));
        let (y0, y1) = (top.max(crop.y), bottom.min(crop.bottom()));
        if x0 > x1 || y0 > y1 {
            return Takes::Nothing;
        }
        // Further than a pixel from the plane, or it passes them by; and where a ray reads a pixel more.
        let mut hides = start.abs() > 1.0
            && left >= crop.x + 1.0
            && right <= crop.right() - 1.0
            && top >= crop.y + 1.0
            && bottom <= crop.bottom() - 1.0;
        if matches!(self.held, Held::Summed(_)) {
            let area = (
                x0.floor() - 1.0,
                y0.floor() - 1.0,
                x1.ceil() + 1.0,
                y1.ceil() + 1.0,
            );
            let (held, whole) = self.holds(area);
            if held == 0 {
                return Takes::Nothing;
            }
            hides = hides && held == whole;
        }
        if hides {
            Takes::All(1.0 - self.opacity)
        } else {
            Takes::Some
        }
    }

    /// What the rectangle from the place `(left, top)` to `(right, bottom)` of its picture holds in
    /// 255ths of a pixel, as a picture opaque all over does too, and what it could.
    fn holds(&self, (left, top, right, bottom): (f64, f64, f64, f64)) -> (u32, u32) {
        let (w, h) = (self.picture.w, self.picture.h);
        let place = |at: f64, most: usize| (at.max(0.0) as usize).min(most);
        let (x0, x1) = (place(left, w), place(right, w));
        let (y0, y1) = (place(top, h), place(bottom, h));
        let whole = ((x1 - x0) * (y1 - y0)) as u32 * 255;
        let held = match &self.held {
            Held::Solid => whole,
            Held::Summed(table) => {
                let at = |x: usize, y: usize| table[y * (w + 1) + x];
                at(x1, y1)
                    .wrapping_add(at(x0, y0))
                    .wrapping_sub(at(x0, y1))
                    .wrapping_sub(at(x1, y0))
            }
        };
        (held, whole)
    }

    /// How much of what leaves `from` along a way, a unit long, for a reach, gets past this clip, where
    /// its share of the light goes `grows` to either side of it for each pixel it has come.
    #[inline]
    fn lets_some_through(&self, from: Vector, (way, reach): (Vector, f64), grows: f64) -> f32 {
        let toward = dot(way, self.plane.normal);
        if toward.abs() < 1e-9 {
            return 1.0;
        }
        let from_origin = minus(from, self.plane.origin);
        let along = -dot(from_origin, self.plane.normal) / toward;
        if along < 1.0 || along > reach - 1.0 {
            return 1.0;
        }
        let hit = add(from_origin, times(way, along));
        let (u, v) = (dot(hit, self.to_u), dot(hit, self.to_v));
        // A disc about the ray, laid on the plane, is longer the way the ray leans. This is how far that
        // goes either way across and down, in the picture's pixels.
        let half = |(axis, per): (Vector, f64)| {
            let lean = dot(way, axis) / toward;
            (grows * along * per * (1.0 + lean * lean).sqrt()).max(0.5)
        };
        let (half_u, half_v) = (half(self.across), half(self.down));
        let crop = self.crop;
        let (left, right) = ((u - half_u).max(crop.x), (u + half_u).min(crop.right()));
        let (top, bottom) = ((v - half_v).max(crop.y), (v + half_v).min(crop.bottom()));
        if left >= right || top >= bottom {
            return 1.0;
        }
        let covered = if matches!(self.held, Held::Solid) {
            (right - left) * (bottom - top) / (4.0 * half_u * half_v)
        } else {
            // On whole pixels of a picture, those of all of it as well as those left within the crop.
            let width = ((u + half_u).round() - (u - half_u).round()).max(1.0);
            let height = ((v + half_v).round() - (v - half_v).round()).max(1.0);
            let box_of = (left.round(), top.round(), right.round(), bottom.round());
            f64::from(self.holds(box_of).0) / 255.0 / (width * height)
        };
        1.0 - (covered as f32).min(1.0) * self.opacity
    }

    /// How much of what leaves `from` along `way`, a unit long, for `reach`, gets past this clip.
    fn lets_through(&self, from: Vector, way: Vector, reach: f64) -> f32 {
        let toward = dot(way, self.plane.normal);
        if toward.abs() < 1e-9 {
            return 1.0;
        }
        let along = dot(minus(self.plane.origin, from), self.plane.normal) / toward;
        // Not behind where it starts, or past the light, with a pixel's room so that a clip laid on
        // another does not catch it.
        if along < 1.0 || along > reach - 1.0 {
            return 1.0;
        }
        let (u, v) = self.plane.at(add(from, times(way, along)));
        let crop = self.crop;
        if u < crop.x || v < crop.y || u > crop.right() || v > crop.bottom() {
            return 1.0;
        }
        1.0 - self.picture.sample_bilinear(u as f32, v as f32)[3] * self.opacity
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
    let draw_row = |y: usize, row: &mut [f32], mut depth: Option<&mut [f32]>| {
        for x in 0..w {
            let Some((hit, ahead)) = scene.meets(&plane, (x as f64 + 0.5, y as f64 + 0.5)) else {
                continue;
            };
            if let Some(depth) = depth.as_deref_mut() {
                depth[x] = ahead as f32;
            }
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
            if let Some(light) = scene.lit(&plan.clip.id, hit, plane.normal) {
                for (value, light) in pixel.iter_mut().zip(light) {
                    *value *= light;
                }
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&pixel);
        }
    };
    let row_size = w.max(1) * 4;
    if with_depth || scene.aperture > 0.0 {
        let mut depth = vec![f32::INFINITY; w * h];
        out.px
            .par_chunks_mut(row_size)
            .zip(depth.par_chunks_mut(w.max(1)))
            .enumerate()
            .for_each(|(y, (row, depth))| draw_row(y, row, Some(depth)));
        if scene.aperture > 0.0 {
            out = super::focus::blurred(&out, &scene.blur_radii(&depth));
        }
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
    fn what_a_clip_is_said_to_take_of_a_light_is_what_its_rays_find() {
        // A square 100 across, 50 back from the wall: opaque, or with its left half opaque and its
        // right half clear, to go by what each ray does to it.
        let mut opaque = Image::new(100, 100);
        opaque.px.iter_mut().for_each(|channel| *channel = 1.0);
        let mut half = opaque.clone();
        half.px
            .chunks_exact_mut(4)
            .enumerate()
            .filter(|(at, _)| at % 100 >= 50)
            .for_each(|(_, pixel)| pixel.fill(0.0));
        let on = |picture: Image| {
            Caster::on(
                Id::new("c").unwrap(),
                plane([0.0, 0.0, -50.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                WHOLE,
                Held::of(&picture),
                (Arc::new(picture), 1.0),
            )
        };
        let casters = [on(opaque), on(half)];
        assert!(matches!(casters[0].held, Held::Solid));
        assert!(matches!(casters[1].held, Held::Summed(_)));
        // A rectangle holds what it holds: half of its columns are opaque.
        assert_eq!(
            casters[1].holds((40.0, 0.0, 60.0, 10.0)),
            (10 * 10 * 255, 20 * 10 * 255)
        );
        assert_eq!(
            casters[0].holds((40.0, 0.0, 60.0, 10.0)),
            (20 * 10 * 255, 20 * 10 * 255)
        );
        assert_eq!(casters[1].holds((60.0, 0.0, 200.0, 10.0)).0, 0);
        let lights: [(Vector, f64); 5] = [
            // Over it, off to its side, past it, behind the wall, and level with it.
            ([50.0, 50.0, -200.0], 40.0),
            ([300.0, 50.0, -200.0], 60.0),
            ([50.0, 50.0, -20.0], 15.0),
            ([50.0, 50.0, 100.0], 40.0),
            ([50.0, 50.0, -50.0], 30.0),
        ];
        let (mut passed, mut met, mut whole) = (0, 0, 0);
        for (place, radius) in lights {
            for x in (-200..300).step_by(20) {
                for y in (-200..300).step_by(20) {
                    for z in [0.0, -10.0, -49.0, -51.0] {
                        let from = [f64::from(x), f64::from(y), z];
                        let to = minus(place, from);
                        let to_length = dot(to, to).sqrt();
                        let (side, other) = square_to(unit(to));
                        let ray = |(a, b): (f64, f64)| {
                            let off = add(times(side, a * radius), times(other, b * radius));
                            let to = add(to, off);
                            let reach = dot(to, to).sqrt();
                            (times(to, 1.0 / reach), reach)
                        };
                        let out = 1.0 + share() / 2.0;
                        let around = [(-out, -out), (out, -out), (out, out), (-out, out)].map(ray);
                        for caster in &casters {
                            let all = spiral().map(|place| {
                                let grows = radius / to_length * share() / 2.0;
                                caster.lets_some_through(from, ray(place), grows)
                            });
                            match caster.takes(from, &around) {
                                Takes::Nothing => {
                                    assert_eq!(all, [1.0; RAYS], "{from:?} to {place:?}");
                                    passed += 1;
                                }
                                Takes::All(lets) => {
                                    assert_eq!(all, [lets; RAYS], "{from:?} to {place:?}");
                                    whole += 1;
                                }
                                Takes::Some => met += 1,
                            }
                        }
                    }
                }
            }
        }
        // It has to be good for something, and the test has to have seen them all.
        assert!(
            passed > 1000 && met > 100 && whole > 10,
            "{passed} {met} {whole}"
        );
    }

    #[test]
    fn the_sides_of_a_light_are_square_and_turn_smoothly() {
        for way in [
            [0.0, 0.0, -1.0],
            unit([0.3, 0.2, -1.0]),
            unit([1.0, -1.0, 0.0]),
            unit([0.0, 1.0, 1.0]),
            [0.0, 0.0, 1.0],
        ] {
            let (side, other) = square_to(way);
            for (a, b) in [(side, other), (side, way), (other, way)] {
                assert!(dot(a, b).abs() < 1e-9, "{way:?}");
            }
            assert!((dot(side, side) - 1.0).abs() < 1e-9, "{way:?}");
            assert!((dot(other, other) - 1.0).abs() < 1e-9, "{way:?}");
        }
        // Straight back it is the picture's own, and a touch off that it hardly turns.
        assert_eq!(
            square_to([0.0, 0.0, -1.0]),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        );
        let (side, _) = square_to(unit([0.01, 0.01, -1.0]));
        assert!(dot(side, [1.0, 0.0, 0.0]) > 0.999);
    }

    #[test]
    fn what_a_ray_reads_of_a_clip_is_where_the_plane_says() {
        let tilted = plane([3.0, 4.0, 5.0], [0.9, 0.1, 0.2], [-0.2, 1.1, 0.3]);
        let caster = Caster::on(
            Id::new("c").unwrap(),
            tilted,
            WHOLE,
            Held::Solid,
            (Arc::new(Image::new(100, 100)), 1.0),
        );
        let hit = caster.plane.point(31.0, 57.0);
        let from_origin = minus(hit, caster.plane.origin);
        assert!((dot(from_origin, caster.to_u) - 31.0).abs() < 1e-9);
        assert!((dot(from_origin, caster.to_v) - 57.0).abs() < 1e-9);
    }

    #[test]
    fn a_lights_rays_spread_evenly_however_an_edge_crosses_them() {
        // However a straight edge lies across the light, what its rays say is left of the light is
        // within three steps of what a whole disc would, which a grid is not along its rows.
        let spread = |points: &[(f64, f64)]| {
            let mut worst = 0.0f64;
            for turn in 0..32 {
                let (s, c) = (f64::from(turn) * std::f64::consts::PI / 32.0).sin_cos();
                for cut in -49..50 {
                    let cut = f64::from(cut) / 50.0;
                    let rays = points.iter().filter(|(a, b)| a * c + b * s < cut).count();
                    let disc =
                        0.5 + (cut * (1.0 - cut * cut).sqrt() + cut.asin()) / std::f64::consts::PI;
                    worst = worst.max((rays as f64 / points.len() as f64 - disc).abs());
                }
            }
            worst
        };
        let ours = spread(spiral());
        assert!(ours < 3.0 / RAYS as f64, "{ours}");
        let grid: Vec<(f64, f64)> = (0..7)
            .flat_map(|x| (0..7).map(move |y| (f64::from(x) / 3.0 - 1.0, f64::from(y) / 3.0 - 1.0)))
            .filter(|(a, b)| a * a + b * b <= 1.0)
            .collect();
        assert!(grid.len().abs_diff(RAYS) < 6, "{}", grid.len());
        assert!(ours < spread(&grid), "{ours} {}", spread(&grid));
    }

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
