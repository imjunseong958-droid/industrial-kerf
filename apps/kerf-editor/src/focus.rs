//! Edit ▸ Focus Region… (`clip.focusRegion`): over a frame of the
//! chosen picture clip a box is dragged round what to enlarge, a time range
//! is chosen within the clip, and a moving subject can be followed. Chosen,
//! the clip is split where the range starts and ends and the part between
//! is cropped to the box (each step of a followed subject a part of its
//! own); after the range the clip keeps the crop it had.
//!
//! A scene cut inside the range ends the box at the cut, as kerfd's index
//! of the media finds its cuts (`media_index`). A followed subject is found
//! frame after frame in small grey frames FFmpeg decodes, two a second, by
//! the patch that looks most like it did at the start; where none does any
//! more, the crop ends at the last frame it was sure of.
//!
//! The dialog is over the window as Settings is ([`Workspace::open_modal`]
//! takes it); its frame is decoded off the window's thread, the media's
//! poster standing in until it comes.
//!
//! [`Workspace::open_modal`]: crate::workspace::Workspace::open_modal

use editor_session::artwork::ffmpeg_bin;
use editor_session::session::{BridgeEvent, Session};
use editor_state::model::{self, Clip, ClipShow, TrackKind};
use editor_state::store::Change;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::theme::{self, over_picture, text};
use editor_ui::ui;
use gpui::{
    canvas, div, point, prelude::*, px, App, Bounds, Context, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    RenderImage, SharedString, Subscription, Task, Window,
};
use kerf_bridge::Event;
use serde_json::{json, Value};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;

gpui::actions!(focus_region, [CloseFocus, ApplyFocus]);

/// The key context the dialog answers in: Esc closes it, Return applies.
/// Tab goes round it only ([`ui::PANEL`]).
const CONTEXT: &str = "FocusRegion";

/// What a refusal says.
pub const PENDING: &str = "Review the pending suggestion before changing the crop.";
pub const NO_FRAME: &str = "Could not read a source frame from this clip.";
pub const INVALID: &str = "Select a box and a valid time range within the clip.";
pub const NO_CUTS: &str =
    "Could not check this range for scene cuts. Choose a shorter range and try again.";
pub const CUT_FOUND: &str =
    "A scene cut was found. This box ends at the cut; select a new box for the next scene.";
pub const NO_TRACK: &str =
    "Tracking could not start. Choose a larger box or a range under two minutes.";
pub const LOST: &str =
    "The subject could no longer be tracked. The crop ends at the last reliable frame.";

/// The size of the grey frames a subject is followed in, how many a
/// second of the program, and the longest range followed.
pub const TRACK_SIZE: (usize, usize) = (160, 90);
pub const TRACK_RATE: f64 = 2.0;
pub const TRACK_MOST: f64 = 120.0;
/// How unlike its start (the mean difference of its grey levels) the
/// patch found may be before the subject counts as lost; how much
/// contrast a box needs to be followed at all; the least box followed, in
/// track pixels.
const LOST_ABOVE: f64 = 28.0;
const LEAST_CONTRAST: u8 = 20;
const LEAST_BOX: usize = 6;
/// The tallest the frame to draw a box on is decoded, in pixels; how wide
/// the picture of it is in the dialog, and the dialog.
const FRAME_TALL: u32 = 540;
const PICKER_WIDE: f32 = 520.0;
const DIALOG_WIDE: f32 = 560.0;
/// A box narrower or shorter than this share of the frame is no box.
const LEAST_REGION: f64 = 0.01;
/// The range suggested for a clip with no crop yet, from the playhead.
const SUGGESTED: f64 = 5.0;

/// A box over the frame, in shares of its width and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Region {
    /// The box from corner `a` to corner `b` (either way round), kept
    /// inside the frame.
    pub fn between(a: (f64, f64), b: (f64, f64)) -> Region {
        let clamp = |value: f64| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let (ax, ay, bx, by) = (clamp(a.0), clamp(a.1), clamp(b.0), clamp(b.1));
        Region {
            x: ax.min(bx),
            y: ay.min(by),
            width: (ax - bx).abs(),
            height: (ay - by).abs(),
        }
    }

    /// Whether it is too small to be a box ([`LEAST_REGION`]).
    pub fn is_empty(&self) -> bool {
        !(self.width >= LEAST_REGION && self.height >= LEAST_REGION)
    }
}

/// A clip's crop as the `set` op writes it (`crop_l`, `crop_t`, `crop_r`,
/// `crop_b`, shares of its picture cut from each side); no box clears it.
pub fn crop_props(region: Option<Region>) -> Value {
    match region {
        None => json!({"crop_l": null, "crop_t": null, "crop_r": null, "crop_b": null}),
        Some(region) => json!({
            "crop_l": region.x,
            "crop_t": region.y,
            "crop_r": (1.0 - region.x - region.width).max(0.0),
            "crop_b": (1.0 - region.y - region.height).max(0.0),
        }),
    }
}

/// The box a clip's crop (its `props`) keeps; None with no crop.
pub fn crop_box(props: &Value) -> Option<Region> {
    let edge = |name: &str| props[name].as_f64().unwrap_or(0.0);
    let (l, t, r, b) = (
        edge("crop_l"),
        edge("crop_t"),
        edge("crop_r"),
        edge("crop_b"),
    );
    if l == 0.0 && t == 0.0 && r == 0.0 && b == 0.0 {
        return None;
    }
    Some(Region {
        x: l,
        y: t,
        width: 1.0 - l - r,
        height: 1.0 - t - b,
    })
}

/// The edit that focuses clip `clip` (its frames `clip_frames`, first and
/// past its last) on `region` over the frames `range`: the clip split
/// where the range starts (a new part, `<clip>-focus-<suffix>`) unless it
/// starts with the clip, that part cropped to the box; each step
/// of a followed subject (`tracked`, its frame and box, the first being the
/// start) split off in turn (`<clip>-track-<n>-<suffix>`) with its own box;
/// and the clip split where the range ends unless it ends with the clip,
/// the part after (`<clip>-after-<suffix>`) keeping `old_crop`. Its ops,
/// and the part last focused, which is chosen after.
pub fn focus_ops(
    clip: &str,
    clip_frames: (i64, i64),
    range: (i64, i64),
    region: Region,
    tracked: &[(i64, Region)],
    old_crop: Option<Region>,
    suffix: &str,
) -> (Vec<Value>, String) {
    let mut ops = Vec::new();
    let split = |ops: &mut Vec<Value>, id: &str, at: i64, tail: &str| {
        ops.push(json!({"op": "split", "clip": id, "at": at, "tail": tail}));
    };
    let crop = |ops: &mut Vec<Value>, id: &str, region: Option<Region>| {
        ops.push(json!({"op": "set", "clip": id, "props": crop_props(region)}));
    };
    let mut focused = clip.to_owned();
    if range.0 > clip_frames.0 {
        focused = format!("{clip}-focus-{suffix}");
        split(&mut ops, clip, range.0, &focused);
    }
    crop(&mut ops, &focused, Some(region));
    for (index, (frame, step)) in tracked.iter().enumerate().skip(1) {
        if *frame >= range.1 {
            break;
        }
        let next = format!("{clip}-track-{index}-{suffix}");
        split(&mut ops, &focused, *frame, &next);
        focused = next;
        crop(&mut ops, &focused, Some(*step));
    }
    if range.1 < clip_frames.1 {
        let after = format!("{clip}-after-{suffix}");
        split(&mut ops, &focused, range.1, &after);
        crop(&mut ops, &after, old_crop);
    }
    (ops, focused)
}

/// Where in the program second `source` of `clip`'s media plays; None
/// when the clip plays none of its media.
fn program_time(clip: &Clip, source: f64) -> Option<f64> {
    let first = clip.source_at(0.0)?;
    let last = clip.source_at(clip.duration)?;
    let span = last - first;
    if span.abs() < 1e-9 || clip.duration <= 0.0 {
        return None;
    }
    Some(clip.timeline_start + (source - first) / span * clip.duration)
}

/// The first scene cut of `clip`'s media (`cuts`, seconds of the media as
/// its index has them) that falls inside `start..end` of the program, more
/// than a frame at `fps` in from either end; None when none does.
pub fn first_cut(cuts: &[f64], clip: &Clip, start: f64, end: f64, fps: f64) -> Option<f64> {
    let frame = 1.0 / if fps > 0.0 { fps } else { 30.0 };
    cuts.iter()
        .filter_map(|cut| program_time(clip, *cut))
        .filter(|at| *at > start + frame && *at < end - frame)
        .min_by(f64::total_cmp)
}

/// A subject followed: each step's frame of the program and its box, the
/// first being where it started, and where it was last found for sure.
#[derive(Clone, Debug, PartialEq)]
pub struct Tracked {
    pub boxes: Vec<(i64, Region)>,
    pub reliable_end: f64,
}

/// Follows the subject in `seed` through `frames` (grey, `width` ×
/// `height` each, back to back, the first at `start` of the program and
/// [`TRACK_RATE`] a second after it): in each frame the box nearest where
/// it was last that looks most like it did at the start. A step is kept
/// where it moved more than a pixel. The subject is lost where nothing
/// looks enough like it ([`LOST_ABOVE`]). None when it cannot be followed:
/// fewer than two frames, a box too small, or one with too little in it to
/// find.
pub fn track_region(
    frames: &[u8],
    (width, height): (usize, usize),
    seed: Region,
    (start, end): (f64, f64),
    fps: f64,
) -> Option<Tracked> {
    let size = width.checked_mul(height)?;
    if size == 0 {
        return None;
    }
    let count = frames.len() / size;
    if count < 2 {
        return None;
    }
    let box_width = ((seed.width * width as f64).round() as usize).clamp(1, width);
    let box_height = ((seed.height * height as f64).round() as usize).clamp(1, height);
    if box_width < LEAST_BOX || box_height < LEAST_BOX {
        return None;
    }
    let x = ((seed.x * width as f64).round().max(0.0) as usize).min(width - box_width);
    let y = ((seed.y * height as f64).round().max(0.0) as usize).min(height - box_height);
    let reference = &frames[..size];
    let (mut darkest, mut brightest) = (u8::MAX, 0u8);
    for row in (0..box_height).step_by(2) {
        for column in (0..box_width).step_by(2) {
            let pixel = reference[(y + row) * width + x + column];
            darkest = darkest.min(pixel);
            brightest = brightest.max(pixel);
        }
    }
    if brightest.saturating_sub(darkest) < LEAST_CONTRAST {
        return None;
    }
    let difference = |image: &[u8], at_x: usize, at_y: usize| {
        let (mut sum, mut pixels) = (0u64, 0u64);
        for row in (0..box_height).step_by(2) {
            for column in (0..box_width).step_by(2) {
                let was = reference[(y + row) * width + x + column];
                let now = image[(at_y + row) * width + at_x + column];
                sum += u64::from(was.abs_diff(now));
                pixels += 1;
            }
        }
        sum as f64 / pixels.max(1) as f64
    };
    let fps = if fps > 0.0 { fps } else { 30.0 };
    let mut boxes = vec![((start * fps).round() as i64, seed)];
    let mut reliable_end = end;
    let (mut previous_x, mut previous_y) = (x, y);
    let radius = LEAST_BOX.max(8).max(box_width.min(box_height) / 2);
    let last_frame = (end * fps).round() as i64;
    for index in 1..count {
        let image = &frames[index * size..(index + 1) * size];
        let (mut best, mut best_x, mut best_y) = (f64::INFINITY, previous_x, previous_y);
        let top = previous_y.saturating_sub(radius);
        let bottom = (height - box_height).min(previous_y + radius);
        let left = previous_x.saturating_sub(radius);
        let right = (width - box_width).min(previous_x + radius);
        for at_y in top..=bottom {
            for at_x in left..=right {
                let score = difference(image, at_x, at_y);
                if score < best {
                    (best, best_x, best_y) = (score, at_x, at_y);
                }
            }
        }
        let time = start + index as f64 / TRACK_RATE;
        if best > LOST_ABOVE {
            reliable_end = end.min(time);
            break;
        }
        let frame = (time * fps).round() as i64;
        if frame >= last_frame {
            break;
        }
        let moved = best_x.abs_diff(previous_x) > 1 || best_y.abs_diff(previous_y) > 1;
        if frame > boxes.last().map_or(i64::MIN, |last| last.0) && moved {
            boxes.push((
                frame,
                Region {
                    x: (best_x as f64 / width as f64).min(1.0 - seed.width),
                    y: (best_y as f64 / height as f64).min(1.0 - seed.height),
                    width: seed.width,
                    height: seed.height,
                },
            ));
        }
        (previous_x, previous_y) = (best_x, best_y);
    }
    Some(Tracked {
        boxes,
        reliable_end,
    })
}

/// A time typed in Start or End, in seconds of the program: seconds, "4.5
/// s", frames ("12f") or a timecode, as Kerf names times.
pub fn typed_time(text: &str, fps: f64) -> Option<f64> {
    let text = text.trim().trim_end_matches(" s").trim();
    model::focus_seconds(&Value::String(text.to_owned()), fps, 0.0)
}

/// A time as Start and End show it: seconds to the thousandth.
pub fn shown_time(seconds: f64) -> String {
    format!("{seconds:.3} s")
}

/// The clip Focus Region works on, and where it opens.
#[derive(Clone, Debug)]
pub struct Target {
    pub clip: Clip,
    /// Its media's file, read against the project's folder.
    pub file: PathBuf,
    pub fps: f64,
    /// The playhead, kept inside the clip: the frame the box is drawn on.
    pub at: f64,
    /// And that frame's second in the media.
    pub seconds: f64,
}

/// The frame of `clip` at program second `at` that a box is drawn on: its
/// media's file and the second in it. [`NO_FRAME`] for a clip with no
/// media to read (a title, a solid) or whose file is not there, refused
/// before the dialog opens, which would have no frame to draw the box on.
pub fn readable_frame(
    file: Option<PathBuf>,
    clip: &Clip,
    at: f64,
) -> Result<(PathBuf, f64), &'static str> {
    let file = file.filter(|file| file.is_file()).ok_or(NO_FRAME)?;
    let seconds = clip
        .source_at(at - clip.timeline_start)
        .filter(|seconds| seconds.is_finite())
        .ok_or(NO_FRAME)?;
    Ok((file, seconds.max(0.0)))
}

/// The clip Focus Region would work on in `session` now: the chosen clip,
/// on a picture track, longer than a frame, with a frame of its media to
/// draw the box on. Refused while a proposal waits ([`PENDING`]), with no
/// such clip ([`INVALID`]), with no frame to read ([`NO_FRAME`]).
pub fn target(session: &Session) -> Result<Target, &'static str> {
    if session.state.snapshot.pending().is_some() {
        return Err(PENDING);
    }
    let timeline = session.timeline();
    let id = session.selection.first().ok_or(INVALID)?;
    let (track, clip) = timeline.clip(id).ok_or(INVALID)?;
    let fps = timeline.rate();
    if track.kind != TrackKind::Video || clip.duration <= 1.0 / fps {
        return Err(INVALID);
    }
    let file = clip
        .source_id
        .as_deref()
        .and_then(|id| timeline.source(id))
        .filter(|source| !source.path.is_empty())
        .map(|source| {
            let path = PathBuf::from(&source.path);
            match (&session.project, path.is_absolute()) {
                (Some(project), false) => project.join(path),
                _ => path,
            }
        });
    let at = session
        .state
        .position
        .clamp(clip.timeline_start, clip.end() - 1.0 / fps);
    let (file, seconds) = readable_frame(file, clip, at)?;
    Ok(Target {
        clip: clip.clone(),
        file,
        fps,
        at,
        seconds,
    })
}

/// On Windows a console tool the editor starts opens no window of its own.
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// What FFmpeg writes for `args`, if it ends well.
fn run_ffmpeg(args: &[String]) -> Option<Vec<u8>> {
    let mut command = Command::new(ffmpeg_bin());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    no_window(&mut command);
    let output = command.output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// The frame `seconds` into `file`, at most [`FRAME_TALL`] tall.
fn decode_frame(file: &Path, seconds: f64) -> Option<Arc<RenderImage>> {
    let args: Vec<String> = [
        "-nostdin",
        "-v",
        "error",
        "-ss",
        &format!("{:.6}", seconds.max(0.0)),
        "-i",
        &file.to_string_lossy(),
        "-frames:v",
        "1",
        "-an",
        "-vf",
        &format!("scale=-2:'min({FRAME_TALL},ih)'"),
        "-f",
        "image2pipe",
        "-c:v",
        "pam",
        "-pix_fmt",
        "rgba",
        "pipe:1",
    ]
    .map(str::to_owned)
    .to_vec();
    let out = run_ffmpeg(&args)?;
    let picture = editor_session::artwork::parse_pam_stream(&out)
        .into_iter()
        .next()?;
    editor_session::artwork::render_image(&picture)
}

/// The grey frames a subject is followed in: `clip`'s media from program
/// second `start` to `end`, [`TRACK_RATE`] a second, [`TRACK_SIZE`].
fn track_frames(file: &Path, clip: &Clip, start: f64, end: f64) -> Option<Vec<u8>> {
    let speed = clip.speed.unwrap_or(1.0).abs().max(1e-6);
    let from = clip.source_at(start - clip.timeline_start)?;
    let (width, height) = TRACK_SIZE;
    let args: Vec<String> = [
        "-nostdin".to_owned(),
        "-v".to_owned(),
        "error".to_owned(),
        "-ss".to_owned(),
        format!("{:.6}", from.max(0.0)),
        "-t".to_owned(),
        format!("{:.6}", (end - start) * speed),
        "-i".to_owned(),
        file.to_string_lossy().into_owned(),
        "-vf".to_owned(),
        format!(
            "fps={},scale={width}:{height},format=gray",
            TRACK_RATE * speed
        ),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "gray".to_owned(),
        "pipe:1".to_owned(),
    ]
    .to_vec();
    run_ffmpeg(&args)
}

/// Where the dialog stands.
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    /// A box and a range are being chosen.
    Choosing,
    /// Waiting for the media's index, to look for a scene cut.
    Checking,
    /// The subject is being followed.
    Following,
}

/// Edit ▸ Focus Region…'s dialog.
pub struct FocusRegion {
    session: Entity<Session>,
    focus: FocusHandle,
    target: Target,
    /// The frame the box is drawn on, once decoded; the media's poster
    /// stands in until then.
    frame: Option<Arc<RenderImage>>,
    frame_failed: bool,
    region: Option<Region>,
    old_crop: Option<Region>,
    /// Where a drag of a new box began, in shares of the frame.
    drag_from: Option<(f64, f64)>,
    /// Where the frame was drawn last, in the window.
    picture: Rc<Cell<Option<Bounds<Pixels>>>>,
    start: Entity<TextInput>,
    end: Entity<TextInput>,
    follow: bool,
    stage: Stage,
    problem: Option<SharedString>,
    work: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for FocusRegion {}

impl Focusable for FocusRegion {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Binds Esc and Return in the dialog, unless they are (binding a keymap
/// clears them).
fn bind_keys(cx: &mut App) {
    if !editor_launcher::settings::escape_bound(cx, &CloseFocus) {
        cx.bind_keys([
            KeyBinding::new("escape", CloseFocus, Some(CONTEXT)),
            KeyBinding::new("enter", ApplyFocus, Some(CONTEXT)),
        ]);
    }
}

impl FocusRegion {
    pub fn new(
        session: Entity<Session>,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        bind_keys(cx);
        let clip = &target.clip;
        let old_crop = crop_box(&Value::Object(clip.props.clone()));
        let start_at = if old_crop.is_none() {
            target.at
        } else {
            clip.timeline_start
        };
        let end_at = if old_crop.is_none() {
            clip.end().min(start_at + SUGGESTED)
        } else {
            clip.end()
        };
        let field = |cx: &mut Context<Self>, at: f64| {
            cx.new(|cx| {
                let mut field = TextInput::new(cx, "0.000 s").with_style(text::CONTROL);
                field.set_text(shown_time(at), cx);
                field
            })
        };
        let start = field(cx, start_at);
        let end = field(cx, end_at);
        let mut subscriptions = Vec::new();
        for input in [&start, &end] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.apply(window, cx),
                    InputEvent::Cancel => cx.emit(DismissEvent),
                    InputEvent::Changed => {
                        this.problem = None;
                        cx.notify();
                    }
                },
            ));
        }
        // The media's index comes, or cannot be read: a range waiting on
        // it goes on.
        subscriptions.push(cx.subscribe_in(
            &session,
            window,
            |this, _, change: &Change, window, cx| {
                if this.stage == Stage::Checking && *change != Change::Clock {
                    this.apply(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &session,
            window,
            |this, _, event: &BridgeEvent, window, cx| {
                let indexed = match &*event.0 {
                    Event::MediaIndex(_) => true,
                    Event::Error(error) => error.command.as_deref() == Some("media_index"),
                    _ => false,
                };
                if this.stage == Stage::Checking && indexed {
                    this.apply(window, cx);
                }
            },
        ));
        // Its cuts are asked for now, so they are there when OK is pressed.
        if let Some(media) = clip.source_id.clone() {
            session.update(cx, |session, _| session.ask_media_index(&media));
        }
        let mut dialog = Self {
            session,
            focus: cx.focus_handle(),
            region: old_crop,
            old_crop,
            target,
            frame: None,
            frame_failed: false,
            drag_from: None,
            picture: Rc::new(Cell::new(None)),
            start,
            end,
            follow: false,
            stage: Stage::Choosing,
            problem: None,
            work: None,
            _subscriptions: subscriptions,
        };
        dialog.read_frame(cx);
        dialog
    }

    /// Decodes the frame at the playhead, off the window's thread. One
    /// that cannot be read leaves nothing to draw on: no box, no OK.
    fn read_frame(&mut self, cx: &mut Context<Self>) {
        let (file, seconds) = (self.target.file.clone(), self.target.seconds);
        let decoding = cx
            .background_executor()
            .spawn(async move { decode_frame(&file, seconds) });
        self.work = Some(cx.spawn(async move |this, cx| {
            let frame = decoding.await;
            let _ = this.update(cx, |this, cx| {
                this.frame_failed = frame.is_none();
                if this.frame_failed {
                    this.region = None;
                    this.drag_from = None;
                }
                this.frame = frame;
                this.work = None;
                cx.notify();
            });
        }));
    }

    /// The picture to draw the box on: the frame, else the media's poster
    /// until it comes; nothing once it could not be read.
    fn shown_frame(&self, cx: &App) -> Option<Arc<RenderImage>> {
        if self.frame_failed {
            return None;
        }
        self.frame.clone().or_else(|| {
            let id = self.target.clip.source_id.as_deref()?;
            self.session.read(cx).artwork.poster(id)
        })
    }

    /// The range typed, in seconds of the program, if it reads. A
    /// subject is followed from the frame the box is drawn on, so Start is
    /// then that frame's, whatever its field held.
    fn range(&self, cx: &App) -> Option<(f64, f64)> {
        let fps = self.target.fps;
        let start = if self.follow {
            self.target.at
        } else {
            typed_time(self.start.read(cx).text(), fps)?
        };
        let end = typed_time(self.end.read(cx).text(), fps)?;
        Some((start, end))
    }

    /// OK: the edit is written, after the media's cuts are known and the
    /// subject followed when asked. What stops it is said in the dialog.
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stage == Stage::Following {
            return;
        }
        if self.frame_failed {
            return self.refuse(NO_FRAME, cx);
        }
        let clip = self.target.clip.clone();
        let fps = self.target.fps;
        let frame_of = |seconds: f64| (seconds * fps).round() as i64;
        let clip_frames = (frame_of(clip.timeline_start), frame_of(clip.end()));
        let Some(region) = self.region.filter(|region| !region.is_empty()) else {
            return self.refuse(INVALID, cx);
        };
        let Some((start, end)) = self.range(cx) else {
            return self.refuse(INVALID, cx);
        };
        let (start_frame, mut end_frame) = (frame_of(start), frame_of(end));
        if start_frame < clip_frames.0 || end_frame > clip_frames.1 || start_frame >= end_frame {
            return self.refuse(INVALID, cx);
        }
        // A scene cut inside the range ends the box at it.
        let mut said = Vec::new();
        if let Some(media) = clip.source_id.clone() {
            let session = self.session.read(cx);
            if let Some(index) = session.state.media_indexes.get(&media) {
                let cuts: Vec<f64> = index["cuts"]
                    .as_array()
                    .map(|cuts| cuts.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                if let Some(cut) = first_cut(&cuts, &clip, start, end, fps) {
                    end_frame = frame_of(cut);
                    said.push(CUT_FOUND);
                }
            } else if session.state.media_index_errors.contains_key(&media) {
                self.stage = Stage::Choosing;
                return self.refuse(NO_CUTS, cx);
            } else {
                // Asked for as the dialog opened; it goes on when it comes.
                self.stage = Stage::Checking;
                cx.notify();
                return;
            }
        }
        self.stage = Stage::Choosing;
        let end_at = end_frame as f64 / fps;
        if !self.follow {
            return self.write(region, (start_frame, end_frame), &[], said, window, cx);
        }
        if end_at - start > TRACK_MOST {
            return self.refuse(NO_TRACK, cx);
        }
        let file = self.target.file.clone();
        self.stage = Stage::Following;
        self.problem = None;
        cx.notify();
        let following = cx.background_executor().spawn(async move {
            let frames = track_frames(&file, &clip, start, end_at)?;
            track_region(&frames, TRACK_SIZE, region, (start, end_at), fps)
        });
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            let tracked = following.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.stage = Stage::Choosing;
                this.work = None;
                let Some(tracked) = tracked else {
                    return this.refuse(NO_TRACK, cx);
                };
                let mut said = said;
                let mut end_frame = end_frame;
                if tracked.reliable_end < end_at {
                    end_frame = (tracked.reliable_end * fps).round() as i64;
                    said.push(LOST);
                }
                this.write(
                    region,
                    (start_frame, end_frame),
                    &tracked.boxes,
                    said,
                    window,
                    cx,
                );
            });
        }));
    }

    /// Sends the edit, chooses the part focused, says what changed the
    /// range, and gives the window back.
    fn write(
        &mut self,
        region: Region,
        range: (i64, i64),
        tracked: &[(i64, Region)],
        said: Vec<&str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clip = &self.target.clip;
        let fps = self.target.fps;
        let frame_of = |seconds: f64| (seconds * fps).round() as i64;
        if range.0 >= range.1 {
            return self.refuse(INVALID, cx);
        }
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default()
            .to_string();
        let (ops, focused) = focus_ops(
            &clip.id,
            (frame_of(clip.timeline_start), frame_of(clip.end())),
            range,
            region,
            tracked,
            self.old_crop,
            &suffix,
        );
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.edit(ops);
            if sent {
                session.selection = vec![focused];
                for words in &said {
                    session.state.notify(words, false);
                }
            }
            cx.notify();
            sent
        });
        if sent {
            cx.emit(DismissEvent);
        } else {
            self.refuse("Kerf could not take the edit now", cx);
        }
    }

    fn refuse(&mut self, words: &str, cx: &mut Context<Self>) {
        self.problem = Some(SharedString::from(words.to_owned()));
        cx.notify();
    }

    /// The box under the pointer at `at` (in the window), in shares of the
    /// frame as drawn.
    fn share_at(&self, at: gpui::Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.picture.get()?;
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        Some((
            f64::from(f32::from(at.x - bounds.origin.x) / width),
            f64::from(f32::from(at.y - bounds.origin.y) / height),
        ))
    }

    fn press(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.frame_failed {
            return;
        }
        let Some(at) = self.share_at(event.position) else {
            return;
        };
        if !(0.0..=1.0).contains(&at.0) || !(0.0..=1.0).contains(&at.1) {
            return;
        }
        self.drag_from = Some(at);
        self.region = None;
        self.problem = None;
        cx.notify();
    }

    fn drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (Some(from), Some(to)) = (self.drag_from, self.share_at(event.position)) else {
            return;
        };
        self.region = Some(Region::between(from, to));
        cx.notify();
    }

    fn release(&mut self, at: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(from) = self.drag_from.take() else {
            return;
        };
        let to = self.share_at(at).unwrap_or(from);
        self.region = Some(Region::between(from, to)).filter(|region| !region.is_empty());
        cx.notify();
    }

    /// Follow a moving subject: it is followed from the frame the box is
    /// drawn on, so Start is that frame's and cannot be typed; off, Start
    /// can be typed again, from there.
    fn set_follow(&mut self, follow: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.follow = follow;
        if follow {
            let at = self.target.at;
            self.start
                .update(cx, |field, cx| field.set_text(shown_time(at), cx));
            // Its field goes: the keys it held come back to the dialog.
            if self.start.focus_handle(cx).contains_focused(window, cx) {
                window.focus(&self.focus);
            }
        }
        self.problem = None;
        cx.notify();
    }

    /// The frame with the box over it, drawn by dragging.
    fn picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.shown_frame(cx);
        let aspect = frame
            .as_ref()
            .map(|frame| editor_session::artwork::image_size(frame))
            .filter(|(width, height)| *width > 0 && *height > 0)
            .map_or(16.0 / 9.0, |(width, height)| width as f32 / height as f32);
        let tall = (PICKER_WIDE / aspect).clamp(160.0, 360.0);
        let region = self.region;
        let cell = self.picture.clone();
        let waiting = frame.is_none();
        let failed = self.frame_failed && waiting;
        div()
            .id("focus-picker")
            .relative()
            .w(px(PICKER_WIDE))
            .h(px(tall))
            .rounded(px(theme::radius::CARD))
            .overflow_hidden()
            .bg(theme::overlay_screen())
            .cursor_crosshair()
            .child(
                canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        let placed = match &frame {
                            Some(frame) => {
                                let placed = editor_session::artwork::fitted(
                                    bounds,
                                    editor_session::artwork::image_size(frame),
                                    false,
                                );
                                if let Err(error) = window.paint_image(
                                    placed,
                                    gpui::Corners::default(),
                                    frame.clone(),
                                    0,
                                    false,
                                ) {
                                    log::warn!("Focus Region's frame did not draw: {error}");
                                }
                                placed
                            }
                            None => bounds,
                        };
                        cell.set(Some(placed));
                        if let Some(region) = region {
                            let (left, top) = (placed.origin.x, placed.origin.y);
                            let (width, height) =
                                (f32::from(placed.size.width), f32::from(placed.size.height));
                            let x0 = left + px(region.x as f32 * width);
                            let y0 = top + px(region.y as f32 * height);
                            let x1 = x0 + px(region.width as f32 * width);
                            let y1 = y0 + px(region.height as f32 * height);
                            window.paint_quad(gpui::fill(
                                Bounds::from_corners(point(x0, y0), point(x1, y1)),
                                theme::with_alpha(over_picture::line(), 0.14),
                            ));
                            editor_stage::stage::stroke_over(
                                window,
                                &[point(x0, y0), point(x1, y0), point(x1, y1), point(x0, y1)],
                                true,
                                true,
                                false,
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .when(waiting, |picker| {
                picker.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::label(
                            if failed {
                                NO_FRAME
                            } else {
                                "Reading a frame…"
                            },
                            text::CONTROL,
                            theme::ink3(),
                        )),
                )
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| this.drag(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.release(event.position, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.release(event.position, cx)
                }),
            )
    }

    /// A time and its field; `held`, the time it is held at, shown in
    /// place of the field, greyed, so it cannot be typed in.
    fn time_field(
        &self,
        words: &'static str,
        field: &Entity<TextInput>,
        held: Option<String>,
    ) -> gpui::Div {
        let place = div()
            .w(px(120.0))
            .h(px(theme::height::H28))
            .flex()
            .items_center()
            .px(px(theme::space::S8))
            .rounded(px(theme::radius::CONTROL))
            .bg(theme::card())
            .border_1()
            .border_color(theme::line2());
        let place = match held {
            Some(time) => place
                .opacity(0.5)
                .child(ui::line(time, text::CONTROL, theme::ink())),
            None => place.child(field.clone()),
        };
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .child(
                ui::line(words, text::CONTROL, theme::ink2())
                    .w(px(40.0))
                    .flex_none(),
            )
            .child(place)
    }
}

impl Render for FocusRegion {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = match self.stage {
            Stage::Choosing => None,
            Stage::Checking => Some("Looking for scene cuts…"),
            Stage::Following => Some("Following the subject…"),
        };
        let can_apply = busy.is_none()
            && !self.frame_failed
            && self.region.is_some_and(|region| !region.is_empty());
        let follow = self.follow;
        let clip = self.target.clip.label().to_owned();
        div()
            .key_context(format!("{} {CONTEXT}", ui::PANEL).as_str())
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &CloseFocus, _, cx| cx.emit(DismissEvent)))
            .on_action(cx.listener(|this, _: &ApplyFocus, window, cx| this.apply(window, cx)))
            // A press on the dim around it does nothing, so a stray click
            // does not lose the box drawn and the range typed: they stay
            // until Esc or its own buttons close it.
            .w(px(DIALOG_WIDE))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S12))
            .bg(theme::popup())
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(theme::space::S8))
                    .child(ui::label("Focus region", text::TITLE, theme::ink()))
                    .child(ui::line(clip, text::CONTROL, theme::ink3()).min_w_0()),
            )
            .child(ui::label(
                "Drag a box over the area to enlarge.",
                text::BODY,
                theme::ink2(),
            ))
            .child(self.picker(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(20.0))
                    .child(self.time_field(
                        "Start",
                        &self.start,
                        follow.then(|| shown_time(self.target.at)),
                    ))
                    .child(self.time_field("End", &self.end, None)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(ui::switch("focus-follow", follow, {
                        let this = cx.entity().downgrade();
                        move |on, window, cx| {
                            let _ = this.update(cx, |this, cx| this.set_follow(on, window, cx));
                        }
                    }))
                    .child(ui::label(
                        "Follow a moving subject (up to 2 minutes)",
                        text::CONTROL,
                        theme::ink(),
                    )),
            )
            .when_some(self.problem.clone(), |dialog, problem| {
                dialog.child(ui::label(problem, text::CONTROL, theme::neg()))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .when_some(busy, |row, busy| {
                        row.child(
                            ui::line(busy, text::CONTROL, theme::ink3())
                                .flex_1()
                                .min_w_0(),
                        )
                    })
                    .child(
                        ui::button("focus-cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        ui::button("focus-ok", "OK")
                            .primary()
                            .disabled(!can_apply)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
    }
}
