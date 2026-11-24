//! A press on the lanes and what the tool and the keys held make of it:
//! choosing, splitting, a drag of a clip, of its rubber band or of its
//! signal, a scrub; the drag following the pointer anywhere in the window;
//! its release sending the edit, drawn where it was dropped until kerfd
//! answers; and a right-click's menu.

use super::drag::{ClipDrag, Edit, Gesture, Ghost, Hit, Pending, Zone, PENDING};
use super::gain::{add_gain_key, gain_at, BandDrag};
use super::lanes::ClipBox;
use super::signal::SignalDraw;
use super::tools::Tool;
use super::{TimelineEvent, TimelineView};
use editor_session::commands::RunCommand;
use editor_state::model::ClipShow;
use editor_ui::theme::layout;
use gpui::{App, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Window};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Instant;

impl TimelineView {
    pub(super) fn press(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The skim ends here, so the picture goes back to the playhead's frame.
        self.end_skim(cx);
        self.tip = None;
        // kerfd connecting or gone: the lanes take nothing, since an edit
        // made then would never reach kerfd.
        if self.offline(cx) {
            self.gesture = None;
            return;
        }
        window.focus(&self.focus_handle);
        if self.gesture.is_some() {
            // Its button came up where it was not seen.
            self.release(event.position, cx);
        }
        let (x, y) = self.local(event.position);
        if let Some(grab) = self.thumb_at(x, y) {
            self.gesture = Some(Gesture::Thumb(grab));
            cx.notify();
            return;
        }
        let lanes = self.lanes.clone();
        let time = self.view.time(x).max(0.0);
        let modifiers = event.modifiers;
        let extending = modifiers.shift || modifiers.secondary();
        let hit = match self.hit(x, y) {
            // The first cut is the proposal's: a press there only seeks.
            Hit::Clip { .. } if lanes.proposed => Hit::Lane,
            hit => hit,
        };
        match hit {
            Hit::Clip { row, index, zone } => {
                self.press_clip(event, row, index, zone, x, y, time, window, cx)
            }
            hit => {
                if !lanes.proposed && matches!(hit, Hit::Lane) && !extending {
                    self.session.update(cx, |session, cx| {
                        if !session.selection.is_empty() {
                            session.selection.clear();
                            cx.notify();
                        }
                    });
                }
                self.gesture = Some(Gesture::Scrub);
                self.seek(time, true, cx);
            }
        }
        self.hover = None;
        cx.notify();
    }

    /// A press on clip `index` of `row`: what the tool and the keys held make of it.
    // Each argument is a distinct part of the press: its event, which clip, where and when.
    #[allow(clippy::too_many_arguments)]
    fn press_clip(
        &mut self,
        event: &MouseDownEvent,
        row: usize,
        index: usize,
        zone: Zone,
        x: f32,
        y: f32,
        time: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lanes = self.lanes.clone();
        let lane = &lanes.rows[row];
        let clip = &lane.clips[index];
        let id = clip.id.clone();
        let modifiers = event.modifiers;
        if modifiers.shift || modifiers.secondary() {
            // ⌘/Ctrl-click toggles a clip in the selection, ⇧-click adds it;
            // neither drags. The one clicked last leads.
            self.extend_selection(&id, modifiers.secondary(), cx);
            return;
        }
        if event.click_count >= 2 {
            self.choose(&id, cx);
            if modifiers.alt {
                window.dispatch_action(Box::new(RunCommand::new("ai.ask")), cx);
            } else if let Some(media) = self.made_media(clip, cx) {
                window.dispatch_action(
                    Box::new(editor_making::making::OpenShotGraph {
                        media: media.into(),
                    }),
                    cx,
                );
            }
            return;
        }
        if self.tool == Tool::Razor {
            self.choose(&id, cx);
            // Split where it was pressed, never at either end.
            let at = (time * lanes.fps).round() as i64;
            let at = at.clamp(
                clip.start_frame + 1,
                clip.start_frame + clip.duration_frames - 1,
            );
            if !lane.locked && clip.duration_frames > 1 {
                self.session.update(cx, |session, cx| {
                    session.edit(vec![json!({"op": "split", "clip": id, "at": at})]);
                    cx.notify();
                });
            }
            return;
        }
        self.choose(&id, cx);
        if lane.locked {
            // A locked track's clips are chosen, never dragged.
            return;
        }
        let (x0, top, across, tall) = self.clip_rect(row, index);
        // Option-drag across a signal clip draws its number.
        if self.tool == Tool::Selection && modifiers.alt && !clip.clip.signal.is_empty() {
            let mut draw = SignalDraw {
                clip: id,
                frames: clip.duration_frames.max(1),
                drawn: BTreeMap::new(),
                last: None,
            };
            draw.draw_to(x, y, (x0, top, across, tall));
            self.gesture = Some(Gesture::Signal(draw));
            return;
        }
        // A sound clip's rubber band: its keys and its line take the press.
        if self.tool == Tool::Selection
            && let Some(key) = self.band_at(row, index, x, y)
        {
            let (mut keys, mut keyed) = clip.clip.gain_keys();
            let mut key = key;
            if modifiers.alt && key.is_none() {
                // Option-click on the line adds a key there, at the level it reads.
                let offset = ((x - x0) / self.view.pps.max(f32::EPSILON)) as f64;
                let frame = (offset.clamp(0.0, clip.duration) * lanes.fps).round() as i64;
                let level = gain_at(y, top, tall);
                key = Some(add_gain_key(&mut keys, keyed, frame, lanes.fps, level));
                keyed = true;
            }
            let from: Vec<f64> = keys.iter().map(|key| key.value).collect();
            self.gesture = Some(Gesture::Band(BandDrag {
                clip: id,
                keyed,
                key: if keyed { key } else { None },
                press_y: f32::from(event.position.y),
                top,
                tall,
                values: from.clone(),
                from,
                moved: modifiers.alt,
                keys,
            }));
            return;
        }
        let can = if zone == Zone::Body {
            self.tool.drags_bodies()
        } else {
            self.tool.trims_edges()
        };
        if can {
            let playhead = self.last_position;
            let lane_y = y - layout::RULER + self.view.scroll_y;
            let mut drag =
                ClipDrag::new(&lanes, row, index, zone, self.tool, time, lane_y, playhead);
            drag.press_window = f32::from(event.position.y);
            self.gesture = Some(Gesture::Clip(drag));
        }
    }

    /// The media a clip plays when it is a shot made from steps: a
    /// double-click opens its graph.
    fn made_media(&self, clip: &ClipBox, cx: &App) -> Option<String> {
        let media = clip.source.as_deref()?;
        self.session
            .read(cx)
            .timeline()
            .source(media)
            .filter(|source| source.is_made())
            .map(|source| source.id.clone())
    }

    /// Clip `id` alone is chosen.
    fn choose(&self, id: &str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            if session.selection != [id] {
                session.selection = vec![id.to_owned()];
                cx.notify();
            }
        });
    }

    /// ⇧ or ⌘ on clip `id`: it joins the chosen and leads them (what one
    /// clip's commands act on); ⇧ on a chosen one makes it lead; ⌘
    /// (`toggle`) on a chosen one lets it go.
    fn extend_selection(&self, id: &str, toggle: bool, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            extend(&mut session.selection, id, toggle);
            cx.notify();
        });
    }

    fn seek(&mut self, time: f64, scrub: bool, cx: &mut Context<Self>) {
        self.last_position = time;
        self.session.update(cx, |session, cx| {
            session.seek(time, scrub);
            cx.notify();
        });
    }

    /// The pointer moved anywhere in the window: a gesture follows it.
    pub(super) fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.gesture.is_none() {
            let inside = self.bounds.get().contains(&event.position);
            if !inside && (self.hover.is_some() || self.skim.is_some() || self.tip.is_some()) {
                self.hover = None;
                self.tip = None;
                self.end_skim(cx);
                cx.notify();
            }
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.release(event.position, cx);
            return;
        }
        let (x, y) = self.local(event.position);
        // A signal's clip where it is drawn now, the timeline perhaps moved.
        let signal_box = match &self.gesture {
            Some(Gesture::Signal(draw)) => self.clip_rect_of(&draw.clip),
            _ => None,
        };
        match &mut self.gesture {
            Some(Gesture::Band(band)) => {
                band.follow(f32::from(event.position.y));
                cx.notify();
                return;
            }
            Some(Gesture::Signal(draw)) => {
                if let Some(rect) = signal_box {
                    draw.draw_to(x, y, rect);
                }
                cx.notify();
                return;
            }
            Some(Gesture::Resize(resize)) => {
                let height = resize.from + f32::from(event.position.y) - resize.anchor;
                let (track, kind, every) = (resize.track.clone(), resize.kind, resize.every);
                self.resize_to(&track, kind, every, height, cx);
                return;
            }
            Some(Gesture::Slider) => {
                self.slide_zoom(event.position, cx);
                return;
            }
            Some(Gesture::Thumb(grab)) => {
                let grab = *grab;
                self.drag_thumb(x - grab, cx);
                return;
            }
            _ => {}
        }
        self.edge_pan(x);
        let time = self.view.time(x).max(0.0);
        let lanes = self.lanes.clone();
        let view = self.view;
        let snap = self.snap;
        match &mut self.gesture {
            Some(Gesture::Scrub) => self.seek(time, true, cx),
            Some(Gesture::Clip(drag)) => {
                let dy = f32::from(event.position.y) - drag.press_window;
                drag.follow(&lanes, &view, x, dy, snap)
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn release(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        match gesture {
            Gesture::Scrub => {
                let (x, _) = self.local(position);
                let time = self.view.time(x).max(0.0);
                self.seek(time, false, cx);
            }
            Gesture::Clip(drag) => self.drop_clip(drag, cx),
            Gesture::Band(band) => self.drop_band(band, cx),
            Gesture::Signal(draw) => self.drop_signal(draw, cx),
            Gesture::Resize(_) => cx.emit(TimelineEvent::Heights(self.heights.clone())),
            Gesture::Slider | Gesture::Thumb(_) => {}
        }
        cx.notify();
    }

    /// Sends what a drag did as one edit.
    fn drop_clip(&mut self, drag: ClipDrag, cx: &mut Context<Self>) {
        let lanes = self.lanes.clone();
        let op = match drag.op(&lanes) {
            Ok(Some(op)) => op,
            Ok(None) => return,
            Err(why) => {
                self.session.update(cx, |session, cx| {
                    session.state.notify(why, true);
                    cx.notify();
                });
                return;
            }
        };
        let (start, duration) = drag.span(lanes.fps);
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.edit(vec![op]);
            cx.notify();
            sent
        });
        if sent {
            let head_shift = if drag.edit == Edit::TrimIn {
                drag.frames as f64 / lanes.fps
            } else {
                0.0
            };
            self.pending = Some(Pending {
                ghost: Ghost {
                    id: drag.id,
                    row: drag.target,
                    start,
                    duration,
                    edge: None,
                    dragging: false,
                    head_shift,
                },
                revision,
                at: Instant::now(),
            });
        }
    }

    /// An edit with no clip to hold in place went: a refusal still puts
    /// back what it showed.
    pub(super) fn pending_marker(&mut self, revision: u64) {
        self.pending = Some(Pending {
            ghost: Ghost {
                id: String::new(),
                row: usize::MAX,
                start: 0.0,
                duration: 0.0,
                edge: None,
                dragging: false,
                head_shift: 0.0,
            },
            revision,
            at: Instant::now(),
        });
    }

    /// An edit kerfd has not answered goes back once it has waited longer
    /// than [`PENDING`], as the timeline draws (drawn again then): put away
    /// meanwhile, it is still held, and a refusal that comes then still
    /// puts back what it drew.
    pub(super) fn let_pending_go(&mut self, cx: &Context<Self>) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.at.elapsed() > PENDING)
        {
            self.pending = None;
        }
        if let Some(pending) = &self.pending {
            self.pending_ends.at(pending.at + PENDING, cx);
        }
    }

    /// A right-click on a clip chooses it and opens its menu.
    pub(super) fn context_press(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_skim(cx);
        self.tip = None;
        if self.offline(cx) {
            return;
        }
        window.focus(&self.focus_handle);
        let (x, y) = self.local(event.position);
        if self.lanes.proposed {
            return;
        }
        let Hit::Clip { row, index, .. } = self.hit(x, y) else {
            return;
        };
        let id = self.lanes.rows[row].clips[index].id.clone();
        self.session.update(cx, |session, cx| {
            if session.selection.first() != Some(&id) {
                if session.selection.contains(&id) {
                    extend(&mut session.selection, &id, false);
                } else {
                    session.selection = vec![id.clone()];
                }
                cx.notify();
            }
        });
        self.clip_menu(row, index, event.position, window, cx);
        // The menu has the keys: the timeline does not take them back.
        window.prevent_default();
    }
}

/// `id` joins `selection` and leads it (what one clip's commands act on);
/// ⇧ on one already chosen makes it lead; `toggle` (⌘) on one already
/// chosen lets it go.
pub fn extend(selection: &mut Vec<String>, id: &str, toggle: bool) {
    match selection.iter().position(|chosen| chosen == id) {
        Some(at) if toggle => {
            selection.remove(at);
        }
        Some(at) => {
            let chosen = selection.remove(at);
            selection.insert(0, chosen);
        }
        None => selection.insert(0, id.to_owned()),
    }
}
