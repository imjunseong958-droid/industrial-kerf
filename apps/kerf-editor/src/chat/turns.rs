//! The conversation's turns: your request, set off on the right with the
//! playhead time it was asked at; Kerf's answer with how long it worked
//! ("Worked for …", which unfolds its steps), the card of what it proposed
//! and the cards of what it asked to have made; and under each, Copy, or
//! Ask again where it failed or was stopped.

use super::ChatView;
use crate::commands::RevealPicture;
use crate::icons::Icon;
use crate::model::{self, Proposal, Speaker, Turn, TurnShow};
use crate::motion;
use crate::selectable;
use crate::theme::{self, motion as scene, text};
use crate::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, ClipboardItem, Context, ElementId, SharedString,
    Window,
};
use std::collections::HashMap;

/// How long Copy under an answer says "Copied" (`theme::motion`).
pub use crate::theme::motion::COPIED_HOLD;

/// How long a turn worked, as its fold says it: "41s", "1m 12s", "1h 4m";
/// never less than a second.
pub fn work_length(seconds: f64) -> String {
    let whole = if seconds.is_finite() {
        seconds.max(0.0).round().max(1.0) as u64
    } else {
        1
    };
    if whole < 60 {
        format!("{whole}s")
    } else if whole < 3600 {
        format!("{}m {}s", whole / 60, whole % 60)
    } else {
        format!("{}h {}m", whole / 3600, (whole % 3600) / 60)
    }
}

/// "Worked for 1m 12s · 6 steps": what a finished turn folds to.
pub fn worked_headline(seconds: Option<f64>, steps: usize) -> String {
    let length = work_length(seconds.unwrap_or(0.0));
    match steps {
        0 => format!("Worked for {length}"),
        1 => format!("Worked for {length} · 1 step"),
        many => format!("Worked for {length} · {many} steps"),
    }
}

/// The request a Kerf turn answered: the last of yours before it.
pub fn request_before(turns: &[Turn], index: usize) -> Option<&str> {
    turns[..index.min(turns.len())]
        .iter()
        .rev()
        .find(|turn| turn.speaker() == Speaker::You)
        .map(|turn| turn.text.as_str())
        .filter(|text| !text.trim().is_empty())
}

/// What the request bubble's timecode chip says on a rest: where pressing
/// it goes ("Go to 00:00:17:08").
pub fn stamp_tip(timecode: &str) -> String {
    format!("Go to {timecode}")
}

/// What every turn reads of the proposals, gathered once for the list:
/// the proposals, which turn holds each one's card (`card_turns`), and the
/// one pinned.
pub(super) struct Cards<'a> {
    pub(super) proposals: &'a [Proposal],
    pub(super) holders: &'a HashMap<String, usize>,
    pub(super) pinned: Option<&'a str>,
}

impl ChatView {
    fn copy(&mut self, turn: String, words: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(words.trim().to_owned()));
        self.copied = Some(turn.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_HOLD).await;
            let _ = this.update(cx, |this, cx| {
                if this.copied.as_deref() == Some(turn.as_str()) {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Turn `index` of `turns`, fading in where it is: your request, or
    /// Kerf's answer with how it worked, the card it holds (`cards`), and
    /// the cards of what it asked to have made.
    pub(super) fn turn(
        &self,
        index: usize,
        turns: &[Turn],
        cards: &Cards<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let turn = &turns[index];
        let id = SharedString::from(format!("turn-{}", turn.id));
        let body = match turn.speaker() {
            // A request can be copied, as an answer can.
            Speaker::You => {
                let copy = (!turn.text.trim().is_empty()).then(|| turn.text.clone());
                let actions = self.turn_actions(&turn.id, copy, None, cx);
                self.you(&turn.text, turn.timecode, &turn.id, cx)
                    .child(actions)
            }
            Speaker::Kerf => {
                let mut column = div().flex().flex_col().gap(px(10.0));
                if !turn.steps.is_empty() || turn.seconds.is_some() {
                    column = column.child(self.worked(turn, window, cx));
                }
                let troubled = turn.error || turn.stopped;
                if turn.error {
                    column = column.child(ui::label(
                        "Couldn't finish",
                        text::META_STRONG,
                        theme::neg(),
                    ));
                } else if turn.stopped {
                    column = column.child(ui::label("Stopped", text::META_STRONG, theme::ink3()));
                }
                if !turn.text.trim().is_empty() {
                    let ink = if troubled {
                        theme::ink2()
                    } else {
                        theme::ink()
                    };
                    column = column.child(self.prose(&turn.text, &turn.id, ink, cx));
                }
                if let Some((named, proposal)) = turn.proposal.as_deref().and_then(|named| {
                    self.book
                        .find(cards.proposals, named)
                        .map(|proposal| (named, proposal))
                }) {
                    // The newest turn that names it holds its card.
                    if cards.holders.get(&proposal.proposal_id) == Some(&index) {
                        let pinned = cards.pinned == Some(proposal.proposal_id.as_str());
                        column = column.child(self.proposal_card(proposal, pinned, window, cx));
                    } else {
                        column = column.child(self.proposal_named(proposal, named));
                    }
                }
                // What it asked to have made, a card each, under its answer.
                if !turn.asks.is_empty() {
                    let cards = self
                        .makings
                        .update(cx, |makings, cx| makings.cards_for(&turn.asks, cx));
                    column = column.children(cards);
                }
                let again = troubled
                    .then(|| request_before(turns, index))
                    .flatten()
                    .map(str::to_owned);
                let copy = (!turn.text.trim().is_empty()).then(|| turn.text.clone());
                if again.is_some() || copy.is_some() {
                    column = column.child(self.turn_actions(&turn.id, copy, again, cx));
                }
                column
            }
        };
        body.with_animation(id, motion::animation(scene::APPEAR), |turn, t| {
            turn.opacity(t).mt(px(scene::APPEAR_RISE * (1.0 - t)))
        })
        .into_any_element()
    }

    /// Under an answer: Copy, and Ask again where it failed or was stopped.
    fn turn_actions(
        &self,
        turn: &str,
        copy: Option<String>,
        again: Option<String>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let copied = self.copied.as_deref() == Some(turn);
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .when_some(copy, |row, words| {
                let turn = turn.to_owned();
                row.child(
                    link(
                        ElementId::Name(format!("copy-{turn}").into()),
                        if copied { "Copied" } else { "Copy" },
                        copied,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.copy(turn.clone(), words.clone(), cx)
                        }),
                    ),
                )
            })
            .when_some(again, |row, request| {
                row.child(
                    link(
                        ElementId::Name(format!("again-{turn}").into()),
                        "Ask again",
                        false,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.ask_again(request.clone(), cx)),
                    ),
                )
            })
    }

    /// Your request, set off on the right, with the playhead time it was
    /// asked at over it, which goes there. Its words can be chosen with
    /// the pointer.
    pub(super) fn you(
        &self,
        words: &str,
        timecode: Option<f64>,
        key: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let fps = self.session.read(cx).timeline().rate();
        let session = self.session.clone();
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(theme::space::S4))
            .when_some(timecode.filter(|at| at.is_finite()), |column, at| {
                let timecode = model::timecode(at, fps);
                let tip = ui::Tip::new(stamp_tip(&timecode));
                column.child(ui::tip_on(
                    theme::type_style(div(), text::MONO_SMALL)
                        .id(ElementId::Name(format!("stamp-{key}").into()))
                        .flex()
                        .items_center()
                        .h(px(20.0))
                        .px(px(theme::space::S6))
                        .rounded(px(theme::radius::SMALL))
                        .bg(theme::raised())
                        .text_color(theme::accent_ink())
                        .cursor_pointer()
                        .hover(|chip| chip.bg(theme::hover2()))
                        .child(timecode)
                        .on_click(move |_, window, cx| {
                            session.update(cx, |session, _| session.seek(at, false));
                            window.dispatch_action(Box::new(RevealPicture), cx);
                        }),
                    Some(tip),
                ))
            })
            .child(
                theme::reading(div())
                    .max_w(gpui::relative(0.8))
                    .px(px(14.0))
                    .py(px(9.0))
                    .rounded(px(theme::radius::PANEL))
                    .bg(theme::raised())
                    .text_color(theme::ink())
                    .child(selectable::selectable(
                        &self.selections,
                        ElementId::Name(format!("you-words-{key}").into()),
                        format!("you-{key}"),
                        0,
                        words.to_owned(),
                    )),
            )
    }

    /// "Worked for …", which unfolds the steps the turn took.
    fn worked(&self, turn: &Turn, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let key = format!("steps-{}", turn.id);
        let open = self.is_open(&key);
        let headline = div()
            .flex()
            .items_center()
            .gap(px(theme::space::S6))
            .child(Icon::Clock.view(12.0, theme::ink3()))
            .child(ui::label(
                worked_headline(turn.seconds, turn.steps.len()),
                text::META,
                theme::ink3(),
            ));
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        if turn.steps.is_empty() {
            return column.child(headline);
        }
        let toggle = key.clone();
        column = column.child(
            headline
                .id(ElementId::Name(key.clone().into()))
                .cursor_pointer()
                .child(self.chevron(&key, theme::ink3(), window))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_fold(&toggle, cx))),
        );
        if open {
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S4))
                    .pl(px(10.0))
                    .border_l_1()
                    .border_color(theme::line2())
                    .children(turn.steps.iter().map(|step| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::space::S6))
                            .child(Icon::Check.view(12.0, theme::ink3()))
                            .child(ui::line(step.clone(), text::CONTROL, theme::ink2()))
                    }))
                    .with_animation(
                        ElementId::Name(format!("{key}-open").into()),
                        motion::animation(scene::FADE),
                        |steps, t| steps.opacity(t),
                    ),
            );
        }
        column
    }
}

/// Words that do something when pressed: a fold, Copy, Ask again. `on` is
/// held in ink.
pub(super) fn link(id: ElementId, words: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    ui::label(
        words,
        text::CONTROL,
        if on { theme::ink() } else { theme::ink3() },
    )
    .id(id)
    .cursor_pointer()
    .hover(|link| link.text_color(theme::ink()))
}
