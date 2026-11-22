//! The app's menu: from the sidebar's account (above it), or from the
//! toolbar's ⋯ while the sidebar is away (under it), headed by who chat runs
//! on (Settings…, Account…, Media, View ▸, Keyboard ▸, Sync ▸, the registry's
//! menus where there is no menu bar, Sign Out); kept inside the window's room
//! for it ([`app_menu_room`], [`app_menu_place`]), cut to it and scrolled by
//! the wheel in a short window. And what puts every one of the window's menus
//! away.

use super::{Regions, Workspace};
use editor_sidebar::sidebar;
use editor_ui::theme::layout;
use editor_ui::ui;
use gpui::{
    canvas, div, prelude::*, px, App, Context, DismissEvent, DispatchPhase, Entity,
    ScrollWheelEvent, Window,
};

impl Workspace {
    /// The app's menu, `menu`, over the window: above the sidebar's
    /// account (`from_account`), or under the toolbar's ⋯ while the sidebar
    /// is away, inside the room the window has for it (`app_menu_room`).
    /// Taller than the room (a short window), it is cut to it and the wheel
    /// over it moves it, so every row can be reached. It fades and rises in
    /// by itself (the kit's menu); until it has been laid out once, and its
    /// height is known, it is not seen.
    pub(super) fn app_menu_layer(
        &self,
        menu: Entity<ui::Menu>,
        from_account: bool,
        regions: &Regions,
        window: &Window,
    ) -> impl IntoElement + use<> {
        let above_account = from_account && !regions.leading;
        let left = if above_account {
            regions.main_left - regions.sidebar_width + 8.0
        } else {
            regions.main_left + regions.places.app_menu
        };
        let (top, room) = app_menu_room(above_account, regions.window_height);
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
                        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, _| {
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
                            let to = (wheel.get() - f32::from(delta.y)).clamp(0.0, most);
                            if to != wheel.get() {
                                wheel.set(to);
                                window.refresh();
                            }
                        });
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        div()
            .absolute()
            .top(px(top))
            .h(px(room))
            // Room for its shadow at the left; its submenus
            // open to the right, to the window's edge.
            .left(px(left - APP_MENU_SHADOW))
            .right_0()
            .when(scrolls, |room| room.overflow_hidden())
            .child(placed)
    }

    /// The app's menu as the window stands now ([`sidebar::app_rows`]).
    fn app_rows(&self, cx: &App) -> Vec<sidebar::AppRow> {
        let session = self.session.read(cx);
        let keymap = session.prefs.keymap;
        let commands = editor_session::commands::catalog(keymap.wire());
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
    pub(super) fn toggle_app_menu(
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
    pub(super) fn close_app_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((menu, _)) = self.app_menu.take() {
            menu.update(cx, |menu, cx| menu.close(window, cx));
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_app_menu_open(false, cx));
            cx.notify();
        }
    }

    /// View ▸ Keyboard: the editor answers to `keymap`'s keys from now on,
    /// in every window (the menus and the tooltips follow).
    pub(super) fn choose_keymap(
        &mut self,
        keymap: editor_state::prefs::Keymap,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |session, cx| {
            session.prefs.keymap = keymap;
            session.save_prefs();
            cx.notify();
        });
        // Binding the keymap clears every binding; it binds the views' own again.
        crate::keymap::bind(keymap.wire(), cx);
        let project = self.session.read(cx).project.clone();
        editor_session::commands::refresh_menus(project.as_deref(), cx);
        self.refresh_keys(cx);
        cx.notify();
    }

    /// Puts away whichever menu is open: the title's, Activity, the app
    /// menu (the keys going back to what had them), the sidebar's. Whether
    /// one was.
    pub(super) fn close_menus(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
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
}

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
