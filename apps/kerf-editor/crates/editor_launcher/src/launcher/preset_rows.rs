//! The From Preset page as it shows: the presets found (the one chosen
//! washed in the accent) and a preset file to choose apart; and the preset
//! chosen, a file for each of its media slots with where it starts, what it
//! makes itself, and its texts.

use super::form::{field, form_row_owned};
use super::keys::{Stop, CONTROL};
use super::plan::folder_of;
use super::preset::SlotFile;
use super::LauncherView;
use crate::presets::{self, Fit};
use editor_ui::theme::{self, height, radius, text};
use editor_ui::ui;
use gpui::{div, prelude::*, px, AnyElement, Context, ElementId, SharedString, Window};
use serde_json::Value;

/// A preset's row in the list of those found.
const PRESET_ROW: f32 = 44.0;
/// A slot's start field.
const START_FIELD: f32 = 64.0;

impl LauncherView {
    /// The From Preset page: the presets found, the one chosen, a file for
    /// each of its slots, what it makes and its texts.
    pub(super) fn preset_rows(&self, window: &Window, cx: &Context<Self>) -> Vec<gpui::AnyElement> {
        let page = &self.preset;
        let mut rows = vec![self.preset_head(window, cx), self.preset_list(window, cx)];

        let Some(path) = &page.path else {
            return rows;
        };
        let Some(preset) = &page.preset else {
            rows.push(
                ui::label("Reading the preset…", text::CONTROL, theme::ink3()).into_any_element(),
            );
            return rows;
        };
        if let Some(error) = preset["error"].as_str() {
            rows.push(
                ui::label(
                    format!("{} could not be read: {error}", folder_of(path)),
                    text::CONTROL,
                    theme::neg(),
                )
                .into_any_element(),
            );
            return rows;
        }
        let made = preset["made"].as_array().cloned().unwrap_or_default();
        rows.push(self.preset_title(&made));
        rows.extend(
            page.slots
                .iter()
                .enumerate()
                .map(|(index, slot)| self.slot_row(index, slot, preset, window, cx)),
        );
        if !made.is_empty() {
            rows.extend(Self::made_rows(&made));
        }
        if !page.texts.is_empty() {
            rows.extend(self.text_rows(window, cx));
        }
        rows.push(
            ui::hairline(true)
                .mt(px(theme::space::S4))
                .into_any_element(),
        );
        rows
    }

    /// The page's head: "Presets in your projects", and a preset file to
    /// choose apart.
    fn preset_head(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let idle = self.busy.is_none();
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .child(ui::label("Presets in your projects", text::META_STRONG, theme::ink3()).flex_1())
            .child(
                self.ring(
                    Stop::PresetFile,
                    window,
                    ui::button("launcher-preset-file", "Choose a Preset File…")
                        .small()
                        .disabled(!idle)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.choose_preset_file(window, cx)),
                        ),
                ),
            )
            .into_any_element()
    }

    /// The presets found, the one chosen washed in the accent; pressed, or
    /// chosen with Up and Down while the list has the keys, a preset is read.
    fn preset_list(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let page = &self.preset;
        let list_focused = self
            .handle(Stop::PresetList)
            .is_some_and(|handle| handle.is_focused(window));
        let mut list = div()
            .id("launcher-preset-list")
            .key_context(CONTROL)
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(if list_focused {
                theme::accent()
            } else {
                theme::line()
            })
            .overflow_hidden();
        if let Some(handle) = self.handle(Stop::PresetList) {
            list = list.track_focus(&handle);
        }
        if page.choices.is_empty() {
            list = list.child(
                ui::label(
                    "Presets saved in your projects' presets/ folders appear here.",
                    text::CONTROL,
                    theme::ink3(),
                )
                .p(px(theme::space::S12)),
            );
        }
        list = list.children(page.choices.iter().enumerate().map(|(index, choice)| {
            let chosen = page.chosen == Some(index);
            div()
                .id(ElementId::named_usize("launcher-preset", index))
                .h(px(PRESET_ROW))
                .px(px(theme::space::S12))
                .flex()
                .flex_col()
                .justify_center()
                .when(index > 0, |row| {
                    row.border_t_1().border_color(theme::line())
                })
                .bg(if chosen {
                    theme::accent_wash()
                } else {
                    theme::with_alpha(theme::chrome(), 0.0)
                })
                .cursor_pointer()
                .hover(|style| style.bg(theme::card()))
                .relative()
                .child(ui::tip_zone(
                    ElementId::named_usize("launcher-preset-tip", index),
                    Some(ui::Tip::new(choice.path.display().to_string())),
                ))
                .child(ui::line(choice.name.clone(), text::BODY_NAME, theme::ink()))
                .child(ui::line(
                    format!("from {}", choice.project),
                    text::META,
                    theme::ink3(),
                ))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.load_choice(index, window, cx)),
                )
        }));
        list.into_any_element()
    }

    /// The preset chosen: its name, how many media slots it has, and how
    /// many media it makes itself (`made`).
    fn preset_title(&self, made: &[Value]) -> AnyElement {
        let page = &self.preset;
        let mut title = format!(
            "{} · {} media slot{}",
            page.name().unwrap_or_default(),
            page.slots.len(),
            if page.slots.len() == 1 { "" } else { "s" }
        );
        if !made.is_empty() {
            title.push_str(&format!(" · makes {}", made.len()));
        }
        ui::label(title, text::TITLE, theme::ink())
            .mt(px(theme::space::S4))
            .into_any_element()
    }

    /// A media slot of `preset` (`index` among them): what it needs and who
    /// reads it, how the file chosen fits it, where in the file it starts
    /// (asked only while the file has length to spare), and Choose File….
    fn slot_row(
        &self,
        index: usize,
        slot: &SlotFile,
        preset: &Value,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let idle = self.busy.is_none();
        let read_by = presets::read_by_line(&presets::slot_read_by(preset, &slot.slot));
        let (status, ink): (SharedString, _) = match (&slot.file, &slot.fit) {
            (None, _) => ("No file chosen".into(), theme::ink3()),
            (Some(_), None) => ("Checking…".into(), theme::ink3()),
            (Some(_), Some(fit)) => (
                fit.words.clone().into(),
                if fit.fits { theme::pos() } else { theme::neg() },
            ),
        };
        let start_open = slot.fit.as_ref().is_some_and(Fit::start_open);
        let start = if start_open {
            field(&slot.start, window, cx)
                .w(px(START_FIELD))
                .into_any_element()
        } else {
            div()
                .w(px(START_FIELD))
                .h(px(height::H28))
                .flex()
                .items_center()
                .px(px(theme::space::S8))
                .rounded(px(radius::CONTROL))
                .border_1()
                .border_color(theme::line())
                .child(ui::label("0", text::BODY, theme::dis()))
                .into_any_element()
        };
        let file_tip = slot
            .file
            .as_ref()
            .map(|file| format!("{status}\n{}", file.display()));
        div()
            .id(ElementId::named_usize("launcher-slot", index))
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .py(px(theme::space::S6))
            .border_t_1()
            .border_color(theme::line())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(ui::line(slot.id.clone(), text::BODY_NAME, theme::ink()))
                    .child(ui::line(
                        presets::needs_line(&slot.slot),
                        text::META,
                        theme::ink3(),
                    ))
                    .when(!read_by.is_empty(), |column| {
                        column.child(ui::line(read_by, text::META, theme::ink3()))
                    }),
            )
            .child(
                div()
                    .id(ElementId::named_usize("launcher-slot-status", index))
                    .w(px(150.0))
                    .flex_none()
                    .relative()
                    .child(ui::line(status, text::CONTROL, ink))
                    .child(ui::tip_zone(
                        ElementId::named_usize("launcher-slot-status-tip", index),
                        file_tip.map(ui::Tip::new),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(theme::space::S6))
                    .child(ui::label("Start", text::CONTROL, theme::ink2()))
                    .child(start)
                    .when_some(
                        slot.fit.as_ref().filter(|fit| fit.start_open()),
                        |row, fit| {
                            row.child(ui::label(
                                format!("s · up to {:.2}", fit.spare),
                                text::META,
                                theme::ink3(),
                            ))
                        },
                    ),
            )
            .child(
                self.ring(
                    Stop::SlotFile(index),
                    window,
                    ui::button(("launcher-slot-file", index), "Choose File…")
                        .small()
                        .disabled(!idle)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_slot_file(index, window, cx)
                        })),
                ),
            )
            .into_any_element()
    }

    /// What the preset makes itself (`made`), which needs no file: each with
    /// its steps in its tip, made after applying.
    fn made_rows(made: &[Value]) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        rows.push(
            ui::label(
                format!("Made by the preset · {} media", made.len()),
                text::META_STRONG,
                theme::ink3(),
            )
            .mt(px(theme::space::S8))
            .into_any_element(),
        );
        rows.extend(made.iter().enumerate().map(|(index, entry)| {
            let steps: Vec<&str> = entry["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let steps = steps.join("\n");
            div()
                .id(ElementId::named_usize("launcher-made", index))
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .py(px(theme::space::S6))
                .border_t_1()
                .border_color(theme::line())
                .relative()
                .child(ui::tip_zone(
                    ElementId::named_usize("launcher-made-tip", index),
                    Some(ui::Tip::new(steps)),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(ui::line(
                            entry["id"].as_str().unwrap_or_default().to_owned(),
                            text::BODY_NAME,
                            theme::ink(),
                        ))
                        .child(ui::line(
                            presets::made_line(entry),
                            text::META,
                            theme::ink3(),
                        )),
                )
                .child(ui::label("No file needed", text::CONTROL, theme::ink3()))
                .into_any_element()
        }));
        rows.push(
            ui::label(
                "Made after applying, with Make… in the bin.",
                text::META,
                theme::ink3(),
            )
            .into_any_element(),
        );
        rows
    }

    /// The preset's texts, each a field of its own, its clip in its tip.
    fn text_rows(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        let page = &self.preset;
        let mut rows = Vec::new();
        let count = page.texts.len();
        rows.push(
            ui::label(
                format!("Text · {count} field{}", if count == 1 { "" } else { "s" }),
                text::META_STRONG,
                theme::ink3(),
            )
            .mt(px(theme::space::S8))
            .into_any_element(),
        );
        rows.extend(page.texts.iter().enumerate().map(|(index, text_slot)| {
            ui::tip_on(
                div()
                    .id(ElementId::named_usize("launcher-text", index))
                    .child(form_row_owned(
                        format!("Text {}", index + 1),
                        field(&text_slot.input, window, cx).flex_1(),
                    )),
                Some(ui::Tip::new(text_slot.clip.clone())),
            )
            .into_any_element()
        }));
        rows
    }
}
