//! The kit's menu: a view of rows that takes the keys while it is open, its
//! submenus opening beside their rows where there is room, and the pieces a
//! menu is made of (its panel, a row, the line between groups, the press
//! that opens one).

mod rows;

pub use rows::*;

use super::{
    control, cut_line, hovered, label, line, HoverAlso, MenuBack, MenuChoose, MenuClose, MenuDown,
    MenuFirst, MenuLast, MenuOpen, MenuUp, Tip, MENU,
};
use crate::icons::Icon;
use crate::motion;
use crate::theme::{self, layout, motion as scene, radius, text};
use gpui::{
    canvas, div, prelude::*, px, size, AnimationExt, AnyElement, App, Bounds, ClickEvent,
    DismissEvent, Div, ElementId, EventEmitter, FocusHandle, Focusable, MouseButton,
    MouseDownEvent, Pixels, SharedString, Size, Stateful, Subscription, Window,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Which way a submenu opens from its row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubmenuPlace {
    /// To the left of the menu, where it would run past the window's right.
    pub left: bool,
    /// Upward, its foot level with the row's, where it would run past the
    /// window's foot.
    pub up: bool,
}

/// Which way a submenu `sub` big opens beside its row at `row`, the menu's
/// panel at `panel`, in a window `window` big: to the right and down, its
/// first row level with the row; to the left where the right has no room
/// (and the left has more); upward, its last row level with the row, where
/// down runs past the window's foot (and up has more room).
pub fn submenu_place(
    row: Bounds<Pixels>,
    panel: Bounds<Pixels>,
    sub: Size<Pixels>,
    window: Size<Pixels>,
) -> SubmenuPlace {
    let edge = layout::TIP_EDGE;
    let (wide, tall) = (f32::from(sub.width), f32::from(sub.height));
    let right_room = f32::from(window.width) - edge - (f32::from(panel.right()) - 1.0);
    let left_room = f32::from(panel.left()) + 1.0 - edge;
    let below = f32::from(window.height) - edge - (f32::from(row.top()) - layout::MENU_PAD - 1.0);
    let above = f32::from(row.bottom()) + layout::MENU_PAD + 1.0 - edge;
    SubmenuPlace {
        left: wide > right_room && left_room > right_room,
        up: tall > below && above > below,
    }
}

static MENU_SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What a menu shows over its rows, drawn afresh each time it is (the sync
/// chip's: where the project stands).
type MenuHead = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// Where each of a menu's panels (by its level) and each of its rows (by
/// its level and place) was laid out.
type MenuLayout = Rc<RefCell<HashMap<(usize, Option<usize>), Bounds<Pixels>>>>;

/// A menu: rows of words to choose from, with keys, submenus and ticks. It
/// takes the keys as it opens and gives them back as it goes; the arrows
/// light a row at once, Enter or Space chooses it, a letter goes to the
/// next row starting with it, Right opens a submenu and Left or Esc closes
/// it, and Esc (or a press outside) closes the menu and does nothing else.
/// Any other key, a window's shortcut with Ctrl, ⌘ or Alt among them, does
/// nothing while it is open ([`menu_key`]), as an open QMenu held the
/// keyboard. It fades in over `FADE`, rising `MENU_RISE`. A submenu opens
/// beside its row, to the left where the window ends on the right, and
/// upward (its foot level with the row's) where it would run past the
/// window's foot. A row's tip (what it does, or why it cannot) is the kit's.
///
/// Whoever opens it holds the entity, places it (in a `deferred`
/// `anchored`, or an absolute box), and drops it on its [`DismissEvent`].
pub struct Menu {
    focus: FocusHandle,
    items: Vec<MenuItem>,
    header: Option<SharedString>,
    head: Option<MenuHead>,
    width: f32,
    /// The row lit at each level open: the menu's own, then each submenu's.
    path: Vec<Option<usize>>,
    /// What had the keys before it opened.
    back: Option<FocusHandle>,
    serial: usize,
    /// Where it and its submenus are laid out, to know a press outside.
    panels: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    /// Where each panel and each row (by its level and place) was laid out
    /// the frame before, to know which way a submenu has room to open.
    laid: MenuLayout,
    closed: bool,
    _letters: Subscription,
}

impl EventEmitter<DismissEvent> for Menu {}

impl Focusable for Menu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Menu {
    /// A menu of `items`, taking the keys from what has them.
    pub fn new(items: Vec<MenuItem>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let back = window.focused(cx);
        window.focus(&focus);
        let letters = {
            let this = cx.weak_entity();
            let focus = focus.clone();
            cx.intercept_keystrokes(move |event, window, cx| {
                if !focus.contains_focused(window, cx) {
                    return;
                }
                let keystroke = &event.keystroke;
                match menu_key(keystroke) {
                    MenuKey::Own => {}
                    MenuKey::Letter(letter) => {
                        let _ = this.update(cx, |menu, cx| menu.jump(letter, cx));
                        cx.stop_propagation();
                    }
                    MenuKey::Held => cx.stop_propagation(),
                }
            })
        };
        Self {
            focus,
            items,
            header: None,
            head: None,
            width: layout::MENU_LEAST,
            path: vec![None],
            back,
            serial: MENU_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            panels: Rc::default(),
            laid: Rc::default(),
            closed: false,
            _letters: letters,
        }
    }

    /// Quiet words over its rows (who chat runs on, a media's name).
    pub fn header(mut self, words: impl Into<SharedString>) -> Self {
        self.header = Some(words.into());
        self
    }

    /// What shows over its rows, drawn afresh each time the menu is (the
    /// sync chip's standing): in place of [`Menu::header`].
    pub fn head(mut self, head: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> Self {
        self.head = Some(Rc::new(head));
        self
    }

    /// How wide it is (at least `MENU_LEAST`).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(layout::MENU_LEAST);
        self
    }

    /// Opened from the keys: its first row that can be chosen is lit.
    pub fn from_keys(mut self) -> Self {
        let enabled: Vec<bool> = self.items.iter().map(MenuItem::enabled).collect();
        self.path = vec![menu_step(&enabled, None, true)];
        self
    }

    /// Opened on row `at`'s submenu, its first row that can be chosen lit
    /// (Collaborate ▸ Switch Branch from the window's menus).
    pub fn opened(mut self, at: usize) -> Self {
        let Some(item) = self.items.get(at) else {
            return self;
        };
        self.path = vec![Some(at)];
        if !item.submenu.is_empty() && !item.disabled {
            let enabled: Vec<bool> = item.submenu.iter().map(MenuItem::enabled).collect();
            self.path.push(menu_step(&enabled, None, true));
        }
        self
    }

    /// Its rows as things stand now, while it is open (the sync chip's,
    /// as git answers): the rows lit stay lit where they still can be.
    pub fn set_items(&mut self, items: Vec<MenuItem>, cx: &mut Context<Self>) {
        self.items = items;
        self.path = kept_path(&self.items, &self.path);
        cx.notify();
    }

    /// The rows at `level`.
    fn rows(&self, level: usize) -> &[MenuItem] {
        rows_at(&self.items, &self.path, level)
    }

    fn deepest(&self) -> usize {
        self.path.len() - 1
    }

    fn enabled(&self, level: usize) -> Vec<bool> {
        self.rows(level).iter().map(MenuItem::enabled).collect()
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        let level = self.deepest();
        let lit = menu_step(&self.enabled(level), self.path[level], down);
        self.path[level] = lit;
        cx.notify();
    }

    fn end(&mut self, last: bool, cx: &mut Context<Self>) {
        let level = self.deepest();
        self.path[level] = menu_step(&self.enabled(level), None, !last);
        cx.notify();
    }

    fn jump(&mut self, letter: char, cx: &mut Context<Self>) {
        let level = self.deepest();
        let rows = self.rows(level);
        let words: Vec<&str> = rows.iter().map(|item| item.words.as_ref()).collect();
        let enabled: Vec<bool> = rows.iter().map(MenuItem::enabled).collect();
        if let Some(at) = menu_letter(&words, &enabled, self.path[level], letter) {
            self.path[level] = Some(at);
            cx.notify();
        }
    }

    /// Opens the lit row's submenu, its first row lit (`keys`) or none.
    fn open_submenu(&mut self, keys: bool, cx: &mut Context<Self>) -> bool {
        let level = self.deepest();
        let Some(item) = self.path[level].and_then(|at| self.rows(level).get(at)) else {
            return false;
        };
        if item.submenu.is_empty() || item.disabled {
            return false;
        }
        let enabled: Vec<bool> = item.submenu.iter().map(MenuItem::enabled).collect();
        self.path.push(if keys {
            menu_step(&enabled, None, true)
        } else {
            None
        });
        cx.notify();
        true
    }

    /// Closes the deepest submenu; false when none is open.
    fn close_submenu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.path.len() <= 1 {
            return false;
        }
        self.path.pop();
        cx.notify();
        true
    }

    /// The pointer is over row `at` of `level`: it is lit, and a submenu
    /// of it opens.
    fn point_at(&mut self, level: usize, at: usize, cx: &mut Context<Self>) {
        if level >= self.path.len() {
            return;
        }
        if self.path[level] == Some(at) && self.path.len() > level + 1 {
            return;
        }
        self.path.truncate(level + 1);
        self.path[level] = Some(at);
        let opens = self
            .rows(level)
            .get(at)
            .is_some_and(|item| !item.submenu.is_empty() && !item.disabled);
        if opens {
            self.path.push(None);
        }
        cx.notify();
    }

    fn choose_lit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.deepest();
        if let Some(at) = self.path[level] {
            self.choose(level, at, true, window, cx);
        }
    }

    /// Row `at` of `level` chosen: its submenu opens, or the menu goes,
    /// the keys go back, and then what it does runs.
    fn choose(
        &mut self,
        level: usize,
        at: usize,
        keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.rows(level).get(at).cloned() else {
            return;
        };
        if item.disabled {
            return;
        }
        if !item.submenu.is_empty() {
            self.path.truncate(level + 1);
            self.path[level] = Some(at);
            self.open_submenu(keys, cx);
            return;
        }
        self.close(window, cx);
        if let Some(run) = item.run {
            run(window, cx);
        }
    }

    /// It goes: the keys go back to what had them, if it still has them.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.closed = true;
        if self.focus.contains_focused(window, cx) {
            match self.back.take() {
                Some(back) => window.focus(&back),
                None => window.blur(),
            }
        }
        cx.emit(DismissEvent);
        cx.notify();
    }

    /// The panel of `level`'s rows.
    fn panel(&self, level: usize, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let rows = self.rows(level).to_vec();
        let panels = self.panels.clone();
        let mut panel = menu_panel().flex().flex_col().w(px(self.width)).child(
            canvas(
                move |bounds, _, _| {
                    let mut panels = panels.borrow_mut();
                    // Where the panels are is laid out afresh each frame,
                    // the menu's own first.
                    if level == 0 {
                        panels.clear();
                    }
                    panels.push(bounds);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        // Which of the panel's children is which row, to keep where each was.
        let mut kinds: Vec<Option<usize>> = vec![None];
        if level == 0 {
            if let Some(head) = self.head.clone() {
                panel = panel.child(head(window, cx));
                kinds.push(None);
            } else if let Some(header) = self.header.clone() {
                panel = panel.child(div().px(px(10.0)).pt(px(4.0)).pb(px(6.0)).child(line(
                    header,
                    text::META,
                    theme::ink3(),
                )));
                kinds.push(None);
            }
        }
        for (at, item) in rows.into_iter().enumerate() {
            if item.separated {
                panel = panel.child(menu_separator());
                kinds.push(None);
            }
            panel = panel.child(self.row(level, at, item, window, cx));
            kinds.push(Some(at));
        }
        let laid = self.laid.clone();
        let kinds = Rc::new(kinds);
        panel.on_children_prepainted(move |bounds, _, _| {
            let mut laid = laid.borrow_mut();
            if let Some(whole) = bounds.first() {
                // The panel's own canvas fills it.
                laid.insert((level, None), *whole);
            }
            for (bounds, kind) in bounds.iter().zip(kinds.iter()) {
                if let Some(at) = kind {
                    laid.insert((level, Some(*at)), *bounds);
                }
            }
        })
    }

    /// Row `at` of `level`, `item`: lit while the keys or the pointer have
    /// it, its key, tick or arrow at its end, its submenu beside it while
    /// that is open, and its tip.
    fn row(
        &self,
        level: usize,
        at: usize,
        item: MenuItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let is_lit = self.path.get(level).copied().flatten() == Some(at);
        let open = is_lit && self.path.len() > level + 1 && !item.submenu.is_empty();
        let row_id = ElementId::Name(format!("kit-menu-{}-{level}-{at}", self.serial).into());
        let mut row = menu_item(
            row_id.clone(),
            item.words.clone(),
            item.icon,
            item.danger,
            item.disabled,
        )
        .relative()
        .when(is_lit, |row| row.bg(theme::popup_hover()))
        .when(
            item.checked || item.key.is_some() || !item.submenu.is_empty(),
            |row| row.child(div().flex_1()),
        )
        .when_some(item.key.clone(), |row, key| {
            row.child(label(key, text::MENU_QUIET, theme::ink3()))
        })
        .when(item.checked, |row| {
            row.child(Icon::Check.view(14.0, theme::ink2()))
        })
        .when(!item.submenu.is_empty(), |row| {
            row.child(Icon::ChevronRight.view(
                14.0,
                if item.disabled {
                    theme::dis()
                } else {
                    theme::ink3()
                },
            ))
        });
        if !item.disabled {
            row = row.on_click(cx.listener(move |menu, event: &ClickEvent, window, cx| {
                menu.choose(level, at, event.is_keyboard(), window, cx)
            }));
        }
        if open {
            row = row.child(self.submenu_beside(level, at, window, cx));
        }
        let also: HoverAlso = {
            let this = cx.weak_entity();
            Rc::new(move |hovered, _, cx| {
                if hovered {
                    let _ = this.update(cx, |menu, cx| menu.point_at(level, at, cx));
                }
            })
        };
        let state = control(&row_id, window, cx);
        let tip = item.tip.clone().map(Tip::new);
        hovered(row, &state, tip, Some(also), window, cx)
    }

    /// The submenu open from row `at` of `level`, beside the row: its edge
    /// on the menu's, its first row level with this one (its last, opening
    /// upward); kept inside the window all the same.
    fn submenu_beside(
        &self,
        level: usize,
        at: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let sub_items = &self.rows(level + 1);
        let sub = size(px(self.width), px(menu_height(sub_items)));
        let place = {
            let laid = self.laid.borrow();
            match (laid.get(&(level, Some(at))), laid.get(&(level, None))) {
                (Some(row), Some(panel)) => {
                    Some(submenu_place(*row, *panel, sub, window.viewport_size()))
                }
                _ => None,
            }
        };
        // Until its row has been laid out (a menu opened on it), it waits a
        // frame unseen rather than show where it will not stay.
        let known = place.is_some();
        let place = place.unwrap_or_default();
        let sub_panel = self.panel(level + 1, window, cx);
        let beside = self.width - layout::MENU_PAD - 2.0;
        let corner = match (place.left, place.up) {
            (false, false) => gpui::Corner::TopLeft,
            (true, false) => gpui::Corner::TopRight,
            (false, true) => gpui::Corner::BottomLeft,
            (true, true) => gpui::Corner::BottomRight,
        };
        div()
            .absolute()
            .when(!known, |at| at.invisible())
            .when(!place.up, |at| at.top(px(-(layout::MENU_PAD + 1.0))))
            .when(place.up, |at| at.bottom(px(-(layout::MENU_PAD + 1.0))))
            .when(!place.left, |at| at.left(px(beside)))
            .when(place.left, |at| at.left(px(-(layout::MENU_PAD))))
            .child(
                gpui::anchored()
                    .anchor(corner)
                    .snap_to_window_with_margin(px(layout::TIP_EDGE))
                    .child(sub_panel),
            )
    }
}

/// What a key pressed while a menu has the keys does there ([`menu_key`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    /// One of the menu's own keys (the arrows, Home, End, Return, Space,
    /// Esc, Tab, Shift-Tab), bound in [`MENU`]: it goes on to them.
    Own,
    /// A letter: the next row starting with it is lit.
    Letter(char),
    /// Anything else, with Ctrl, ⌘ or Alt or without: held, so nothing of
    /// the window's acts under the menu and nothing closes (as an open
    /// QMenu held the keyboard).
    Held,
}

/// What `keystroke`, pressed while a menu has the keys, does there.
pub fn menu_key(keystroke: &gpui::Keystroke) -> MenuKey {
    let modifiers = keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return MenuKey::Held;
    }
    // The menu's own keys are bound in MENU.
    if matches!(
        keystroke.key.as_str(),
        "up" | "down" | "left" | "right" | "home" | "end" | "enter" | "space" | "escape" | "tab"
    ) {
        return MenuKey::Own;
    }
    match typed_letter(&keystroke.key) {
        Some(letter) => MenuKey::Letter(letter),
        None => MenuKey::Held,
    }
}

/// The letter a keystroke types, if it types one (a menu's letter jump).
pub fn typed_letter(key: &str) -> Option<char> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) if letter.is_alphanumeric() => Some(letter),
        _ => None,
    }
}

impl Render for Menu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = self.panel(0, window, cx);
        let panels = self.panels.clone();
        let rise = scene::MENU_RISE;
        panel
            .id(ElementId::Name(format!("kit-menu-{}", self.serial).into()))
            .relative()
            .track_focus(&self.focus)
            .key_context(MENU)
            .on_action(cx.listener(|menu, _: &MenuUp, _, cx| menu.step(false, cx)))
            .on_action(cx.listener(|menu, _: &MenuDown, _, cx| menu.step(true, cx)))
            .on_action(cx.listener(|menu, _: &MenuFirst, _, cx| menu.end(false, cx)))
            .on_action(cx.listener(|menu, _: &MenuLast, _, cx| menu.end(true, cx)))
            .on_action(cx.listener(|menu, _: &MenuChoose, window, cx| menu.choose_lit(window, cx)))
            .on_action(cx.listener(|menu, _: &MenuOpen, _, cx| {
                menu.open_submenu(true, cx);
            }))
            .on_action(cx.listener(|menu, _: &MenuBack, _, cx| {
                menu.close_submenu(cx);
            }))
            .on_action(cx.listener(|menu, _: &MenuClose, window, cx| {
                if !menu.close_submenu(cx) {
                    menu.close(window, cx);
                }
            }))
            .on_mouse_down_out(
                cx.listener(move |menu, event: &MouseDownEvent, window, cx| {
                    // A press on one of its submenus is not outside it.
                    let inside = panels
                        .borrow()
                        .iter()
                        .any(|bounds| bounds.contains(&event.position));
                    if !inside {
                        menu.close(window, cx);
                    }
                }),
            )
            .with_animation(
                ElementId::Name(format!("kit-menu-open-{}", self.serial).into()),
                motion::animation(scene::FADE),
                move |panel, t| panel.opacity(t).top(px(rise * (1.0 - t))),
            )
    }
}

/// What opens from a press: a panel of rows a step up from what it opens over.
pub fn menu_panel() -> Div {
    div()
        .p(px(layout::MENU_PAD))
        .rounded(px(radius::CARD))
        .bg(theme::popup())
        .border_1()
        .border_color(theme::line2())
        .shadow_lg()
        .occlude()
}

/// One row of a menu: its icon (or the room for one), its words (on one
/// line, cut with "…" where the menu is too narrow for them). What
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
        .gap(px(8.0))
        .h(px(layout::MENU_ROW))
        .px(px(10.0))
        .rounded(px(radius::CONTROL))
        .text_color(ink)
        .whitespace_nowrap()
        .when(!disabled, |item| item.cursor_pointer())
        .child(match icon {
            Some(icon) => icon.view(14.0, glyph).into_any_element(),
            None => div().size(px(14.0)).flex_none().into_any_element(),
        })
        // Words longer than the menu is wide (a sync's failure, as git
        // said it) are cut with "…" on one line; the row's tip has them all.
        .child(cut_line(words))
}

/// A line between a menu's groups.
pub fn menu_separator() -> Div {
    div().my(px(4.0)).mx(px(6.0)).h(px(1.0)).bg(theme::line())
}

/// A press that toggles what it opens (a menu), taken as the button goes
/// down. Any menu open has seen it first, as a press outside, and gone (a
/// press outside a popup closes it first; one that keeps the
/// press for itself, a question asked over the window, leaves the trigger
/// none). Then the trigger takes it, before what it sits in (a tile, the
/// toolbar) has it. Its own menu closes on the press too, but whoever
/// opened it still holds it until its [`DismissEvent`] arrives, after the
/// press: so the press puts it away rather than opening it again.
pub fn trigger(
    id: impl Into<ElementId>,
    press: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div().id(id).on_mouse_down(
        MouseButton::Left,
        move |event: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            press(event, window, cx);
        },
    )
}
