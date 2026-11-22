//! What changes how tall the tracks are (each track's own height or every
//! track's, [`TrackHeights`], with the ladder they step along in
//! `editor_state`'s track_heights): a head's line dragged ([`ResizeDrag`]),
//! Option-scroll over the heads, a head's menu, View ▸ Fit Tracks to Panel
//! and Reset Track Heights.

use super::heads::ADD_ROW;
use super::{TimelineEvent, TimelineView};
use editor_state::model::TrackKind;
use editor_state::prefs::TrackHeights;
use editor_ui::theme::layout;
use gpui::{px, Context, ScrollDelta, ScrollWheelEvent, TouchPhase, Window};
use std::rc::Rc;

/// Pixels of a trackpad's scroll that make one step of Option-scroll.
const WHEEL_STEP: f32 = 40.0;

/// The line under a head being dragged: one track's height, or every track
/// of its kind (Option).
pub(super) struct ResizeDrag {
    pub(super) track: String,
    pub(super) kind: TrackKind,
    pub(super) every: bool,
    /// The pointer's window y at the press, and the track's height then.
    pub(super) anchor: f32,
    pub(super) from: f32,
}

impl TimelineView {
    /// How tall each track is, as the person left them.
    pub fn track_heights(&self) -> &TrackHeights {
        &self.heights
    }

    /// The heights kept for the project (with its settings), as it opens.
    pub fn set_track_heights(&mut self, heights: TrackHeights, cx: &mut Context<Self>) {
        if self.heights != heights {
            self.heights = heights;
            self.relayout(cx);
        }
    }

    /// View ▸ Fit Tracks to Panel: every track one height, sharing the
    /// panel's height under the ruler with the add-track row in view.
    pub fn fit_tracks(&mut self, cx: &mut Context<Self>) {
        let rows = self.lanes.rows.len();
        if rows == 0 {
            return;
        }
        let room = self.view.lanes_height() - ADD_ROW;
        self.heights.set_all(Some(room / rows as f32));
        self.heights_changed(cx);
    }

    /// View ▸ Reset Track Heights: each kind its own height again.
    pub fn reset_track_heights(&mut self, cx: &mut Context<Self>) {
        self.heights = TrackHeights::default();
        self.heights_changed(cx);
    }

    /// Every track `steps` along the ladder (Option-Up and Option-Down).
    pub fn step_track_heights(&mut self, steps: i32, cx: &mut Context<Self>) {
        let first = self.first_row_shown();
        self.step_keeping(steps, first, cx);
    }

    /// The rows laid out again at the heights now.
    pub(super) fn relayout(&mut self, cx: &mut Context<Self>) {
        let heights = self.heights.clone();
        Rc::make_mut(&mut self.lanes).relayout(&heights);
        cx.notify();
    }

    pub(super) fn heights_changed(&mut self, cx: &mut Context<Self>) {
        self.relayout(cx);
        cx.emit(TimelineEvent::Heights(self.heights.clone()));
    }

    /// The first row whose lane shows under the ruler.
    fn first_row_shown(&self) -> Option<usize> {
        self.lanes
            .row_at(self.scroll_y)
            .or_else(|| (!self.lanes.rows.is_empty()).then_some(0))
    }

    /// Every track `steps` along the ladder, keeping `row` where it shows.
    fn step_keeping(&mut self, steps: i32, row: Option<usize>, cx: &mut Context<Self>) {
        if steps == 0 || self.lanes.rows.is_empty() {
            return;
        }
        let before = row.map(|row| self.lanes.rows[row].top - self.scroll_y);
        let kinds = self.lanes.kinds();
        self.heights.step(&kinds, steps);
        self.heights_changed(cx);
        if let (Some(row), Some(shown)) = (row, before) {
            self.scroll_y = (self.lanes.rows[row].top - shown).max(0.0);
        }
    }

    /// Track `track` (or every track of `kind`, `every`) to `height`.
    pub(super) fn resize_to(
        &mut self,
        track: &str,
        kind: TrackKind,
        every: bool,
        height: f32,
        cx: &mut Context<Self>,
    ) {
        if every {
            let ids: Vec<String> = self
                .lanes
                .rows
                .iter()
                .filter(|row| row.kind == kind)
                .map(|row| row.id.clone())
                .collect();
            for id in ids {
                self.heights.set(&id, Some(height));
            }
        } else {
            self.heights.set(track, Some(height));
        }
        self.relayout(cx);
    }

    /// Option-scroll over the heads steps every track, a step a notch (a
    /// trackpad's small deltas add up to one), keeping the track under the
    /// pointer where it is; otherwise the heads scroll as the lanes do.
    pub(super) fn heads_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.alt {
            self.wheel = 0.0;
            self.wheel(event, window, cx);
            return;
        }
        cx.stop_propagation();
        if matches!(event.touch_phase, TouchPhase::Started | TouchPhase::Ended) {
            self.wheel = 0.0;
        }
        let steps = match event.delta {
            // A notch is one step, however many lines the system says it
            // is (three on Linux); a high-resolution wheel's parts add up.
            ScrollDelta::Lines(lines) => {
                let lines = if lines.y != 0.0 { lines.y } else { lines.x };
                self.wheel += lines;
                if self.wheel.abs() >= 1.0 {
                    let step = self.wheel.signum() as i32;
                    self.wheel = 0.0;
                    step
                } else {
                    0
                }
            }
            ScrollDelta::Pixels(pixels) => {
                let pixels = if pixels.y != px(0.0) {
                    pixels.y
                } else {
                    pixels.x
                };
                self.wheel += f32::from(pixels);
                let steps = (self.wheel / WHEEL_STEP).trunc();
                self.wheel -= steps * WHEEL_STEP;
                steps as i32
            }
        };
        if steps == 0 {
            return;
        }
        let (_, y) = self.local(event.position);
        let row = self
            .lanes
            .row_at(y - layout::RULER + self.scroll_y)
            .or_else(|| self.lanes.rows.len().checked_sub(1));
        self.step_keeping(steps, row, cx);
    }
}
