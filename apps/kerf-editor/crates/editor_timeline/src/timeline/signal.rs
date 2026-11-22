//! A signal clip's number as a curve: the points it passes through
//! ([`signal_points`]); drawing it anew by hand with an Option-drag
//! ([`SignalDraw`], [`signal_at`]), its keys thinned as it is let go and
//! the drawing shown until kerfd has it; and its painting.

use super::canvas::Pen;
use super::drag::Gesture;
use super::lane_ink;
use super::TimelineView;
use editor_state::model;
use editor_ui::theme;
use gpui::{fill, px, Context, PathBuilder, Window};
use serde_json::json;
use std::collections::BTreeMap;

/// How far a hand-drawn signal may stray from the line between two keys.
const SIGNAL_WITHIN: f64 = 0.02;

/// A signal being drawn by hand across its clip: a value 0..1 at each frame.
/// It reads the pointer against the clip's box as it is drawn at each move,
/// never as it was at the press: choosing the clip brings the Inspector in,
/// which moves the timeline while the drag goes on.
pub(super) struct SignalDraw {
    pub(super) clip: String,
    pub(super) frames: i64,
    pub(super) drawn: BTreeMap<i64, f64>,
    pub(super) last: Option<i64>,
}

/// The frame of a signal clip `frames` long, and the value 0..1, that the
/// pointer at `x`, `y` draws over the clip's box `(left, top, width,
/// height)` (its curve stands 2 points in from its top and its foot, as
/// it is drawn); None when the box has no width.
pub fn signal_at(
    x: f32,
    y: f32,
    (left, top, across, tall): (f32, f32, f32, f32),
    frames: i64,
) -> Option<(i64, f64)> {
    if across <= 0.0 {
        return None;
    }
    let (top, tall) = (top + 2.0, (tall - 4.0).max(1.0));
    let last = (frames - 1).max(0);
    let frame = (((x - left) / across) as f64 * last as f64)
        .round()
        .clamp(0.0, last as f64) as i64;
    let value = (1.0 - ((y - top) / tall) as f64).clamp(0.0, 1.0);
    Some((frame, value))
}

impl SignalDraw {
    /// The pointer at `x`, `y` over the clip's box `rect` as drawn now:
    /// every frame it passed since the last is on the line between the two.
    pub(super) fn draw_to(&mut self, x: f32, y: f32, rect: (f32, f32, f32, f32)) {
        let Some((frame, value)) = signal_at(x, y, rect, self.frames) else {
            return;
        };
        if let Some(from) = self.last.filter(|from| *from != frame) {
            let start = self.drawn.get(&from).copied().unwrap_or(value);
            let step = if frame > from { 1 } else { -1 };
            let mut at = from + step;
            while at != frame {
                let share = (at - from) as f64 / (frame - from) as f64;
                self.drawn.insert(at, start + (value - start) * share);
                at += step;
            }
        }
        self.drawn.insert(frame, value);
        self.last = Some(frame);
    }
}

/// A signal's values 0..1 spread across `(x, y, w, h)`, filled under.
pub(super) fn paint_signal(
    values: &[f64],
    (x, y, w, h): (f32, f32, f32, f32),
    pen: Pen,
    window: &mut Window,
) {
    let points = signal_points(values, (x, y, w, h));
    if points.len() < 2 {
        return;
    }
    let mut under = PathBuilder::fill();
    under.move_to(pen.at(points[0].0, y + h));
    for (px_, py) in &points {
        under.line_to(pen.at(*px_, *py));
    }
    under.line_to(pen.at(points[points.len() - 1].0, y + h));
    under.close();
    if let Ok(path) = under.build() {
        window.paint_path(path, lane_ink::peak(false));
    }
    let mut line = PathBuilder::stroke(px(1.0));
    line.move_to(pen.at(points[0].0, points[0].1));
    for (px_, py) in &points[1..] {
        line.line_to(pen.at(*px_, *py));
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, lane_ink::rms(false));
    }
}

/// The points a signal's curve passes through across a box: one value is
/// a level across the whole of it.
pub fn signal_points(values: &[f64], (x, y, w, h): (f32, f32, f32, f32)) -> Vec<(f32, f32)> {
    if values.is_empty() || w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let last = values.len() - 1;
    let mut points: Vec<(f32, f32)> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let along = if last == 0 {
                0.0
            } else {
                index as f32 / last as f32
            };
            (x + along * w, y + h - value.clamp(0.0, 1.0) as f32 * h)
        })
        .collect();
    if last == 0 {
        points.push((x + w, points[0].1));
    }
    points
}

/// A signal being drawn by hand, over the clip's own, in ink.
pub(super) fn paint_drawn(
    drawn: &BTreeMap<i64, f64>,
    frames: i64,
    (x, y, w, h): (f32, f32, f32, f32),
    pen: Pen,
    window: &mut Window,
) {
    let last = (frames - 1).max(1) as f32;
    let mut line = PathBuilder::stroke(px(1.5));
    for (index, (frame, value)) in drawn.iter().enumerate() {
        let at = pen.at(x + w * *frame as f32 / last, y + h - h * *value as f32);
        if index == 0 {
            line.move_to(at);
        } else {
            line.line_to(at);
        }
    }
    if drawn.len() == 1 {
        if let Some((frame, value)) = drawn.iter().next() {
            let (cx_, cy) = (x + w * *frame as f32 / last, y + h - h * *value as f32);
            window.paint_quad(fill(pen.rect(cx_ - 1.0, cy - 1.0, 2.0, 2.0), theme::ink()));
        }
        return;
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, theme::ink());
    }
}

impl TimelineView {
    /// Sets the signal drawn: its keys, thinned. A click draws nothing.
    pub(super) fn drop_signal(&mut self, draw: SignalDraw, cx: &mut Context<Self>) {
        if draw.drawn.len() < 2 {
            return;
        }
        let keys = model::signal_keys(&draw.drawn, SIGNAL_WITHIN);
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, cx| {
            let op = json!({"op": "set", "clip": draw.clip, "props": {"value": {"keys": keys}}});
            let sent = session.edit(vec![op]);
            cx.notify();
            sent
        });
        if sent {
            self.drawn = Some((draw.clip, draw.drawn, revision));
            self.pending_marker(revision);
        }
    }

    /// A signal as it is drawn, or as it was drawn until kerfd has it.
    pub(super) fn signal_shown(&self) -> Option<(String, BTreeMap<i64, f64>)> {
        match &self.gesture {
            Some(Gesture::Signal(draw)) => Some((draw.clip.clone(), draw.drawn.clone())),
            _ => self
                .drawn
                .as_ref()
                .map(|(clip, drawn, _)| (clip.clone(), drawn.clone())),
        }
    }
}
