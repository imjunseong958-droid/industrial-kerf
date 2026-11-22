//! A number's row: its slider and its value, dragged on the slider,
//! scrubbed on the value or typed in its place, and sent as one edit when
//! the button comes up.

use super::catalog::Param;
use super::clip::{Field, Inspected, Target};
use super::values::{decimals, format_number, parse_number, round_to, soft_range, unit_suffix};
use super::{field_frame, InspectorView};
use crate::theme::{self, height, radius, space, text};
use gpui::{
    canvas, div, prelude::*, px, relative, AnyElement, Context, CursorStyle, DispatchPhase,
    Focusable, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Rgba, Window,
};
use serde_json::json;

/// A number's value beside its slider.
const VALUE: f32 = 58.0;
/// A slider's knob; the rail runs between half of it at each end.
const KNOB: f32 = 10.0;
/// A slider's height.
const SLIDER: f32 = 16.0;
/// Points of pointer travel a scrub on a value takes to cross its range.
const SCRUB_SPAN: f32 = 240.0;
/// Points the pointer moves before a press on a value scrubs rather than types.
const SCRUB_SLOP: f32 = 3.0;

/// A number being dragged on its slider or scrubbed on its value.
#[derive(Clone, Debug)]
pub(super) struct Drag {
    pub(super) clip: String,
    pub(super) field: Field,
    pub(super) soft: (f64, f64),
    pub(super) hard: (f64, f64),
    pub(super) decimals: usize,
    pub(super) from: f64,
    pub(super) value: f64,
    pub(super) start_x: f32,
    /// On the value, not the slider: a press that does not move types.
    pub(super) scrub: bool,
    pub(super) moved: bool,
    /// The slider's rail, left and width.
    pub(super) rail: Option<(f32, f32)>,
}

impl Drag {
    /// The pointer at `x`: on the rail, the value there; scrubbing, the
    /// value moved by how far it went, once it is past the slop.
    pub(super) fn follow(&mut self, x: f32) {
        let (lo, hi) = self.soft;
        let value = match self.rail {
            Some((left, width)) if width > 0.0 => {
                lo + f64::from(((x - left) / width).clamp(0.0, 1.0)) * (hi - lo)
            }
            _ => {
                if (x - self.start_x).abs() < SCRUB_SLOP && !self.moved {
                    return;
                }
                self.from + f64::from((x - self.start_x) / SCRUB_SPAN) * (hi - lo)
            }
        };
        self.moved = true;
        self.value = round_to(value, self.decimals).clamp(self.hard.0, self.hard.1);
    }
}

/// A number typed in its value's place.
#[derive(Clone, Debug)]
pub(super) struct Typing {
    clip: String,
    field: Field,
    from: f64,
    hard: (f64, f64),
}

/// A number sent and not yet back in a snapshot: shown meanwhile, so the
/// knob does not go back to where it was before it comes forward again.
#[derive(Clone, Debug)]
pub(super) struct Pending {
    field: Field,
    value: f64,
    pub(super) revision: u64,
}

impl InspectorView {
    fn send_number(&mut self, clip: &str, field: &Field, value: f64, cx: &mut Context<Self>) {
        self.set_param(clip, field, json!(value), cx);
        self.pending = Some(Pending {
            field: field.clone(),
            value,
            revision: self.revision,
        });
    }

    pub(super) fn commit_typed(&mut self, cx: &mut Context<Self>) {
        let Some(typing) = self.typing.take() else {
            return;
        };
        let typed = self.number.read(cx).text().to_owned();
        match parse_number(&typed) {
            Some(value) => {
                let value = value.clamp(typing.hard.0, typing.hard.1);
                if (value - typing.from).abs() > 1e-12 {
                    self.send_number(&typing.clip, &typing.field, value, cx);
                }
            }
            None if typed.trim().is_empty() => {}
            None => self.session.update(cx, |session, _| {
                session
                    .state
                    .notify(&format!("“{typed}” is not a number."), true);
            }),
        }
        cx.notify();
    }

    fn start_typing(&mut self, drag: &Drag, window: &mut Window, cx: &mut Context<Self>) {
        self.typing = Some(Typing {
            clip: drag.clip.clone(),
            field: drag.field.clone(),
            from: drag.from,
            hard: drag.hard,
        });
        // The value it has stands in the empty field, so what is typed
        // replaces it, and Enter on nothing leaves it as it was.
        let shown = format_number(drag.from, drag.decimals);
        self.number.update(cx, |input, cx| {
            input.set_placeholder(shown, cx);
            input.clear(cx);
        });
        window.focus(&self.number.focus_handle(cx));
        cx.notify();
    }

    fn press(
        &mut self,
        mut drag: Drag,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();
        self.commit_typed(cx);
        self.palette = None;
        let x = f32::from(event.position.x);
        drag.start_x = x;
        if !drag.scrub {
            drag.rail = self.rails.borrow().get(&drag.field).copied();
            drag.follow(x);
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_to(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            // Its button came up where it was not seen.
            self.release(window, cx);
            return;
        }
        if let Some(drag) = self.drag.as_mut() {
            drag.follow(f32::from(event.position.x));
        }
        cx.notify();
    }

    /// The button comes up: one edit for the whole drag.
    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.scrub && !drag.moved {
            self.start_typing(&drag, window, cx);
            return;
        }
        if (drag.value - drag.from).abs() > 1e-12 {
            self.send_number(&drag.clip, &drag.field, drag.value, cx);
        }
        cx.notify();
    }

    // Each argument is a distinct part of the row: its label, what it sets and the value held.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn number_row(
        &self,
        row: gpui::Div,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        stored: f64,
        changed: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dragged = self.drag.as_ref().filter(|drag| drag.field == *field);
        let pending = self
            .pending
            .as_ref()
            .filter(|pending| pending.field == *field);
        let shown = dragged
            .map(|drag| drag.value)
            .or(pending.map(|pending| pending.value))
            .unwrap_or(stored);
        let (lo, hi) = match dragged {
            Some(drag) => drag.soft,
            None => soft_range(param, field.target == Target::Clip, shown),
        };
        let decimals = decimals(hi - lo);
        let start = Drag {
            clip: clip.id.clone(),
            field: field.clone(),
            soft: (lo, hi),
            hard: param.hard(),
            decimals,
            from: shown,
            value: shown,
            start_x: 0.0,
            scrub: false,
            moved: false,
            rail: None,
        };
        let row = row.child(self.slider(field, shown, (lo, hi), start.clone(), cx));
        let typing = self
            .typing
            .as_ref()
            .is_some_and(|typing| typing.field == *field);
        if typing {
            let focused = self.number.focus_handle(cx).is_focused(window);
            return row
                .child(
                    field_frame(&self.number, focused, height::H24)
                        .w(px(VALUE))
                        .flex_none(),
                )
                .into_any_element();
        }
        let ink = if dragged.is_some() || changed {
            theme::accent_ink()
        } else {
            theme::ink()
        };
        let words = format!(
            "{}{}",
            format_number(shown, decimals),
            unit_suffix(param.unit.as_deref())
        );
        let scrub = Drag {
            scrub: true,
            ..start
        };
        row.child(self.number_value(field, words, ink, scrub, cx))
            .into_any_element()
    }

    /// A number's value in words, beside its slider: dragged sideways it
    /// scrubs, and a press that does not move types in its place.
    fn number_value(
        &self,
        field: &Field,
        words: String,
        ink: Rgba,
        scrub: Drag,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        theme::type_style(div(), text::TIMECODE)
            .id(field.id("value"))
            .w(px(VALUE))
            .h(px(height::H24 - 2.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .px(px(space::S4))
            .rounded(px(radius::SMALL))
            .whitespace_nowrap()
            .overflow_hidden()
            .text_color(ink)
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(|style| style.bg(theme::raised()))
            .child(words)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.press(scrub.clone(), event, window, cx)
                }),
            )
    }

    /// A 16-point slider: a 2-point rail, its fill from the left (or from
    /// zero when the range runs either side of it) and a 10-point knob.
    fn slider(
        &self,
        field: &Field,
        value: f64,
        (lo, hi): (f64, f64),
        start: Drag,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let span = (hi - lo).max(f64::EPSILON);
        let at = ((value - lo) / span).clamp(0.0, 1.0) as f32;
        let bipolar = lo < 0.0 && hi > 0.0;
        let zero = if bipolar {
            ((0.0 - lo) / span) as f32
        } else {
            0.0
        };
        let (from, to) = (at.min(zero), at.max(zero));
        let rails = self.rails.clone();
        let key = field.clone();
        let rail_top = (SLIDER - 2.0) / 2.0;
        div()
            .id(field.id("slider"))
            .flex_1()
            .min_w_0()
            .h(px(SLIDER))
            .relative()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.press(start.clone(), event, window, cx)
                }),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(KNOB / 2.0))
                    .right(px(KNOB / 2.0))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                let rail =
                                    (f32::from(bounds.origin.x), f32::from(bounds.size.width));
                                rails.borrow_mut().insert(key, rail);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(rail_top))
                            .h(px(2.0))
                            .rounded(px(1.0))
                            .bg(theme::track()),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(rail_top))
                            .h(px(2.0))
                            .left(relative(from))
                            .w(relative(to - from))
                            .bg(theme::track2()),
                    )
                    .when(bipolar, |rail| {
                        rail.child(
                            div()
                                .absolute()
                                .top(px((SLIDER - 6.0) / 2.0))
                                .left(relative(zero))
                                .w(px(1.0))
                                .h(px(6.0))
                                .bg(theme::track2()),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .top(px((SLIDER - KNOB) / 2.0))
                            .left(relative(at))
                            .ml(px(-KNOB / 2.0))
                            .size(px(KNOB))
                            .rounded_full()
                            .bg(theme::ink())
                            .shadow_sm(),
                    ),
            )
            .into_any_element()
    }

    /// While a number is dragged, it follows the pointer anywhere in the
    /// window and ends wherever its button comes up.
    pub(super) fn drag_listener(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.drag.as_ref()?;
        let this = cx.entity().downgrade();
        Some(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let moved = this.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            moved
                                .update(cx, |view, cx| view.drag_to(event, window, cx))
                                .ok();
                        }
                    });
                    let released = this;
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                            released
                                .update(cx, |view, cx| view.release(window, cx))
                                .ok();
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
            .into_any_element(),
        )
    }
}
