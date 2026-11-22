//! A square button holding an icon, held down while what it toggles is on.

use super::{control, hovered, keyed, tip_for, Handler, CONTROL};
use crate::icons::Icon;
use crate::theme::{self, height, radius};
use gpui::{div, prelude::*, px, App, ClickEvent, ElementId, Rgba, SharedString, Window};
use std::rc::Rc;

/// A square button holding an icon.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: Icon,
    size: f32,
    on: bool,
    disabled: bool,
    focusable: bool,
    color: Option<Rgba>,
    tooltip: Option<SharedString>,
    key: Option<SharedString>,
    reason: Option<SharedString>,
    on_click: Option<Handler>,
}

pub fn icon_button(id: impl Into<ElementId>, icon: Icon) -> IconButton {
    IconButton {
        id: id.into(),
        icon,
        size: height::H28,
        on: false,
        disabled: false,
        focusable: true,
        color: None,
        tooltip: None,
        key: None,
        reason: None,
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

    /// Whether Tab reaches it: it does when it has an `on_click` (one
    /// without, a menu's trigger takes the press, is no stop), unless a
    /// view keeps its own stops.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    pub fn color(mut self, color: Rgba) -> Self {
        self.color = Some(color);
        self
    }

    /// Its name, in its tip.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Its key, in its tip after its name ("Space", "⌘,").
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

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = control(&self.id, window, cx);
        let lit = if self.disabled {
            0.0
        } else {
            state.read(cx).hover.sample(window)
        };
        let ink = if self.disabled {
            theme::dis()
        } else if self.on {
            theme::ink()
        } else {
            self.color.unwrap_or(theme::ink2())
        };
        let (fill, hover) = if self.on {
            (theme::on(), theme::hover2())
        } else {
            (theme::with_alpha(theme::raised(), 0.0), theme::raised())
        };
        let mut button = div()
            .id(self.id.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(self.size))
            .rounded(px(radius::CONTROL))
            .bg(theme::mix(fill, hover, lit))
            .child(self.icon.view(16.0, ink));
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
            false,
            window,
            cx,
            self.on_click,
        );
        hovered(button, &state, tip, None, window, cx)
    }
}
