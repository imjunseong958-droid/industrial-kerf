//! Shots another program makes: what Kerf asks to make in the conversation,
//! a media's Make… plan and its yes, and a shot's graph.
//!
//! Nothing here runs a step on its own. A making is `media_make`: asked
//! without `confirmed`, the bridge answers with the plan, what would run; only
//! a person's Make sends it again with `confirmed: true`. The graph lays out a
//! media's steps as the bridge reads them (`recipe`), and a change to one is a
//! `recipe_edit` the bridge applies as one edit, which undo takes back.
//!
//! Nothing in the graph moves by itself: a card stays where it was laid out
//! or dragged, and only a making's amber stripes run over its takes' card.

use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::{Ask, Source};
use crate::motion;
use crate::session::{BridgeEvent, Session};
use crate::store::Making;
use crate::theme::{self, motion as scene, radius, text};
use crate::ui::{self, icon_button};
use gpui::{
    actions, canvas, div, fill, point, prelude::*, px, size, AnimationExt, AnyElement, App, Bounds,
    Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Rgba,
    ScrollWheelEvent, SharedString, Subscription, Window,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Open the graph of the steps that make `media`.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct OpenShotGraph {
    pub media: SharedString,
}

/// Make `rolls` takes of `media` (1 for Make…, more for Make Several), after
/// a person has read the plan and said yes.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct MakeMedia {
    pub media: SharedString,
    pub rolls: u32,
}

actions!(making, [Close]);

/// The key context the plan and the graph answer in.
const CONTEXT: &str = "Making";

/// Esc gives the window back, before the window's own Esc has it: this
/// context is the deeper one. Binding a keymap clears it, so it is bound
/// again whenever a panel opens and finds it missing.
fn bind_close(cx: &mut App) {
    if !crate::settings::escape_bound(cx, &Close) {
        cx.bind_keys([KeyBinding::new("escape", Close, Some(CONTEXT))]);
    }
}

// ------------------------------------------------------------------ plans

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
    /// Reads a `media_make_plan` event; None for any other.
    pub fn from_event(event: &Value) -> Option<Self> {
        if event["type"] != "media_make_plan" {
            return None;
        }
        let media = event["media"].as_str()?.to_owned();
        let mut steps = strings(&event["steps"]);
        if steps.is_empty() {
            steps = event["command"]
                .as_str()
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect();
        }
        Some(Self {
            media,
            steps,
            notes: strings(&event["notes"]),
            take: number(&event["take"]),
            file: event["file"].as_str().unwrap_or_default().to_owned(),
            rolls: number(&event["rolls"]).max(1),
        })
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

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn number(value: &Value) -> u32 {
    value
        .as_u64()
        .map_or(0, |value| value.min(u64::from(u32::MAX)) as u32)
}

/// A value as a field shows it: words as they are, a whole number without a
/// point, nothing for null.
fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(words) => words.clone(),
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(whole), _) => whole.to_string(),
            (None, Some(real)) => plain(real),
            _ => number.to_string(),
        },
        Value::Bool(on) => on.to_string(),
        other => other.to_string(),
    }
}

fn plain(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        let written = format!("{value:.3}");
        written
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// `m:ss` since `since`.
fn clock(since: std::time::Instant) -> String {
    let elapsed = since.elapsed().as_secs();
    format!("{}:{:02}", elapsed / 60, elapsed % 60)
}

/// The steps a plan runs, a line each, in mono on the well.
fn steps_block(steps: &[String]) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .p(px(10.0))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line())
        .children(steps.iter().enumerate().map(|(index, step)| {
            div()
                .flex()
                .gap(px(8.0))
                .child(
                    ui::label(format!("{}", index + 1), text::MONO_SMALL, theme::ink3())
                        .flex_none(),
                )
                .child(
                    ui::label(step.clone(), text::MONO_SMALL, theme::ink2())
                        .flex_1()
                        .min_w_0(),
                )
        }))
}

/// What a model has said it does not take, each worth mending before a making.
fn notes_block(heading: &str, notes: &[String]) -> Option<gpui::Div> {
    if notes.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(ui::label(
                heading.to_owned(),
                text::META_STRONG,
                theme::ink3(),
            ))
            .children(notes.iter().map(|note| {
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(ui::label("•", text::CONTROL, theme::warn()).flex_none())
                    .child(
                        ui::label(note.clone(), text::CONTROL, theme::warn())
                            .flex_1()
                            .min_w_0(),
                    )
            })),
    )
}

/// The message of a refused making without the "cannot make m1: " the
/// bridge puts first, which the panel's title says already.
fn refusal(message: &str, media: &str) -> String {
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
    source.recipe.is_some() && source.steps.unwrap_or(0) == 0
}

// ------------------------------------------------------- makings in the conversation

/// How a making asked for in the conversation goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardState {
    Asked,
    Making,
    Made,
    Stopped,
    Failed,
    Declined,
}

/// What the cards have heard of a media's making.
#[derive(Clone, Debug, PartialEq)]
struct Progress {
    state: CardState,
    detail: String,
    take: u32,
}

/// The makings Kerf asked for in the conversation open, as cards: what it
/// asks to make and the plan, with Make and Not now; then the making as it
/// goes, with Stop; once made, Look at it.
pub struct Makings {
    session: Entity<Session>,
    /// Asks heard as Kerf made them, with the conversation they were asked
    /// in, until the conversation keeps them.
    live: Vec<(Option<String>, Ask)>,
    /// What each media's making came to, as far as these cards know.
    progress: HashMap<String, Progress>,
    /// Cards a person said Not now to.
    declined: HashSet<String>,
    /// Each plan by media and rolls, or why there is none.
    plans: HashMap<(String, u32), Result<Plan, String>>,
    /// Plans asked for and not yet answered.
    asked: HashSet<(String, u32)>,
    /// Cards whose plan is unfolded or folded by hand.
    unfolded: HashSet<String>,
    folded: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl Makings {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.heard(&event.0, cx)
            }),
        ];
        let mut makings = Self {
            session,
            live: Vec::new(),
            progress: HashMap::new(),
            declined: HashSet::new(),
            plans: HashMap::new(),
            asked: HashSet::new(),
            unfolded: HashSet::new(),
            folded: HashSet::new(),
            _subscriptions: subscriptions,
        };
        makings.ask_plans(cx);
        makings
    }

    fn heard(&mut self, event: &Value, cx: &mut Context<Self>) {
        let kind = event["type"].as_str().unwrap_or_default();
        let media = event["media"].as_str().unwrap_or_default().to_owned();
        match kind {
            "agent_ask" => {
                let ask = Ask {
                    media,
                    rolls: event["rolls"]
                        .as_u64()
                        .map(|rolls| rolls as u32)
                        .filter(|rolls| *rolls > 1),
                    take: event["take"].as_u64().map(|take| take as u32),
                    text: event["text"].as_str().unwrap_or_default().to_owned(),
                };
                let conversation = event["conversation"].as_str().map(str::to_owned);
                if !ask.media.is_empty()
                    && !self.live.iter().any(|(_, known)| key(known) == key(&ask))
                {
                    self.live.push((conversation, ask));
                }
            }
            "media_make_plan" => {
                if let Some(plan) = Plan::from_event(event) {
                    let wanted = (plan.media.clone(), plan.rolls);
                    self.asked.remove(&wanted);
                    self.plans.insert(wanted, Ok(plan));
                }
            }
            "media_making" => {
                let line = event["line"].as_str().unwrap_or_default().to_owned();
                let take = number(&event["take"]);
                self.progress.insert(
                    media,
                    Progress {
                        state: CardState::Making,
                        detail: line,
                        take,
                    },
                );
            }
            "media_made" => {
                self.progress.insert(
                    media,
                    Progress {
                        state: CardState::Made,
                        detail: String::new(),
                        take: number(&event["take"]),
                    },
                );
            }
            "media_make_cancelled" => {
                self.progress.insert(
                    media,
                    Progress {
                        state: CardState::Stopped,
                        detail: String::new(),
                        take: 0,
                    },
                );
            }
            "error" if event["command"] == "media_make" && !media.is_empty() => {
                let message = event["message"]
                    .as_str()
                    .unwrap_or("The making failed")
                    .to_owned();
                let making = self
                    .progress
                    .get(&media)
                    .is_some_and(|known| known.state == CardState::Making);
                if making {
                    self.progress.insert(
                        media,
                        Progress {
                            state: CardState::Failed,
                            detail: refusal(&message, event["media"].as_str().unwrap_or_default()),
                            take: 0,
                        },
                    );
                } else {
                    // A plan asked for that cannot be made: why, on its card.
                    let waiting: Vec<_> = self
                        .asked
                        .iter()
                        .filter(|(of, _)| *of == media)
                        .cloned()
                        .collect();
                    for wanted in waiting {
                        self.asked.remove(&wanted);
                        self.plans.insert(wanted, Err(refusal(&message, &media)));
                    }
                }
            }
            _ => {}
        }
        if matches!(kind, "agent_ask" | "conversation" | "connection" | "state") {
            self.ask_plans(cx);
        }
    }

    /// The asks of the conversation open: those it keeps, then those heard
    /// since that it does not keep yet. One asked twice is one card.
    fn asks(&self, session: &Session) -> Vec<Ask> {
        // The conversation the chat shows: the one open, else the one spoken in.
        let open = session.state.open_conversation.as_ref();
        let open_id = open
            .map(|conversation| conversation.id.clone())
            .or_else(|| session.conversation.clone());
        let mut asks: Vec<Ask> = Vec::new();
        if let Some(conversation) = open {
            for ask in conversation.items.iter().flat_map(|turn| turn.asks.iter()) {
                if !asks.iter().any(|known| key(known) == key(ask)) {
                    asks.push(ask.clone());
                }
            }
        }
        for (conversation, ask) in &self.live {
            let here = conversation.is_none()
                || conversation.as_deref() == open_id.as_deref()
                || conversation.as_deref() == session.state.work.conversation.as_deref();
            if here && !asks.iter().any(|known| key(known) == key(ask)) {
                asks.push(ask.clone());
            }
        }
        asks
    }

    /// Asks the bridge for the plan of each card still waiting on a yes.
    fn ask_plans(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if !session.has_project() {
            return;
        }
        let mut wanted = Vec::new();
        for ask in self.asks(session) {
            let (state, _) = self.state_of(&ask, session);
            let rolls = ask.rolls.unwrap_or(1).max(1);
            let plan = (ask.media.clone(), rolls);
            if matches!(state, CardState::Asked)
                && !self.plans.contains_key(&plan)
                && !self.asked.contains(&plan)
            {
                wanted.push(plan);
            }
        }
        if wanted.is_empty() {
            return;
        }
        self.session.update(cx, |session, _| {
            for (media, rolls) in &wanted {
                let mut command = json!({"type": "media_make", "media": media});
                if *rolls > 1 {
                    command["rolls"] = json!(rolls);
                }
                session.send(command);
            }
        });
        self.asked.extend(wanted);
    }

    /// How a card stands, and what its making said last.
    fn state_of(&self, ask: &Ask, session: &Session) -> (CardState, Progress) {
        let take = ask.take.unwrap_or(0);
        let last = take + ask.rolls.unwrap_or(1).max(1) - 1;
        let made_already = take > 0
            && session
                .timeline()
                .source(&ask.media)
                .is_some_and(|source| source.takes.iter().any(|known| known.number >= last));
        let progress = self.progress.get(&ask.media).cloned();
        // A making that failed is said so, whatever was last heard of it running.
        if let Some(failed) = progress
            .as_ref()
            .filter(|known| known.state == CardState::Failed)
        {
            return (CardState::Failed, failed.clone());
        }
        // A making under way, whoever runs it, is shown as it goes.
        if let Some(making) = session.state.makings.get(&ask.media) {
            let detail = progress
                .as_ref()
                .filter(|known| known.state == CardState::Making)
                .map(|known| known.detail.clone())
                .unwrap_or_else(|| making.line.clone());
            return (
                CardState::Making,
                Progress {
                    state: CardState::Making,
                    detail,
                    take: making.take,
                },
            );
        }
        if made_already {
            return (
                CardState::Made,
                Progress {
                    state: CardState::Made,
                    detail: String::new(),
                    take: last,
                },
            );
        }
        match progress {
            // A take made before this ask's is not what it asked for.
            Some(known) if known.state == CardState::Made && known.take < take => {}
            Some(known) => return (known.state, known),
            None => {}
        }
        let state = if self.declined.contains(&key(ask)) {
            CardState::Declined
        } else {
            CardState::Asked
        };
        (
            state,
            Progress {
                state,
                detail: String::new(),
                take,
            },
        )
    }

    fn make(&mut self, ask: &Ask, take: u32, cx: &mut Context<Self>) {
        let rolls = ask.rolls.unwrap_or(1).max(1);
        let media = ask.media.clone();
        self.session.update(cx, |session, cx| {
            let mut command = json!({"type": "media_make", "media": media, "confirmed": true});
            if rolls > 1 {
                command["rolls"] = json!(rolls);
            }
            session.send(command);
            cx.notify();
        });
        // Shown at once; the bridge's media_making takes it from here.
        self.progress.insert(
            ask.media.clone(),
            Progress {
                state: CardState::Making,
                detail: String::new(),
                take,
            },
        );
        self.declined.remove(&key(ask));
        cx.notify();
    }

    fn card(&self, ask: &Ask, facts: Facts, cx: &mut Context<Self>) -> AnyElement {
        let card_key = key(ask);
        let Facts {
            state,
            progress,
            making,
            connected,
            busy,
        } = facts;
        let rolls = ask.rolls.unwrap_or(1).max(1);
        let media = ask.media.clone();
        let plan = self.plans.get(&(ask.media.clone(), rolls)).cloned();
        let words = ask_words(&ask.text);
        let again = matches!(
            state,
            CardState::Stopped | CardState::Failed | CardState::Declined
        );
        let asked = state == CardState::Asked;
        let plan_open = if self.folded.contains(&card_key) {
            false
        } else {
            asked || again || self.unfolded.contains(&card_key)
        };
        let take = match &plan {
            Some(Ok(plan)) => plan.take,
            _ => ask.take.unwrap_or(0),
        };
        let title = match state {
            CardState::Asked if rolls > 1 => format!("Make {rolls} takes of {media}?"),
            CardState::Asked => format!("Make {media}?"),
            CardState::Making => format!("Making {media}"),
            CardState::Made if progress.take > 0 => {
                format!("Take {} of {media} is made", progress.take)
            }
            CardState::Made => format!("{media} is made"),
            CardState::Stopped => format!("Stopped making {media}"),
            CardState::Failed => format!("{media} was not made"),
            CardState::Declined => format!("Not made: {media}"),
        };
        let status: Option<(String, Rgba)> = match state {
            CardState::Making => {
                let elapsed = making
                    .as_ref()
                    .map(|making| clock(making.since))
                    .unwrap_or_else(|| "0:00".into());
                let line = if progress.detail.is_empty() {
                    "Starting…".to_owned()
                } else {
                    progress.detail.clone()
                };
                Some((
                    format!("Take {} · {elapsed} · {line}", progress.take.max(take)),
                    theme::activity::gpu(),
                ))
            }
            CardState::Failed => Some((progress.detail.clone(), theme::neg())),
            CardState::Stopped => Some(("The media is as it was.".into(), theme::ink3())),
            _ => None,
        };
        let plan_error = match &plan {
            Some(Err(why)) => Some(why.clone()),
            _ => None,
        };

        let mut body = div().flex().flex_col().gap(px(10.0));
        if plan_open {
            if !words.why.is_empty() {
                body = body.child(
                    theme::reading(div())
                        .text_color(theme::ink2())
                        .child(words.why.join(" ")),
                );
            }
            match &plan {
                Some(Ok(plan)) => {
                    body = body.child(steps_block(&plan.steps));
                    if let Some(notes) = notes_block("Before you run it", &plan.notes) {
                        body = body.child(notes);
                    }
                    if !plan.file.is_empty() {
                        body = body.child(ui::label(
                            format!("Writes {} · {}", plan.takes(), plan.file),
                            text::MONO_SMALL,
                            theme::ink3(),
                        ));
                    }
                }
                Some(Err(why)) => {
                    if !words.steps.is_empty() {
                        body = body.child(steps_block(&words.steps));
                    }
                    body = body.child(ui::label(
                        format!("It cannot be made as things stand: {why}"),
                        text::CONTROL,
                        theme::neg(),
                    ));
                }
                None => {
                    if !words.steps.is_empty() {
                        body = body.child(steps_block(&words.steps));
                    }
                    if let Some(notes) = notes_block("Before you run it", &words.notes) {
                        body = body.child(notes);
                    }
                    if asked {
                        body =
                            body.child(ui::label("Reading the plan…", text::META, theme::ink3()));
                    }
                }
            }
        }
        if let Some((words, color)) = status {
            body = body.child(ui::label(words, text::CONTROL, color));
        }

        let toggle_key = card_key.clone();
        let mut footer = div().flex().items_center().gap(px(8.0)).child(
            div()
                .id(ElementId::Name(format!("plan-toggle-{card_key}").into()))
                .flex()
                .items_center()
                .gap(px(4.0))
                .cursor_pointer()
                .child(
                    if plan_open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    }
                    .view(12.0, theme::ink3()),
                )
                .child(ui::label("Plan", text::META, theme::ink3()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if plan_open {
                        this.unfolded.remove(&toggle_key);
                        this.folded.insert(toggle_key.clone());
                    } else {
                        this.folded.remove(&toggle_key);
                        this.unfolded.insert(toggle_key.clone());
                    }
                    cx.notify();
                })),
        );
        footer = footer.child(div().flex_1());
        if asked {
            let declined = card_key.clone();
            footer = footer.child(
                ui::button(
                    ElementId::Name(format!("not-now-{card_key}").into()),
                    "Not now",
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.declined.insert(declined.clone());
                    cx.notify();
                })),
            );
        }
        if asked || again {
            let label = if asked && rolls > 1 {
                format!("Make {rolls}")
            } else {
                "Make".to_owned()
            };
            let ask = ask.clone();
            footer = footer.child(
                ui::button(ElementId::Name(format!("make-{card_key}").into()), label)
                    .primary()
                    .disabled(!connected || plan_error.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| this.make(&ask, take, cx))),
            );
        }
        if state == CardState::Making {
            let stop = ask.media.clone();
            footer = footer.child(
                ui::button(ElementId::Name(format!("stop-{card_key}").into()), "Stop")
                    .icon(Icon::Stop)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session.update(cx, |session, _| {
                            session.send(json!({"type": "media_make_cancel", "media": stop}));
                        });
                    })),
            );
        }
        if state == CardState::Made && progress.take > 0 {
            let look = format!(
                "Take {} of {} is made. Look at it and tell me how it came out against what I asked for.",
                progress.take, ask.media
            );
            footer = footer.child(
                ui::button(
                    ElementId::Name(format!("look-{card_key}").into()),
                    "Look at it",
                )
                .icon(Icon::Eye)
                .disabled(busy || !connected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session.update(cx, |session, cx| {
                        session.ask(&look);
                        cx.notify();
                    });
                })),
            );
        }

        let takes = takes_words(take, rolls);
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0))
            .rounded(px(radius::CARD))
            .overflow_hidden()
            .bg(theme::raised())
            .border_1()
            .border_color(theme::line())
            // Asked: an amber edge along its top, a card waiting on the person.
            .when(asked, |card| {
                card.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.0))
                        .bg(theme::activity::gpu()),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(Icon::Sparkle.view(
                        16.0,
                        if asked {
                            theme::activity::gpu()
                        } else {
                            theme::ink2()
                        },
                    ))
                    .child(
                        ui::line(title, text::TITLE, theme::ink())
                            .flex_1()
                            .min_w_0(),
                    )
                    .when(!takes.is_empty(), |row| {
                        row.child(ui::label(takes, text::MONO_SMALL, theme::ink3()))
                    }),
            )
            .child(body)
            .child(footer)
            .with_animation(
                ElementId::Name(format!("make-card-{card_key}").into()),
                motion::animation(scene::APPEAR),
                |card, t| card.opacity(t).mt(px(scene::APPEAR_RISE * (1.0 - t))),
            )
            .into_any_element()
    }
}

/// What a card is drawn from, read from the session before it is drawn.
struct Facts {
    state: CardState,
    progress: Progress,
    making: Option<Making>,
    connected: bool,
    busy: bool,
}

/// A card's key: a making asked for twice is one card.
fn key(ask: &Ask) -> String {
    format!("{}#{}", ask.media, ask.take.unwrap_or(0))
}

impl Render for Makings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let connected = session.state.connection == crate::store::Connection::Connected;
        let busy = session.state.work.busy();
        let shown: Vec<(Ask, Facts)> = self
            .asks(session)
            .into_iter()
            .map(|ask| {
                let (state, progress) = self.state_of(&ask, session);
                let making = session.state.makings.get(&ask.media).cloned();
                let facts = Facts {
                    state,
                    progress,
                    making,
                    connected,
                    busy,
                };
                (ask, facts)
            })
            .collect();
        let cards: Vec<AnyElement> = shown
            .into_iter()
            .map(|(ask, facts)| self.card(&ask, facts, cx))
            .collect();
        div().flex().flex_col().gap(px(12.0)).children(cards)
    }
}

// ------------------------------------------------------------------ Make… plan

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
        let mut command = json!({"type": "media_make", "media": self.media.as_ref()});
        if self.rolls > 1 {
            command["rolls"] = json!(self.rolls);
        }
        self.stage = Stage::Asking;
        let sent = self.session.update(cx, |session, _| session.send(command));
        if !sent {
            self.stage = Stage::Refused("Kerf is not connected.".into());
        }
        cx.notify();
    }

    fn ask_carry(&mut self, cx: &mut Context<Self>) {
        let command = json!({"type": "media_carry", "media": self.media.as_ref()});
        self.stage = Stage::Carrying;
        let sent = self.session.update(cx, |session, _| session.send(command));
        if !sent {
            self.stage = Stage::CarryRefused("Kerf is not connected.".into());
        }
        cx.notify();
    }

    fn heard(&mut self, event: &Value, cx: &mut Context<Self>) {
        if event["media"].as_str() != Some(self.media.as_ref()) {
            return;
        }
        let message = || {
            event["message"]
                .as_str()
                .unwrap_or("It was refused")
                .to_owned()
        };
        match event["type"].as_str().unwrap_or_default() {
            "media_make_plan" if matches!(self.stage, Stage::Asking) => {
                if let Some(plan) = Plan::from_event(event) {
                    self.stage = Stage::Plan(plan);
                }
            }
            "error" if event["command"] == "media_make" && matches!(self.stage, Stage::Asking) => {
                self.stage = Stage::Refused(refusal(&message(), &self.media));
            }
            "media_carry_plan" if matches!(self.stage, Stage::Carrying) => {
                self.stage = Stage::CarryPlan {
                    recipe: event["recipe"].as_str().unwrap_or_default().to_owned(),
                    script: strings(&event["script"]),
                    notes: strings(&event["notes"]),
                };
            }
            "error" if event["command"] == "media_carry" => {
                self.stage = Stage::CarryRefused(refusal(&message(), &self.media));
            }
            "media_carried" => {
                let held = number(&event["held"]);
                let recipe = event["recipe"].as_str().unwrap_or_default();
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
        let mut command =
            json!({"type": "media_make", "media": self.media.as_ref(), "confirmed": true});
        if self.rolls > 1 {
            command["rolls"] = json!(self.rolls);
        }
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
        let command =
            json!({"type": "media_carry", "media": self.media.as_ref(), "confirmed": true});
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
        let mut body = div().flex().flex_col().gap(px(12.0));
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
                    .gap(px(8.0))
                    .px(px(16.0))
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
                    .py(px(16.0))
                    .child(body),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(theme::line())
                    .children(buttons),
            )
    }
}

// ------------------------------------------------------------------ the graph

/// One node of a shot's graph, as the bridge's `recipe` lays it out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub gives: String,
    /// The step's line, from 1, for a step.
    pub line: Option<usize>,
    /// The whole node, for what else it holds by its kind.
    pub holds: Value,
}

/// One reading: what `from` gives, read by `to` under the name `port`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub port: String,
    pub nth: usize,
    pub gives: String,
}

/// A media's steps as a graph (the bridge's `recipe`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recipe {
    pub media: String,
    pub recipe: String,
    pub document: bool,
    pub name: String,
    /// The steps as text, as the bridge read them: what an edit says it changes.
    pub text: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub error: Option<String>,
    pub cannot: Option<String>,
    pub notes: Vec<String>,
    pub models_error: Option<String>,
}

impl Recipe {
    /// Reads a `recipe` event, leniently.
    pub fn read(event: &Value) -> Self {
        let words = |key: &str| event[key].as_str().unwrap_or_default().to_owned();
        let said = |key: &str| {
            event[key]
                .as_str()
                .filter(|said| !said.is_empty())
                .map(str::to_owned)
        };
        let nodes = event["nodes"]
            .as_array()
            .map(|nodes| {
                nodes
                    .iter()
                    .map(|node| GraphNode {
                        id: node["id"].as_str().unwrap_or_default().to_owned(),
                        kind: node["kind"].as_str().unwrap_or_default().to_owned(),
                        title: node["title"].as_str().unwrap_or_default().to_owned(),
                        detail: node["detail"].as_str().unwrap_or_default().to_owned(),
                        gives: node["gives"].as_str().unwrap_or_default().to_owned(),
                        line: node["line"].as_u64().map(|line| line as usize),
                        holds: node.clone(),
                    })
                    .filter(|node| !node.id.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let edges = event["edges"]
            .as_array()
            .map(|edges| {
                edges
                    .iter()
                    .map(|edge| GraphEdge {
                        from: edge["from"].as_str().unwrap_or_default().to_owned(),
                        to: edge["to"].as_str().unwrap_or_default().to_owned(),
                        port: edge["port"].as_str().unwrap_or_default().to_owned(),
                        nth: edge["nth"].as_u64().unwrap_or(0) as usize,
                        gives: edge["gives"].as_str().unwrap_or_default().to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            media: words("media"),
            recipe: words("recipe"),
            document: event["document"].as_bool().unwrap_or(false),
            name: words("name"),
            text: words("text"),
            nodes,
            edges,
            error: said("error"),
            cannot: said("cannot"),
            notes: strings(&event["notes"]),
            models_error: said("modelsError"),
        }
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// What reads into `id`, in the order the steps give it.
    pub fn reads_into(&self, id: &str) -> Vec<&GraphEdge> {
        self.edges.iter().filter(|edge| edge.to == id).collect()
    }
}

/// A node's card, and the room between the cards for the edges to turn in.
const CARD_W: f32 = 208.0;
const CARD_H: f32 = 56.0;
/// What a model's line of its seed and size adds to its card.
const CARD_MODEL: f32 = 18.0;
const COLUMN_GAP: f32 = 112.0;
const ROW_GAP: f32 = 20.0;
/// The canvas's dots.
const GRID: f32 = 24.0;
/// The panel at least and at most, where the window has room, and the air
/// it leaves around it; its head; the column that says what the chosen node
/// holds.
const GRAPH_LEAST: (f32, f32) = (960.0, 620.0);
const GRAPH_MOST: (f32, f32) = (1400.0, 900.0);
const GRAPH_AIR: (f32, f32) = (120.0, 100.0);
const HEAD: f32 = 48.0;
const INSPECTOR: f32 = 300.0;
/// The least a graph is drawn at to fit its canvas, and the air it leaves there.
const LEAST_ZOOM: f32 = 0.6;
const FIT_AIR: f32 = 24.0;
/// The legend of what the edges carry, at the canvas's foot.
const LEGEND: f32 = 28.0;
/// A field's name, before it.
const FIELD_LABEL: f32 = 76.0;

/// The graph panel's size in a window `viewport` big: as much as the window
/// has room for, between the least and the most.
fn graph_size(viewport: (f32, f32)) -> (f32, f32) {
    let side = |room: f32, air: f32, least: f32, most: f32| {
        (room - air)
            .min(most)
            .max(least.min(room - 32.0))
            .max(320.0)
    };
    (
        side(viewport.0, GRAPH_AIR.0, GRAPH_LEAST.0, GRAPH_MOST.0),
        side(viewport.1, GRAPH_AIR.1, GRAPH_LEAST.1, GRAPH_MOST.1),
    )
}

/// How a laid-out graph is drawn on a canvas `room` big: at the zoom that fits
/// it, no more than as laid out and no less than [`LEAST_ZOOM`], in the
/// middle, then moved by `pan`. Where a card is drawn is `offset + place * zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub zoom: f32,
    pub offset: (f32, f32),
}

pub fn fit(
    recipe: &Recipe,
    laid: &HashMap<String, (f32, f32)>,
    room: (f32, f32),
    pan: (f32, f32),
) -> Fit {
    let inputs = |id: &str| recipe.edges.iter().filter(|edge| edge.to == id).count();
    let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for node in &recipe.nodes {
        if let Some((x, y)) = laid.get(&node.id) {
            left = left.min(*x);
            top = top.min(*y);
            right = right.max(x + CARD_W);
            bottom = bottom.max(y + card_height(node, inputs(&node.id)));
        }
    }
    if left > right {
        return Fit {
            zoom: 1.0,
            offset: pan,
        };
    }
    let (width, height) = (room.0 - 2.0 * FIT_AIR, room.1 - LEGEND - 2.0 * FIT_AIR);
    let zoom = (width / (right - left))
        .min(height / (bottom - top))
        .clamp(LEAST_ZOOM, 1.0);
    Fit {
        zoom,
        offset: (
            ((room.0 - (right - left) * zoom) / 2.0 - left * zoom + pan.0).round(),
            ((room.1 - LEGEND - (bottom - top) * zoom) / 2.0 - top * zoom + pan.1).round(),
        ),
    }
}

/// `element`'s type set in `style` at `zoom`.
fn zoomed(element: gpui::Div, style: theme::TypeStyle, zoom: f32) -> gpui::Div {
    element
        .text_size(px(style.px * zoom))
        .line_height(px(style.line * zoom))
}

/// A card's height: room for its lines, and for each thing it reads to come in apart.
pub fn card_height(node: &GraphNode, inputs: usize) -> f32 {
    let lines = if node.kind == "model" {
        CARD_H + CARD_MODEL
    } else {
        CARD_H
    };
    lines.max(16.0 + 24.0 * inputs as f32)
}

/// Where each card goes, its top left, left to right by what it reads: a
/// node a column after the furthest thing it reads, what nothing comes into
/// beside the first that reads it, and each column centred on the middle,
/// top to bottom in the order of what each feeds.
pub fn arrange(recipe: &Recipe) -> HashMap<String, (f32, f32)> {
    let count = recipe.nodes.len();
    let index: HashMap<&str, usize> = recipe
        .nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.id.as_str(), at))
        .collect();
    let wired: Vec<(usize, usize)> = recipe
        .edges
        .iter()
        .filter_map(|edge| {
            Some((
                *index.get(edge.from.as_str())?,
                *index.get(edge.to.as_str())?,
            ))
        })
        .collect();
    let mut column = vec![0usize; count];
    // How far each is from what it reads: the longest way to it. A loop,
    // which steps never make, stops after as many passes as there are nodes.
    for _ in 0..count {
        let mut moved = false;
        for &(from, to) in &wired {
            if column[to] < column[from] + 1 {
                column[to] = column[from] + 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    for node in 0..count {
        if wired.iter().any(|&(_, to)| to == node) {
            continue;
        }
        let nearest = wired
            .iter()
            .filter(|&&(from, _)| from == node)
            .map(|&(_, to)| column[to])
            .min();
        if let Some(nearest) = nearest.filter(|nearest| *nearest > 0) {
            column[node] = nearest - 1;
        }
    }
    let inputs = |node: usize| wired.iter().filter(|&&(_, to)| to == node).count();
    let last = column.iter().copied().max().unwrap_or(0);
    let mut row: HashMap<usize, usize> = HashMap::new();
    let mut places = HashMap::new();
    for at in (0..=last).rev() {
        let mut here: Vec<usize> = (0..count).filter(|node| column[*node] == at).collect();
        let order = |node: usize| {
            wired
                .iter()
                .filter(|&&(from, _)| from == node)
                .map(|&(_, to)| {
                    let into: Vec<usize> = wired
                        .iter()
                        .filter(|&&(_, reader)| reader == to)
                        .map(|&(from, _)| from)
                        .collect();
                    let input = into.iter().rposition(|from| *from == node).unwrap_or(0);
                    row.get(&to).copied().unwrap_or(0) * 64 + input
                })
                .min()
                .unwrap_or(usize::MAX)
        };
        here.sort_by_key(|node| (order(*node), *node));
        let heights: Vec<f32> = here
            .iter()
            .map(|node| card_height(&recipe.nodes[*node], inputs(*node)))
            .collect();
        let total: f32 =
            heights.iter().sum::<f32>() + ROW_GAP * here.len().saturating_sub(1) as f32;
        let mut y = -total / 2.0;
        for (place, (node, height)) in here.iter().zip(heights).enumerate() {
            row.insert(*node, place);
            places.insert(
                recipe.nodes[*node].id.clone(),
                (at as f32 * (CARD_W + COLUMN_GAP), y.round()),
            );
            y += height + ROW_GAP;
        }
    }
    places
}

/// What an edge carrying `gives` is coloured.
fn gives_color(gives: &str) -> Rgba {
    match gives {
        "video" => theme::accent_ink(),
        "image" => theme::pos(),
        "path" => theme::playhead(),
        "signal" => theme::warn(),
        _ => theme::ink3(),
    }
}

fn kind_icon(kind: &str) -> Icon {
    match kind {
        "sequence" => Icon::Film,
        "render" => Icon::Eye,
        "model" => Icon::Sparkle,
        "join" => Icon::Scissors,
        "file" => Icon::Folder,
        "signal" => Icon::Clock,
        "moves" => Icon::ChevronRight,
        "cast" | "style" => Icon::Chat,
        "takes" => Icon::Media,
        _ => Icon::Dot,
    }
}

/// A model's seed, length and size, as its card's third line says them.
fn model_line(node: &GraphNode) -> String {
    let holds = &node.holds;
    let mut said = Vec::new();
    if !holds["seed"].is_null() {
        said.push(format!("seed {}", value_text(&holds["seed"])));
    }
    if !holds["seconds"].is_null() {
        said.push(format!("{} s", value_text(&holds["seconds"])));
    }
    if let Some(size) = holds["size"].as_str() {
        said.push(size.to_owned());
    }
    if said.is_empty() {
        "as the model comes".to_owned()
    } else {
        said.join(" · ")
    }
}

/// What is wrong with a node, by a dot: not there in brick, wanting a look in ochre.
fn node_mark(node: &GraphNode) -> Option<Rgba> {
    let holds = &node.holds;
    if holds["there"] == Value::Bool(false) {
        return Some(theme::neg());
    }
    if node.kind == "model" && holds["setUp"] == Value::Bool(false) {
        return Some(theme::warn());
    }
    if node.kind == "takes" && !holds["wants"].is_null() {
        return Some(theme::warn());
    }
    None
}

/// A field the chosen node's column shows: a name on its line to set.
#[derive(Clone, Debug, PartialEq)]
struct Want {
    key: String,
    label: String,
    value: String,
    placeholder: String,
    multiline: bool,
}

/// Any other name goes to the model as it is written: `name=value`.
const ADD: &str = "+add";

fn want(key: &str, label: &str, value: &Value, placeholder: &str) -> Want {
    Want {
        key: key.to_owned(),
        label: label.to_owned(),
        value: value_text(value),
        placeholder: placeholder.to_owned(),
        multiline: false,
    }
}

/// The fields of a node, by its kind: what its line sets.
fn wants_of(node: &GraphNode) -> Vec<Want> {
    let holds = &node.holds;
    match node.kind.as_str() {
        "model" => {
            let mut wants = vec![Want {
                multiline: true,
                ..want(
                    "prompt",
                    "Prompt",
                    &holds["prompt"],
                    "What the file should show",
                )
            }];
            wants.push(want("seed", "Seed", &holds["seed"], "the model picks one"));
            let longest = if holds["modelSeconds"].is_null() {
                "as the model makes".to_owned()
            } else {
                format!("up to {}", value_text(&holds["modelSeconds"]))
            };
            wants.push(want("seconds", "Seconds", &holds["seconds"], &longest));
            let size = holds["modelSize"]
                .as_str()
                .unwrap_or("the model's own")
                .to_owned();
            wants.push(want("size", "Size", &holds["size"], &size));
            if !holds["from"].is_null() {
                wants.push(want("from", "From", &holds["from"], "the sequence's start"));
            }
            for knob in holds["knobs"].as_array().into_iter().flatten() {
                let name = knob["name"].as_str().unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                let comes = value_text(&knob["default"]);
                wants.push(want(name, name, &knob["value"], &comes));
            }
            wants.push(want(ADD, "Add", &Value::Null, "name=value"));
            wants
        }
        "render" => vec![
            want("sequence", "Sequence", &holds["sequence"], ""),
            want("pass", "Draws", &holds["pass"], "picture"),
            want("size", "Size", &holds["size"], "the sequence's"),
            want("rate", "Rate", &holds["rate"], "the sequence's"),
            want(
                "matte",
                "Matte of",
                &holds["matte"],
                "a clip, alone in white",
            ),
            want("from", "From", &holds["from"], "its start"),
            want("seconds", "Seconds", &holds["seconds"], "to its end"),
        ],
        "join" => vec![want("lap", "Lap", &holds["lap"], "0 frames")],
        _ => Vec::new(),
    }
}

/// A field of the chosen node, and what the steps say there now.
struct Field {
    key: String,
    label: String,
    shown: String,
    input: Entity<TextInput>,
    multiline: bool,
}

/// The chosen node's fields.
struct Inspecting {
    /// The node, its line and its fields' names: the same shape keeps its fields.
    shape: String,
    fields: Vec<Field>,
    _subscriptions: Vec<Subscription>,
}

/// A drag under way on the canvas.
enum Drag {
    None,
    /// The canvas, panned from where it was.
    Canvas {
        from: Point<Pixels>,
        pan: (f32, f32),
        moved: bool,
    },
    /// A card, moved from where it was.
    Card {
        id: String,
        from: Point<Pixels>,
        place: (f32, f32),
    },
}

/// What the head's menu shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeadMenu {
    Several,
    AddStep,
}

/// Holding the takes to their guide.
#[derive(Clone, Debug, PartialEq)]
enum Holding {
    Idle,
    Under,
    Read,
    Failed(String),
}

/// A shot's graph, over the window: its steps and what they read, wired
/// left to right on a dotted canvas, a chosen node's settings at the right,
/// and its making and takes.
pub struct ShotGraph {
    session: Entity<Session>,
    media: SharedString,
    focus_handle: FocusHandle,
    recipe: Option<Recipe>,
    /// Why the graph could not be read, before any was.
    refused: Option<String>,
    /// Why the last change was refused.
    said: Option<String>,
    /// The revision the graph was last asked for at.
    asked: Option<u64>,
    /// Where the person put a card, by its node, over where it was laid out.
    placed: HashMap<String, (f32, f32)>,
    pan: (f32, f32),
    /// The zoom the graph was last drawn at, which a card dragged moves by.
    zoom: f32,
    drag: Drag,
    selected: Option<String>,
    inspecting: Option<Inspecting>,
    menu: Option<HeadMenu>,
    readings: HashMap<u32, f64>,
    best: Option<u32>,
    reading: Holding,
    _subscriptions: Vec<Subscription>,
}

impl ShotGraph {
    pub fn new(
        session: Entity<Session>,
        media: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        bind_close(cx);
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &session,
                window,
                |this, _, event: &BridgeEvent, window, cx| this.heard(&event.0, window, cx),
            ),
        ];
        let mut graph = Self {
            session,
            media,
            focus_handle: cx.focus_handle(),
            recipe: None,
            refused: None,
            said: None,
            asked: None,
            placed: HashMap::new(),
            pan: (0.0, 0.0),
            zoom: 1.0,
            drag: Drag::None,
            selected: None,
            inspecting: None,
            menu: None,
            readings: HashMap::new(),
            best: None,
            reading: Holding::Idle,
            _subscriptions: subscriptions,
        };
        graph.request(cx);
        graph
    }

    fn request(&mut self, cx: &mut Context<Self>) {
        let media = self.media.to_string();
        let revision = self.session.update(cx, |session, _| {
            session.send(json!({"type": "recipe_get", "media": media}));
            session.revision()
        });
        self.asked = Some(revision);
    }

    fn heard(&mut self, event: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let kind = event["type"].as_str().unwrap_or_default();
        let ours = event["media"].as_str() == Some(self.media.as_ref());
        let command = event["command"].as_str().unwrap_or_default();
        let message = event["message"].as_str().unwrap_or_default().to_owned();
        match kind {
            "recipe" if ours => {
                let recipe = Recipe::read(event);
                // A step just added is the one to set next.
                if let Some(line) = event["added"].as_u64() {
                    if let Some(node) = recipe
                        .nodes
                        .iter()
                        .find(|node| node.line == Some(line as usize))
                    {
                        self.selected = Some(node.id.clone());
                    }
                }
                if self
                    .selected
                    .as_deref()
                    .is_some_and(|id| recipe.node(id).is_none())
                {
                    self.selected = None;
                }
                self.recipe = Some(recipe);
                self.refused = None;
                self.inspect(window, cx);
            }
            "error" if ours && command == "recipe" => {
                if self.recipe.is_some() {
                    self.said = Some(message);
                } else {
                    self.refused = Some(message);
                }
            }
            "error" if ours && command == "takes_read" => self.reading = Holding::Failed(message),
            "take_read" if ours => {
                if let (Some(take), Some(overlap)) =
                    (event["take"].as_u64(), event["overlap"].as_f64())
                {
                    self.readings.insert(take as u32, overlap);
                }
            }
            "takes_read" if ours => {
                self.best = event["best"]
                    .as_u64()
                    .map(|best| best as u32)
                    .filter(|best| *best > 0);
                self.reading = Holding::Read;
            }
            // Its takes are other than they were.
            "media_made" | "media_make_cancelled" if ours => self.request(cx),
            "error" if ours && command == "media_make" => self.request(cx),
            // The project changed: a signal's keys, a clip's way, a take chosen.
            "state" => {
                let revision = event["snapshot"]["revision"].as_u64();
                if revision.is_some() && revision != self.asked {
                    self.request(cx);
                }
            }
            "models" | "cast" => self.request(cx),
            "connection" if self.recipe.is_none() => self.request(cx),
            _ => return,
        }
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    /// A change to the steps, as they read when the graph was drawn.
    fn edit(&mut self, mut change: Value, cx: &mut Context<Self>) {
        let Some(recipe) = &self.recipe else {
            return;
        };
        change["type"] = json!("recipe_edit");
        change["media"] = json!(self.media.as_ref());
        change["was"] = json!(recipe.text);
        self.said = None;
        self.session.update(cx, |session, _| session.send(change));
        cx.notify();
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            self.said = None;
            self.inspect(window, cx);
        }
        cx.notify();
    }

    /// The chosen node's fields: made again for another node, or another
    /// shape of the same, else set to what the steps now say, except where
    /// a person is typing.
    fn inspect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self
            .selected
            .as_ref()
            .and_then(|id| self.recipe.as_ref()?.node(id))
            .cloned();
        let Some(node) = node else {
            self.inspecting = None;
            return;
        };
        let wants = wants_of(&node);
        let line = node.line.unwrap_or(0);
        let names: Vec<&str> = wants.iter().map(|want| want.key.as_str()).collect();
        let shape = format!("{}\n{line}\n{}", node.id, names.join("\n"));
        if let Some(inspecting) = self
            .inspecting
            .as_mut()
            .filter(|inspecting| inspecting.shape == shape)
        {
            for (field, want) in inspecting.fields.iter_mut().zip(&wants) {
                let typing = field.input.read(cx).focus_handle(cx).is_focused(window);
                if field.shown != want.value && !typing && field.key != ADD {
                    field.shown = want.value.clone();
                    let value = want.value.clone();
                    field
                        .input
                        .update(cx, |input, cx| input.set_text(value, cx));
                }
            }
            return;
        }
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for want in wants {
            let input = cx.new(|cx| {
                let input = TextInput::new(cx, want.placeholder.clone());
                let mut input = if want.multiline {
                    input.multiline()
                } else {
                    input.with_style(text::TIMECODE)
                };
                input.set_text(want.value.clone(), cx);
                input
            });
            let key = want.key.clone();
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Submit => {
                            let said = input.read(cx).text().to_owned();
                            this.set_field(&key, line, said, cx);
                        }
                        // Esc leaves the field; another gives the window back.
                        InputEvent::Cancel => window.focus(&this.focus_handle),
                        InputEvent::Changed => {}
                    }
                },
            ));
            fields.push(Field {
                key: want.key,
                label: want.label,
                shown: want.value,
                input,
                multiline: want.multiline,
            });
        }
        self.inspecting = Some(Inspecting {
            shape,
            fields,
            _subscriptions: subscriptions,
        });
    }

    /// A field set with Enter: the name on its line set to what it says,
    /// or taken off the line when it says nothing.
    fn set_field(&mut self, key: &str, line: usize, said: String, cx: &mut Context<Self>) {
        if line == 0 {
            return;
        }
        let said = if key == "prompt" {
            said.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            said.trim().to_owned()
        };
        if key == ADD {
            let Some((name, value)) = said.split_once('=') else {
                return;
            };
            let (name, value) = (name.trim().to_owned(), value.trim().to_owned());
            if name.is_empty() || value.is_empty() {
                return;
            }
            if let Some(field) = self
                .inspecting
                .as_ref()
                .and_then(|inspecting| inspecting.fields.iter().find(|field| field.key == ADD))
            {
                field.input.update(cx, |input, cx| input.clear(cx));
            }
            self.edit(
                json!({"line": line, "key": name, "nth": 0, "value": value}),
                cx,
            );
            return;
        }
        let Some(field) = self
            .inspecting
            .as_mut()
            .and_then(|inspecting| inspecting.fields.iter_mut().find(|field| field.key == key))
        else {
            return;
        };
        if field.shown == said {
            return;
        }
        field.shown = said.clone();
        // The picture is what a guide draws when its line names no pass.
        if said.is_empty() || (key == "pass" && said == "picture") {
            self.edit(json!({"line": line, "key": key, "nth": 0}), cx);
        } else {
            self.edit(
                json!({"line": line, "key": key, "nth": 0, "value": said}),
                cx,
            );
        }
    }

    fn press_canvas(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.menu = None;
        self.drag = Drag::Canvas {
            from: event.position,
            pan: self.pan,
            moved: false,
        };
        cx.notify();
    }

    fn press_card(
        &mut self,
        id: &str,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let place = self.place_of(id).unwrap_or((0.0, 0.0));
        self.drag = Drag::Card {
            id: id.to_owned(),
            from: at,
            place,
        };
        window.focus(&self.focus_handle);
        self.select(Some(id.to_owned()), window, cx);
    }

    /// Where a card is: where the person put it, else where it is laid out.
    fn place_of(&self, id: &str) -> Option<(f32, f32)> {
        if let Some(placed) = self.placed.get(id) {
            return Some(*placed);
        }
        arrange(self.recipe.as_ref()?).get(id).copied()
    }

    fn on_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.drag, Drag::None) {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = Drag::None;
            cx.notify();
            return;
        }
        match &mut self.drag {
            Drag::None => {}
            Drag::Canvas { from, pan, moved } => {
                let (dx, dy) = (
                    f32::from(event.position.x - from.x),
                    f32::from(event.position.y - from.y),
                );
                if dx.abs() + dy.abs() > 2.0 {
                    *moved = true;
                }
                self.pan = (pan.0 + dx, pan.1 + dy);
            }
            Drag::Card { id, from, place } => {
                let zoom = self.zoom.max(0.01);
                let (dx, dy) = (
                    f32::from(event.position.x - from.x),
                    f32::from(event.position.y - from.y),
                );
                self.placed.insert(
                    id.clone(),
                    ((place.0 + dx / zoom).round(), (place.1 + dy / zoom).round()),
                );
            }
        }
        cx.notify();
    }

    fn on_up(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A press on the canvas that did not pan it chooses nothing: the shot itself.
        if let Drag::Canvas { moved: false, .. } = self.drag {
            self.drag = Drag::None;
            self.select(None, window, cx);
            return;
        }
        if !matches!(self.drag, Drag::None) {
            self.drag = Drag::None;
            cx.notify();
        }
    }

    fn making<'a>(&self, session: &'a Session) -> Option<&'a Making> {
        session.state.makings.get(self.media.as_ref())
    }

    /// How the shot stands, the most pressing thing first.
    fn status(&self, making: Option<&Making>) -> (String, Rgba) {
        if let Some(making) = making {
            let said = if making.line.is_empty() {
                format!("Making take {}…", making.take)
            } else {
                format!("Making take {}: {}", making.take, making.line)
            };
            return (said, theme::activity::gpu());
        }
        let Some(recipe) = &self.recipe else {
            return match &self.refused {
                Some(_) => ("Cannot be shown".into(), theme::neg()),
                None => ("Reading its steps…".into(), theme::ink3()),
            };
        };
        if recipe.error.is_some() {
            let said = if recipe.document {
                "The steps do not read"
            } else {
                "The recipe does not read"
            };
            return (said.into(), theme::neg());
        }
        if recipe.cannot.is_some() {
            return ("Cannot be made as things stand".into(), theme::neg());
        }
        let takes = recipe.node("takes");
        if let Some(wants) = takes.and_then(|takes| takes.holds["wants"].as_str()) {
            return (format!("Wants making: {wants}"), theme::warn());
        }
        (
            takes.map(|takes| takes.detail.clone()).unwrap_or_default(),
            theme::ink3(),
        )
    }

    fn head(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let making = self.making(session).cloned();
        let connected = session.state.connection == crate::store::Connection::Connected;
        let (status, status_color) = self.status(making.as_ref());
        let name = self
            .recipe
            .as_ref()
            .map(|recipe| recipe.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.media.to_string());
        let path = match &self.recipe {
            Some(recipe) if recipe.document => "steps in the document".to_owned(),
            Some(recipe) => recipe.recipe.clone(),
            None => String::new(),
        };
        let reads = self
            .recipe
            .as_ref()
            .is_some_and(|recipe| recipe.error.is_none() && !recipe.nodes.is_empty());
        let has_takes = self
            .recipe
            .as_ref()
            .and_then(|recipe| recipe.node("takes"))
            .and_then(|takes| takes.holds["takes"].as_array())
            .is_some_and(|takes| !takes.is_empty());
        let can = connected && reads;
        let media = self.media.clone();
        let mut head = div()
            .h(px(HEAD))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(16.0))
            .pr(px(12.0))
            .border_b_1()
            .border_color(theme::line())
            .child(Icon::Media.view(16.0, theme::ink2()))
            .child(
                ui::line(name, text::TITLE, theme::ink())
                    .flex_none()
                    .max_w(px(240.0)),
            )
            .when(!path.is_empty(), |head| {
                head.child(ui::line(path, text::MONO_SMALL, theme::ink3()).flex_none())
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_end()
                    .child(ui::line(status, text::CONTROL, status_color)),
            );
        if making.is_some() {
            head = head.child(ui::button("graph-stop", "Stop").icon(Icon::Stop).on_click(
                cx.listener(move |this, _, _, cx| {
                    let media = this.media.to_string();
                    this.session.update(cx, |session, _| {
                        session.send(json!({"type": "media_make_cancel", "media": media}));
                    });
                }),
            ));
        } else {
            head = head
                .child(
                    ui::button("graph-add-step", "Add step")
                        .ghost()
                        .icon(Icon::Plus)
                        .disabled(!can)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu = if this.menu == Some(HeadMenu::AddStep) {
                                None
                            } else {
                                Some(HeadMenu::AddStep)
                            };
                            cx.notify();
                        })),
                )
                .child(
                    ui::button("graph-several", "Several")
                        .ghost()
                        .icon(Icon::ChevronDown)
                        .disabled(!can)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu = if this.menu == Some(HeadMenu::Several) {
                                None
                            } else {
                                Some(HeadMenu::Several)
                            };
                            cx.notify();
                        })),
                )
                .child(
                    ui::button("graph-make", if has_takes { "Make again" } else { "Make" })
                        .primary()
                        .disabled(!can)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(MakeMedia {
                                    media: media.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            )
                        }),
                );
        }
        head.child(
            icon_button("graph-close", Icon::Close)
                .tooltip("Close")
                .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
        )
        .into_any_element()
    }

    fn head_menu(&self, menu: HeadMenu, media: SharedString, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: String, words: String| {
            theme::type_style(div(), text::MENU_ITEM)
                .id(ElementId::Name(id.into()))
                .flex()
                .items_center()
                .h(px(30.0))
                .px(px(10.0))
                .rounded(px(radius::CONTROL))
                .cursor_pointer()
                .text_color(theme::ink())
                .hover(|item| item.bg(theme::popup_hover()))
                .child(words)
        };
        let (heading, right, items): (&str, f32, Vec<AnyElement>) = match menu {
            HeadMenu::Several => (
                "Each seed one on from the last",
                132.0,
                [2u32, 4, 8]
                    .into_iter()
                    .map(|rolls| {
                        let media = media.clone();
                        item(format!("graph-make-{rolls}"), format!("{rolls} Takes…"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.menu = None;
                                window.dispatch_action(
                                    Box::new(MakeMedia {
                                        media: media.clone(),
                                        rolls,
                                    }),
                                    cx,
                                );
                            }))
                            .into_any_element()
                    })
                    .collect(),
            ),
            HeadMenu::AddStep => (
                "A new step of the shot",
                220.0,
                [
                    ("guide", "Guide, drawn from a sequence"),
                    ("model", "Model, asked for a file"),
                    ("join", "Join, parts set end to end"),
                ]
                .into_iter()
                .map(|(kind, words)| {
                    item(format!("graph-add-{kind}"), words.to_owned())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.menu = None;
                            this.edit(json!({"add": kind}), cx);
                        }))
                        .into_any_element()
                })
                .collect(),
            ),
        };
        div()
            .absolute()
            .top(px(HEAD - 4.0))
            .right(px(right))
            .w(px(232.0))
            .p(px(4.0))
            .rounded(px(radius::CARD))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::line2())
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }))
            .child(div().px(px(10.0)).py(px(4.0)).child(ui::label(
                heading,
                text::META,
                theme::ink3(),
            )))
            .children(items)
            .with_animation("graph-menu", motion::animation(scene::FADE), |menu, t| {
                menu.opacity(t)
            })
            .into_any_element()
    }

    /// The canvas: its dots, the edges and the cards, panned as a whole.
    fn canvas(
        &mut self,
        room_size: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let room = div()
            .id("graph-canvas")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(theme::well())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.press_canvas(event, cx)),
            )
            // A wheel or a trackpad looks around it, as a drag does.
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(20.0));
                this.pan = (
                    this.pan.0 + f32::from(delta.x),
                    this.pan.1 + f32::from(delta.y),
                );
                cx.notify();
            }));
        let Some(recipe) = self.recipe.clone() else {
            let said = match &self.refused {
                Some(why) => ui::label(why.clone(), text::BODY, theme::neg()),
                None => ui::label(
                    format!("Reading how {} is made…", self.media),
                    text::BODY,
                    theme::ink3(),
                ),
            };
            let mut column = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.0))
                .max_w(px(480.0))
                .child(said);
            if self
                .refused
                .as_deref()
                .is_some_and(|why| why.contains("recipe file") || why.contains("kerf carry"))
            {
                let media = self.media.clone();
                column = column.child(
                    ui::button("graph-carry", "Carry Steps into the Document…").on_click(
                        move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(MakeMedia {
                                    media: media.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            )
                        },
                    ),
                );
            }
            return room
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .child(column)
                .into_any_element();
        };
        if recipe.nodes.is_empty() {
            let why = recipe
                .error
                .clone()
                .unwrap_or_else(|| "There are no steps to show.".into());
            let said = if recipe.document {
                format!("These steps do not read: {why}")
            } else {
                why
            };
            return room
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .child(ui::label(said, text::BODY, theme::neg()).max_w(px(480.0)))
                .into_any_element();
        }
        let laid = arrange(&recipe);
        let inputs = |id: &str| recipe.edges.iter().filter(|edge| edge.to == id).count();
        // Fitted in the middle as laid out, so that a card dragged moves nothing else.
        let Fit { zoom, offset } = fit(&recipe, &laid, room_size, self.pan);
        self.zoom = zoom;
        let place = |id: &str| {
            self.placed
                .get(id)
                .or_else(|| laid.get(id))
                .map(|(x, y)| (offset.0 + x * zoom, offset.1 + y * zoom))
        };
        let height = |id: &str| {
            zoom * recipe
                .node(id)
                .map_or(CARD_H, |node| card_height(node, inputs(id)))
        };
        // Each edge leaves level and arrives level, turning in the room between.
        let edges: Vec<((f32, f32), (f32, f32), Rgba)> = recipe
            .edges
            .iter()
            .filter_map(|edge| {
                let from = place(&edge.from)?;
                let to = place(&edge.to)?;
                let into = recipe.reads_into(&edge.to);
                let at = into
                    .iter()
                    .position(|other| {
                        other.from == edge.from && other.port == edge.port && other.nth == edge.nth
                    })
                    .unwrap_or(0);
                let start = (from.0 + CARD_W * zoom, from.1 + height(&edge.from) / 2.0);
                let end = (
                    to.0,
                    to.1 + height(&edge.to) * (at as f32 + 0.5) / into.len().max(1) as f32,
                );
                Some((start, end, gives_color(&edge.gives)))
            })
            .collect();
        let grid = GRID * zoom;
        let dots = (offset.0.rem_euclid(grid), offset.1.rem_euclid(grid));
        let session = self.session.read(cx);
        let making = self.making(session).cloned();
        if making.is_some() && !motion::reduced() {
            window.request_animation_frame();
        }
        let cards: Vec<AnyElement> = recipe
            .nodes
            .iter()
            .filter_map(|node| {
                let at = place(&node.id)?;
                Some(self.card(node, at, height(&node.id), zoom, making.as_ref(), cx))
            })
            .collect();
        let mut shown: Vec<&str> = vec!["video", "image", "path", "signal"];
        for more in ["words", "file"] {
            if recipe.edges.iter().any(|edge| edge.gives == more) {
                shown.push(more);
            }
        }
        let legend = div()
            .absolute()
            .left(px(12.0))
            .bottom(px(10.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(10.0))
            .h(px(24.0))
            .rounded(px(radius::CONTROL))
            .bg(theme::with_alpha(theme::chrome(), 0.9))
            .border_1()
            .border_color(theme::line())
            .children(shown.into_iter().map(|gives| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(2.0))
                            .rounded_full()
                            .bg(gives_color(gives)),
                    )
                    .child(ui::label(gives, text::META, theme::ink3()))
            }));
        room.child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let origin = bounds.origin;
                    let dot = theme::with_alpha(theme::ink3(), 0.22);
                    let (width, height) =
                        (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    let mut y = dots.1;
                    while y < height {
                        let mut x = dots.0;
                        while x < width {
                            window.paint_quad(fill(
                                Bounds::new(
                                    origin + point(px(x - 0.75), px(y - 0.75)),
                                    size(px(1.5), px(1.5)),
                                ),
                                dot,
                            ));
                            x += grid;
                        }
                        y += grid;
                    }
                    for (start, end, color) in &edges {
                        let reach = (48.0 * zoom).max((end.0 - start.0).abs() * 0.6);
                        let at = |(x, y): (f32, f32)| origin + point(px(x), px(y));
                        let mut path = PathBuilder::stroke(px(1.5));
                        path.move_to(at(*start));
                        path.cubic_bezier_to(
                            at(*end),
                            at((start.0 + reach, start.1)),
                            at((end.0 - reach, end.1)),
                        );
                        if let Ok(path) = path.build() {
                            window.paint_path(path, *color);
                        }
                        for (x, y) in [*start, *end] {
                            window.paint_quad(
                                fill(
                                    Bounds::new(at((x - 3.0, y - 3.0)), size(px(6.0), px(6.0))),
                                    *color,
                                )
                                .corner_radii(px(3.0)),
                            );
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .children(cards)
        .child(legend)
        .into_any_element()
    }

    fn card(
        &self,
        node: &GraphNode,
        at: (f32, f32),
        height: f32,
        zoom: f32,
        making: Option<&Making>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let z = |points: f32| px(points * zoom);
        let color = gives_color(&node.gives);
        let selected = self.selected.as_deref() == Some(node.id.as_str());
        let id = node.id.clone();
        let striped = making.filter(|_| node.kind == "takes");
        let detail = match striped {
            Some(making) => format!("Making take {} · {}", making.take, clock(making.since)),
            None if node.kind == "model" && node.detail.trim().is_empty() => "no prompt".to_owned(),
            None => node.detail.split_whitespace().collect::<Vec<_>>().join(" "),
        };
        let mut lines = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(z(6.0))
                    .h(z(20.0))
                    .child(
                        zoomed(
                            ui::line(node.title.clone(), text::BODY_NAME, theme::ink()),
                            text::BODY_NAME,
                            zoom,
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .when_some(node_mark(node), |row, mark| {
                        row.child(ui::dot(mark, 6.0 * zoom))
                    })
                    .child(
                        div()
                            .flex_none()
                            .px(z(6.0))
                            .h(z(16.0))
                            .flex()
                            .items_center()
                            .rounded(z(8.0))
                            .bg(theme::with_alpha(color, 0.14))
                            .child(zoomed(
                                ui::label(node.gives.clone(), text::BADGE, color),
                                text::BADGE,
                                zoom,
                            )),
                    ),
            )
            // Over the stripes it is said in ink: amber on amber is not read.
            .child(zoomed(
                ui::line(
                    detail,
                    text::META,
                    if striped.is_some() {
                        theme::ink()
                    } else {
                        theme::ink3()
                    },
                ),
                text::META,
                zoom,
            ));
        if node.kind == "model" {
            lines = lines.child(zoomed(
                ui::line(model_line(node), text::MONO_SMALL, theme::ink3()),
                text::MONO_SMALL,
                zoom,
            ));
        }
        let phase = striped.map(|making| motion::phase(making.since, scene::STRIPE_CYCLE));
        div()
            .id(ElementId::Name(format!("node-{}", node.id).into()))
            .absolute()
            .left(px(at.0))
            .top(px(at.1))
            .w(z(CARD_W))
            .h(px(height))
            .rounded(z(radius::PANEL))
            .overflow_hidden()
            .bg(theme::card())
            .border_1()
            .border_color(if selected {
                theme::accent()
            } else {
                theme::line2()
            })
            .when(!selected, |card| {
                card.hover(|card| card.border_color(theme::lineh()))
            })
            .cursor_pointer()
            .when_some(phase, |card, phase| {
                card.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            crate::media::paint_stripes(window, bounds, phase)
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .gap(z(10.0))
                    .p(z(10.0))
                    .child(
                        div()
                            .flex_none()
                            .size(z(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(z(radius::CARD))
                            .bg(theme::with_alpha(color, 0.16))
                            .child(kind_icon(&node.kind).view(16.0 * zoom, color)),
                    )
                    .child(lines),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.press_card(&id, event.position, window, cx);
                }),
            )
            .into_any_element()
    }

    /// The column at the right: the chosen node and its settings, or the shot.
    fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div().flex().flex_col().gap(px(8.0)).p(px(14.0));
        let session = self.session.read(cx);
        let making = self.making(session).is_some();
        let connected = session.state.connection == crate::store::Connection::Connected;
        let node = self
            .selected
            .as_ref()
            .and_then(|id| self.recipe.as_ref()?.node(id))
            .cloned();
        if let Some(said) = &self.said {
            column = column.child(ui::label(said.clone(), text::CONTROL, theme::neg()));
        }
        match (&self.recipe, node) {
            (None, _) => {
                column = column.child(ui::label(self.media.to_string(), text::TITLE, theme::ink()));
            }
            (Some(recipe), None) => {
                column = column
                    .child(ui::label(recipe.name.clone(), text::TITLE, theme::ink()))
                    .child(ui::label(
                        if recipe.document {
                            "steps in the document".to_owned()
                        } else {
                            recipe.recipe.clone()
                        },
                        text::MONO_SMALL,
                        theme::ink3(),
                    ));
                if let Some(error) = &recipe.error {
                    let said = if recipe.document {
                        "These steps do not read"
                    } else {
                        "This recipe does not read"
                    };
                    column = column.child(ui::label(
                        format!("{said}: {error}"),
                        text::CONTROL,
                        theme::neg(),
                    ));
                }
                if let Some(cannot) = &recipe.cannot {
                    column = column.child(ui::label(
                        format!("It cannot be made as things stand: {cannot}"),
                        text::CONTROL,
                        theme::neg(),
                    ));
                }
                if let Some(error) = &recipe.models_error {
                    column = column.child(ui::label(error.clone(), text::CONTROL, theme::neg()));
                }
                if let Some(notes) = notes_block("Before it is made", &recipe.notes) {
                    column = column.child(notes);
                }
                if recipe.error.is_none() {
                    column = column.child(ui::label(
                        "Choose a node to see and set what it holds. A change is written into the shot's steps in the document, as an edit that is undone as any other; Make then writes a new take. Drag the canvas to look around it, and a card to move it.",
                        text::CONTROL,
                        theme::ink2(),
                    ));
                }
                if let Some(takes) = recipe.node("takes") {
                    column = column.child(self.takes(takes, making, connected, cx));
                }
            }
            (Some(recipe), Some(node)) => {
                let color = gives_color(&node.gives);
                column = column.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(kind_icon(&node.kind).view(16.0, color))
                        .child(
                            ui::line(node.title.clone(), text::TITLE, theme::ink())
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(ui::label(
                            node.gives.clone(),
                            text::MONO_SMALL,
                            theme::ink3(),
                        )),
                );
                column = column.child(self.about(&node));
                if let Some(inspecting) = &self.inspecting {
                    if !inspecting.fields.is_empty() {
                        column = column.child(ui::label(
                            "Enter sets a field; emptied, it is taken off the line.",
                            text::META,
                            theme::ink3(),
                        ));
                    }
                    for field in &inspecting.fields {
                        if field.key == ADD {
                            column = column.child(section("More"));
                        }
                        column = column.child(field_row(field));
                    }
                }
                if node.kind == "takes" {
                    column = column.child(self.takes(&node, making, connected, cx));
                }
                column = column.child(self.reads(recipe, &node, cx));
                if let Some(line) = node.line {
                    column = column.child(
                        div().flex().pt(px(8.0)).child(
                            ui::button("graph-remove-step", "Take this step out")
                                .ghost()
                                .icon(Icon::Trash)
                                .disabled(making)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit(json!({"line": line, "remove": true}), cx);
                                    this.select(None, window, cx);
                                })),
                        ),
                    );
                }
            }
        }
        div()
            .id("graph-inspector")
            .w(px(INSPECTOR))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .bg(theme::chrome())
            .border_l_1()
            .border_color(theme::line())
            .child(column)
            .into_any_element()
    }

    /// What a node is, by its kind, beside its fields.
    fn about(&self, node: &GraphNode) -> gpui::Div {
        let holds = &node.holds;
        let note = |words: String, color: Rgba| ui::label(words, text::CONTROL, color);
        let missing = holds["there"] == Value::Bool(false);
        let mut about = div().flex().flex_col().gap(px(6.0));
        match node.kind.as_str() {
            "model" => {
                if let Some(said) = holds["about"].as_str() {
                    about = about.child(note(said.to_owned(), theme::ink2()));
                }
                if holds["setUp"] == Value::Bool(false) {
                    about = about.child(note(
                        format!(
                            "No model named {} is set up on this computer. A model is a program your own models file names.",
                            holds["model"].as_str().unwrap_or_default()
                        ),
                        theme::warn(),
                    ));
                }
                if holds["runner"]["kind"] == "comfy" {
                    let workflow = holds["workflow"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            format!(
                                "{}, as it comes with kerf-comfy",
                                value_text(&holds["runner"]["workflow"])
                            )
                        });
                    about = about.child(note(
                        format!("Run with the ComfyUI workflow {workflow}."),
                        theme::ink3(),
                    ));
                }
            }
            "render" => {
                about = about.child(note(
                    "Kerf draws a sequence of this project into a file a model follows, with no sound.".into(),
                    theme::ink2(),
                ));
            }
            "join" => {
                about = about.child(note(
                    "Kerf sets the parts it reads end to end into one file: a shot longer than a model makes at once, made a part at a time. Lap is how many frames each part after the first shares with the one before it.".into(),
                    theme::ink2(),
                ));
            }
            "sequence" => {
                about = about
                    .child(note(node.detail.clone(), if missing { theme::neg() } else { theme::ink2() }))
                    .child(note(
                        "A sequence of this project, which the guide is drawn from. Edit it on the timeline.".into(),
                        theme::ink3(),
                    ));
            }
            "file" => {
                about = about.child(ui::label(
                    value_text(&holds["path"]),
                    text::TIMECODE,
                    theme::ink2(),
                ));
                if missing {
                    about = about.child(note(
                        "This file is not there. The steps cannot run until it is.".into(),
                        theme::neg(),
                    ));
                }
            }
            "signal" => {
                about = about.child(note(
                    node.detail.clone(),
                    if missing { theme::neg() } else { theme::ink2() },
                ));
                if !missing {
                    about = about.child(note(
                        "A number from 0 to 1 over the shot's time. The prompt says what it is of; this says when, and how much. It is the clip's value in the project: set it on the timeline.".into(),
                        theme::ink3(),
                    ));
                }
            }
            "moves" => {
                let said = if missing {
                    node.detail.clone()
                } else {
                    format!(
                        "The model is handed where this clip is on each frame of {}. Move the clip, or its path on the picture, to change it.",
                        holds["sequence"].as_str().unwrap_or("its sequence")
                    )
                };
                about = about.child(note(
                    said,
                    if missing { theme::neg() } else { theme::ink2() },
                ));
            }
            "cast" | "style" => {
                about = about.child(note(
                    node.detail.clone(),
                    if missing { theme::neg() } else { theme::ink2() },
                ));
                let refs = strings(&holds["refs"]);
                if !refs.is_empty() {
                    about =
                        about.child(ui::label(refs.join("\n"), text::MONO_SMALL, theme::ink3()));
                }
            }
            "takes" => {}
            _ => {
                if !node.detail.is_empty() {
                    about = about.child(note(node.detail.clone(), theme::ink2()));
                }
            }
        }
        about
    }

    /// What reads into a node, a row each; a model's can be let go of.
    fn reads(&self, recipe: &Recipe, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let into = recipe.reads_into(&node.id);
        let mut reads = div().flex().flex_col().gap(px(4.0));
        if into.is_empty() {
            return reads;
        }
        reads = reads.child(section("Reads"));
        let line = node.line.unwrap_or(0);
        for (index, edge) in into.into_iter().enumerate() {
            let from = recipe
                .node(&edge.from)
                .map_or(edge.from.clone(), |from| from.title.clone());
            let (port, nth) = (edge.port.clone(), edge.nth);
            reads = reads.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(24.0))
                    .child(ui::dot(gives_color(&edge.gives), 6.0))
                    .child(
                        ui::line(edge.port.clone(), text::TIMECODE, theme::ink2())
                            .w(px(FIELD_LABEL - 14.0))
                            .flex_none(),
                    )
                    .child(
                        ui::line(from, text::CONTROL, theme::ink())
                            .flex_1()
                            .min_w_0(),
                    )
                    .when(node.kind == "model" && line > 0, |row| {
                        row.child(
                            icon_button(
                                ElementId::Name(format!("let-go-{index}").into()),
                                Icon::Close,
                            )
                            .size(20.0)
                            .tooltip(format!("Take {port} off the line"))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.edit(json!({"line": line, "key": port, "nth": nth}), cx);
                                },
                            )),
                        )
                    }),
            );
        }
        reads
    }

    /// The shot's takes: each kept, the one the shot is ticked, another
    /// chosen with a press; held to their guide on asking.
    fn takes(
        &self,
        node: &GraphNode,
        making: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let holds = &node.holds;
        let mut column = div().flex().flex_col().gap(px(6.0));
        if let Some(wants) = holds["wants"].as_str() {
            column = column.child(ui::label(
                format!("Wants making: {wants}."),
                text::CONTROL,
                theme::warn(),
            ));
        }
        let takes = holds["takes"].as_array().cloned().unwrap_or_default();
        column = column.child(section("Takes"));
        if takes.is_empty() {
            return column.child(ui::label(
                "No take yet. Make runs its steps and writes the first; every take after it is kept beside it.",
                text::CONTROL,
                theme::ink2(),
            ));
        }
        for take in &takes {
            let number_of = number(&take["number"]);
            let chosen = take["chosen"].as_bool().unwrap_or(false);
            let path = take["path"].as_str().unwrap_or_default().to_owned();
            let mut says = Vec::new();
            if let Some(overlap) = self.readings.get(&number_of) {
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
            let can = !making && !chosen && connected;
            let row = div()
                .id(ElementId::Name(format!("take-{number_of}").into()))
                .flex()
                .items_center()
                .gap(px(8.0))
                .min_h(px(32.0))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(radius::CONTROL))
                .border_1()
                .border_color(if chosen {
                    theme::accent()
                } else {
                    theme::line()
                })
                .when(chosen, |row| row.bg(theme::accent_wash()))
                .child(if chosen {
                    Icon::Check.view(14.0, theme::accent_ink())
                } else {
                    Icon::Film.view(14.0, theme::ink3())
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(ui::label(
                            format!("Take {number_of}"),
                            text::CONTROL_STRONG,
                            theme::ink(),
                        ))
                        .when(!says.is_empty(), |words| {
                            words.child(ui::line(says.join(" · "), text::META, theme::ink3()))
                        }),
                )
                .when(chosen, |row| {
                    row.child(ui::label("the shot", text::META, theme::accent_ink()))
                });
            let row = if can {
                let media = self.media.to_string();
                row.cursor_pointer()
                    .hover(|row| row.bg(theme::raised()))
                    .tooltip(move |_, cx| {
                        cx.new(|_| {
                            ui::Tooltip(
                                format!("Have the shot be take {number_of} again; its clips keep their timing, and it is undone as any edit")
                                    .into(),
                            )
                        })
                        .into()
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // A take is a file of the project already: the media's line names it.
                        let command = json!({"type": "media_replace", "media": media, "path": path});
                        this.session.update(cx, |session, _| session.send(command));
                    }))
            } else {
                row
            };
            column = column.child(row);
        }
        // The take the shot is, made from steps that have changed since: back to them.
        for take in &takes {
            let kept = take["chosen"].as_bool().unwrap_or(false)
                && take["kept"].as_bool().unwrap_or(false);
            if !kept || strings(&take["changed"]).is_empty() {
                continue;
            }
            let restore = number(&take["number"]);
            column = column.child(
                div().flex().child(
                    ui::button("graph-restore", format!("Restore take {restore}'s steps"))
                        .disabled(making)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit(json!({"restore": restore}), cx)
                        })),
                ),
            );
        }
        let reading = self.reading == Holding::Under;
        let hold_words = if reading {
            "Reading…"
        } else if self.readings.is_empty() {
            "Hold to guide"
        } else {
            "Hold to guide again"
        };
        column = column.child(
            div().flex().pt(px(4.0)).child(
                ui::button("graph-hold", hold_words)
                    .disabled(reading || !connected)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let media = this.media.to_string();
                        this.session.update(cx, |session, _| {
                            session.send(json!({"type": "takes_read", "media": media}))
                        });
                        this.readings.clear();
                        this.best = None;
                        this.reading = Holding::Under;
                        cx.notify();
                    })),
            ),
        );
        let said = match &self.reading {
            Holding::Idle => None,
            Holding::Under => Some((
                "Drawing the guide and following the subject through each take. A take is a few seconds.".to_owned(),
                theme::ink3(),
            )),
            Holding::Read => Some((
                "1 is a take that has the subject where and as large as its guide asked on every frame, 0 one that has none of it.".to_owned(),
                theme::ink3(),
            )),
            Holding::Failed(why) => Some((why.clone(), theme::neg())),
        };
        if let Some((said, color)) = said {
            column = column.child(ui::label(said, text::CONTROL, color));
        }
        column.child(ui::label(
            "Every take is kept. Make again writes the next beside these.",
            text::META,
            theme::ink3(),
        ))
    }
}

/// A heading over a part of the column.
fn section(words: &str) -> gpui::Div {
    div().pt(px(6.0)).child(ui::label(
        words.to_owned(),
        text::META_STRONG,
        theme::ink3(),
    ))
}

/// A field's name and its box.
fn field_row(field: &Field) -> gpui::Div {
    let input = div()
        .flex_1()
        .min_w_0()
        .px(px(8.0))
        .py(px(5.0))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line2())
        .when(field.multiline, |input| input.min_h(px(72.0)))
        .child(field.input.clone());
    if field.multiline {
        return div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(ui::label(field.label.clone(), text::CONTROL, theme::ink2()))
            .child(input);
    }
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            ui::line(field.label.clone(), text::CONTROL, theme::ink2())
                .w(px(FIELD_LABEL))
                .flex_none(),
        )
        .child(input)
}

impl EventEmitter<DismissEvent> for ShotGraph {}

impl Focusable for ShotGraph {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShotGraph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (width, height) = graph_size((f32::from(viewport.width), f32::from(viewport.height)));
        let head = self.head(cx);
        let canvas = self.canvas((width - INSPECTOR - 1.0, height - HEAD - 1.0), window, cx);
        let inspector = self.inspector(cx);
        // The head's menu is drawn last, over the canvas and the column under it.
        let menu = self
            .menu
            .map(|menu| self.head_menu(menu, self.media.clone(), cx));
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _, cx| this.dismiss(cx)))
            // A press on the dim around the panel gives the window back.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss(cx)))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_up))
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .bg(theme::chrome())
            .font(theme::ui_font(gpui::FontWeight::NORMAL))
            .text_color(theme::ink())
            .relative()
            .child(head)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(canvas)
                    .child(inspector),
            )
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: kind.into(),
            ..GraphNode::default()
        }
    }

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
            port: "guide".into(),
            ..GraphEdge::default()
        }
    }

    /// The bowling shot: a sequence drawn into a guide, a signal and a clip's
    /// way read by the model with it, and the model into the takes.
    fn bowling() -> Recipe {
        Recipe {
            nodes: vec![
                node("sequence:bowling", "sequence"),
                node("made:guide.mp4", "render"),
                node("signal:strike", "signal"),
                node("moves:ball", "moves"),
                node("made:{out}", "model"),
                node("takes", "takes"),
            ],
            edges: vec![
                edge("sequence:bowling", "made:guide.mp4"),
                edge("made:guide.mp4", "made:{out}"),
                edge("signal:strike", "made:{out}"),
                edge("moves:ball", "made:{out}"),
                edge("made:{out}", "takes"),
            ],
            ..Recipe::default()
        }
    }

    #[test]
    fn a_graph_is_laid_out_left_to_right_by_what_each_reads() {
        let recipe = bowling();
        let places = arrange(&recipe);
        assert_eq!(places.len(), recipe.nodes.len());
        let column = |id: &str| (places[id].0 / (CARD_W + COLUMN_GAP)).round() as i32;
        assert_eq!(column("sequence:bowling"), 0);
        assert_eq!(column("made:guide.mp4"), 1);
        assert_eq!(column("made:{out}"), 2);
        assert_eq!(column("takes"), 3);
        // What nothing comes into stands beside the first thing that reads it.
        assert_eq!(column("signal:strike"), 1);
        assert_eq!(column("moves:ball"), 1);
        // Every edge goes left to right, and no two cards of a column overlap.
        for edge in &recipe.edges {
            assert!(places[&edge.from].0 < places[&edge.to].0, "{edge:?}");
        }
        let mut second: Vec<f32> = ["made:guide.mp4", "signal:strike", "moves:ball"]
            .iter()
            .map(|id| places[*id].1)
            .collect();
        second.sort_by(f32::total_cmp);
        for pair in second.windows(2) {
            assert!(pair[1] - pair[0] >= CARD_H + ROW_GAP - 0.5, "{second:?}");
        }
        // In the column, in the order the model reads them.
        assert!(places["made:guide.mp4"].1 < places["signal:strike"].1);
        assert!(places["signal:strike"].1 < places["moves:ball"].1);
    }

    #[test]
    fn a_graph_is_fitted_in_the_middle_of_its_canvas_at_no_more_than_its_size() {
        let recipe = bowling();
        let laid = arrange(&recipe);
        // Room enough: as laid out, in the middle.
        let roomy = fit(&recipe, &laid, (2000.0, 1200.0), (0.0, 0.0));
        assert_eq!(roomy.zoom, 1.0);
        let left = laid.values().map(|place| place.0).fold(f32::MAX, f32::min);
        let right = laid.values().map(|place| place.0).fold(f32::MIN, f32::max) + CARD_W;
        let drawn = (roomy.offset.0 + left, roomy.offset.0 + right);
        assert!((drawn.0 - (2000.0 - drawn.1)).abs() <= 1.0, "{drawn:?}");
        // A narrow canvas draws it smaller, so all of it shows, down to the least.
        let narrow = fit(&recipe, &laid, (900.0, 560.0), (0.0, 0.0));
        assert!(narrow.zoom < 1.0 && narrow.zoom >= LEAST_ZOOM, "{narrow:?}");
        assert!(narrow.offset.0 + left * narrow.zoom >= FIT_AIR - 1.0);
        assert!(narrow.offset.0 + right * narrow.zoom <= 900.0 - FIT_AIR + 1.0);
        assert_eq!(
            fit(&recipe, &laid, (200.0, 200.0), (0.0, 0.0)).zoom,
            LEAST_ZOOM
        );
        // Panned, all of it moves by as much.
        let panned = fit(&recipe, &laid, (900.0, 560.0), (30.0, -12.0));
        assert_eq!(
            (
                panned.offset.0 - narrow.offset.0,
                panned.offset.1 - narrow.offset.1
            ),
            (30.0, -12.0)
        );
        // The panel takes the window's room, between the least and the most.
        assert_eq!(graph_size((1600.0, 1000.0)), (1400.0, 900.0));
        assert_eq!(graph_size((1280.0, 800.0)), (1160.0, 700.0));
        assert_eq!(graph_size((900.0, 600.0)), (868.0, 568.0));
    }

    #[test]
    fn a_loop_does_not_keep_the_layout_from_ending() {
        let recipe = Recipe {
            nodes: vec![node("a", "render"), node("b", "model")],
            edges: vec![edge("a", "b"), edge("b", "a"), edge("a", "missing")],
            ..Recipe::default()
        };
        assert_eq!(arrange(&recipe).len(), 2);
        assert!(arrange(&Recipe::default()).is_empty());
    }

    #[test]
    fn a_recipe_event_reads_leniently() {
        let event = json!({
            "type": "recipe", "media": "m1", "recipe": "", "document": true, "name": "m1",
            "text": "kerf-recipe 1\ngenerate model=dawn prompt=\"a boat\" seed=7\n",
            "nodes": [
                {"id": "made:{out}", "kind": "model", "title": "dawn", "detail": "a boat", "gives": "video",
                 "line": 2, "model": "dawn", "setUp": false, "seed": 7, "knobs": [{"name": "steps", "value": null, "default": 30}]},
                {"id": "takes", "kind": "takes", "title": "m1", "detail": "no take yet", "gives": "video",
                 "takes": [], "wants": null},
                {"kind": "file"}
            ],
            "edges": [{"from": "made:{out}", "to": "takes", "port": "take", "nth": 0, "gives": "video"}],
            "cannot": "no model `dawn` is set up", "notes": []
        });
        let recipe = Recipe::read(&event);
        assert_eq!(recipe.nodes.len(), 2, "a node without an id is left out");
        assert!(recipe.document);
        assert_eq!(recipe.cannot.as_deref(), Some("no model `dawn` is set up"));
        assert_eq!(recipe.error, None);
        assert_eq!(recipe.reads_into("takes").len(), 1);
        let model = recipe.node("made:{out}").unwrap();
        assert_eq!(model.line, Some(2));
        assert_eq!(node_mark(model), Some(theme::warn()));
        let wants = wants_of(model);
        let keys: Vec<&str> = wants.iter().map(|want| want.key.as_str()).collect();
        assert_eq!(keys, ["prompt", "seed", "seconds", "size", "steps", ADD]);
        assert_eq!(wants[1].value, "7");
        assert_eq!(wants[4].placeholder, "30");
        assert_eq!(model_line(model), "seed 7");
    }

    #[test]
    fn a_plan_and_an_ask_read_as_the_bridge_and_kerf_say_them() {
        let plan = Plan::from_event(&json!({
            "type": "media_make_plan", "media": "m4", "recipe": "", "command": "render s1\ngenerate",
            "steps": [], "notes": ["the model makes 5 s"], "take": 3, "file": "/p/shots/m4.takes/003.mp4", "rolls": 4
        }))
        .unwrap();
        assert_eq!(
            plan.steps,
            ["render s1", "generate"],
            "the command's lines stand in for no steps"
        );
        assert_eq!(plan.takes(), "takes 3–6");
        assert!(Plan::from_event(&json!({"type": "media_made", "media": "m4"})).is_none());
        assert_eq!(takes_words(2, 1), "take 2");
        assert_eq!(takes_words(0, 3), "");

        let words = ask_words(
            "Asks to make m4, as take 2, from the steps under its line in the document:\nThe opening needs a boat.\n1. render s1 {guide.mp4} pass=depth\n2. generate model=dawn prompt=\"a boat\"\nNote: the model makes 832x480\nMake it with `kerf make m4`, or Make… in the editor's bin.",
        );
        assert_eq!(words.why, ["The opening needs a boat."]);
        assert_eq!(
            words.steps,
            [
                "render s1 {guide.mp4} pass=depth",
                "generate model=dawn prompt=\"a boat\""
            ]
        );
        assert_eq!(words.notes, ["the model makes 832x480"]);
    }

    #[test]
    fn a_refusal_reads_without_what_the_title_says() {
        assert_eq!(
            refusal("cannot make m1: no model `dawn` is set up", "m1"),
            "No model `dawn` is set up"
        );
        assert_eq!(
            refusal("cannot carry m1: it runs a program", "m1"),
            "It runs a program"
        );
        assert_eq!(refusal("something else", "m1"), "Something else");
        assert_eq!(value_text(&json!(0.5)), "0.5");
        assert_eq!(value_text(&json!(16.0)), "16");
        assert_eq!(value_text(&Value::Null), "");
    }
}
