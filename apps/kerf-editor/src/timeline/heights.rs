//! How tall the tracks are: each track's own height or every track's
//! ([`TrackHeights`]), the ladder Option-scroll and Option-Up/Down step
//! them along, and what changes them: a head's line dragged
//! ([`ResizeDrag`]), Option-scroll over the heads, a head's menu, View ▸
//! Fit Tracks to Panel and Reset Track Heights.

use super::heads::ADD_ROW;
use super::{TimelineEvent, TimelineView};
use crate::model::TrackKind;
use crate::prefs::TrackHeights;
use crate::theme::layout;
use gpui::{px, Context, ScrollDelta, ScrollWheelEvent, TouchPhase, Window};
use std::rc::Rc;

/// The shortest and tallest a track is, in points.
pub const TRACK_LEAST: f32 = 20.0;
pub const TRACK_MOST: f32 = 160.0;
/// The heights Option-scroll and Option-Up/Down step every track through.
pub const TRACK_STEPS: [f32; 14] = [
    20.0, 24.0, 28.0, 32.0, 36.0, 42.0, 48.0, 56.0, 64.0, 72.0, 86.0, 104.0, 128.0, 160.0,
];
/// Pixels of a trackpad's scroll that make one step of Option-scroll.
const WHEEL_STEP: f32 = 40.0;

impl TrackHeights {
    /// The height of track `id` of `kind`.
    pub fn height(&self, id: &str, kind: TrackKind) -> f32 {
        self.tracks
            .get(id)
            .copied()
            .or(self.all)
            .unwrap_or_else(|| default_height(kind))
            .clamp(TRACK_LEAST, TRACK_MOST)
    }

    /// Track `id` to `height`, or back to the height of the rest (None).
    pub fn set(&mut self, id: &str, height: Option<f32>) {
        match height {
            Some(height) => {
                self.tracks
                    .insert(id.to_owned(), height.clamp(TRACK_LEAST, TRACK_MOST));
            }
            None => {
                self.tracks.remove(id);
            }
        }
    }

    /// Every track to `height`, none of its own; None gives each kind its own.
    pub fn set_all(&mut self, height: Option<f32>) {
        self.tracks.clear();
        self.all = height.map(|height| height.clamp(TRACK_LEAST, TRACK_MOST));
    }

    /// Every track of `rows` (id and kind) `steps` along [`TRACK_STEPS`],
    /// each from its own height. Tracks added later take the height a
    /// picture track without one of its own steps to; only the tracks that
    /// end up apart from it keep one of their own.
    pub fn step(&mut self, rows: &[(String, TrackKind)], steps: i32) {
        if steps == 0 || rows.is_empty() {
            return;
        }
        let all = stepped_height(
            self.all.unwrap_or_else(|| default_height(TrackKind::Video)),
            steps,
        );
        let tracks = rows
            .iter()
            .map(|(id, kind)| (id.clone(), stepped_height(self.height(id, *kind), steps)))
            .filter(|(_, height)| (height - all).abs() > 0.01)
            .collect();
        self.all = Some(all);
        self.tracks = tracks;
    }
}

/// A track's height when nothing else is set: one roomy height for every
/// kind ([`layout::TRACK_LANE`]), none larger than another for what it
/// is (Medium and Reset go back to it).
pub fn default_height(_kind: TrackKind) -> f32 {
    layout::TRACK_LANE
}

/// `height` moved `steps` along [`TRACK_STEPS`] (up when positive), from
/// the next height of the ladder on that side; held at its ends.
pub fn stepped_height(height: f32, steps: i32) -> f32 {
    let mut height = height.clamp(TRACK_LEAST, TRACK_MOST);
    for _ in 0..steps.max(0) {
        match TRACK_STEPS.iter().find(|step| **step > height + 0.01) {
            Some(next) => height = *next,
            None => break,
        }
    }
    for _ in 0..(-steps).max(0) {
        match TRACK_STEPS.iter().rev().find(|step| **step < height - 0.01) {
            Some(next) => height = *next,
            None => break,
        }
    }
    height
}

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
