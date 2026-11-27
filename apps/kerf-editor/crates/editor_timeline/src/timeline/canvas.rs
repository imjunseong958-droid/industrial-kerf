//! The lanes' canvas: what one frame of it draws ([`Frame`], taken from
//! the view as it stands), the area it fills with what a press, a
//! right-click and the pointer moving over it do, and its painting: the
//! lanes under the clips, each lane's clips and transitions, a clip being
//! dragged and where it was with what it takes along, the line from each
//! clip that follows another to that one, the snap line and a drag's
//! readout, then the ruler, Kerf's cursor, the playhead, the skimmer and the
//! scroll bar over them. The pieces every painting family shares are here
//! too ([`Pen`], [`Fonts`], [`shape`], [`masked`]).

use super::activity::{activities, Activity};
use super::clip::paint_clip;
use super::drag::Ghost;
use super::gain::GainShown;
use super::hover::Hover;
use super::lanes::{ClipBox, Lanes, Row, CLIP_INSET};
use super::ruler::{paint_kerf, paint_playhead, paint_ruler};
use super::tools::Tool;
use super::viewport::{thumb, Viewport, SCROLLER};
use super::watch::{watch_look, WatchLook};
use super::TimelineView;
use editor_session::session::Session;
use editor_state::model;
use editor_ui::motion;
use editor_ui::theme::{self, layout, radius, text, TypeStyle};
use editor_ui::ui;
use gpui::{
    canvas, div, fill, pattern_slash, point, prelude::*, px, quad, size, App, BorderStyle, Bounds,
    ContentMask, Context, CursorStyle, DispatchPhase, ElementId, Entity, Font, FontWeight,
    MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, ShapedLine, SharedString,
    TextRun, WeakEntity, Window,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

/// What one frame of the canvas draws.
pub(super) struct Frame {
    pub(super) lanes: Rc<Lanes>,
    pub(super) view: Viewport,
    pub(super) playhead: f64,
    /// Where the pointer skims the picture or the lanes, while it does.
    pub(super) skimmer: Option<f64>,
    /// The program's in and out marks.
    pub(super) marks: (Option<f64>, Option<f64>),
    pub(super) selection: Vec<String>,
    pub(super) hover: Option<Hover>,
    pub(super) ghost: Option<Ghost>,
    pub(super) snapped: Option<f64>,
    /// While Kerf works: where its cursor stands, and the clips it looks at.
    pub(super) kerf: Option<(Option<f64>, Vec<String>)>,
    /// What the media on the lanes are doing, where it is more than ready.
    pub(super) activities: Vec<Activity>,
    /// The window's cursor while a drag holds it.
    pub(super) cursor: Option<CursorStyle>,
    pub(super) tool: Tool,
    /// Zoomed in: a bar under the lanes shows where the view is.
    pub(super) zoomed: bool,
    /// A drag's readout: the edge's time, how far it moved, its row.
    pub(super) readout: Option<(f64, f64, usize)>,
    /// A rubber band as dragged, or as just let go: its keys and values.
    pub(super) gain: Option<GainShown>,
    /// A signal as it is drawn, or as it was drawn until kerfd has it.
    pub(super) signal: Option<(String, BTreeMap<i64, f64>)>,
    /// While a change reel plays (and as it settles back): the clips it
    /// watches, and how far into watching, 0..1.
    pub(super) watch: Option<(Rc<HashSet<String>>, f32)>,
    pub(super) session: Entity<Session>,
    pub(super) this: WeakEntity<TimelineView>,
}

impl Frame {
    /// How `clip` on `row` is drawn while a change reel watches.
    fn watch_look(&self, row: &Row, clip: &ClipBox) -> WatchLook {
        self.watch
            .as_ref()
            .map(|(clips, amount)| watch_look(*amount, clips.contains(&clip.id), row.audio()))
            .unwrap_or_default()
    }

    pub(super) fn activity(&self, clip: &ClipBox) -> Option<&Activity> {
        if clip.card() {
            return None;
        }
        let media = clip.source.as_ref()?;
        self.activities
            .iter()
            .find(|activity| activity.media == *media)
    }
}

/// Points on the canvas, as the window places them.
#[derive(Clone, Copy)]
pub(super) struct Pen {
    x: f32,
    y: f32,
}

impl Pen {
    pub(super) fn rect(&self, x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.x + x), px(self.y + y)),
            size(px(width.max(0.0)), px(height.max(0.0))),
        )
    }

    pub(super) fn at(&self, x: f32, y: f32) -> Point<Pixels> {
        point(px(self.x + x), px(self.y + y))
    }
}

pub(super) struct Fonts {
    pub(super) ui: Font,
    pub(super) strong: Font,
    pub(super) mono: Font,
}

impl Fonts {
    fn new() -> Self {
        Self {
            ui: theme::ui_font(FontWeight::NORMAL),
            strong: theme::ui_font(FontWeight::MEDIUM),
            mono: theme::mono_font(FontWeight::NORMAL),
        }
    }
}

pub(super) fn shape(
    window: &Window,
    words: SharedString,
    style: TypeStyle,
    font: &Font,
    color: Rgba,
) -> ShapedLine {
    let run = TextRun {
        len: words.len(),
        font: font.clone(),
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(words, px(style.px), &[run], None)
}

pub(super) fn masked<R>(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    draw: impl FnOnce(&mut Window) -> R,
) -> R {
    window.with_content_mask(Some(ContentMask { bounds }), draw)
}

impl TimelineView {
    /// What the canvas draws this frame, from the view as it stands: where
    /// the playhead is and the pointer skims (`skim`), what is chosen and
    /// pointed at, a drag and its readout, what Kerf looks at, what the
    /// media are doing, a band or a signal drawn, a change reel. Frames are
    /// asked for while stripes move or a reel's clips rise or settle.
    fn frame(&self, skim: Option<f64>, window: &Window, cx: &Context<Self>) -> Frame {
        let lanes = self.lanes.clone();
        let session = self.session.read(cx);
        let state = &session.state;
        let (mark_in, mark_out, _) = session.marks();
        let selection = if lanes.proposed {
            Vec::new()
        } else {
            session.selection.clone()
        };
        let kerf = state.work.busy().then(|| {
            (
                self.kerf_at.or_else(|| state.agent_focus.first().copied()),
                state.agent_clips.clone(),
            )
        });
        let activities = activities(&lanes, state);
        let (snapped, readout) = self.drag_readout();
        // The stripes move only while a clip shows them.
        if activities.iter().any(Activity::making) && !motion::reduced() {
            window.request_animation_frame();
        }
        // While a change reel plays, what it watches rises off the lanes.
        let now = Instant::now();
        if self.watch.running_at(now) {
            window.request_animation_frame();
        }
        Frame {
            lanes,
            view: self.view,
            playhead: state.position,
            skimmer: skim,
            marks: (mark_in, mark_out),
            selection,
            hover: self.hover.clone(),
            ghost: self.ghost(),
            snapped,
            kerf,
            activities,
            cursor: self.drag_cursor(),
            tool: self.tool,
            zoomed: self.zoom.is_some(),
            readout,
            gain: self.gain_over(),
            signal: self.signal_shown(),
            watch: self.watch.shown_at(now),
            session: self.session.clone(),
            this: cx.entity().downgrade(),
        }
    }

    /// The lanes: the canvas drawing this frame (the playhead where the
    /// pointer skims, `skim`), the words of an empty sequence, and the tip
    /// of what the pointer rests on; a press, a right-click and the pointer
    /// moving over them, with the pointer saying what a press would do.
    pub(super) fn lanes_area(
        &self,
        skim: Option<f64>,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let frame = self.frame(skim, window, cx);
        let hover_cursor = self.hover_cursor();
        let cell = self.bounds.clone();
        let canvas = canvas(
            move |bounds, window, _| {
                // A new size lays the lanes out again on the next frame.
                if cell.get().size != bounds.size {
                    window.request_animation_frame();
                }
                cell.set(bounds);
            },
            move |bounds, (), window, cx| paint(&frame, bounds, window, cx),
        )
        .size_full();
        let tip = self.tip.clone().map(|tip| {
            let (x, y, w, h) = tip.bounds;
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w.max(1.0)))
                .h(px(h.max(1.0)))
                .child(ui::tipped(
                    ElementId::Name(format!("timeline-tip-{}", tip.key).into()),
                    tip.tip,
                    div().w(px(w.max(1.0))).h(px(h.max(1.0))),
                ))
        });
        let empty = (!self.lanes.has_clips()).then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(layout::RULER))
                .bottom_0()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::label(
                    "No cut yet \u{2014} ask Kerf, or import media",
                    text::BODY,
                    theme::ink3(),
                ))
        });
        div()
            .flex_1()
            .h_full()
            .relative()
            .overflow_hidden()
            .when_some(hover_cursor, |area, cursor| area.cursor(cursor))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::context_press))
            .on_mouse_move(cx.listener(Self::hover_at))
            .child(canvas)
            .children(empty)
            .children(tip)
    }
}

fn paint(frame: &Frame, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    listen(frame, window);
    if let Some(cursor) = frame.cursor {
        window.set_window_cursor_style(cursor);
    }
    let view = &frame.view;
    if view.width <= 0.0 {
        // Laid out for the first time: drawn on the next frame, at its size.
        return;
    }
    let pen = Pen {
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
    };
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let fonts = Fonts::new();
    let lanes_box = pen.rect(0.0, layout::RULER, width, height - layout::RULER);
    masked(window, lanes_box, |window| {
        paint_lanes(frame, pen, width, height, &fonts, window, cx)
    });
    paint_ruler(frame, pen, width, &fonts, window, cx);
    if let Some((Some(time), _)) = &frame.kerf {
        paint_kerf(view, *time, pen, width, height, &fonts, window, cx);
    }
    paint_playhead(view, frame.playhead, pen, width, height, window);
    if let Some(time) = frame.skimmer {
        let x = view.x(time).floor();
        if (0.0..width).contains(&x) {
            window.paint_quad(fill(pen.rect(x, 0.0, 1.0, height), theme::ink2()));
        }
    }
    if frame.zoomed {
        // Where the view is along the program.
        let (left, across) = thumb(view, frame.lanes.end);
        let bar = pen.rect(left, height - SCROLLER - 2.0, across, SCROLLER);
        window.paint_quad(quad(
            bar,
            px(SCROLLER / 2.0),
            theme::with_alpha(theme::ink(), 0.22),
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// While the canvas shows, a drag follows the pointer anywhere in the
/// window and ends wherever its button comes up.
fn listen(frame: &Frame, window: &mut Window) {
    let this = frame.this.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture {
            this.update(cx, |view, cx| view.pointer_moved(event, cx))
                .ok();
        }
    });
    let this = frame.this.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
            this.update(cx, |view, cx| view.release(event.position, cx))
                .ok();
        }
    });
}

/// Whether `row` shows on a canvas `height` tall, under the ruler.
fn visible(view: &Viewport, height: f32, row: &Row) -> bool {
    let y = view.row_y(row);
    y < height && y + row.height > layout::RULER
}

fn paint_lanes(
    frame: &Frame,
    pen: Pen,
    width: f32,
    height: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let lanes = &*frame.lanes;
    paint_grounds(frame, pen, width, height, window);
    paint_rows(frame, pen, width, height, fonts, window, cx);
    paint_moved(frame, pen, width, fonts, window, cx);
    paint_tethers(frame, pen, width, height, window);
    if let Some(time) = frame.snapped {
        let x = view.x(time).floor();
        window.paint_quad(fill(
            pen.rect(x, layout::RULER, 1.0, height - layout::RULER),
            theme::accent_ink(),
        ));
    }
    if let Some((edge, delta, row)) = frame.readout
        && let Some(row) = lanes.rows.get(row)
    {
        paint_readout(
            view,
            edge,
            delta,
            view.row_y(row),
            lanes.fps,
            pen,
            width,
            fonts,
            window,
            cx,
        );
    }
}

/// Under the clips: a locked lane's hatching, a veil over what each open
/// proposal changes, each lane's line; the lanes quieter after the
/// program's end; each marker's line down them.
fn paint_grounds(frame: &Frame, pen: Pen, width: f32, height: f32, window: &mut Window) {
    let view = &frame.view;
    let lanes = &*frame.lanes;
    for row in lanes.rows.iter().filter(|row| visible(view, height, row)) {
        let y = view.row_y(row);
        if row.locked {
            let hatch = pattern_slash(theme::with_alpha(theme::ink(), 0.05).into(), 1.0, 7.0);
            window.paint_quad(fill(pen.rect(0.0, y, width, row.height), hatch));
        }
        // A veil over what each open proposal changes on it.
        for &(start, end) in &row.veils {
            let (x0, x1) = (view.x(start).max(-1.0), view.x(end).min(width + 1.0));
            if x1 > x0 {
                window.paint_quad(fill(
                    pen.rect(x0, y, x1 - x0, row.height),
                    theme::with_alpha(theme::ink(), 0.035),
                ));
            }
        }
        window.paint_quad(fill(
            pen.rect(0.0, y + row.height - 1.0, width, 1.0),
            theme::line(),
        ));
    }
    // After the program's end the lanes are quieter.
    let end_x = view.x(lanes.end).max(0.0);
    if end_x < width {
        let after = pen.rect(end_x, layout::RULER, width - end_x, height - layout::RULER);
        window.paint_quad(fill(after, theme::with_alpha(theme::surround_dim(), 0.45)));
    }
    for mark in &lanes.markers {
        let x = view.x(mark.time).floor();
        if (0.0..width).contains(&x) {
            let line = pen.rect(x, layout::RULER, 1.0, height - layout::RULER);
            window.paint_quad(fill(line, theme::with_alpha(mark.color, 0.22)));
        }
    }
}

/// Each lane that shows: where a previewed proposal's clips are now
/// (dashed), its clips (a dragged one's outline where it was, and of each
/// it takes along; a change reel's glow, cover and ring round each), its
/// transitions.
fn paint_rows(
    frame: &Frame,
    pen: Pen,
    width: f32,
    height: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let lanes = &*frame.lanes;
    let ghost = frame.ghost.as_ref();
    for row in lanes.rows.iter().filter(|row| visible(view, height, row)) {
        let y = view.row_y(row);
        // Where a previewed proposal's clips are now, dashed, under its own.
        for &(start, end) in &row.ghosts {
            if let Some((left, top, across, tall)) = clip_box(view, width, row, y, start, end) {
                window.paint_quad(quad(
                    pen.rect(left, top, across, tall),
                    px(radius::SMALL),
                    theme::with_alpha(theme::ink(), 0.03),
                    px(1.0),
                    theme::with_alpha(theme::ink(), 0.3),
                    BorderStyle::Dashed,
                ));
            }
        }
        for clip in &row.clips {
            match ghost.filter(|ghost| ghost.moves(&clip.id) && !row.previewed) {
                // Where a dragged clip was shows as an outline, as does each it takes along; a
                // dropped one is drawn where it went.
                Some(ghost) if ghost.dragging => {
                    outline_clip(view, pen, width, row, y, clip.start, clip.end(), window)
                }
                Some(_) => {}
                None => {
                    let look = frame.watch_look(row, clip);
                    paint_watched(view, pen, width, row, y, clip, look, false, window);
                    paint_clip(
                        frame,
                        pen,
                        width,
                        row,
                        y - look.lift,
                        clip,
                        (clip.start, clip.duration),
                        None,
                        0.0,
                        fonts,
                        window,
                        cx,
                    );
                    paint_watched(view, pen, width, row, y, clip, look, true, window);
                }
            }
        }
        for &(start, end) in &row.transitions {
            paint_transition(view, pen, width, row, y, start, end, window);
        }
    }
}

/// Points across the line from a clip to the one it follows, of the dark
/// edge on either side of it that keeps it apart from a filmstrip, and
/// across the dot it ends in.
const TETHER: f32 = 2.0;
const TETHER_EDGE: f32 = 1.0;
const TETHER_DOT: f32 = 7.0;

/// A line from a clip that follows another to the one it follows, as drawn:
/// down (or up) the lanes from where the follower starts (`x`, from `top`
/// to `bottom`), then along the followed clip's facing edge (`to`) to the
/// dot (`end_x`).
#[derive(Clone, Copy)]
struct Tether {
    x: f32,
    top: f32,
    bottom: f32,
    to: f32,
    end_x: f32,
    /// The follower or the clip it follows is chosen.
    lit: bool,
}

/// From each clip that follows another to the one it follows: a line from where the
/// follower starts, across the lanes between them, to a dot on the followed clip. Where the
/// follower starts before or after that clip, the line runs along the clip's lane to its
/// nearer end. A dragged clip, and what it takes along, are joined where they are drawn.
/// The lines of the chosen clips, and of the clips they follow, are in the accent, over
/// the others.
fn paint_tethers(frame: &Frame, pen: Pen, width: f32, height: f32, window: &mut Window) {
    let view = &frame.view;
    let lanes = &*frame.lanes;
    // Where a clip drawn elsewhere is drawn: its row, start and end.
    let placed: HashMap<&str, (usize, f64, f64)> = frame
        .ghost
        .iter()
        .flat_map(|ghost| ghost.placed(lanes))
        .map(|placed| {
            (
                placed.clip.id.as_str(),
                (placed.row, placed.start, placed.end()),
            )
        })
        .collect();
    let at = |row: usize, clip: &ClipBox| {
        placed
            .get(clip.id.as_str())
            .copied()
            .unwrap_or((row, clip.start, clip.end()))
    };
    let mut tethers = Vec::new();
    for (index, row) in lanes.rows.iter().enumerate() {
        for clip in &row.clips {
            let Some(followed) = clip.clip.follows.as_deref() else {
                continue;
            };
            let Some((lead_index, lead)) = lanes
                .find(followed)
                .map(|(lead_row, lead)| (lead_row, &lanes.rows[lead_row].clips[lead]))
            else {
                continue;
            };
            let (index, start, _) = at(index, clip);
            let (lead_index, lead_start, lead_end) = at(lead_index, lead);
            if index == lead_index {
                continue;
            }
            let (row, lead_row) = (&lanes.rows[index], &lanes.rows[lead_index]);
            let (y, lead_y) = (view.row_y(row), view.row_y(lead_row));
            // From the follower's edge that faces the followed clip, to that clip's edge
            // that faces the follower.
            let (from, to) = if y < lead_y {
                (y + row.height - CLIP_INSET, lead_y + CLIP_INSET)
            } else {
                (y + CLIP_INSET, lead_y + lead_row.height - CLIP_INSET)
            };
            let (top, bottom) = (from.min(to).max(layout::RULER), from.max(to).min(height));
            let x = view.x(start).floor();
            // Where on the followed clip the line ends: under the follower's start, or at
            // the end of the clip nearer to it.
            let (lead_x0, lead_x1) = (view.x(lead_start), view.x(lead_end));
            let end_x = x.clamp(lead_x0.floor(), (lead_x1 - TETHER).max(lead_x0).floor());
            if bottom <= top || (x < 0.0 && end_x < 0.0) || (x > width && end_x > width) {
                continue;
            }
            let lit = frame
                .selection
                .iter()
                .any(|chosen| *chosen == clip.id || chosen == followed);
            tethers.push(Tether {
                x,
                top,
                bottom,
                to,
                end_x,
                lit,
            });
        }
    }
    tethers.sort_by_key(|tether| tether.lit);
    for tether in &tethers {
        paint_tether(tether, pen, width, height, window);
    }
}

/// One line and its dot: a dark edge under all of it first, so where it
/// crosses a filmstrip it stays apart from the picture, then the line, in
/// ink2, or the accent's ink when it is lit.
fn paint_tether(tether: &Tether, pen: Pen, width: f32, height: f32, window: &mut Window) {
    let Tether {
        x,
        top,
        bottom,
        to,
        end_x,
        lit,
    } = *tether;
    let ink = if lit {
        theme::accent_ink()
    } else {
        theme::ink2()
    };
    let shows = |x: f32| (-TETHER..width).contains(&x);
    let along = (layout::RULER..height).contains(&to);
    let (left, right) = (x.min(end_x).max(-TETHER), x.max(end_x).min(width));
    for (grow, ink) in [(TETHER_EDGE, theme::lane::name_shadow()), (0.0, ink)] {
        if shows(x) {
            window.paint_quad(fill(
                pen.rect(x - grow, top, TETHER + 2.0 * grow, bottom - top),
                ink,
            ));
        }
        if end_x != x && right > left && along {
            let half = TETHER / 2.0 + grow;
            window.paint_quad(fill(
                pen.rect(
                    left - grow,
                    to - half,
                    right - left + TETHER + 2.0 * grow,
                    2.0 * half,
                ),
                ink,
            ));
        }
        if shows(end_x) && along {
            let across = TETHER_DOT + 2.0 * grow;
            let middle = end_x + TETHER / 2.0;
            window.paint_quad(quad(
                pen.rect(middle - across / 2.0, to - across / 2.0, across, across),
                px(across / 2.0),
                ink,
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
        }
    }
}

/// A clip drawn other than where the snapshot has it, on the row it would
/// land on, and each it takes along, as far along on its own row: where one
/// of them will meet another clip on its row, a dashed box where they meet.
fn paint_moved(
    frame: &Frame,
    pen: Pen,
    width: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let lanes = &*frame.lanes;
    let Some(ghost) = frame.ghost.as_ref() else {
        return;
    };
    let placed = ghost.placed(lanes);
    for moved in &placed {
        let row = &lanes.rows[moved.row];
        let (edge, head_shift) = if moved.clip.id == ghost.id {
            (ghost.edge, ghost.head_shift)
        } else {
            (None, 0.0)
        };
        paint_clip(
            frame,
            pen,
            width,
            row,
            view.row_y(row),
            moved.clip,
            (moved.start, moved.duration),
            edge,
            head_shift,
            fonts,
            window,
            cx,
        );
    }
    // Where a moved clip will meet another, a dashed box where they meet.
    if !ghost.dragging || ghost.edge.is_some() || frame.tool == Tool::Slip {
        return;
    }
    for (index, moved) in placed.iter().enumerate() {
        let row = &lanes.rows[moved.row];
        let y = view.row_y(row);
        // The others on its row where they will be: those that stay, and those moved after it.
        let staying = row
            .clips
            .iter()
            .filter(|other| !ghost.moves(&other.id))
            .map(|other| (other.start, other.end()));
        let moved_after = placed[index + 1..]
            .iter()
            .filter(|other| other.row == moved.row)
            .map(|other| (other.start, other.end()));
        let (g0, g1) = (view.x(moved.start), view.x(moved.end()));
        for (start, end) in staying.chain(moved_after) {
            let (o0, o1) = (view.x(start), view.x(end));
            let (left, right) = (g0.max(o0).max(-4.0), g1.min(o1).min(width + 4.0));
            if right - left < 6.0 {
                continue;
            }
            let top = y + CLIP_INSET + 2.5;
            let tall = row.height - 2.0 * CLIP_INSET - 5.0;
            window.paint_quad(quad(
                pen.rect(left + 2.5, top, right - left - 5.0, tall),
                px(0.0),
                gpui::transparent_black(),
                px(1.0),
                theme::ink(),
                BorderStyle::Dashed,
            ));
        }
    }
}

/// By a dragged edge: its new timecode and how far it moved,
/// "00:00:12:04  +00:00:01:02".
#[expect(
    clippy::too_many_arguments,
    reason = "painting takes the window, where the canvas is and each measure it draws with"
)]
fn paint_readout(
    view: &Viewport,
    edge: f64,
    delta: f64,
    row_y: f32,
    fps: f64,
    pen: Pen,
    width: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let words = format!(
        "{}  {}{}",
        model::timecode(edge, fps),
        if delta < 0.0 { "-" } else { "+" },
        model::timecode(delta.abs(), fps)
    );
    let line = shape(
        window,
        words.into(),
        text::MONO_SMALL,
        &fonts.mono,
        theme::ink(),
    );
    let across = f32::from(line.width) + 16.0;
    let x = view.x(edge);
    let mut left = x + 6.0;
    if left + across > width - 4.0 {
        left = x - 6.0 - across;
    }
    let top = (row_y - 22.0).max(layout::RULER + 2.0);
    window.paint_quad(quad(
        pen.rect(left, top, across, 20.0),
        px(radius::CONTROL),
        theme::card(),
        px(1.0),
        theme::line2(),
        BorderStyle::Solid,
    ));
    if let Err(error) = line.paint(
        pen.at(left + 8.0, top + (20.0 - text::MONO_SMALL.line) / 2.0),
        px(text::MONO_SMALL.line),
        window,
        cx,
    ) {
        log::warn!("a dragged edge's timecode did not draw: {error}");
    }
}

/// What a change reel draws round `clip` on `row` (at `y`), as `look`
/// says: under it (`over` false), the glow it rises over; over it, the
/// cover on a clip it does not watch, or the accent's ring round one it
/// does, where it has risen to.
#[expect(
    clippy::too_many_arguments,
    reason = "painting takes the window, where the canvas is and each measure it draws with"
)]
fn paint_watched(
    view: &Viewport,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    clip: &ClipBox,
    look: WatchLook,
    over: bool,
    window: &mut Window,
) {
    if look == WatchLook::default() {
        return;
    }
    let Some((left, top, across, tall)) = clip_box(view, width, row, y, clip.start, clip.end())
    else {
        return;
    };
    let top = top - look.lift;
    if !over {
        if look.glow > 0.0 {
            let glow = theme::lane::watch_glow();
            window.paint_quad(quad(
                pen.rect(left - 4.0, top - 2.0, across + 8.0, tall + 10.0),
                px(radius::SMALL + 2.0),
                theme::with_alpha(glow, glow.a * look.glow),
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
        }
        return;
    }
    if look.dim > 0.0 {
        let dim = theme::lane::watch_dim();
        window.paint_quad(fill(
            pen.rect(left, top, across, tall),
            theme::with_alpha(dim, dim.a * look.dim),
        ));
    }
    if look.ring > 0.0 {
        // Just outside it, so the ring covers none of the clip.
        window.paint_quad(quad(
            pen.rect(left - 2.0, top - 2.0, across + 4.0, tall + 4.0),
            px(radius::SMALL + 2.0),
            gpui::transparent_black(),
            px(2.0),
            theme::with_alpha(theme::accent_ink(), look.ring),
            BorderStyle::Solid,
        ));
    }
}

/// The box a clip from `start` to `end` takes on `row` drawn at `y`, kept
/// within a little of the canvas so a long clip makes no huge quad.
pub(super) fn clip_box(
    view: &Viewport,
    width: f32,
    row: &Row,
    y: f32,
    start: f64,
    end: f64,
) -> Option<(f32, f32, f32, f32)> {
    let x0 = view.x(start);
    let x1 = view.x(end).max(x0 + 2.0);
    if x1 < -8.0 || x0 > width + 8.0 {
        return None;
    }
    let left = x0.max(-8.0);
    let right = x1.min(width + 8.0);
    Some((
        left,
        y + CLIP_INSET,
        right - left,
        row.height - 2.0 * CLIP_INSET,
    ))
}

/// Where a dragged clip was.
#[expect(
    clippy::too_many_arguments,
    reason = "painting takes the window, where the canvas is and each measure it draws with"
)]
fn outline_clip(
    view: &Viewport,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    start: f64,
    end: f64,
    window: &mut Window,
) {
    if let Some((left, top, across, tall)) = clip_box(view, width, row, y, start, end) {
        let body = pen.rect(left, top, across, tall);
        let ground = theme::with_alpha(theme::card(), 0.5);
        window.paint_quad(quad(
            body,
            px(radius::SMALL),
            ground,
            px(1.0),
            theme::line2(),
            BorderStyle::Dashed,
        ));
    }
}

/// A transition: a small hatched mark over its cut.
#[expect(
    clippy::too_many_arguments,
    reason = "painting takes the window, where the canvas is and each measure it draws with"
)]
fn paint_transition(
    view: &Viewport,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    start: f64,
    end: f64,
    window: &mut Window,
) {
    let (mut x0, mut x1) = (view.x(start), view.x(end));
    if x1 - x0 < 8.0 {
        let middle = (x0 + x1) / 2.0;
        (x0, x1) = (middle - 4.0, middle + 4.0);
    }
    if x1 < 0.0 || x0 > width {
        return;
    }
    let tall = row.height - 2.0 * CLIP_INSET;
    let mark = pen.rect(x0, y + CLIP_INSET + tall * 0.25, x1 - x0, tall * 0.5);
    let hatch = pattern_slash(theme::with_alpha(theme::ink(), 0.55).into(), 1.0, 3.0);
    let outline = theme::with_alpha(theme::ink2(), 0.7);
    window.paint_quad(quad(
        mark,
        px(3.0),
        hatch,
        px(1.0),
        outline,
        BorderStyle::Solid,
    ));
}
