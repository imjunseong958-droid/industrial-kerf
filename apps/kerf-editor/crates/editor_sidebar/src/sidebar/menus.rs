//! The sidebar's own menus as they show and answer: a conversation's (its
//! ⋯, a right-click, Shift+F10 or the Menu key), the open folder's, and
//! another folder's. Each fades in rising into place and holds the keys:
//! the arrows and a letter move in it, Enter or Space chooses, Esc or a
//! press outside closes it, and the keys go back where they were. And the
//! pieces the window's own small menus are drawn with.

use super::app_menu::SignOut;
use super::entries::{
    folder_entries, menu_step, other_folder_entries, row_entries, Entry, MenuEntry,
};
use super::folders::forget_folder;
use super::keys::{MenuChoose, MenuClose, MenuDown, MenuUp, ITEM, MENU};
use super::{SidebarEvent, SidebarMenu, SidebarView};
use editor_session::commands::{OpenRecent, RunCommand};
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, text};
use editor_ui::ui;
use gpui::{
    deferred, div, prelude::*, px, AnimationExt, AnyElement, App, ClipboardItem, Context, Div,
    ElementId, FocusHandle, KeyUpEvent, SharedString, Stateful, Subscription, Window,
};
use std::path::PathBuf;

/// What opens from a press: a panel of items a step up from what it opens over.
pub fn menu_panel() -> Div {
    div()
        .p(px(theme::space::S4))
        .rounded(px(theme::radius::CARD))
        .bg(theme::popup())
        .border_1()
        .border_color(theme::line2())
        .shadow_lg()
        .occlude()
}

/// One item of a menu: its icon (or the room for one), its words. What
/// cannot be undone is in brick; what cannot be done now is in the ink of
/// what cannot be pressed, and takes no press.
pub fn menu_item(
    id: impl Into<ElementId>,
    words: impl Into<SharedString>,
    icon: Option<Icon>,
    danger: bool,
    disabled: bool,
) -> Stateful<Div> {
    let ink = if disabled {
        theme::dis()
    } else if danger {
        theme::neg()
    } else {
        theme::ink()
    };
    let glyph = if disabled {
        theme::dis()
    } else if danger {
        theme::neg()
    } else {
        theme::ink2()
    };
    theme::type_style(div(), text::MENU_ITEM)
        .id(id)
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(theme::radius::CONTROL))
        .text_color(ink)
        .when(!disabled, |item| {
            item.cursor_pointer()
                .hover(|item| item.bg(theme::popup_hover()))
        })
        .child(match icon {
            Some(icon) => icon.view(14.0, glyph).into_any_element(),
            None => div().size(px(14.0)).flex_none().into_any_element(),
        })
        .child(words.into())
}

/// A line between a menu's groups.
pub fn menu_separator() -> Div {
    div()
        .my(px(theme::space::S4))
        .mx(px(theme::space::S6))
        .h(px(1.0))
        .bg(theme::line())
}

/// A letter typed in an open menu (`focus` its keys) goes to its next item
/// that starts with it, as in the kit's menus; nothing else typed there
/// reaches the window under it.
pub(super) fn letters_in_menu(focus: FocusHandle, cx: &mut Context<SidebarView>) -> Subscription {
    let this = cx.weak_entity();
    cx.intercept_keystrokes(move |event, window, cx| {
        if !focus.is_focused(window) {
            return;
        }
        // The menu's own keys are bound in MENU (it has no
        // submenus, nor Home and End); a letter goes to its item;
        // anything else, a window's shortcut with Ctrl, ⌘ or Alt
        // among them, does nothing while it is open.
        let keystroke = &event.keystroke;
        let key = match ui::menu_key(keystroke) {
            ui::MenuKey::Own
                if matches!(
                    keystroke.key.as_str(),
                    "up" | "down" | "enter" | "space" | "escape" | "tab"
                ) =>
            {
                return;
            }
            ui::MenuKey::Own => ui::MenuKey::Held,
            key => key,
        };
        let open = this
            .update(cx, |this, cx| {
                let open = this.menu.is_some();
                if let ui::MenuKey::Letter(letter) = key {
                    this.jump_in_menu(letter, cx);
                }
                open
            })
            .unwrap_or(false);
        if open {
            cx.stop_propagation();
        }
    })
}

impl SidebarView {
    /// Whether one of its menus is open (a conversation's, the folder's).
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Puts away whichever of its menus is open (a row's, a folder's);
    /// whether one was. The keys go back to what had them.
    pub fn close_menus(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.menu.take().is_some();
        if open {
            self.menu_chosen = None;
            self.refocus = self.menu_return.take();
            cx.notify();
        }
        open
    }

    /// The menu open is `menu`.
    pub(super) fn menu_is(&self, menu: &SidebarMenu) -> bool {
        self.menu.as_ref() == Some(menu)
    }

    /// Opens `menu` (another open one goes), taking the keys so the arrows
    /// move in it; from the keys (`keyboard`), its first item is chosen.
    pub(super) fn open_menu(
        &mut self,
        menu: SidebarMenu,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = self.entries_of(&menu, cx);
        self.menu = Some(menu);
        self.menu_at = None;
        self.menu_chosen = if keyboard {
            menu_step(&entries, None, true)
        } else {
            None
        };
        // One opening in place of a menu just closed (a right-click on
        // another row) keeps the keys' way back of the first.
        let earlier = self.refocus.take();
        self.give_back_on_key_up = None;
        if !self.menu_focus.is_focused(window) {
            self.menu_return = window.focused(cx);
        } else if self.menu_return.is_none() {
            self.menu_return = earlier;
        }
        window.focus(&self.menu_focus);
        cx.emit(SidebarEvent::MenuOpened);
        cx.notify();
    }

    /// A row's open `menu`, `width` wide: at the pointer where a
    /// right-click opened it (kept `S8` inside the window), else
    /// under the row's right end, `top` below the row's top.
    pub(super) fn placed_menu(
        &self,
        menu: &SidebarMenu,
        top: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = self.menu_view(menu, None, width, cx);
        match self.menu_at {
            Some(at) => deferred(
                gpui::anchored()
                    .position(at)
                    .snap_to_window_with_margin(px(theme::space::S8))
                    .child(view),
            )
            .with_priority(1)
            .into_any_element(),
            None => deferred(div().absolute().top(px(top)).right(px(8.0)).child(view))
                .with_priority(1)
                .into_any_element(),
        }
    }

    /// `menu` opens, or closes when it is the one open.
    pub(super) fn toggle_menu(
        &mut self,
        menu: SidebarMenu,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu_is(&menu) {
            self.close_menus(cx);
        } else {
            self.open_menu(menu, keyboard, window, cx);
        }
    }

    /// A press outside the menu put it away: what was pressed has the keys
    /// if it takes them, else they go back to what had them.
    pub(super) fn menu_pressed_out(&mut self, cx: &mut Context<Self>) {
        self.close_menus(cx);
    }

    /// What the menu `menu` holds now.
    fn entries_of(&self, menu: &SidebarMenu, cx: &App) -> Vec<MenuEntry> {
        let session = self.session.read(cx);
        match menu {
            SidebarMenu::Row(id) => {
                let archived = session
                    .state
                    .conversations
                    .iter()
                    .find(|conversation| &conversation.id == id)
                    .is_some_and(|conversation| conversation.archived);
                let working = session.state.work.busy()
                    && (session.state.work.conversation.as_deref() == Some(id.as_str())
                        || (session.state.work.conversation.is_none()
                            && session.conversation.as_deref() == Some(id.as_str())));
                row_entries(id, archived, working)
            }
            SidebarMenu::Folder => {
                folder_entries(&session.prefs.recent, session.project.as_deref())
            }
            SidebarMenu::Other(folder) => other_folder_entries(folder),
        }
    }

    fn step_menu(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let entries = self.entries_of(&menu, cx);
        self.menu_chosen = menu_step(&entries, self.menu_chosen, down);
        cx.notify();
    }

    /// A letter typed in the open menu: its next item that starts with it
    /// and can be chosen is lit.
    fn jump_in_menu(&mut self, letter: char, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let entries = self.entries_of(&menu, cx);
        let words: Vec<&str> = entries.iter().map(|entry| entry.words.as_str()).collect();
        let enabled: Vec<bool> = entries.iter().map(|entry| !entry.disabled).collect();
        if let Some(at) = ui::menu_letter(&words, &enabled, self.menu_chosen, letter) {
            self.menu_chosen = Some(at);
            cx.notify();
        }
    }

    fn choose_in_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let entries = self.entries_of(&menu, cx);
        if let Some(entry) = self
            .menu_chosen
            .and_then(|at| entries.get(at))
            .filter(|entry| !entry.disabled)
        {
            self.run_entry(entry.entry.clone(), true, window, cx);
        }
    }

    /// The key that chose in a menu came up (or the window went to the
    /// back first): the keys go back to what had them, if the menu still
    /// holds them.
    pub(super) fn give_back_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(back) = self.give_back_on_key_up.take() else {
            return;
        };
        if self.menu_focus.is_focused(window) {
            match back {
                Some(back) => window.focus(&back),
                None => window.blur(),
            }
        }
        cx.notify();
    }

    /// Esc in an open menu: it goes, and the keys go back.
    fn menu_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self.menu_return.clone();
        self.close_menus(cx);
        self.refocus = None;
        if let Some(back) = back {
            window.focus(&back);
        }
    }

    /// Does what a menu's item says (`from_keys`: chosen with Enter or
    /// Space); the menu goes first. Where nothing else took the keys, they
    /// go back to what had them: from the keys, once the key comes up.
    pub(super) fn run_entry(
        &mut self,
        entry: Entry,
        from_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self.menu_chosen = None;
        let back = self.menu_return.take();
        match entry {
            Entry::Rename(id) => self.begin_rename(&id, window, cx),
            Entry::Archive(id, archived) => self
                .session
                .update(cx, |session, _| session.archive_conversation(&id, archived)),
            Entry::Delete(id) => cx.emit(SidebarEvent::Delete(id)),
            Entry::Command(id) => {
                window.dispatch_action(Box::new(RunCommand { id: id.into() }), cx)
            }
            Entry::SignOut => window.dispatch_action(Box::new(SignOut), cx),
            Entry::Open(folder) => {
                let here = self
                    .session
                    .read(cx)
                    .project
                    .as_deref()
                    .is_some_and(|open| editor_state::folders::same_folder(open, &folder));
                if !here {
                    window.dispatch_action(
                        Box::new(OpenRecent {
                            path: folder.to_string_lossy().into_owned().into(),
                        }),
                        cx,
                    );
                }
            }
            Entry::Reveal(folder) => cx.reveal_path(&folder),
            Entry::CopyPath(folder) => {
                cx.write_to_clipboard(ClipboardItem::new_string(folder.display().to_string()))
            }
            Entry::Forget(folder) => {
                let project = self.session.update(cx, |session, cx| {
                    session.prefs.recent = forget_folder(&session.prefs.recent, &folder);
                    session.save_prefs();
                    cx.notify();
                    session.project.clone()
                });
                editor_session::commands::refresh_menus(project.as_deref(), cx);
            }
            Entry::NewConversation(folder) => self.open_in(folder, None, window, cx),
            Entry::OpenConversation(folder, id) => self.open_in(folder, Some(id), window, cx),
        }
        if self.menu_focus.is_focused(window) {
            if from_keys {
                self.give_back_on_key_up = Some(back);
            } else if let Some(back) = back {
                window.focus(&back);
            }
        }
        cx.notify();
    }

    /// Opens conversation `id` (None: a new one) of `folder`: here, at
    /// once; another folder's project opens in place of this window first.
    fn open_in(
        &mut self,
        folder: PathBuf,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let here = self
            .session
            .read(cx)
            .project
            .as_deref()
            .is_some_and(|open| editor_state::folders::same_folder(open, &folder));
        if here {
            self.session.update(cx, |session, cx| {
                session.open_conversation(id);
                cx.notify();
            });
            cx.emit(SidebarEvent::Chosen);
            return;
        }
        cx.emit(SidebarEvent::OpenConversationIn {
            folder: folder.clone(),
            id,
        });
        window.dispatch_action(
            Box::new(OpenRecent {
                path: folder.to_string_lossy().into_owned().into(),
            }),
            cx,
        );
    }

    /// While a menu gone by a key still down holds the keys (until the key
    /// comes up), an unseen piece of it has them, so Enter and Space press
    /// nothing meanwhile.
    pub(super) fn held_keys(&self, window: &Window, cx: &Context<Self>) -> Option<Div> {
        (self.give_back_on_key_up.is_some() && self.menu_focus.is_focused(window)).then(|| {
            div()
                .absolute()
                .size_0()
                .track_focus(&self.menu_focus)
                .key_context(ITEM)
                .on_key_up(
                    cx.listener(|this, _: &KeyUpEvent, window, cx| this.give_back_keys(window, cx)),
                )
        })
    }

    /// The menu `menu` as it shows, `width` wide: its header if it has one,
    /// its items, the one the arrows are on lit.
    pub(super) fn menu_view(
        &self,
        menu: &SidebarMenu,
        header: Option<SharedString>,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries = self.entries_of(menu, cx);
        let mut panel = menu_panel()
            .id("sidebar-menu")
            .w(px(width))
            .track_focus(&self.menu_focus)
            .key_context(MENU)
            .on_action(cx.listener(|this, _: &MenuUp, _, cx| this.step_menu(false, cx)))
            .on_action(cx.listener(|this, _: &MenuDown, _, cx| this.step_menu(true, cx)))
            .on_action(
                cx.listener(|this, _: &MenuChoose, window, cx| this.choose_in_menu(window, cx)),
            )
            .on_action(cx.listener(|this, _: &MenuClose, window, cx| this.menu_escape(window, cx)))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.menu_pressed_out(cx)));
        if let Some(header) = header {
            panel = panel.child(
                div()
                    .px(px(10.0))
                    .pt(px(theme::space::S4))
                    .pb(px(theme::space::S6))
                    .child(ui::line(header, text::META, theme::ink3())),
            );
        }
        for (index, entry) in entries.into_iter().enumerate() {
            if entry.separated {
                panel = panel.child(menu_separator());
            }
            let chosen = self.menu_chosen == Some(index);
            let mut item = menu_item(
                ElementId::Name(format!("sidebar-menu-{index}").into()),
                entry.words.clone(),
                entry.icon,
                entry.danger,
                entry.disabled,
            )
            .when(chosen, |item| item.bg(theme::popup_hover()))
            .when(entry.checked, |item| {
                item.child(div().flex_1())
                    .child(Icon::Check.view(14.0, theme::ink2()))
            });
            if !entry.disabled {
                let run = entry.entry.clone();
                item = item.on_click(cx.listener(move |this, _, window, cx| {
                    this.run_entry(run.clone(), false, window, cx)
                }));
            }
            // A folder's path, the kit's tip.
            panel = panel.child(ui::tip_on(item, entry.tip.clone().map(ui::Tip::new)));
        }
        // It rises into place as it fades in, as the kit's menus do.
        let rise = scene::MENU_RISE;
        panel
            .relative()
            .with_animation(
                ElementId::Name(format!("sidebar-menu-{menu:?}").into()),
                motion::animation(scene::FADE),
                move |menu, t| menu.opacity(t).top(px(rise * (1.0 - t))),
            )
            .into_any_element()
    }
}
