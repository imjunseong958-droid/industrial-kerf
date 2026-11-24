//! The conversation with Kerf: what you asked, what Kerf did and said, the
//! cut it proposes, and the composer.
//!
//! Your request and Kerf's answer fade in where they are; the list keeps to
//! the latest unless you scrolled away. While Kerf works, under your request
//! "Working" has a band of light passing over it once each `WORK_SHIMMER`
//! (still under Reduce Motion), with the time so far and the steps it took;
//! done, it folds to "Worked for …", which unfolds its steps, its chevron
//! turning over `FADE`.
//!
//! What Kerf writes reads as written: bold, code, and the timecodes it names
//! as links that move the playhead there. Under a finished answer, Copy; under
//! a failed or stopped one, Ask again. A request written while Kerf works
//! waits over the composer ("Next: …") and goes when the turn ends.
//!
//! Proposal cards say what each proposal does and the clips it changes (a
//! colour change on one clip in a line of its own), with its versions; the
//! open one nearest the playhead is pinned. A proposal has one card, under
//! the newest turn that names it; an earlier one that named it says so in a
//! line. Esc puts a menu of the conversation away first.
//!
//! The composer's chips name the folder, the model (Claude Code's chosen
//! there) and what Kerf does without asking, that one with its mode's icon
//! and tone ([`permission_look`]). Where nothing can be asked (no
//! folder, kerfd gone, no key or no Kerf account) its placeholder says so,
//! a hover says why, and Send is off. A request written while Kerf works
//! keeps the playhead time it was written at.

mod asking;
mod cards;
mod composer;
mod menus;
mod picture;
mod prose;
mod rim;
mod script;
mod spans;
mod turns;
mod welcome;
mod working;

pub use asking::{access_reason, join_queued, refusal, Queued, Refusal};
pub use cards::{
    previewing, settled_age, settled_age_changes, settled_words, strays_note, PREVIEW_TIP,
    SCRIPT_TIP, VERSIONS_TIP,
};
pub use composer::{
    check_suggestions, over_composer, COMPOSER_MOST, FOLDER_CHIP_MOST, OVER_COMPOSER,
    OVER_COMPOSER_RISE,
};
pub use editor_state::book::{
    card_turns, changes_span, distance, pin_order, ProposalBook, Version,
};
pub use editor_state::prefs::model_label;
/// The conversation while what a proposal changed is watched: it falls
/// back, so the picture holds the eye.
pub use editor_ui::theme::motion::CHAT_BEHIND;
pub use menus::{
    model_choices, next_permissions, permission_look, permission_words, say_permissions, MODE_HOLD,
};
pub use picture::{
    card_picture_size, cut_frames, finish_drawn, finishing, in_sight, length_badge, shown_frame,
    Finish, CARD_CROSSFADE, CARD_DONE, CARD_DONE_FADE, CARD_MARKS_AFTER, CARD_MARKS_FADE,
    CARD_PICTURE_RING, CARD_PICTURE_TALLEST, CARD_PICTURE_WIDEST, CARD_PLAY_MARK, CARD_RING,
    CARD_TURN,
};
pub use rim::rim_point;
pub use script::{
    clock_time, colour_change, describe_script_line, effect_label, media_stem, script_seconds,
    script_summary, script_words, signed_seconds,
};
pub use spans::{spans, timecode_position, timecode_seconds, Span};
pub use turns::{request_before, stamp_tip, work_length, worked_headline, COPIED_HOLD};
pub use welcome::{recent_conversations, try_asking, WELCOME_COLUMN};
pub use working::{draft_dot, work_clock, DRAFT_BEAT, FOOTAGE_MOST, FOOTAGE_TILE};

use cards::CardKeys;
use menus::Menu;
use turns::Cards;

use editor_session::session::{BridgeEvent, Session};
use editor_state::model::Proposal;
use editor_state::store::Change;
use editor_ui::icons::Icon;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::motion::{self, Tween};
use editor_ui::selectable::{self, Selections};
use editor_ui::theme::{self, layout, motion as scene, text};
use editor_ui::ui;
use gpui::{
    anchored, deferred, div, prelude::*, px, radians, AnimationExt, App, ClipboardItem, Context,
    Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, Rgba, ScrollHandle, SharedString,
    Subscription, Task, Transformation, Window,
};
use kerf_bridge::event::{AgentState, Check};
use kerf_bridge::Event;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// The key context of the composer's box, where Shift-Tab changes what
/// Kerf does without asking.
pub const COMPOSER_CONTEXT: &str = "Composer";

gpui::actions!(chat, [CyclePermissions]);

/// The composer's keys: `keymap::bind` binds them right after the
/// field's ([`editor_ui::input::key_bindings`]) whenever it binds a keymap.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        // In the composer, what Kerf does without asking goes round
        // (after the field's own Shift-Tab, so it comes first there).
        KeyBinding::new(
            "shift-tab",
            CyclePermissions,
            Some(format!("{COMPOSER_CONTEXT} > {}", editor_ui::input::KEYS).as_str()),
        ),
    ]
}

/// What the conversation asks of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatEvent {
    /// A cut's card was pressed: its picture grows out of the card into the
    /// stage (`proposal` names the cut).
    GrowPicture { proposal: String },
}

// ---------------------------------------------------------------- the view

pub struct ChatView {
    session: Entity<Session>,
    composer: Entity<TextInput>,
    makings: Entity<editor_making::making::Makings>,
    scroll: ScrollHandle,
    /// Proposals whose script shows, by proposal id.
    scripts: HashSet<String>,
    /// What folds (a turn's steps, a card's changes), each with its
    /// chevron's turn: 1 open.
    folds: HashMap<String, Tween>,
    /// The version a card shows where it is not the newest, by proposal id.
    shown: HashMap<String, String>,
    book: ProposalBook,
    /// A request written while Kerf worked, to go when the turn ends, and
    /// the playhead time it goes with.
    queued: Queued,
    /// The turn under way was stopped, here or from a menu or the palette,
    /// and has not ended yet.
    stopping: bool,
    /// The answer just copied, which says so for a moment.
    copied: Option<String>,
    menu: Option<Menu>,
    /// The open menu's rows, as the kit's menu (all but the context card's).
    kit_menu: Option<Entity<ui::Menu>>,
    /// Esc just put a menu of the conversation away: that Esc is the
    /// menu's alone, and the window's own Esc order takes it as a menu's
    /// ([`ChatView::take_escape`]), not as a render's to stop.
    escape_took_menu: bool,
    /// The menu of the conversation's words (Copy, Select All) a
    /// right-click on them opened, and where, while it is open.
    words_menu: Option<(Entity<ui::Menu>, gpui::Point<gpui::Pixels>)>,
    /// The menu a press outside it closed, and when: that press on its own
    /// chip does not open it again.
    menu_closed: Option<(Menu, Instant)>,
    placeholder: SharedString,
    /// Why nothing can be asked now (its placeholder, and the reason), as
    /// last worked out; the connection's access (None: it can take
    /// requests) it was worked out for, and the registry's reason for it.
    refusal: Option<Refusal>,
    access: Option<Option<String>>,
    ask_reason: Option<String>,
    /// Under Reduce Motion the working clock goes on once a second: the one
    /// timer that says so, while Kerf works ([`Self::follow_clock`]).
    clock: Option<Task<()>>,
    /// What the last check found failing, as requests to fill the composer
    /// with: the words shown, and the request.
    fixes: Vec<(String, String)>,
    /// The keys a card's Accept and Reject name, as the keymap they were
    /// read from has them: read as a card first names them, and again once
    /// the keymap has changed.
    keys: std::cell::RefCell<Option<CardKeys>>,
    /// The Ask popover, while it is open.
    ask: Option<Entity<crate::ask::AskView>>,
    /// The cut whose picture went to the stage from its card.
    stage_proposal: Option<String>,
    /// Whether the picture showed as last seen.
    picture_seen: Option<bool>,
    /// A cut's picture under the pointer: the proposal, and how far across.
    scrub: Option<(String, f32)>,
    /// Where each cut's picture was last laid out, by proposal id, for the
    /// pointer's place over it.
    pictures: std::rc::Rc<std::cell::RefCell<HashMap<String, gpui::Bounds<gpui::Pixels>>>>,
    /// The first cut Kerf is making, while it does; and the one it just
    /// finished, and when, which says Done for a moment.
    building: Option<String>,
    built: Option<(String, Instant)>,
    /// The poster a building cut showed before its newest clip, and when
    /// the newest began to show over it.
    turning: Option<(String, Instant)>,
    /// The media of the newest clip a building cut's picture shows.
    newest: Option<String>,
    /// The footage gathered as Kerf starts on a project with no cut, and
    /// when it gathered.
    footage: Option<(Vec<String>, Instant)>,
    /// Whether footage is to gather for the turn under way, once its
    /// posters have come.
    footage_wanted: bool,
    /// The turn last seen starting.
    turn_seen: Option<Instant>,
    /// When the view began: what repeats goes on from here.
    started_at: Instant,
    /// How far the conversation sits back while a change reel plays.
    behind: Tween,
    /// What is chosen of the requests' and answers' words.
    selections: selectable::Shared,
    /// Where each footage tile was last laid out, by its media, for what
    /// goes on from it to fly from; and the tiles that have gone on.
    tiles: std::rc::Rc<std::cell::RefCell<HashMap<String, gpui::Bounds<gpui::Pixels>>>>,
    taken: HashSet<String>,
    /// Where the composer was last laid out.
    composer_at: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    focus_handle: FocusHandle,
    /// Drawn again as a settled card's age ("· 4 min") turns over.
    ages: editor_ui::motion::Redraw,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChatEvent> for ChatView {}

const PLACEHOLDER: &str = "Ask Kerf to cut, trim, find, or make…";

impl ChatView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextInput::new(cx, PLACEHOLDER)
                .multiline()
                .max_height(px(COMPOSER_MOST))
        });
        let composer_focus = composer.read(cx).focus_handle(cx);
        let selections = Selections::shared();
        let playhead = session.read(cx).playhead.clone();
        let subscriptions = vec![
            // The card pinned is the one nearest the playhead, and the
            // composer's struck-through stamp says its time.
            cx.observe(&playhead, |_, _, cx| cx.notify()),
            // What the view follows of the session, followed as it changes.
            cx.observe(&session, |this, _, cx| {
                this.follow_session(cx);
                cx.notify()
            }),
            // Run QC is offered while the box has the keys and is empty.
            cx.on_focus(&composer_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&composer_focus, window, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                this.hear_change(change, cx)
            }),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.hear_bridge(event, cx)
            }),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| this.hear_composer(event, window, cx),
            ),
            copy_keys(&selections, window, cx),
            menus::escape_keys(window, cx),
        ];
        // A right-click on the conversation's words opens their menu.
        {
            let this = cx.weak_entity();
            selections
                .borrow_mut()
                .on_menu(std::rc::Rc::new(move |run, at, window, cx| {
                    let _ = this.update(cx, |chat, cx| chat.open_words_menu(run, at, window, cx));
                }));
        }
        let makings = cx.new(|cx| editor_making::making::Makings::new(session.clone(), window, cx));
        // Each turn draws the cards of what it asked to have made; the list's
        // foot only those heard as Kerf works, no turn holding them yet.
        makings.update(cx, |makings, cx| {
            makings.set_cards_in_turns(true, cx);
            // Their plans' words are chosen and copied with the conversation's.
            makings.share_selections(selections.clone());
        });
        // A conversation opens at its latest.
        let scroll = ScrollHandle::new();
        scroll.scroll_to_bottom();
        let mut view = Self {
            session,
            composer,
            makings,
            scroll,
            scripts: HashSet::new(),
            folds: HashMap::new(),
            shown: HashMap::new(),
            book: ProposalBook::default(),
            queued: Queued::default(),
            stopping: false,
            copied: None,
            menu: None,
            escape_took_menu: false,
            words_menu: None,
            kit_menu: None,
            menu_closed: None,
            placeholder: PLACEHOLDER.into(),
            refusal: None,
            access: None,
            ask_reason: None,
            clock: None,
            fixes: Vec::new(),
            keys: Default::default(),
            ask: None,
            stage_proposal: None,
            picture_seen: None,
            scrub: None,
            pictures: Default::default(),
            building: None,
            built: None,
            turning: None,
            newest: None,
            footage: None,
            footage_wanted: false,
            turn_seen: None,
            started_at: Instant::now(),
            behind: Tween::at(1.0),
            selections,
            tiles: Default::default(),
            taken: HashSet::new(),
            composer_at: Default::default(),
            focus_handle: cx.focus_handle(),
            ages: editor_ui::motion::Redraw::default(),
            _subscriptions: subscriptions,
        };
        view.follow_session(cx);
        view
    }

    /// Follows the session as it changes, before the conversation is drawn
    /// again: whether a request can go, the turn under way and the first
    /// cut it makes ([`ChatView::follow_turn`]), and whether the
    /// conversation falls back for a change reel.
    fn follow_session(&mut self, cx: &mut Context<Self>) {
        self.refresh_placeholder(cx);
        self.follow_turn(cx);
        self.fall_back(cx);
    }

    /// While what a proposal changed plays, the conversation falls back to
    /// `CHAT_BEHIND` over `SETTLE`, and comes forward as the reel ends.
    fn fall_back(&mut self, cx: &App) {
        let watching = self.session.read(cx).reel().is_some();
        let behind = if watching { CHAT_BEHIND } else { 1.0 };
        if (self.behind.target() - behind).abs() > f32::EPSILON {
            self.behind.go(behind, scene::SETTLE);
        }
    }

    /// What the session says changed: a turn ended, and what waited goes;
    /// and while the list is at its foot, it keeps to the latest.
    fn hear_change(&mut self, change: &Change, cx: &mut Context<Self>) {
        if *change == Change::TurnEnded {
            self.turn_ended(cx);
        }
        if matches!(
            change,
            Change::Conversation(_) | Change::Work | Change::TurnEnded
        ) && self.near_bottom()
        {
            self.scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    /// What the bridge says that the conversation keeps: the proposals'
    /// book, what a check found failing, Kerf's account, and a turn stopped
    /// from anywhere.
    fn hear_bridge(&mut self, event: &BridgeEvent, cx: &mut Context<Self>) {
        match &*event.0 {
            Event::State { snapshot } => {
                self.book = ProposalBook::read(snapshot);
                cx.notify();
            }
            Event::Check(Check::Done { summary, .. }) => {
                // A check's failing rows are offered as what to ask next.
                let fps = self.session.read(cx).timeline().rate();
                self.fixes = check_suggestions(summary, fps);
                cx.notify();
            }
            Event::Cloud(status) => {
                // Kerf's account, as every window keeps it, says whether
                // Kerf can be asked.
                editor_session::session::hear_cloud(status, cx);
                self.access = None;
                self.refresh_placeholder(cx);
                cx.notify();
            }
            Event::AgentState {
                state: AgentState::Aborted,
                ..
            } => {
                // Stopped from anywhere (here, a menu, the palette). The
                // turn says "completed" after this, so it is kept here
                // until the turn ends, which only a turn under way says.
                self.stopping = true;
                cx.notify();
            }
            _ => {}
        }
    }

    fn near_bottom(&self) -> bool {
        let offset = -f32::from(self.scroll.offset().y);
        let most = f32::from(self.scroll.max_offset().height);
        most - offset < 48.0
    }

    fn is_open(&self, key: &str) -> bool {
        self.folds
            .get(key)
            .is_some_and(|tween| tween.target() > 0.5)
    }

    fn toggle_fold(&mut self, key: &str, cx: &mut Context<Self>) {
        let open = self.is_open(key);
        self.folds
            .entry(key.to_owned())
            .or_insert_with(|| Tween::at(0.0))
            .go(if open { 0.0 } else { 1.0 }, scene::FADE);
        cx.notify();
    }

    /// A fold's chevron, down when closed and turned over when open.
    fn chevron(&self, key: &str, color: Rgba, window: &Window) -> gpui::Svg {
        let turn = self
            .folds
            .get(key)
            .map_or(0.0, |tween| tween.sample(window));
        Icon::ChevronDown
            .view(12.0, color)
            .with_transformation(Transformation::rotate(radians(std::f32::consts::PI * turn)))
    }

    /// The conversation's list: a new one's welcome, its turns, the request
    /// under way and what Kerf is doing, the open proposals no turn shows
    /// yet (the pinned first), and the cards of what Kerf is asked to make.
    fn list(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let session = self.session.read(cx);
        let turns = session
            .state
            .open_conversation
            .as_ref()
            .map(|c| c.items.clone())
            .unwrap_or_default();
        let proposals = session.state.snapshot.proposals.clone();
        let request = session.state.work.request.clone();
        let busy = session.state.work.busy();
        // The turn under way shows in its own conversation only (a request
        // sent with none open goes on in the new one it starts).
        let showing_this = session.state.work.conversation == session.conversation;
        // A conversation chosen whose turns have not come yet: its title is
        // in the toolbar, and nothing here says it is empty.
        let loading = session.state.chosen.is_some() && session.state.open_conversation.is_none();
        let pins = pin_order(
            &proposals,
            &self.book,
            session.timeline(),
            session.state.snapshot.proposal_timeline.as_ref(),
            session.state.position,
        );
        let pinned = pins.first().cloned();
        // An open proposal no turn shows yet shows at the end, the pinned first.
        let holders = card_turns(&turns, &proposals, &self.book);
        let loose = self.loose(&pins, &holders, &proposals);
        // A turn under way elsewhere leaves a new conversation as it is.
        let live_here = showing_this && (busy || request.is_some());
        let nothing = !loading && turns.is_empty() && !live_here && loose.is_empty();

        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .pt(px(24.0))
            .pb(px(24.0));
        if nothing {
            list = list.child(self.empty(cx));
        }
        let cards = Cards {
            proposals: &proposals,
            holders: &holders,
            pinned: pinned.as_deref(),
        };
        for index in 0..turns.len() {
            list = list.child(self.turn(index, &turns, &cards, window, cx));
        }
        if let Some(request) = request.filter(|_| showing_this) {
            list = list.child(self.you(&request, None, "live", cx).with_animation(
                "request",
                motion::animation(scene::APPEAR),
                |turn, t| turn.opacity(t),
            ));
        }
        if busy
            && showing_this
            && let Some(working) = self.working(window, cx)
        {
            list = list.child(working);
        }
        if !loose.is_empty() {
            // While a turn runs, what it stages has no turn yet to sit under.
            if !busy {
                list = list.child(ui::label(strays_note(&loose), text::BODY, theme::ink3()));
            }
            for proposal in &loose {
                let pinned = pinned.as_deref() == Some(proposal.proposal_id.as_str());
                list = list.child(self.proposal_card(proposal, pinned, window, cx));
            }
        }
        list.child(self.makings.clone())
    }

    /// The open proposals in pin order (`pins`) that no turn holds the
    /// card of (`holders`), but the cut Kerf is making.
    fn loose(
        &self,
        pins: &[String],
        holders: &HashMap<String, usize>,
        proposals: &[Proposal],
    ) -> Vec<Proposal> {
        pins.iter()
            .filter(|id| !holders.contains_key(*id))
            // The cut Kerf is making is its card at work, in the live answer.
            .filter(|id| {
                self.building
                    .as_deref()
                    .is_none_or(|building| !self.book.same(building, id))
            })
            .filter_map(|id| {
                proposals
                    .iter()
                    .find(|proposal| &proposal.proposal_id == id)
            })
            .cloned()
            .collect()
    }

    /// The list, scrolling in the conversation's column.
    fn scroller(&self, list: gpui::Div) -> impl IntoElement + use<> {
        div()
            .id("conversation")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div().w_full().flex().justify_center().px(px(24.0)).child(
                    // Its words wrap to the column: what they would be
                    // on one line never widens it.
                    div()
                        .w_full()
                        .min_w_0()
                        .max_w(px(layout::CONVERSATION_COLUMN))
                        .child(list),
                ),
            )
    }
}

/// ⌘C (Ctrl+C) copies the words chosen in the conversation (`chosen`),
/// wherever the keys are: a press anywhere else would have let them go,
/// and so does any other key a field takes (what is typed or chosen there
/// is then what is copied).
fn copy_keys(
    chosen: &selectable::Shared,
    window: &Window,
    cx: &mut Context<ChatView>,
) -> Subscription {
    let chosen = chosen.clone();
    let handle = window.window_handle();
    cx.intercept_keystrokes(move |event, window, cx| {
        if window.window_handle() != handle {
            return;
        }
        let keystroke = &event.keystroke;
        let copy = keystroke.key == "c"
            && keystroke.modifiers.secondary()
            && !keystroke.modifiers.shift
            && !keystroke.modifiers.alt;
        if !copy {
            let in_a_field = event
                .context_stack
                .iter()
                .any(|context| context.contains(editor_ui::input::CONTEXT));
            if in_a_field && chosen.borrow_mut().clear() {
                window.refresh();
            }
            return;
        }
        let Some(words) = chosen.borrow().copied() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(words));
        cx.stop_propagation();
    })
}

impl Focusable for ChatView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.follow_newest(cx);
        self.follow_clock(cx);
        // What is laid out this frame says again where it is.
        self.selections.borrow_mut().begin_frame();
        self.pictures.borrow_mut().clear();
        self.tiles.borrow_mut().clear();
        let list = self.list(window, cx);
        let composer = self.composer(window, cx);
        let ask = self.ask.clone();
        // While what a proposal changed plays, the conversation falls back
        // (`fall_back`, as the reel starts and ends).
        let opacity = self.behind.sample(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .opacity(opacity)
            .children(ask)
            .child(self.scroller(list))
            // Its fades: where an answer is cut at the top, and at the foot
            // while there is more below.
            .child(ui::scroll_fade(&self.scroll, theme::surround()))
            .child(self.composer_foot(composer))
            .children(self.words_menu.as_ref().map(|(menu, at)| {
                deferred(
                    anchored()
                        .position(*at)
                        .snap_to_window_with_margin(px(theme::space::S8))
                        .child(menu.clone()),
                )
                .with_priority(2)
            }))
    }
}
