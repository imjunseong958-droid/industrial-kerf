//! The sidebar: the project's conversations, the new one, a search over
//! them, the archived ones folded at the end, and at its foot the account,
//! which opens the app's menu, and Settings.
//!
//! A conversation Kerf works in has a band of light passing over its title
//! once each `TITLE_SHIMMER`, then resting; under Reduce Motion the title is
//! simply in ink. Done while you looked elsewhere, waiting for you, or
//! failed: a dot at the row's end, there at once. A conversation new to the
//! list fades in where it is; archived conversations fade in where they
//! are, one after another, as they unfold. A title is renamed where it is
//! (Rename… in its menu, or F2), and deleting one asks first, over the
//! window.
//!
//! The open folder's name opens the folders and the ways to another; each
//! other folder has its own menu (Remove from Sidebar). The account at the
//! foot says who chat runs on, and its menu has Sign Out while signed in to
//! Kerf. The edge, pressed once, puts the sidebar away after a double-click's
//! wait; pressed twice, its width goes back. Tab reaches the rows and
//! buttons from the window, with a ring; Enter or Space presses them,
//! Shift+F10 or the Menu key opens a row's menu, the arrows move in menus.

mod account;
mod ages;
mod app_menu;
mod conversation_files;
mod edge;
mod entries;
mod folders;
mod keys;
mod list;
mod menus;
mod row;

pub use account::{account_line, account_of};
pub use ages::{parse_time, relative_time, relative_time_changes};
pub use app_menu::{
    app_menu_items, app_rows, sync_problem, sync_summary, with_sync_failure, AppDoes, AppMenuInput,
    AppRow, ChooseKeymap, SignOut, SwitchBranch, SYNC_FAILURE,
};
pub use conversation_files::{conversation_summary, folder_conversations, CONVERSATIONS};
pub use edge::{is_second_press, EDGE_TIP};
pub use entries::{
    folder_entries, menu_step, other_folder_entries, recent_folders, reveal_words, row_entries,
    Entry, MenuEntry,
};
pub use folders::forget_folder;
pub use keys::{
    bind_keys, key_bindings, DeleteRow, FocusNext, FocusPrevious, Fold, MenuChoose, MenuClose,
    MenuDown, MenuUp, OpenMenu, RenameRow, Unfold, CONTEXT, FOLDER, FOLDER_KEYS, ITEM, MENU, ROW,
    ROW_KEYS,
};
pub use list::{matches, row_state, shown, More, RowState, SHOWN_AT_FIRST};
pub use menus::{menu_item, menu_panel, menu_separator, trigger};
pub use row::{renamed, shimmer_step, TITLE_SHIMMER_STEPS};

use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::ConversationSummary;
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, layout, text};
use crate::ui::{self, icon_button};
use edge::EdgePress;
use gpui::{
    div, prelude::*, px, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable,
    MouseButton, Pixels, Point, Subscription, Task, Window,
};
use kerf_bridge::Event;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

pub enum SidebarEvent {
    Settings,
    Hide,
    /// Delete this conversation, once the person has said so.
    Delete(String),
    /// A field the sidebar had the keys in is done: the window takes them back.
    Done,
    /// One of its menus opened: the window puts its own away.
    MenuOpened,
    /// The account was pressed (`from_keys`: with Enter, Space, Shift+F10
    /// or the Menu key): the window opens the app's menu above it, or puts
    /// it away when it is open.
    AppMenu {
        from_keys: bool,
    },
    /// A conversation was chosen, or New conversation: the window gives
    /// the composer the keys, so what is typed next is the request.
    Chosen,
    /// Another folder's conversation `id` (None: a new one) was chosen: the
    /// window has it opened in `folder`'s window once `OpenRecent`,
    /// dispatched right after, has run
    /// ([`crate::workspace::open_conversation_in`]).
    OpenConversationIn {
        folder: PathBuf,
        id: Option<String>,
    },
}

impl EventEmitter<SidebarEvent> for SidebarView {}

/// Which of the sidebar's menus is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarMenu {
    /// A conversation's, by its id.
    Row(String),
    /// The open folder's name: the folders, and the ways to another.
    Folder,
    /// Another folder's, by its path.
    Other(PathBuf),
}

pub struct SidebarView {
    session: Entity<Session>,
    /// The conversations' list, for its fades.
    list_scroll: gpui::ScrollHandle,
    archived_open: bool,
    /// How many times the archived ones were unfolded: each unfolding
    /// staggers them in afresh.
    unfoldings: u64,
    /// The menu open, if one is, and the item of it the arrows are on.
    menu: Option<SidebarMenu>,
    menu_chosen: Option<usize>,
    /// An open menu holds the keys; it gives them back to what had them
    /// (`menu_return`) when it closes from the keys or by a choice.
    menu_focus: FocusHandle,
    menu_return: Option<FocusHandle>,
    /// Where a right-click opened the menu open, in the window: it opens
    /// at the pointer there, as a context menu does. None: anchored under
    /// its row (its ⋯ button, the keys).
    menu_at: Option<Point<Pixels>>,
    /// Keys to give back once the menu that had them is gone.
    refocus: Option<FocusHandle>,
    /// An item was chosen from the keys: the menu, gone, holds them until
    /// that key comes up, then gives them back to what had them (None:
    /// nothing did). Given back at once, the key coming up would press
    /// what had them, which opens the menu again or the folder it was for.
    give_back_on_key_up: Option<Option<FocusHandle>>,
    shimmer_since: Instant,
    search: Entity<TextInput>,
    /// The Search row is a field.
    searching: bool,
    query: String,
    rename: Entity<TextInput>,
    /// The conversation whose title is being edited.
    renaming: Option<String>,
    /// A long list shows all of itself.
    show_all: bool,
    /// The other folders unfolded, and their conversations as read when
    /// they were (None while they are being read, away from the window);
    /// those of them showing all their conversations.
    unfolded: HashMap<PathBuf, Option<Vec<ConversationSummary>>>,
    others_shown_all: HashSet<PathBuf>,
    /// Every conversation the list has held, and whether it has been
    /// listed at all: what was there at first shows at once.
    seen: HashSet<String>,
    listed: bool,
    /// Conversations new to the list, and when they came.
    arrivals: HashMap<String, Instant>,
    /// The sidebar is in the window: put away, its titles' light rests.
    shown: bool,
    /// The app's menu is open above the account, which stays held.
    app_menu_open: bool,
    /// Draws the working titles' light again, a step at a time, while Kerf
    /// works, the sidebar shows and motion is on ([`Self::follow_shimmer`]).
    shimmer_tick: Option<Task<()>>,
    /// A press held on the edge; the last press on it let go where it was,
    /// and the wait before that press puts the sidebar away (dropped when
    /// a second press comes).
    edge_press: Option<EdgePress>,
    edge_clicked: Option<Instant>,
    edge_hide: Option<Task<()>>,
    /// Where the open folder's heading was laid out last, in the window.
    heading_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Drawn again as a conversation's age in its tip ("4m") turns over.
    ages: crate::motion::Redraw,
    _subscriptions: Vec<Subscription>,
}

impl SidebarView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        let search = cx.new(|cx| TextInput::new(cx, "Search").with_style(text::SIDE));
        let rename = cx.new(|cx| TextInput::new(cx, "Conversation title").with_style(text::SIDE));
        let search_focus = search.read(cx).focus_handle(cx);
        let rename_focus = rename.read(cx).focus_handle(cx);
        let menu_focus = cx.focus_handle();
        let subscriptions = vec![
            // A menu that loses the keys to something else goes; one
            // waiting for a key to come up gives them back if the window
            // goes to the back first.
            cx.on_blur(&menu_focus, window, |this, window, cx| {
                if this.give_back_on_key_up.is_some() {
                    if window.is_window_active() {
                        this.give_back_on_key_up = None;
                    } else {
                        this.give_back_keys(window, cx);
                    }
                } else {
                    this.close_menus(cx);
                }
            }),
            // A letter typed in an open menu goes to its next item that
            // starts with it, as in the kit's menus; nothing else typed
            // there reaches the window under it.
            menus::letters_in_menu(menu_focus.clone(), cx),
            // The light over a working title follows Kerf's work.
            cx.observe(&session, |this, _, cx| {
                this.follow_shimmer(cx);
                cx.notify();
            }),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                if *change == Change::Conversations {
                    this.note_arrivals(true, cx);
                }
            }),
            // Kerf's account, as the foot says it.
            cx.subscribe(&session, |_, _, event: &BridgeEvent, cx| {
                if let Event::Cloud { .. } = &*event.0 {
                    crate::session::hear_cloud(
                        &serde_json::to_value(&event.0).unwrap_or_default(),
                        cx,
                    );
                    cx.notify();
                }
            }),
            cx.subscribe_in(&search, window, |this, field, event: &InputEvent, _, cx| {
                match event {
                    InputEvent::Changed => {
                        this.query = field.read(cx).text().trim().to_owned();
                        cx.notify();
                    }
                    // Esc empties the field, then gives way to the Search row.
                    InputEvent::Cancel => this.search_escape(cx),
                    InputEvent::Submit => {}
                }
            }),
            // Left empty, the field gives way to the Search row again.
            cx.on_blur(&search_focus, window, |this, _, cx| {
                if this.search.read(cx).text().trim().is_empty() {
                    this.searching = false;
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &rename,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Submit => this.finish_rename(true, true, cx),
                    InputEvent::Cancel => this.finish_rename(false, true, cx),
                    InputEvent::Changed => {}
                },
            ),
            // Leaving the field keeps what was typed.
            cx.on_blur(&rename_focus, window, |this, _, cx| {
                this.finish_rename(true, false, cx)
            }),
            // Tab from the window itself (nothing in it holding the keys)
            // comes into the sidebar; a field, a menu or what is over the
            // window keeps its own Tab.
            {
                let this = cx.weak_entity();
                let handle = window.window_handle();
                cx.intercept_keystrokes(move |event, window, cx| {
                    let keystroke = &event.keystroke;
                    let modifiers = keystroke.modifiers;
                    if window.window_handle() != handle
                        || keystroke.key != "tab"
                        || modifiers.control
                        || modifiers.alt
                        || modifiers.platform
                        || modifiers.function
                    {
                        return;
                    }
                    let at_window = window
                        .context_stack()
                        .last()
                        .is_none_or(|context| context.contains(crate::commands::WORKSPACE));
                    let shown = this.read_with(cx, |this, _| this.shown).unwrap_or(false);
                    if at_window && shown {
                        if modifiers.shift {
                            window.focus_prev();
                        } else {
                            window.focus_next();
                        }
                        cx.stop_propagation();
                    }
                })
            },
        ];
        let mut sidebar = Self {
            session,
            list_scroll: gpui::ScrollHandle::new(),
            archived_open: false,
            unfoldings: 0,
            menu: None,
            menu_chosen: None,
            menu_focus,
            menu_return: None,
            menu_at: None,
            refocus: None,
            give_back_on_key_up: None,
            shimmer_since: Instant::now(),
            search,
            searching: false,
            query: String::new(),
            rename,
            renaming: None,
            show_all: false,
            unfolded: HashMap::new(),
            others_shown_all: HashSet::new(),
            seen: HashSet::new(),
            listed: false,
            arrivals: HashMap::new(),
            shown: true,
            app_menu_open: false,
            shimmer_tick: None,
            edge_press: None,
            edge_clicked: None,
            edge_hide: None,
            heading_bounds: Rc::new(Cell::new(None)),
            ages: crate::motion::Redraw::default(),
            _subscriptions: subscriptions,
        };
        sidebar.note_arrivals(false, cx);
        sidebar.unfold_as_left(cx);
        sidebar.follow_shimmer(cx);
        sidebar
    }

    /// The open folder's heading (the project's row) as last laid out, in
    /// the window's coordinates: where a finished export's frame lands
    /// (`Workspace::fly_export_home`).
    pub fn project_row_bounds(&self) -> Option<Bounds<Pixels>> {
        self.heading_bounds.get()
    }

    /// Whether the sidebar is in the window; put away, it asks for no
    /// frames of its own (its titles' light rests).
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown != shown {
            self.shown = shown;
            // Put away, it takes its menus with it.
            if !shown {
                self.close_menus(cx);
            }
            self.follow_shimmer(cx);
            cx.notify();
        }
    }

    /// The sidebar's head, as tall as the toolbar beside it: Hide sidebar.
    fn head(window: &Window, cx: &Context<Self>) -> impl IntoElement {
        div()
            .h(px(layout::TOOLBAR))
            .flex_none()
            .flex()
            .items_center()
            // On macOS outside full screen the window buttons sit
            // here, over the sidebar, so Hide sidebar starts
            // past them, and the row's empty stretch moves and
            // zooms the window.
            .pl(px(crate::chrome::title_bar_lead(window)))
            .pr(px(8.0))
            .gap(px(4.0))
            .child(
                icon_button("hide-sidebar", Icon::Sidebar)
                    .tooltip("Hide sidebar")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Hide))),
            )
            .child(crate::chrome::title_bar_stretch(div().flex_1().h_full()))
    }
}

impl Render for SidebarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The light over a working title rests under Reduce Motion, read as
        // the sidebar draws.
        self.follow_shimmer(cx);
        // A menu that had the keys has gone: they go back where they were,
        // once what closed it has run its course (a press elsewhere giving
        // them to what it pressed, the app's menu opening in its place).
        if let Some(back) = self.refocus.take() {
            if self.menu_focus.is_focused(window) {
                cx.defer_in(window, move |_, window, _| window.focus(&back));
            }
        }
        // A menu gone by a key still down holds the keys, unseen, until it
        // comes up; Enter and Space press nothing meanwhile.
        let key_down_after_choice = self.held_keys(window, cx);
        let session = self.session.read(cx);
        let project = session.project_name();
        let new_chosen = session.conversation.is_none();
        // Who chat runs on, at the foot.
        let (account_title, account_below, _) = account_of(session, cx);
        let list = self.conversations(cx);
        let heading = self.folder_heading(project, cx);
        let foot = self.foot(account_title, account_below, cx);
        let edge = self.edge(cx);

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::line())
            .key_context(CONTEXT)
            .on_action(cx.listener(|_, _: &FocusNext, window, _| window.focus_next()))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, _| window.focus_prev()))
            .children(key_down_after_choice)
            .child(Self::head(window, cx))
            .child(self.top_rows(new_chosen, cx))
            .child(heading)
            .child(list)
            // Over the list: a fade where a row is cut at its top, and at
            // its foot while there is more.
            .child(ui::scroll_fade(&self.list_scroll, theme::sidebar()))
            .child(foot)
            .child(edge)
            .on_mouse_down(MouseButton::Right, |_, _, _| {})
    }
}
