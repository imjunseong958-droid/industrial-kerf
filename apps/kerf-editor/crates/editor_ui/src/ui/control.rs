//! What a control of the kit keeps while it shows (its keys, its hover, its
//! tip's comings and goings), and what every control wears: its keys
//! (`keyed`: a Tab stop, the ring, Space and Enter) and its hover (`hovered`:
//! the fade, and its tip after a rest).

use super::{
    note_shown, tab_to, tip_gone, tip_layer, tip_shown, tips_are_warm, FocusNext, FocusPrevious,
    Handler, Tip,
};
use crate::motion::Tween;
use crate::theme::{self, layout, motion as scene};
use gpui::{
    canvas, div, prelude::*, px, App, Bounds, Div, ElementId, Entity, FocusHandle, HitboxBehavior,
    KeyDownEvent, MouseButton, Pixels, Stateful, Task, Window,
};
use std::rc::Rc;
use std::time::Instant;

/// What a control keeps while it shows: its keys, its hover, its tip.
pub struct Control {
    focus: FocusHandle,
    pub(super) hover: Tween,
    /// Enter or Space went down while it had the keys: coming up, they press it.
    armed: bool,
    pub(super) tip: TipState,
}

#[derive(Default)]
pub(super) struct TipState {
    hovered: bool,
    /// Since when it shows, and whether it faded in (not when it came at
    /// once after another).
    pub(super) shown: Option<(Instant, bool)>,
    /// Since when it goes, fading out.
    pub(super) hiding: Option<Instant>,
    waiting: Option<Task<()>>,
    /// Where the control was laid out last.
    pub(super) bounds: Option<Bounds<Pixels>>,
}

impl Control {
    fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            hover: Tween::at(0.0),
            armed: false,
            tip: TipState::default(),
        }
    }

    /// The pointer came over it or left it: its hover fades, and its tip
    /// (`tipped`) waits to show, shows at once, or goes.
    fn set_hovered(&mut self, hovered: bool, tipped: bool, cx: &mut Context<Self>) {
        self.hover.go(if hovered { 1.0 } else { 0.0 }, scene::HOVER);
        self.tip.hovered = hovered;
        if !hovered || !tipped {
            self.hide_tip();
            return;
        }
        if self.tip.shown.is_some() {
            return;
        }
        let now = Instant::now();
        if tips_are_warm(now) {
            self.show_tip(false, now);
            return;
        }
        self.tip.waiting = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(scene::TIP_DELAY).await;
            let _ = this.update(cx, |control, cx| {
                if control.tip.hovered && control.tip.shown.is_none() {
                    control.show_tip(true, Instant::now());
                    cx.notify();
                }
            });
        }));
    }

    fn show_tip(&mut self, fade: bool, now: Instant) {
        self.tip.waiting = None;
        self.tip.hiding = None;
        self.tip.shown = Some((now, fade));
        tip_shown();
    }

    fn hide_tip(&mut self) {
        self.tip.waiting = None;
        if self.tip.shown.take().is_some() {
            let now = Instant::now();
            self.tip.hiding = Some(now);
            tip_gone(now);
        }
    }

    /// Something came over the control without the pointer moving (a
    /// panel opened over the window, its scrim over it): the tip goes at
    /// once, with no fade, and does not come back until the pointer is
    /// over the control again.
    fn cover(&mut self) {
        self.tip.hovered = false;
        self.hide_tip();
        self.tip.hiding = None;
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.hide_tip();
    }
}

/// The state of the control `id`, kept for as long as it shows.
pub(super) fn control(id: &ElementId, window: &mut Window, cx: &mut App) -> Entity<Control> {
    window.use_keyed_state(id.clone(), cx, |_, cx| Control::new(cx))
}

/// What else the pointer coming over an element or leaving it does, beside
/// its hover and tip (a menu's row is lit).
pub(super) type HoverAlso = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// `element` with its hover followed in `state` (for its fade) and its tip
/// `tip` shown where the pointer rests on it. The pointer coming over it or
/// leaving it also does `also`.
pub(super) fn hovered(
    element: Stateful<Div>,
    state: &Entity<Control>,
    tip: Option<Tip>,
    also: Option<HoverAlso>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let tipped = tip.is_some();
    let hover_state = state.clone();
    let mut element = element.on_hover(move |hovered, window, cx| {
        hover_state.update(cx, |control, cx| {
            control.set_hovered(*hovered, tipped, cx);
            cx.notify();
        });
        if let Some(also) = &also {
            also(*hovered, window, cx);
        }
    });
    let Some(tip) = tip else {
        return element;
    };
    // A press puts the tip away, so it does not stand over what the press
    // opens.
    let press_state = state.clone();
    element = element.on_mouse_down(MouseButton::Left, move |_, _, cx| {
        press_state.update(cx, |control, cx| {
            control.hide_tip();
            cx.notify();
        })
    });
    // Where the control is, for the tip to stand by; and whether the
    // pointer is still over it as this frame has things, for a panel that
    // opened over it (its scrim) takes it away from under a still pointer.
    let bounds_state = state.clone();
    let cover_state = state.clone();
    element = element.child(
        canvas(
            move |bounds, window, cx| {
                bounds_state.update(cx, |control, _| control.tip.bounds = Some(bounds));
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            },
            move |_, hitbox, window, cx| {
                let tip = &cover_state.read(cx).tip;
                let asked = tip.hovered && (tip.shown.is_some() || tip.waiting.is_some());
                if asked && !hitbox.is_hovered(window) {
                    cover_state.update(cx, |control, cx| {
                        control.cover();
                        cx.notify();
                    });
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    );
    match tip_layer(state, &tip, window, cx) {
        Some(layer) => element.child(layer),
        None => element,
    }
}

/// `element`, a control of the kit keeping `state`: Tab reaches it (unless
/// `disabled`, not `focusable`, or folded away to nothing), a ring shows
/// just inside its edge while it has the keys, Space or Enter presses it,
/// and `on_click` runs on a press of the pointer or the keys. A press of
/// the pointer leaves the keys where they were. On a control `filled`
/// with the accent the ring goes round it outside ([`ring_around`]).
#[expect(
    clippy::too_many_arguments,
    reason = "each control of the kit gives its own key context, Tab stop, corner, fill and press"
)]
pub(super) fn keyed(
    element: Stateful<Div>,
    state: &Entity<Control>,
    context: &'static str,
    focusable: bool,
    disabled: bool,
    corner: f32,
    filled: bool,
    window: &Window,
    cx: &App,
    on_click: Option<Handler>,
) -> Stateful<Div> {
    let focus = state.read(cx).focus.clone();
    let mut element = element
        .relative()
        .key_context(context)
        // The pointer leaves the keys where they were: the ring is the keyboard's.
        .capture_any_mouse_down(|_, window, _| window.prevent_default())
        .on_action(|_: &FocusNext, window, cx| tab_to(true, window, cx))
        .on_action(|_: &FocusPrevious, window, cx| tab_to(false, window, cx));
    if focusable && !disabled {
        element = element.track_focus(&focus);
        if focus.is_focused(window) {
            element = element.child(if filled {
                ring_around(corner)
            } else {
                ring(corner)
            });
        }
        // Folded away to nothing, Tab passes it over.
        let noted = focus.clone();
        element = element.child(
            canvas(
                move |bounds, window, _| note_shown(&noted, bounds, window),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
    }
    if disabled {
        return element;
    }
    let arm = state.clone();
    element = element.on_key_down(move |event: &KeyDownEvent, _, cx| {
        let keystroke = &event.keystroke;
        if matches!(keystroke.key.as_str(), "enter" | "space") && !keystroke.modifiers.modified() {
            arm.update(cx, |control, _| control.armed = true);
        }
    });
    if let Some(handler) = on_click {
        let armed = state.clone();
        element = element.on_click(move |event, window, cx| {
            // The keys press only what they went down on: an Enter that
            // chose in a menu does not press what has the keys after it.
            if event.is_keyboard()
                && !armed.update(cx, |control, _| std::mem::take(&mut control.armed))
            {
                return;
            }
            handler(event, window, cx)
        });
    }
    element
}

/// The ring round what has the keys: `FOCUS_RING` of the accent just
/// inside its edge, its corners `corner` round.
pub fn ring(corner: f32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(corner))
        .border(px(layout::FOCUS_RING))
        .border_color(theme::accent())
}

/// The ring round what is filled with the accent, where [`ring`] would not
/// show: `FOCUS_RING` of the accent `FOCUS_RING_OUT` outside its edge,
/// round a control whose corners are `corner` round.
pub fn ring_around(corner: f32) -> Div {
    let out = -(layout::FOCUS_RING_OUT + layout::FOCUS_RING);
    div()
        .absolute()
        .top(px(out))
        .left(px(out))
        .right(px(out))
        .bottom(px(out))
        .rounded(px(corner - out))
        .border(px(layout::FOCUS_RING))
        .border_color(theme::accent())
}
