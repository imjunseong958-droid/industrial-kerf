//! Where a clip's picture lands: its geometry's numbers with its motion path added in, read
//! here and placed by kerf-geom ([`Geometry::placed`]), which whatever draws over the picture
//! asks as well, and then moved with each of its parents; and what its crop leaves of its
//! picture, of a generator's that reaches past its frame as well.
//!
//! A clip lands somewhere at a moment, and the moment is part of the answer: a moment of its
//! sequence is one of the clip's own time, which its numbers are read at, and each of its
//! parents is read at its own. So when a clip is and where it is are worked out together and
//! kept together ([`Placement`]), for what is drawn on the frame and what is lifted into depth
//! alike.

use super::*;

/// A clip at a moment: when that is of the clip's own time, which its keyframes are read at,
/// and where the pixels of a picture of it are on the output then. The two are one answer of a
/// moment of the clip's sequence ([`SequencePlan::placement`]) and go about as one: a clip in
/// depth is lifted from both at once ([`three_d::Plane::of`]), so that none is put where it is
/// at one time and turned as it is at another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    pub(super) time: Time,
    pub(super) to_output: Affine,
}

impl Placement {
    /// The same moment, drawn at `scale` of what it was.
    pub(super) fn scaled(self, scale: f64) -> Self {
        self.with(Affine::scale(scale, scale).then_apply(&self.to_output))
    }

    /// The same moment, with a picture that `to_output` puts on the output: one made of the
    /// clip's own, at another size or with more around it.
    pub(super) fn with(self, to_output: Affine) -> Self {
        Self { to_output, ..self }
    }
}

impl SequencePlan {
    /// The clip of `plan` at `at` of the sequence: its own time then, and where its source
    /// pixels land on the canvas, in canvas pixels: cropped, fitted, scaled, flipped, rotated
    /// about its anchor, and moved, as the graph places it, and then moved with each of its
    /// parents, each as it is at that moment. Where its parents are in depth is not in this: a
    /// clip in depth takes that where its plane is made ([`three_d::Plane::of`]), from the
    /// scene.
    pub(super) fn placement(&self, plan: &ClipPlan, at: Time) -> Placement {
        let canvas = (f64::from(self.width), f64::from(self.height));
        let time = plan.clip.time_at(at);
        Placement {
            time,
            to_output: self
                .moved_by_parents(&plan.clip, at)
                .then_apply(&place(plan, canvas, time).0),
        }
    }
}

impl Drawing {
    /// Where a clip's source pixels land on the canvas at clip time `time`, in canvas pixels:
    /// where it is at that moment of its sequence ([`SequencePlan::placement`]), for one who
    /// has the clip's own time alone.
    pub(super) fn geometry(&self, sequence: &SequencePlan, plan: &ClipPlan, time: Time) -> Affine {
        sequence
            .placement(plan, plan.clip.sequence_time(time))
            .to_output
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
/// and how many canvas pixels a source pixel is across and down: on whole pixels when it
/// holds still.
pub(super) fn place(plan: &ClipPlan, canvas: (f64, f64), time: Time) -> (Affine, Vec2) {
    let numbers = geometry_numbers(&plan.clip, plan.path.as_ref(), time);
    numbers.placed(pixels(plan.size), plan.fit, !plan.animated, canvas)
}

/// A clip's geometry at clip time `time`, with its motion path added in.
pub(super) fn geometry_numbers(clip: &Clip, path: Option<&MotionPath>, time: Time) -> Geometry {
    let numbers =
        Geometry::of(Geometry::NAMES.map(|name| number(&param(clip, name).value_at(time))));
    let Some(path) = path else {
        return numbers;
    };
    let on = path.at(number(&param(clip, "along").value_at(time)));
    let faces = matches!(param(clip, "orient"), Param::Static(Value::Bool(true)));
    numbers.along(Vec2::new(on.x, on.y), faces.then_some(on.heading))
}

/// A size in whole pixels, as kerf-geom takes one.
pub(super) fn pixels((width, height): (u32, u32)) -> (f64, f64) {
    (f64::from(width), f64::from(height))
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
