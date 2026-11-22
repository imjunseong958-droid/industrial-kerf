//! How tall a track is ([`TrackHeights::height`]) and how the person
//! sets it: one track's height or every track's, and the ladder
//! Option-scroll and Option-Up/Down step them along ([`TRACK_STEPS`]). The
//! settings keep the heights; the timeline (`editor_timeline`) draws them
//! and changes them.

use crate::model::TrackKind;
use crate::prefs::TrackHeights;
use crate::tokens::layout;

/// The shortest and tallest a track is, in points.
pub const TRACK_LEAST: f32 = 20.0;
pub const TRACK_MOST: f32 = 160.0;
/// The heights Option-scroll and Option-Up/Down step every track through.
pub const TRACK_STEPS: [f32; 14] = [
    20.0, 24.0, 28.0, 32.0, 36.0, 42.0, 48.0, 56.0, 64.0, 72.0, 86.0, 104.0, 128.0, 160.0,
];

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
