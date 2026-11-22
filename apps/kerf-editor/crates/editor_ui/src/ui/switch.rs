//! A switch: its knob slides to the other end, and its track takes or
//! loses the accent, as it is set the other way.

use super::{control, hovered, keyed, Tip, CONTROL};
use crate::motion::Tween;
use crate::theme::{self, motion as scene};
use gpui::{div, prelude::*, px, App, ClickEvent, ElementId, SharedString, Window};
use std::rc::Rc;

/// A switch. Pressed, its knob slides to the other end and its track takes
/// or loses the accent over `segment`; set before it shows, it is already
/// in place; under Reduce Motion it is there at once. Tab reaches it, and
/// Space presses it.
pub fn switch(
    id: impl Into<SharedString>,
    on: bool,
    toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Switch {
    Switch {
        id: id.into(),
        on,
        disabled: false,
        focusable: true,
        tooltip: None,
        toggle: Rc::new(toggle),
    }
}

/// What pressing a [`Switch`] does, given whether it is to be on.
type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Switch {
    id: SharedString,
    on: bool,
    disabled: bool,
    focusable: bool,
    tooltip: Option<SharedString>,
    toggle: ToggleHandler,
}

impl Switch {
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Whether Tab reaches it (it does unless a view keeps its own stops).
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    pub fn tooltip(mut self, words: impl Into<SharedString>) -> Self {
        self.tooltip = Some(words.into());
        self
    }
}

/// Where a switch's knob is: where it was set when it first showed, then
/// sliding to each new setting.
pub(super) struct Knob {
    on: bool,
    at: Tween,
}

/// A switch drawn `on`: its knob slides there over `SEGMENT` if it was set
/// the other way (where it was first drawn, it is already in place).
pub(super) fn knob_to(knob: &mut Knob, on: bool) {
    if knob.on != on {
        knob.on = on;
        knob.at.go(if on { 1.0 } else { 0.0 }, scene::SEGMENT);
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(self.id.clone());
        let state = control(&id, window, cx);
        let on = self.on;
        let knob = window.use_keyed_state(id.clone(), cx, |_, _| Knob {
            on,
            at: Tween::at(if on { 1.0 } else { 0.0 }),
        });
        let at = knob.update(cx, |knob, _| {
            knob_to(knob, on);
            knob.at.clone()
        });
        let at = at.sample(window);
        let track = theme::mix(theme::track(), theme::accent(), at);
        let body = div()
            .relative()
            .w(px(34.0))
            .h(px(20.0))
            .rounded_full()
            .bg(track)
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(px(2.0 + 14.0 * at))
                    .size(px(16.0))
                    .rounded_full()
                    .bg(theme::ink())
                    .shadow_sm(),
            );
        let toggle = self.toggle.clone();
        let element = div()
            .id(id)
            .flex_none()
            .rounded_full()
            .when(!self.disabled, |element| element.cursor_pointer())
            .when(self.disabled, |element| element.opacity(0.4))
            .child(body);
        let element = keyed(
            element,
            &state,
            CONTROL,
            self.focusable,
            self.disabled,
            10.0,
            // On, its track is the accent.
            on,
            window,
            cx,
            Some(Rc::new(
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| toggle(!on, window, cx),
            )),
        );
        let tip = self.tooltip.map(Tip::new);
        hovered(element, &state, tip, None, window, cx)
    }
}
