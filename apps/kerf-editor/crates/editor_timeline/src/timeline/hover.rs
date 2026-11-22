//! The pointer resting over the lanes: the clip and the edge or rubber
//! band it is over ([`Hover`]) and the pointer that shows what a press
//! would do there, the tip of what it rests on ([`CanvasTip`]), and the
//! skim, which shows the frame under it without moving the playhead.

use super::drag::{Hit, Zone};
use super::ruler::BAND_ROW;
use super::tools::Tool;
use super::{TimelineEvent, TimelineView};
use editor_state::model;
use editor_ui::theme::layout;
use editor_ui::ui;
use gpui::{App, Context, CursorStyle, MouseMoveEvent, Window};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Hover {
    pub(super) clip: String,
    pub(super) zone: Zone,
    /// On the rubber band: its line, or a key.
    pub(super) band: bool,
}

/// What the pointer rests on, for its tip: where it is on the canvas.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CanvasTip {
    pub(super) key: String,
    pub(super) tip: ui::Tip,
    pub(super) bounds: (f32, f32, f32, f32),
}

impl TimelineView {
    pub(super) fn hover_at(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.gesture.is_some() {
            return;
        }
        // Nothing to point at while kerfd is not connected: no trim edges,
        // no tips, no skim.
        if self.offline(cx) {
            if self.hover.is_some() || self.tip.is_some() {
                self.hover = None;
                self.tip = None;
                cx.notify();
            }
            self.end_skim(cx);
            return;
        }
        let (x, y) = self.local(event.position);
        let lanes = self.lanes.clone();
        let hover = match self.hit(x, y) {
            Hit::Clip { row, index, zone } if !lanes.proposed => {
                let lane = &lanes.rows[row];
                let band = self.tool == Tool::Selection
                    && !lane.locked
                    && self.band_at(row, index, x, y).is_some();
                Some(Hover {
                    clip: lane.clips[index].id.clone(),
                    zone: if lane.locked { Zone::Body } else { zone },
                    band,
                })
            }
            _ => None,
        };
        let tip = self.tip_at(x, y, cx);
        if hover != self.hover || tip != self.tip {
            self.hover = hover;
            self.tip = tip;
            cx.notify();
        }
        self.skim_to(x, y, cx);
    }

    /// The pointer over the lanes skims: the picture shows the frame under
    /// it while nothing plays, and the playhead stays where it was.
    fn skim_to(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        let state = &self.session.read(cx).state;
        let inside = x >= 0.0 && y >= 0.0 && x < self.view.width && y < self.view.height;
        if state.playing || !inside || self.thumb_at(x, y).is_some() {
            self.end_skim(cx);
            return;
        }
        let fps = self.lanes.fps;
        let end = self.lanes.end.max(state.program_length());
        let frame = (self.view.time(x).clamp(0.0, end) * fps).round() / fps;
        if self
            .skim
            .is_some_and(|known| (known - frame).abs() < 0.5 / fps)
        {
            return;
        }
        self.skim = Some(frame);
        cx.emit(TimelineEvent::Skim(Some(frame)));
        cx.notify();
    }

    pub(super) fn end_skim(&mut self, cx: &mut Context<Self>) {
        if self.skim.take().is_some() {
            cx.emit(TimelineEvent::Skim(None));
            cx.notify();
        }
    }

    /// What the pointer rests on, for its tip: a clip (its name and its
    /// in and out), a sequence marker's flag, a proposal's band.
    fn tip_at(&self, x: f32, y: f32, cx: &App) -> Option<CanvasTip> {
        let lanes = &self.lanes;
        let view = &self.view;
        let fps = lanes.fps;
        if y < layout::RULER {
            for (index, mark) in lanes.markers.iter().enumerate() {
                let mx = view.x(mark.time);
                if x >= mx - 5.0 && x <= mx + 5.0 && (9.0..=layout::RULER - 2.0).contains(&y) {
                    let words = match &mark.label {
                        Some(name) => format!("Sequence marker · {name}"),
                        None => "Sequence marker".to_owned(),
                    };
                    return Some(CanvasTip {
                        key: format!("marker-{index}"),
                        tip: ui::Tip::new(words).key(model::timecode(mark.time, fps)),
                        bounds: (mx - 5.0, 9.0, 10.0, 12.0),
                    });
                }
            }
            for band in &lanes.bands {
                let (left, right) = (
                    view.x(band.start),
                    view.x(band.end).max(view.x(band.start) + 3.0),
                );
                if x >= left && x <= right && y <= BAND_ROW + 3.0 {
                    let words = if band.author.is_empty() {
                        "Proposal".to_owned()
                    } else {
                        format!("Proposal · {}", band.author)
                    };
                    return Some(CanvasTip {
                        key: format!("band-{}", band.id),
                        tip: ui::Tip::new(words),
                        bounds: (left, 0.0, right - left, BAND_ROW + 3.0),
                    });
                }
            }
            return None;
        }
        let Hit::Clip { row, index, .. } = self.hit(x, y) else {
            return None;
        };
        let clip = &lanes.rows[row].clips[index];
        let (x0, top, across, tall) = self.clip_rect(row, index);
        let _ = cx;
        Some(CanvasTip {
            key: format!("clip-{}", clip.id),
            tip: ui::Tip::new(clip.label.clone()).key(format!(
                "{}–{}",
                model::timecode(clip.start, fps),
                model::timecode(clip.end(), fps)
            )),
            bounds: (
                x0.max(0.0),
                top,
                (x0 + across).min(view.width) - x0.max(0.0),
                tall,
            ),
        })
    }

    /// The pointer over the lanes, while no drag holds it: what a press on
    /// the clip under it would do there.
    pub(super) fn hover_cursor(&self) -> Option<CursorStyle> {
        let lanes = &self.lanes;
        self.hover.as_ref().map(|hover| {
            let locked = lanes
                .find(&hover.clip)
                .is_some_and(|(row, _)| lanes.rows[row].locked);
            if hover.band {
                CursorStyle::ResizeUpDown
            } else if locked {
                CursorStyle::Arrow
            } else {
                self.tool.cursor(hover.zone != Zone::Body)
            }
        })
    }
}
