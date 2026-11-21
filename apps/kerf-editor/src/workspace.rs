//! The editor's window: one room. The folders and conversations at the
//! left, the conversation with Kerf beside them, and at the right a panel
//! with the picture, the Media band and the timeline once there is a cut
//! to show. What one goes into (Settings) is over the window, which dims.
//!
//! Every region moves by a [`Tween`] of its own: the sidebar slides in from
//! the window's edge, the panel from the right with its left edge against
//! the conversation's right as it narrows, the Media band down from under
//! the picture. State changes at once; motion follows.
//!
//! Under the toolbar, bars say what stands in the way until it is dealt
//! with: kerfd gone (Reconnect), project.kerf changed on disk (keep the
//! editor's version or load the file's), the last error (Dismiss). What
//! cannot be undone, and a folder that is not a project yet, are asked
//! first, over the window ([`Confirm`]).
//!
//! A command that cannot run now says why, as a notice, and the toolbar's
//! buttons are greyed with the reason (and their keys) in their tooltips.
//! Another project, new, chosen or recent, opens in place of this window.

use crate::chat::{ChatEvent, ChatView};
use crate::commands::{ClearRecent, OpenRecent, RunCommand, WORKSPACE};
use crate::export::{ExportEvent, ExportView, Quality};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::media::MediaBand;
use crate::motion::{self, Tween};
use crate::session::{BridgeEvent, Session};
use crate::settings::{Page, SettingsEvent, SettingsView};
use crate::sidebar::{self, SidebarEvent, SidebarView};
use crate::stage::StageView;
use crate::store::{Change, Connection};
use crate::theme::{self, layout, motion as scene, text};
use crate::timeline::TimelineView;
use crate::ui::{self, icon_button};
use gpui::{
    canvas, div, img, point, prelude::*, px, size, AnimationExt, AnyElement, AnyView, App, Bounds,
    ClickEvent, Context, DismissEvent, DispatchPhase, Entity, EventEmitter, FocusHandle, Focusable,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, PathPromptOptions, Pixels,
    RenderImage, Rgba, ScrollWheelEvent, SharedString, Stateful, Subscription, Window,
    WindowBounds,
};
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The last error, kept in a bar under the toolbar until it is dismissed.
struct ErrorBar {
    /// As the bridge said it, to tell its notice from the rest.
    raw: String,
    /// As a person reads it.
    text: String,
    /// A failed turn's: the next turn takes it away.
    from_agent: bool,
    /// Its place among the errors, so a new one fades in afresh.
    key: u64,
}

pub struct Workspace {
    session: Entity<Session>,
    sidebar: Entity<SidebarView>,
    chat: Entity<ChatView>,
    stage: Entity<StageView>,
    timeline: Entity<TimelineView>,
    media: Entity<MediaBand>,
    inspector: Entity<crate::inspector::InspectorView>,
    formats: Entity<crate::inspector::FormatBank>,
    collab: Entity<crate::finish::Collab>,
    /// Settings while open, and how many times it has been opened.
    /// What one has gone into (Settings, a shot's graph, Finish), over the
    /// window, and how many times one has been opened.
    modal: Option<(AnyView, u64)>,
    modals_opened: u64,
    export: Option<Entity<ExportView>>,
    focus_handle: FocusHandle,
    sidebar_open: bool,
    sidebar_shown: Tween,
    panel_open: bool,
    panel_shown: Tween,
    /// The person closed the panel, or it went home with the picture: a
    /// cut arriving does not open it again until something asks for it.
    panel_by_hand: bool,
    media_open: bool,
    media_shown: Tween,
    picture_shown: Tween,
    timeline_shown: Tween,
    /// The window is too narrow for the sidebar (as last drawn): it gives
    /// way, whatever its switch says.
    sidebar_gives_way: bool,
    /// The conversation's menu, from the chevron after its title.
    title_menu: Option<Entity<ui::Menu>>,
    /// The app's menu while it is open, and whether it opened from the
    /// sidebar's account (above it) rather than the toolbar's ⋯ (under it).
    app_menu: Option<(Entity<ui::Menu>, bool)>,
    /// How tall the app's menu was laid out, once it has been: where it
    /// sits in the room the window has for it follows.
    app_menu_tall: Rc<Cell<Option<f32>>>,
    /// How far the wheel has moved it in its room, when it is taller.
    app_menu_scrolled: Rc<Cell<f32>>,
    title_field: Entity<TextInput>,
    /// The conversation whose title the toolbar edits in place.
    renaming_title: Option<String>,
    error: Option<ErrorBar>,
    errors: u64,
    /// A folder that was no project yet: its media comes in once kerfd has it.
    start_media: Option<PathBuf>,
    window_title: String,
    /// Activity, from the title's menu: what the window and Kerf did.
    activity_open: bool,
    /// The renders this window queued, by job: one that finishes flies home.
    queued_here: HashSet<String>,
    /// What travels over the window: a frame going home (Manual export,
    /// Going home), the clips of a cut being built, the picture growing
    /// out of what it came from or going back into its card.
    flights: Vec<Flight>,
    /// The clips a cut being built placed, waiting for the timeline to
    /// draw them (and for the picture to have grown) before they fly.
    pending_clips: Vec<PendingClip>,
    /// What the cut being built had placed when last seen ("clip@end").
    live_seen: HashSet<String>,
    /// The picture growing out of what it came from, since when, and the
    /// clip it stood in for (which does not fly again).
    grow: Option<(Instant, Option<String>)>,
    /// What was called off before it landed (a stand-in a new one takes
    /// over from, one yet to leave as its pane went): it leaves the window
    /// as it next draws, a frame it carried with it ([`Workspace::flights_now`]).
    called_off: Vec<Flight>,
    /// The picture going back into its card, or its stand-in let go as the
    /// picture went before it had grown, since when: the stage is not drawn
    /// meanwhile.
    going_back: Option<Instant>,
    /// Where the stage was laid out last, in the window (None: it was not).
    stage_slot: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Where the panel's left edge was drawn last frame, and where it was
    /// to come to rest.
    panel_left: (f32, f32),
    /// Each command's keys in the keymap they were read for, as the system
    /// writes them, for the toolbar's tooltips.
    keys: (String, HashMap<String, Vec<String>>),
    /// The keys have been settled once, as the window first came to the
    /// front ([`Workspace::settle_focus`]); and they are to be settled again
    /// once this frame is drawn (a region that held them has folded away).
    focus_settled: bool,
    settle_soon: bool,
    /// The toolbar's own Tab stops that are not the kit's: the title, and
    /// Export's two parts.
    title_stop: FocusHandle,
    export_focus: FocusHandle,
    export_options_focus: FocusHandle,
    /// The first snapshot has settled the keys: on the timeline when it
    /// came with a cut, else in the composer.
    keys_found_their_place: bool,
    /// The conversation Kerf works in, while it does: a turn that ends in
    /// it while the window is in the back leaves its dot.
    working_in: Option<String>,
    /// When the project was last edited (`revisionTime`), seconds since 1970.
    revision_time: Option<i64>,
    /// Kerf rendered the cut itself in the turn under way (Going home).
    turn_rendered: bool,
    /// While what a proposal changed is watched, the stage reaches over
    /// the conversation, which falls back (`ChatView` dims itself).
    reel_reach: Tween,
    /// The picture's mask tool out when last seen, for the menu bar's tick.
    mask_tool_seen: Option<crate::stage::MaskTool>,
    _subscriptions: Vec<Subscription>,
}

/// Something travelling over the window, from where it leaves to where it
/// lands, an `img` over everything ([`Workspace::flights_now`]).
struct Flight {
    image: Arc<RenderImage>,
    /// `image` is a frame of the picture (the stage's own), which leaves
    /// the atlas once it has landed unless the stage still shows it or
    /// another flight carries it; else a poster, the artwork's, which stays.
    frame: bool,
    from: Bounds<Pixels>,
    to: Bounds<Pixels>,
    /// When it leaves: later than it was asked for when it is staggered.
    started: Instant,
    kind: FlightKind,
}

/// What goes from the panel: all of it, or one of its two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneGoes {
    Panel,
    Picture,
    Timeline,
}

/// What a flight is, which says how it goes and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum FlightKind {
    /// Manual export: a frame of the picture, fading as it goes
    /// ([`export_flight_at`]).
    Export,
    /// Going home: the picture itself, whole and clear until it lands
    /// ([`home_flight_at`]).
    Home,
    /// A clip of a cut being built, from its footage tile (or over the
    /// composer) to where the timeline draws it, ringed and named
    /// ([`clip_flight_at`]); it lands where the clip is drawn now.
    Clip { clip: String, name: SharedString },
    /// The picture's stand-in growing out of what it came from into the
    /// picture, whole on black ([`grow_flight_at`]); it lands where the
    /// picture is to be, and holds there while a picture is coming, until
    /// it comes (the frame shown is no longer `frame`), a clip lands over
    /// it, or nothing is coming: then, `released`, it fades.
    Grow {
        frame: Option<usize>,
        released: Option<Instant>,
    },
    /// The picture going back into cut `proposal`'s card, whole on black
    /// ([`back_flight_at`]); it lands where the card's picture is drawn.
    Back { proposal: String },
    /// A flight (`was`, drawn as it was) whose landing went away before
    /// it got there, a pane going: it holds where it had got to and fades
    /// there from how clear it was ([`let_go_flight_at`]).
    LetGo { was: Box<FlightKind>, clear: f32 },
}

impl FlightKind {
    /// Where it is `elapsed` after it left: how far along and how clear;
    /// None once it has gone.
    pub fn at(&self, elapsed: Duration) -> Option<(f32, f32)> {
        match self {
            FlightKind::Export => export_flight_at(elapsed),
            FlightKind::Home => home_flight_at(elapsed),
            FlightKind::Clip { .. } => clip_flight_at(elapsed),
            FlightKind::Grow { released, .. } => grow_flight_at(
                elapsed,
                released.map(|at| elapsed.saturating_sub(at.elapsed())),
            ),
            FlightKind::Back { .. } => back_flight_at(elapsed),
            FlightKind::LetGo { clear, .. } => let_go_flight_at(elapsed, *clear),
        }
    }

    /// Whether it flies on as `goes` goes from the panel: a clip lands in
    /// the timeline and the picture's stand-in in the picture, so each is
    /// let go when that goes; the rest land outside the panel (a frame
    /// going home, the picture going back into its card) or are let go.
    pub fn flies_on(&self, goes: PaneGoes) -> bool {
        match self {
            FlightKind::Clip { .. } => goes == PaneGoes::Picture,
            FlightKind::Grow { .. } => goes == PaneGoes::Timeline,
            FlightKind::Export
            | FlightKind::Home
            | FlightKind::Back { .. }
            | FlightKind::LetGo { .. } => true,
        }
    }
}

/// A clip a cut being built placed, waiting to fly to where the timeline
/// draws it.
struct PendingClip {
    placed: PlacedClip,
    asked: Instant,
}

/// What is drawn of a flight this frame.
struct Drawn {
    bounds: Bounds<Pixels>,
    opacity: f32,
    image: Arc<RenderImage>,
    kind: FlightKind,
}

impl Workspace {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar = cx.new(|cx| SidebarView::new(session.clone(), window, cx));
        let chat = cx.new(|cx| ChatView::new(session.clone(), window, cx));
        let stage = cx.new(|cx| StageView::new(session.clone(), window, cx));
        let timeline = cx.new(|cx| TimelineView::new(session.clone(), window, cx));
        // The tracks are as tall as they were left in this project.
        if let Some(project) = session.read(cx).project.clone() {
            let heights = session.read(cx).prefs.track_heights_of(&project);
            timeline.update(cx, |timeline, cx| timeline.set_track_heights(heights, cx));
        }
        let media = cx.new(|cx| MediaBand::new(session.clone(), window, cx));
        let inspector =
            cx.new(|cx| crate::inspector::InspectorView::new(session.clone(), window, cx));
        let formats = cx.new(|cx| crate::inspector::FormatBank::new(session.clone(), window, cx));
        let collab = cx.new(|cx| crate::finish::Collab::new(session.clone(), window, cx));
        let sidebar_open = session.read(cx).prefs.sidebar_open;
        sidebar.update(cx, |sidebar, cx| sidebar.set_shown(sidebar_open, cx));
        let title_field =
            cx.new(|cx| TextInput::new(cx, "Conversation title").with_style(text::TITLE));
        let title_focus = title_field.read(cx).focus_handle(cx);
        let mut subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            // A mask tool taken out or put down (⇧R ⇧E ⇧P, the transport):
            // the menu bar ticks it as the window next draws.
            cx.observe(&stage, |this, stage, cx| {
                let tool = stage.read(cx).mask_tool();
                if tool != this.mask_tool_seen {
                    this.mask_tool_seen = tool;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&session, window, Self::session_changed),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.bridge_event(&event.0, cx)
            }),
            cx.subscribe_in(
                &sidebar,
                window,
                |this, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::Settings => this.show_settings(None, window, cx),
                    SidebarEvent::Hide => this.set_sidebar(false, cx),
                    SidebarEvent::Delete(id) => this.confirm_delete(id.clone(), window, cx),
                    SidebarEvent::Done => {
                        if this.modal.is_none() {
                            window.focus(&this.focus_handle);
                            this.settle_focus(window, cx);
                        }
                    }
                    SidebarEvent::MenuOpened => {
                        this.close_title_menu(window, cx);
                        this.close_app_menu(window, cx);
                        cx.notify();
                    }
                    SidebarEvent::AppMenu { from_keys } => {
                        this.toggle_app_menu(true, *from_keys, window, cx)
                    }
                    SidebarEvent::Chosen => {
                        if this.modal.is_none() {
                            this.chat
                                .update(cx, |chat, cx| chat.focus_composer(window, cx));
                        }
                    }
                },
            ),
            // What the timeline cannot reach itself: the picture follows its
            // skim, Insert and Overwrite its targets, and the project keeps
            // its tracks' heights.
            cx.subscribe(
                &timeline,
                |this, _, event: &crate::timeline::TimelineEvent, cx| match event {
                    crate::timeline::TimelineEvent::Skim(time) => {
                        let time = *time;
                        this.stage.update(cx, |stage, cx| stage.skim_at(time, cx));
                    }
                    crate::timeline::TimelineEvent::Targets(targets) => {
                        let targets = targets.clone();
                        this.session
                            .update(cx, |session, _| session.source_targets = targets);
                    }
                    crate::timeline::TimelineEvent::Heights(heights) => {
                        let heights = heights.clone();
                        this.session.update(cx, |session, _| {
                            if let Some(project) = session.project.clone() {
                                session.prefs.keep_track_heights(&project, heights);
                                session.save_prefs();
                            }
                        });
                    }
                },
            ),
            // A cut's card pressed: its picture grows out of the card.
            cx.subscribe_in(
                &chat,
                window,
                |this, _, event: &ChatEvent, window, cx| match event {
                    ChatEvent::GrowPicture { proposal } => {
                        this.grow_from_card(proposal, window, cx)
                    }
                },
            ),
            // The lanes draw the version of a proposal the pinned card shows.
            cx.observe(&chat, |this, chat, cx| {
                let shown = chat.read(cx).pinned_proposal(cx);
                this.timeline
                    .update(cx, |timeline, cx| timeline.show_version(shown, cx));
            }),
            cx.subscribe_in(
                &title_field,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit | InputEvent::Cancel => {
                        this.finish_title_rename(*event == InputEvent::Submit, cx);
                        window.focus(&this.focus_handle);
                        this.settle_focus(window, cx);
                    }
                    InputEvent::Changed => {}
                },
            ),
            // File ▸ Open Recent checks this window's project while it is in
            // front. The first time, the keys go where they belong; and the
            // conversation looked at again lets its dot go.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    let project = this.session.read(cx).project.clone();
                    crate::commands::refresh_menus(project.as_deref(), cx);
                    if !this.focus_settled {
                        this.focus_settled = true;
                        this.settle_focus(window, cx);
                    }
                    this.session.update(cx, |session, cx| {
                        if let Some(open) = session.conversation.clone() {
                            if session.state.settled.contains(&open) {
                                session.state.looked_at(&open);
                                cx.notify();
                            }
                        }
                    });
                }
            }),
            // Leaving the title's field keeps what was typed.
            cx.on_blur(&title_focus, window, |this, _, cx| {
                this.finish_title_rename(true, cx)
            }),
            // Nothing has the keys (Esc left the composer or a field, what
            // had them went away): they settle again, on the timeline while
            // it shows, else with the window, so the timeline's keys (Play,
            // the arrows, S, Delete) and the window's (the palette, the
            // mask tools) go on working.
            cx.on_focus_lost(window, |this, window, cx| {
                window.focus(&this.focus_handle);
                this.settle_focus(window, cx);
            }),
            // Esc, before the timeline's (Reject Proposal, or letting the
            // chosen clip go) or a field's has it ([`escape_goes`]): an open
            // menu goes, wherever the keys are (a menu's press leaves them
            // where they were); else the preview render its card shows
            // stops; else the Media band is put away.
            {
                let this = cx.weak_entity();
                let handle = window.window_handle();
                cx.intercept_keystrokes(move |event, window, cx| {
                    let keystroke = &event.keystroke;
                    if window.window_handle() != handle
                        || keystroke.key != "escape"
                        || keystroke.modifiers.modified()
                    {
                        return;
                    }
                    let taken = this
                        .update(cx, |this, cx| {
                            match escape_goes(this.escape_at(&event.context_stack, cx)) {
                                EscapeGoes::On => false,
                                EscapeGoes::CloseMenus => this.close_menus(window, cx),
                                EscapeGoes::StopRender => {
                                    this.stop_render(cx);
                                    true
                                }
                                EscapeGoes::PutBandAway => {
                                    this.set_media(false, cx);
                                    true
                                }
                            }
                        })
                        .unwrap_or(false);
                    if taken {
                        cx.stop_propagation();
                    }
                })
            },
        ];
        let focus_handle = cx.focus_handle();
        // A press on what takes no keys (the conversation's empty room, a
        // chip, a label) gives them to the window's own box: they settle
        // again, on the timeline while it shows, as Qt's widgets that took
        // no focus left them there.
        subscriptions.push(cx.on_focus(&focus_handle, window, |this, _, cx| this.settle_soon(cx)));
        window.focus(&focus_handle);
        Self {
            session,
            sidebar,
            chat,
            stage,
            timeline,
            media,
            inspector,
            formats,
            collab,
            modal: None,
            modals_opened: 0,
            export: None,
            focus_handle,
            sidebar_open,
            sidebar_shown: Tween::at(if sidebar_open { 1.0 } else { 0.0 }),
            panel_open: false,
            panel_shown: Tween::at(0.0),
            panel_by_hand: false,
            media_open: false,
            media_shown: Tween::at(0.0),
            picture_shown: Tween::at(1.0),
            timeline_shown: Tween::at(1.0),
            sidebar_gives_way: false,
            title_menu: None,
            app_menu: None,
            app_menu_tall: Rc::new(Cell::new(None)),
            app_menu_scrolled: Rc::new(Cell::new(0.0)),
            title_field,
            renaming_title: None,
            error: None,
            errors: 0,
            start_media: None,
            window_title: String::new(),
            activity_open: false,
            queued_here: HashSet::new(),
            flights: Vec::new(),
            pending_clips: Vec::new(),
            live_seen: HashSet::new(),
            grow: None,
            called_off: Vec::new(),
            going_back: None,
            stage_slot: Rc::new(Cell::new(None)),
            panel_left: (0.0, 0.0),
            keys: (String::new(), HashMap::new()),
            focus_settled: false,
            settle_soon: false,
            title_stop: cx.focus_handle().tab_stop(true),
            export_focus: cx.focus_handle().tab_stop(true),
            export_options_focus: cx.focus_handle().tab_stop(true),
            keys_found_their_place: false,
            working_in: None,
            revision_time: None,
            turn_rendered: false,
            reel_reach: Tween::at(0.0),
            mask_tool_seen: None,
            _subscriptions: subscriptions,
        }
    }

    /// Whether the panel shows the timeline, with kerfd there to answer
    /// its keys: where the keys belong when nothing else holds them.
    fn timeline_in_view(&self, cx: &App) -> bool {
        self.panel_open
            && self.timeline_shown.target() > 0.5
            && self.modal.is_none()
            && self.session.read(cx).state.connection == Connection::Connected
    }

    /// Settles the keys where they belong (Qt's settleFocus): on the
    /// timeline while the panel shows it, since its keys (Space, Home and
    /// End, the arrows, S, Delete, M, the tools) work only from it; never
    /// away from a field being typed in, a menu, or what is open over the
    /// window. A region folded away gives back the keys it held. Called as
    /// the window first comes to the front, as what is over it or the Media
    /// band closes, when Esc leaves a field, and when nothing holds them.
    pub fn settle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        // The Media band put away keeps nothing of the keys (it stays laid
        // out, no height; what leaves the window gives them up by itself).
        if !self.media_open && self.media.focus_handle(cx).contains_focused(window, cx) {
            window.focus(&self.focus_handle);
        }
        let timeline = self.timeline.focus_handle(cx);
        let holder = match window.focused(cx) {
            None => Holder::Nothing,
            Some(focused) if focused == self.focus_handle => Holder::Nothing,
            Some(focused) if focused == timeline => Holder::Timeline,
            Some(_) if holds_keys_itself(&window.context_stack()) => Holder::Busy,
            Some(_) => Holder::Other,
        };
        match settle_keys(self.timeline_in_view(cx), holder) {
            KeysGo::Timeline => self.timeline.read(cx).focus(window),
            KeysGo::Window => window.focus(&self.focus_handle),
            KeysGo::Stay => {}
        }
    }

    /// The registry's checkable commands that are on in this window: the
    /// panes shown, the switches on, the timeline's tool; and the picture's
    /// mask tool out. Their rows in the menus and the palette are ticked.
    pub fn checked_commands(&self, cx: &App) -> BTreeSet<String> {
        let session = self.session.read(cx);
        let picture = self.panel_open && self.picture_shown.target() > 0.5;
        let timeline = self.panel_open && self.timeline_shown.target() > 0.5;
        let (_, _, looping) = session.marks();
        let live = session
            .state
            .collab
            .as_ref()
            .is_some_and(|collab| collab["live"].as_bool() == Some(true));
        [
            ("view.sidebar", self.sidebar_open),
            ("view.picture", picture),
            ("view.timeline", timeline),
            ("window.project", self.media_open),
            ("view.followKerf", session.prefs.follow_kerf),
            ("view.safeZones", session.prefs.safe_zones),
            ("playback.loop", looping),
            ("collab.live", live),
            ("sequence.snap", self.timeline.read(cx).snapping()),
            (self.timeline.read(cx).tool().command(), true),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(id, _)| id)
        // The mask tool out, by its own id ([`crate::commands::mask_tool_id`]).
        .chain(
            self.stage
                .read(cx)
                .mask_tool()
                .map(crate::commands::mask_tool_id),
        )
        .map(str::to_owned)
        .collect()
    }

    /// How the registry's commands stand here, for the menu bar (greyed,
    /// ticked). Only macOS draws one.
    fn menu_states(&self, cx: &App) -> crate::commands::MenuStates {
        let states = self.command_states(cx);
        let unavailable = states
            .as_object()
            .map(|states| {
                states
                    .iter()
                    .filter(|(_, reason)| reason.is_string())
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        crate::commands::MenuStates {
            unavailable,
            checked: self.checked_commands(cx),
        }
    }

    /// Why each of the registry's commands cannot run now (null: it can),
    /// as every menu that lists them greys them: Ask Kerf… with Kerf's
    /// access too (no key, signed out), and the picture's mask tools by
    /// their own ids ([`crate::commands::mask_tool_id`]).
    fn command_states(&self, cx: &App) -> serde_json::Value {
        let mut states = self.session.read(cx).command_states(crate::app::cloud(cx));
        let reason = self.mask_reason(cx);
        for tool in crate::stage::MaskTool::ALL {
            states[crate::commands::mask_tool_id(tool)] =
                reason.map_or(serde_json::Value::Null, serde_json::Value::from);
        }
        states
    }

    /// Why no mask tool can be taken out now, as the picture says it; None
    /// while one is out, or the chosen clip is a picture clip the picture
    /// places where it can be seen, on the program shown
    /// ([`StageView::can_draw`], Qt's invertible placement).
    fn mask_reason(&self, cx: &App) -> Option<&'static str> {
        let stage = self.stage.read(cx);
        if stage.mask_tool().is_some() {
            return None;
        }
        (!stage.can_draw(cx)).then_some(crate::commands::NO_MASK_CLIP)
    }

    /// The keys settle once this frame is drawn: what folds away now is
    /// still laid out where it was.
    fn settle_soon(&mut self, cx: &mut Context<Self>) {
        self.settle_soon = true;
        cx.notify();
    }

    pub fn session(&self) -> &Entity<Session> {
        &self.session
    }

    fn session_changed(
        &mut self,
        _: &Entity<Session>,
        change: &Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *change == Change::Snapshot {
            // Picture arrives: a project with clips on its timeline opens the
            // panel, the stage growing as the conversation gives way.
            let has_clips = self.session.read(cx).timeline().has_clips()
                || self
                    .session
                    .read(cx)
                    .state
                    .snapshot
                    .proposal_timeline
                    .is_some();
            // The window's title follows the project and its sequence.
            let title = {
                let session = self.session.read(cx);
                window_title(&session.project_name(), session.timeline())
            };
            if title != self.window_title {
                window.set_window_title(&title);
                self.window_title = title;
            }
            if has_clips && !self.panel_open && !self.panel_by_hand {
                // The picture grows out of what it came from.
                self.grow_for_cut(cx);
                window.refresh();
                // The timeline is in view: the keys go to it, unless
                // someone is typing.
                self.settle_focus(window, cx);
            }
            // What a cut being built placed flies to where it lands.
            self.follow_cut_build(cx);
            // The first snapshot puts the keys where they belong: on the
            // timeline with a cut, else in the composer to ask Kerf, unless
            // something holds them already.
            if !self.keys_found_their_place && self.modal.is_none() {
                self.keys_found_their_place = true;
                let free = window
                    .focused(cx)
                    .is_none_or(|focused| focused == self.focus_handle);
                if !has_clips && free {
                    self.chat
                        .update(cx, |chat, cx| chat.focus_composer(window, cx));
                }
            }
            // A folder that was no project yet: its media comes in now
            // that kerfd has the project.
            if let Some(folder) = self.start_media.take() {
                let files = folder_media(&folder);
                if !files.is_empty() {
                    self.session.update(cx, |session, cx| {
                        for file in &files {
                            session.import(file, true);
                        }
                        cx.notify();
                    });
                    if !self.media_open {
                        self.set_media(true, cx);
                    }
                }
            }
        }
        match change {
            // Back in touch with kerfd, the timeline answers its keys again.
            Change::Connection => self.settle_focus(window, cx),
            // Where Kerf works, for its dot when it ends unseen.
            Change::Work => {
                let session = self.session.read(cx);
                let work = &session.state.work;
                if work.busy() {
                    // A turn starts: what its cut has placed already is not new.
                    if self.working_in.is_none() {
                        self.live_seen = session
                            .live_cut()
                            .map(|(_, cut)| {
                                placed_since(session.timeline(), cut, &HashSet::new()).1
                            })
                            .unwrap_or_default();
                    }
                    self.working_in = work
                        .conversation
                        .clone()
                        .or_else(|| session.conversation.clone());
                }
            }
            // A turn that ends in the open conversation while the window is
            // in the back leaves its dot too, until the window comes to the
            // front on it.
            Change::TurnEnded => {
                self.live_seen.clear();
                if std::mem::take(&mut self.turn_rendered) {
                    self.go_home(cx);
                }
                let ended = self.working_in.take();
                if !window.is_window_active() {
                    if let Some(ended) = ended {
                        self.session.update(cx, |session, cx| {
                            if !session.state.settled.contains(&ended) {
                                session.state.settled.push(ended);
                                cx.notify();
                            }
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// Keeps the last error in its bar (what a view answers in place
    /// stays there); a failed turn's goes when the next turn starts.
    fn bridge_event(&mut self, event: &serde_json::Value, cx: &mut Context<Self>) {
        match event["type"].as_str().unwrap_or_default() {
            // The cut itself was rendered in this turn (not a range, a
            // draft or a proposal): it goes home when the turn ends.
            "agent" if event["kind"] == "tool" && event["rendered"].as_bool() == Some(true) => {
                self.turn_rendered = true;
            }
            // When the project was last edited, for the title's tooltip.
            "state" => {
                self.revision_time = event["snapshot"]["revisionTime"]
                    .as_str()
                    .and_then(sidebar::parse_time);
            }
            "error" => {
                let command = event["command"].as_str().unwrap_or_default();
                if !kept_in_bar(command) {
                    return;
                }
                let raw = event["message"]
                    .as_str()
                    .unwrap_or("Something went wrong")
                    .to_owned();
                // Saving over a preset of that name: the Save dialog asks
                // about it itself (the store kept the question).
                if crate::presets::existing_preset(&raw).is_some()
                    && self.session.read(cx).state.preset_exists.is_some()
                {
                    return;
                }
                // A file imported for a preset's slot, refused: said under
                // its slot in the dialog, not again here.
                if self.preset_importing(cx) && crate::presets::import_refusal(event).is_some() {
                    self.session.update(cx, |session, cx| {
                        session
                            .state
                            .notices
                            .retain(|notice| !(notice.error && notice.text == raw));
                        cx.notify();
                    });
                    return;
                }
                // Changing a conversation failed (renaming, archiving,
                // deleting, opening it): the notice says so, for Qt's 6 s,
                // and the conversations are listed again, so what was shown
                // at once goes back to what is kept (main_window_chat.cpp).
                if command == "conversation" {
                    self.session.update(cx, |session, cx| {
                        let said = session
                            .state
                            .notices
                            .iter_mut()
                            .rev()
                            .find(|notice| notice.error && notice.text == raw);
                        if let Some(notice) = said {
                            notice.text = conversation_error(&raw);
                            notice.hold = Some(scene::NOTICE_HOLD);
                        }
                        session
                            .send(serde_json::json!({"type": "conversations", "action": "list"}));
                        cx.notify();
                    });
                }
                let readable = readable_error(&raw);
                let text = if command == "forward" {
                    format!("Not brought forward · {readable}")
                } else {
                    readable
                };
                self.errors += 1;
                self.error = Some(ErrorBar {
                    raw,
                    text,
                    from_agent: command == "agent",
                    key: self.errors,
                });
                cx.notify();
            }
            // Connected again: what was refused while kerfd could not be
            // reached is not said over the new connection.
            "connection"
                if event["status"] == "ready"
                    && self
                        .error
                        .as_ref()
                        .is_some_and(|error| error.raw == DISCONNECTED) =>
            {
                self.error = None;
                cx.notify();
            }
            "agent_state"
                if event["state"] == "running"
                    && self.error.as_ref().is_some_and(|error| error.from_agent) =>
            {
                self.error = None;
                cx.notify();
            }
            // A render this window queued is answered here first; once it
            // is done, a frame of the picture goes home to the project.
            "export" => {
                let Some(job) = event["jobId"].as_str() else {
                    return;
                };
                match event["status"].as_str().unwrap_or_default() {
                    "queued" => {
                        self.queued_here.insert(job.to_owned());
                    }
                    "completed" | "complete" | "ready" => {
                        if self.queued_here.remove(job) && event["kind"] != "captions" {
                            self.fly_export_home(cx);
                        }
                    }
                    "failed" | "cancelled" | "canceled" => {
                        self.queued_here.remove(job);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Whether Import Editing Preset's dialog is open over the window with
    /// a file for one of its slots coming in.
    fn preset_importing(&self, cx: &App) -> bool {
        self.modal
            .as_ref()
            .and_then(|(view, _)| view.clone().downcast::<crate::presets::ApplyPreset>().ok())
            .is_some_and(|dialog| dialog.read(cx).importing())
    }

    /// Manual export: a frame of the picture flies from the stage to the
    /// project's row in the sidebar, fading as it goes. Not under Reduce
    /// Motion, while Kerf works, with no picture showing, or with the
    /// sidebar away (the row is not there to land on).
    fn fly_export_home(&mut self, cx: &mut Context<Self>) {
        if motion::reduced()
            || !self.sidebar_open
            || self.sidebar_gives_way
            || !self.panel_open
            || self.picture_shown.target() < 0.5
        {
            return;
        }
        let session = self.session.read(cx);
        if session.state.work.busy() {
            return;
        }
        let Some(picture) = session.picture.as_ref() else {
            return;
        };
        // From the picture where the stage placed it (else where it would
        // be in its slot), to the project's row where the sidebar laid it.
        let Some(from) = self.stage.read(cx).picture_bounds().or_else(|| {
            self.stage_slot
                .get()
                .map(|slot| picture_within(slot, picture.width, picture.height))
        }) else {
            return;
        };
        let to = self
            .sidebar
            .read(cx)
            .project_row_bounds()
            .map(landing_in)
            .unwrap_or_else(project_row);
        self.flights.push(Flight {
            image: picture.image.clone(),
            frame: true,
            from,
            to,
            started: Instant::now(),
            kind: FlightKind::Export,
        });
        cx.notify();
    }

    /// Going home: Kerf rendered the whole cut itself in the turn that
    /// ended, so the picture shrinks into the project's row in the sidebar
    /// and the panel goes, the conversation taking the room back, as Qt's
    /// goHome. Not under Reduce Motion (nothing moves; the panel stays), nor
    /// with no picture showing or the sidebar away (no row to go into).
    fn go_home(&mut self, cx: &mut Context<Self>) {
        if motion::reduced()
            || !self.sidebar_open
            || self.sidebar_gives_way
            || !self.panel_open
            || self.picture_shown.target() < 0.5
            || self.modal.is_some()
        {
            return;
        }
        let Some(row) = self.sidebar.read(cx).project_row_bounds() else {
            return;
        };
        let session = self.session.read(cx);
        let Some(picture) = session.picture.as_ref() else {
            return;
        };
        let Some(from) = self.stage.read(cx).picture_bounds().or_else(|| {
            self.stage_slot
                .get()
                .map(|slot| picture_within(slot, picture.width, picture.height))
        }) else {
            return;
        };
        let flight = Flight {
            image: picture.image.clone(),
            frame: true,
            from,
            to: landing_in(row),
            started: Instant::now(),
            kind: FlightKind::Home,
        };
        // What flew into the panel has nowhere to land now.
        self.let_flights_go(PaneGoes::Panel);
        self.flights.push(flight);
        // The panel goes as the picture does, and stays gone: the snapshot
        // the turn's end brings (and any after it) does not open it again,
        // as Qt's panes followed what came only once. Asked for, it comes.
        self.panel_open = false;
        self.panel_by_hand = true;
        self.panel_shown.go(0.0, scene::STAGE);
        if self.media_open {
            self.media_open = false;
            self.media_shown.go(0.0, scene::STAGE);
        }
        self.settle_soon(cx);
    }

    /// What flies this frame: each flight where it has got to and how
    /// clear, `to` following what it lands on (a clip where the timeline
    /// draws it now, the picture where it is to be, a card where the
    /// conversation lays it). One that has landed and faded goes; a frame
    /// it carried leaves the atlas unless the stage still shows it. Frames
    /// are asked for while any flies.
    fn flights_now(
        &mut self,
        picture_home: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Drawn> {
        let now = Instant::now();
        let mut drawn = Vec::new();
        // What was called off goes as what has landed does.
        let mut landed = std::mem::take(&mut self.called_off);
        // A clip of the cut has landed over the picture's stand-in.
        let covered = self.flights.iter().any(|flight| {
            matches!(flight.kind, FlightKind::Clip { .. })
                && now.saturating_duration_since(flight.started) >= scene::FLIGHT
        });
        for flight in std::mem::take(&mut self.flights) {
            // Where it lands, as that is now.
            let to = match &flight.kind {
                FlightKind::Clip { clip, .. } => self.timeline.read(cx).clip_bounds(clip),
                FlightKind::Grow { .. } => picture_home,
                FlightKind::Back { proposal } => self.chat.read(cx).cut_picture_bounds(proposal),
                FlightKind::Export | FlightKind::Home | FlightKind::LetGo { .. } => None,
            }
            .unwrap_or(flight.to);
            let mut flight = Flight { to, ..flight };
            self.hold_or_release(&mut flight, covered, cx);
            let Some(elapsed) = now.checked_duration_since(flight.started) else {
                // Not left yet: it waits its turn, unseen.
                window.request_animation_frame();
                self.flights.push(flight);
                continue;
            };
            match flight.kind.at(elapsed) {
                Some((along, opacity)) => {
                    window.request_animation_frame();
                    drawn.push(Drawn {
                        bounds: lerp_bounds(flight.from, flight.to, along),
                        opacity,
                        image: flight.image.clone(),
                        kind: flight.kind.clone(),
                    });
                    self.flights.push(flight);
                }
                None => landed.push(flight),
            }
        }
        for flight in landed {
            if !flight.frame {
                continue;
            }
            let shown = self
                .session
                .read(cx)
                .picture
                .as_ref()
                .is_some_and(|picture| Arc::ptr_eq(&picture.image, &flight.image));
            let flying = self
                .flights
                .iter()
                .any(|other| Arc::ptr_eq(&other.image, &flight.image));
            if !shown && !flying {
                cx.drop_image(flight.image, Some(window));
            }
        }
        drawn
    }

    /// The picture's stand-in, landed, is let go once its picture comes
    /// (the frame shown changed), a clip of the cut lands over it, nothing
    /// is coming any more (no render, Kerf not at work), or it has held for
    /// [`STAND_IN_MOST`].
    fn hold_or_release(&self, flight: &mut Flight, covered: bool, cx: &App) {
        let FlightKind::Grow { frame, released } = &mut flight.kind else {
            return;
        };
        if released.is_some() {
            return;
        }
        let elapsed = flight.started.elapsed();
        if elapsed < scene::STAGE {
            return;
        }
        let session = self.session.read(cx);
        let shown = session
            .picture
            .as_ref()
            .map(|picture| Arc::as_ptr(&picture.image) as usize);
        let came = shown.is_some() && shown != *frame;
        let coming = session.state.preview.rendering() || session.state.work.busy();
        if came || !coming || covered || elapsed >= scene::STAGE + STAND_IN_MOST {
            *released = Some(Instant::now());
        }
    }

    /// Where the picture is to be once the panel has come to rest: where
    /// the stage placed it last frame, moved by as far as the panel has
    /// still to come, else where the stage would place it in its slot.
    fn picture_home(
        &self,
        stage: Option<Bounds<Pixels>>,
        slot: Bounds<Pixels>,
        cx: &App,
    ) -> Bounds<Pixels> {
        let session = self.session.read(cx);
        let placed = stage.and(self.stage.read(cx).picture_bounds());
        let (drawn, rest) = self.panel_left;
        match placed {
            Some(picture) => Bounds::new(
                point(picture.origin.x + px(rest - drawn), picture.origin.y),
                picture.size,
            ),
            None => picture_in(slot, crate::stage::shown_aspect(session)),
        }
    }

    /// The picture comes in growing out of `origin` (a cut's card, a
    /// footage tile, over the composer), its stand-in `poster` flying from
    /// there to where the picture will be over `STAGE` as the panel comes
    /// in under it, unseen, and the conversation gives way; the timeline
    /// comes in after it. With nowhere to grow from, nothing to stand in,
    /// or under Reduce Motion, the panel simply comes in over `STAGE`.
    fn grow_picture(
        &mut self,
        origin: Option<Bounds<Pixels>>,
        poster: Option<Arc<RenderImage>>,
        stands_for: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let open = self.panel_open && self.picture_shown.target() > 0.5;
        if open {
            self.open_panel(true, true, scene::PANEL, cx);
            return;
        }
        let duration = if self.panel_open {
            scene::PANEL
        } else {
            scene::STAGE
        };
        self.open_panel(true, !self.panel_open, duration, cx);
        let (Some(origin), Some(poster), false) = (origin, poster, motion::reduced()) else {
            return;
        };
        let now = Instant::now();
        self.going_back = None;
        self.call_off_picture();
        let shown = self
            .session
            .read(cx)
            .picture
            .as_ref()
            .map(|picture| picture.image.clone());
        let frame = shown.as_ref().map(|image| Arc::as_ptr(image) as usize);
        // The stand-in is the frame shown when there was no poster for it.
        let is_frame = shown.is_some_and(|image| Arc::ptr_eq(&image, &poster));
        self.flights.push(Flight {
            image: poster,
            frame: is_frame,
            from: origin,
            // Where the picture will be follows as it is laid out.
            to: origin,
            started: now,
            kind: FlightKind::Grow {
                frame,
                released: None,
            },
        });
        self.grow = Some((now, stands_for));
        cx.notify();
    }

    /// A cut's card was pressed: the picture grows out of its picture.
    fn grow_from_card(&mut self, proposal: &str, window: &mut Window, cx: &mut Context<Self>) {
        let chat = self.chat.read(cx);
        let origin = chat.cut_picture_bounds(proposal);
        let poster = chat.cut_poster(proposal, cx).or_else(|| {
            self.session
                .read(cx)
                .picture
                .as_ref()
                .map(|picture| picture.image.clone())
        });
        self.grow_picture(origin, poster, None, cx);
        window.refresh();
    }

    /// A cut arrived with the panel closed: the picture grows out of what
    /// its first clip came from, its card where it shows, else its
    /// footage's tile (which it takes), else over the composer.
    fn grow_for_cut(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let cut = session.live_cut();
        let first = cut
            .map(|(_, cut)| cut)
            .into_iter()
            .chain(std::iter::once(session.timeline()))
            .find_map(first_picture_clip);
        let card = cut.and_then(|(proposal, _)| {
            let chat = self.chat.read(cx);
            chat.cut_picture_bounds(&proposal.proposal_id)
                .map(|bounds| (bounds, chat.cut_poster(&proposal.proposal_id, cx)))
        });
        let (origin, poster, stands_for) = match (card, first) {
            (Some((bounds, poster)), _) => (Some(bounds), poster, None),
            (None, Some((clip, source))) => {
                let poster = session.artwork.poster(&source);
                let tile = self.chat.read(cx).footage_tile_bounds(&source);
                let origin = tile.or_else(|| self.chat.read(cx).over_composer());
                if tile.is_some() {
                    self.chat
                        .update(cx, |chat, cx| chat.take_footage(&source, cx));
                }
                (origin, poster, Some(clip))
            }
            (None, None) => (None, None, None),
        };
        self.grow_picture(origin, poster, stands_for, cx);
    }

    /// The picture is put away while its cut's card is in sight: it goes
    /// back into the card's picture over `PANEL` as the panel goes, the
    /// stage not drawn meanwhile. Not under Reduce Motion.
    fn send_picture_back(&mut self, cx: &mut Context<Self>) {
        if motion::reduced() || !self.panel_open || self.picture_shown.target() < 0.5 {
            return;
        }
        let chat = self.chat.read(cx);
        let Some(proposal) = chat.stage_proposal().map(str::to_owned) else {
            return;
        };
        let Some(card) = chat.cut_picture_bounds(&proposal) else {
            return;
        };
        let poster = chat.cut_poster(&proposal, cx);
        let session = self.session.read(cx);
        let image = match (session.picture.as_ref(), poster) {
            (Some(picture), _) => Some((picture.image.clone(), true)),
            (None, poster) => poster.map(|poster| (poster, false)),
        };
        let (Some((image, frame)), Some(from)) = (image, self.stage.read(cx).picture_bounds())
        else {
            return;
        };
        let now = Instant::now();
        self.call_off_picture();
        self.grow = None;
        self.flights.push(Flight {
            image,
            frame,
            from,
            to: card,
            started: now,
            kind: FlightKind::Back { proposal },
        });
        self.going_back = Some(now);
        cx.notify();
    }

    /// The picture's stand-in and the picture going back into its card
    /// are called off, for another to take over.
    fn call_off_picture(&mut self) {
        let (off, on): (Vec<Flight>, Vec<Flight>) = std::mem::take(&mut self.flights)
            .into_iter()
            .partition(|flight| {
                matches!(
                    flight.kind,
                    FlightKind::Grow { .. } | FlightKind::Back { .. }
                )
            });
        self.flights = on;
        self.called_off.extend(off);
    }

    /// As `goes` goes from the panel, what was flying into it (a clip into
    /// the timeline, the picture's stand-in into the picture) has nowhere
    /// to land: each is let go where it has got to, fading there
    /// ([`let_go_flight_at`]), and one yet to leave does not. The picture
    /// going before it had grown, the stage is not drawn as it goes (the
    /// timeline still comes in as it was to).
    fn let_flights_go(&mut self, goes: PaneGoes) {
        let now = Instant::now();
        let mut stand_in = false;
        for flight in std::mem::take(&mut self.flights) {
            if flight.kind.flies_on(goes) {
                self.flights.push(flight);
                continue;
            }
            stand_in |= matches!(flight.kind, FlightKind::Grow { .. });
            let Some(elapsed) = now.checked_duration_since(flight.started) else {
                self.called_off.push(flight);
                continue;
            };
            match flight.kind.at(elapsed) {
                Some((along, clear)) => {
                    let here = lerp_bounds(flight.from, flight.to, along);
                    let was = Box::new(flight.kind.clone());
                    self.flights.push(Flight {
                        from: here,
                        to: here,
                        started: now,
                        kind: FlightKind::LetGo { was, clear },
                        ..flight
                    });
                }
                None => self.called_off.push(flight),
            }
        }
        if stand_in {
            self.going_back = Some(now);
        }
        if goes != PaneGoes::Picture {
            self.pending_clips.clear();
        }
    }

    /// As Kerf builds a cut, what it placed since it was last seen waits
    /// to fly from its footage tile to its clip ([`placed_since`]). What
    /// was there as the turn began is not new.
    fn follow_cut_build(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if !session.state.work.busy() {
            return;
        }
        let Some((_, cut)) = session.live_cut() else {
            return;
        };
        let (placed, seen) = placed_since(session.timeline(), cut, &self.live_seen);
        self.live_seen = seen;
        let now = Instant::now();
        let grown = self.grow.as_ref().and_then(|(_, clip)| clip.clone());
        self.pending_clips.extend(
            placed
                .into_iter()
                .filter(|placed| Some(&placed.clip) != grown.as_ref())
                .map(|placed| PendingClip { placed, asked: now }),
        );
    }

    /// The clips waiting to fly leave once the picture has grown and the
    /// timeline draws them, one after another `FLIGHT_STAGGER` apart: from
    /// the footage's tile (which they take), else over the composer. Ones
    /// the timeline does not draw in time, or with no poster, do not fly.
    fn launch_clips(&mut self, cx: &mut Context<Self>) {
        if self.pending_clips.is_empty() {
            return;
        }
        let growing = self
            .grow
            .as_ref()
            .is_some_and(|(since, _)| since.elapsed() < scene::STAGE);
        let in_view = self.panel_open
            && self.timeline_shown.target() > 0.5
            && self.panel_shown.target() > 0.5
            && !motion::reduced();
        if !in_view {
            self.pending_clips.clear();
            return;
        }
        if growing {
            return;
        }
        let now = Instant::now();
        let busy = self.session.read(cx).state.work.busy();
        let mut leaving = 0u32;
        for pending in std::mem::take(&mut self.pending_clips) {
            let placed = &pending.placed;
            let lane = self.timeline.read(cx).clip_bounds(&placed.clip);
            let poster = placed
                .source
                .as_deref()
                .and_then(|source| self.session.read(cx).artwork.poster(source));
            match (lane, poster) {
                (Some(lane), Some(poster)) => {
                    let tile = placed
                        .source
                        .as_deref()
                        .and_then(|source| self.chat.read(cx).footage_tile_bounds(source));
                    let Some(from) = tile.or_else(|| self.chat.read(cx).over_composer()) else {
                        continue;
                    };
                    if let (Some(source), true) = (placed.source.clone(), tile.is_some()) {
                        self.chat
                            .update(cx, |chat, cx| chat.take_footage(&source, cx));
                    }
                    self.flights.push(Flight {
                        image: poster,
                        frame: false,
                        from,
                        to: lane,
                        started: now + scene::FLIGHT_STAGGER * leaving,
                        kind: FlightKind::Clip {
                            clip: placed.clip.clone(),
                            name: placed.name.clone().into(),
                        },
                    });
                    leaving += 1;
                }
                // The timeline has yet to draw it: it waits while Kerf
                // works, and a little after.
                (None, Some(_)) if busy || pending.asked.elapsed() < scene::FLIGHT_WAIT => {
                    self.pending_clips.push(pending);
                }
                _ => {}
            }
        }
        if leaving > 0 || !self.pending_clips.is_empty() {
            cx.notify();
        }
    }

    /// How much of the stage and of the timeline is drawn now: both whole,
    /// but while the picture grows ([`grow_shows`]) or goes back into its
    /// card (the stage not at all, as the panel goes).
    fn panel_shows(&mut self) -> (f32, f32) {
        let growing = match &self.grow {
            Some((since, _))
                if since.elapsed() < scene::STAGE + scene::FLIGHT_LAND.max(scene::SETTLE) =>
            {
                Some(grow_shows(since.elapsed()))
            }
            _ => {
                self.grow = None;
                None
            }
        };
        if let Some(since) = self.going_back {
            if since.elapsed() < scene::PANEL {
                return (0.0, growing.map_or(1.0, |(_, lanes)| lanes));
            }
            self.going_back = None;
        }
        growing.unwrap_or((1.0, 1.0))
    }

    /// Whether Kerf works in conversation `id` now.
    fn conversation_working(&self, id: &str, cx: &App) -> bool {
        let session = self.session.read(cx);
        let work = &session.state.work;
        work.busy()
            && (work.conversation.as_deref() == Some(id)
                || (work.conversation.is_none() && session.conversation.as_deref() == Some(id)))
    }

    /// Asks, over the window, before deleting conversation `id`; not while
    /// Kerf works in it.
    fn confirm_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() || self.conversation_working(&id, cx) {
            return;
        }
        let Some(title) = self
            .session
            .read(cx)
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.label().to_owned())
        else {
            return;
        };
        let session = self.session.clone();
        let confirm = cx.new(|cx| {
            Confirm::new(
                cx,
                format!("Delete “{title}”?"),
                Some("Its messages are removed from this project. This can't be undone."),
                "Delete",
                true,
                false,
                move |_, cx| {
                    session.update(cx, |session, _| session.delete_conversation(&id));
                },
            )
        });
        self.open_modal(confirm, window, cx);
    }

    /// Renames the open conversation where it is named: in the sidebar
    /// while it shows, else in the toolbar.
    fn rename_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.read(cx).conversation.clone() else {
            return;
        };
        if self.sidebar_open {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.begin_rename(&id, window, cx));
        } else {
            self.begin_title_rename(window, cx);
        }
    }

    /// The toolbar's title becomes a field holding it.
    fn begin_title_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let Some(id) = session.conversation.clone() else {
            return;
        };
        let Some(title) = session
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.title.clone())
        else {
            return;
        };
        self.close_title_menu(window, cx);
        self.renaming_title = Some(id);
        // The whole title is chosen, to type over.
        self.title_field.update(cx, |field, cx| {
            field.set_text(title, cx);
            field.choose_all(cx);
        });
        self.title_field.read(cx).focus(window);
        cx.notify();
    }

    /// Ends the toolbar's rename: `keep` sends the new title (an empty or
    /// unchanged one is not sent).
    fn finish_title_rename(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(id) = self.renaming_title.take() else {
            return;
        };
        if keep {
            let typed = self.title_field.read(cx).text().to_owned();
            let old = self
                .session
                .read(cx)
                .state
                .conversations
                .iter()
                .find(|conversation| conversation.id == id)
                .map(|conversation| conversation.title.clone());
            if let Some(title) = old.and_then(|old| sidebar::renamed(&old, &typed)) {
                self.session
                    .update(cx, |session, _| session.rename_conversation(&id, &title));
            }
        }
        cx.notify();
    }

    /// What stands in the way, as the toolbar's line says it in red.
    fn standing_problem(&self, session: &Session) -> Option<String> {
        if !session.has_project() {
            return None;
        }
        match &session.state.connection {
            Connection::Failed(_) => Some("Offline".into()),
            Connection::Connecting => Some("Connecting…".into()),
            Connection::Connected => self
                .error
                .as_ref()
                .map(|error| first_line(&error.text).to_owned()),
        }
    }

    fn set_sidebar(&mut self, open: bool, cx: &mut Context<Self>) {
        self.sidebar_open = open;
        if let Some((menu, _)) = self.app_menu.take() {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_app_menu_open(false, cx));
            // It closes when it is next drawn without the window.
            drop(menu);
        }
        let shown = open && !self.sidebar_gives_way;
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_shown(shown, cx));
        self.sidebar_shown
            .go(if open { 1.0 } else { 0.0 }, scene::PANEL);
        self.session.update(cx, |session, _| {
            session.prefs.sidebar_open = open;
            session.save_prefs();
        });
        cx.notify();
    }

    /// The panel's button: both the picture and the timeline come in (Qt's
    /// setPanelOpen), or the panel goes with whatever it showed.
    pub fn set_panel(&mut self, open: bool, cx: &mut Context<Self>) {
        if open {
            self.open_panel(true, true, scene::PANEL, cx);
            return;
        }
        // The picture goes back into its cut's card, while that is in sight;
        // what else flew into the panel is let go where it is.
        self.send_picture_back(cx);
        self.let_flights_go(PaneGoes::Panel);
        self.panel_open = false;
        self.panel_by_hand = true;
        self.panel_shown.go(0.0, scene::PANEL);
        if self.media_open {
            self.media_open = false;
            self.media_shown.go(0.0, scene::PANEL);
        }
        self.settle_soon(cx);
    }

    /// Opens the panel over `duration` with what is asked for in it: with
    /// the panel closed only that comes in, and has the whole panel (Qt's
    /// revealPanel); open already, it joins what shows.
    fn open_panel(
        &mut self,
        picture: bool,
        timeline: bool,
        duration: Duration,
        cx: &mut Context<Self>,
    ) {
        let shown = (
            self.picture_shown.target() > 0.5,
            self.timeline_shown.target() > 0.5,
        );
        let (show_picture, show_timeline) =
            panes_opening(self.panel_open, shown, (picture, timeline));
        if !self.panel_open {
            // Coming in anew, nothing in it is growing or going back.
            self.grow = None;
            self.going_back = None;
        }
        for (tween, show, was) in [
            (&mut self.picture_shown, show_picture, shown.0),
            (&mut self.timeline_shown, show_timeline, shown.1),
        ] {
            let target = if show { 1.0 } else { 0.0 };
            if !self.panel_open {
                // It comes in as it will be: nothing in it moves meanwhile.
                tween.set(target);
            } else if show != was {
                tween.go(target, scene::PANEL);
            }
        }
        self.panel_open = true;
        self.panel_by_hand = false;
        self.panel_shown.go(1.0, duration);
        cx.notify();
    }

    fn set_media(&mut self, open: bool, cx: &mut Context<Self>) {
        self.media_open = open;
        if open && !self.panel_open {
            // With the panel closed, the panel comes in with the band in it,
            // the picture and the timeline with it.
            self.media_shown.set(1.0);
            self.open_panel(true, true, scene::PANEL, cx);
        } else {
            self.media_shown
                .go(if open { 1.0 } else { 0.0 }, scene::PANEL);
        }
        if !open {
            self.settle_soon(cx);
        }
        cx.notify();
    }

    /// Looking at something (a timecode, a Preview, a mask tool) opens the
    /// picture: with the panel closed, the picture alone.
    fn reveal_picture(&mut self, cx: &mut Context<Self>) {
        if !self.panel_open || self.picture_shown.target() < 0.5 {
            self.open_panel(true, false, scene::PANEL, cx);
        }
    }

    /// Takes out mask tool `tool` on the picture, or puts it down; the
    /// picture comes in for it. Not while something is open over the window.
    fn toggle_mask_tool(
        &mut self,
        tool: crate::stage::MaskTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal.is_some() {
            return;
        }
        let taken = self
            .stage
            .update(cx, |stage, cx| stage.toggle_tool(tool, window, cx));
        if taken && self.stage.read(cx).mask_tool().is_some() {
            self.reveal_picture(cx);
        }
    }

    /// One of the panel's two, the picture or the timeline, on or off;
    /// asked for with the panel closed, it comes in alone.
    fn toggle_pane(&mut self, picture: bool, cx: &mut Context<Self>) {
        if !self.panel_open {
            self.open_panel(picture, !picture, scene::PANEL, cx);
            return;
        }
        let (this, other) = if picture {
            (&self.picture_shown, &self.timeline_shown)
        } else {
            (&self.timeline_shown, &self.picture_shown)
        };
        let showing = this.target() > 0.5;
        if showing && other.target() < 0.5 {
            // The last of the two going closes the panel.
            self.set_panel(false, cx);
            return;
        }
        if picture && showing {
            self.send_picture_back(cx);
        }
        if showing {
            self.let_flights_go(if picture {
                PaneGoes::Picture
            } else {
                PaneGoes::Timeline
            });
        }
        let this = if picture {
            &mut self.picture_shown
        } else {
            &mut self.timeline_shown
        };
        this.go(if showing { 0.0 } else { 1.0 }, scene::PANEL);
        // The timeline gone gives the keys back; come, it takes them.
        self.settle_soon(cx);
    }

    /// Opens Settings over the window, on `page` (None: where Settings
    /// opens by itself, Chat). Open already, it goes to `page`; anything
    /// else open over the window gives way to it.
    pub fn show_settings(
        &mut self,
        page: Option<Page>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self
            .modal
            .as_ref()
            .and_then(|(view, _)| view.clone().downcast::<SettingsView>().ok());
        if let Some(settings) = open {
            if let Some(page) = page {
                settings.update(cx, |settings, cx| settings.show_page(page, cx));
            }
            return;
        }
        let session = self.session.clone();
        let settings = cx.new(|cx| match page {
            Some(page) => SettingsView::new_at(session, page, window, cx),
            None => SettingsView::new(session, window, cx),
        });
        cx.subscribe_in(
            &settings,
            window,
            |this, settings, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Dismiss => this.close_modal_of(settings.entity_id(), window, cx),
            },
        )
        .detach();
        window.focus(&settings.focus_handle(cx));
        self.put_modal(settings.into(), window, cx);
    }

    /// Goes into `view` over the window: the window dims at once and the
    /// view rises into its middle. It gives the window back by emitting
    /// [`DismissEvent`]. One is open at a time.
    pub fn open_modal<V>(&mut self, view: Entity<V>, window: &mut Window, cx: &mut Context<Self>)
    where
        V: Render + Focusable + EventEmitter<DismissEvent>,
    {
        cx.subscribe_in(&view, window, |this, view, _: &DismissEvent, window, cx| {
            this.close_modal_of(view.entity_id(), window, cx)
        })
        .detach();
        window.focus(&view.focus_handle(cx));
        self.put_modal(view.into(), window, cx);
    }

    fn put_modal(&mut self, view: AnyView, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menus(window, cx);
        self.modals_opened += 1;
        self.modal = Some((view, self.modals_opened));
        cx.notify();
    }

    /// Gives the window back from `view`, if it is what is open over the
    /// window still: one another has taken the place of (Settings opened
    /// from the palette) has nothing to give back.
    fn close_modal_of(
        &mut self,
        view: gpui::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .modal
            .as_ref()
            .is_some_and(|(open, _)| open.entity_id() == view)
        {
            self.modal = None;
            // What it held goes with it: the keys settle afresh.
            window.focus(&self.focus_handle);
            self.settle_focus(window, cx);
            cx.notify();
        }
    }

    fn toggle_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.export.take().is_some() {
            cx.notify();
            return;
        }
        let export = cx.new(|cx| ExportView::new(self.session.clone(), window, cx));
        cx.subscribe(&export, |this, _, event: &ExportEvent, cx| match event {
            ExportEvent::Dismiss => {
                this.export = None;
                cx.notify();
            }
        })
        .detach();
        self.session.update(cx, |session, _| {
            session.send(serde_json::json!({"type": "render_queue"}));
        });
        self.export = Some(export);
        cx.notify();
    }

    /// File ▸ Import Media…: the files chosen come into the project's media.
    /// No chooser to ask (no portal), or a name kerfd cannot be sent, says so.
    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("Import".into()),
            },
        );
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let answer = chosen.await;
            let _ = session.update(cx, |session, cx| {
                if let Some(paths) = chosen_paths(answer, session) {
                    for path in &paths {
                        session.import(path, false);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a folder in place of this project. One that is not a project
    /// yet is asked about first; this project is open already.
    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Open".into()),
            },
        );
        let current = self.session.read(cx).project.clone();
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let Ok(Some(paths)) = session.update(cx, |session, cx| {
                cx.notify();
                chosen_paths(answer, session)
            }) else {
                return;
            };
            let Some(project) = paths.into_iter().next() else {
                return;
            };
            if current
                .as_deref()
                .is_some_and(|current| crate::app::same_folder(current, &project))
            {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.open_chosen(project, window, cx));
        })
        .detach();
    }

    /// Opens `project` in place of this window: one that is not a project
    /// yet is asked about first; this window's own project stays as it is.
    fn open_chosen(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if needs_start(&project) {
            self.ask_to_start(project, window, cx);
        } else {
            self.replace_with(project, false, window, cx);
        }
    }

    /// Opens `project` in a window of its own with this window's kerfd,
    /// and closes this one: another project takes its place, as in Qt.
    /// This window's own project stays open.
    fn replace_with(
        &mut self,
        project: PathBuf,
        starts: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
        if session
            .project
            .as_deref()
            .is_some_and(|current| crate::app::same_folder(current, &project))
        {
            window.activate_window();
            return;
        }
        let kerfd = session.kerfd().map(Path::to_path_buf);
        open_folder_in(project, starts, kerfd, Some(window.window_bounds()), cx);
        window.remove_window();
    }

    /// New Project: the launcher, in a window of its own. What it opens
    /// takes this window's place, with this window's kerfd; closed with
    /// nothing chosen, this window stays.
    fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let kerfd = session.kerfd().map(Path::to_path_buf);
        let current = session.project.clone();
        let replaced = window.window_handle();
        let prefs = crate::prefs::Prefs::load();
        // Over this window, where it is; what it opens takes this window's place.
        let options = crate::app::launcher_options(Some(window.window_bounds()), cx);
        let opened = cx.open_window(options, move |launcher_window, cx| {
            // What it makes from a preset brings its media in with this kerfd too.
            let launcher = cx.new(|cx| {
                crate::launcher::LauncherView::new(prefs, launcher_window, cx)
                    .with_kerfd(kerfd.clone())
            });
            let handle = launcher_window.window_handle();
            cx.subscribe(
                &launcher,
                move |_, event: &crate::launcher::LauncherEvent, cx| match event {
                    crate::launcher::LauncherEvent::Open { project, starts } => {
                        let here = current
                            .as_deref()
                            .is_some_and(|current| crate::app::same_folder(current, project));
                        if !here {
                            let place = replaced
                                .update(cx, |_, window, _| window.window_bounds())
                                .ok();
                            open_folder_in(project.clone(), *starts, kerfd.clone(), place, cx);
                            let _ = replaced.update(cx, |_, window, _| window.remove_window());
                        } else {
                            let _ = replaced.update(cx, |_, window, _| window.activate_window());
                        }
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    }
                },
            )
            .detach();
            launcher
        });
        if let Err(error) = opened {
            log::error!("could not open the launcher: {error}");
        }
    }

    /// "“X” is not a project yet. Start one here, with its media?"
    fn ask_to_start(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        let name = project
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| project.display().to_string());
        let kerfd = self.session.read(cx).kerfd().map(Path::to_path_buf);
        let confirm = cx.new(|cx| {
            Confirm::new(
                cx,
                format!("“{name}” is not a project yet."),
                Some("Start one here, with its media?"),
                "Start",
                false,
                true,
                move |window, cx| {
                    open_folder_in(project, true, kerfd, Some(window.window_bounds()), cx);
                    window.remove_window();
                },
            )
        });
        self.open_modal(confirm, window, cx);
    }

    /// Whether a dialog may open over the window now: nothing is open over
    /// it, or the palette is, which a command chosen in it replaces (its
    /// command comes before it has gone).
    fn modal_gives_way(&self) -> bool {
        self.modal.as_ref().is_none_or(|(view, _)| {
            view.clone()
                .downcast::<crate::palette::CommandPalette>()
                .is_ok()
        })
    }

    /// Edit ▸ Focus Region…: its dialog over the window for the chosen
    /// picture clip; while a proposal waits, Qt's word that it comes first.
    fn focus_region(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal_gives_way() {
            return;
        }
        match crate::focus::target(self.session.read(cx)) {
            Ok(target) => {
                let session = self.session.clone();
                let dialog =
                    cx.new(|cx| crate::focus::FocusRegion::new(session, target, window, cx));
                self.open_modal(dialog, window, cx);
            }
            Err(words) => self.session.update(cx, |session, cx| {
                session.state.notify(words, false);
                cx.notify();
            }),
        }
    }

    /// Asks over the window for the words sync command `id` needs (a
    /// commit's message, a branch's name, a pull request's title, the
    /// GitHub repository's name), then runs it with them.
    fn ask_sync_words(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal_gives_way() {
            return;
        }
        let session = self.session.read(cx);
        let branch = session
            .state
            .collab
            .as_ref()
            .and_then(|collab| collab["branch"].as_str())
            .unwrap_or_default()
            .to_owned();
        let folder = session.project_name();
        let Some(asked) = sync_question(id, &branch, &folder) else {
            return;
        };
        let session = self.session.clone();
        let command = id.to_owned();
        let view = cx.new(|cx| {
            AskWords::new(asked, window, cx, move |words, cx| {
                let params = sync_params(&command, words);
                session.update(cx, |session, cx| {
                    session.run_command(&command, params);
                    cx.notify();
                });
            })
        });
        self.open_modal(view, window, cx);
    }

    /// The pinned proposal (the card the conversation pins) is what the
    /// registry's proposal commands act on and are said to be available for.
    /// Opens `media`'s shot graph over the window (Open Graph, a clip's
    /// double-click, `--graph`).
    pub fn open_shot_graph(
        &mut self,
        media: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let graph =
            cx.new(|cx| crate::making::ShotGraph::new(self.session.clone(), media, window, cx));
        self.open_modal(graph, window, cx);
    }

    /// Where the Ask popover stands: over the chosen clip while the
    /// timeline shows it, else over the lanes at the playhead, else (no
    /// timeline in view) over the middle of the window.
    fn ask_anchor(&self, window: &Window, cx: &App) -> Bounds<Pixels> {
        let timeline_shown = self.panel_open && self.timeline_shown.target() > 0.5;
        if timeline_shown {
            let timeline = self.timeline.read(cx);
            let chosen = self.session.read(cx).selection.first().cloned();
            if let Some(bounds) = chosen.and_then(|clip| timeline.clip_bounds(&clip)) {
                return bounds;
            }
            if let Some(bounds) = timeline.playhead_anchor(cx) {
                return bounds;
            }
        }
        let viewport = window.viewport_size();
        Bounds::new(
            point(viewport.width / 2.0, viewport.height / 2.0),
            size(px(1.0), px(1.0)),
        )
    }

    fn pin_proposal(&mut self, cx: &mut Context<Self>) {
        let pinned = self.chat.read(cx).proposal_target("proposal.reject", cx);
        self.session
            .update(cx, |session, _| session.pinned_proposal = pinned);
    }

    /// Why command `id` cannot run now, as the registry says; None when it
    /// can. Only Ask's reason asks after the connection's key (the keychain).
    fn unavailable(&mut self, id: &str, cx: &mut Context<Self>) -> Option<String> {
        self.pin_proposal(cx);
        let session = self.session.read(cx);
        let states = if id == "ai.ask" {
            let cloud = crate::app::cloud(cx).cloned();
            session.command_states(cloud.as_ref())
        } else {
            kerf_bridge::commands_states(&session.command_context())
        };
        crate::palette::unavailable(&states, id).map(str::to_owned)
    }

    /// Carries out a command by its id: what the window does itself, else
    /// the request the registry resolves it to. One that cannot run now
    /// says why, and its key goes on to whatever else it means there.
    fn run_command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let id = action.id.as_ref();
        self.refresh_keys(cx);
        if let Some(reason) = self.unavailable(id, cx) {
            // Esc puts things away everywhere: saying on each press that
            // there is no proposal to reject would be noise.
            if !self.bound_to_escape(id) {
                self.session.update(cx, |session, cx| {
                    session.state.notify(&reason, true);
                    cx.notify();
                });
            }
            cx.propagate();
            return;
        }
        let session = self.session.clone();
        match id {
            "app.quit" => cx.quit(),
            "app.preferences" => self.show_settings(None, window, cx),
            "app.account" => self.show_settings(Some(Page::Account), window, cx),
            "view.sidebar" => self.set_sidebar(!self.sidebar_open, cx),
            "view.picture" => self.toggle_pane(true, cx),
            "view.timeline" => self.toggle_pane(false, cx),
            "window.project" => self.set_media(!self.media_open, cx),
            "window.assistant" => self
                .chat
                .update(cx, |chat, cx| chat.focus_composer(window, cx)),
            // Ask Kerf…: its popover over the chosen clip, else over the
            // timeline at the playhead (Qt's askAboutSelection).
            "ai.ask" => {
                let anchor = self.ask_anchor(window, cx);
                self.chat
                    .update(cx, |chat, cx| chat.open_ask(anchor, window, cx));
            }
            "file.import" => self.import(window, cx),
            "file.openProject" => self.open_project(window, cx),
            // The launcher, in a window of its own; what it opens takes this one's place.
            "file.newProject" => self.new_project(window, cx),
            // The quality chosen in Export's menu, full size or a preview,
            // where kerfd puts it (Qt's File ▸ Export…); the toolbar's
            // Export renders it into deliveries/ (`ExportChosen`).
            "file.export" => session.update(cx, |session, _| session.export_file()),
            "export.master" => session.update(cx, |session, _| {
                session.deliver(Quality::Master.wire());
            }),
            "file.refresh" => session.update(cx, |session, _| {
                session.send(serde_json::json!({"type": "refresh"}));
            }),
            "edit.undo" => session.update(cx, |session, _| session.undo()),
            "edit.redo" => session.update(cx, |session, _| session.redo()),
            "playback.toggle" => {
                // A skim ends first, so kerfd is back at the playhead before it plays.
                self.stage.update(cx, |stage, cx| stage.end_skim(cx));
                session.update(cx, |session, _| session.toggle_playback());
            }
            "playback.goToStart" => session.update(cx, |session, _| session.seek(0.0, false)),
            "playback.goToEnd" => session.update(cx, |session, _| {
                let end = session.state.program_length();
                session.seek(end, false);
            }),
            "playback.previousFrame" | "playback.nextFrame" => session.update(cx, |session, _| {
                let frames = if id == "playback.nextFrame" { 1 } else { -1 };
                let at = frame_step(session.state.position, session.timeline().rate(), frames);
                session.seek(at, false);
            }),
            // As the conversation's Stop: its work row says "Stopping…" at once.
            "ai.stop" => self.chat.update(cx, |chat, cx| chat.stop(cx)),
            // As the pinned card's own buttons: Accept takes the version it
            // shows, Reject and Bring Forward the proposal.
            "proposal.accept" | "proposal.reject" | "proposal.forward" => {
                let Some(proposal) = self.chat.read(cx).proposal_target(id, cx) else {
                    return;
                };
                session.update(cx, |session, _| match id {
                    "proposal.accept" => session.accept(&proposal),
                    "proposal.reject" => session.reject(&proposal),
                    _ => session.forward(&proposal),
                })
            }
            "clip.startPath" => {
                let started = self.stage.update(cx, |stage, cx| stage.start_path(cx));
                if !started {
                    session.update(cx, |session, _| {
                        session
                            .state
                            .notify("Choose a picture clip with no motion path", true)
                    });
                }
            }
            // Editing presets: Import asks for the file, then its dialog
            // comes over the window (through this entity: the window is
            // being updated now); Export asks where; Save asks the name.
            "file.applyPreset" => {
                let workspace = cx.entity().downgrade();
                crate::presets::import_preset(session, window, cx, move |view, window, cx| {
                    let _ = workspace.update(cx, |this, cx| this.open_modal(view, window, cx));
                });
            }
            "file.exportPreset" => crate::presets::export_preset(session, window, cx),
            "preset.save" => {
                let view = crate::presets::save_preset(session, window, cx);
                self.open_modal(view, window, cx);
            }
            // The zoom's menu over the transport (⇧Z, View ▸ Viewer Zoom…).
            "viewer.zoom" => {
                self.reveal_picture(cx);
                self.stage
                    .update(cx, |stage, cx| stage.toggle_zoom_menu(window, cx));
            }
            "help.guide" => cx.open_url(&user_guide()),
            "help.about" => self.show_settings(Some(Page::About), window, cx),
            // Where crash reports are kept, in the system's file manager.
            "help.crashReports" => match kerf_bridge::crash_reports()["folder"].as_str() {
                Some(folder) => {
                    let folder = PathBuf::from(folder);
                    let _ = std::fs::create_dir_all(&folder);
                    cx.reveal_path(&folder);
                }
                None => session.update(cx, |session, _| {
                    session.state.notify(
                        "There is nowhere to keep crash reports on this computer",
                        true,
                    )
                }),
            },
            "mark.in" => session.update(cx, |session, _| {
                let at = session.state.position;
                session.set_mark_in(Some(at));
            }),
            "mark.out" => session.update(cx, |session, _| {
                let at = session.state.position;
                session.set_mark_out(Some(at));
            }),
            "mark.clear" => session.update(cx, |session, _| {
                session.set_mark_in(None);
                session.set_mark_out(None);
            }),
            "view.safeZones" => session.update(cx, |session, _| {
                session.prefs.safe_zones = !session.prefs.safe_zones;
                session.save_prefs();
            }),
            "playback.loop" => session.update(cx, |session, _| session.toggle_loop()),
            "view.followKerf" => session.update(cx, |session, _| {
                session.prefs.follow_kerf = !session.prefs.follow_kerf;
                session.save_prefs();
            }),
            "qc.run" => {
                window.dispatch_action(Box::new(crate::finish::OpenFinish { run_check: true }), cx)
            }
            // The timeline's own, from the menus and the palette (its keys
            // reach it first while it has them).
            tool if crate::timeline::Tool::from_command(tool).is_some() => {
                if let Some(tool) = crate::timeline::Tool::from_command(tool) {
                    self.timeline
                        .update(cx, |timeline, cx| timeline.set_tool(tool, cx));
                }
            }
            "sequence.snap" => self.timeline.update(cx, |timeline, cx| {
                let on = !timeline.snapping();
                timeline.set_snapping(on, cx)
            }),
            "view.fitTracks" => self
                .timeline
                .update(cx, |timeline, cx| timeline.fit_tracks(cx)),
            "view.resetTrackHeights" => self
                .timeline
                .update(cx, |timeline, cx| timeline.reset_track_heights(cx)),
            // Its two choices open under the tracks, the timeline brought
            // in for them.
            "timeline.addTrack" => {
                if !self.panel_open || self.timeline_shown.target() < 0.5 {
                    self.open_panel(false, true, scene::PANEL, cx);
                }
                self.timeline
                    .update(cx, |timeline, cx| timeline.open_add_track(window, cx));
            }
            // The Media band's search, the band brought in for it; its view.
            "bin.search" => {
                if !self.media_open {
                    self.set_media(true, cx);
                }
                self.media
                    .update(cx, |band, cx| band.focus_search(window, cx));
            }
            "bin.toggleView" => self.media.update(cx, |band, cx| band.toggle_view(cx)),
            // The toolbar's format: the sequence becomes that shape.
            "sequence.setFormat.169" | "sequence.setFormat.916" => {
                let index = usize::from(id == "sequence.setFormat.916");
                self.formats
                    .update(cx, |formats, cx| formats.choose(index, cx));
            }
            // Edit ▸ Focus Region…: its dialog over the window.
            "clip.focusRegion" => self.focus_region(window, cx),
            // What the sync needs words for asks for them first, as Qt did,
            // in the branch chip's menu (Publish's ways, a pull request's
            // description); Switch Branch opens it on the branches. Until
            // the project's Git status is known, a line of words is asked
            // for over the window.
            "collab.commit"
            | "collab.newBranch"
            | "collab.pullRequest"
            | "collab.publish"
            | "collab.switchBranch" => {
                let opened = self
                    .collab
                    .update(cx, |collab, cx| collab.open_command(id, window, cx));
                if !opened {
                    if id == "collab.switchBranch" {
                        session.update(cx, |session, _| {
                            session
                                .state
                                .notify("Reading the project's Git status", false)
                        });
                    } else {
                        self.ask_sync_words(id, window, cx);
                    }
                }
            }
            // As the chip's Sync Now and Live Sync: the sync's end is said,
            // and Live Sync is remembered for the project.
            "collab.sync" => self.collab.update(cx, |collab, cx| collab.sync_now(cx)),
            "collab.live" => self.collab.update(cx, |collab, cx| collab.toggle_live(cx)),
            _ => session.update(cx, |session, _| {
                session.run_command(id, serde_json::json!({}));
            }),
        }
        cx.notify();
    }

    /// The toolbar's Export (its main part): renders the quality chosen in
    /// its menu into deliveries/. While Kerf cannot export it says why, as
    /// Export Master would.
    fn export_chosen(&mut self, cx: &mut Context<Self>) {
        if let Some(reason) = self.unavailable("export.master", cx) {
            self.session.update(cx, |session, cx| {
                session.state.notify(&reason, true);
                cx.notify();
            });
            return;
        }
        self.session.update(cx, |session, cx| {
            let quality = session.export_quality;
            session.export(quality.wire());
            cx.notify();
        });
    }

    /// Whether `id` answers plain Esc in the keymap in use.
    fn bound_to_escape(&self, id: &str) -> bool {
        self.keys.1.get(id).is_some_and(|keys| {
            keys.iter()
                .any(|key| crate::commands::keystroke(key).as_deref() == Some("escape"))
        })
    }

    /// Reads each command's keys again when the keymap has changed.
    fn refresh_keys(&mut self, cx: &App) {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 == keymap {
            return;
        }
        let keys = crate::commands::catalog(keymap)
            .into_iter()
            .map(|command| (command.id, command.keys))
            .collect();
        self.keys = (keymap.to_owned(), keys);
    }

    /// Command `id`'s first key as the system writes it ("⌘E", "Ctrl+E").
    fn shown_key(&self, id: &str) -> Option<String> {
        self.keys
            .1
            .get(id)
            .and_then(|keys| keys.first())
            .map(|key| crate::commands::shown_keys(key))
    }

    /// Esc in the window (outside a field, the timeline, and whatever is
    /// open over the window): stops the preview render its card shows,
    /// else puts the Media band away.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_menus(window, cx) {
            return;
        }
        if self.render_card_shows(cx) {
            self.stop_render(cx);
        } else if self.media_open {
            self.set_media(false, cx);
        }
    }

    /// Whether the preview render's card shows over the picture: the stage
    /// draws it ([`StageView::render_card_shown`]) and the picture is in.
    fn render_card_shows(&self, cx: &App) -> bool {
        self.panel_open
            && self.picture_shown.target() > 0.5
            && self.stage.read(cx).render_card_shown(cx)
    }

    /// Where the keys are (`stack`, their key contexts) and what stands
    /// open, as Esc is pressed ([`escape_goes`]).
    fn escape_at(&self, stack: &[gpui::KeyContext], cx: &App) -> EscapeAt {
        let has = |name: &str| stack.iter().any(|context| context.contains(name));
        EscapeAt {
            menu_has_keys: has(ui::MENU),
            menu_open: self.title_menu.is_some()
                || self.app_menu.is_some()
                || self.activity_open
                || self.sidebar.read(cx).menu_open(),
            in_field: has(crate::input::CONTEXT),
            // The chip's questions were Qt's modal dialogs: their own Close
            // has Esc, keys in them or not.
            in_dialog: self.modal.is_some()
                || self.export.is_some()
                || ui::panel_of(stack).is_some()
                || has(crate::palette::CONTEXT)
                || has(CONFIRM)
                || self.collab.read(cx).asking(cx),
            on_picture: has(crate::stage::CONTEXT),
            rendering: self.render_card_shows(cx),
            band_open: self.media_open,
        }
    }

    /// Stops the preview rendering as the render card's Stop does: its card
    /// goes at once, done with until that render ends, as Qt's
    /// `AgentHud::stop` hid it ([`StageView::stop_render`]).
    fn stop_render(&mut self, cx: &mut Context<Self>) {
        self.stage.update(cx, |stage, cx| stage.stop_render(cx));
    }

    /// The app's menu as the window stands now ([`sidebar::app_rows`]).
    fn app_rows(&self, cx: &App) -> Vec<sidebar::AppRow> {
        let session = self.session.read(cx);
        let keymap = session.prefs.keymap;
        let commands = crate::commands::catalog(keymap.wire());
        let menus: Vec<String> = kerf_bridge::commands_catalog(keymap.wire())["menus"]
            .as_array()
            .map(|menus| {
                menus
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let states = self.command_states(cx);
        let checked = self.checked_commands(cx);
        let (_, _, signed_in) = sidebar::account_of(session, cx);
        // Why the last sync failed, until one succeeds: View ▸ Sync says so.
        let collab = sidebar::with_sync_failure(
            session.state.collab.as_ref(),
            self.collab.read(cx).failure(),
        );
        sidebar::app_rows(&sidebar::AppMenuInput {
            signed_in,
            keymap,
            commands: &commands,
            menus: &menus,
            states: &states,
            checked: &checked,
            zoom: self.stage.read(cx).zoom(),
            collab: collab.as_ref(),
            revision: session.revision(),
            // Only macOS draws a menu bar.
            menu_bar: cfg!(target_os = "macos"),
            recent: &session.prefs.recent,
            current: session.project.as_deref(),
        })
    }

    /// Opens the app's menu above the sidebar's account (`from_account`)
    /// or under the toolbar's ⋯, its first row lit when opened from the
    /// keys; open, it goes.
    fn toggle_app_menu(
        &mut self,
        from_account: bool,
        from_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.app_menu.is_some() {
            self.close_app_menu(window, cx);
            return;
        }
        self.close_title_menu(window, cx);
        self.activity_open = false;
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.close_menus(cx));
        let items = sidebar::app_menu_items(&self.app_rows(cx));
        let (title, _, _) = sidebar::account_of(self.session.read(cx), cx);
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx)
                .header(title)
                .width(APP_MENU_WIDTH);
            if from_keys {
                menu.from_keys()
            } else {
                menu
            }
        });
        cx.subscribe_in(&menu, window, |this, menu, _: &DismissEvent, _, cx| {
            if this.app_menu.as_ref().is_some_and(|(open, _)| open == menu) {
                this.app_menu = None;
                this.sidebar
                    .update(cx, |sidebar, cx| sidebar.set_app_menu_open(false, cx));
                cx.notify();
            }
        })
        .detach();
        self.app_menu = Some((menu, from_account));
        self.app_menu_tall.set(None);
        self.app_menu_scrolled.set(0.0);
        if from_account {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_app_menu_open(true, cx));
        }
        cx.notify();
    }

    /// Puts the app's menu away, the keys back where they were.
    fn close_app_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((menu, _)) = self.app_menu.take() {
            menu.update(cx, |menu, cx| menu.close(window, cx));
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_app_menu_open(false, cx));
            cx.notify();
        }
    }

    /// View ▸ Keyboard: the editor answers to `keymap`'s keys from now on,
    /// in every window (the menus and the tooltips follow).
    fn choose_keymap(&mut self, keymap: crate::prefs::Keymap, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.prefs.keymap = keymap;
            session.save_prefs();
            cx.notify();
        });
        // Binding the keymap clears every binding; it binds the views' own again.
        crate::commands::bind(keymap.wire(), cx);
        let project = self.session.read(cx).project.clone();
        crate::commands::refresh_menus(project.as_deref(), cx);
        self.refresh_keys(cx);
        cx.notify();
    }

    /// Puts away whichever menu is open: the title's, Activity, the app
    /// menu (the keys going back to what had them), the sidebar's. Whether
    /// one was.
    fn close_menus(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mine = self.title_menu.is_some() || self.app_menu.is_some() || self.activity_open;
        if mine {
            self.close_title_menu(window, cx);
            self.activity_open = false;
            self.close_app_menu(window, cx);
            cx.notify();
        }
        let sidebar = self
            .sidebar
            .update(cx, |sidebar, cx| sidebar.close_menus(cx));
        mine || sidebar
    }

    fn sidebar_width(&self, session: &Session) -> f32 {
        session
            .prefs
            .sidebar_width
            .clamp(layout::SIDEBAR_LEAST, 900.0)
    }

    fn toolbar(&self, leading: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let project = session.project_name();
        // The title is the open conversation's name, the project after it;
        // with none open, "New conversation" (or the first line of the
        // request under way), as Qt's.
        let conversation = session.conversation.as_ref().and_then(|id| {
            session
                .state
                .conversations
                .iter()
                .find(|conversation| &conversation.id == id)
                .map(|conversation| conversation.label().to_owned())
                .or_else(|| {
                    session
                        .state
                        .open_conversation
                        .as_ref()
                        .filter(|conversation| &conversation.id == id)
                        .map(|conversation| conversation.title.trim().to_owned())
                        .filter(|title| !title.is_empty())
                })
        });
        let title = conversation.unwrap_or_else(|| session.state.conversation_title());
        let subtitle: Option<(String, Rgba)> = match self.standing_problem(session) {
            Some(problem) => Some((problem, theme::neg())),
            None => Some((project.clone(), theme::ink3())),
        };
        let follow = session.prefs.follow_kerf;
        // What renders, queued exports too ("Draft export · 42%").
        let rendering = running_words(&session.state.renders, &session.state.preview);
        // The title's tip: the project and its sequence after a
        // conversation's name, when it was edited and at which revision.
        let title_tip = title_tip(
            &self.window_title,
            true,
            session.revision(),
            self.revision_time,
            now_seconds(),
        );
        let quality = session.export_quality;
        // What can be done now, as the registry says it (none of these is
        // Ask, so the keychain is not asked).
        let states = kerf_bridge::commands_states(&session.command_context());
        let reason = |id: &str| crate::palette::unavailable(&states, id).map(str::to_owned);
        let (undo_reason, redo_reason, export_reason) = (
            reason("edit.undo"),
            reason("edit.redo"),
            reason("export.master"),
        );
        let tip = |words: &str, id: &str, why: Option<&String>| -> (String, Option<String>) {
            match why {
                Some(why) => (format!("{words} · {why}"), None),
                None => (words.to_owned(), self.shown_key(id)),
            }
        };
        let undo_tip = tip("Undo", "edit.undo", undo_reason.as_ref());
        let redo_tip = tip("Redo", "edit.redo", redo_reason.as_ref());
        // The master's key goes with it; a draft has none of its own.
        let export_tip = (
            crate::export::export_tip(quality, export_reason.as_deref()),
            (quality == Quality::Master && export_reason.is_none())
                .then(|| self.shown_key("export.master"))
                .flatten(),
        );
        let sidebar_key = self.shown_key("view.sidebar");
        let follow_key = self.shown_key("view.followKerf");
        let media_key = self.shown_key("window.project");
        let export_open = self.export.is_some();
        let app_menu_open = self.app_menu.as_ref().is_some_and(|(_, account)| !account);
        let renaming = self.renaming_title.is_some();
        let title_menu = self.title_menu.is_some();
        // At the window's leading edge on macOS, the window buttons' room.
        let lead = toolbar_places(leading, crate::app::window_buttons_room(window)).lead;
        div()
            .h(px(layout::TOOLBAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .pl(px(lead))
            .pr(px(8.0))
            .when(leading, |bar| {
                bar.child(
                    with_key(
                        icon_button("show-sidebar", Icon::Sidebar).tooltip("Sidebar"),
                        sidebar_key,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.set_sidebar(true, cx))),
                )
                // With the sidebar away, its account's menu opens from here.
                .child(
                    sidebar::trigger(
                        "toolbar-app-menu",
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.toggle_app_menu(false, false, window, cx)
                        }),
                    )
                    .child(
                        // Its menu says all the tooltip would, so the tooltip waits while it is open.
                        icon_button("toolbar-app-menu-button", Icon::More)
                            .when(!app_menu_open, |button| {
                                button.tooltip("Settings, account and more")
                            })
                            .on(app_menu_open)
                            // From the keys (the press is the trigger's).
                            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                                if event.is_keyboard() {
                                    this.toggle_app_menu(false, true, window, cx);
                                }
                            })),
                    ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .pl(px(8.0))
                    .min_w_0()
                    .child(if renaming {
                        // Renamed where it is named: the field takes the title's place.
                        div()
                            .w(px(TITLE_FIELD))
                            .h(px(theme::height::H28))
                            .flex()
                            .items_center()
                            .ml(px(-5.0))
                            .px(px(4.0))
                            .rounded(px(theme::radius::SMALL))
                            .bg(theme::card())
                            .border_1()
                            .border_color(theme::accent())
                            .child(self.title_field.clone())
                            .into_any_element()
                    } else {
                        // Pressed, what can be done with the conversation;
                        // pressed twice, its name becomes a field.
                        let title = toolbar_stop(
                            sidebar::trigger(
                                "toolbar-title",
                                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                    this.close_app_menu(window, cx);
                                    this.activity_open = false;
                                    this.sidebar
                                        .update(cx, |sidebar, cx| sidebar.close_menus(cx));
                                    if event.click_count >= 2 {
                                        this.close_title_menu(window, cx);
                                        this.begin_title_rename(window, cx);
                                    } else {
                                        this.toggle_title_menu(false, window, cx);
                                    }
                                    cx.notify();
                                }),
                            ),
                            &self.title_stop,
                            theme::radius::CONTROL,
                            true,
                            window,
                        )
                        // From the keys, its menu.
                        .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                            if event.is_keyboard() {
                                this.close_app_menu(window, cx);
                                this.toggle_title_menu(true, window, cx);
                                cx.notify();
                            }
                        }))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .min_w_0()
                        .h(px(theme::height::H28))
                        .pl(px(4.0))
                        .pr(px(2.0))
                        .ml(px(-4.0))
                        .rounded(px(theme::radius::CONTROL))
                        .cursor_pointer()
                        .when(title_menu, |title| title.bg(theme::raised()))
                        .hover(|title| title.bg(theme::raised()))
                        .child(ui::line(title, text::TITLE, theme::ink()))
                        .child(Icon::ChevronDown.view(14.0, theme::ink3()));
                        // Its tip, over all of it, so that it still gives way
                        // in a narrow window; its menu says all the tip
                        // would, so the tip waits while it is open.
                        title
                            .when(!title_menu && !title_tip.is_empty(), |title| {
                                title.child(div().absolute().inset_0().flex().flex_col().child(
                                    ui::tipped(
                                        "toolbar-title-tip",
                                        ui::Tip::new(title_tip.clone()),
                                        div().w_full().h(px(theme::height::H28)),
                                    ),
                                ))
                            })
                            .into_any_element()
                    })
                    .when_some(subtitle, |row, (words, color)| {
                        row.child(ui::line(words, text::CONTROL, color).min_w_0())
                    }),
            )
            // Its empty stretch: on macOS, where the toolbar is the title
            // bar, it moves the window and zooms it.
            .child(crate::app::title_bar_stretch(div().flex_1().h_full()))
            .child(self.formats.clone())
            .child(self.collab.clone())
            .when_some(rendering, |bar, rendering| {
                bar.child(
                    ui::chip(rendering, theme::activity::gpu())
                        .child(ui::dot(theme::activity::gpu(), 6.0)),
                )
            })
            .child(
                kit_tip(
                    icon_button("undo", Icon::Undo),
                    undo_tip,
                    undo_reason.is_some(),
                )
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("edit.undo")), cx)
                }),
            )
            .child(
                kit_tip(
                    icon_button("redo", Icon::Redo),
                    redo_tip,
                    redo_reason.is_some(),
                )
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("edit.redo")), cx)
                }),
            )
            .child(
                ui::button("finish", "Finish")
                    .small()
                    .ghost()
                    .icon(Icon::Check)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::finish::OpenFinish { run_check: false }),
                            cx,
                        )
                    }),
            )
            .child(self.export_button(
                quality,
                export_tip,
                export_reason.is_none(),
                export_open,
                window,
                cx,
            ))
            .child(
                // While Kerf works, the playhead goes where it looks.
                with_key(
                    icon_button("follow-kerf", Icon::Eye).tooltip("Follow Kerf"),
                    follow_key,
                )
                .on(follow)
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("view.followKerf")), cx)
                }),
            )
            .child(
                with_key(
                    icon_button("media", Icon::Media).tooltip("Media"),
                    media_key,
                )
                .on(self.media_open)
                .on_click(cx.listener(|this, _, _, cx| this.set_media(!this.media_open, cx))),
            )
            .child(
                icon_button("panel", Icon::Panel)
                    .tooltip("Picture and timeline")
                    .on(self.panel_open)
                    .on_click(cx.listener(|this, _, _, cx| this.set_panel(!this.panel_open, cx))),
            )
    }

    /// Export, a split button: its main part renders the quality chosen
    /// in one press ("Export Master"); its chevron opens the menu that
    /// chooses it, with the queue. Off, with the reason in its tooltip,
    /// while it cannot render.
    fn export_button(
        &self,
        quality: Quality,
        tip: (String, Option<String>),
        able: bool,
        open: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ink = if able { theme::ink() } else { theme::dis() };
        let main = toolbar_stop(
            div().id("export-main"),
            &self.export_focus,
            0.0,
            able,
            window,
        )
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(EXPORT_INNER))
        .px(px(10.0))
        .child(Icon::Export.view(14.0, ink))
        .child(ui::line("Export", text::CONTROL_STRONG, ink))
        .child(ui::line(
            quality.name(),
            text::CONTROL,
            if able { theme::ink3() } else { theme::dis() },
        ))
        .when(able, |main| {
            main.cursor_pointer()
                .hover(|main| main.bg(theme::hover2()))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::export::ExportChosen), cx)
                })
        });
        let chevron = toolbar_stop(
            div().id("export-options"),
            &self.export_options_focus,
            0.0,
            true,
            window,
        )
        .flex()
        .items_center()
        .justify_center()
        .w(px(EXPORT_CHEVRON))
        .h(px(EXPORT_INNER))
        .border_l_1()
        .border_color(theme::line2())
        .cursor_pointer()
        .when(open, |chevron| chevron.bg(theme::on()))
        .hover(|chevron| chevron.bg(theme::hover2()))
        .child(Icon::ChevronDown.view(14.0, theme::ink2()))
        .on_click(cx.listener(|this, _, window, cx| this.toggle_export(window, cx)));
        // Its menu says all the tip would: the tip waits while it is open.
        let chevron = if open {
            chevron.into_any_element()
        } else {
            ui::tipped(
                "export-options-tip",
                ui::Tip::new("Export options"),
                chevron,
            )
            .into_any_element()
        };
        div()
            .flex()
            .flex_none()
            .h(px(theme::height::H28))
            .rounded(px(theme::radius::CONTROL))
            .overflow_hidden()
            .bg(theme::raised())
            .border_1()
            .border_color(theme::line2())
            .child(ui::tipped(
                "export-tip",
                ui::Tip::new(tip.0).maybe_key(tip.1),
                main,
            ))
            .child(chevron)
    }

    /// The conversation's menu, under its title (the kit's): Rename,
    /// Archive, Delete…, Agent settings… with the model in use, the models
    /// to choose between, the playhead's stamp, Activity. Open, it goes.
    fn toggle_title_menu(&mut self, from_keys: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_menu.is_some() {
            self.close_title_menu(window, cx);
            return;
        }
        let items = self.title_menu_items(cx);
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(TITLE_MENU_WIDTH);
            if from_keys {
                menu.from_keys()
            } else {
                menu
            }
        });
        cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this.title_menu.as_ref() == Some(&menu) {
                this.title_menu = None;
                cx.notify();
            }
        })
        .detach();
        self.title_menu = Some(menu);
        cx.notify();
    }

    /// Puts the conversation's menu away, the keys back where they were.
    fn close_title_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.title_menu.take() {
            menu.update(cx, |menu, cx| menu.close(window, cx));
            cx.notify();
        }
    }

    fn title_menu_items(&self, cx: &mut Context<Self>) -> Vec<ui::MenuItem> {
        let this = cx.entity().downgrade();
        let session = self.session.read(cx);
        let id = session.conversation.clone();
        let connected = session.state.connection == Connection::Connected;
        let summary = id.as_ref().and_then(|id| {
            session
                .state
                .conversations
                .iter()
                .find(|conversation| &conversation.id == id)
        });
        let kept = summary.is_some() && connected;
        let archived = summary.is_some_and(|summary| summary.archived);
        let working = id
            .as_deref()
            .is_some_and(|id| self.conversation_working(id, cx));
        let can_delete = kept && !working;
        // The models the connection offers to choose between (Claude
        // Code's), and the one in use.
        let provider = session.prefs.provider;
        let configured = session.prefs.model().to_owned();
        let model = if configured.is_empty() {
            "default".to_owned()
        } else {
            configured.clone()
        };
        let choices = crate::chat::model_choices(provider, &configured);
        let choosable = choices.len() > 1 && provider == crate::prefs::Provider::ClaudeCode;
        // A request carries the playhead's time while the picture is open.
        let stamp_offered = self.panel_open;
        let stamped = session.prefs.send_playhead;
        let mut items = vec![
            ui::MenuItem::new("Rename").disabled(!kept).on_choose({
                let this = this.clone();
                move |window, cx| {
                    let _ = this.update(cx, |this, cx| this.begin_title_rename(window, cx));
                }
            }),
            ui::MenuItem::new(if archived { "Unarchive" } else { "Archive" })
                .icon(Icon::Archive)
                .disabled(!kept)
                .on_choose({
                    let session = self.session.clone();
                    let id = id.clone();
                    move |_, cx| {
                        if let Some(id) = id.clone() {
                            session.update(cx, |session, cx| {
                                session.archive_conversation(&id, !archived);
                                cx.notify();
                            });
                        }
                    }
                }),
            ui::MenuItem::new("Delete…")
                .icon(Icon::Trash)
                .danger()
                .disabled(!can_delete)
                .on_choose({
                    let this = this.clone();
                    let id = id.clone();
                    move |window, cx| {
                        if let Some(id) = id.clone() {
                            let _ = this.update(cx, |this, cx| this.confirm_delete(id, window, cx));
                        }
                    }
                }),
            ui::MenuItem::new("Agent settings…")
                .icon(Icon::Sparkle)
                .separated()
                // The model in use, where a menu writes a key.
                .key(crate::chat::model_label(&model))
                .on_choose({
                    let this = this.clone();
                    move |window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.show_settings(Some(Page::Chat), window, cx)
                        });
                    }
                }),
        ];
        if choosable {
            let rows = choices
                .into_iter()
                .map(|choice| {
                    let session = self.session.clone();
                    ui::MenuItem::new(crate::chat::model_label(&choice))
                        .checked(choice == model)
                        .on_choose(move |_, cx| {
                            session.update(cx, |session, cx| {
                                session.prefs.claude_code_model = choice.clone();
                                session.save_prefs();
                                cx.notify();
                            });
                        })
                })
                .collect();
            items.push(ui::MenuItem::new("Model").submenu(rows));
        }
        if stamp_offered {
            let session = self.session.clone();
            items.push(
                ui::MenuItem::new("Send playhead time")
                    .checked(stamped)
                    .on_choose(move |_, cx| {
                        session.update(cx, |session, cx| {
                            session.prefs.send_playhead = !session.prefs.send_playhead;
                            session.save_prefs();
                            cx.notify();
                        });
                    }),
            );
        }
        items.push(ui::MenuItem::new("Activity").on_choose(move |_, cx| {
            let _ = this.update(cx, |this, cx| {
                this.activity_open = true;
                cx.notify();
            });
        }));
        items
    }

    /// Activity, under the title: the newest of what the window and Kerf
    /// did, a line each. A press outside or Esc puts it away.
    fn activity_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lines: Vec<(String, String)> = self
            .session
            .read(cx)
            .activity
            .latest(crate::session::ACTIVITY_SHOWN)
            .map(|(kind, text)| (kind.to_uppercase(), text.to_owned()))
            .collect();
        sidebar::menu_panel()
            .w(px(ACTIVITY_WIDTH))
            .px(px(12.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.activity_open = false;
                cx.notify();
            }))
            .child(ui::line("Activity", text::META_STRONG, theme::ink3()))
            .when(lines.is_empty(), |panel| {
                panel.child(ui::label(
                    "Nothing yet: imports, exports and previews are listed here.",
                    text::CONTROL,
                    theme::ink3(),
                ))
            })
            .children(lines.into_iter().map(|(kind, words)| {
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        ui::line(kind, text::MONO_SMALL, theme::ink3())
                            .w(px(ACTIVITY_KIND))
                            .flex_none(),
                    )
                    .child(
                        ui::label(words, text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    )
            }))
            .relative()
            .with_animation("activity", motion::animation(scene::FADE), |panel, t| {
                panel.opacity(t).top(px(scene::MENU_RISE * (1.0 - t)))
            })
    }

    /// The bars under the toolbar, each until what it says is dealt with:
    /// kerfd gone, project.kerf changed on disk, the last error.
    fn bars(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let mut bars = Vec::new();
        if let (true, Connection::Failed(reason)) =
            (session.has_project(), &session.state.connection)
        {
            bars.push(bar(
                "bar-disconnected".into(),
                theme::neg(),
                format!("Project disconnected. {reason}").into(),
                None,
                vec![ui::button("bar-reconnect", "Reconnect")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.reconnect();
                            cx.notify();
                        })
                    }))
                    .into_any_element()],
                None,
            ));
        }
        if session.state.snapshot.projection_status == "diverged" {
            bars.push(bar(
                "bar-diverged".into(),
                theme::warn(),
                "project.kerf changed on disk.".into(),
                Some(
                    "Load it to bring in edits made outside Kerf (a git pull, a text editor), \
                     or keep the editor's version and write it over the file."
                        .into(),
                ),
                vec![
                    ui::button("bar-keep-editor", "Keep Editor Version")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(serde_json::json!({"type": "projection_repair"}));
                            })
                        }))
                        .into_any_element(),
                    ui::button("bar-load-disk", "Load from Disk")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(serde_json::json!({"type": "projection_import"}));
                            })
                        }))
                        .into_any_element(),
                ],
                None,
            ));
        }
        if let Some(error) = &self.error {
            // Its first line; the whole of a longer one (an import's ffmpeg
            // arguments and stderr) in its tooltip.
            let line = first_line(&error.text);
            let whole = (line != error.text.trim()).then(|| error.text.clone().into());
            bars.push(bar(
                format!("bar-error-{}", error.key).into(),
                theme::neg(),
                line.to_owned().into(),
                None,
                vec![ui::button("bar-dismiss", "Dismiss")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    }))
                    .into_any_element()],
                whole,
            ));
        }
        bars
    }

    /// The panel, `width` × `height`: the picture (and the inspector), the
    /// Media band, the timeline. `shows` is how much of the stage and of
    /// the timeline is drawn (both whole, but while the picture grows or
    /// goes back into its card).
    fn panel(
        &self,
        width: f32,
        height: f32,
        inspecting: bool,
        aspect: f32,
        shows: (f32, f32),
        window: &Window,
    ) -> AnyElement {
        let picture = self.picture_shown.sample(window);
        let timeline = self.timeline_shown.sample(window);
        let media = self.media_shown.sample(window);
        let stage_width = width - if inspecting { layout::INSPECTOR } else { 0.0 };
        let (picture_height, band, timeline_height) =
            panel_heights(height, stage_width, aspect, (picture, timeline, media));
        let (stage_shows, timeline_shows) = shows;
        div()
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(picture_height))
                    .w_full()
                    .flex_none()
                    .flex()
                    .overflow_hidden()
                    .when(stage_shows < 1.0, |row| row.opacity(stage_shows))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .relative()
                            .child(self.stage.clone())
                            // Where the stage is, for a frame to fly from.
                            .child({
                                let slot = self.stage_slot.clone();
                                canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {})
                                    .absolute()
                                    .inset_0()
                            }),
                    )
                    .when(inspecting, |row| {
                        row.child(
                            div()
                                .w(px(layout::INSPECTOR))
                                .flex_none()
                                .h_full()
                                .border_l_1()
                                .border_color(theme::line())
                                .child(self.inspector.clone()),
                        )
                    }),
            )
            .child(
                // The band comes down out from under the picture. Put away
                // and at rest, nothing in it is a Tab stop.
                div()
                    .h(px(band))
                    .w_full()
                    .flex_none()
                    .overflow_hidden()
                    .relative()
                    .when(!self.media_open && media <= 0.0, |band| band.invisible())
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(layout::MEDIA_BAND))
                            .child(self.media.clone()),
                    ),
            )
            .child(
                div()
                    .h(px(timeline_height))
                    .w_full()
                    .flex_none()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(theme::line())
                    .when(timeline_shows < 1.0, |lanes| lanes.opacity(timeline_shows))
                    .child(self.timeline.clone()),
            )
            .into_any_element()
    }

    fn notices(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut notices = self.session.read(cx).state.notices.clone();
        // What the error bar holds is said there, not again here.
        if let Some(error) = &self.error {
            notices.retain(|notice| !(notice.error && notice.text == error.raw));
        }
        let bottom = notice_bottom(
            f32::from(window.viewport_size().height),
            self.chat.read(cx).composer_top(),
        );
        div()
            .absolute()
            .bottom(px(bottom))
            .left_0()
            .right_0()
            .px(px(24.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.0))
            .children(notices.into_iter().map(|notice| {
                let color = if notice.error {
                    theme::neg()
                } else {
                    theme::pos()
                };
                let key = notice.id;
                theme::type_style(div(), text::CONTROL)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .max_w(px(layout::NOTICE_MOST))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(theme::radius::CARD))
                    .bg(theme::popup())
                    .border_1()
                    .border_color(theme::line2())
                    .shadow_lg()
                    .text_color(theme::ink())
                    .child(div().flex_none().child(ui::dot(color, 6.0)))
                    // Long words (a failed push's ssh line) wrap inside it.
                    .child(div().flex_1().min_w_0().child(notice.text.clone()))
                    .with_animation(
                        ("notice", key),
                        motion::animation(scene::APPEAR),
                        |notice, t| notice.opacity(t),
                    )
            }))
    }

    /// What is open over the window, on the dim. A panel of a known size
    /// (Settings, Finish) is placed by [`ui::modal_bounds`]: at most that
    /// size, else the window less `MODAL_MARGIN` all round, with
    /// `MODAL_ABOVE` of the height to spare above it. Any other keeps its
    /// own size, held to the same room and sat as high.
    fn modal(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (view, opened) = self.modal.as_ref()?;
        let key = *opened;
        let viewport = window.viewport_size();
        let most = modal_most(view, cx);
        let margin = layout::MODAL_MARGIN * 2.0;
        let panel = div()
            .relative()
            .rounded(px(theme::radius::PANEL))
            .overflow_hidden()
            .border_1()
            .border_color(theme::line2())
            .shadow_2xl()
            .child(view.clone());
        let panel = match most {
            Some(most) => {
                let placed = ui::modal_bounds(most, viewport);
                div()
                    .absolute()
                    .left(placed.origin.x)
                    .top(placed.origin.y)
                    .w(placed.size.width)
                    .h(placed.size.height)
                    .child(panel.size_full())
            }
            None => div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                // MODAL_ABOVE of the height to spare above it, the rest below.
                .child(spacer(layout::MODAL_ABOVE))
                .child(
                    panel
                        .flex_none()
                        .max_w(viewport.width - px(margin))
                        .max_h(viewport.height - px(margin)),
                )
                .child(spacer(1.0 - layout::MODAL_ABOVE)),
        };
        Some(
            div()
                .id("scrim")
                .absolute()
                .inset_0()
                // The window dims at once.
                .bg(theme::with_alpha(gpui::rgb(0x000000), scene::SCRIM))
                // Nothing under the dim answers the pointer, not even its hover.
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    // Its body comes up into place.
                    panel.with_animation(
                        ("settings-rise", key),
                        motion::animation(scene::PAGE),
                        |body, t| {
                            body.mt(px(scene::PAGE_RISE * (1.0 - t)))
                                .opacity(0.2 + 0.8 * t)
                        },
                    ),
                )
                .into_any_element(),
        )
    }
}

/// The most a panel that fills what it is given is over the window:
/// Settings and Finish, at their largest (`settings::MOST`, `finish::MOST`).
fn modal_most(view: &AnyView, _cx: &App) -> Option<gpui::Size<Pixels>> {
    let view = view.clone();
    let most = if view.clone().downcast::<SettingsView>().is_ok() {
        crate::settings::MOST
    } else if view.downcast::<crate::finish::FinishView>().is_ok() {
        crate::finish::MOST
    } else {
        return None;
    };
    Some(size(px(most.0), px(most.1)))
}

/// Room that grows by `weight` against the others in its column.
fn spacer(weight: f32) -> gpui::Div {
    let mut spacer = div().flex_basis(px(0.0)).min_h_0();
    spacer.style().flex_grow = Some(weight);
    spacer
}

/// The user guide the package carries (beside the editor on Windows, in the
/// app's Resources on macOS, in share/doc on Linux), else the one on GitHub.
fn user_guide() -> String {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    beside
        .and_then(|folder| user_guide_in(&folder))
        .unwrap_or_else(|| format!("{}/blob/main/docs/user-guide.md", kerf_bridge::REPOSITORY))
}

/// The packaged guide for an editor in `folder`, as a `file://` URL: beside
/// it (Windows), in the app's Resources (macOS), in share/doc (Linux). Only
/// a guide with something in it counts. None when there is none.
pub fn user_guide_in(folder: &Path) -> Option<String> {
    [
        folder.join("user-guide.html"),
        folder.join("../Resources/user-guide.html"),
        folder.join("../share/doc/industrial-kerf/user-guide.html"),
    ]
    .into_iter()
    .filter_map(|candidate| candidate.canonicalize().ok())
    .find(|path| {
        std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
    })
    .map(|path| file_url(&path))
}

/// A local file as a `file://` URL, as QUrl::fromLocalFile writes one: a
/// Windows path with forward slashes after a third, and every byte that
/// may not stand in a URL's path ("Industrial Kerf.app"'s space among them)
/// percent-encoded, so the system opens it whatever the folder is called.
pub fn file_url(path: &Path) -> String {
    let text = path.display().to_string();
    let text = text.trim_start_matches(r"\\?\").replace('\\', "/");
    let mut url = String::from(if text.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for byte in text.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'/' | b'-'
                    | b'.'
                    | b'_'
                    | b'~'
                    | b':'
                    | b'@'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
            );
        if keep {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_keys(cx);
        // Where the stage was laid out last frame, if it was: this frame
        // lays it out again, or not.
        let stage_drawn = self.stage_slot.take();
        let viewport = window.viewport_size();
        let (window_width, window_height) = (f32::from(viewport.width), f32::from(viewport.height));
        let sidebar_width = self.sidebar_width(self.session.read(cx));
        // In a window too narrow for it, the sidebar gives way (Qt's
        // sidebarWindowLeast), and the toolbar has its menu. Given way, it
        // is put away: its titles' light rests.
        let gives_way = window_width < layout::SIDEBAR_WINDOW_LEAST;
        if gives_way != self.sidebar_gives_way {
            let shown = self.sidebar_open && !gives_way;
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_shown(shown, cx));
        }
        self.sidebar_gives_way = gives_way;
        // A region folded away gave back the keys it held: they settle now
        // that it is laid out as it is.
        if std::mem::take(&mut self.settle_soon) {
            cx.defer_in(window, |this, window, cx| this.settle_focus(window, cx));
        }
        // The session knows whether the picture shows: a request is stamped
        // with the playhead, and a cut's card says "On the stage", only
        // while it does. However the panel came or went, it is told once.
        let picture = self.panel_open && self.picture_shown.target() > 0.5;
        if self.session.read(cx).picture_open != Some(picture) {
            let session = self.session.clone();
            cx.defer(move |cx| {
                session.update(cx, |session, cx| {
                    session.picture_open = Some(picture);
                    cx.notify();
                })
            });
        }
        // The menu bar greys and ticks the registry's rows as this window,
        // in front, has them.
        if cfg!(target_os = "macos") && window.is_window_active() {
            let states = self.menu_states(cx);
            let project = self.session.read(cx).project.clone();
            crate::commands::set_menu_states(states, project.as_deref(), cx);
        }
        let sidebar = if self.sidebar_gives_way {
            0.0
        } else {
            self.sidebar_shown.sample(window)
        };
        // Watching changes, the picture comes in and the stage reaches
        // over the conversation.
        let watching = self.session.read(cx).reel().is_some();
        if watching != (self.reel_reach.target() > 0.5) {
            self.reel_reach
                .go(if watching { 1.0 } else { 0.0 }, scene::PANEL);
            if watching {
                self.reveal_picture(cx);
            }
        }
        let reach = self.reel_reach.sample(window);
        let panel = self.panel_shown.sample(window);
        let main_left = sidebar_width * sidebar;
        let main_width = (window_width - main_left).max(0.0);
        let bars = self.bars(cx);
        let bars_height = BAR * bars.len() as f32;
        let content_height = (window_height - layout::TOOLBAR - bars_height).max(0.0);
        // The conversation keeps a rail beside an open panel.
        let rail = layout::RAIL
            .min((main_width * 0.45).max(layout::RAIL_LEAST))
            .min(main_width);
        let panel_width = (main_width - rail).max(0.0);
        let conversation_width = main_width - panel_width * panel;
        // How far the stage reaches over the conversation while changes are watched.
        let reach = REEL_REACH.min(conversation_width) * reach * panel;
        let leading = sidebar < 0.02;
        // The chosen clip's inspector stands beside the picture when there is room.
        let inspecting =
            !self.session.read(cx).selection.is_empty() && panel_width > layout::INSPECTOR * 2.2;
        // The picture's shape, which splits the panel.
        let aspect = crate::stage::shown_aspect(self.session.read(cx));
        // Where the title and ⋯ are, for their menus to open under them.
        let places = toolbar_places(leading, crate::app::window_buttons_room(window));
        let title_left = main_left + places.title;
        // Where the panel's left edge is drawn, and where it comes to rest;
        // where the picture will be there, for what grows into it.
        let panel_left = main_left + conversation_width - reach;
        let rest_left = main_left + main_width - panel_width;
        let picture_home = {
            let stage_width = panel_width - 8.0 - if inspecting { layout::INSPECTOR } else { 0.0 };
            let shown = |tween: &Tween| tween.target().clamp(0.0, 1.0);
            let (picture_height, _, _) = panel_heights(
                content_height - 8.0,
                stage_width,
                aspect,
                (
                    shown(&self.picture_shown),
                    shown(&self.timeline_shown),
                    shown(&self.media_shown),
                ),
            );
            let slot = Bounds::new(
                point(px(rest_left), px(layout::TOOLBAR + bars_height)),
                size(px(stage_width.max(1.0)), px(picture_height)),
            );
            self.picture_home(stage_drawn, slot, cx)
        };
        self.panel_left = (panel_left, rest_left);
        // What a cut being built placed flies once the timeline draws it.
        self.launch_clips(cx);
        if !self.pending_clips.is_empty() {
            window.request_animation_frame();
        }
        let flights = self.flights_now(Some(picture_home), window, cx);
        // While the picture grows the stage is not drawn until its
        // stand-in lands, and the timeline comes in after it; going back
        // into its card, the stage is not drawn.
        let shows = self.panel_shows();

        div()
            .key_context(WORKSPACE)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::run_command))
            .on_action(
                cx.listener(|this, _: &crate::export::ExportChosen, _, cx| this.export_chosen(cx)),
            )
            .on_action(
                cx.listener(|this, _: &crate::commands::Escape, window, cx| {
                    this.escape(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::commands::RevealPicture, _, cx| {
                    this.reveal_picture(cx)
                }),
            )
            // ⇧R ⇧E ⇧P wherever the window has the keys, as Qt's View menu
            // had them: a mask tool on the chosen picture clip.
            .on_action(
                cx.listener(|this, _: &crate::stage::RectangleMask, window, cx| {
                    this.toggle_mask_tool(crate::stage::MaskTool::Rectangle, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::stage::EllipseMask, window, cx| {
                    this.toggle_mask_tool(crate::stage::MaskTool::Ellipse, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &crate::stage::PenMask, window, cx| {
                this.toggle_mask_tool(crate::stage::MaskTool::Pen, window, cx)
            }))
            // The app's menu: out of the Kerf account, another keymap,
            // another branch, the viewer's zoom.
            .on_action(cx.listener(|this, _: &sidebar::SignOut, _, cx| {
                this.session.update(cx, |session, cx| {
                    session.cloud_sign(None);
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, action: &sidebar::ChooseKeymap, _, cx| {
                this.choose_keymap(action.keymap, cx)
            }))
            .on_action(cx.listener(|this, action: &sidebar::SwitchBranch, _, cx| {
                let name = action.name.to_string();
                this.session.update(cx, |session, cx| {
                    session.run_command("collab.switchBranch", serde_json::json!({"name": name}));
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, action: &crate::stage::SetZoom, _, cx| {
                this.reveal_picture(cx);
                let zoom = action.percent as f32 / 100.0;
                this.stage.update(cx, |stage, cx| stage.set_zoom(zoom, cx));
            }))
            .on_action(cx.listener(|this, action: &OpenRecent, window, cx| {
                this.open_chosen(PathBuf::from(action.path.as_ref()), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearRecent, _, cx| {
                this.session.update(cx, |session, _| {
                    session.prefs.recent.clear();
                    session.save_prefs();
                });
                let project = this.session.read(cx).project.clone();
                crate::commands::refresh_menus(project.as_deref(), cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::palette::OpenPalette, window, cx| {
                    if this.modal.is_none() {
                        // What the palette says of a proposal is of the pinned one.
                        this.pin_proposal(cx);
                        let checked = this.checked_commands(cx);
                        let palette = cx.new(|cx| {
                            crate::palette::CommandPalette::new(this.session.clone(), window, cx)
                                .with_checked(checked)
                        });
                        this.open_modal(palette, window, cx);
                    }
                }),
            )
            // Ask Kerf on a failed check (Finish's QC, Export's Preflight):
            // its words wait in the composer, as Qt's askPrefill.
            .on_action(
                cx.listener(|this, action: &crate::finish::AskKerf, window, cx| {
                    let words = action.words.to_string();
                    this.chat
                        .update(cx, |chat, cx| chat.fill(words, window, cx));
                }),
            )
            // Cast and Styles…, and Edit roles…, over the window.
            .on_action(cx.listener(|this, _: &crate::cast::OpenCast, window, cx| {
                if this.modal_gives_way() {
                    let view =
                        cx.new(|cx| crate::cast::CastView::new(this.session.clone(), window, cx));
                    this.open_modal(view, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::media::EditRoles, window, cx| {
                    if this.modal_gives_way() {
                        let view = cx.new(|cx| {
                            crate::media::RolesView::new(this.session.clone(), window, cx)
                        });
                        this.open_modal(view, window, cx);
                    }
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::making::OpenShotGraph, window, cx| {
                    this.open_shot_graph(action.media.clone(), window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::making::MakeMedia, window, cx| {
                    let plan = cx.new(|cx| {
                        crate::making::MakePlan::new(
                            this.session.clone(),
                            action.media.clone(),
                            action.rolls,
                            window,
                            cx,
                        )
                    });
                    this.open_modal(plan, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::finish::OpenFinish, window, cx| {
                    let finish = cx.new(|cx| {
                        crate::finish::FinishView::new(
                            this.session.clone(),
                            action.run_check,
                            window,
                            cx,
                        )
                    });
                    this.open_modal(finish, window, cx);
                }),
            )
            // A press on what takes no keys (the conversation's empty room, a
            // chip, a label) leaves them with the window's own box: they
            // settle again once the press is dealt with, on the timeline
            // while it shows, as Qt's widgets that took no focus left them.
            .on_any_mouse_down(cx.listener(|_, _: &MouseDownEvent, window, cx| {
                cx.defer_in(window, |this, window, cx| this.settle_focus(window, cx))
            }))
            // F2 renames the open conversation where it is named.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "f2"
                    && !event.keystroke.modifiers.modified()
                    && this.modal.is_none()
                {
                    cx.stop_propagation();
                    this.rename_open(window, cx);
                }
            }))
            // Files dropped on the window come into the project's media.
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.session.update(cx, |session, cx| {
                    for path in paths.paths() {
                        session.import(path, true);
                    }
                    cx.notify();
                });
                if !this.media_open {
                    this.set_media(true, cx);
                }
            }))
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(theme::surround())
            .font(theme::ui_font(gpui::FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(
                // The sidebar slides in from the window's edge.
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-sidebar_width * (1.0 - sidebar)))
                    .w(px(sidebar_width))
                    // Put away and at rest off the window's edge, its rows are
                    // no Tab stops.
                    .when(sidebar <= 0.0, |away| away.invisible())
                    // Its edge (drag, press, double-click) is the sidebar's own.
                    .child(self.sidebar.clone()),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(main_left))
                    .w(px(main_width))
                    .flex()
                    .flex_col()
                    .child(self.toolbar(leading, window, cx))
                    .children(bars)
                    .child(
                        div()
                            .relative()
                            .h(px(content_height))
                            .w_full()
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .w(px(conversation_width))
                                    .child(self.chat.clone()),
                            )
                            .when(panel > 0.001, |content| {
                                content.child(
                                    // The panel comes in from the right, its left edge
                                    // against the conversation's right.
                                    div()
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left(px(conversation_width - reach))
                                        .w(px(panel_width + reach))
                                        .pr(px(8.0))
                                        .pb(px(8.0))
                                        // Reaching over the conversation, the
                                        // words under it take no press.
                                        .occlude()
                                        .child(self.panel(
                                            panel_width + reach - 8.0,
                                            content_height - 8.0,
                                            inspecting,
                                            aspect,
                                            shows,
                                            window,
                                        )),
                                )
                            }),
                    ),
            )
            .when_some(self.export.clone(), |root, export| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR))
                        .right(px(EXPORT_RIGHT))
                        .child(export)
                        .with_animation(
                            "export-pop",
                            motion::animation(scene::FADE),
                            |popover, t| popover.opacity(t).mt(px(scene::MENU_RISE * (1.0 - t))),
                        ),
                )
            })
            .when_some(self.title_menu.clone(), |root, menu| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR - 2.0))
                        .left(px(title_left))
                        .child(menu),
                )
            })
            .when(self.activity_open, |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR - 2.0))
                        .left(px(title_left))
                        .child(self.activity_view(cx)),
                )
            })
            // The app's menu: above the sidebar's account, or under the
            // toolbar's ⋯ while the sidebar is away, inside the room the
            // window has for it (`app_menu_room`). Taller than the room (a
            // short window), it is cut to it and the wheel over it moves
            // it, so every row can be reached. It fades and rises in by
            // itself (the kit's menu); until it has been laid out once, and
            // its height is known, it is not seen.
            .when_some(self.app_menu.clone(), |root, (menu, from_account)| {
                let above_account = from_account && !leading;
                let left = if above_account {
                    main_left - sidebar_width + 8.0
                } else {
                    main_left + places.app_menu
                };
                let (top, room) = app_menu_room(above_account, window_height);
                let tall = self.app_menu_tall.get();
                if tall.is_none() {
                    window.request_animation_frame();
                }
                let (down, scrolls) = app_menu_place(above_account, room, tall.unwrap_or(0.0));
                let most = (tall.unwrap_or(0.0) - room).max(0.0);
                let scrolled = self.app_menu_scrolled.get().clamp(0.0, most);
                let measured = self.app_menu_tall.clone();
                let wheel = self.app_menu_scrolled.clone();
                let placed = div()
                    .absolute()
                    .top(px(down - scrolled))
                    .left(px(APP_MENU_SHADOW))
                    .when(tall.is_none(), |placed| placed.opacity(0.0))
                    .child(menu)
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                measured.set(Some(f32::from(bounds.size.height)));
                                bounds
                            },
                            move |_, bounds, window, _| {
                                if !scrolls {
                                    return;
                                }
                                // The menu holds the pointer itself (it
                                // occludes), so its wheel is heard here.
                                let (room_top, room_foot) = (px(top), px(top + room));
                                let wheel = wheel.clone();
                                window.on_mouse_event(
                                    move |event: &ScrollWheelEvent, phase, window, _| {
                                        let at = event.position;
                                        if phase != DispatchPhase::Bubble
                                            || at.x < bounds.left()
                                            || at.x > bounds.right()
                                            || at.y < room_top
                                            || at.y > room_foot
                                        {
                                            return;
                                        }
                                        let delta = event.delta.pixel_delta(window.line_height());
                                        let to =
                                            (wheel.get() - f32::from(delta.y)).clamp(0.0, most);
                                        if to != wheel.get() {
                                            wheel.set(to);
                                            window.refresh();
                                        }
                                    },
                                );
                            },
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    );
                root.child(
                    div()
                        .absolute()
                        .top(px(top))
                        .h(px(room))
                        // Room for its shadow at the left; its submenus
                        // open to the right, to the window's edge.
                        .left(px(left - APP_MENU_SHADOW))
                        .right_0()
                        .when(scrolls, |room| room.overflow_hidden())
                        .child(placed),
                )
            })
            // What travels over the window: frames going home, clips
            // flying to the timeline, the picture growing or going back.
            .children(flights.into_iter().map(flown))
            .children(self.modal(window, cx))
            // Notices sit over the conversation, above what is open over the window.
            .child(
                div()
                    .absolute()
                    .top(px(layout::TOOLBAR + bars_height))
                    .bottom_0()
                    .left(px(main_left))
                    .w(px(conversation_width))
                    .child(self.notices(window, cx)),
            )
    }
}

/// A flight as it is drawn this frame: a frame going home covers its box,
/// rounded as the picture is; a clip's poster covers its box with a ring of
/// Kerf's ink round it and its name at its foot, as Qt's; the picture
/// growing or going back is shown whole on black, as the stage shows it.
fn flown(drawn: Drawn) -> AnyElement {
    let Drawn {
        bounds,
        opacity,
        image,
        kind,
    } = drawn;
    // Let go, it is drawn as it was flying.
    let kind = match kind {
        FlightKind::LetGo { was, .. } => *was,
        kind => kind,
    };
    let (radius, whole) = match &kind {
        FlightKind::Clip { .. } => (theme::radius::SMALL, false),
        FlightKind::Grow { .. } | FlightKind::Back { .. } => (theme::radius::CARD, true),
        FlightKind::Export | FlightKind::Home | FlightKind::LetGo { .. } => {
            (theme::radius::CARD, false)
        }
    };
    let fit = if whole {
        ObjectFit::Contain
    } else {
        ObjectFit::Cover
    };
    let flight = div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .opacity(opacity)
        // Cut to its box.
        .overflow_hidden()
        .rounded(px(radius))
        .when(whole, |flight| flight.bg(gpui::black()))
        .child(img(image).size_full().rounded(px(radius)).object_fit(fit));
    match kind {
        FlightKind::Clip { name, .. } => flight
            .when(
                f32::from(bounds.size.height) >= layout::CLIP_FLIGHT_NAMED,
                |flight| {
                    flight.child(
                        div()
                            .absolute()
                            .left(px(theme::space::S4))
                            .bottom(px(theme::space::S4))
                            .max_w(bounds.size.width - px(theme::space::S8))
                            .h(px(17.0))
                            .px(px(theme::space::S4))
                            .flex()
                            .items_center()
                            .rounded(px(theme::radius::SMALL))
                            .bg(gpui::rgba(0x00000099))
                            .child(ui::line(name, text::META, gpui::white().into())),
                    )
                },
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(radius))
                    .border(px(layout::CLIP_FLIGHT_RING))
                    .border_color(theme::accent_ink()),
            )
            .into_any_element(),
        _ => flight.into_any_element(),
    }
}

/// Pixels as points.
pub fn points(value: Pixels) -> f32 {
    f32::from(value)
}

/// How far up from the window's foot (a window `height` points tall) the
/// notices stand: `NOTICE_GAP` above the composer with what stands over it
/// (its top at `composer_top`, as last laid out), so that they never cover
/// Kerf's next steps or what waits to be asked; `NOTICE_FOOT` before the
/// composer has been laid out.
pub fn notice_bottom(height: f32, composer_top: Option<f32>) -> f32 {
    match composer_top {
        Some(top) => (height - top).max(0.0) + layout::NOTICE_GAP,
        None => layout::NOTICE_FOOT,
    }
}

/// Seconds since 1970, now.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// How long ago `then` was at `now` (seconds since 1970), as Qt's
/// editedAgo said it: "just now", "4 min ago", "2 h ago" the same day,
/// "yesterday", "3 days ago", then the date ("4 Mar 2026"). Days, and the
/// date, are the person's own: on the calendar of the system's time zone,
/// as Qt's toLocalTime had them ([`edited_ago_in`]).
pub fn edited_ago(then: i64, now: i64) -> String {
    edited_ago_in(then, now, &jiff::tz::TimeZone::system())
}

/// [`edited_ago`] on the calendar of `zone`.
pub fn edited_ago_in(then: i64, now: i64, zone: &jiff::tz::TimeZone) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let seconds = (now - then).max(0);
    if seconds < 60 {
        return "just now".into();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = minutes / 60;
    let days = local_day(now, zone) - local_day(then, zone);
    if hours < 24 && days == 0 {
        return format!("{hours} h ago");
    }
    if days <= 1 {
        return "yesterday".into();
    }
    if days < 7 {
        return format!("{days} days ago");
    }
    let (year, month, day) = civil(local_day(then, zone));
    format!("{day} {} {year}", MONTHS[(month - 1).clamp(0, 11) as usize])
}

/// The day (since 1970-01-01) that `seconds` since 1970 falls on in
/// `zone`: on its calendar, its offset from UTC then (summer time or not)
/// counted in. A time out of the calendar's reach counts as UTC.
pub fn local_day(seconds: i64, zone: &jiff::tz::TimeZone) -> i64 {
    let offset = jiff::Timestamp::from_second(seconds)
        .map(|at| zone.to_offset(at).seconds())
        .unwrap_or(0);
    (seconds + i64::from(offset)).div_euclid(86_400)
}

/// The date of a day since 1970-01-01: year, month, day.
fn civil(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// The toolbar title's tooltip, as Qt's: when the project was edited and
/// at which revision ("Edited 2 min ago · rev 22", "rev 22" with no time),
/// after the project and its sequence (`window_title`) when the title is a
/// conversation's.
pub fn title_tip(
    window_title: &str,
    conversation: bool,
    revision: u64,
    edited: Option<i64>,
    now: i64,
) -> String {
    let about = match edited {
        Some(then) => format!("Edited {} · rev {revision}", edited_ago(then, now)),
        None => format!("rev {revision}"),
    };
    if conversation && !window_title.is_empty() {
        format!("{window_title} · {about}")
    } else {
        about
    }
}

/// What the toolbar says renders, as Qt's: the newest export in the queue,
/// a running one before a queued one ("Draft export · 42%", "Master export
/// · 0%" while it waits, or the job's name), else the preview rendering
/// ("Rendering preview · 42%"). None when nothing does.
pub fn running_words(
    jobs: &[crate::model::RenderJob],
    preview: &crate::store::Preview,
) -> Option<String> {
    let percent = |fraction: f64| (fraction.clamp(0.0, 1.0) * 100.0).floor() as i64;
    let latest = jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.is_active())
        .max_by_key(|(at, job)| (job.status == "running", *at))
        .map(|(_, job)| job);
    if let Some(job) = latest {
        let label = if matches!(job.quality.as_str(), "review" | "preview") {
            "Draft export".to_owned()
        } else if job.quality == "master" && job.name.is_empty() {
            "Master export".to_owned()
        } else if !job.name.is_empty() {
            job.name.clone()
        } else {
            "Export".to_owned()
        };
        return Some(format!("{label} · {}%", percent(job.progress)));
    }
    preview.rendering().then(|| {
        format!(
            "Rendering preview · {}%",
            percent(preview.progress.unwrap_or(0.0))
        )
    })
}

/// What the panel shows once something in it is asked for: `asked` (the
/// picture, the timeline) alone when the panel is closed (`open` false),
/// else what it `shown` with what is asked joining it. Neither asked for
/// opens both.
pub fn panes_opening(open: bool, shown: (bool, bool), asked: (bool, bool)) -> (bool, bool) {
    let asked = if asked == (false, false) {
        (true, true)
    } else {
        asked
    };
    if open {
        (shown.0 || asked.0, shown.1 || asked.1)
    } else {
        asked
    }
}

/// What holds the keys as the window settles them ([`settle_keys`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holder {
    /// Nothing, or the window's own box.
    Nothing,
    /// The timeline.
    Timeline,
    /// A field being typed in, a menu, or a panel open over the window
    /// ([`holds_keys_itself`]).
    Busy,
    /// Something else that takes the keys: a button, a row, the picture.
    Other,
}

/// Where the keys go as the window settles them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeysGo {
    Timeline,
    /// The window's own box: its keys (the palette, the mask tools) work.
    Window,
    /// Where they are.
    Stay,
}

/// Where the keys go as the window settles them, as Qt's settleFocus had
/// them: to the timeline while it is in view (`timeline_in_view`), unless
/// someone is typing, choosing in a menu, or in a panel over the window;
/// with no timeline in view they stay where they are, and the timeline
/// folded away gives them back to the window.
pub fn settle_keys(timeline_in_view: bool, holder: Holder) -> KeysGo {
    match holder {
        Holder::Busy => KeysGo::Stay,
        Holder::Timeline if timeline_in_view => KeysGo::Stay,
        Holder::Timeline => KeysGo::Window,
        Holder::Nothing | Holder::Other if timeline_in_view => KeysGo::Timeline,
        Holder::Nothing => KeysGo::Window,
        Holder::Other => KeysGo::Stay,
    }
}

/// Whether what has the keys (its key contexts, outermost first) keeps
/// them as the window settles them: a field being typed in, a menu, or a
/// panel open over the window.
pub fn holds_keys_itself(stack: &[gpui::KeyContext]) -> bool {
    let busy = [
        crate::input::CONTEXT,
        crate::ui::MENU,
        sidebar::MENU,
        crate::palette::CONTEXT,
        CONFIRM,
    ];
    ui::panel_of(stack).is_some()
        || stack
            .iter()
            .any(|context| busy.iter().any(|name| context.contains(name)))
}

/// Where the keys are as Esc is pressed, and what stands open, for
/// [`escape_goes`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EscapeAt {
    /// One of the kit's menus has the keys: it closes itself (its deepest
    /// submenu first).
    pub menu_has_keys: bool,
    /// One of the window's own menus is open (the title's, Activity, the
    /// app's, the sidebar's), whether or not it has the keys.
    pub menu_open: bool,
    /// A field is being typed in: its own Esc goes first.
    pub in_field: bool,
    /// What one goes into is over the window, with the keys or not:
    /// Settings, the palette, a question, a shot's graph, Export's popover.
    pub in_dialog: bool,
    /// The picture has the keys (a mask tool out, a mask chosen).
    pub on_picture: bool,
    /// The preview render's card shows over the picture
    /// ([`render_card_shows`]): Qt's filter took Esc only while the card
    /// was active and visible.
    pub rendering: bool,
    /// The Media band is open.
    pub band_open: bool,
}

/// What Esc does first, ahead of every Esc the window binds (the
/// timeline's Reject Proposal and its letting a clip go among them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeGoes {
    /// Nothing here: it goes on to what has the keys (a menu, a field, a
    /// dialog, the picture), then to the window's own bindings.
    On,
    /// The open menus close.
    CloseMenus,
    /// The preview rendering stops.
    StopRender,
    /// The Media band is put away.
    PutBandAway,
}

/// What Esc does first, as Qt's filters took it: an open menu goes first;
/// then a preview rendering stops (not from a field, a menu or a dialog:
/// agent_hud.cpp); then the Media band closes (not from a field, the
/// picture or a dialog: stage_shell.cpp); else it goes on to whatever has
/// the keys.
pub fn escape_goes(at: EscapeAt) -> EscapeGoes {
    if at.menu_has_keys {
        return EscapeGoes::On;
    }
    if at.menu_open {
        return EscapeGoes::CloseMenus;
    }
    if at.in_field || at.in_dialog {
        return EscapeGoes::On;
    }
    if at.rendering {
        return EscapeGoes::StopRender;
    }
    if at.band_open && !at.on_picture {
        return EscapeGoes::PutBandAway;
    }
    EscapeGoes::On
}

/// What the window says when changing a conversation (renaming,
/// archiving, deleting or opening it) failed, as Qt's status said it.
pub fn conversation_error(message: &str) -> String {
    format!("Couldn't change the conversation · {message}")
}

/// What the bridge answers a command sent while kerfd cannot be reached.
const DISCONNECTED: &str = "Runtime is disconnected; reconnect to continue";

/// A bar's height under the toolbar.
const BAR: f32 = theme::height::H40;
/// Export's chevron, and the height inside the button's outline.
const EXPORT_CHEVRON: f32 = 24.0;
const EXPORT_INNER: f32 = theme::height::H28 - 2.0;
/// How far Export's right edge is from the window's: the toolbar's end,
/// past Follow Kerf, Media and the panel's button. Its menu opens there.
const EXPORT_RIGHT: f32 = 8.0 + 3.0 * (theme::height::H28 + 4.0);
/// The title's menu: room for Agent settings… and the model after it.
const TITLE_MENU_WIDTH: f32 = 260.0;
/// The app's menu, and its submenus: room for a command and its keys.
const APP_MENU_WIDTH: f32 = 248.0;
/// How far the app's menu keeps from the window's top and foot, and from
/// the account it opens over; the room kept at its left for its shadow.
const APP_MENU_GAP: f32 = 4.0;
const APP_MENU_SHADOW: f32 = 12.0;

/// The room a window `window_height` tall has for the app's menu: its top
/// and height. Over the sidebar's account, from just under the window's
/// top to just over the account; under the toolbar's ⋯, from the
/// toolbar's foot to just over the window's.
pub fn app_menu_room(above_account: bool, window_height: f32) -> (f32, f32) {
    let (top, foot) = if above_account {
        (APP_MENU_GAP, layout::SIDEBAR_FOOT + APP_MENU_GAP)
    } else {
        (layout::TOOLBAR - 2.0, APP_MENU_GAP)
    };
    (top, (window_height - top - foot).max(0.0))
}

/// Where the app's menu, `tall` points tall, sits in its room, `room`
/// points tall: how far down from the room's top, and whether it scrolls
/// there. Over the account it stands on the room's foot, under the ⋯ it
/// hangs from its top; taller than the room it starts at its top and
/// scrolls, so that every row can be reached.
pub fn app_menu_place(above_account: bool, room: f32, tall: f32) -> (f32, bool) {
    let spare = room - tall;
    if spare < 0.0 {
        (0.0, true)
    } else if above_account {
        (spare, false)
    } else {
        (0.0, false)
    }
}

/// Where the toolbar's controls start, and the popups they open, from
/// the main column's left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarPlaces {
    /// Its first control (its leading padding).
    pub lead: f32,
    /// The title's menu and Activity, under the title.
    pub title: f32,
    /// The app's menu, under the toolbar's ⋯ (the sidebar away).
    pub app_menu: f32,
}

/// [`ToolbarPlaces`] for a toolbar at the window's leading edge or not
/// (`leading`: the sidebar away), with `room` what macOS's window buttons
/// need there ([`crate::app::window_buttons_room`]). At that edge the
/// toolbar starts past the buttons, Sidebar and ⋯ first, and its popups
/// move on with the controls that open them.
pub fn toolbar_places(leading: bool, room: f32) -> ToolbarPlaces {
    let pad = 8.0;
    if !leading {
        // The title first, past its row's own 8.
        return ToolbarPlaces {
            lead: pad,
            title: pad + 8.0,
            app_menu: pad + TOOLBAR_STEP,
        };
    }
    let lead = crate::app::lead_past(room);
    ToolbarPlaces {
        lead,
        // Sidebar, then ⋯, then the title's row, 8 in, and 4 past it.
        title: lead + 2.0 * TOOLBAR_STEP + 8.0 + 4.0,
        app_menu: lead + TOOLBAR_STEP,
    }
}
/// A toolbar button and the gap after it.
const TOOLBAR_STEP: f32 = theme::height::H28 + 4.0;
/// How far the stage grows over the conversation while changes are
/// watched (Qt's reelReach).
use crate::theme::motion::REEL_REACH;
/// Activity's width, and its column of kinds.
const ACTIVITY_WIDTH: f32 = 380.0;
const ACTIVITY_KIND: f32 = 64.0;
/// The toolbar's title field, while a conversation is renamed there.
const TITLE_FIELD: f32 = 280.0;

/// A bar under the toolbar: its colour in the dot only, the bar and its
/// words neutral; what it says, then its buttons. It fades in where it is.
fn bar(
    id: SharedString,
    color: Rgba,
    title: SharedString,
    detail: Option<SharedString>,
    buttons: Vec<AnyElement>,
    tip: Option<SharedString>,
) -> AnyElement {
    let tip_id = SharedString::from(format!("{id}-tip"));
    let bar = div()
        .id(id.clone())
        .w_full()
        .h(px(BAR))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .bg(theme::chrome())
        .border_b_1()
        .border_color(theme::line())
        .child(ui::dot(color, 7.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .flex_1()
                .min_w_0()
                .child(
                    ui::line(title, text::CONTROL_STRONG, theme::ink())
                        .min_w_0()
                        .when(detail.is_some(), |title| title.flex_none()),
                )
                .when_some(detail, |row, detail| {
                    row.child(
                        ui::line(detail, text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    )
                }),
        )
        .children(buttons)
        .with_animation(id, motion::animation(scene::FADE), |bar, t| bar.opacity(t));
    // The whole of what it says, where its line cuts it.
    match tip {
        Some(tip) => ui::tipped(tip_id, ui::Tip::new(tip), bar).into_any_element(),
        None => bar.into_any_element(),
    }
}

/// The paths a file chooser gave; None, with a notice in `session`'s window
/// saying why where there is something to say (no chooser could open, a
/// name kerfd cannot be sent), when there are none to use.
fn chosen_paths(answer: crate::presets::Picked, session: &mut Session) -> Option<Vec<PathBuf>> {
    use crate::presets::Picked;
    match answer {
        Picked::Paths(paths) => Some(paths),
        Picked::Failed(error) => {
            session
                .state
                .notify(&format!("No file chooser could open: {error}"), true);
            None
        }
        Picked::Unusable(words) => {
            session.state.notify(&words, true);
            None
        }
        Picked::Nothing => None,
    }
}

/// The first line of what `text` says, trimmed (the whole of it when it
/// has one): what the error bar and the toolbar's line show of an error.
pub fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// One frame on from `position`, or `frames` back, on the frame grid at
/// `rate` frames a second: the playhead rounded to its frame first (after a
/// skim or the clock it can sit between frames), never before the start.
pub fn frame_step(position: f64, rate: f64, frames: i64) -> f64 {
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        30.0
    };
    (((position * rate).round() + frames as f64) / rate).max(0.0)
}

/// Whether an error the bridge sends stays in the error bar: not what a
/// view answers in place (a making, a shot's graph, a recipe, a workflow,
/// the Cast dialog).
pub fn kept_in_bar(command: &str) -> bool {
    !(matches!(
        command,
        "media_make" | "media_carry" | "takes_read" | "cast"
    ) || command.starts_with("recipe")
        || command.starts_with("shot_")
        || command.starts_with("workflow"))
}

/// An error as a person reads it: an engine diagnostic's severity and code
/// ("error[ripple.blocked]: ", "warning[x.y] 3:14: ") are for tools.
pub fn readable_error(message: &str) -> String {
    let Some(rest) = message
        .strip_prefix("error[")
        .or_else(|| message.strip_prefix("warning["))
    else {
        return message.to_owned();
    };
    let Some(close) = rest.find(']') else {
        return message.to_owned();
    };
    let code = &rest[..close];
    if code.is_empty()
        || !code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return message.to_owned();
    }
    let after = &rest[close + 1..];
    // An optional " line:column".
    let position = |text: &str| -> Option<usize> {
        let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
        let tail = text.strip_prefix(' ')?;
        let line = digits(tail);
        let tail = tail.get(line..)?.strip_prefix(':').filter(|_| line > 0)?;
        let column = digits(tail);
        (column > 0).then_some(1 + line + 1 + column)
    };
    let after = match position(after) {
        Some(skip) if after[skip..].starts_with(": ") => &after[skip..],
        _ => after,
    };
    match after.strip_prefix(": ") {
        Some(text) if !text.trim().is_empty() => text.to_owned(),
        _ => message.to_owned(),
    }
}

/// The media files a folder that becomes a project brings in.
pub const FOLDER_MEDIA: [&str; 7] = ["mov", "mp4", "m4v", "mkv", "webm", "wav", "mp3"];

/// The media files directly in `folder`, by name.
pub fn folder_media(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    FOLDER_MEDIA.contains(&extension.to_ascii_lowercase().as_str())
                })
        })
        .collect();
    files.sort();
    files
}

/// The window's title: the project, and its sequence when that is named
/// otherwise ("Atlas · 30 s spot v7").
pub fn window_title(project: &str, timeline: &crate::model::Timeline) -> String {
    let sequence = timeline
        .sequence
        .as_deref()
        .and_then(|id| timeline.sequences.iter().find(|sequence| sequence.id == id))
        .map(|sequence| sequence.name.trim())
        .unwrap_or_default();
    if sequence.is_empty() || sequence == project {
        project.to_owned()
    } else if project.is_empty() {
        sequence.to_owned()
    } else {
        format!("{project} · {sequence}")
    }
}

/// Whether `folder` is no project yet: it has no project.kerf.
pub fn needs_start(folder: &Path) -> bool {
    !folder.join("project.kerf").exists()
}

/// Opens `project` in a window of its own. `starts` when it was no project
/// yet: once kerfd has it, its folder's media comes in (the launcher's
/// `LauncherEvent::Open { starts }` means the same).
pub fn open_folder(project: PathBuf, starts: bool, cx: &mut App) {
    open_folder_with(project, starts, None, cx);
}

/// [`open_folder`] with the kerfd the project is opened with (None finds
/// one as at launch): a window opening another project keeps its own.
pub fn open_folder_with(project: PathBuf, starts: bool, kerfd: Option<PathBuf>, cx: &mut App) {
    open_folder_in(project, starts, kerfd, None, cx);
}

/// [`open_folder_with`], in `place`: the bounds of the window it takes
/// the place of (`window.window_bounds()`), so it opens where that one
/// was, at its size, maximized if it was, as Qt's openEditor did.
pub fn open_folder_in(
    project: PathBuf,
    starts: bool,
    kerfd: Option<PathBuf>,
    place: Option<WindowBounds>,
    cx: &mut App,
) {
    crate::app::open_in_place(project.clone(), kerfd, place, cx);
    if !starts {
        return;
    }
    for handle in cx.windows() {
        let Some(workspace) = handle.downcast::<Workspace>() else {
            continue;
        };
        // The window being updated (this one) answers Err, and is left be.
        let _ = workspace.update(cx, |workspace, _, cx| {
            let session = workspace.session.read(cx);
            let fresh = !session.state.has_snapshot;
            let here = session
                .project
                .as_deref()
                .is_some_and(|open| crate::app::same_folder(open, &project));
            if fresh && here && workspace.start_media.is_none() {
                workspace.start_media = Some(project.clone());
            }
        });
    }
}

/// `button` with its key after its name in its tip, when it has one.
fn with_key(button: ui::IconButton, key: Option<String>) -> ui::IconButton {
    match key {
        Some(key) => button.key(key),
        None => button,
    }
}

/// `button` with the tip the toolbar worked out for it (its name and key,
/// or its name and why not), greyed while it cannot be pressed.
fn kit_tip(
    button: ui::IconButton,
    (words, key): (String, Option<String>),
    disabled: bool,
) -> ui::IconButton {
    let button = button.disabled(disabled);
    if disabled {
        button.tooltip(words.clone()).reason(words)
    } else {
        with_key(button.tooltip(words), key)
    }
}

/// `element`, a Tab stop of the toolbar's own (not a kit control): its
/// keys tracked by `focus`, the kit's ring just inside its edge (`corner`
/// round) while it has them, and the pointer's press leaving them where
/// they were. Not a stop while it cannot be pressed (`able` false).
fn toolbar_stop(
    element: Stateful<gpui::Div>,
    focus: &FocusHandle,
    corner: f32,
    able: bool,
    window: &Window,
) -> Stateful<gpui::Div> {
    if !able {
        return element;
    }
    element
        .relative()
        .track_focus(focus)
        .key_context(ui::CONTROL)
        .capture_any_mouse_down(|_, window, _| window.prevent_default())
        .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
        .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
        .when(focus.is_focused(window), |element| {
            element.child(ui::ring(corner))
        })
}

/// Where a manual export's frame is at `elapsed`: how far along (eased,
/// 0..1) and how clear. It crosses over [`scene::FLIGHT`] fading from 0.9
/// to 0.2, then fades out where it landed over [`scene::FLIGHT_LAND`].
/// None once it has gone.
pub fn export_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    if elapsed >= scene::FLIGHT + scene::FLIGHT_LAND {
        return None;
    }
    let flight = scene::FLIGHT.as_secs_f32();
    let land = scene::FLIGHT_LAND.as_secs_f32();
    let elapsed = elapsed.as_secs_f32();
    let t = (elapsed / flight).clamp(0.0, 1.0);
    let opacity = FLIGHT_FROM + (FLIGHT_TO - FLIGHT_FROM) * t;
    let fading = ((elapsed - flight) / land).clamp(0.0, 1.0);
    Some((motion::ease(t), opacity * (1.0 - fading)))
}

/// Where the picture going home is at `elapsed`: how far along (eased,
/// 0..1) over [`scene::STAGE`], whole and clear, then how clear as it fades
/// where it landed over [`scene::FLIGHT_LAND`]. None once it has gone.
pub fn home_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    if elapsed >= scene::STAGE + scene::FLIGHT_LAND {
        return None;
    }
    let travel = scene::STAGE.as_secs_f32();
    let land = scene::FLIGHT_LAND.as_secs_f32();
    let elapsed = elapsed.as_secs_f32();
    let t = (elapsed / travel).clamp(0.0, 1.0);
    let fading = ((elapsed - travel) / land).clamp(0.0, 1.0);
    Some((motion::ease(t), 1.0 - fading))
}

/// How clear a manual export's frame is as it leaves the picture and as it lands.
const FLIGHT_FROM: f32 = 0.9;
const FLIGHT_TO: f32 = 0.2;

/// How long before the picture has grown the timeline starts to come in,
/// and how long its stand-in takes to fade (kept here for the tests and
/// views that name them so).
pub use crate::stage::{render_card_shows, render_stop_holds, RenderCard};
pub use crate::theme::motion::{STAND_IN_MOST, STAND_IN_RELEASE, TIMELINE_LEAD};

/// Where a clip of a cut being built is at `elapsed` on its way from its
/// footage to the timeline: how far along (eased, over [`scene::FLIGHT`])
/// and how clear (whole, then fading over [`scene::FLIGHT_LAND`] where it
/// landed, over what it became). None once it has gone.
pub fn clip_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    travel_then_fade(elapsed, scene::FLIGHT, scene::FLIGHT_LAND)
}

/// Where the picture's stand-in is at `elapsed` as the picture grows out
/// of what it came from: how far along (eased, over [`scene::STAGE`]),
/// whole, holding where it landed until it is let go (`released`, that
/// long after it left; None: not yet), then fading over
/// [`STAND_IN_RELEASE`] over the picture. None once it has gone.
pub fn grow_flight_at(elapsed: Duration, released: Option<Duration>) -> Option<(f32, f32)> {
    let Some(released) = released else {
        let t = elapsed.as_secs_f32() / scene::STAGE.as_secs_f32();
        return Some((motion::ease(t.clamp(0.0, 1.0)), 1.0));
    };
    let fades_from = released.max(scene::STAGE);
    if elapsed >= fades_from + STAND_IN_RELEASE {
        return None;
    }
    let t = elapsed.as_secs_f32() / scene::STAGE.as_secs_f32();
    let fading =
        (elapsed.as_secs_f32() - fades_from.as_secs_f32()) / STAND_IN_RELEASE.as_secs_f32();
    Some((
        motion::ease(t.clamp(0.0, 1.0)),
        1.0 - fading.clamp(0.0, 1.0),
    ))
}

/// Where the picture is at `elapsed` as it goes back into its card: how
/// far along (eased, over [`scene::PANEL`], as the panel goes), whole,
/// then fading over [`scene::FLIGHT_LAND`] over the card's own.
pub fn back_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    travel_then_fade(elapsed, scene::PANEL, scene::FLIGHT_LAND)
}

/// What flew, let go where it had got to as its landing went away, `clear`
/// as it was then: there (all along), fading from `clear` over
/// [`scene::FLIGHT_LAND`], as one that has landed. None once it has gone.
pub fn let_go_flight_at(elapsed: Duration, clear: f32) -> Option<(f32, f32)> {
    if elapsed >= scene::FLIGHT_LAND {
        return None;
    }
    let fading = elapsed.as_secs_f32() / scene::FLIGHT_LAND.as_secs_f32();
    Some((1.0, clear.clamp(0.0, 1.0) * (1.0 - fading)))
}

/// Eased along over `travel`, whole; then fading over `land`. Under
/// Reduce Motion nothing flies, so this is never asked then.
fn travel_then_fade(elapsed: Duration, travel: Duration, land: Duration) -> Option<(f32, f32)> {
    if elapsed >= travel + land {
        return None;
    }
    let t = elapsed.as_secs_f32() / travel.as_secs_f32().max(f32::EPSILON);
    let fading =
        (elapsed.as_secs_f32() - travel.as_secs_f32()) / land.as_secs_f32().max(f32::EPSILON);
    Some((
        motion::ease(t.clamp(0.0, 1.0)),
        1.0 - fading.clamp(0.0, 1.0),
    ))
}

/// How much of the stage and of the timeline shows `elapsed` after the
/// picture began to grow: the stage once its stand-in has landed, over
/// [`scene::FLIGHT_LAND`]; the timeline after it, from
/// [`TIMELINE_LEAD`] before the picture has grown, over
/// [`scene::SETTLE`], as Qt's veil lifted.
pub fn grow_shows(elapsed: Duration) -> (f32, f32) {
    let after = |start: Duration, over: Duration| {
        let t = (elapsed.as_secs_f32() - start.as_secs_f32()) / over.as_secs_f32();
        motion::ease(t.clamp(0.0, 1.0))
    };
    (
        after(scene::STAGE, scene::FLIGHT_LAND),
        after(scene::STAGE.saturating_sub(TIMELINE_LEAD), scene::SETTLE),
    )
}

/// A clip a cut being built placed: its id, the footage it plays (whose
/// tile and poster it flies with), its name, and where it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedClip {
    pub clip: String,
    pub source: Option<String>,
    pub name: String,
    pub start: f64,
}

/// What `cut` (a proposal over `program`) placed that `seen` (each clip
/// as "id@end") had not: each picture clip added or moved since, not one
/// taken out and none on a sound track (its sound comes with it), in the
/// order they play; and what is seen now.
pub fn placed_since(
    program: &crate::model::Timeline,
    cut: &crate::model::Timeline,
    seen: &HashSet<String>,
) -> (Vec<PlacedClip>, HashSet<String>) {
    let changes = crate::chat::clip_changes(program, cut);
    let key = |change: &crate::chat::ClipChange| format!("{}@{:.4}", change.clip, change.new_end);
    let now: HashSet<String> = changes.iter().map(key).collect();
    let mut placed: Vec<PlacedClip> = changes
        .iter()
        .filter(|change| !change.removed && !change.transition && !seen.contains(&key(change)))
        .filter_map(|change| {
            let (track, clip) = cut.clip(&change.clip)?;
            (track.kind == crate::model::TrackKind::Video).then(|| PlacedClip {
                clip: change.clip.clone(),
                source: clip.source_id.clone(),
                name: change.name.clone(),
                start: change.new_start,
            })
        })
        .collect();
    placed.sort_by(|a, b| a.start.total_cmp(&b.start));
    (placed, now)
}

/// The first clip of `timeline`'s first picture track that has one, and
/// the footage it plays: what the picture grows out of.
fn first_picture_clip(timeline: &crate::model::Timeline) -> Option<(String, String)> {
    timeline
        .tracks
        .iter()
        .filter(|track| track.kind == crate::model::TrackKind::Video)
        .find_map(|track| {
            let clip = track
                .clips
                .iter()
                .min_by(|a, b| a.timeline_start.total_cmp(&b.timeline_start))?;
            Some((clip.id.clone(), clip.source_id.clone()?))
        })
}

/// The rect `along` (0..1) of the way from `from` to `to`.
pub fn lerp_bounds(from: Bounds<Pixels>, to: Bounds<Pixels>, along: f32) -> Bounds<Pixels> {
    let mix = |a: Pixels, b: Pixels| a + (b - a) * along;
    Bounds::new(
        point(
            mix(from.origin.x, to.origin.x),
            mix(from.origin.y, to.origin.y),
        ),
        size(
            mix(from.size.width, to.size.width),
            mix(from.size.height, to.size.height),
        ),
    )
}

/// Where the stage puts a `width` × `height` picture within `slot` (the
/// stage, its transport under it): in its room, inset as the stage insets
/// its device, at its own aspect, centred.
pub fn picture_within(slot: Bounds<Pixels>, width: u32, height: u32) -> Bounds<Pixels> {
    let aspect = if width == 0 || height == 0 {
        16.0 / 9.0
    } else {
        width as f32 / height as f32
    };
    picture_in(slot, aspect)
}

/// [`picture_within`] for a picture `aspect` wide to its height.
pub fn picture_in(slot: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let area_height = (f32::from(slot.size.height) - layout::TRANSPORT).max(1.0);
    let inset = STAGE_MARGIN + STAGE_BEZEL;
    let room_width = (f32::from(slot.size.width) - inset * 2.0).max(1.0);
    let room_height = (area_height - inset * 2.0).max(1.0);
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let (shown_width, shown_height) = if room_width / room_height > aspect {
        (room_height * aspect, room_height)
    } else {
        (room_width, room_width / aspect)
    };
    Bounds::new(
        point(
            slot.origin.x + px((f32::from(slot.size.width) - shown_width) / 2.0),
            slot.origin.y + px((area_height - shown_height) / 2.0),
        ),
        size(px(shown_width), px(shown_height)),
    )
}

/// How tall the timeline is under the picture in a panel `height` tall,
/// with the Media band `band` tall between them and the stage `width` wide,
/// split as Qt split it by the picture's shape (`aspect`, width over
/// height): under a wide picture [`layout::TIMELINE_WIDE_SHARE`] of the
/// height, at least [`layout::TIMELINE_WIDE_LEAST`], and what the picture
/// cannot use at that width; under a vertical or square one
/// [`layout::TIMELINE_UPRIGHT_SHARE`], at least
/// [`layout::TIMELINE_UPRIGHT_LEAST`]. The band takes the timeline's top
/// until that would leave it under [`layout::MEDIA_BAND_TIMELINE_LEAST`];
/// then the picture gives way. The transport, which Qt's timeline carried,
/// is under the picture here, and counts as the timeline's.
pub fn timeline_split(height: f32, band: f32, width: f32, aspect: f32) -> f32 {
    let height = height.max(0.0);
    let band = band.clamp(0.0, height);
    let wide = aspect >= layout::WIDE;
    let (share, least) = if wide {
        (layout::TIMELINE_WIDE_SHARE, layout::TIMELINE_WIDE_LEAST)
    } else {
        (
            layout::TIMELINE_UPRIGHT_SHARE,
            layout::TIMELINE_UPRIGHT_LEAST,
        )
    };
    // Qt's slot: the picture with its air, under it the timeline and its transport.
    let mut slot = height - (height * share).max(least).min(height);
    if wide {
        let air = (STAGE_MARGIN + STAGE_BEZEL) * 2.0;
        slot = slot.min((width - air).max(0.0) / aspect + air);
    }
    if band > 0.0 {
        slot = slot.min(height - band - layout::MEDIA_BAND_TIMELINE_LEAST);
    }
    let picture = (slot.max(0.0) + layout::TRANSPORT).min(height - band);
    (height - band - picture).max(0.0)
}

/// How a panel `height` tall, its stage `stage_width` wide showing a
/// picture `aspect` wide to its height, is shared with the picture, the
/// timeline and the Media band shown `(picture, timeline, media)` (each
/// 0..1, as far as they have come): the picture's height, the band's and
/// the timeline's. The timeline's share follows the picture's shape
/// ([`timeline_split`]); the picture takes the rest; one of the two alone
/// has all of it.
pub fn panel_heights(
    height: f32,
    stage_width: f32,
    aspect: f32,
    (picture, timeline, media): (f32, f32, f32),
) -> (f32, f32, f32) {
    let band = layout::MEDIA_BAND * media;
    let timeline_full = (height - band).max(0.0);
    let timeline_tall = timeline_split(height, band, stage_width, aspect);
    let timeline_height = timeline_tall + (timeline_full - timeline_tall) * (1.0 - picture);
    let timeline_height = timeline_height * timeline;
    let picture_height = (height - band - timeline_height).max(0.0);
    (picture_height, band, timeline_height)
}

/// The stage's air round its device, and the device's bezel.
const STAGE_MARGIN: f32 = 24.0;
const STAGE_BEZEL: f32 = 8.0;

/// Where a flight lands on `row` (the project's row in the sidebar), as
/// Qt landed it: a small frame at the row's start, inside its height.
pub fn landing_in(row: Bounds<Pixels>) -> Bounds<Pixels> {
    let (left, top) = scene::LANDING_INSET;
    let height = (f32::from(row.size.height) - top * 2.0).max(1.0);
    let width = scene::LANDING_WIDTH.min(f32::from(row.size.width)).max(1.0);
    Bounds::new(
        point(row.origin.x + px(left), row.origin.y + px(top)),
        size(px(width), px(height)),
    )
}

/// Where a manual export's frame lands when the sidebar has not laid the
/// project's row out yet: at its folder, under the toolbar, New
/// conversation and Search.
fn project_row() -> Bounds<Pixels> {
    let top = layout::TOOLBAR + theme::height::H32 * 2.0 + 2.0 + 8.0 + 4.0;
    Bounds::new(point(px(12.0), px(top)), size(px(44.0), px(26.0)))
}

gpui::actions!(confirm, [CloseConfirm, ConfirmDefault]);

/// The key context a question over the window answers in.
const CONFIRM: &str = "Confirm";

/// Esc and Enter answer the question before the window's own keys have
/// them. Binding a keymap clears these, so they are bound again whenever a
/// question opens and finds them missing.
fn bind_confirm(cx: &mut App) {
    if !crate::settings::escape_bound(cx, &CloseConfirm) {
        cx.bind_keys([
            KeyBinding::new("escape", CloseConfirm, Some(CONFIRM)),
            KeyBinding::new("enter", ConfirmDefault, Some(CONFIRM)),
        ]);
    }
}

/// A question over the window, before what cannot be undone or what
/// changes the window: Cancel, or what it asks. Esc, or a press on the dim
/// around it, cancels; Enter does the default (Cancel, unless said).
pub struct Confirm {
    focus_handle: FocusHandle,
    title: SharedString,
    detail: Option<SharedString>,
    action: SharedString,
    /// What it asks cannot be undone: its button is in brick.
    danger: bool,
    /// Enter does what it asks rather than cancelling.
    act_by_default: bool,
    on_confirm: Option<Box<dyn FnOnce(&mut Window, &mut App)>>,
}

impl EventEmitter<DismissEvent> for Confirm {}

impl Confirm {
    pub fn new(
        cx: &mut Context<Self>,
        title: impl Into<SharedString>,
        detail: Option<&str>,
        action: impl Into<SharedString>,
        danger: bool,
        act_by_default: bool,
        on_confirm: impl FnOnce(&mut Window, &mut App) + 'static,
    ) -> Self {
        bind_confirm(cx);
        Self {
            focus_handle: cx.focus_handle(),
            title: title.into(),
            detail: detail.map(|detail| detail.to_owned().into()),
            action: action.into(),
            danger,
            act_by_default,
            on_confirm: Some(Box::new(on_confirm)),
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.on_confirm = None;
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_confirm) = self.on_confirm.take() {
            cx.emit(DismissEvent);
            on_confirm(window, cx);
        }
    }
}

impl Focusable for Confirm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Confirm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(CONFIRM)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &CloseConfirm, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &ConfirmDefault, window, cx| {
                if this.act_by_default {
                    this.confirm(window, cx);
                } else {
                    this.cancel(cx);
                }
            }))
            // A press on the dim around it cancels, and does nothing to
            // what lies under the dim.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.cancel(cx);
            }))
            .w(px(CONFIRM_WIDTH))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .bg(theme::popup())
            .child(ui::label(self.title.clone(), text::TITLE, theme::ink()))
            .when_some(self.detail.clone(), |body, detail| {
                body.child(ui::label(detail, text::BODY, theme::ink2()))
            })
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        ui::button("confirm-cancel", "Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx))),
                    )
                    .child(
                        ui::button("confirm-action", self.action.clone())
                            .kind(if self.danger {
                                ui::Kind::Danger
                            } else {
                                ui::Kind::Primary
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm(window, cx)
                            })),
                    ),
            )
    }
}

/// A question's width over the window.
const CONFIRM_WIDTH: f32 = 420.0;

/// What a sync command asks before it runs ([`AskWords`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordsAsked {
    pub title: String,
    pub detail: String,
    /// What the field says while empty, and what it holds at first.
    pub placeholder: String,
    pub text: String,
    /// Its button's words.
    pub action: String,
}

/// What sync command `id` asks for, in Qt's words: Commit's message, New
/// Branch's name, a pull request's title (from `branch`, as Qt suggested
/// it), the GitHub repository's name (from the project's `folder`). None
/// for a command that needs no words.
pub fn sync_question(id: &str, branch: &str, folder: &str) -> Option<WordsAsked> {
    let asked =
        |title: &str, detail: String, placeholder: &str, text: String, action: &str| WordsAsked {
            title: title.into(),
            detail,
            placeholder: placeholder.into(),
            text,
            action: action.into(),
        };
    Some(match id {
        "collab.commit" => asked(
            "Commit",
            "Describe the change".into(),
            "Message",
            String::new(),
            "Commit",
        ),
        "collab.newBranch" => asked(
            "New Branch",
            "Commits your edits, then starts a branch from them.".into(),
            "Name",
            String::new(),
            "Create",
        ),
        "collab.pullRequest" => {
            let mut title: String = branch.replace(['-', '_'], " ");
            if let Some(first) = title.chars().next() {
                title.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
            }
            let which = if branch.is_empty() {
                "this branch"
            } else {
                branch
            };
            asked(
                "Open Pull Request",
                format!(
                    "Pushes {which} and opens a pull request on GitHub. Reviewers read the change as Kerf code."
                ),
                "Title",
                title,
                "Open",
            )
        }
        "collab.publish" => {
            let name: String = folder
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                        ch
                    } else {
                        '-'
                    }
                })
                .collect();
            asked(
                "Publish to GitHub",
                "Makes the project a Git repository if it is not one yet, then pushes it so collaborators can clone it, sync, and open pull requests. The repository is private."
                    .into(),
                "Repository name",
                name,
                "Publish",
            )
        }
        _ => return None,
    })
}

/// The parameters sync command `id` runs with, given the `words` asked for.
pub fn sync_params(id: &str, words: &str) -> serde_json::Value {
    let words = words.trim();
    match id {
        "collab.commit" => serde_json::json!({"message": words}),
        "collab.newBranch" => serde_json::json!({"name": words}),
        "collab.pullRequest" => serde_json::json!({"title": words}),
        "collab.publish" => serde_json::json!({"github": {"name": words, "private": true}}),
        _ => serde_json::json!({}),
    }
}

/// Asks over the window for a line of words a command needs, as Qt's
/// input dialogs did: its title, what it is for, the field, Cancel and the
/// button that runs it (not while the field is empty). Esc or a press on
/// the dim cancels; Return runs it.
pub struct AskWords {
    focus_handle: FocusHandle,
    asked: WordsAsked,
    field: Entity<TextInput>,
    on_accept: Option<Box<dyn FnOnce(&str, &mut App)>>,
    _subscription: Subscription,
}

impl EventEmitter<DismissEvent> for AskWords {}

impl AskWords {
    pub fn new(
        asked: WordsAsked,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_accept: impl FnOnce(&str, &mut App) + 'static,
    ) -> Self {
        bind_confirm(cx);
        let field = cx.new(|cx| {
            let mut field = TextInput::new(cx, asked.placeholder.clone()).with_style(text::CONTROL);
            field.set_text(asked.text.clone(), cx);
            field.choose_all(cx);
            field
        });
        let subscription =
            cx.subscribe_in(
                &field,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Submit => this.accept(cx),
                    InputEvent::Cancel => cx.emit(DismissEvent),
                    InputEvent::Changed => cx.notify(),
                },
            );
        Self {
            focus_handle: cx.focus_handle(),
            asked,
            field,
            on_accept: Some(Box::new(on_accept)),
            _subscription: subscription,
        }
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        let words = self.field.read(cx).text().trim().to_owned();
        if words.is_empty() {
            return;
        }
        if let Some(on_accept) = self.on_accept.take() {
            cx.emit(DismissEvent);
            on_accept(&words, cx);
        }
    }
}

impl Focusable for AskWords {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // Typing goes into the field at once.
        self.field.read(cx).focus_handle(cx)
    }
}

impl Render for AskWords {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = !self.field.read(cx).text().trim().is_empty();
        div()
            .key_context(CONFIRM)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &CloseConfirm, _, cx| cx.emit(DismissEvent)))
            .on_action(cx.listener(|this, _: &ConfirmDefault, _, cx| this.accept(cx)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(DismissEvent);
            }))
            .w(px(CONFIRM_WIDTH))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .bg(theme::popup())
            .child(ui::label(
                self.asked.title.clone(),
                text::TITLE,
                theme::ink(),
            ))
            .child(ui::label(
                self.asked.detail.clone(),
                text::BODY,
                theme::ink2(),
            ))
            .child(
                div()
                    .mt(px(8.0))
                    .h(px(theme::height::H28))
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .rounded(px(theme::radius::CONTROL))
                    .bg(theme::card())
                    .border_1()
                    .border_color(theme::line2())
                    .child(self.field.clone()),
            )
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        ui::button("words-cancel", "Cancel").on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(DismissEvent)),
                        ),
                    )
                    .child(
                        ui::button("words-action", self.asked.action.clone())
                            .primary()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.accept(cx))),
                    ),
            )
    }
}
