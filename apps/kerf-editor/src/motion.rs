//! How the editor moves. Motion never decides anything: state changes at
//! once and motion follows it, so a stopped animation leaves nothing wrong.
//! Under Reduce Motion everything is simply there.
//!
//! [`Tween`] carries one animated value; a view samples it while rendering
//! and asks for the next frame while it runs ([`Tween::sample`]). Fades and
//! rises of whole elements use GPUI's `with_animation` with [`ease`].

use crate::theme::motion;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// Whether motion is off: `KERF_REDUCE_MOTION=1`, or the setting.
pub fn reduced() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
}

pub fn set_reduced(reduced: bool) {
    REDUCE_MOTION.store(reduced, Ordering::Relaxed);
}

/// Reads `KERF_REDUCE_MOTION` once at start.
pub fn init_from_env() {
    if std::env::var("KERF_REDUCE_MOTION").is_ok_and(|value| !value.is_empty() && value != "0") {
        set_reduced(true);
    }
}

/// The one easing curve, cubic-bezier(.22, 1, .36, 1), at `t` in 0..1.
pub fn ease(t: f32) -> f32 {
    let (x1, y1, x2, y2) = motion::EASE;
    cubic_bezier(x1, y1, x2, y2, t)
}

/// A CSS cubic-bezier timing function at `t`: solves x(s) = t for the
/// curve's parameter `s` by Newton's method, falling back to bisection,
/// and answers y(s).
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let coordinate = |a: f32, b: f32, s: f32| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * s * a + 3.0 * inverse * s * s * b + s * s * s
    };
    let slope = |a: f32, b: f32, s: f32| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * a + 6.0 * inverse * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };
    let mut s = t;
    for _ in 0..8 {
        let error = coordinate(x1, x2, s) - t;
        if error.abs() < 1e-5 {
            return coordinate(y1, y2, s);
        }
        let derivative = slope(x1, x2, s);
        if derivative.abs() < 1e-6 {
            break;
        }
        s = (s - error / derivative).clamp(0.0, 1.0);
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    s = t;
    for _ in 0..32 {
        let x = coordinate(x1, x2, s);
        if (x - t).abs() < 1e-5 {
            break;
        }
        if x < t {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    coordinate(y1, y2, s)
}

/// The duration a scene takes: none under Reduce Motion.
pub fn duration(scene: Duration) -> Duration {
    if reduced() {
        Duration::ZERO
    } else {
        scene
    }
}

/// One animated value, from where it was to where it goes. Setting a new
/// target while it runs goes on from where it has got to, so two motions
/// never fight over one property.
#[derive(Clone, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
}

impl Tween {
    /// A value at rest at `value`.
    pub fn at(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: Instant::now(),
            duration: Duration::ZERO,
        }
    }

    /// Where it is going.
    pub fn target(&self) -> f32 {
        self.to
    }

    /// Goes to `to` over `scene` (none under Reduce Motion), from where it is now.
    pub fn go(&mut self, to: f32, scene: Duration) {
        self.go_at(to, scene, Instant::now());
    }

    pub fn go_at(&mut self, to: f32, scene: Duration, now: Instant) {
        if (to - self.to).abs() < f32::EPSILON && !self.running_at(now) {
            return;
        }
        self.from = self.value_at(now);
        self.to = to;
        self.start = now;
        self.duration = duration(scene);
    }

    /// Jumps to `value` with no motion.
    pub fn set(&mut self, value: f32) {
        *self = Self::at(value);
    }

    pub fn value(&self) -> f32 {
        self.value_at(Instant::now())
    }

    pub fn value_at(&self, now: Instant) -> f32 {
        let progress = self.progress_at(now);
        self.from + (self.to - self.from) * ease(progress)
    }

    /// How far through it is, 0..1, before easing.
    pub fn progress_at(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start);
        (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    pub fn running(&self) -> bool {
        self.running_at(Instant::now())
    }

    pub fn running_at(&self, now: Instant) -> bool {
        self.progress_at(now) < 1.0
    }

    /// Its value now, asking `window` for another frame while it runs.
    pub fn sample(&self, window: &gpui::Window) -> f32 {
        let now = Instant::now();
        if self.running_at(now) {
            window.request_animation_frame();
        }
        self.value_at(now)
    }
}

/// The phase 0..1 of something repeating every `period`, for lights and
/// stripes that run while something works. Still (0) under Reduce Motion.
pub fn phase(since: Instant, period: Duration) -> f32 {
    if reduced() || period.is_zero() {
        return 0.0;
    }
    let elapsed = since.elapsed().as_secs_f32();
    (elapsed / period.as_secs_f32()).fract()
}

/// Where a band of light passing over a title is, -0.3..1.3 of its width,
/// during the share [`motion::TITLE_CROSS`] of each pass; None while it rests.
pub fn shimmer_at(phase: f32, cross: f32) -> Option<f32> {
    if phase >= cross {
        return None;
    }
    Some(-0.3 + 1.6 * (phase / cross))
}

/// A GPUI animation over `scene`, on the one curve. Under Reduce Motion it
/// takes a millisecond, which GPUI shows as its end at once.
pub fn animation(scene: Duration) -> gpui::Animation {
    gpui::Animation::new(duration(scene).max(Duration::from_millis(1))).with_easing(ease)
}
