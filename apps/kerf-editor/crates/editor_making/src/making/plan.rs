//! A making's plan, read before its yes ([`MakePlan`]): what Make… would
//! run on this computer, what its model has said it does not take, and the
//! take it writes, with Carry Steps into the Document first for a media
//! whose steps are still in a recipe file; and the plan and an ask as they
//! are said, on the panel and on the conversation's make cards.

use crate::making::{bind_close, Close, CONTEXT, HEAD};
use editor_session::session::{BridgeEvent, Session};
use editor_state::model::Source;
use editor_ui::icons::Icon;
use editor_ui::selectable;
use editor_ui::theme::{self, radius, text};
use editor_ui::ui::{self, icon_button};
use gpui::{
    div, prelude::*, px, AnyElement, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Rgba, SharedString, Subscription, Window,
};
use kerf_bridge::event::MediaMakePlan;
use kerf_bridge::{Command, Event};

/// What a making would run, as the bridge says it before anything runs
/// (`media_make_plan`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub media: String,
    /// Each step as it would run, in order.
    pub steps: Vec<String>,
    /// What its model has said it does not take.
    pub notes: Vec<String>,
    /// The first take it writes, and that take's file.
    pub take: u32,
    pub file: String,
    pub rolls: u32,
}

impl Plan {
    /// Reads the plan of a `media_make_plan` event.
    pub fn from_event(plan: &MediaMakePlan) -> Self {
        let mut steps = plan.steps.clone();
        if steps.is_empty() {
            steps = plan.command.lines().map(str::to_owned).collect();
        }
        Self {
            media: plan.media.clone(),
            steps,
            notes: plan.notes.clone(),
            take: plan.take,
            file: plan.file.clone(),
            rolls: plan.rolls.max(1),
        }
    }

    /// "take 3", or "takes 3–6" for several.
    pub fn takes(&self) -> String {
        takes_words(self.take, self.rolls)
    }
}

/// "take 3", "takes 3–6", or nothing before the first take is known.
pub fn takes_words(take: u32, rolls: u32) -> String {
    match (take, rolls) {
        (0, _) => String::new(),
        (take, 0 | 1) => format!("take {take}"),
        (take, rolls) => format!("takes {take}–{}", take + rolls - 1),
    }
}

/// What an ask's own words hold beside its first line: why Kerf asks, the
/// steps as it read them, and its notes. Where else to make it is left out:
/// the card's own Make says that.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AskWords {
    pub why: Vec<String>,
    pub steps: Vec<String>,
    pub notes: Vec<String>,
}

/// Reads an ask as `make` writes it ("Asks to make m4, as take 2, from …:",
/// why, "1. step", "Note: …", where else to make it).
pub fn ask_words(text: &str) -> AskWords {
    let mut words = AskWords::default();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || (index == 0 && line.starts_with("Asks to make ")) {
            continue;
        }
        if line.starts_with("Make it with ") || line.starts_with("Make them with ") {
            continue;
        }
        if line.starts_with("Each take after the first ") {
            continue;
        }
        if let Some(note) = line.strip_prefix("Note: ") {
            words.notes.push(note.to_owned());
            continue;
        }
        let numbered = line
            .split_once(". ")
            .filter(|(number, _)| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()));
        match numbered {
            Some((_, step)) => words.steps.push(step.to_owned()),
            None => words.why.push(line.to_owned()),
        }
    }
    words
}

/// Where a make card's words can be chosen and copied, as the
/// conversation's can: the conversation's selections, and the run the
/// words are pieces of. None: plain labels.
pub(super) type Choose<'a> = Option<(&'a selectable::Shared, SharedString)>;

/// `words` in `style` and `color`, piece `piece` of `choose`'s run where
/// they can be chosen, else a label.
fn words_piece(
    choose: &Choose,
    piece: usize,
    words: String,
    style: theme::TypeStyle,
    color: Rgba,
) -> gpui::Div {
    match choose {
        Some((shared, run)) => theme::type_style(div(), style).text_color(color).child(
            selectable::selectable(
                shared,
                ElementId::Name(format!("{run}-{piece}").into()),
                run.clone(),
                piece,
                words,
            )
            .before("\n"),
        ),
        None => ui::label(words, style, color),
    }
}

/// The steps a plan runs, a line each, in mono on the well.
fn steps_block(steps: &[String]) -> gpui::Div {
    steps_block_in(steps, &None)
}

/// [`steps_block`], its steps chosen and copied by `choose`.
pub(super) fn steps_block_in(steps: &[String], choose: &Choose) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::space::S2))
        .p(px(10.0))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line())
        .children(steps.iter().enumerate().map(|(index, step)| {
            div()
                .flex()
                .gap(px(theme::space::S8))
                .child(
                    ui::label(format!("{}", index + 1), text::MONO_SMALL, theme::ink3())
                        .flex_none(),
                )
                .child(
                    words_piece(choose, index, step.clone(), text::MONO_SMALL, theme::ink2())
                        .flex_1()
                        .min_w_0(),
                )
        }))
}

/// What a model has said it does not take, each worth mending before a making.
pub(super) fn notes_block(heading: &str, notes: &[String]) -> Option<gpui::Div> {
    notes_block_in(heading, notes, &None)
}

/// [`notes_block`], its notes chosen and copied by `choose`.
pub(super) fn notes_block_in(
    heading: &str,
    notes: &[String],
    choose: &Choose,
) -> Option<gpui::Div> {
    if notes.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S4))
            .child(ui::label(
                heading.to_owned(),
                text::META_STRONG,
                theme::ink3(),
            ))
            .children(notes.iter().enumerate().map(|(index, note)| {
                div()
                    .flex()
                    .gap(px(theme::space::S6))
                    .child(ui::label("•", text::CONTROL, theme::warn()).flex_none())
                    .child(
                        words_piece(choose, index, note.clone(), text::CONTROL, theme::warn())
                            .flex_1()
                            .min_w_0(),
                    )
            })),
    )
}

/// The message of a refused making without the "cannot make m1: " the
/// bridge puts first, which the panel's title says already.
pub(super) fn refusal(message: &str, media: &str) -> String {
    let said = message
        .strip_prefix(&format!("cannot make {media}: "))
        .or_else(|| message.strip_prefix(&format!("cannot carry {media}: ")))
        .unwrap_or(message);
    let mut letters = said.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// A media whose steps are still in the recipe file it names.
fn uncarried(source: &Source) -> bool {
    source.recipe.is_some() && source.steps == 0
}

/// Where the plan is.
enum Stage {
    /// The plan is asked for.
    Asking,
    Plan(Plan),
    /// It cannot be made as things stand: why.
    Refused(String),
    /// What carrying its steps into the document would write is asked for.
    Carrying,
    CarryPlan {
        recipe: String,
        script: Vec<String>,
        notes: Vec<String>,
    },
    /// The carry was refused: why.
    CarryRefused(String),
}

/// A making's plan, read before saying yes: what would run on this
/// computer, what its model has said it does not take, and the take it
/// writes. A media whose steps are still in a recipe file is offered Carry
/// Steps into the Document first.
pub struct MakePlan {
    session: Entity<Session>,
    media: SharedString,
    rolls: u32,
    focus_handle: FocusHandle,
    stage: Stage,
    /// What carrying said, once its steps were carried.
    carried: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl MakePlan {
    pub fn new(
        session: Entity<Session>,
        media: SharedString,
        rolls: u32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        bind_close(cx);
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.heard(&event.0, cx)
            }),
        ];
        let carry_first = session
            .read(cx)
            .timeline()
            .source(&media)
            .is_some_and(uncarried);
        let mut plan = Self {
            session,
            media,
            rolls: rolls.clamp(1, 16),
            focus_handle: cx.focus_handle(),
            stage: Stage::Asking,
            carried: None,
            _subscriptions: subscriptions,
        };
        if carry_first {
            plan.ask_carry(cx);
        } else {
            plan.ask_plan(cx);
        }
        plan
    }

    fn ask_plan(&mut self, cx: &mut Context<Self>) {
        let command = Command::MediaMake {
            media: self.media.to_string(),
            confirmed: false,
            rolls: (self.rolls > 1).then_some(self.rolls),
        };
        self.stage = Stage::Asking;
        let sent = self.session.update(cx, |session, _| session.send(command));
        if !sent {
            self.stage = Stage::Refused("Kerf is not connected.".into());
        }
        cx.notify();
    }

    fn ask_carry(&mut self, cx: &mut Context<Self>) {
        let command = Command::MediaCarry {
            media: self.media.to_string(),
            confirmed: false,
        };
        self.stage = Stage::Carrying;
        let sent = self.session.update(cx, |session, _| session.send(command));
        if !sent {
            self.stage = Stage::CarryRefused("Kerf is not connected.".into());
        }
        cx.notify();
    }

    fn heard(&mut self, event: &Event, cx: &mut Context<Self>) {
        // Only what is said of its own media.
        let media = match event {
            Event::MediaMakePlan(plan) => Some(plan.media.as_str()),
            Event::Error(error) => error.media.as_deref(),
            Event::MediaCarryPlan { media, .. } | Event::MediaCarried { media, .. } => {
                Some(media.as_str())
            }
            _ => None,
        };
        if media != Some(self.media.as_str()) {
            return;
        }
        match event {
            Event::MediaMakePlan(plan) if matches!(self.stage, Stage::Asking) => {
                self.stage = Stage::Plan(Plan::from_event(plan));
            }
            Event::Error(error)
                if error.command.as_deref() == Some("media_make")
                    && matches!(self.stage, Stage::Asking) =>
            {
                self.stage = Stage::Refused(refusal(&error.message, &self.media));
            }
            Event::MediaCarryPlan {
                recipe,
                script,
                notes,
                ..
            } if matches!(self.stage, Stage::Carrying) => {
                self.stage = Stage::CarryPlan {
                    recipe: recipe.clone(),
                    script: script.clone(),
                    notes: notes.clone(),
                };
            }
            Event::Error(error) if error.command.as_deref() == Some("media_carry") => {
                self.stage = Stage::CarryRefused(refusal(&error.message, &self.media));
            }
            Event::MediaCarried { recipe, held, .. } => {
                let held = *held;
                let takes = if held == 1 {
                    "1 of its takes is".to_owned()
                } else {
                    format!("{held} of its takes are")
                };
                self.carried = Some(format!(
                    "Carried the steps of {} into the document; {takes} held to them, and {recipe} is left where it is.",
                    self.media
                ));
                // Its steps are the document's now: what making them would run.
                self.ask_plan(cx);
            }
            _ => return,
        }
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn make(&mut self, cx: &mut Context<Self>) {
        let command = Command::MediaMake {
            media: self.media.to_string(),
            confirmed: true,
            rolls: (self.rolls > 1).then_some(self.rolls),
        };
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.send(command);
            cx.notify();
            sent
        });
        // The making shows on its tile, its clips and the picture's edge from here.
        if sent {
            self.dismiss(cx);
        }
    }

    fn carry(&mut self, cx: &mut Context<Self>) {
        let command = Command::MediaCarry {
            media: self.media.to_string(),
            confirmed: true,
        };
        let sent = self.session.update(cx, |session, _| session.send(command));
        if sent {
            self.stage = Stage::Carrying;
        }
        cx.notify();
    }

    fn body(&self, cx: &mut Context<Self>) -> (String, gpui::Div, Vec<AnyElement>) {
        let media = self.media.to_string();
        let rolls = self.rolls;
        let not_now = || {
            ui::button("plan-not-now", "Not now")
                .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx)))
                .into_any_element()
        };
        let mut body = div().flex().flex_col().gap(px(theme::space::S12));
        if let Some(carried) = &self.carried {
            body = body.child(ui::label(carried.clone(), text::CONTROL, theme::pos()));
        }
        let make_title = if rolls > 1 {
            format!("Make {rolls} takes of {media}")
        } else {
            format!("Make {media}")
        };
        match &self.stage {
            Stage::Asking => {
                body = body.child(ui::label(
                    "Reading what making it would run…",
                    text::BODY,
                    theme::ink3(),
                ));
                (make_title, body, vec![not_now()])
            }
            Stage::Plan(plan) => {
                let runs = if rolls > 1 {
                    format!("Making {media} runs this on your computer, {rolls} times:")
                } else {
                    format!("Making {media} runs this on your computer:")
                };
                body = body
                    .child(theme::reading(div()).text_color(theme::ink()).child(runs))
                    .child(steps_block(&plan.steps));
                if let Some(notes) = notes_block("Before you run it", &plan.notes) {
                    body = body.child(notes);
                }
                let writes = if rolls > 1 {
                    format!(
                        "It comes from its steps in the document and writes {rolls} new takes, {} to {}, the first at {}, each with every seed one on from the last. {media} then is the last of them, and you choose among them under its takes. The takes before them are kept.",
                        plan.take,
                        plan.take + rolls - 1,
                        plan.file
                    )
                } else {
                    format!(
                        "It comes from its steps in the document and writes a new take, {}, which {media} then is. The takes before it are kept.",
                        plan.file
                    )
                };
                body = body.child(ui::label(writes, text::CONTROL, theme::ink2()));
                let label = if rolls > 1 {
                    format!("Make {rolls}")
                } else {
                    "Make".to_owned()
                };
                let make = ui::button("plan-make", label)
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.make(cx)))
                    .into_any_element();
                (make_title, body, vec![not_now(), make])
            }
            Stage::Refused(why) => {
                body = body.child(ui::label(
                    "It cannot be made as things stand.",
                    text::BODY_NAME,
                    theme::ink(),
                ));
                body = body.child(ui::label(why.clone(), text::CONTROL, theme::neg()));
                let mut buttons = vec![];
                // A media whose steps are still in a recipe file is carried first.
                if why.contains("recipe file") || why.contains("kerf carry") {
                    buttons.push(
                        ui::button("plan-carry-ask", "Carry Steps into the Document…")
                            .on_click(cx.listener(|this, _, _, cx| this.ask_carry(cx)))
                            .into_any_element(),
                    );
                }
                buttons.push(
                    ui::button("plan-again", "Ask again")
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| this.ask_plan(cx)))
                        .into_any_element(),
                );
                buttons.push(
                    ui::button("plan-close", "Close")
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx)))
                        .into_any_element(),
                );
                (make_title, body, buttons)
            }
            Stage::Carrying => {
                body = body.child(ui::label(
                    "Reading what carrying its steps would write…",
                    text::BODY,
                    theme::ink3(),
                ));
                (
                    "Carry Steps into the Document".into(),
                    body,
                    vec![not_now()],
                )
            }
            Stage::CarryPlan {
                recipe,
                script,
                notes,
            } => {
                body = body
                    .child(theme::reading(div()).text_color(theme::ink()).child(format!(
                        "The steps of {media} are in the recipe file {recipe}, and a media is made from the steps under its line. Carrying them writes this into the document, as one edit undo takes back:"
                    )))
                    .child(steps_block(script));
                if let Some(notes) = notes_block("What does not come with them", notes) {
                    body = body.child(notes);
                }
                body = body.child(ui::label(
                    format!("Its takes are kept, {recipe} is left where it is, and nothing names it then."),
                    text::CONTROL,
                    theme::ink2(),
                ));
                let carry = ui::button("plan-carry", "Carry")
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.carry(cx)))
                    .into_any_element();
                (
                    "Carry Steps into the Document".into(),
                    body,
                    vec![not_now(), carry],
                )
            }
            Stage::CarryRefused(why) => {
                body = body.child(ui::label(why.clone(), text::CONTROL, theme::neg()));
                let close = ui::button("plan-close", "Close")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx)))
                    .into_any_element();
                ("Carry Steps into the Document".into(), body, vec![close])
            }
        }
    }
}

impl EventEmitter<DismissEvent> for MakePlan {}

impl Focusable for MakePlan {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The plan's width; its height is what it holds, up to the most.
const PLAN_WIDTH: f32 = 640.0;
const PLAN_MOST: f32 = 600.0;

impl Render for MakePlan {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, body, buttons) = self.body(cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _, cx| this.dismiss(cx)))
            // A press on the dim around the panel gives the window back.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss(cx)))
            .w(px(PLAN_WIDTH))
            .max_h(px(PLAN_MOST))
            .flex()
            .flex_col()
            .bg(theme::chrome())
            .font(theme::ui_font(gpui::FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(
                div()
                    .h(px(HEAD))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .px(px(theme::space::S16))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(Icon::Sparkle.view(16.0, theme::activity::gpu()))
                    .child(
                        ui::line(title, text::TITLE, theme::ink())
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(
                        icon_button("plan-x", Icon::Close)
                            .tooltip("Close")
                            .key("Esc")
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                    ),
            )
            .child(
                div()
                    .id("plan-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.0))
                    .py(px(theme::space::S16))
                    .child(body),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .px(px(theme::space::S16))
                    .py(px(theme::space::S12))
                    .border_t_1()
                    .border_color(theme::line())
                    .children(buttons),
            )
    }
}
