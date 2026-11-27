//! The timeline: the program's tracks and clips, its ruler and playhead.
//!
//! A strip along the top says where the playhead is (or, in ink2, where the
//! pointer skims), and holds the editing tools, snap and zoom (− slider +),
//! and, while the window shows no picture, the transport first;
//! under it the track heads stand at the left, and the ruler and the lanes
//! are drawn on one canvas from [`Lanes`], a compact copy of the snapshot's
//! timeline rebuilt only when a snapshot comes. Subtitle lanes stack on
//! top, then video with the highest track uppermost (V2 over V1), then
//! audio; under the last an add-track row. An empty sequence still shows
//! its heads and a ruler of at least 30 s.
//!
//! What a press or a drag on a clip does is the tool's ([`Tool`]):
//! Selection chooses, moves (onto another lane of its kind too) and trims;
//! Razor splits where it is pressed; Ripple trims and closes or opens the
//! gap; Roll moves the cut between two clips; Slip shows other media in the
//! same span; Slide moves a clip between its neighbours. The edit goes to
//! kerfd as one typed operation when the button comes up; until the next
//! snapshot says where the clip is, it is drawn where it was dropped, so it
//! never jumps back. A click or a drag on the ruler or on an empty lane
//! seeks, scrubbing while it drags; moving over the lanes skims.
//!
//! A sound clip carries its waveform and its volume rubber band; a picture
//! clip its filmstrip; a signal its number as a curve, which an Option-drag
//! draws anew. A right-click on a clip opens its menu, on a head the
//! track's (Sync lock, heights); a double-click on a clip of a shot made
//! from steps opens its graph.
//!
//! A clip that follows another (a title its shot, music a scene's first
//! shot) is joined to it by a line from where it starts, with a dot on the
//! clip it follows: it moves when that one moves, and ripple edits pass
//! under it. A clip's menu has it follow its shot, the clip of the first
//! video track where it starts, or lets it go.
//!
//! The program stays editable while proposals are open: each open proposal
//! has a band on the ruler ("Proposal · author"), a veil over the lanes it
//! changes, and the program's clips it changes are outlined in sage. While
//! one plays as a preview, its tracks show the clips where it puts them,
//! with dashed ghosts where the clips it moves, trims or takes out are now.
//! Only a first cut on an empty program shows as the proposal's own cut,
//! read only.
//!
//! The program's in and out marks stand on the ruler, the range between
//! them shaded. While the pointer skims the picture or the lanes a skimmer
//! stands at the time it shows, and the playhead stays where it was.
//!
//! While Kerf works, its cursor stands where it is looking (the time it
//! names, else the first clip it names, else the newest change in its live
//! proposal) and rings the clips it looks at. Clips whose media is being
//! made carry moving amber stripes (still under Reduce Motion); a making
//! waiting its turn a dashed amber outline and "Queued"; a shot not made
//! yet, or a file that is gone, a dashed hole saying "Not made" or
//! "Offline". The view asks for frames only while stripes show.

mod activity;
mod canvas;
mod clip;
mod drag;
mod follow;
mod gain;
mod heads;
mod heights;
mod hover;
mod lane_ink;
mod lanes;
mod menus;
mod pointer;
mod ruler;
mod signal;
mod strip;
mod tools;
mod viewport;
mod watch;

pub use activity::activity_label;
pub use editor_state::track_heights::{
    default_height, stepped_height, TRACK_LEAST, TRACK_MOST, TRACK_STEPS,
};
/// − or + held down zooms again after this, and then every
/// [`ZOOM_REPEAT`], until it is let go or the pointer leaves it.
pub use editor_ui::theme::motion::{ZOOM_REPEAT, ZOOM_REPEAT_DELAY};
pub use gain::{add_gain_key, band_line, gain_at, gain_y, GAIN_CEILING, GAIN_FLOOR};
pub use lanes::shown_version;
pub use pointer::extend;
pub use signal::{signal_at, signal_points};
pub use strip::{strip_has_transport, StripFit};
pub use tools::Tool;
pub use viewport::{zoom_of_share, zoom_share};
pub use watch::{watch_look, watched_clips, ClipWatch, WatchLook};

use drag::{Gesture, Pending};
use editor_session::commands::RunCommand;
use editor_session::session::{BridgeEvent, Session};
use editor_stage::stage::{self, MaskTool};
use editor_state::prefs::TrackHeights;
use editor_state::store::Change;
use editor_ui::theme::{self, layout};
use gain::GainShown;
use gpui::{
    anchored, deferred, div, point, prelude::*, px, size, App, Bounds, Context, Entity,
    EventEmitter, FocusHandle, Focusable, KeyDownEvent, Pixels, Subscription, Task, Window,
};
use hover::{CanvasTip, Hover};
use lanes::Lanes;
use menus::OpenMenu;
use serde_json::Value;
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use strip::StripWidths;
use viewport::{Viewport, ZOOM_STEP};

/// What the timeline tells the window, which subscribes to it: the
/// timeline cannot reach the picture, the session's command context or
/// the project's settings itself.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineEvent {
    /// The pointer skims the lanes at this time (a whole frame's), or has
    /// stopped skimming (None, also on a press or as playback starts). The
    /// window has the picture show that frame without moving the playhead,
    /// as skimming the picture itself does, and show the playhead's again
    /// on None.
    Skim(Option<f64>),
    /// The tracks a source's picture and sound go on now (Insert,
    /// Overwrite), as the person targeted them, at most one of each kind:
    /// the window keeps them for `Session::command_context`'s
    /// `sourceTargets`, which [`model::source_targets`](editor_state::model::source_targets)
    /// works out from them. None follows the first unlocked track of each kind.
    Targets(Option<Vec<String>>),
    /// The tracks' heights changed: the window keeps them for the project
    /// (with the editor's settings, by its path) and gives them back with
    /// [`TimelineView::set_track_heights`] when it opens.
    Heights(TrackHeights),
}

// -------------------------------------------------------------------- view

pub struct TimelineView {
    session: Entity<Session>,
    focus_handle: FocusHandle,
    lanes: Rc<Lanes>,
    /// Points a second; None fits the program to the width.
    zoom: Option<f32>,
    /// The time at the lanes' left edge, in seconds.
    left: f64,
    scroll_y: f32,
    snap: bool,
    tool: Tool,
    /// The lanes as last laid out: what a press hits, and what draws.
    view: Viewport,
    /// Where the canvas was last laid out, in the window.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Where the zoom slider was last laid out, in the window.
    slider: Rc<Cell<Bounds<Pixels>>>,
    gesture: Option<Gesture>,
    hover: Option<Hover>,
    pending: Option<Pending>,
    /// The playhead as last laid out, to follow it when it moves.
    last_position: f64,
    /// Ticks a making's clock once a second while one runs.
    clock: Option<Task<()>>,
    /// − or + held down: zooming again while it is ([`ZOOM_REPEAT`]).
    zoom_held: Option<Task<()>>,
    heights: TrackHeights,
    /// The tracks a source's picture and sound go on; None: the first
    /// unlocked track of each kind.
    targets: Option<Vec<String>>,
    /// The sound tracks soloed, for this session's monitoring only.
    solo: Vec<String>,
    /// Clips the last finished check found a warning or an error in.
    qc: HashSet<String>,
    /// The proposal playing as a preview, as the preview's events say.
    previewing: Option<String>,
    /// The cut Kerf builds, while it does, as the lanes were last built:
    /// they show its newest version.
    building: Option<String>,
    /// The version of its proposal the pinned card shows, as the window
    /// last said ([`TimelineView::show_version`]).
    shown_version: Option<String>,
    /// Where Kerf's cursor stands while it works, and the changes of its
    /// live proposal it has stood on.
    kerf_at: Option<f64>,
    live_changes: HashSet<String>,
    /// The clips a change reel watches, risen off their lanes while it plays.
    watch: ClipWatch,
    /// Where the pointer skims the lanes.
    skim: Option<f64>,
    /// A signal drawn, or a band's values let go, shown until kerfd answers.
    drawn: Option<(String, BTreeMap<i64, f64>, u64)>,
    held_gain: Option<(GainShown, u64)>,
    menu: Option<OpenMenu>,
    tip: Option<CanvasTip>,
    /// Each command's keys in the keymap in use, for tips and menus, as the
    /// session last said.
    keys: (String, HashMap<String, Vec<String>>),
    /// Option-scroll over the heads, gathering a trackpad's small steps.
    wheel: f32,
    /// The pointer over the add-track row.
    add_hover: bool,
    /// Why each of the strip's transport controls cannot run, for the
    /// command context it was asked of (the playhead left out).
    transport_asked: Option<(Value, HashMap<&'static str, String>)>,
    /// Whether the strip carries the transport, as the session last said,
    /// and how many times it has come: each time it fades in anew.
    strip_transport: (bool, u64),
    /// How wide the strip and its pieces were last laid out, and what it
    /// showed then.
    strip_widths: StripWidths,
    /// Drawn again as an edit waiting on kerfd runs out (`PENDING`).
    pending_ends: editor_ui::motion::Redraw,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TimelineEvent> for TimelineView {}

impl TimelineView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let playhead = session.read(cx).playhead.clone();
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            // The playhead, and the timecodes that say where it is.
            cx.observe(&playhead, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                this.session_changed(change, cx)
            }),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.bridge_event(&event.0, cx)
            }),
        ];
        let mut view = Self {
            session,
            focus_handle: cx.focus_handle(),
            lanes: Rc::default(),
            zoom: None,
            left: 0.0,
            scroll_y: 0.0,
            snap: true,
            tool: Tool::Selection,
            view: Viewport::default(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            slider: Rc::new(Cell::new(Bounds::default())),
            gesture: None,
            hover: None,
            pending: None,
            last_position: 0.0,
            clock: None,
            zoom_held: None,
            heights: TrackHeights::default(),
            targets: None,
            solo: Vec::new(),
            qc: HashSet::new(),
            previewing: None,
            building: None,
            shown_version: None,
            kerf_at: None,
            live_changes: HashSet::new(),
            watch: ClipWatch::default(),
            skim: None,
            drawn: None,
            held_gain: None,
            menu: None,
            tip: None,
            keys: (String::new(), HashMap::new()),
            wheel: 0.0,
            add_hover: false,
            transport_asked: None,
            strip_transport: (false, 0),
            strip_widths: StripWidths::default(),
            pending_ends: editor_ui::motion::Redraw::default(),
            _subscriptions: subscriptions,
        };
        view.rebuild(cx);
        view.tick_makings(cx);
        view
    }

    // ------------------------------------------------------------- the api

    /// Gives the timeline the keys: Space, Home and End, the arrows, S,
    /// Delete, M and the tool keys work at once. The window's
    /// `settle_focus` calls it when the panel shows the timeline: on first
    /// activation, when a sheet or the Media band closes, and when nothing
    /// else has the keys.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    /// The tool a press or a drag on a clip uses.
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Chooses the tool (the registry's `tool.*`: its keys while the
    /// timeline has them, and the window's menus and palette).
    pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if self.tool != tool {
            self.tool = tool;
            self.hover = None;
            cx.notify();
        }
    }

    /// Whether a drag snaps to clip edges, the playhead and markers
    /// (Sequence ▸ Snap, the strip's Snap).
    pub fn snapping(&self) -> bool {
        self.snap
    }

    /// Turns snapping on or off (`sequence.snap`).
    pub fn set_snapping(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.snap != on {
            self.snap = on;
            cx.notify();
        }
    }

    /// The tracks a source's picture and sound go on, as the person
    /// targeted them; None follows the first unlocked track of each kind.
    pub fn targets(&self) -> Option<&[String]> {
        self.targets.as_deref()
    }

    /// Clip `id`'s box in the window as the lanes last drew it, cut to the
    /// lanes; None while it is scrolled out of view or the lanes are not
    /// laid out. The window stands the Ask popover over it (`ai.ask`).
    pub fn clip_bounds(&self, id: &str) -> Option<Bounds<Pixels>> {
        let canvas = self.bounds.get();
        let (width, height) = (f32::from(canvas.size.width), f32::from(canvas.size.height));
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let (x, y, across, tall) = self.clip_rect_of(id)?;
        let (left, right) = (x.max(0.0), (x + across).min(width));
        let (top, bottom) = (y.max(layout::RULER), (y + tall).min(height));
        (right > left && bottom > top).then(|| {
            Bounds::new(
                canvas.origin + point(px(left), px(top)),
                size(px(right - left), px(bottom - top)),
            )
        })
    }

    /// A point at the playhead on the lanes' top, in the window (the lanes'
    /// middle while the playhead is out of view); None while the lanes are
    /// not laid out. The Ask popover stands there with no clip chosen.
    pub fn playhead_anchor(&self, cx: &App) -> Option<Bounds<Pixels>> {
        let canvas = self.bounds.get();
        let width = f32::from(canvas.size.width);
        if width <= 0.0 || f32::from(canvas.size.height) <= 0.0 {
            return None;
        }
        let x = self.view.x(self.session.read(cx).state.position);
        let x = if (0.0..=width).contains(&x) {
            x
        } else {
            width / 2.0
        };
        Some(Bounds::new(
            canvas.origin + point(px(x), px(layout::RULER)),
            size(px(1.0), px(1.0)),
        ))
    }

    /// Whether kerfd is connecting or gone: the lanes and the heads take
    /// no presses, drags or switches then, and show none of what a press
    /// would take, since nothing done then would reach kerfd.
    pub fn offline(&self, cx: &App) -> bool {
        !self.session.read(cx).connected()
    }

    // ---------------------------------------------------------------- keys

    /// The registry's commands the timeline answers itself while it has the
    /// keys: the tools, Esc letting a chosen clip go when there is no
    /// proposal to reject, the track heights and Add Track….
    fn run_command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let id = action.id.as_ref();
        if let Some(tool) = Tool::from_command(id) {
            self.set_tool(tool, cx);
            return;
        }
        match id {
            "proposal.reject" => {
                let session = self.session.read(cx);
                let none_open = session.state.snapshot.pending().is_none();
                if none_open && !session.selection.is_empty() {
                    self.session.update(cx, |session, cx| {
                        session.selection.clear();
                        cx.notify();
                    });
                } else {
                    cx.propagate();
                }
            }
            "view.fitTracks" => self.fit_tracks(cx),
            "view.resetTrackHeights" => self.reset_track_heights(cx),
            "timeline.addTrack" => self.open_add_track(window, cx),
            "sequence.snap" => {
                let on = !self.snap;
                self.set_snapping(on, cx);
            }
            _ => cx.propagate(),
        }
    }

    /// Option-Up and Option-Down step every track; ⌘= and ⌘− zoom (and
    /// repeat while held).
    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let only_alt = modifiers.alt && !modifiers.shift && !modifiers.secondary();
        match keystroke.key.as_str() {
            "up" | "down" if only_alt => {
                let steps = if keystroke.key == "up" { 1 } else { -1 };
                self.step_track_heights(steps, cx);
                cx.stop_propagation();
            }
            "=" | "+" if modifiers.secondary() && !modifiers.alt => {
                self.zoom_by(ZOOM_STEP, None);
                cx.stop_propagation();
                cx.notify();
            }
            "-" if modifiers.secondary() && !modifiers.alt => {
                self.zoom_by(1.0 / ZOOM_STEP, None);
                cx.stop_propagation();
                cx.notify();
            }
            _ => {}
        }
    }
}

impl Focusable for TimelineView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Only as it draws: put away with the panel, the timeline neither
        // pages after the playhead nor lets an edit go; it does as it shows
        // again.
        self.let_pending_go(cx);
        self.lay_out(cx);
        self.follow(cx);
        // Skimming the picture or the lanes moves what it shows, not the
        // playhead: the picture keeps the window's playhead where it was.
        let skim = self
            .skim
            .or_else(|| editor_stage::stage::skim(&self.session, cx));
        let position = self.session.read(cx).state.position;
        let lanes_area = self.lanes_area(skim, window, cx);
        let skimming =
            self.skim.is_some() || editor_stage::stage::skim(&self.session, cx).is_some();
        let strip = self.strip(skim.unwrap_or(position), skimming, cx);
        let heads = self.heads(cx);
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(heads)
            .child(lanes_area);
        let menu = self.menu.as_ref().map(|open| {
            deferred(
                anchored()
                    .position(open.at)
                    .snap_to_window_with_margin(px(8.0))
                    .child(open.menu.clone()),
            )
            .with_priority(2)
        });

        div()
            .key_context(editor_session::commands::TIMELINE)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::run_command))
            .on_key_down(cx.listener(Self::key_down))
            // ⇧R ⇧E ⇧P: the picture's mask tools, for the clip chosen here.
            .on_action(cx.listener(|this, _: &stage::RectangleMask, window, cx| {
                stage::toggle_mask_tool(&this.session, MaskTool::Rectangle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &stage::EllipseMask, window, cx| {
                stage::toggle_mask_tool(&this.session, MaskTool::Ellipse, window, cx);
            }))
            .on_action(cx.listener(|this, _: &stage::PenMask, window, cx| {
                stage::toggle_mask_tool(&this.session, MaskTool::Pen, window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::well())
            .child(strip)
            .child(body)
            .children(menu)
    }
}
