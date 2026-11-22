//! Where the lanes look ([`Viewport`]): their size, the zoom (the fit, −
//! and +, the slider's log scale, ⌘-scroll around the pointer), how far
//! they are scrolled across and down and the bar that shows it, and the
//! page they turn to as the playhead leaves them; laid out again whenever
//! the view changes or asks for a frame.

use super::heads::ADD_ROW;
use super::lanes::Row;
use super::TimelineView;
use crate::theme::{layout, text};
use gpui::{px, App, Context, Pixels, Point, ScrollWheelEvent, Window};

/// Points before time zero, so the playhead's head shows whole at the start.
pub(super) const PAD: f32 = 10.0;
/// Points the playhead keeps from the right edge before the view pages on.
const FOLLOW_MARGIN: f32 = 24.0;
/// Points from either end of the lanes where a drag pans them.
const EDGE_PAN: f32 = 16.0;
/// The widest a frame is drawn, in points.
const MOST_FRAME: f32 = 48.0;
/// What one press of − or + zooms by.
pub(super) const ZOOM_STEP: f32 = 1.5;
/// Points of the bar under the lanes that shows where a zoomed view is.
pub(super) const SCROLLER: f32 = 5.0;

/// Where the lanes look: their size, the zoom, and how far they are scrolled.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Viewport {
    pub(super) width: f32,
    pub(super) height: f32,
    /// Points a second.
    pub(super) pps: f32,
    /// The time at the left edge, in seconds.
    pub(super) left: f64,
    pub(super) scroll_y: f32,
}

impl Viewport {
    pub(super) fn x(&self, time: f64) -> f32 {
        PAD + ((time - self.left) * self.pps as f64) as f32
    }

    pub(super) fn time(&self, x: f32) -> f64 {
        self.left + ((x - PAD) / self.pps.max(f32::EPSILON)) as f64
    }

    /// Seconds the lanes show across.
    pub(super) fn seconds(&self) -> f64 {
        ((self.width - PAD).max(0.0) / self.pps.max(f32::EPSILON)) as f64
    }

    /// Points the lanes show down, under the ruler.
    pub(super) fn lanes_height(&self) -> f32 {
        (self.height - layout::RULER).max(0.0)
    }

    /// Where `row` is drawn, from the canvas's top.
    pub(super) fn row_y(&self, row: &Row) -> f32 {
        layout::RULER + row.top - self.scroll_y
    }
}

/// Points a second that show `end` seconds across `width`.
fn fit_pps(width: f32, end: f64) -> f32 {
    (width - 2.0 * PAD).max(1.0) / end.max(1.0) as f32
}

/// The latest the left edge goes: the program's end halfway across.
fn max_left(view: &Viewport, end: f64) -> f64 {
    (end - view.seconds() * 0.5).max(0.0)
}

/// The span a zoomed view's scroll bar stands for, in seconds: the
/// program and the half view after it that the left edge can reach.
fn scroll_span(view: &Viewport, end: f64) -> f64 {
    (max_left(view, end) + view.seconds())
        .max(view.seconds())
        .max(1e-6)
}

/// The scroll bar's thumb, left and width in points across the lanes.
pub(super) fn thumb(view: &Viewport, end: f64) -> (f32, f32) {
    let span = scroll_span(view, end);
    let track = (view.width - 2.0 * PAD).max(1.0);
    let width = ((view.seconds() / span) as f32 * track).clamp(24.0_f32.min(track), track);
    let left = PAD + (view.left / span) as f32 * track;
    (left.min(PAD + track - width), width)
}

/// Where the zoom slider stands, 0..1, for `pps` points a second between
/// the fit and the most: on a log scale, so each step zooms alike.
pub fn zoom_share(pps: f32, fit: f32, most: f32) -> f32 {
    if most <= fit * 1.001 || pps <= fit {
        return 0.0;
    }
    ((pps / fit).ln() / (most / fit).ln()).clamp(0.0, 1.0)
}

/// The points a second the slider at `share` gives; within a 50th of its
/// travel from the start it holds at the fit (the detent).
pub fn zoom_of_share(share: f32, fit: f32, most: f32) -> f32 {
    if share <= 0.02 || most <= fit {
        return fit;
    }
    fit * (most / fit).powf(share.clamp(0.0, 1.0))
}

impl TimelineView {
    /// The lanes laid out again as the timeline draws: at the size the
    /// canvas was last laid out at (a new size asks for another frame),
    /// with the playhead where it is now, the same the frame draws.
    pub(super) fn lay_out(&mut self, cx: &App) {
        let bounds = self.bounds.get();
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let position = self.session.read(cx).state.position;
        self.view = self.layout(width, height, position);
    }

    /// The zoom, scroll and follow for lanes `width` × `height` with the
    /// playhead at `position`.
    fn layout(&mut self, width: f32, height: f32, position: f64) -> Viewport {
        let end = self.lanes.end;
        let fit = fit_pps(width, end);
        let most = (self.lanes.fps as f32 * MOST_FRAME).max(fit);
        let pps = match self.zoom {
            Some(zoom) if width <= 0.0 || zoom > fit => zoom.min(most),
            _ => {
                self.zoom = None;
                fit
            }
        };
        let mut view = Viewport {
            width,
            height,
            pps,
            left: if self.zoom.is_some() { self.left } else { 0.0 },
            scroll_y: self.scroll_y,
        };
        // The view pages on when the playhead moves out of it: playing, or sent there.
        if position != self.last_position && self.gesture.is_none() && width > 0.0 {
            let x = view.x(position);
            if x < PAD || x > width - FOLLOW_MARGIN {
                view.left = position - view.seconds() * 0.1;
            }
        }
        self.last_position = position;
        view.left = view.left.clamp(0.0, max_left(&view, end));
        view.scroll_y = view.scroll_y.clamp(
            0.0,
            (self.lanes.height + ADD_ROW - view.lanes_height()).max(0.0),
        );
        self.left = view.left;
        self.scroll_y = view.scroll_y;
        view
    }

    /// The fit and the most points a second at the lanes' width.
    pub(super) fn zoom_range(&self) -> (f32, f32) {
        let fit = fit_pps(self.view.width, self.lanes.end);
        (fit, (self.lanes.fps as f32 * MOST_FRAME).max(fit))
    }

    /// A window position on the canvas.
    pub(super) fn local(&self, position: Point<Pixels>) -> (f32, f32) {
        let origin = self.bounds.get().origin;
        (
            f32::from(position.x - origin.x),
            f32::from(position.y - origin.y),
        )
    }

    pub(super) fn wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(text::BODY.line));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let overflow = (self.lanes.height + ADD_ROW - self.view.lanes_height()).max(0.0);
        if event.modifiers.secondary() {
            // Zooms around the pointer.
            let (x, _) = self.local(event.position);
            self.zoom_by((dy * 0.01).exp(), Some(x));
        } else if event.modifiers.shift || dx.abs() > dy.abs() {
            self.pan(-if dx.abs() > dy.abs() { dx } else { dy });
        } else if overflow > 0.0 {
            self.scroll_y = (self.scroll_y - dy).clamp(0.0, overflow);
        } else {
            self.pan(-dy);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Pans by `points`, later when positive.
    fn pan(&mut self, points: f32) {
        if self.zoom.is_none() {
            return;
        }
        let left = self.left + (points / self.view.pps.max(f32::EPSILON)) as f64;
        self.left = left.clamp(0.0, max_left(&self.view, self.lanes.end));
    }

    /// Zooms by `factor` around `anchor` points across the lanes, else
    /// around the playhead when it shows, else the middle. Zooming out
    /// stops at the fit.
    pub(super) fn zoom_by(&mut self, factor: f32, anchor: Option<f32>) {
        let view = self.view;
        if view.width <= 0.0 {
            return;
        }
        let (fit, most) = self.zoom_range();
        self.zoom_to((view.pps * factor).clamp(fit, most), anchor);
    }

    fn zoom_to(&mut self, pps: f32, anchor: Option<f32>) {
        let view = self.view;
        let (fit, _) = self.zoom_range();
        let anchor = anchor.unwrap_or_else(|| {
            let x = view.x(self.last_position);
            if (0.0..=view.width).contains(&x) {
                x
            } else {
                view.width / 2.0
            }
        });
        let time = view.time(anchor);
        if pps <= fit * 1.001 {
            self.zoom = None;
            self.left = 0.0;
        } else {
            self.zoom = Some(pps);
            self.left = (time - ((anchor - PAD) / pps) as f64).max(0.0);
        }
        self.view.pps = pps.max(fit);
    }

    /// The zoom slider's knob follows the pointer.
    pub(super) fn slide_zoom(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let bounds = self.slider.get();
        let width = f32::from(bounds.size.width).max(1.0);
        let share = (f32::from(position.x - bounds.origin.x) - 5.0) / (width - 10.0).max(1.0);
        let (fit, most) = self.zoom_range();
        self.zoom_to(zoom_of_share(share.clamp(0.0, 1.0), fit, most), None);
        cx.notify();
    }

    /// The scroll bar's thumb under (`x`, `y`): how far into it.
    pub(super) fn thumb_at(&self, x: f32, y: f32) -> Option<f32> {
        if self.zoom.is_none() || self.view.width <= 0.0 {
            return None;
        }
        let top = self.view.height - SCROLLER - 2.0;
        if y < top - 2.0 || y > self.view.height {
            return None;
        }
        let (left, width) = thumb(&self.view, self.lanes.end);
        (x >= left && x <= left + width).then_some(x - left)
    }

    /// The thumb dragged to `left` points across.
    pub(super) fn drag_thumb(&mut self, left: f32, cx: &mut Context<Self>) {
        let span = scroll_span(&self.view, self.lanes.end);
        let track = (self.view.width - 2.0 * PAD).max(1.0);
        let time = ((left - PAD) / track) as f64 * span;
        self.left = time.clamp(0.0, max_left(&self.view, self.lanes.end));
        cx.notify();
    }

    /// Pans the lanes while a drag holds the pointer near either end.
    pub(super) fn edge_pan(&mut self, x: f32) {
        if self.zoom.is_none() {
            return;
        }
        let width = self.view.width;
        let shift = if x > width - EDGE_PAN {
            x - (width - EDGE_PAN)
        } else if x < EDGE_PAN {
            x - EDGE_PAN
        } else {
            return;
        };
        let left = self.view.left + (shift * 0.5 / self.view.pps) as f64;
        self.view.left = left.clamp(0.0, max_left(&self.view, self.lanes.end));
        self.left = self.view.left;
    }
}
