//! Where a clip's picture lands: its geometry's numbers with its motion path added in, cropped,
//! fitted to the canvas as FFmpeg fits it, scaled, flipped, rotated about its anchor, and moved,
//! on whole pixels when it holds still, and then moved with each of its parents; and what its crop
//! leaves of its picture, of a generator's that reaches past its frame as well.

use super::*;

/// The clip properties that place a picture.
pub(super) const GEOMETRY: [&str; 14] = [
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

impl Drawing {
    /// Where a clip's source pixels land on the canvas at clip time `time`,
    /// in canvas pixels: cropped, fitted, scaled, flipped, rotated about
    /// its anchor, and moved, as the graph places it, and then moved with each of its parents.
    /// Where its parents are in depth is not in this: a clip in depth takes that where its
    /// plane is made ([`three_d::Plane::of`]), from the scene.
    pub(super) fn geometry(&self, sequence: &SequencePlan, plan: &ClipPlan, time: Time) -> Affine {
        let canvas = (f64::from(sequence.width), f64::from(sequence.height));
        sequence
            .moved_by_parents(&plan.clip, plan.clip.sequence_time(time))
            .then_apply(&place(plan, canvas, time).0)
    }
}

/// How far apart on the canvas, in its pixels, the copies of a smeared picture may be drawn at most:
/// no further, and they read as one smear.
const SMEAR_STEP: f64 = 1.0;

/// The most copies a smeared picture is drawn as.
const MOST_SMEARS: usize = 64;

impl Drawing {
    /// Where a picture that `to_output` puts on the canvas at clip time `time`, `rect` of it showing,
    /// is at each moment the clip's shutter is open about that time: as many as keep the copies
    /// [`SMEAR_STEP`] apart where it moves fastest. None when its `shutter` is shut, when it is in
    /// depth, where a camera puts it, or when it moves less than half a pixel while it is open.
    pub(super) fn smeared(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
        to_output: &Affine,
        rect: Rect,
    ) -> Option<Vec<Affine>> {
        if plan.three_d {
            return None;
        }
        let shutter = number(&param(&plan.clip, "shutter").value_at(time));
        if shutter.is_nan() || shutter <= 0.0 {
            return None;
        }
        let open = sequence.rate.frame_duration().seconds() * shutter / 360.0;
        let placed = |time: Time| {
            Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time))
        };
        let back = placed(time).inverse()?;
        // Where it is `share` of the way through the shutter, which opens half before `time`.
        let moved = |share: f64| {
            let at = time + Time::from_seconds(open * (share - 0.5)).unwrap_or(Time::ZERO);
            placed(at).then_apply(&back).then_apply(to_output)
        };
        let corners = [
            Vec2::new(rect.x, rect.y),
            Vec2::new(rect.right(), rect.y),
            Vec2::new(rect.x, rect.bottom()),
            Vec2::new(rect.right(), rect.bottom()),
        ];
        // How far its corners go over the shutter, at the speed of its fastest stretch.
        const STRETCHES: usize = 8;
        let mut fastest = 0.0f64;
        let mut before = moved(0.0);
        for stretch in 1..=STRETCHES {
            let after = moved(stretch as f64 / STRETCHES as f64);
            for corner in corners {
                let (from, to) = (before.apply(corner), after.apply(corner));
                fastest = fastest.max((to.x - from.x).hypot(to.y - from.y));
            }
            before = after;
        }
        let across = fastest * STRETCHES as f64;
        if across.is_nan() || across < 0.5 {
            return None;
        }
        let count = ((across / SMEAR_STEP).ceil() as usize + 1).clamp(2, MOST_SMEARS);
        Some(
            (0..count)
                .map(|at| moved(at as f64 / (count - 1) as f64))
                .collect(),
        )
    }
}

/// Where a clip's source pixels land on a `canvas` at clip time `time`,
/// and how many canvas pixels a source pixel is across and down.
pub(super) fn place(plan: &ClipPlan, canvas: (f64, f64), time: Time) -> (Affine, Vec2) {
    let numbers = geometry_numbers(&plan.clip, plan.path.as_ref(), time);
    placed(plan.size, plan.fit, !plan.animated, canvas, numbers)
}

/// The values of a clip's [`GEOMETRY`] at clip time `time`, with its motion path added in.
pub(super) fn geometry_numbers(clip: &Clip, path: Option<&MotionPath>, time: Time) -> [f64; 14] {
    let mut numbers = GEOMETRY.map(|name| number(&param(clip, name).value_at(time)));
    if let Some(path) = path {
        let on = path.at(number(&param(clip, "along").value_at(time)));
        numbers[10] += on.x;
        numbers[11] += on.y;
        if matches!(param(clip, "orient"), Param::Static(Value::Bool(true))) {
            numbers[7] += on.heading;
        }
    }
    numbers
}

/// A clip's motion path on a canvas of this shape, if it has one.
pub(super) fn motion_path(clip: &Clip, canvas: (u32, u32)) -> Option<MotionPath> {
    let Param::Static(Value::Text(text)) = param(clip, "path") else {
        return None;
    };
    let points = kerf_model::parse_path_points(&text).ok()?;
    let aspect = f64::from(canvas.0) / f64::from(canvas.1.max(1));
    Some(MotionPath::new(points, aspect))
}

/// [`place`] from the geometry's values: pixels of a source of `size`, fitted to the canvas when
/// `fit`, and on whole pixels when `snap` and nothing rules that out.
pub(super) fn placed(
    size: (u32, u32),
    fit: bool,
    snap: bool,
    (cw, ch): (f64, f64),
    [l, r, t, b, s, sx, sy, rotation, ax, ay, x, y, skew, skew_axis]: [f64; 14],
) -> (Affine, Vec2) {
    let (sw, sh) = (f64::from(size.0), f64::from(size.1));
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
    let anchor = Vec2::new(sw * l + width * (0.5 + ax), sh * t + height * (0.5 + ay));
    let moved = Affine::motion(position, scale, rotation, anchor);
    if skew == 0.0 {
        return (moved, scale);
    }
    // Slanted between being scaled and being turned, as an After Effects layer is: along its own
    // axes, turned by `skew_axis`.
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

/// How a parent has moved its children at its clip time `time`: from where it would sit with no
/// position, scale, or rotation of its own, to where it is. Across the frame, which is all of it
/// for a flat child; one in depth is then turned and moved back with it
/// ([`three_d::Scene::at`]).
pub(super) fn parent_motion(rig: &Rig, canvas: (f64, f64), time: Time) -> Affine {
    let numbers = geometry_numbers(&rig.clip, rig.path.as_ref(), time);
    let [l, r, t, b, _, _, _, _, ax, ay, _, _, _, _] = numbers;
    let rest = [l, r, t, b, 1.0, 1.0, 1.0, 0.0, ax, ay, 0.0, 0.0, 0.0, 0.0];
    let (now, _) = placed(rig.size, rig.fit, false, canvas, numbers);
    let (before, _) = placed(rig.size, rig.fit, false, canvas, rest);
    before
        .inverse()
        .map_or(Affine::IDENTITY, |inverse| now.then_apply(&inverse))
}

/// What a clip's crop at clip time `time` leaves of its `picture`, in the
/// picture's pixels (`per_source` of the source's); none when nothing is
/// cropped.
pub(super) fn crop_rect(
    plan: &ClipPlan,
    time: Time,
    (sw, sh): (f64, f64),
    per_source: f64,
    picture: (usize, usize),
) -> Option<Rect> {
    let [l, r, t, b] = ["crop_l", "crop_r", "crop_t", "crop_b"]
        .map(|name| number(&param(&plan.clip, name).value_at(time)));
    if l == 0.0 && r == 0.0 && t == 0.0 && b == 0.0 {
        return None;
    }
    let (w, h) = (sw * per_source, sh * per_source);
    let (w, h) = (w.min(picture.0 as f64), h.min(picture.1 as f64));
    Some(Rect::new(
        l * w,
        t * h,
        w * (1.0 - l - r),
        h * (1.0 - t - b),
    ))
}

/// What a clip's crop at clip time `time` leaves of `whole`, in pixels in which its frame is `frame`:
/// a side cropped is cut where it is on the frame, and one that is not is left as far as `whole`
/// goes. Of a generator's picture in depth, which reaches past its frame, as [`crop_rect`] is of one
/// that does not. None when nothing is cropped.
pub(super) fn crop_past(plan: &ClipPlan, time: Time, frame: Rect, whole: Rect) -> Option<Rect> {
    let [l, r, t, b] = ["crop_l", "crop_r", "crop_t", "crop_b"]
        .map(|name| number(&param(&plan.clip, name).value_at(time)));
    if l == 0.0 && r == 0.0 && t == 0.0 && b == 0.0 {
        return None;
    }
    let left = if l > 0.0 {
        (frame.x + l * frame.w).max(whole.x)
    } else {
        whole.x
    };
    let top = if t > 0.0 {
        (frame.y + t * frame.h).max(whole.y)
    } else {
        whole.y
    };
    let right = if r > 0.0 {
        (frame.right() - r * frame.w).min(whole.right())
    } else {
        whole.right()
    };
    let bottom = if b > 0.0 {
        (frame.bottom() - b * frame.h).min(whole.bottom())
    } else {
        whole.bottom()
    };
    Some(Rect::new(
        left,
        top,
        (right - left).max(0.0),
        (bottom - top).max(0.0),
    ))
}

/// Where the canvas is in the picture a generator draws on `run`, in its pixels.
pub(super) fn frame_in(run: &SourceRun) -> Rect {
    Rect::new(
        run.beyond.0 as f64,
        run.beyond.1 as f64,
        run.size.0 as f64,
        run.size.1 as f64,
    )
}

/// `masks`, ready for a picture whose frame is `at` in it.
pub(super) fn past_frame(
    mut masks: Vec<mask::FlatMask>,
    (x, y): (f64, f64),
) -> Vec<mask::FlatMask> {
    if (x, y) != (0.0, 0.0) {
        for point in masks.iter_mut().flat_map(|mask| &mut mask.points) {
            point[0] += x as f32;
            point[1] += y as f32;
        }
    }
    masks
}

/// The size FFmpeg's `scale=W:H:force_original_aspect_ratio=decrease`
/// gives a `width` × `height` picture on a `cw` × `ch` canvas.
pub(super) fn fitted(width: f64, height: f64, cw: f64, ch: f64) -> (f64, f64) {
    let tall = (ch * width / height).round();
    let wide = (cw * height / width).round();
    (tall.min(cw).max(1.0), wide.min(ch).max(1.0))
}

pub(super) fn output_size(width: u32, height: u32, scale: f64) -> (usize, usize) {
    (
        ((f64::from(width) * scale).round() as usize).max(1),
        ((f64::from(height) * scale).round() as usize).max(1),
    )
}

pub(super) fn identity(m: &Affine) -> bool {
    (m.a - 1.0).abs() < 1e-9
        && (m.d - 1.0).abs() < 1e-9
        && m.b.abs() < 1e-12
        && m.c.abs() < 1e-12
        && m.e.abs() < 1e-9
        && m.f.abs() < 1e-9
}
