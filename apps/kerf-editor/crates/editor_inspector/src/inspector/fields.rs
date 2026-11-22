//! The fields of words and colours, and the clip's name: each shows what
//! the snapshot says while it is not typed in, and what is typed goes to
//! the clip as it is entered or left. A colour has its swatch, its hex and
//! a palette of colours a picture often wants.

use super::catalog::{Kind, Param};
use super::clip::{held, same_value, Field, Inspected};
use super::sections::LABEL;
use super::values::{humanize, parse_hex, swatch_color};
use super::{field_frame, InspectorView};
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::theme::{self, height, radius, space, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnyElement, ClickEvent, Context, Entity, Focusable, Subscription, Window,
};
use serde_json::{json, Value};
use std::collections::HashSet;

/// A colour's swatch.
pub(super) const SWATCH: f32 = 18.0;

/// Colours a picture often wants, offered under a colour's swatch.
const PALETTE: [&str; 10] = [
    "#ffffff", "#000000", "#808080", "#e03c31", "#f28c28", "#f7d23e", "#3fa34d", "#2bb3c0",
    "#2f5bd3", "#b04ac0",
];

/// A field of words (or a colour's hex) and what was last sent from it.
pub(super) struct TextField {
    pub(super) input: Entity<TextInput>,
    kind: Kind,
    pub(super) sent: Option<String>,
    _subscriptions: [Subscription; 2],
}

impl InspectorView {
    /// Fields not being typed in show what the snapshot says.
    pub(super) fn sync_fields(
        &mut self,
        clip: &Inspected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name_focused = self.name.focus_handle(cx).is_focused(window);
        if !name_focused && self.name.read(cx).text() != clip.name {
            let name = clip.name.clone();
            self.name.update(cx, |input, cx| input.set_text(name, cx));
        }
        self.sync_choices(clip, window, cx);
        let mut wanted = HashSet::new();
        for (field, param, set) in clip.fields() {
            if !matches!(param.kind, Kind::Text | Kind::Color)
                || matches!(set, Some(Value::Array(_)))
            {
                continue;
            }
            let current = held(param, set.as_ref(), clip.at)
                .value
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if !self.texts.contains_key(&field) {
                self.add_text_field(field.clone(), param, window, cx);
            }
            let entry = &self.texts[&field];
            let focused = entry.input.focus_handle(cx).is_focused(window);
            if !focused && entry.input.read(cx).text() != current {
                entry
                    .input
                    .update(cx, |input, cx| input.set_text(current, cx));
            }
            wanted.insert(field);
        }
        self.texts.retain(|field, _| wanted.contains(field));
    }

    fn add_text_field(
        &mut self,
        field: Field,
        param: &Param,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let style = if param.kind == Kind::Color {
            text::TIMECODE
        } else {
            text::CONTROL
        };
        let placeholder = if param.kind == Kind::Color {
            "#rrggbb".to_owned()
        } else {
            humanize(&param.name)
        };
        let input = cx.new(|cx| TextInput::new(cx, placeholder).with_style(style));
        let key = field.clone();
        let submit = cx.subscribe_in(
            &input,
            window,
            move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submit => this.commit_text(&key, cx),
                InputEvent::Cancel => {
                    this.revert_text(&key, cx);
                    window.blur();
                }
                InputEvent::Changed => {}
            },
        );
        let focus = input.focus_handle(cx);
        let key = field.clone();
        let blur = cx.on_blur(&focus, window, move |this, _, cx| {
            this.commit_text(&key, cx)
        });
        self.texts.insert(
            field,
            TextField {
                input,
                kind: param.kind,
                sent: None,
                _subscriptions: [submit, blur],
            },
        );
    }

    pub(super) fn commit_name(&mut self, cx: &mut Context<Self>) {
        let Some(clip) = self.inspected(cx) else {
            return;
        };
        let typed = self.name.read(cx).text().trim().to_owned();
        if typed == clip.name || self.name_sent.as_deref() == Some(typed.as_str()) {
            return;
        }
        self.name_sent = Some(typed.clone());
        // An empty name gives the clip back the name of what it shows.
        self.edit(
            vec![json!({"op": "set", "clip": clip.id, "name": typed})],
            cx,
        );
    }

    pub(super) fn revert_name(&mut self, cx: &mut Context<Self>) {
        let name = self.inspected(cx).map(|clip| clip.name).unwrap_or_default();
        self.name.update(cx, |input, cx| input.set_text(name, cx));
    }

    pub(super) fn commit_text(&mut self, field: &Field, cx: &mut Context<Self>) {
        let Some(clip) = self.inspected(cx) else {
            return;
        };
        let Some((param, set)) = clip.lookup(field) else {
            return;
        };
        let Some(entry) = self.texts.get(field) else {
            return;
        };
        let typed = entry.input.read(cx).text().to_owned();
        let current = held(param, set.as_ref(), clip.at)
            .value
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let value = if entry.kind == Kind::Color {
            match parse_hex(&typed) {
                Some(colour) => colour,
                None => {
                    self.session.update(cx, |session, _| {
                        session.state.notify(
                            &format!("“{typed}” is not a colour: write it as #rrggbb."),
                            true,
                        );
                    });
                    self.revert_text(field, cx);
                    return;
                }
            }
        } else {
            typed
        };
        if same_value(&json!(value), &json!(current))
            || entry.sent.as_deref() == Some(value.as_str())
        {
            return;
        }
        if let Some(entry) = self.texts.get_mut(field) {
            entry.sent = Some(value.clone());
        }
        self.set_param(&clip.id, field, json!(value), cx);
    }

    fn revert_text(&mut self, field: &Field, cx: &mut Context<Self>) {
        let Some(clip) = self.inspected(cx) else {
            return;
        };
        let Some((param, set)) = clip.lookup(field) else {
            return;
        };
        let current = held(param, set.as_ref(), clip.at)
            .value
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if let Some(entry) = self.texts.get(field) {
            entry
                .input
                .update(cx, |input, cx| input.set_text(current, cx));
        }
    }

    pub(super) fn color_row(
        &self,
        row: gpui::Div,
        clip: &Inspected,
        field: &Field,
        colour: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.palette.as_ref() == Some(field);
        let key = field.clone();
        let swatch = div()
            .id(field.id("swatch"))
            .size(px(SWATCH))
            .flex_none()
            .rounded(px(radius::SMALL))
            .border_1()
            .border_color(if open { theme::lineh() } else { theme::line2() })
            .bg(swatch_color(colour))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.palette = if this.palette.as_ref() == Some(&key) {
                    None
                } else {
                    Some(key.clone())
                };
                cx.notify();
            }));
        let hex_field = match self.texts.get(field) {
            Some(entry) => {
                let focused = entry.input.focus_handle(cx).is_focused(window);
                field_frame(&entry.input, focused, height::H24)
                    .flex_1()
                    .into_any_element()
            }
            None => ui::line(colour.to_owned(), text::TIMECODE, theme::ink2()).into_any_element(),
        };
        let row = row.child(swatch).child(hex_field);
        if !open {
            return row.into_any_element();
        }
        let current = parse_hex(colour).unwrap_or_default();
        let swatches = PALETTE.iter().map(|choice| {
            let selected = *choice == current;
            let (clip_id, key) = (clip.id.clone(), field.clone());
            div()
                .id(field.id(&format!("palette-{choice}")))
                .size(px(SWATCH))
                .rounded(px(radius::SMALL))
                .border_1()
                .border_color(if selected {
                    theme::ink()
                } else {
                    theme::line2()
                })
                .bg(swatch_color(choice))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.palette = None;
                    if !selected {
                        this.set_param(&clip_id, &key, json!(choice), cx);
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .child(row)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(space::S6))
                    .pl(px(space::S12 + LABEL + space::S8))
                    .pr(px(space::S12))
                    .pb(px(space::S6))
                    .children(swatches),
            )
            .into_any_element()
    }
}
