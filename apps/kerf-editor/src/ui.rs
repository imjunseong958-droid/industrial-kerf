//! The kit every region is built from: buttons, icon buttons, a switch, a
//! segmented control, chips and labels, set in the editor's tokens.

use crate::icons::Icon;
use crate::motion;
use crate::theme::{self, height, radius, text, TypeStyle};
use gpui::{
    div, prelude::*, px, AnimationExt, App, ClickEvent, ElementId, Hsla, IntoElement, Rgba,
    SharedString, Window,
};
use std::rc::Rc;

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Words in a type style and colour.
pub fn label(words: impl Into<SharedString>, style: TypeStyle, color: Rgba) -> gpui::Div {
    theme::type_style(div(), style)
        .text_color(color)
        .child(words.into())
}

/// Words that keep to one line, cut with an ellipsis.
pub fn line(words: impl Into<SharedString>, style: TypeStyle, color: Rgba) -> gpui::Div {
    label(words, style, color)
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis()
}

/// A hairline, across or down.
pub fn hairline(across: bool) -> gpui::Div {
    let line = div().flex_none().bg(theme::line());
    if across {
        line.h(px(1.0)).w_full()
    } else {
        line.w(px(1.0)).h_full()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The one thing to do: the accent.
    Primary,
    /// An outline on the chrome.
    Secondary,
    /// Words alone until the pointer is over it.
    Ghost,
    /// What cannot be undone.
    Danger,
}

/// A button with words and an optional icon before them.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    icon: Option<Icon>,
    kind: Kind,
    disabled: bool,
    small: bool,
    on_click: Option<Handler>,
}

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button {
        id: id.into(),
        label: label.into(),
        icon: None,
        kind: Kind::Secondary,
        disabled: false,
        small: false,
        on_click: None,
    }
}

impl Button {
    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    pub fn primary(self) -> Self {
        self.kind(Kind::Primary)
    }

    pub fn ghost(self) -> Self {
        self.kind(Kind::Ghost)
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (fill, hover, ink, outline) = match self.kind {
            Kind::Primary => (
                theme::accent(),
                theme::accent_hover(),
                theme::accent_text(),
                None,
            ),
            Kind::Secondary => (
                theme::raised(),
                theme::hover2(),
                theme::ink(),
                Some(theme::line2()),
            ),
            Kind::Ghost => (
                theme::with_alpha(theme::raised(), 0.0),
                theme::raised(),
                theme::ink2(),
                None,
            ),
            Kind::Danger => (
                theme::raised(),
                theme::hover2(),
                theme::neg(),
                Some(theme::line2()),
            ),
        };
        let ink = if self.disabled { theme::dis() } else { ink };
        let tall = if self.small { height::H24 } else { height::H28 };
        let mut button = theme::type_style(div(), text::CONTROL_STRONG)
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .h(px(tall))
            .px(px(if self.small { 8.0 } else { 12.0 }))
            .rounded(px(radius::CONTROL))
            .bg(fill)
            .text_color(ink)
            .whitespace_nowrap();
        if let Some(outline) = outline {
            button = button.border_1().border_color(outline);
        }
        if let Some(icon) = self.icon {
            button = button.child(icon.view(14.0, ink));
        }
        button = button.child(self.label);
        if self.disabled {
            return button;
        }
        button = button.cursor_pointer().hover(move |style| style.bg(hover));
        if let Some(handler) = self.on_click {
            button = button.on_click(move |event, window, cx| handler(event, window, cx));
        }
        button
    }
}

/// A square button holding an icon.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: Icon,
    size: f32,
    on: bool,
    disabled: bool,
    color: Option<Rgba>,
    tooltip: Option<SharedString>,
    on_click: Option<Handler>,
}

pub fn icon_button(id: impl Into<ElementId>, icon: Icon) -> IconButton {
    IconButton {
        id: id.into(),
        icon,
        size: height::H28,
        on: false,
        disabled: false,
        color: None,
        tooltip: None,
        on_click: None,
    }
}

impl IconButton {
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Shows it held down: what it toggles is on.
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn color(mut self, color: Rgba) -> Self {
        self.color = Some(color);
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ink = if self.disabled {
            theme::dis()
        } else if self.on {
            theme::ink()
        } else {
            self.color.unwrap_or(theme::ink2())
        };
        let mut button = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(self.size))
            .rounded(px(radius::CONTROL))
            .when(self.on, |button| button.bg(theme::on()))
            .child(self.icon.view(16.0, ink));
        if let Some(tooltip) = self.tooltip {
            button = button.tooltip(move |_window, cx| cx.new(|_| Tooltip(tooltip.clone())).into());
        }
        if self.disabled {
            return button;
        }
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(theme::raised()));
        if let Some(handler) = self.on_click {
            button = button.on_click(move |event, window, cx| handler(event, window, cx));
        }
        button
    }
}

/// What a control says when the pointer rests on it.
pub struct Tooltip(pub SharedString);

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        theme::type_style(div(), text::CONTROL)
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(radius::CONTROL))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::line2())
            .text_color(theme::ink())
            .shadow_md()
            .child(self.0.clone())
    }
}

/// A switch. Its knob slides to the other end and its track takes or loses
/// the accent over `segment`; under Reduce Motion it is there at once.
pub fn switch(
    id: impl Into<SharedString>,
    on: bool,
    toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id: SharedString = id.into();
    let (from, to) = if on { (0.0, 1.0) } else { (1.0, 0.0) };
    let track = |amount: f32| theme::mix(theme::track(), theme::accent(), amount);
    let knob = |at: f32| {
        div()
            .absolute()
            .top(px(2.0))
            .left(px(2.0 + 14.0 * at))
            .size(px(16.0))
            .rounded_full()
            .bg(theme::ink())
            .shadow_sm()
    };
    let body = div().relative().w(px(34.0)).h(px(20.0)).rounded_full();
    let animated = body.with_animation(
        ElementId::Name(format!("{id}-{on}").into()),
        motion::animation(theme::motion::SEGMENT),
        move |body, delta| {
            let at = from + (to - from) * delta;
            body.bg(track(at)).child(knob(at))
        },
    );
    div()
        .id(ElementId::Name(id))
        .flex_none()
        .cursor_pointer()
        .child(animated)
        .on_click(move |_, window, cx| toggle(!on, window, cx))
}

/// A segmented control: one of a few, the chosen one filled.
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<SharedString>,
    options: &[(T, &'static str)],
    chosen: T,
    choose: impl Fn(T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id: SharedString = id.into();
    let choose = Rc::new(choose);
    div()
        .flex()
        .flex_none()
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(radius::CONTROL + 1.0))
        .bg(theme::raised())
        .children(options.iter().enumerate().map(|(index, (value, words))| {
            let value = *value;
            let selected = value == chosen;
            let choose = choose.clone();
            theme::type_style(
                div(),
                if selected {
                    text::CONTROL_STRONG
                } else {
                    text::CONTROL
                },
            )
            .id(ElementId::Name(format!("{id}-{index}").into()))
            .px(px(10.0))
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded(px(radius::SMALL + 1.0))
            .cursor_pointer()
            .text_color(if selected {
                theme::ink()
            } else {
                theme::ink2()
            })
            .when(selected, |segment| segment.bg(theme::on()))
            .when(!selected, |segment| {
                segment.hover(|style| style.text_color(theme::ink()))
            })
            .child(*words)
            .on_click(move |_, window, cx| choose(value, window, cx))
        }))
}

/// A small rounded chip of words.
pub fn chip(words: impl Into<SharedString>, color: Rgba) -> gpui::Div {
    theme::type_style(div(), text::META)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .h(px(20.0))
        .px(px(8.0))
        .rounded(px(10.0))
        .bg(theme::raised())
        .text_color(color)
        .child(words.into())
}

/// A filled dot, as a status says.
pub fn dot(color: Rgba, size: f32) -> gpui::Div {
    div().flex_none().size(px(size)).rounded_full().bg(color)
}

/// A thin progress bar, 0..1.
pub fn progress(amount: f64, color: Rgba) -> gpui::Div {
    let amount = amount.clamp(0.0, 1.0) as f32;
    div()
        .h(px(3.0))
        .w_full()
        .rounded_full()
        .bg(theme::track())
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(gpui::relative(amount)),
        )
}

/// A colour as GPUI's Hsla.
pub fn hsla(color: Rgba) -> Hsla {
    color.into()
}
