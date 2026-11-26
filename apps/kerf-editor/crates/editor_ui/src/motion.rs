//! How the editor moves. Motion never decides anything: state changes at
//! once and motion follows it, so a stopped animation leaves nothing wrong.
//! Under Reduce Motion everything is simply there: the editor's own switch,
//! the system's, or `KERF_REDUCE_MOTION` over both. The system's is read
//! as the editor starts, then again as a motion starts (a
//! [`Tween`] set going) when the last read is more than `SYSTEM_POLL` old,
//! so an editor where nothing moves reads nothing, however often it draws.
//!
//! [`Tween`] carries one animated value; a view samples it while rendering
//! and asks for the next frame while it runs ([`Tween::sample`]). Fades and
//! rises of whole elements use GPUI's `with_animation` with [`ease`].

use crate::theme::motion;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The editor's own Reduce Motion (Settings ▸ Appearance).
static SETTING: AtomicBool = AtomicBool::new(false);
/// The system's (macOS ▸ Accessibility ▸ Reduce motion; GNOME's
/// animations off), as last read.
static SYSTEM: AtomicBool = AtomicBool::new(false);
/// `KERF_REDUCE_MOTION`: 0 unset, 1 motion on, 2 motion off.
static ENV: AtomicU8 = AtomicU8::new(0);

/// Whether motion is off: `KERF_REDUCE_MOTION` if it is set, else the
/// editor's own setting or the system's ([`resolve`]), as last read.
pub fn reduced() -> bool {
    let env = match ENV.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    };
    if env.is_none()
        && let Some(reader) = READER.get()
    {
        reader.wait_first();
    }
    resolve(
        env,
        SETTING.load(Ordering::Relaxed),
        SYSTEM.load(Ordering::Relaxed),
    )
}

/// Whether motion is off, from `KERF_REDUCE_MOTION` (`env`: Some when it
/// is set), the editor's setting and the system's. The variable decides
/// either way; else either switch on turns motion off.
pub fn resolve(env: Option<bool>, setting: bool, system: bool) -> bool {
    env.unwrap_or(setting || system)
}

/// The editor's own Reduce Motion, from Settings or its saved choice.
pub fn set_reduced(reduced: bool) {
    SETTING.store(reduced, Ordering::Relaxed);
}

/// Reads `KERF_REDUCE_MOTION` once at start (`1` turns motion off, `0` on,
/// whatever the system says), and begins following the system's setting.
pub fn init_from_env() {
    let env = env_override(std::env::var("KERF_REDUCE_MOTION").ok().as_deref());
    ENV.store(
        match env {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        },
        Ordering::Relaxed,
    );
    if env.is_none() {
        follow_system();
    }
}

/// What `KERF_REDUCE_MOTION` says: unset or empty says nothing, `0` keeps
/// motion, anything else turns it off.
pub fn env_override(value: Option<&str>) -> Option<bool> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value != "0")
}

/// What `defaults read com.apple.universalaccess reduceMotion` printed:
/// `1` is on. Anything else (the key unset prints nothing) is off.
pub fn parse_macos(output: &str) -> bool {
    matches!(output.trim(), "1" | "true" | "YES")
}

/// What `gsettings get org.gnome.desktop.interface enable-animations`
/// printed, as Reduce motion: animations off is motion reduced. None for
/// anything else.
pub fn parse_gnome(output: &str) -> Option<bool> {
    match output.trim() {
        "false" => Some(true),
        "true" => Some(false),
        _ => None,
    }
}

/// Whether the system's setting is to be read again now: none is being
/// read, and the last read (`last`, None before the first) is
/// [`motion::SYSTEM_POLL`] old or more at `now`.
pub fn system_read_due(last: Option<Instant>, now: Instant, reading: bool) -> bool {
    !reading && last.is_none_or(|at| now.saturating_duration_since(at) >= motion::SYSTEM_POLL)
}

/// What reads the system's setting, off the window's thread, when asked.
struct Reader {
    ask: mpsc::Sender<()>,
    /// A read is asked for or under way.
    reading: AtomicBool,
    /// When the last read was done, in milliseconds from `since`.
    read_at: AtomicU64,
    since: Instant,
    /// Whether the first read is done (it ends the wait for it).
    first: Mutex<bool>,
    first_read: Condvar,
    first_seen: AtomicBool,
}

static READER: OnceLock<Reader> = OnceLock::new();

impl Reader {
    /// The first read, made as the editor starts: the first motion waits
    /// for it (at most [`motion::SYSTEM_FIRST_WAIT`]), so a system with
    /// motion off never shows one.
    fn wait_first(&self) {
        if self.first_seen.load(Ordering::Acquire) {
            return;
        }
        if let Ok(first) = self.first.lock() {
            // Read or not, the motion goes on once this has waited.
            let _ = self
                .first_read
                .wait_timeout_while(first, motion::SYSTEM_FIRST_WAIT, |done| !*done);
        }
        self.first_seen.store(true, Ordering::Release);
    }

    /// A read is done: the next is due `SYSTEM_POLL` from now.
    fn done(&self) {
        let millis = u64::try_from(self.since.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.read_at.store(millis, Ordering::Relaxed);
        self.reading.store(false, Ordering::Release);
    }

    /// The first read is done (or could not be made): no motion waits.
    fn first_done(&self) {
        if let Ok(mut done) = self.first.lock() {
            *done = true;
        }
        self.first_read.notify_all();
    }
}

/// A motion starts: the system's setting is read again, off the window's
/// thread, if its last read is old ([`system_read_due`]); this motion has
/// the last read, and the next the new one.
fn ask_system() {
    let Some(reader) = READER.get() else {
        return;
    };
    reader.wait_first();
    let now = Instant::now();
    let last = reader.since + Duration::from_millis(reader.read_at.load(Ordering::Relaxed));
    if !system_read_due(Some(last), now, reader.reading.load(Ordering::Acquire)) {
        return;
    }
    if reader.reading.swap(true, Ordering::AcqRel) {
        return;
    }
    // The reader gone (the system's setting cannot be read), `reading`
    // stays set and nothing is asked again.
    let _ = reader.ask.send(());
}

/// Begins following the system's Reduce motion: macOS's Accessibility
/// switch, or GNOME's animations where `gsettings` has them. A thread
/// reads it at once, then waits until a motion starts ([`ask_system`]).
/// Where neither can be read, nothing is followed.
#[expect(
    clippy::disallowed_methods,
    reason = "short-lived and waited on: defaults or gsettings reads one setting"
)]
fn follow_system() {
    let read: fn() -> Option<bool> = if cfg!(target_os = "macos") {
        || {
            let output = std::process::Command::new("defaults")
                .args(["read", "com.apple.universalaccess", "reduceMotion"])
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            // The key unset (never switched) fails, and is off.
            Some(output.status.success() && parse_macos(&String::from_utf8_lossy(&output.stdout)))
        }
    } else if cfg!(target_os = "linux") {
        || {
            let output = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "enable-animations"])
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|output| output.status.success())?;
            parse_gnome(&String::from_utf8_lossy(&output.stdout))
        }
    } else {
        return;
    };
    let (ask, asked) = mpsc::channel();
    let reader = Reader {
        ask,
        reading: AtomicBool::new(true),
        read_at: AtomicU64::new(0),
        since: Instant::now(),
        first: Mutex::new(false),
        first_read: Condvar::new(),
        first_seen: AtomicBool::new(false),
    };
    if READER.set(reader).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("reduce-motion".into())
        .spawn(move || {
            let Some(reader) = READER.get() else {
                return;
            };
            let mut first = true;
            loop {
                let value = read();
                if let Some(reduced) = value {
                    SYSTEM.store(reduced, Ordering::Relaxed);
                    reader.done();
                }
                if first {
                    reader.first_done();
                    first = false;
                }
                // Unreadable, nothing more is asked: `reading` stays set.
                if value.is_none() || asked.recv().is_err() {
                    return;
                }
            }
        });
    if spawned.is_err() {
        // No reader: the first motion does not wait for one.
        if let Some(reader) = READER.get() {
            reader.first_seen.store(true, Ordering::Release);
        }
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
        // So the system's Reduce motion, switched while the editor runs,
        // holds from the next motion on.
        ask_system();
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

/// A view drawn again by itself at the moment something it draws from the
/// clock changes (an age such as "4 min ago" turning over, a hold running
/// out), as the window draws only when something changes: one timer, for the
/// soonest moment asked for. A view asks each time it draws such a thing;
/// asking for a later moment keeps the sooner one.
#[derive(Default)]
pub struct Redraw(std::cell::RefCell<Option<(Instant, gpui::Task<()>)>>);

impl Redraw {
    /// Draws the view of `cx` again at `at`, unless it is to be sooner.
    pub fn at<V: 'static>(&self, at: Instant, cx: &gpui::Context<V>) {
        let mut due = self.0.borrow_mut();
        if due
            .as_ref()
            .is_some_and(|(due, _)| *due > Instant::now() && *due <= at)
        {
            return;
        }
        let timer = cx.spawn(async move |view, cx| {
            let wait = at.saturating_duration_since(Instant::now());
            cx.background_executor().timer(wait).await;
            let _ = view.update(cx, |_, cx| cx.notify());
        });
        *due = Some((at, timer));
    }
}
