//! A choice's row: a few short options as segments, the rest a button that
//! opens the kit's menu of them, placed to stay inside the window and put
//! away rather than ride the Inspector's scroll.

use super::catalog::{Kind, Param};
use super::clip::{held, Field, Inspected};
use super::values::words_of;
use super::InspectorView;
use editor_state::model::EditParam;
use editor_ui::icons::Icon;
use editor_ui::theme::{self, height, radius, space, text};
use editor_ui::ui;
use gpui::{
    anchored, canvas, deferred, div, point, prelude::*, px, AnyElement, Bounds, ClickEvent,
    Context, Corner, DismissEvent, Div, Edges, Entity, Hsla, KeyDownEvent, MouseDownEvent, Pixels,
    ScrollHandle, SharedString, Stateful, TextRun, Window,
};
use serde_json::json;
use std::collections::HashSet;

/// Whether a choice of `options` is a row of segments (at most three, none
/// longer than eight letters) rather than a button that opens a menu.
pub fn choice_is_compact(options: &[String]) -> bool {
    options.len() <= 3 && options.iter().all(|option| option.chars().count() <= 8)
}

/// A choice's menu: each option in words, in the catalog's order, the one
/// `chosen` ticked.
pub fn choice_rows(options: &[String], chosen: &str) -> Vec<(String, bool)> {
    options
        .iter()
        .map(|option| (words_of(option), option == chosen))
        .collect()
}

/// Whether a choice's menu `height` tall opens over its button (laid out
/// from `top` to `bottom`) in a window `window` tall: under it while it
/// has the room there, else on the side with more room.
pub fn choice_menu_opens_up(top: f32, bottom: f32, height: f32, window: f32) -> bool {
    let margin = space::S4 + space::S8;
    let below = window - bottom - margin;
    let above = top - margin;
    below < height && above > below
}

/// Where a list scrolled to `offset` (0 at its top, down to `-most`) goes
/// to show an item laid out from `item.0` to `item.1` whole in a view
/// from `view.0` to `view.1`, clear of its fades (`TOP_FADE` at the top,
/// `BOTTOM_FADE` at the foot): as little as it must, and nowhere when it
/// shows already.
pub fn scroll_to_show(item: (f32, f32), view: (f32, f32), offset: f32, most: f32) -> f32 {
    let top = view.0 + theme::layout::TOP_FADE;
    let bottom = view.1 - theme::layout::BOTTOM_FADE;
    let moved = if item.0 < top {
        offset + (top - item.0)
    } else if item.1 > bottom {
        offset - (item.1 - bottom).min(item.0 - top)
    } else {
        offset
    };
    moved.clamp(-most.max(0.0), 0.0)
}

/// Whether a choice's menu opened with the body `scroll` scrolled to
/// `opened` stays open, its button laid out at `button` ([`choice_menu_stays`]);
/// a button not laid out yet is taken to show.
fn choice_menu_shown(opened: f32, scroll: &ScrollHandle, button: Option<Bounds<Pixels>>) -> bool {
    let view = scroll.bounds();
    let view = (f32::from(view.top()), f32::from(view.bottom()));
    let button = button.map_or(view, |button| {
        (f32::from(button.top()), f32::from(button.bottom()))
    });
    choice_menu_stays(opened, f32::from(scroll.offset().y), button, view)
}

/// How wide `words` are set as a menu's row sets them.
fn menu_words_width(words: &SharedString, window: &Window) -> f32 {
    if words.is_empty() {
        return 0.0;
    }
    let style = text::MENU_ITEM;
    let font = if style.mono {
        theme::mono_font(style.weight)
    } else {
        theme::ui_font(style.weight)
    };
    let run = TextRun {
        len: words.len(),
        font,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(words.clone(), px(style.px), &[run], None);
    f32::from(line.width).ceil()
}

/// How far left of its button's left edge (at `left`) a choice's menu
/// `width` wide opens in a window `window` wide: none while it ends `S8`
/// inside the window's right edge, else as far as keeps it so, though
/// never past `S8` inside its left edge.
pub fn choice_menu_shift(left: f32, width: f32, window: f32) -> f32 {
    let over = left + width - (window - space::S8);
    over.min(left - space::S8).max(0.0)
}

/// Whether a choice's open menu stays open: while the Inspector's body is
/// scrolled as it was when the menu opened (`opened`, `scrolled` now) and
/// some of its button (laid out from `button.0` to `button.1`) shows in the
/// body's view (from `view.0` to `view.1`). Else it goes, rather than ride
/// the scroll away from where it opened, or stay over the timeline or the
/// toolbar under a button scrolled out of sight.
pub fn choice_menu_stays(opened: f32, scrolled: f32, button: (f32, f32), view: (f32, f32)) -> bool {
    (scrolled - opened).abs() < 0.5 && button.1 > view.0 && button.0 < view.1
}

/// Whether choosing `option` in a choice's menu sends it: not when the
/// choice holds it now, whatever it held as the menu opened (a snapshot can
/// set another while it is open), nor while the choice is keyed.
pub fn choice_sends(option: &str, held: &str, keyed: bool) -> bool {
    !keyed && option != held
}

/// A choice's menu, open under (or over) its button.
pub(super) struct ChoiceMenu {
    field: Field,
    menu: Entity<ui::Menu>,
    /// Over its button, where there was no room under it.
    up: bool,
    /// How far left of its button's left edge it opens, to keep inside
    /// the window.
    shift: f32,
    /// The option ticked in it: a snapshot that sets another ticks that.
    chosen: String,
    /// How far the body was scrolled as it opened.
    scrolled: f32,
}

impl InspectorView {
    /// Opens the menu of choice `field` (from the keys: its chosen row lit),
    /// or puts it away when it is open. Another choice's menu goes first.
    fn toggle_choice(
        &mut self,
        field: &Field,
        keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was = self.choice_menu.as_ref().map(|open| open.field.clone());
        self.close_choice(window, cx);
        if was.as_ref() == Some(field) {
            return;
        }
        let Some(clip) = self.inspected(cx) else {
            return;
        };
        let Some((param, set)) = clip.lookup(field) else {
            return;
        };
        let held = held(param, set.as_ref(), clip.at);
        if held.keyed {
            return;
        }
        let chosen = held.value.as_str().unwrap_or_default().to_owned();
        let lit = param.options.iter().position(|option| *option == chosen);
        let items = self.choice_items(&clip.id, field, &param.options, &chosen, cx);
        // From the keys, its button is scrolled into sight first (it may
        // have been rolled out of it since the keys came to it): the menu
        // opens where it will show.
        let moved = if keys {
            self.reveal_choice(field, cx)
        } else {
            0.0
        };
        let button = self
            .choice_bounds
            .borrow_mut()
            .get_mut(field)
            .map(|button| {
                button.origin.y += px(moved);
                *button
            });
        // As wide as its button, and wider where its longest option would
        // be cut ("Warming filter 85", ticked).
        let widest = items
            .iter()
            .map(|item| menu_words_width(item.words(), window))
            .fold(0.0, f32::max);
        let width = button
            .map_or(0.0, |button| f32::from(button.size.width))
            .max(widest + theme::layout::MENU_ROW_ROOM);
        let viewport = window.viewport_size();
        let up = button.is_some_and(|button| {
            choice_menu_opens_up(
                f32::from(button.top()),
                f32::from(button.bottom()),
                ui::menu_height(&items),
                f32::from(viewport.height),
            )
        });
        let shift = button.map_or(0.0, |button| {
            choice_menu_shift(f32::from(button.left()), width, f32::from(viewport.width))
        });
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(width);
            match (keys, lit) {
                (true, Some(lit)) => menu.opened(lit),
                (true, None) => menu.from_keys(),
                (false, _) => menu,
            }
        });
        cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this
                .choice_menu
                .as_ref()
                .is_some_and(|open| open.menu == menu)
            {
                this.choice_menu = None;
                cx.notify();
            }
        })
        .detach();
        self.choice_menu = Some(ChoiceMenu {
            field: field.clone(),
            menu,
            up,
            shift,
            chosen,
            scrolled: f32::from(self.scroll.offset().y),
        });
        cx.notify();
    }

    /// The rows of choice `field`'s menu: each of its `options` in words,
    /// the one `chosen` ticked.
    fn choice_items(
        &self,
        clip_id: &str,
        field: &Field,
        options: &[String],
        chosen: &str,
        cx: &mut Context<Self>,
    ) -> Vec<ui::MenuItem> {
        let this = cx.weak_entity();
        choice_rows(options, chosen)
            .into_iter()
            .zip(options)
            .map(|((words, checked), option)| {
                let (this, clip_id, key, value) = (
                    this.clone(),
                    clip_id.to_owned(),
                    field.clone(),
                    option.clone(),
                );
                ui::MenuItem::new(words)
                    .checked(checked)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.choose_option(&clip_id, &key, &value, cx)
                        });
                    })
            })
            .collect()
    }

    /// Option `value` chosen in choice `field`'s menu: sent, unless the
    /// choice holds it now, as the newest snapshot says.
    fn choose_option(&mut self, clip_id: &str, field: &Field, value: &str, cx: &mut Context<Self>) {
        let Some(clip) = self.inspected(cx).filter(|clip| clip.id == clip_id) else {
            return;
        };
        let Some((param, set)) = clip.lookup(field) else {
            return;
        };
        let held = held(param, set.as_ref(), clip.at);
        if choice_sends(value, held.value.as_str().unwrap_or_default(), held.keyed) {
            self.set_param(clip_id, field, EditParam::Value(json!(value)), cx);
        }
    }

    /// The choices of `clip` that open a menu, as the snapshot has them:
    /// each has a button the keys can reach, scrolled into sight as they
    /// come to it; an open menu whose choice is gone goes, and one still
    /// there ticks what the snapshot sets.
    pub(super) fn sync_choices(
        &mut self,
        clip: &Inspected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Each choice that opens a menu has a button the keys can reach.
        let choices: HashSet<Field> = clip
            .fields()
            .into_iter()
            .filter(|(_, param, _)| {
                param.kind == Kind::Choice && !choice_is_compact(&param.options)
            })
            .map(|(field, ..)| field)
            .collect();
        for field in &choices {
            if !self.choice_focus.contains_key(field) {
                let focus = cx.focus_handle().tab_stop(true);
                let shown = field.clone();
                let reveal = cx.on_focus(&focus, window, move |this, _, cx| {
                    this.reveal_choice(&shown, cx);
                });
                self.choice_focus.insert(field.clone(), (focus, reveal));
            }
        }
        self.choice_focus.retain(|field, _| choices.contains(field));
        let gone = self
            .choice_menu
            .as_ref()
            .is_some_and(|open| !choices.contains(&open.field));
        if gone {
            self.close_choice(window, cx);
        }
        self.sync_choice_menu(clip, window, cx);
    }

    /// A choice's open menu as the newest snapshot has its choice: another
    /// option set is ticked in it; keyed now, or gone, it goes.
    fn sync_choice_menu(&mut self, clip: &Inspected, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.choice_menu.as_ref() else {
            return;
        };
        let now = clip.lookup(&open.field).and_then(|(param, set)| {
            let held = held(param, set.as_ref(), clip.at);
            (param.kind == Kind::Choice && !choice_is_compact(&param.options) && !held.keyed)
                .then(|| (param, held.value.as_str().unwrap_or_default().to_owned()))
        });
        let Some((param, chosen)) = now else {
            self.close_choice(window, cx);
            return;
        };
        if chosen == open.chosen {
            return;
        }
        let (field, menu) = (open.field.clone(), open.menu.clone());
        let items = self.choice_items(&clip.id, &field, &param.options, &chosen, cx);
        menu.update(cx, |menu, cx| menu.set_items(items, cx));
        if let Some(open) = self.choice_menu.as_mut() {
            open.chosen = chosen;
        }
    }

    /// A choice's open menu goes once the body has scrolled since it opened,
    /// or its button (as last laid out) has left the body's view.
    pub(super) fn keep_choice_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.choice_menu.as_ref() else {
            return;
        };
        let button = self.choice_bounds.borrow().get(&open.field).copied();
        if !choice_menu_shown(open.scrolled, &self.scroll, button) {
            self.close_choice(window, cx);
        }
    }

    /// The keys came to choice `field`'s button (Tab): the body scrolls, as
    /// little as it must, to show it whole, clear of its fades. Gives how
    /// far that moved the button down.
    fn reveal_choice(&mut self, field: &Field, cx: &mut Context<Self>) -> f32 {
        let Some(button) = self.choice_bounds.borrow().get(field).copied() else {
            return 0.0;
        };
        let view = self.scroll.bounds();
        let offset = f32::from(self.scroll.offset().y);
        let shown = scroll_to_show(
            (f32::from(button.top()), f32::from(button.bottom())),
            (f32::from(view.top()), f32::from(view.bottom())),
            offset,
            f32::from(self.scroll.max_offset().height),
        );
        if shown != offset {
            let x = self.scroll.offset().x;
            self.scroll.set_offset(point(x, px(shown)));
            cx.notify();
        }
        shown - offset
    }

    /// Puts a choice's open menu away; the keys go back where they were.
    pub(super) fn close_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.choice_menu.take() {
            open.menu.update(cx, |menu, cx| menu.close(window, cx));
            cx.notify();
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "each argument is a distinct part of the row: its label, what it sets and the value held"
    )]
    pub(super) fn choice_row(
        &self,
        row: gpui::Div,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        chosen: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if choice_is_compact(&param.options) {
            return row
                .child(self.choice_segments(clip, field, param, chosen, cx))
                .into_any_element();
        }
        // More options, or longer ones: a button that opens the kit's menu
        // of them (the arrows, a letter to its row, Return or Space to
        // choose, Esc closes only it). A Tab stop of its own, ringed
        // while it has the keys; Return or Space opens the menu, the chosen
        // row lit. A press leaves the keys where they were, and a press on it
        // again puts its menu away.
        let open = self
            .choice_menu
            .as_ref()
            .filter(|open| open.field == *field);
        let mut place = div()
            .relative()
            .flex_1()
            .min_w_0()
            .child(self.choice_button(field, chosen, open, window, cx));
        if let Some(open) = open {
            place = place.child(choice_menu_place(open));
        }
        row.child(place).into_any_element()
    }

    /// A choice of a few short options: a row of segments, the one set filled.
    fn choice_segments(
        &self,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        chosen: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let segments = param.options.iter().enumerate().map(|(index, option)| {
            let selected = option == chosen;
            let (clip_id, key, value) = (clip.id.clone(), field.clone(), option.clone());
            theme::type_style(
                div(),
                if selected {
                    text::CONTROL_STRONG
                } else {
                    text::CONTROL
                },
            )
            .id(field.id(&format!("segment{index}")))
            .flex_1()
            .min_w_0()
            .h(px(height::H20))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(radius::SMALL))
            .cursor_pointer()
            .whitespace_nowrap()
            .text_color(if selected {
                theme::ink()
            } else {
                theme::ink2()
            })
            .when(selected, |segment| segment.bg(theme::on()))
            .when(!selected, |segment| {
                segment.hover(|style| style.text_color(theme::ink()))
            })
            .child(words_of(option))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !selected {
                    this.set_param(&clip_id, &key, EditParam::Value(json!(value)), cx);
                }
            }))
        });
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .p(px(space::S2))
            .gap(px(space::S2))
            .rounded(px(radius::CONTROL))
            .bg(theme::raised())
            .children(segments)
    }

    /// The button of a choice that opens a menu: the option set, a Tab
    /// stop of its own, and where it was laid out for its menu to open.
    fn choice_button(
        &self,
        field: &Field,
        chosen: &str,
        open: Option<&ChoiceMenu>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = self.choice_focus.get(field).map(|(focus, _)| focus);
        let focused = focus.is_some_and(|focus| focus.is_focused(window));
        let pressed = field.clone();
        let mut button = ui::trigger(
            field.id("choice"),
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                this.toggle_choice(&pressed, false, window, cx)
            }),
        )
        .relative()
        .w_full()
        .h(px(height::H24))
        .px(px(space::S8))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(space::S4))
        .rounded(px(radius::SMALL))
        .bg(if open.is_some() {
            theme::on()
        } else {
            theme::raised()
        })
        .cursor_pointer()
        .when(open.is_none(), |button| {
            button.hover(|style| style.bg(theme::hover2()))
        })
        .child(ui::line(words_of(chosen), text::CONTROL, theme::ink()))
        .child(Icon::ChevronDown.view(12.0, theme::ink2()));
        // Where it was laid out: its menu opens there, as wide; folded away
        // (its effect folded), Tab passes it over. Moved out of the body's
        // sight with its menu open (what is over it grew or went), the menu
        // goes.
        let laid = self.choice_bounds.clone();
        let (key, shown) = (field.clone(), focus.cloned());
        let kept = open.map(|open| (open.scrolled, self.scroll.clone(), cx.weak_entity()));
        button = button.child(
            canvas(
                move |bounds, window, cx| {
                    laid.borrow_mut().insert(key, bounds);
                    if let Some(focus) = &shown {
                        ui::note_shown(focus, bounds, window);
                    }
                    if let Some((opened, scroll, this)) = kept
                        && !choice_menu_shown(opened, &scroll, Some(bounds))
                    {
                        window.defer(cx, move |window, cx| {
                            let _ = this.update(cx, |this, cx| this.keep_choice_menu(window, cx));
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        if let Some(focus) = focus {
            let (armed, keyed) = (field.clone(), field.clone());
            button = button
                .track_focus(focus)
                .key_context(ui::CONTROL)
                .capture_any_mouse_down(|_, window, _| window.prevent_default())
                .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
                .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, _| {
                    let keystroke = &event.keystroke;
                    if matches!(keystroke.key.as_str(), "enter" | "space")
                        && !keystroke.modifiers.modified()
                    {
                        this.choice_armed = Some(armed.clone());
                    }
                }))
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    let armed = this.choice_armed.take();
                    if event.is_keyboard() && armed.as_ref() == Some(&keyed) {
                        this.toggle_choice(&keyed, true, window, cx);
                    }
                }))
                .when(focused, |button| button.child(ui::ring(radius::SMALL)));
        }
        button
    }
}

/// Where a choice's open menu is drawn: under its button, or over it where
/// there was no room under it, moved left to keep inside the window.
fn choice_menu_place(open: &ChoiceMenu) -> Div {
    let (corner, gap) = if open.up {
        (Corner::BottomLeft, -space::S4)
    } else {
        (Corner::TopLeft, space::S4)
    };
    let menu = deferred(
        anchored()
            .anchor(corner)
            .offset(point(px(-open.shift), px(gap)))
            .snap_to_window_with_margin(Edges::all(px(space::S8)))
            .child(open.menu.clone()),
    )
    .with_priority(1);
    div()
        .absolute()
        .left_0()
        .when(open.up, |at| at.bottom_full())
        .when(!open.up, |at| at.top_full())
        .child(menu)
}
