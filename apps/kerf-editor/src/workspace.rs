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

mod app_menu;
mod asked;
mod bars;
mod events;
mod flights;
mod keys;
mod modal;
mod panel;
mod projects;
mod run;
mod title;
mod toolbar;

pub use app_menu::{app_menu_place, app_menu_room};
pub use asked::{
    sync_params, sync_question, AskWords, CloseConfirm, Confirm, ConfirmDefault, WordsAsked,
};
pub use bars::{
    bar_takes_error, conversation_error, first_line, kept_in_bar, notice_bottom, readable_error,
    standing_problem, sync_failed, StandingInput,
};
pub use flights::{
    back_flight_at, clip_flight_at, export_flight_at, grow_flight_at, grow_shows, home_flight_at,
    landing_in, lerp_bounds, let_go_flight_at, placed_since, FlightKind, PaneGoes, PlacedClip,
};
pub use keys::{escape_goes, holds_keys_itself, settle_keys, EscapeAt, EscapeGoes, Holder, KeysGo};
pub use panel::{panel_heights, panes_opening, picture_in, picture_within, timeline_split};
pub use projects::{
    folder_media, open_conversation_in, open_folder, open_folder_in, open_folder_with,
    window_title, FOLDER_MEDIA,
};
pub use run::frame_step;
pub use title::{
    edited_ago, edited_ago_changes, edited_ago_changes_in, edited_ago_in, local_day, title_tip,
};
pub use toolbar::{running_words, toolbar_places, ToolbarPlaces};

/// How long before the picture has grown the timeline starts to come in,
/// and how long its stand-in takes to fade (kept here for the tests and
/// views that name them so).
pub use crate::stage::{render_card_shows, render_stop_holds, RenderCard};
pub use crate::theme::motion::{STAND_IN_MOST, STAND_IN_RELEASE, TIMELINE_LEAD};

use crate::chat::ChatView;
use crate::commands::WORKSPACE;
use crate::export::ExportView;
use crate::input::TextInput;
use crate::media::MediaBand;
use crate::motion::Tween;
use crate::session::Session;
use crate::sidebar::SidebarView;
use crate::stage::StageView;
use crate::theme::{self, layout, motion as scene, text};
use crate::timeline::TimelineView;
use crate::ui;
use bars::{ErrorBar, BAR};
use flights::{flown, Flight, PendingClip};
use gpui::{
    div, prelude::*, px, AnyElement, AnyView, App, Context, Entity, FocusHandle, Focusable, Pixels,
    Subscription, Window,
};
use panel::StageSlot;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use title::under_title;
use toolbar::export_popover;

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
    /// The window is too narrow for the sidebar (as it was last sized): it
    /// gives way, whatever its switch says.
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
    /// Where the stage was laid out last, in the window (None: it was not),
    /// and where the panel's left edge was drawn then and was to come to
    /// rest: kept as the stage is laid out.
    stage_slot: Rc<Cell<Option<StageSlot>>>,
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
    /// Drawn again as the title tip's "Edited 2 min ago" turns over.
    ages: crate::motion::Redraw,
    _subscriptions: Vec<Subscription>,
}

/// Where the window's regions are this frame, each where its tween has
/// got to: worked out as the window draws, and never kept.
struct Regions {
    window_height: f32,
    sidebar_width: f32,
    /// How far the sidebar has come in (none while it gives way).
    sidebar: f32,
    /// The main column, right of the sidebar.
    main_left: f32,
    main_width: f32,
    /// The bars under the toolbar, and the room under them.
    bars_height: f32,
    content_height: f32,
    /// How far the panel has come in, and its room beside the rail the
    /// conversation keeps.
    panel: f32,
    panel_width: f32,
    conversation_width: f32,
    /// How far the stage reaches over the conversation while changes are
    /// watched.
    reach: f32,
    /// The toolbar is at the window's leading edge (the sidebar away).
    leading: bool,
    /// The chosen clip's inspector stands beside the picture.
    inspecting: bool,
    /// The picture's shape, which splits the panel.
    aspect: f32,
    /// Where the toolbar's controls and their popups start.
    places: ToolbarPlaces,
    title_left: f32,
    /// Where the panel's left edge is drawn, and where it comes to rest.
    panel_left: f32,
    rest_left: f32,
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
        let mut subscriptions = Self::follow_session(&session, &stage, &collab, window, cx);
        subscriptions.extend(Self::follow_views(
            &sidebar,
            &timeline,
            &chat,
            &title_field,
            window,
            cx,
        ));
        subscriptions.extend(Self::follow_window(&title_focus, window, cx));
        subscriptions.push(Self::escape_first(window, cx));
        let focus_handle = cx.focus_handle();
        // A press on what takes no keys (the conversation's empty room, a
        // chip, a label) gives them to the window's own box: they settle
        // again, on the timeline while it shows, so its keys still work
        // after the press.
        subscriptions.push(cx.on_focus(&focus_handle, window, |this, _, cx| this.settle_soon(cx)));
        window.focus(&focus_handle);
        let mut this = Self {
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
            ages: crate::motion::Redraw::default(),
            _subscriptions: subscriptions,
        };
        // What the window's first frame would otherwise have had to set: the
        // sidebar given way in a window too narrow for it, the keys the
        // toolbar's tips name, the stage reaching over a change being
        // watched, and whether the picture shows, told to the session.
        this.follow_window_width(window, cx);
        this.refresh_keys(cx);
        this.follow_reel(cx);
        this.tell_picture_open(cx);
        this
    }

    pub fn session(&self) -> &Entity<Session> {
        &self.session
    }

    /// Whether Kerf works in conversation `id` now.
    fn conversation_working(&self, id: &str, cx: &App) -> bool {
        let session = self.session.read(cx);
        let work = &session.state.work;
        work.busy()
            && (work.conversation.as_deref() == Some(id)
                || (work.conversation.is_none() && session.conversation.as_deref() == Some(id)))
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

    /// In a window too narrow for it, the sidebar gives way, and the
    /// toolbar has its menu. Given way, it is put away: its titles' light
    /// rests. Followed as the window is resized.
    fn follow_window_width(&mut self, window: &Window, cx: &mut Context<Self>) {
        let gives_way = f32::from(window.viewport_size().width) < layout::SIDEBAR_WINDOW_LEAST;
        if gives_way != self.sidebar_gives_way {
            let shown = self.sidebar_open && !gives_way;
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_shown(shown, cx));
        }
        self.sidebar_gives_way = gives_way;
    }

    fn sidebar_width(&self, session: &Session) -> f32 {
        session
            .prefs
            .sidebar_width
            .clamp(layout::SIDEBAR_LEAST, 900.0)
    }

    /// Where the window's regions are this frame, with `bars` bars under
    /// the toolbar: each where its tween has got to.
    fn regions(&self, bars: usize, window: &Window, cx: &App) -> Regions {
        let session = self.session.read(cx);
        let viewport = window.viewport_size();
        let (window_width, window_height) = (f32::from(viewport.width), f32::from(viewport.height));
        let sidebar_width = self.sidebar_width(session);
        let sidebar = if self.sidebar_gives_way {
            0.0
        } else {
            self.sidebar_shown.sample(window)
        };
        let reach = self.reel_reach.sample(window);
        let panel = self.panel_shown.sample(window);
        let main_left = sidebar_width * sidebar;
        let main_width = (window_width - main_left).max(0.0);
        let bars_height = BAR * bars as f32;
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
        let inspecting = !session.selection.is_empty() && panel_width > layout::INSPECTOR * 2.2;
        // The picture's shape, which splits the panel.
        let aspect = crate::stage::shown_aspect(session);
        // Where the title and ⋯ are, for their menus to open under them.
        let places = toolbar_places(leading, crate::chrome::window_buttons_room(window));
        Regions {
            window_height,
            sidebar_width,
            sidebar,
            main_left,
            main_width,
            bars_height,
            content_height,
            panel,
            panel_width,
            conversation_width,
            reach,
            leading,
            inspecting,
            aspect,
            places,
            title_left: main_left + places.title,
            // Where the panel's left edge is drawn, and where it comes to rest.
            panel_left: main_left + conversation_width - reach,
            rest_left: main_left + main_width - panel_width,
        }
    }

    /// The sidebar, sliding in from the window's edge.
    fn sidebar_slot(&self, regions: &Regions) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-regions.sidebar_width * (1.0 - regions.sidebar)))
            .w(px(regions.sidebar_width))
            // Put away and at rest off the window's edge, its rows are
            // no Tab stops.
            .when(regions.sidebar <= 0.0, |away| away.invisible())
            // Its edge (drag, press, double-click) is the sidebar's own.
            .child(self.sidebar.clone())
    }

    /// The main column, right of the sidebar: the toolbar, the bars under
    /// it, and the conversation with the panel beside it. `shows` is how
    /// much of the stage and of the timeline the panel draws.
    fn main_column(
        &self,
        regions: &Regions,
        bars: Vec<AnyElement>,
        shows: (f32, f32),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(regions.main_left))
            .w(px(regions.main_width))
            .flex()
            .flex_col()
            .child(self.toolbar(regions.leading, window, cx))
            .children(bars)
            .child(
                div()
                    .relative()
                    .h(px(regions.content_height))
                    .w_full()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(px(regions.conversation_width))
                            .child(self.chat.clone()),
                    )
                    .when(regions.panel > 0.001, |content| {
                        content.child(
                            // The panel comes in from the right, its left edge
                            // against the conversation's right.
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(regions.conversation_width - regions.reach))
                                .w(px(regions.panel_width + regions.reach))
                                .pr(px(8.0))
                                .pb(px(8.0))
                                // Reaching over the conversation, the
                                // words under it take no press.
                                .occlude()
                                .child(self.panel(regions, shows, window)),
                        )
                    }),
            )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Where the stage was laid out last frame, if it was: this frame
        // lays it out again, or not.
        let stage_drawn = self.stage_slot.take();
        // A region folded away gave back the keys it held: they settle now
        // that it is laid out as it is.
        if std::mem::take(&mut self.settle_soon) {
            cx.defer_in(window, |this, window, cx| this.settle_focus(window, cx));
        }
        // The menu bar greys and ticks the registry's rows as this window,
        // in front, has them.
        if cfg!(target_os = "macos") && window.is_window_active() {
            let states = self.menu_states(cx);
            let project = self.session.read(cx).project.clone();
            crate::commands::set_menu_states(states, project.as_deref(), cx);
        }
        let bars = self.bars(cx);
        let regions = self.regions(bars.len(), window, cx);
        // Where the picture will be once the panel has come to rest, for
        // what grows into it.
        let picture_home = self.picture_home(stage_drawn, &regions, cx);
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

        Self::actions(
            div().key_context(WORKSPACE).track_focus(&self.focus_handle),
            cx,
        )
        .size_full()
        .relative()
        .overflow_hidden()
        .bg(theme::surround())
        .font(theme::ui_font(gpui::FontWeight::NORMAL))
        .text_color(theme::ink())
        .child(self.sidebar_slot(&regions))
        .child(self.main_column(&regions, bars, shows, window, cx))
        .when_some(self.export.clone(), |root, export| {
            root.child(export_popover(export))
        })
        .when_some(self.title_menu.clone(), |root, menu| {
            root.child(under_title(regions.title_left, menu))
        })
        .when(self.activity_open, |root| {
            root.child(under_title(regions.title_left, self.activity_view(cx)))
        })
        .when_some(self.app_menu.clone(), |root, (menu, from_account)| {
            root.child(self.app_menu_layer(menu, from_account, &regions, window))
        })
        // What travels over the window: frames going home, clips
        // flying to the timeline, the picture growing or going back.
        .children(flights.into_iter().map(flown))
        .children(self.modal(window, cx))
        // Notices sit over the conversation, above what is open over the window.
        .child(
            div()
                .absolute()
                .top(px(layout::TOOLBAR + regions.bars_height))
                .bottom_0()
                .left(px(regions.main_left))
                .w(px(regions.conversation_width))
                .child(self.notices(window, cx)),
        )
    }
}

/// Pixels as points.
pub fn points(value: Pixels) -> f32 {
    f32::from(value)
}

/// How far the stage grows over the conversation while changes are
/// watched.
use crate::theme::motion::REEL_REACH;
