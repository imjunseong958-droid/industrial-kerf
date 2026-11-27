//! Where a clip's picture lands, from its numbers: the sums the compositor draws a clip by.
//! Whatever draws over the picture answers from them too (the editor's handles, masks, and
//! motion paths), so that it cannot land beside what it is drawn over. Each reads a clip's
//! numbers its own way, the compositor from the model at an exact time and the editor from the
//! view it is sent, and asks here.
//!
//! On the frame, a clip's picture is cropped, fitted to the canvas as FFmpeg fits it, scaled,
//! flipped, slanted, rotated about its anchor, and moved ([`Geometry::placed`]), and then moved
//! with each of its parents ([`Taken`]). In depth, that placing is taken as it is into a world
//! where x runs right, y down, and z away from the viewer, in pixels, from the frame's middle
//! ([`lifted`]): held there by the clip about its own middle and by each of its parents in depth
//! about theirs ([`held`]). A camera sees what is placed there ([`Camera`]), and at rest it sees
//! the plane z = 0 as the frame.
//!
//! The order of the sums in each of these is part of what it is, as it is for [`Affine3`]:
//! frames are drawn from them, and a sum taken in another order draws another picture in its
//! last bits.

use crate::{Affine, Affine3, Mat3, Vec2};

/// A point or a way in the world.
pub type Vector = [f64; 3];

#[inline]
fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn minus(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn times(a: Vector, by: f64) -> Vector {
    [a[0] * by, a[1] * by, a[2] * by]
}

#[inline]
fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `a` turned about x by `pitch` and then about y by `yaw`, in radians: as a camera is tilted
/// and then panned.
pub fn turned(a: Vector, pitch: f64, yaw: f64) -> Vector {
    let (s, c) = pitch.sin_cos();
    let [x, y, z] = a;
    let (y, z) = (y * c - z * s, y * s + z * c);
    let (s, c) = yaw.sin_cos();
    [x * c + z * s, y, -x * s + z * c]
}

/// `a` turned about z by `roll`, in radians: from x toward y, as a clip's `rotation` turns it
/// on the frame.
fn rolled(a: Vector, roll: f64) -> Vector {
    let (s, c) = roll.sin_cos();
    [a[0] * c - a[1] * s, a[0] * s + a[1] * c, a[2]]
}

/// The numbers that place a clip's picture on the frame at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// The shares of its picture its crop takes from the left, the right, the top, and the
    /// bottom.
    pub crop: [f64; 4],
    pub scale: f64,
    /// More across and more down, on top of `scale`; negative flips.
    pub scale_x: f64,
    pub scale_y: f64,
    /// Degrees clockwise, about its anchor.
    pub rotation: f64,
    /// Its anchor from its middle, in widths and heights of what its crop leaves.
    pub anchor: Vec2,
    /// Its anchor from the middle of the canvas, in canvas widths and heights.
    pub at: Vec2,
    /// Its slant, in degrees, and how its own axes are turned for it.
    pub skew: f64,
    pub skew_axis: f64,
}

impl Geometry {
    /// What a document calls these numbers, in the order [`Self::of`] takes them.
    pub const NAMES: [&'static str; 14] = [
        "crop_l",
        "crop_r",
        "crop_t",
        "crop_b",
        "scale",
        "scale_x",
        "scale_y",
        "rotation",
        "anchor_x",
        "anchor_y",
        "x",
        "y",
        "skew",
        "skew_axis",
    ];

    /// A clip left where it is put: at its own size, unturned, in the middle of the canvas.
    pub const REST: Self = Self {
        crop: [0.0; 4],
        scale: 1.0,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation: 0.0,
        anchor: Vec2::ZERO,
        at: Vec2::ZERO,
        skew: 0.0,
        skew_axis: 0.0,
    };

    /// From its numbers, in the order of [`Self::NAMES`].
    pub const fn of(numbers: [f64; 14]) -> Self {
        let [l, r, t, b, scale, scale_x, scale_y, rotation, ax, ay, x, y, skew, skew_axis] =
            numbers;
        Self {
            crop: [l, r, t, b],
            scale,
            scale_x,
            scale_y,
            rotation,
            anchor: Vec2::new(ax, ay),
            at: Vec2::new(x, y),
            skew,
            skew_axis,
        }
    }

    /// With its motion path added in: where the path has it, in canvas widths and heights, adds
    /// to where it would be, and the way the path heads there, in degrees, to how it is turned,
    /// for a clip that faces along its path.
    pub fn along(mut self, on: Vec2, heading: Option<f64>) -> Self {
        self.at.x += on.x;
        self.at.y += on.y;
        if let Some(heading) = heading {
            self.rotation += heading;
        }
        self
    }

    /// Where its anchor is in a picture of `size`, in the picture's pixels: what it is scaled
    /// and turned about, and what a motion path says the place of.
    pub fn anchor_in(&self, (sw, sh): (f64, f64)) -> Vec2 {
        let [l, r, t, b] = self.crop;
        let (width, height) = (sw * (1.0 - l - r), sh * (1.0 - t - b));
        Vec2::new(
            sw * l + width * (0.5 + self.anchor.x),
            sh * t + height * (0.5 + self.anchor.y),
        )
    }

    /// Where the pixels of a picture of `size` land on a `canvas`, in canvas pixels, and how
    /// many canvas pixels one of them is across and down: fitted to the canvas when `fit`, and
    /// on whole pixels when `snap` and nothing rules that out, as a picture that holds still is
    /// drawn. Without `snap` it is where the document says it, under a pixel from there.
    pub fn placed(
        &self,
        size: (f64, f64),
        fit: bool,
        snap: bool,
        (cw, ch): (f64, f64),
    ) -> (Affine, Vec2) {
        let [l, r, t, b] = self.crop;
        let (s, sx, sy) = (self.scale, self.scale_x, self.scale_y);
        let (rotation, skew, skew_axis) = (self.rotation, self.skew, self.skew_axis);
        let (ax, ay, x, y) = (self.anchor.x, self.anchor.y, self.at.x, self.at.y);
        let (sw, sh) = size;
        let (width, height) = (sw * (1.0 - l - r), sh * (1.0 - t - b));
        let (fw, fh) = if fit {
            fitted(width, height, cw, ch)
        } else {
            (width, height)
        };
        let (kx, ky) = (s * sx, s * sy);
        let mut position = Vec2::new(cw / 2.0 + x * cw + ax * fw, ch / 2.0 + y * ch + ay * fh);
        let mut scale = Vec2::new(fw / width * kx, fh / height * ky);
        if snap && rotation == 0.0 && skew == 0.0 && ax == 0.0 && ay == 0.0 {
            // A still picture lands on whole pixels.
            let (ow, oh) = ((fw * kx.abs()).round(), (fh * ky.abs()).round());
            position = Vec2::new(
                (position.x - ow / 2.0 + 1e-9).floor() + ow / 2.0,
                (position.y - oh / 2.0 + 1e-9).floor() + oh / 2.0,
            );
            scale = Vec2::new(ow / width * kx.signum(), oh / height * ky.signum());
        }
        let anchor = self.anchor_in(size);
        let moved = Affine::motion(position, scale, rotation, anchor);
        if skew == 0.0 {
            return (moved, scale);
        }
        // Slanted between being scaled and being turned, as an After Effects layer is: along
        // its own axes, turned by `skew_axis`.
        let slant = Affine::rotate_deg(skew_axis)
            .then_apply(&Affine {
                a: 1.0,
                b: 0.0,
                c: skew.to_radians().tan(),
                d: 1.0,
                e: 0.0,
                f: 0.0,
            })
            .then_apply(&Affine::rotate_deg(-skew_axis));
        let slanted = Affine::translate(position.x, position.y)
            .then_apply(&Affine::rotate_deg(rotation))
            .then_apply(&slant)
            .then_apply(&Affine::scale(scale.x, scale.y))
            .then_apply(&Affine::translate(-anchor.x, -anchor.y));
        (slanted, scale)
    }

    /// Where a clip other clips move with has its own picture on the canvas, as the document
    /// says it; and how it has moved them: from where it would sit with no position, scale,
    /// or rotation of its own, to there. Across the frame, which is all of it for a flat
    /// child; one in depth is then held in depth with it, about the middle of that picture
    /// ([`Taken`]).
    pub fn carries(&self, size: (f64, f64), fit: bool, canvas: (f64, f64)) -> (Affine, Affine) {
        let rest = Self {
            crop: self.crop,
            anchor: self.anchor,
            ..Self::REST
        };
        let (now, _) = self.placed(size, fit, false, canvas);
        let (before, _) = rest.placed(size, fit, false, canvas);
        let moved = before
            .inverse()
            .map_or(Affine::IDENTITY, |inverse| now.then_apply(&inverse));
        (now, moved)
    }
}

/// The size FFmpeg's `scale=W:H:force_original_aspect_ratio=decrease` gives a `width` ×
/// `height` picture on a `cw` × `ch` canvas.
pub fn fitted(width: f64, height: f64, cw: f64, ch: f64) -> (f64, f64) {
    let tall = (ch * width / height).round();
    let wide = (cw * height / width).round();
    (tall.min(cw).max(1.0), wide.min(ch).max(1.0))
}

/// `place`, and then as what is at `about` on the frame holds it in depth: turned about there
/// by its `rotate_x` and then by its `rotate_y`, in degrees, and moved back by its `z`, here in
/// pixels. A clip in depth holds its own picture so, about its middle, and each of its parents
/// in depth then holds it so, about its own.
///
/// What is not turned is not taken to `about` and back, which would cost its numbers their
/// last place: a clip in depth that is nowhere in depth is where it is on the frame, to the bit.
pub fn held(place: Affine3, about: Vector, [back, pitch, yaw]: [f64; 3]) -> Affine3 {
    let turned = if pitch == 0.0 && yaw == 0.0 {
        place
    } else {
        place
            .moved(times(about, -1.0))
            .turned_x_deg(pitch)
            .turned_y_deg(yaw)
            .moved(about)
    };
    turned.moved([0.0, 0.0, back])
}

/// Where a picture that `flat` places on the frame is in the world, and where its middle is:
/// `flat` as it is in depth ([`Affine3::from`]), about the frame's `center`, which is the
/// world's middle; then held in depth by `numbers`, the clip's own `z`, in pixels, its
/// `rotate_x`, and its `rotate_y`, about the middle of its `size`; and then as its parents in
/// depth carry it ([`Taken::carried`]).
pub fn lifted(
    flat: &Affine,
    size: (f64, f64),
    center: (f64, f64),
    numbers: [f64; 3],
    carried: Option<&Affine3>,
) -> (Affine3, Vector) {
    // A point of the frame on the plane z = 0, in the world.
    let world = |x: f64, y: f64| [x - center.0, y - center.1, 0.0];
    // Flat on the frame: the same numbers as on the frame.
    let on_frame = Affine3::from(*flat).moved(world(0.0, 0.0));
    let on = flat.apply(Vec2::new(size.0 / 2.0, size.1 / 2.0));
    let about = world(on.x, on.y);
    let own = held(on_frame, about, numbers);
    // Turned about its middle, which is moved back with it.
    let middle = [about[0], about[1], numbers[0]];
    match carried {
        Some(carried) => (carried.then_apply(&own), carried.apply(middle)),
        None => (own, middle),
    }
}

/// A clip other clips move with, at a moment: what it takes to work out how it has moved them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parent {
    /// Its own numbers, its motion path added in.
    pub geometry: Geometry,
    /// Its picture's size in its own pixels, and whether that is fitted to the canvas.
    pub size: (f64, f64),
    pub fit: bool,
    /// Its `z`, in frame heights, its `rotate_x`, and its `rotate_y`, where its place in depth
    /// is its children's: a clip in depth, or one that draws nothing and only carries. None
    /// for a flat clip that draws, whose numbers in depth are nothing to itself or to them.
    pub depth: Option<[f64; 3]>,
}

/// How its parents take a clip at a moment: across the canvas, as they take any clip, and
/// then those of them in depth take a clip in depth with them.
#[derive(Clone, Debug, PartialEq)]
pub struct Taken {
    /// Across the canvas, in its pixels: each parent's motion after the one before, the one
    /// the clip names first.
    pub moved: Affine,
    /// Each parent in depth that is anywhere in depth but where a flat clip is, the one the
    /// clip names first: the middle of its picture on the canvas, its own parents having moved
    /// it, which it holds what it holds about; and its `z`, in frame heights, its `rotate_x`,
    /// and its `rotate_y`.
    pub held: Vec<(Vec2, [f64; 3])>,
}

impl Taken {
    /// By `parents`, the one the clip names first, on a `canvas`, in one pass over them: each
    /// moves it across the canvas ([`Geometry::carries`]), and with it where the parents in
    /// depth before it hold what they hold.
    pub fn by(parents: impl IntoIterator<Item = Parent>, canvas: (f64, f64)) -> Self {
        let mut taken = Self {
            moved: Affine::IDENTITY,
            held: Vec::new(),
        };
        for parent in parents {
            let (placed, moved) = parent.geometry.carries(parent.size, parent.fit, canvas);
            taken.moved = moved.then_apply(&taken.moved);
            for (about, _) in &mut taken.held {
                *about = moved.apply(*about);
            }
            if let Some(numbers) = parent.depth.filter(|numbers| *numbers != [0.0; 3]) {
                let middle = Vec2::new(parent.size.0 / 2.0, parent.size.1 / 2.0);
                taken.held.push((placed.apply(middle), numbers));
            }
        }
        taken
    }

    /// How its parents in depth hold a clip in depth, each about where its middle is on the
    /// frame ([`held`]), in a world of `per` pixels to each of the `canvas`'s, across and down,
    /// and `height` of them to its height. None where none of them is anywhere in depth but
    /// where a flat clip is.
    pub fn carried(&self, canvas: (f64, f64), per: (f64, f64), height: f64) -> Option<Affine3> {
        self.held.iter().fold(
            None,
            |carried: Option<Affine3>, (middle, [z, pitch, yaw])| {
                let about = [
                    (middle.x - canvas.0 / 2.0) * per.0,
                    (middle.y - canvas.1 / 2.0) * per.1,
                    0.0,
                ];
                let before = carried.unwrap_or(Affine3::IDENTITY);
                Some(held(before, about, [z * height, *pitch, *yaw]))
            },
        )
    }

    /// How these parents have moved what has no picture for them to move, a camera or a
    /// light, in the world of [`Self::carried`].
    pub fn rigged(&self, canvas: (f64, f64), per: (f64, f64), height: f64) -> Rigged {
        Rigged {
            moved: self.moved,
            carried: self
                .carried(canvas, per, height)
                .unwrap_or(Affine3::IDENTITY),
            canvas,
            per,
        }
    }
}

/// How the parents of a camera or a light have moved it: across the frame, as they move any
/// clip, and then in depth. A camera and a light have no picture for a parent to move, so it
/// is where they are and which way they are turned that go with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigged {
    /// Across the canvas, in its pixels.
    moved: Affine,
    /// And then in depth, in the world's.
    carried: Affine3,
    /// The canvas's size, and how many of the world's pixels one of its own is across and down.
    canvas: (f64, f64),
    per: (f64, f64),
}

impl Rigged {
    /// Where a point of the world is, its parents having moved it.
    pub fn point(&self, [x, y, z]: Vector) -> Vector {
        let middle = (self.canvas.0 / 2.0, self.canvas.1 / 2.0);
        let on = self.moved.apply(Vec2::new(
            middle.0 + x / self.per.0,
            middle.1 + y / self.per.1,
        ));
        self.carried.apply([
            (on.x - middle.0) * self.per.0,
            (on.y - middle.1) * self.per.1,
            z,
        ])
    }

    /// Where a way is turned to: by as much as its parents turn a clip on the frame, however
    /// they size it, and then as they turn in depth.
    pub fn way(&self, of: Vector) -> Vector {
        let roll = self.moved.b.atan2(self.moved.a);
        self.carried.apply_vec(rolled(of, roll))
    }
}

/// A camera's own numbers at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Where it is from where it rests: across in frame widths, down and back in frame heights.
    pub at: Vector,
    /// In degrees: tilted, then panned, then rolled about its own line.
    pub tilt: f64,
    pub pan: f64,
    pub roll: f64,
    /// How much it sees down the frame, in degrees.
    pub fov: f64,
}

impl Lens {
    /// The camera a sequence is seen by with none of its own showing: it sees the plane z = 0
    /// as the frame.
    pub const RESTING: Self = Self {
        at: [0.0; 3],
        tilt: 0.0,
        pan: 0.0,
        roll: 0.0,
        fov: Camera::RESTING_FOV,
    };
}

/// How the world is seen at a moment, onto a picture in pixels: from `eye`, along `forward`,
/// with `right` and `down` the ways its picture runs, each a unit long.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vector,
    pub right: Vector,
    pub down: Vector,
    pub forward: Vector,
    /// How far in front of the eye its picture is, in pixels.
    pub focal: f64,
    /// The middle of its picture, and the picture's height.
    pub center: (f64, f64),
    pub height: f64,
}

impl Camera {
    /// The field of view at which a clip at z = 0 is its flat self, in degrees.
    pub const RESTING_FOV: f64 = 40.0;
    /// How near a camera sees, in frame heights.
    pub const NEAR: f64 = 0.001;

    /// The camera of `lens` onto a picture `width` by `height`, where its parents take it,
    /// looking as they turn it.
    pub fn of(lens: &Lens, (width, height): (f64, f64), rigged: Option<&Rigged>) -> Self {
        let distance = |fov: f64| height / 2.0 / (fov.clamp(1.0, 170.0).to_radians() / 2.0).tan();
        let eye = [
            lens.at[0] * width,
            lens.at[1] * height,
            lens.at[2] * height - distance(Self::RESTING_FOV),
        ];
        let (pitch, yaw, roll) = (
            lens.tilt.to_radians(),
            lens.pan.to_radians(),
            lens.roll.to_radians(),
        );
        let forward = turned([0.0, 0.0, 1.0], pitch, yaw);
        let level_right = turned([1.0, 0.0, 0.0], pitch, yaw);
        let level_down = cross(forward, level_right);
        let (s, c) = roll.sin_cos();
        let right = add(times(level_right, c), times(level_down, s));
        let down = add(times(level_down, c), times(level_right, -s));
        let (eye, [right, down, forward]) = match rigged {
            Some(rigged) => (
                rigged.point(eye),
                [right, down, forward].map(|way| rigged.way(way)),
            ),
            None => (eye, [right, down, forward]),
        };
        Self {
            eye,
            right,
            down,
            forward,
            focal: distance(lens.fov),
            center: (width / 2.0, height / 2.0),
            height,
        }
    }

    /// Where the ray through the place `(x, y)` of its picture meets the plane through
    /// `origin` that `normal` is square to, and how far along the camera's way that is:
    /// nowhere if it runs along the plane, or meets it no nearer than the camera sees or
    /// behind it.
    #[inline]
    pub fn meets(
        &self,
        origin: Vector,
        normal: Vector,
        (x, y): (f64, f64),
    ) -> Option<(Vector, f64)> {
        let ray = add(
            add(
                times(self.right, x - self.center.0),
                times(self.down, y - self.center.1),
            ),
            times(self.forward, self.focal),
        );
        let toward = dot(ray, normal);
        if toward.abs() < 1e-12 {
            return None;
        }
        let along = dot(minus(origin, self.eye), normal) / toward;
        let ahead = along * dot(ray, self.forward);
        if ahead < Self::NEAR * self.height {
            return None;
        }
        Some((add(self.eye, times(ray, along)), ahead))
    }

    /// Where a point of the world is seen on its picture, in pixels: nowhere for one at the
    /// eye or behind it.
    #[inline]
    pub fn on_screen(&self, at: Vector) -> Option<(f64, f64)> {
        let from = minus(at, self.eye);
        let ahead = dot(from, self.forward);
        (ahead > 1e-9).then(|| {
            (
                self.center.0 + self.focal * dot(from, self.right) / ahead,
                self.center.1 + self.focal * dot(from, self.down) / ahead,
            )
        })
    }

    /// Its sight of the plane `place` puts at z = 0, as one map: `(u, v)` of the plane's own
    /// terms goes to where on its picture the camera sees it, in pixels, what
    /// [`Self::on_screen`] says of that point and [`Self::meets`] finds again from there; and
    /// what the map divides by is how far along the camera's way the point is, in frame
    /// heights. None for a plane seen edge on, from under [`Self::NEAR`] of its side: a line
    /// at most a pixel wide, which the rays miss.
    pub fn sees(&self, place: &Affine3) -> Option<Mat3> {
        let (across, below) = (place.x, place.y);
        let from = minus(place.t, self.eye);
        let perpendicular = cross(across, below);
        let area = dot(perpendicular, perpendicular).sqrt();
        if area <= 0.0 || dot(from, perpendicular).abs() / area < Self::NEAR * self.height {
            return None;
        }
        let sideways = add(
            times(self.right, self.focal),
            times(self.forward, self.center.0),
        );
        let downwards = add(
            times(self.down, self.focal),
            times(self.forward, self.center.1),
        );
        let over = |of: Vector, way: Vector| dot(of, way) / self.height;
        Some(Mat3([
            [
                over(across, sideways),
                over(below, sideways),
                over(from, sideways),
            ],
            [
                over(across, downwards),
                over(below, downwards),
                over(from, downwards),
            ],
            [
                over(across, self.forward),
                over(below, self.forward),
                over(from, self.forward),
            ],
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const FRAME: (f64, f64) = (1920.0, 1080.0);

    fn lens() -> impl Strategy<Value = Lens> {
        (
            proptest::array::uniform3(-0.4f64..0.4),
            (-30.0f64..30.0, -30.0f64..30.0, -45.0f64..45.0),
            20.0f64..90.0,
        )
            .prop_map(|(at, (tilt, pan, roll), fov)| Lens {
                at,
                tilt,
                pan,
                roll,
                fov,
            })
    }

    /// A picture placed on the frame by a clip's numbers and then held in depth by them.
    fn in_depth() -> impl Strategy<Value = Affine3> {
        (
            (-0.4f64..0.4, -0.4f64..0.4),
            (0.2f64..1.5, -1.5f64..1.5, -180.0f64..180.0),
            (-0.5f64..0.5, -0.5f64..0.5, -30.0f64..30.0),
            (-0.5f64..2.0, -75.0f64..75.0, -75.0f64..75.0),
        )
            .prop_map(|(at, (scale, scale_x, rotation), (ax, ay, skew), depth)| {
                let geometry = Geometry {
                    scale,
                    scale_x,
                    rotation,
                    skew,
                    anchor: Vec2::new(ax, ay),
                    at: Vec2::new(at.0, at.1),
                    ..Geometry::REST
                };
                let size = (1280.0, 720.0);
                let (flat, _) = geometry.placed(size, true, false, FRAME);
                let numbers = [depth.0 * FRAME.1, depth.1, depth.2];
                lifted(&flat, size, (FRAME.0 / 2.0, FRAME.1 / 2.0), numbers, None).0
            })
    }

    proptest! {
        /// What is drawn over a picture in depth lands on what the rays draw of it: the one
        /// map a camera has of a plane puts a point of the plane where the camera sees that
        /// point, as far along its way as the point is, and the ray through there meets the
        /// plane at that point again.
        #[test]
        fn a_cameras_map_of_a_plane_is_where_its_rays_meet_it(
            lens in lens(),
            place in in_depth(),
            (u, v) in (0.0f64..1280.0, 0.0f64..720.0),
        ) {
            let camera = Camera::of(&lens, FRAME, None);
            let point = place.apply([u, v, 0.0]);
            let ahead = dot(minus(point, camera.eye), camera.forward);
            // In sight, and not seen so nearly edge on that a pixel is most of the plane.
            let normal = cross(place.x, place.y);
            let normal = times(normal, 1.0 / dot(normal, normal).sqrt());
            let facing = dot(minus(point, camera.eye), normal).abs();
            prop_assume!(ahead > 0.05 * FRAME.1 && facing > 0.05 * FRAME.1);
            let map = camera.sees(&place).expect("a plane seen from off its side");
            let [x, y, w] = map.undivided(Vec2::new(u, v));
            prop_assert!((w * FRAME.1 - ahead).abs() < 1e-9 * ahead, "{w} of {ahead}");
            let seen = (x / w, y / w);
            let on = camera.on_screen(point).expect("it is ahead");
            let near = |a: f64, b: f64| (a - b).abs() < 1e-6 * (1.0 + b.abs());
            prop_assert!(near(seen.0, on.0) && near(seen.1, on.1), "{seen:?} and {on:?}");
            let (hit, along) = camera.meets(place.t, normal, seen).expect("the ray meets it");
            prop_assert!((0..3).all(|k| near(hit[k], point[k])), "{hit:?} and {point:?}");
            prop_assert!(near(along, ahead), "{along} and {ahead}");
        }
    }
}
