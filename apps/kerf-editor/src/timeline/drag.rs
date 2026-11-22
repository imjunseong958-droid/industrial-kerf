//! What a press on the lanes finds ([`Hit`]) and the gestures it starts
//! ([`Gesture`]): a clip moved, slipped or slid by its body, or trimmed,
//! rippled or rolled by an edge ([`ClipDrag`]), snapping as it goes, and
//! the edit it makes; where the clip is drawn while it is dragged and once
//! dropped until kerfd answers ([`Ghost`], [`Pending`]); and the pointer a
//! drag holds.

use super::gain::BandDrag;
use super::heights::ResizeDrag;
use super::lanes::{Lanes, CLIP_INSET};
use super::signal::SignalDraw;
use super::tools::Tool;
use super::viewport::Viewport;
use super::TimelineView;
use crate::theme::layout;
use gpui::CursorStyle;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Points inside a clip's edge that take a trim.
const GRAB: f32 = 6.0;
/// Points within which a drag snaps to an edge, a marker or the playhead.
const SNAP: f32 = 6.0;
/// Points a press moves before it drags.
const DRAG_START: f32 = 3.0;
/// How long a dropped clip waits where it was dropped for kerfd's answer.
pub(super) const PENDING: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Zone {
    Body,
    In,
    Out,
}

pub(super) enum Hit {
    Ruler,
    Clip {
        row: usize,
        index: usize,
        zone: Zone,
    },
    Lane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edit {
    Move,
    TrimIn,
    TrimOut,
}

pub(super) enum Gesture {
    Scrub,
    Clip(ClipDrag),
    Band(BandDrag),
    Signal(SignalDraw),
    Resize(ResizeDrag),
    /// The zoom slider's knob, dragged.
    Slider,
    /// The scroll bar's thumb, held this far from its left.
    Thumb(f32),
}

/// A clip being dragged: moved, slipped or slid by its body, or trimmed,
/// rippled or rolled by an edge.
pub(super) struct ClipDrag {
    pub(super) id: String,
    pub(super) edit: Edit,
    pub(super) tool: Tool,
    /// The row it is on, and the row it would land on.
    pub(super) row: usize,
    pub(super) target: usize,
    pub(super) press_time: f64,
    /// Where it was pressed: down the lanes, and down the window. It
    /// follows the pointer by how far it has moved in the window, so a
    /// layout that shifts while it drags (the Inspector coming in) does
    /// not move it.
    pub(super) press_y: f32,
    pub(super) press_window: f32,
    pub(super) moved: bool,
    pub(super) start_frame: i64,
    pub(super) duration_frames: i64,
    /// The amount, in frames: of the move, or of the edge.
    pub(super) frames: i64,
    pub(super) least: i64,
    pub(super) most: i64,
    /// What it snaps to, in seconds, and what it snapped to last.
    pub(super) targets: Vec<f64>,
    pub(super) snapped: Option<f64>,
}

impl ClipDrag {
    // A drag starts from what its press found: the clip, where and when, the tool, the playhead.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        lanes: &Lanes,
        row: usize,
        index: usize,
        zone: Zone,
        tool: Tool,
        press_time: f64,
        press_y: f32,
        playhead: f64,
    ) -> Self {
        let clip = &lanes.rows[row].clips[index];
        let edit = match zone {
            Zone::Body => Edit::Move,
            Zone::In => Edit::TrimIn,
            Zone::Out => Edit::TrimOut,
        };
        // What it can snap to, gathered once rather than on every move.
        let mut targets = vec![0.0, playhead];
        for other in lanes.rows.iter().flat_map(|row| &row.clips) {
            if other.id != clip.id {
                targets.push(other.start);
                targets.push(other.end());
            }
        }
        for mark in &lanes.markers {
            targets.push(mark.time);
            if mark.duration > 0.0 {
                targets.push(mark.time + mark.duration);
            }
        }
        // Never before the sequence, never to no frames, never past its media.
        let (least, most) = match edit {
            // A slip shows other media in the same span: as far as its
            // media reaches either way (later media is a drag to the left).
            Edit::Move if tool == Tool::Slip => (
                -clip.tail.unwrap_or(i64::MAX),
                clip.head.unwrap_or(i64::MAX),
            ),
            Edit::Move => (-clip.start_frame, i64::MAX),
            Edit::TrimIn => (
                -clip.head.unwrap_or(i64::MAX).min(clip.start_frame),
                clip.duration_frames - 1,
            ),
            Edit::TrimOut => (-(clip.duration_frames - 1), clip.tail.unwrap_or(i64::MAX)),
        };
        Self {
            id: clip.id.clone(),
            edit,
            tool,
            row,
            target: row,
            press_time,
            press_y,
            press_window: 0.0,
            moved: false,
            start_frame: clip.start_frame,
            duration_frames: clip.duration_frames,
            frames: 0,
            least,
            most: most.max(least),
            targets,
            snapped: None,
        }
    }

    /// Follows the pointer at `x` across the canvas, `dy` points down the
    /// window from where it was pressed.
    pub(super) fn follow(&mut self, lanes: &Lanes, view: &Viewport, x: f32, dy: f32, snap: bool) {
        let fps = lanes.fps;
        let time = view.time(x);
        let delta = time - self.press_time;
        if (delta * view.pps as f64).abs() >= DRAG_START as f64 || dy.abs() >= DRAG_START {
            self.moved = true;
        }
        if !self.moved {
            return;
        }
        let start = self.start_frame as f64 / fps;
        let end = (self.start_frame + self.duration_frames) as f64 / fps;
        let threshold = (SNAP / view.pps.max(f32::EPSILON)) as f64;
        // A slip leaves the clip where it is: nothing to snap.
        let snap = snap && self.tool != Tool::Slip;
        self.snapped = None;
        let frames = match self.edit {
            Edit::Move => {
                let mut at = start + delta;
                if snap {
                    if let Some((target, shift)) =
                        nearest(&self.targets, &[at, at + (end - start)], threshold)
                    {
                        at += shift;
                        self.snapped = Some(target);
                    }
                }
                (at * fps).round() as i64 - self.start_frame
            }
            Edit::TrimIn | Edit::TrimOut => {
                let edge = if self.edit == Edit::TrimIn {
                    start
                } else {
                    end
                };
                let mut at = edge + delta;
                if snap {
                    if let Some((target, shift)) = nearest(&self.targets, &[at], threshold) {
                        at += shift;
                        self.snapped = Some(target);
                    }
                }
                (at * fps).round() as i64 - (edge * fps).round() as i64
            }
        };
        self.frames = frames.clamp(self.least, self.most);
        if self.edit == Edit::Move && self.tool == Tool::Selection {
            // Onto another lane of its kind, unless that one is locked.
            let kind = lanes.rows[self.row].kind;
            self.target = lanes
                .row_at(self.press_y + dy)
                .filter(|&row| lanes.rows[row].kind == kind && !lanes.rows[row].locked)
                .unwrap_or(self.row);
        }
    }

    /// Where it is drawn now: start and length, in seconds. A slip leaves
    /// it in place.
    pub(super) fn span(&self, fps: f64) -> (f64, f64) {
        let (start, length) = match self.edit {
            Edit::Move if self.tool == Tool::Slip => (self.start_frame, self.duration_frames),
            Edit::Move => (self.start_frame + self.frames, self.duration_frames),
            Edit::TrimIn => (
                self.start_frame + self.frames,
                self.duration_frames - self.frames,
            ),
            Edit::TrimOut => (self.start_frame, self.duration_frames + self.frames),
        };
        (start as f64 / fps, length.max(1) as f64 / fps)
    }

    /// The time of the edge it moves, and how far it has moved, in seconds:
    /// what the readout by it says.
    pub(super) fn readout(&self, fps: f64) -> (f64, f64) {
        let delta = self.frames as f64 / fps;
        let edge = match self.edit {
            Edit::Move | Edit::TrimIn => self.start_frame as f64 / fps + delta,
            Edit::TrimOut => (self.start_frame + self.duration_frames) as f64 / fps + delta,
        };
        (edge, delta)
    }

    /// The edit it makes, as kerfd takes it; None when it moved nothing.
    /// A roll's leading edge is the cut after the clip before it, found
    /// in `lanes`; with none there it says why.
    pub(super) fn op(&self, lanes: &Lanes) -> Result<Option<Value>, &'static str> {
        let other_track = self.edit == Edit::Move && self.target != self.row;
        if !self.moved || (self.frames == 0 && !other_track) {
            return Ok(None);
        }
        let id = &self.id;
        let frames = self.frames;
        let edge = if self.edit == Edit::TrimIn {
            "in"
        } else {
            "out"
        };
        Ok(Some(match (self.tool, self.edit) {
            (Tool::Slip, Edit::Move) => json!({"op": "slip", "clip": id, "frames": -frames}),
            (Tool::Slide, Edit::Move) => json!({"op": "slide", "clip": id, "frames": frames}),
            (_, Edit::Move) => {
                let mut op = json!({"op": "move", "clip": id, "frames": frames});
                if other_track {
                    op["track"] = json!(lanes.rows[self.target].id);
                }
                op
            }
            (Tool::Roll, Edit::TrimOut) => json!({"op": "roll", "clip": id, "frames": frames}),
            (Tool::Roll, Edit::TrimIn) => {
                let before = lanes.rows[self.row]
                    .clips
                    .iter()
                    .find(|clip| clip.start_frame + clip.duration_frames == self.start_frame)
                    .ok_or("A roll needs a clip on the other side of the cut")?;
                json!({"op": "roll", "clip": before.id, "frames": frames})
            }
            // A positive amount moves the edge later: the in point shortens, the out point lengthens.
            (tool, _) => json!({
                "op": "trim",
                "clip": id,
                "edge": edge,
                "frames": frames,
                "ripple": tool == Tool::Ripple,
            }),
        }))
    }
}

/// The target nearest one of `points` within `threshold`, and how far to
/// shift to meet it.
fn nearest(targets: &[f64], points: &[f64], threshold: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for &point in points {
        for &target in targets {
            let shift = target - point;
            if shift.abs() <= threshold && best.is_none_or(|(_, known)| shift.abs() < known.abs()) {
                best = Some((target, shift));
            }
        }
    }
    best
}

/// A clip drawn somewhere other than where the snapshot has it.
#[derive(Clone, Debug)]
pub(super) struct Ghost {
    pub(super) id: String,
    pub(super) row: usize,
    pub(super) start: f64,
    pub(super) duration: f64,
    /// The edge being trimmed.
    pub(super) edge: Option<Zone>,
    /// Still being dragged: where it was shows as an outline.
    pub(super) dragging: bool,
    /// Its media's head moves with a leading trim: seconds of it.
    pub(super) head_shift: f64,
}

/// A dropped clip, until kerfd answers.
pub(super) struct Pending {
    pub(super) ghost: Ghost,
    pub(super) revision: u64,
    pub(super) at: Instant,
}

impl TimelineView {
    pub(super) fn hit(&self, x: f32, y: f32) -> Hit {
        if y < layout::RULER {
            return Hit::Ruler;
        }
        let view = &self.view;
        let lane_y = y - layout::RULER + view.scroll_y;
        let Some(row) = self.lanes.row_at(lane_y) else {
            return Hit::Lane;
        };
        let lane = &self.lanes.rows[row];
        // A previewed proposal's clips are its own, not the program's.
        if lane.previewed
            || lane_y < lane.top + CLIP_INSET
            || lane_y >= lane.top + lane.height - CLIP_INSET
        {
            return Hit::Lane;
        }
        // The last drawn is on top.
        for (index, clip) in lane.clips.iter().enumerate().rev() {
            let x0 = view.x(clip.start);
            let x1 = view.x(clip.end()).max(x0 + 2.0);
            if x < x0 || x >= x1 {
                continue;
            }
            let grab = GRAB.min((x1 - x0) / 3.0);
            let zone = if x - x0 < grab {
                Zone::In
            } else if x1 - x <= grab {
                Zone::Out
            } else {
                Zone::Body
            };
            return Hit::Clip { row, index, zone };
        }
        Hit::Lane
    }

    /// The clip `index` of `row`'s box on the canvas: left, top, width, height.
    pub(super) fn clip_rect(&self, row: usize, index: usize) -> (f32, f32, f32, f32) {
        let lane = &self.lanes.rows[row];
        let clip = &lane.clips[index];
        let x0 = self.view.x(clip.start);
        let x1 = self.view.x(clip.end()).max(x0 + 2.0);
        let top = self.view.row_y(lane) + CLIP_INSET;
        (x0, top, x1 - x0, lane.height - 2.0 * CLIP_INSET)
    }

    /// Clip `id`'s box on the canvas, as [`Self::clip_rect`], where the
    /// lanes last drew it.
    pub(super) fn clip_rect_of(&self, id: &str) -> Option<(f32, f32, f32, f32)> {
        let (row, index) = self.lanes.find(id)?;
        Some(self.clip_rect(row, index))
    }

    /// The clip drawn other than where the snapshot has it: the one being
    /// dragged, else one dropped and not yet answered for.
    pub(super) fn ghost(&self) -> Option<Ghost> {
        let fps = self.lanes.fps;
        match &self.gesture {
            Some(Gesture::Clip(drag)) if drag.moved => {
                let (start, duration) = drag.span(fps);
                Some(Ghost {
                    id: drag.id.clone(),
                    row: drag.target,
                    start,
                    duration,
                    edge: match drag.edit {
                        Edit::Move => None,
                        Edit::TrimIn => Some(Zone::In),
                        Edit::TrimOut => Some(Zone::Out),
                    },
                    dragging: true,
                    head_shift: if drag.edit == Edit::TrimIn {
                        drag.frames as f64 / fps
                    } else if drag.tool == Tool::Slip {
                        -(drag.frames as f64) / fps
                    } else {
                        0.0
                    },
                })
            }
            _ => self
                .pending
                .as_ref()
                .filter(|pending| !pending.ghost.id.is_empty())
                .map(|pending| pending.ghost.clone()),
        }
    }

    /// What a clip being dragged snapped to, and its readout: the edge's
    /// time, how far it moved, its row.
    pub(super) fn drag_readout(&self) -> (Option<f64>, Option<(f64, f64, usize)>) {
        match &self.gesture {
            Some(Gesture::Clip(drag)) if drag.moved => {
                let (edge, delta) = drag.readout(self.lanes.fps);
                (drag.snapped, Some((edge, delta, drag.target)))
            }
            _ => (None, None),
        }
    }

    /// The window's cursor while a drag holds it.
    pub(super) fn drag_cursor(&self) -> Option<CursorStyle> {
        match &self.gesture {
            Some(Gesture::Clip(drag)) => Some(match (drag.edit, drag.tool) {
                (Edit::Move, Tool::Selection) => CursorStyle::ClosedHand,
                (Edit::Move, _) => CursorStyle::ResizeLeftRight,
                (_, Tool::Roll) => CursorStyle::ResizeColumn,
                _ => CursorStyle::ResizeLeftRight,
            }),
            Some(Gesture::Band(_)) | Some(Gesture::Resize(_)) => Some(CursorStyle::ResizeUpDown),
            Some(Gesture::Signal(_)) => Some(CursorStyle::Crosshair),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProposalBand, Timeline, TimelineShow};
    use crate::prefs::TrackHeights;
    use crate::timeline::lanes::Build;
    use std::collections::HashSet;

    fn timeline(value: serde_json::Value) -> Timeline {
        Timeline::from_value(&value)
    }

    fn lanes_of(
        current: &Timeline,
        first_cut: Option<&Timeline>,
        bands: Vec<ProposalBand>,
    ) -> Lanes {
        Lanes::build(Build {
            current,
            first_cut,
            bands,
            preview: None,
            heights: &TrackHeights::default(),
            qc: &HashSet::new(),
        })
    }

    #[test]
    fn a_trim_cannot_reach_past_the_media_or_to_no_frames() {
        let current = timeline(json!({
            "fps": 24.0,
            "sources": [{"id": "m", "path": "m.mp4", "duration": 10.0}],
            "tracks": [{"id": "V1", "type": "video", "enabled": true, "clips": [
                {"id": "a", "timelineStart": 2.0, "duration": 4.0, "startFrame": 48, "durationFrames": 96,
                 "enabled": true, "sourceId": "m", "in": 1.0, "out": 5.0}
            ]}]
        }));
        let lanes = lanes_of(&current, None, Vec::new());
        let clip = &lanes.rows[0].clips[0];
        assert_eq!((clip.head, clip.tail), (Some(24), Some(120)));
        let drag = |zone, tool, at| ClipDrag::new(&lanes, 0, 0, zone, tool, at, 30.0, 0.0);
        let trim_in = drag(Zone::In, Tool::Selection, 2.0);
        assert_eq!((trim_in.least, trim_in.most), (-24, 95));
        let trim_out = drag(Zone::Out, Tool::Selection, 6.0);
        assert_eq!((trim_out.least, trim_out.most), (-95, 120));
        let moving = drag(Zone::Body, Tool::Selection, 3.0);
        assert_eq!(moving.least, -48);
        // A slip reaches as far as the media goes either way.
        let slip = drag(Zone::Body, Tool::Slip, 3.0);
        assert_eq!((slip.least, slip.most), (-120, 24));
    }
}
