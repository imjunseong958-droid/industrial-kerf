//! The sidebar's keys: Tab from row to button, Enter and Space pressing
//! what has them, Shift+F10 or the Menu key opening a row's menu, F2 and
//! Delete on a row, the arrows in an open menu, Right and Left on another
//! folder; and the ring round what has them.

use super::SidebarView;
use editor_ui::theme;
use gpui::{prelude::*, App, Div, KeyBinding, Keystroke, NoAction, Stateful};

gpui::actions!(
    sidebar,
    [
        FocusNext,
        FocusPrevious,
        OpenMenu,
        RenameRow,
        DeleteRow,
        MenuUp,
        MenuDown,
        MenuChoose,
        MenuClose,
        Unfold,
        Fold,
    ]
);

/// The sidebar's key context: Tab goes from one of its rows and buttons
/// to the next, Shift-Tab back.
pub const CONTEXT: &str = "Sidebar";
/// What in it takes the keys: Enter and Space press it, Shift+F10 or the
/// Menu key opens its menu.
pub const ITEM: &str = "SidebarItem";
/// A conversation's row, where F2 renames it and Delete or Backspace
/// deletes it (after asking).
pub const ROW: &str = "SidebarRow";
/// One of its menus, open: the arrows move in it (Tab and Shift-Tab too),
/// Enter or Space chooses, Esc closes it.
pub const MENU: &str = "SidebarMenu";
/// What a conversation's row answers in: [`ITEM`] and [`ROW`] at once.
pub const ROW_KEYS: &str = "SidebarItem SidebarRow";
/// Another folder's row, where Right unfolds its conversations and Left
/// folds them.
pub const FOLDER: &str = "SidebarFolder";
/// What another folder's row answers in: [`ITEM`] and [`FOLDER`].
pub const FOLDER_KEYS: &str = "SidebarItem SidebarFolder";

/// The sidebar's keys, as [`bind_keys`] binds them. Enter and Space on a
/// row or button run nothing of the window's (Play, a render): GPUI
/// presses what has the keys as they come up.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(CONTEXT)),
        KeyBinding::new("enter", NoAction, Some(ITEM)),
        KeyBinding::new("space", NoAction, Some(ITEM)),
        KeyBinding::new("shift-f10", OpenMenu, Some(ITEM)),
        KeyBinding::new("menu", OpenMenu, Some(ITEM)),
        KeyBinding::new("f2", RenameRow, Some(ROW)),
        KeyBinding::new("delete", DeleteRow, Some(ROW)),
        KeyBinding::new("backspace", DeleteRow, Some(ROW)),
        KeyBinding::new("up", MenuUp, Some(MENU)),
        KeyBinding::new("down", MenuDown, Some(MENU)),
        KeyBinding::new("shift-tab", MenuUp, Some(MENU)),
        KeyBinding::new("tab", MenuDown, Some(MENU)),
        KeyBinding::new("enter", MenuChoose, Some(MENU)),
        KeyBinding::new("space", MenuChoose, Some(MENU)),
        KeyBinding::new("escape", MenuClose, Some(MENU)),
        KeyBinding::new("right", Unfold, Some(FOLDER)),
        KeyBinding::new("left", Fold, Some(FOLDER)),
    ]
}

/// Binds the sidebar's keys, unless they are bound. Binding a keymap
/// clears every binding, so Settings calls this again when it changes one,
/// and every sidebar as it opens.
pub fn bind_keys(cx: &mut App) {
    let bound = Keystroke::parse("shift-f10").is_ok_and(|keystroke| {
        cx.all_bindings_for_input(&[keystroke])
            .iter()
            .any(|binding| binding.action().partial_eq(&OpenMenu))
    });
    if !bound {
        cx.bind_keys(key_bindings());
    }
}

impl SidebarView {
    /// What takes the keys in the sidebar (in key context `context`, an
    /// [`ITEM`]): Tab reaches it, a ring shows round it while it has them,
    /// and Enter or Space presses it.
    pub(super) fn keyed(element: Stateful<Div>, context: &'static str) -> Stateful<Div> {
        element
            .tab_index(0)
            .key_context(context)
            .border_1()
            .border_color(theme::with_alpha(theme::accent(), 0.0))
            .focus(|style| style.border_color(theme::accent()))
            // A press of the pointer leaves the keys where they were.
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
    }
}
