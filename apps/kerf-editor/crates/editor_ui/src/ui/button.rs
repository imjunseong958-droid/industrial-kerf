//! A button: words, an optional icon before them, in one of four kinds.

use super::{control, hovered, keyed, tip_for, Handler, CONTROL};
use crate::icons::Icon;
use crate::theme::{self, height, radius, text};
use gpui::{div, prelude::*, px, App, ClickEvent, ElementId, Rgba, SharedString, Window};
use std::rc::Rc;

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
    focusable: bool,
    tooltip: Option<SharedString>,
    key: Option<SharedString>,
    reason: Option<SharedString>,
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
        focusable: true,
        tooltip: None,
        key: None,
        reason: None,
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

    /// Whether Tab reaches it: it does when it has an `on_click` (one
    /// without, a menu's trigger takes the press, is no stop), unless a
    /// view keeps its own stops.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// What its tip says while it can be pressed.
    pub fn tooltip(mut self, words: impl Into<SharedString>) -> Self {
        self.tooltip = Some(words.into());
        self
    }

    /// Its key, in its tip after the words ("⌘↩", "Esc").
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Why it cannot be pressed now: its tip while it is greyed.
    pub fn reason(mut self, words: impl Into<SharedString>) -> Self {
        self.reason = Some(words.into());
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

/// A button's fill, its fill under the pointer, its ink and its ink under
/// the pointer, and its outline.
fn button_colours(kind: Kind) -> (Rgba, Rgba, Rgba, Rgba, Option<Rgba>) {
    match kind {
        Kind::Primary => (
            theme::accent(),
            theme::accent_hover(),
            theme::accent_text(),
            theme::accent_text(),
            None,
        ),
        Kind::Secondary => (
            theme::raised(),
            theme::hover2(),
            theme::ink(),
            theme::ink(),
            Some(theme::line2()),
        ),
        Kind::Ghost => (
            theme::with_alpha(theme::raised(), 0.0),
            theme::raised(),
            theme::ink2(),
            theme::ink(),
            None,
        ),
        Kind::Danger => (
            theme::raised(),
            theme::hover2(),
            theme::neg(),
            theme::neg(),
            Some(theme::line2()),
        ),
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = control(&self.id, window, cx);
        let (fill, hover, ink, hover_ink, outline) = button_colours(self.kind);
        // The hover fades over HOVER; a greyed button has none.
        let lit = if self.disabled {
            0.0
        } else {
            state.read(cx).hover.sample(window)
        };
        let ink = if self.disabled {
            theme::dis()
        } else {
            theme::mix(ink, hover_ink, lit)
        };
        let tall = if self.small { height::H24 } else { height::H28 };
        let mut button = theme::type_style(div(), text::CONTROL_STRONG)
            .id(self.id.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(theme::space::S6))
            .h(px(tall))
            .px(px(if self.small { 8.0 } else { 12.0 }))
            .rounded(px(radius::CONTROL))
            .bg(theme::mix(fill, hover, lit))
            .text_color(ink)
            .whitespace_nowrap();
        if let Some(outline) = outline {
            button = button.border_1().border_color(outline);
        }
        if let Some(icon) = self.icon {
            button = button.child(icon.view(14.0, ink));
        }
        button = button.child(self.label);
        if !self.disabled {
            button = button.cursor_pointer();
        }
        let tip = tip_for(
            self.tooltip.as_ref(),
            self.key.as_ref(),
            self.disabled,
            self.reason.as_ref(),
        );
        // Only what its press does can the keys press.
        let focusable = self.focusable && self.on_click.is_some();
        let button = keyed(
            button,
            &state,
            CONTROL,
            focusable,
            self.disabled,
            radius::CONTROL,
            self.kind == Kind::Primary,
            window,
            cx,
            self.on_click,
        );
        hovered(button, &state, tip, None, window, cx)
    }
}
