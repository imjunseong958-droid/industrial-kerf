//! The stage: the picture, in the frame of the device it is made for (a
//! laptop for 16:9 and wider, a phone for vertical and square), with its
//! transport under it.
//!
//! Nothing moves the picture's colours: the room around it stays neutral.
//! While something works a light runs round just outside the picture's
//! edge, blue for Kerf and amber for the computer, amber winning; the chip
//! at its top left says what runs. Both fade in, and out when it ends;
//! under Reduce Motion the ring is still.
//!
//! The picture takes the format of the cut it shows (a first cut Kerf
//! proposes on an empty program brings its own), and a frame is drawn whole
//! at its own shape, never stretched. Over it: the safe zones when they are
//! on (View ▸ Safe Zones), and the chosen clip's masks and motion path,
//! which a drag reshapes. The mask tools (⇧R ⇧E ⇧P, or the transport's)
//! draw a new mask on the chosen picture clip: a rectangle or an ellipse
//! dragged out, or a pen's corners clicked, closed on its first point. A
//! click on the picture plays or pauses; moving the pointer across it skims
//! the program, the timeline showing a skimmer, and leaving it brings the
//! playhead's frame back. Nothing here moves by itself: frames are asked
//! for only while the light runs.

mod activity;
mod caption;
mod editing;
pub mod hud;
pub mod overlay;
mod paint;
mod picture;
mod preview;
mod reel;
mod shown;
mod skim;
mod stand_in;
mod tools;
mod transport;

pub use activity::{kerf_cutting, pulse_level};
pub use hud::{render_card_shows, render_stop_holds, RenderCard};
pub use paint::stroke_over;
pub use picture::{shown_aspect, wide, zoom_words, zoomed, WIDE_FROM, ZOOMS};
pub use preview::{preview_chip_words, previewed_version};
pub use reel::{base64_decode, compare_at};
pub use shown::shows_program;
pub use skim::{skim, Heard, Kept};
pub use stand_in::{placed_clips, poster_source, releases_cover};
pub use tools::{mask_tool_id, MaskTool};
pub use transport::{transport_reasons, transport_tip, TRANSPORT};

use crate::motion::{self, Tween};
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme;
use crate::ui;
use activity::{Hue, Said};
use gpui::{
    div, point, prelude::*, px, App, Bounds, Context, CursorStyle, Div, Entity, EntityId,
    FocusHandle, Global, KeyBinding, KeyContext, MouseButton, Pixels, RenderImage, Subscription,
    Task, WeakEntity, Window,
};
use kerf_bridge::event::{Check, Finding};
use kerf_bridge::Event;
use overlay::{Overlap, Pt};
use picture::{place, Painting, Placement};
use serde_json::Value;
use shown::{Drag, Shown};
use skim::forget_skim;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

// RemoveChosen: Delete or Backspace on what the picture has chosen (a
// mask, a path point) or the pen's last corner. RectangleMask, EllipseMask
// and PenMask take out a mask tool, or put it down again. CancelDrawing:
// Esc lets go of what is being drawn, then puts the tool down. FinishPen:
// Return leaves the pen's outline open.
gpui::actions!(
    stage,
    [
        RemoveChosen,
        RectangleMask,
        EllipseMask,
        PenMask,
        CancelDrawing,
        FinishPen
    ]
);

/// The viewer's zoom chosen from a menu (the app's View ▸ Viewer Zoom): a
/// percent of the frame's own pixels, 0 to fit it. The window carries it
/// out, bringing the picture in for it.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct SetZoom {
    pub percent: u32,
}

/// The picture's key context, there while the stage has the focus.
pub const CONTEXT: &str = "Stage";
/// With it while Delete and Backspace have something on the picture to
/// take: a mask or a path point chosen, or a pen's corners. Otherwise the
/// window's own Delete has the key.
const DELETES: &str = "deletes";
/// With it while a mask tool is out: Esc is the picture's.
const DRAWS: &str = "draws";
/// With it while the pen has two corners or more: Return leaves it open.
const PEN_OPEN: &str = "pen_open";

/// Binds the picture's keys. ⇧R ⇧E ⇧P take out (or put down) a mask tool
/// wherever the window has the keys, so one is to hand from any pane
/// (typing in a field types them); Delete and Backspace, Esc and Return act
/// on what the picture has chosen or is drawing, ahead of the window's own
/// keys, only while it has something to act on.
///
/// `keymap::bind` calls it after the registry's keys, since binding a
/// keymap clears these; each stage then reads its transport's keys again
/// (`KeymapBound`). A stage binds them again as it is made, and as a tool
/// is taken out or something on the picture chosen, should they be gone.
pub fn bind_keys(cx: &mut App) {
    let mut bindings = Vec::new();
    for context in [
        CONTEXT,
        crate::commands::TIMELINE,
        crate::commands::WORKSPACE,
    ] {
        bindings.push(KeyBinding::new("shift-r", RectangleMask, Some(context)));
        bindings.push(KeyBinding::new("shift-e", EllipseMask, Some(context)));
        bindings.push(KeyBinding::new("shift-p", PenMask, Some(context)));
    }
    for key in ["shift-r", "shift-e", "shift-p"] {
        bindings.push(KeyBinding::new(
            key,
            gpui::NoAction,
            Some(crate::input::CONTEXT),
        ));
    }
    let deletes = format!("{CONTEXT} && {DELETES}");
    bindings.push(KeyBinding::new("delete", RemoveChosen, Some(&deletes)));
    bindings.push(KeyBinding::new("backspace", RemoveChosen, Some(&deletes)));
    let draws = format!("{CONTEXT} && {DRAWS}");
    bindings.push(KeyBinding::new("escape", CancelDrawing, Some(&draws)));
    let open = format!("{CONTEXT} && {PEN_OPEN}");
    bindings.push(KeyBinding::new("enter", FinishPen, Some(&open)));
    cx.bind_keys(bindings);
    cx.set_global(KeymapBound);
}

/// Set each time the picture's keys are bound, as a keymap is (Settings,
/// View ▸ Keyboard): each stage reads its transport's keys again then.
struct KeymapBound;

impl Global for KeymapBound {}

/// Binds the picture's keys again if a keymap bound since cleared them.
fn ensure_keys(cx: &mut App) {
    let bound = cx
        .key_bindings()
        .borrow()
        .bindings_for_action(&RectangleMask)
        .next()
        .is_some();
    if !bound {
        bind_keys(cx);
    }
}

/// Each window's picture, by its session.
#[derive(Default)]
struct Stages(HashMap<EntityId, WeakEntity<StageView>>);

impl Global for Stages {}

/// Takes out the mask tool `tool` on `session`'s picture, or puts it down
/// if it is out, as ⇧R ⇧E ⇧P do: the timeline calls it for its keys, and a
/// menu may. False (and a notice saying why) when no picture clip is chosen
/// to draw on.
pub fn toggle_mask_tool(
    session: &Entity<Session>,
    tool: MaskTool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let stage = cx
        .try_global::<Stages>()
        .and_then(|stages| stages.0.get(&session.entity_id()))
        .and_then(WeakEntity::upgrade);
    stage.is_some_and(|stage| stage.update(cx, |stage, cx| stage.toggle_tool(tool, window, cx)))
}

// --------------------------------------------------------------------- view

pub struct StageView {
    session: Entity<Session>,
    focus_handle: FocusHandle,
    area: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The picture as last placed, in the window: what a press and the pointer hit.
    picture: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The frame as last drawn in it, in the window: the picture itself at
    /// Fit, else the frame at the zoom, centred and cut to the picture.
    /// What the overlays are drawn over and hit on.
    canvas: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The viewer's zoom: 0 fits the frame to the picture; else the share
    /// of its own pixels it is drawn at.
    zoom: f32,
    /// The zoom's menu while it is open (the kit's: its keys, Esc and a
    /// press outside close it), and what drops it as it closes.
    zoom_menu: Option<(Entity<ui::Menu>, Subscription)>,
    hue: Option<Hue>,
    /// The light's hue as it fades out.
    last_hue: Hue,
    edge: Tween,
    edge_since: Instant,
    /// The program's timeline as the last snapshot sent it, whole, as its
    /// JSON: the overlays read it so (props, parent, masks, effects).
    program: Rc<Value>,
    /// The overlays as last drawn, kept as the picture's canvas is laid
    /// out: what a press and the pointer hit.
    shown: Rc<RefCell<Option<Rc<Shown>>>>,
    /// The time the pointer shows across the picture, while it skims.
    skim: Option<f64>,
    /// The playhead while kerfd may still answer the skim's seeks.
    kept: Option<Kept>,
    /// The clip whose mask (`Session::chosen_mask`, shared with the
    /// inspector) or path point is chosen, and which point.
    chosen_clip: Option<String>,
    chosen_point: Option<usize>,
    /// The mask tool out, if one is.
    tool: Option<MaskTool>,
    /// The pen's corners so far, in fractions of the clip's picture.
    pen: Vec<Pt>,
    /// Where the pointer is over the stage, in the window, while the pen draws.
    pointer: Option<Pt>,
    /// What the chip over the picture said last, kept while it fades out.
    said: Option<Said>,
    /// Ticks the chip's clock once a second while a making runs and
    /// nothing else asks for frames (Reduce Motion).
    clock: Option<Task<()>>,
    /// The snapshot's proposals, their versions and timelines: what the
    /// preview's card says it renders.
    book: Rc<crate::chat::ProposalBook>,
    /// The proposal whose preview render Stop was pressed on, hidden until
    /// that run ends.
    stopped: Option<String>,
    /// The last finished check's findings (`check` done), for its
    /// caption-overlap box.
    check: Option<Rc<Vec<Finding>>>,
    /// The overlap box over the picture, fading in and out, and the last
    /// one shown, kept while it fades out.
    overlap_level: Tween,
    overlap: Option<Overlap>,
    /// The caption's balanced width on a vertical picture, for its words
    /// and the most it may be: words, most, width, lines. Kept as the
    /// caption is laid out ([`Self::on_frame`]), so it is worked out again
    /// only when the words or the room change, not on every frame played.
    caption_fit: RefCell<Option<(gpui::SharedString, i32, f32, usize)>>,
    drag: Option<Drag>,
    /// What a drag let go, drawn where it was dropped until kerfd's
    /// snapshot (the revision it was sent at) says where it is.
    held: Option<(Drag, u64, Instant)>,
    cursor: Option<CursorStyle>,
    /// The transport's commands' keys in the keymap they were read for, as
    /// the system writes them.
    keys: (String, HashMap<String, String>),
    /// Why each transport control cannot run, and the command context
    /// (less the playhead) it was asked for: asked again only when that
    /// changes, not on every frame played.
    transport_asked: Option<(Value, HashMap<&'static str, String>)>,
    /// The clips of a cut Kerf builds that have been seen placed, and the
    /// media of the newest: the picture stands in as that clip until its
    /// render comes (Cut builds).
    cut_seen: HashSet<String>,
    cut_newest: Option<String>,
    /// The poster standing in for the picture, and the one before it as it
    /// crossfades over APPEAR.
    stand_in_poster: Option<(String, Instant)>,
    stand_in_before: Option<String>,
    /// Watching what a proposal changed: the segment whose Before still
    /// is asked for or shown ("proposal/index"), the still and since when
    /// its line sweeps; the side last seen, the frame showing as the side
    /// changed (it wipes away once the next frame comes), the wipe, and
    /// the frame last seen.
    compare_key: Option<String>,
    compare: Option<(Arc<RenderImage>, Instant)>,
    reel_side: Option<crate::session::ReelSide>,
    wipe_armed: Option<Arc<RenderImage>>,
    wipe: Option<(Arc<RenderImage>, Instant)>,
    last_frame: Option<Arc<RenderImage>>,
    /// Pictures let go of, for the atlas to drop as the stage next draws.
    retired: Rc<RefCell<Vec<Arc<RenderImage>>>>,
    /// Drawn again as a drop held for kerfd's answer runs out (`HOLD`).
    hold_ends: motion::Redraw,
    _subscriptions: Vec<Subscription>,
}

impl StageView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = session.entity_id();
        let this = cx.weak_entity();
        cx.default_global::<Stages>().0.insert(id, this);
        ensure_keys(cx);
        let playhead = session.read(cx).playhead.clone();
        let subscriptions = vec![
            cx.observe(&session, |this, session, cx| {
                this.follow_selection(&session, cx);
                cx.notify();
            }),
            // The picture, its caption and the transport's time.
            cx.observe(&playhead, |_, _, cx| cx.notify()),
            // A keymap bound: the transport's tips say its keys.
            cx.observe_global::<KeymapBound>(|this, cx| {
                if this.follow_keymap(cx) {
                    cx.notify();
                }
            }),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.bridge_event(&event.0, cx)
            }),
            cx.subscribe(&session, |this, session, change: &Change, cx| {
                this.session_change(&session, change, cx)
            }),
            // A key that moves the playhead while the pointer skims (a frame
            // on, the start) ends the skim, from the playhead.
            cx.observe_keystrokes(|this, _, _, cx| this.follow_playhead(cx)),
            cx.on_release(move |_, cx| {
                forget_skim(id, cx);
                if cx.has_global::<Stages>() {
                    cx.global_mut::<Stages>().0.remove(&id);
                }
            }),
        ];
        Self {
            session,
            focus_handle: cx.focus_handle(),
            area: Rc::new(Cell::new(None)),
            picture: Rc::new(Cell::new(None)),
            canvas: Rc::new(Cell::new(None)),
            zoom: 0.0,
            zoom_menu: None,
            hue: None,
            last_hue: Hue::Kerf,
            edge: Tween::at(0.0),
            edge_since: Instant::now(),
            program: Rc::new(Value::Null),
            shown: Rc::new(RefCell::new(None)),
            skim: None,
            kept: None,
            chosen_clip: None,
            chosen_point: None,
            tool: None,
            pen: Vec::new(),
            pointer: None,
            said: None,
            clock: None,
            book: Rc::default(),
            stopped: None,
            check: None,
            overlap_level: Tween::at(0.0),
            overlap: None,
            caption_fit: RefCell::new(None),
            drag: None,
            held: None,
            cursor: None,
            keys: (String::new(), HashMap::new()),
            cut_seen: HashSet::new(),
            cut_newest: None,
            transport_asked: None,
            stand_in_poster: None,
            stand_in_before: None,
            compare_key: None,
            compare: None,
            reel_side: None,
            wipe_armed: None,
            wipe: None,
            last_frame: None,
            retired: Rc::new(RefCell::new(Vec::new())),
            hold_ends: motion::Redraw::default(),
            _subscriptions: subscriptions,
        }
    }

    /// What the bridge said, as far as the picture keeps it: the program
    /// and its proposals (a drag let go is drawn as kerfd says once a
    /// snapshot after the one it was sent at has come), the last finished
    /// check, a preview render ending, a clock, a Before still.
    fn bridge_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::State { snapshot } => {
                let before = std::mem::replace(
                    &mut self.program,
                    Rc::new(serde_json::to_value(&snapshot.timeline).unwrap_or_default()),
                );
                self.follow_chosen_point(&before, cx);
                self.book = Rc::new(crate::chat::ProposalBook::read(snapshot));
                self.follow_cut(cx);
                // What a drag let go is drawn where kerfd says once
                // a snapshot after the one it was sent at has come.
                let revision = self.session.read(cx).revision();
                if self
                    .held
                    .as_ref()
                    .is_some_and(|(_, sent_at, _)| *sent_at != revision)
                {
                    self.held = None;
                }
            }
            // The last finished check's findings; one running keeps them.
            Event::Check(Check::Done { findings, .. }) => {
                self.check = Some(Rc::new(findings.clone()));
                cx.notify();
            }
            // A render stopped here stays hidden until its run ends,
            // unless another proposal's render takes over.
            Event::Preview(_) => {
                if !render_stop_holds(self.stopped.as_deref(), event) {
                    self.stopped = None;
                }
                // The render of the cut Kerf builds has come: it is
                // the picture now, and the poster standing in goes
                // (until Kerf places another clip).
                let cut = self
                    .session
                    .read(cx)
                    .first_cut()
                    .map(|(proposal, _)| proposal.proposal_id.clone());
                if releases_cover(event, cut.as_deref()) && self.cut_newest.is_some() {
                    self.cut_newest = None;
                    cx.notify();
                }
            }
            // A position that is not finite is none (JSON's `null`).
            Event::Clock { position, .. } => {
                if position.is_finite() {
                    self.hear_clock(*position, cx);
                }
            }
            // The Before still of the segment watched.
            Event::EffectPreview(preview) => self.hear_compare(preview, cx),
            _ => {}
        }
    }

    /// What the session says changed: playback starting from the playhead,
    /// kerfd gone, an edit refused.
    fn session_change(
        &mut self,
        session: &Entity<Session>,
        change: &Change,
        cx: &mut Context<Self>,
    ) {
        let state = &session.read(cx).state;
        match change {
            // Playback starts from the playhead, never from where the
            // pointer skimmed, and its clocks are the playhead's.
            Change::Playback if state.playing && self.kept.is_some() => {
                let kept = self.kept.take();
                if self.skim.take().is_some() {
                    forget_skim(session.entity_id(), cx);
                    if let Some(kept) = kept {
                        session.update(cx, |session, cx| {
                            session.seek(kept.playhead(), false);
                            cx.notify();
                        });
                    }
                }
                cx.notify();
            }
            // kerfd gone or only connecting: the picture is cleared,
            // and the pointer skims nothing until it is back.
            Change::Connection if state.connection != crate::store::Connection::Connected => {
                if self.skim.take().is_some() {
                    forget_skim(session.entity_id(), cx);
                }
                self.kept = None;
                cx.notify();
            }
            // An edit kerfd refused says so: what was dropped goes back.
            Change::Notice => {
                let refused = self.held.as_ref().is_some_and(|(_, _, at)| {
                    state
                        .notices
                        .last()
                        .is_some_and(|notice| notice.error && notice.at >= *at)
                });
                if refused {
                    self.held = None;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// Follows what the picture keeps of the session, the program and the
    /// playhead as it draws: what is chosen on it, the change reel's still
    /// and wipe (ended once it has crossed the picture), the light and the
    /// chip, a drop held for kerfd's answer, the poster standing in, the
    /// check's box once the stage has been laid out, and the transport's
    /// reasons and keys (what can still be drawn on follows the overlays
    /// drawn, [`Self::follow_shown`]). Each part changes only what has
    /// changed since. What the chip says now.
    ///
    /// Only as it draws: a stage the panel has put away asks kerfd for no
    /// Before still and starts no fade, and what it missed meanwhile starts
    /// from the frame it shows again (the light and the chip fading in, the
    /// poster crossfading in). Only its draw knows that it is drawn.
    fn follow(&mut self, cx: &mut Context<Self>) -> Option<Said> {
        let session = self.session.clone();
        self.follow_selection(&session, cx);
        self.follow_reel(cx);
        self.end_wipe(cx);
        let chip = self.follow_activity(cx);
        self.follow_held(cx);
        self.follow_posters(cx);
        if self.area.get().is_some() {
            self.follow_overlap(cx);
        }
        self.follow_transport(cx);
        self.follow_keymap(cx);
        chip
    }

    /// The picture's key context: Delete, Esc and Return are its own only
    /// while it has something to act on (`shown`, the overlays drawn now).
    fn key_context(&self, shown: Option<&Shown>, cx: &App) -> KeyContext {
        let mut context = KeyContext::default();
        context.add(CONTEXT);
        if self.takes_delete(shown, cx) {
            context.add(DELETES);
        }
        if self.tool.is_some() || self.drawing() {
            context.add(DRAWS);
        }
        if self.pen.len() >= 2 {
            context.add(PEN_OPEN);
        }
        context
    }

    /// What lies over the picture as it was placed last frame, in the
    /// stage's own points: the chip saying what runs, the change reel's
    /// tags and strip, the posters or the words standing in, what sits on
    /// the frame (the caption, the check's box), and the preview render's
    /// card. Nothing until the stage has been laid out once.
    fn over_picture(
        &self,
        placed: Option<Placement>,
        over: Over,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> Div {
        let mut overlays = div().absolute().inset_0();
        let Some(placed) = placed else {
            return overlays;
        };
        let picture = placed.picture;
        let card_says =
            over.preview_card.is_some() && self.session.read(cx).state.makings.is_empty();
        overlays =
            overlays.children(self.work_chip(&placed, over.chip, over.edge, card_says, window, cx));
        // Before and After, at the picture's foot, while the still sweeps.
        if over.comparing && over.framed {
            overlays = overlays.children(self.compare_tags(picture));
        }
        overlays = overlays
            .children(self.reel_strip_over(picture, cx))
            .children(stand_in::poster_layers(&over.posters, picture))
            .children(
                over.stand_in
                    .map(|words| stand_in::words_over(words, picture)),
            )
            .child(self.on_frame(picture, over.aspect, over.frame_size, window, cx));
        // The preview render's card, in title safe at the picture's top right.
        if let Some((title, detail, progress)) = over.preview_card {
            overlays = overlays.child(preview::PreviewCard {
                card: picture,
                title,
                detail,
                progress,
                on_stop: Box::new(
                    cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.stop_render(cx)),
                ),
            });
        }
        overlays
    }
}

/// What the stage lays over the picture this frame, worked out as it draws.
struct Over {
    /// What the chip says now, if anything runs.
    chip: Option<Said>,
    /// How clear the light round the picture is.
    edge: f32,
    /// The Before still sweeps, and there is a frame under it.
    comparing: bool,
    framed: bool,
    /// The posters standing in, each with how clear it is.
    posters: Vec<(Arc<RenderImage>, f32)>,
    /// The words standing in for a picture with none, nor a poster.
    stand_in: Option<gpui::SharedString>,
    aspect: f32,
    frame_size: (u32, u32),
    /// What the preview render's card says, while it shows.
    preview_card: Option<(String, String, f64)>,
}

impl Render for StageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = self.follow(cx);
        // Once a frame, for what is drawn and what the pointer hits (the
        // canvas keeps it as it lays the picture out).
        let shown = self.overlays(self.session.read(cx)).map(Rc::new);
        self.follow_shown(shown.as_deref());
        let session = self.session.read(cx);
        let aspect = shown_aspect(session);
        let frame_size = picture::frame_size(session);
        let frame = session
            .picture
            .as_ref()
            .map(|picture| (picture.image.clone(), picture.width, picture.height));
        let framed = frame.is_some();
        let safe_zones = session.prefs.safe_zones;
        let chosen_mask = session.chosen_mask.clone();
        // The Before still sweeping, and the wipe of a side changed.
        let compare = self.compare_now(window);
        let comparing = compare.is_some();
        let wipe = self.wipe_now(window);
        let edge = self.edge.sample(window);
        if edge > 0.0 && !motion::reduced() {
            window.request_animation_frame();
        }
        // Never a blank while a picture is coming: the posters standing in,
        // the newest fading in over the last; else words.
        let posters = self.posters(window, cx);
        // The frame shows under the first poster as it fades in; once one
        // covers it whole it is not drawn.
        let covering = posters.iter().any(|(_, level)| *level >= 1.0);
        let stand_in = self.standing_in(framed, !posters.is_empty(), cx);
        let context = self.key_context(shown.as_deref(), cx);

        // Overlays sit on the picture as it was placed last frame.
        // The canvas is placed in window coordinates; the overlays in the stage's own.
        let placed = self
            .area
            .get()
            .map(|area| place(Bounds::new(point(px(0.0), px(0.0)), area.size), aspect));
        let stage = self.picture_canvas(
            Painting {
                aspect,
                frame_size,
                zoom: self.zoom,
                frame,
                covering,
                wipe,
                compare,
                safe_zones,
                shown: shown.clone(),
                chosen_mask,
                chosen_point: self.chosen_point,
                in_hand: self.in_hand(),
                edge,
                hue_color: self.last_hue.color(),
                since: self.edge_since,
                dragging: self.drag_cursor(),
            },
            cx,
        );
        let overlays = self.over_picture(
            placed,
            Over {
                chip,
                edge,
                comparing,
                framed,
                posters,
                stand_in,
                aspect,
                frame_size,
                preview_card: self.preview_card(cx),
            },
            window,
            cx,
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::surround())
            .child(
                div()
                    .id("stage-picture")
                    .track_focus(&self.focus_handle)
                    .key_context(context)
                    .on_action(cx.listener(Self::remove_chosen))
                    .on_action(cx.listener(Self::cancel_drawing))
                    .on_action(cx.listener(Self::finish_pen_open))
                    .on_action(cx.listener(|this, _: &RectangleMask, window, cx| {
                        this.toggle_tool(MaskTool::Rectangle, window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &EllipseMask, window, cx| {
                        this.toggle_tool(MaskTool::Ellipse, window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &PenMask, window, cx| {
                        this.toggle_tool(MaskTool::Pen, window, cx);
                    }))
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .when_some(self.cursor, |area, cursor| area.cursor(cursor))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
                    .on_mouse_move(cx.listener(Self::hover))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !*hovered && this.drag.is_none() {
                            this.end_skim(cx);
                            this.set_cursor(None, cx);
                        }
                    }))
                    .child(stage)
                    .child(overlays),
            )
            .child(self.transport(shown.as_deref(), cx))
    }
}
