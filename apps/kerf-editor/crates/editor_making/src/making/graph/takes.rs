//! A shot's takes in its graph: each kept, by its picture, the one the
//! shot is ticked and another chosen with a press, a take made from steps
//! changed since restored to them, and the takes held to their guide on
//! asking; and what is said of a take's reading.

use super::inspector::{note, section};
use super::ShotGraph;
use crate::making::{number, strings, GraphNode};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, radius, text};
use editor_ui::ui;
use gpui::{canvas, div, prelude::*, px, Context, ElementId, Rgba};
use kerf_bridge::event::TakeRead;
use kerf_bridge::{Command, RecipeEdit};
use serde_json::Value;

/// Holding the takes to their guide.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Holding {
    Idle,
    Under,
    Read,
    Failed(String),
}

/// A take's picture in its row.
const TAKE_PICTURE: (f32, f32) = (57.0, 32.0);

/// What a take's reading beside its guide says of each subject: how much of
/// where it was asked for it has, and on how many frames it was followed;
/// or the clip's note.
pub fn reading_lines(reading: &TakeRead) -> Vec<String> {
    reading
        .clips
        .iter()
        .map(|clip| {
            let name = clip["clip"].as_str().unwrap_or_default();
            match clip["note"].as_str() {
                Some(note) => format!("{name}: {note}"),
                None => format!(
                    "{name}: {:.2} of where it was asked for, followed on {} of the {} frames the guide shows it on",
                    clip["overlap"].as_f64().unwrap_or(0.0),
                    clip["frames"].as_u64().unwrap_or(0),
                    clip["shown"].as_u64().unwrap_or(0),
                ),
            }
        })
        .collect()
}

/// A take row's tip: what pressing it does, then each subject's reading.
pub fn take_tip(number: u32, chosen: bool, reading: Option<&TakeRead>) -> String {
    let mut lines = vec![if chosen {
        "The shot is this take".to_owned()
    } else {
        format!(
            "Have the shot be take {number} again; its clips keep their timing, and it is undone as any edit"
        )
    }];
    lines.extend(reading.map(reading_lines).unwrap_or_default());
    lines.join("\n")
}

impl ShotGraph {
    /// The shot's takes: each kept, by its picture, the one the shot is
    /// ticked, another chosen with a press; held to their guide on asking.
    pub(super) fn takes(
        &self,
        node: &GraphNode,
        making: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let holds = &node.holds;
        let mut column = div().flex().flex_col().gap(px(theme::space::S6));
        if let Some(wants) = holds["wants"].as_str() {
            column = column.child(note(format!("Wants making: {wants}."), theme::warn()));
        }
        let takes = holds["takes"].as_array().cloned().unwrap_or_default();
        if takes.is_empty() {
            return column.child(note(
                "No take yet. Make runs its steps and writes the first; every take after it is kept beside it.".into(),
                theme::ink2(),
            ));
        }
        column = column.child(section("Takes"));
        for take in &takes {
            column = column.child(self.take_row(take, making, connected, cx));
        }
        // The take the shot is, made from steps that have changed since: back to them.
        for take in &takes {
            if let Some(restore) = self.restore_button(take, making, cx) {
                column = column.child(restore);
            }
        }
        column = column.child(self.hold_button(connected, cx));
        if let Some((said, color)) = self.holding_said() {
            column = column.child(note(said, color));
        }
        column.child(ui::label(
            "Every take is kept. Make again writes the next beside these.",
            text::META,
            theme::ink3(),
        ))
    }

    /// A take's row: its picture, its number and what there is to say of
    /// it; the take the shot is ticked, and another chosen with a press;
    /// its tip says what a press does, then how each subject was read.
    fn take_row(
        &self,
        take: &Value,
        making: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let number_of = number(&take["number"]);
        let chosen = take["chosen"].as_bool().unwrap_or(false);
        let path = take["path"].as_str().unwrap_or_default().to_owned();
        let reading = self.readings.get(&number_of);
        // What there is to say of it: how it holds beside its guide once
        // that is read, what has changed since it was made, or which roll it is.
        let mut says = Vec::new();
        // An overlap that is not a number was said as none.
        if let Some(overlap) = reading
            .map(|reading| reading.overlap)
            .filter(|overlap| overlap.is_finite())
        {
            let closest = if self.best == Some(number_of) {
                ", the closest"
            } else {
                ""
            };
            says.push(format!("{overlap:.2} of its guide{closest}"));
        }
        let changed = strings(&take["changed"]);
        if !changed.is_empty() {
            says.push(format!("{} changed since", changed.join(", ")));
        } else if let Some(rolled) = take["rolled"].as_u64().filter(|rolled| *rolled > 0) {
            says.push(format!("roll {rolled}"));
        }
        let picture = self
            .in_project(&path, cx)
            .and_then(|file| self.still(&file));
        let can = !making && !chosen && connected;
        let tip = take_tip(number_of, chosen, reading);
        let row = div()
            .id(ElementId::Name(format!("take-{number_of}").into()))
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .min_h(px(if picture.is_some() {
                TAKE_PICTURE.1 + 8.0
            } else {
                28.0
            }))
            .px(px(if picture.is_some() { 4.0 } else { 8.0 }))
            .py(px(theme::space::S2))
            .rounded(px(radius::CONTROL))
            .border_1()
            .border_color(if chosen {
                theme::accent()
            } else {
                theme::line()
            })
            .when(chosen, |row| row.bg(theme::accent_wash()))
            .child(match picture {
                // The take's picture, filling its box, its corners rounded as a card's.
                Some(image) => div()
                    .flex_none()
                    .w(px(TAKE_PICTURE.0))
                    .h(px(TAKE_PICTURE.1))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                editor_session::artwork::paint_fitted(
                                    window,
                                    bounds,
                                    image.clone(),
                                    true,
                                    px(radius::CONTROL),
                                )
                            },
                        )
                        .size_full(),
                    )
                    .into_any_element(),
                None => Icon::Film.view(14.0, theme::ink3()).into_any_element(),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(ui::label(
                        format!("Take {number_of}"),
                        if chosen {
                            text::CONTROL_STRONG
                        } else {
                            text::CONTROL
                        },
                        if can || chosen {
                            theme::ink()
                        } else {
                            theme::dis()
                        },
                    ))
                    .when(!says.is_empty(), |words| {
                        words.child(ui::line(says.join(" · "), text::META, theme::ink3()))
                    }),
            )
            .when(chosen, |row| {
                row.child(Icon::Check.view(14.0, theme::accent_ink()))
            });
        let row = if can {
            let media = self.media.to_string();
            row.cursor_pointer()
                .hover(|row| row.bg(theme::raised()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    // A take is a file of the project already: the media's line names it.
                    let command = Command::MediaReplace {
                        media: media.clone(),
                        path: path.clone(),
                    };
                    this.session.update(cx, |session, _| session.send(command));
                }))
        } else {
            row
        };
        ui::tip_on(row, Some(ui::Tip::new(tip)))
    }

    /// The take the shot is, made from steps that have changed since: a
    /// button back to them. None for any other take.
    fn restore_button(
        &self,
        take: &Value,
        making: bool,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        let kept =
            take["chosen"].as_bool().unwrap_or(false) && take["kept"].as_bool().unwrap_or(false);
        if !kept || strings(&take["changed"]).is_empty() {
            return None;
        }
        let restore = number(&take["number"]);
        Some(
            div().flex().child(
                ui::button("graph-restore", format!("Restore take {restore}'s steps"))
                    .disabled(making)
                    .tooltip(format!(
                        "Write the shot's steps back as take {restore} was made from them: its words, its seeds, and the workflow it named. The sequences and signals it was made from are the project's, and come back by undoing there"
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(
                            RecipeEdit {
                                restore: Some(restore),
                                ..RecipeEdit::default()
                            },
                            cx,
                        )
                    })),
            ),
        )
    }

    /// Hold to guide: where each take has the shot's subject read beside
    /// where its guide asked for it, on asking.
    fn hold_button(&self, connected: bool, cx: &mut Context<Self>) -> gpui::Div {
        let reading = self.reading == Holding::Under;
        let hold_words = if reading {
            "Reading…"
        } else if self.readings.is_empty() {
            "Hold to guide"
        } else {
            "Hold to guide again"
        };
        div().flex().pt(px(theme::space::S4)).child(
            ui::button("graph-hold", hold_words)
                .disabled(reading || !connected)
                .tooltip("Read where each take has the shot's subject beside where its guide asked for it. No model is run")
                .on_click(cx.listener(|this, _, _, cx| {
                    let media = this.media.to_string();
                    this.session
                        .update(cx, |session, _| session.send(Command::TakesRead { media }));
                    this.readings.clear();
                    this.best = None;
                    this.reading = Holding::Under;
                    cx.notify();
                })),
        )
    }

    /// What holding the takes to their guide has said, and in what colour.
    fn holding_said(&self) -> Option<(String, Rgba)> {
        match &self.reading {
            Holding::Idle => None,
            Holding::Under => Some((
                "Drawing the guide and following the subject through each take. A take is a few seconds.".to_owned(),
                theme::ink3(),
            )),
            Holding::Read => Some((
                "1 is a take that has the subject where and as large as its guide asked on every frame, 0 one that has none of it. Hover a take for each subject; one followed on few frames says little.".to_owned(),
                theme::ink3(),
            )),
            Holding::Failed(why) => Some((why.clone(), theme::neg())),
        }
    }
}
