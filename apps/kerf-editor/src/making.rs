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
use crate::selectable;
use crate::session::{BridgeEvent, Session};
use crate::store::Making;
use crate::theme::{self, motion as scene, radius, text};
use crate::ui::{self, icon_button};
use gpui::{
    actions, anchored, canvas, deferred, div, fill, point, prelude::*, px, size, AnimationExt,
    AnyElement, App, Bounds, Context, CursorStyle, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, Pixels, Point, RenderImage, Rgba, ScrollDelta, ScrollWheelEvent, SharedString,
    Subscription, Window,
};
use kerf_bridge::event::{MediaMakePlan, Recipe as RecipeEvent, TakeRead};
use kerf_bridge::{Command, Event, RecipeEdit, ShotFollow, ShotOf, WorkflowEdit};
use serde_json::{json, Map, Value};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

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

/// The plan's and the graph's own keys: Esc, which gives the window back,
/// and the canvas's fit, zoom and arrows. Binding a keymap clears them, so
/// each is bound again where it is missing: as a plan or a graph opens, as
/// what was over the graph closes, and by `keymap::bind` after a keymap.
pub fn bind_keys(cx: &mut App) {
    bind_close(cx);
    bind_graph_keys(cx);
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

/// Where a make card's words can be chosen and copied, as the
/// conversation's can: the conversation's selections, and the run the
/// words are pieces of. None: plain labels.
type Choose<'a> = Option<(&'a selectable::Shared, SharedString)>;

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
fn steps_block_in(steps: &[String], choose: &Choose) -> gpui::Div {
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
                    words_piece(choose, index, step.clone(), text::MONO_SMALL, theme::ink2())
                        .flex_1()
                        .min_w_0(),
                )
        }))
}

/// What a model has said it does not take, each worth mending before a making.
fn notes_block(heading: &str, notes: &[String]) -> Option<gpui::Div> {
    notes_block_in(heading, notes, &None)
}

/// [`notes_block`], its notes chosen and copied by `choose`.
fn notes_block_in(heading: &str, notes: &[String], choose: &Choose) -> Option<gpui::Div> {
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
            .children(notes.iter().enumerate().map(|(index, note)| {
                div()
                    .flex()
                    .gap(px(6.0))
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
    source.recipe.is_some() && source.steps == 0
}

// ------------------------------------------------------- makings in the conversation

/// How a making asked for in the conversation goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardState {
    Asked,
    Making,
    Made,
    Stopped,
    Failed,
    Declined,
}

/// What the cards have heard of a media's making: how it stands, what its
/// step said last, and the take it is of (0 while that is not known).
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub state: CardState,
    pub detail: String,
    pub take: u32,
}

/// How a card for `ask` stands, and what its making said last, from what is
/// known of its media: whether the take it asked for is made
/// (`made_already`), what the cards last heard of a making of that media
/// (`progress`), the making the project says is under way (`making`), and
/// whether a person said Not now to it.
///
/// A card made already stays made: a later making of the same media, and
/// that making's failure, belong to another card. A making is this card's
/// when its take is one this card asked for (any, for an ask that named no
/// take), or when its take is not known yet.
pub fn card_state(
    ask: &Ask,
    made_already: bool,
    progress: Option<&Progress>,
    making: Option<&Making>,
    declined: bool,
) -> (CardState, Progress) {
    let first = ask.take.unwrap_or(0);
    let last = first + ask.rolls.unwrap_or(1).max(1) - 1;
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
    let ours = |take: u32| first == 0 || take == 0 || (first..=last).contains(&take);
    let progress = progress.filter(|known| ours(known.take));
    // A making that failed is said so, whatever was last heard of it running.
    if let Some(failed) = progress.filter(|known| known.state == CardState::Failed) {
        return (CardState::Failed, failed.clone());
    }
    // A making under way, whoever runs it, is shown as it goes.
    if let Some(making) = making.filter(|making| ours(making.take)) {
        let detail = progress
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
    match progress {
        // A take made before this ask's is not what it asked for.
        Some(known) if known.state == CardState::Made && known.take < first => {}
        Some(known) => return (known.state, known.clone()),
        None => {}
    }
    let state = if declined {
        CardState::Declined
    } else {
        CardState::Asked
    };
    (
        state,
        Progress {
            state,
            detail: String::new(),
            take: first,
        },
    )
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
    /// Whether the conversation draws each turn's cards ([`Makings::cards_for`]).
    cards_in_turns: bool,
    /// The conversation's selections, which its cards' plans join so their
    /// words can be chosen and copied ([`Makings::share_selections`]).
    selections: Option<selectable::Shared>,
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
            cards_in_turns: false,
            selections: None,
            _subscriptions: subscriptions,
        };
        makings.ask_plans(cx);
        makings
    }

    fn heard(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::AgentAsk {
                media,
                rolls,
                take,
                text,
                conversation,
            } => {
                let ask = Ask {
                    media: media.clone(),
                    rolls: rolls.filter(|rolls| *rolls > 1),
                    take: *take,
                    text: text.clone(),
                };
                if !ask.media.is_empty()
                    && !self.live.iter().any(|(_, known)| key(known) == key(&ask))
                {
                    self.live.push((conversation.clone(), ask));
                }
            }
            Event::MediaMakePlan(plan) => {
                let plan = Plan::from_event(plan);
                let wanted = (plan.media.clone(), plan.rolls);
                self.asked.remove(&wanted);
                self.plans.insert(wanted, Ok(plan));
            }
            Event::MediaMaking { media, take, line } => {
                self.progress.insert(
                    media.clone(),
                    Progress {
                        state: CardState::Making,
                        detail: line.clone(),
                        take: *take,
                    },
                );
            }
            Event::MediaMade { media, take } => {
                self.progress.insert(
                    media.clone(),
                    Progress {
                        state: CardState::Made,
                        detail: String::new(),
                        take: *take,
                    },
                );
            }
            Event::MediaMakeCancelled { media } => {
                // The take it was of, so that a card made already is not this.
                let take = self.progress.get(media).map_or(0, |known| known.take);
                self.progress.insert(
                    media.clone(),
                    Progress {
                        state: CardState::Stopped,
                        detail: String::new(),
                        take,
                    },
                );
            }
            Event::Error(error)
                if error.command.as_deref() == Some("media_make")
                    && !error.media.as_deref().unwrap_or_default().is_empty() =>
            {
                let media = error.media.clone().unwrap_or_default();
                let message = error.message.as_str();
                let making = self
                    .progress
                    .get(&media)
                    .filter(|known| known.state == CardState::Making)
                    .map(|known| known.take);
                if let Some(take) = making {
                    let detail = refusal(message, &media);
                    self.progress.insert(
                        media,
                        Progress {
                            state: CardState::Failed,
                            detail,
                            take,
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
                        self.plans.insert(wanted, Err(refusal(message, &media)));
                    }
                }
            }
            _ => {}
        }
        if matches!(
            event,
            Event::AgentAsk { .. }
                | Event::Conversation(_)
                | Event::Connection { .. }
                | Event::State { .. }
        ) {
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
                session.send(Command::MediaMake {
                    media: media.clone(),
                    confirmed: false,
                    rolls: (*rolls > 1).then_some(*rolls),
                });
            }
        });
        self.asked.extend(wanted);
    }

    /// How a card stands, and what its making said last ([`card_state`]).
    fn state_of(&self, ask: &Ask, session: &Session) -> (CardState, Progress) {
        let take = ask.take.unwrap_or(0);
        let last = take + ask.rolls.unwrap_or(1).max(1) - 1;
        let made_already = take > 0
            && session
                .timeline()
                .source(&ask.media)
                .is_some_and(|source| source.takes.iter().any(|known| known.number >= last));
        card_state(
            ask,
            made_already,
            self.progress.get(&ask.media),
            session.state.makings.get(&ask.media),
            self.declined.contains(&key(ask)),
        )
    }

    fn make(&mut self, ask: &Ask, take: u32, cx: &mut Context<Self>) {
        let rolls = ask.rolls.unwrap_or(1).max(1);
        let media = ask.media.clone();
        self.session.update(cx, |session, cx| {
            session.send(Command::MediaMake {
                media,
                confirmed: true,
                rolls: (rolls > 1).then_some(rolls),
            });
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
            name,
        } = facts;
        let rolls = ask.rolls.unwrap_or(1).max(1);
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
        let title = card_title(state, &name, rolls, progress.take);
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
        // In the conversation, the plan's words can be chosen and copied.
        let choose = |part: &str| -> Choose {
            self.selections
                .as_ref()
                .map(|shared| (shared, format!("plan-{card_key}-{part}").into()))
        };
        if plan_open {
            if !words.why.is_empty() {
                let why = words.why.join(" ");
                body = body.child(match choose("why") {
                    Some((shared, run)) => theme::reading(div()).text_color(theme::ink2()).child(
                        selectable::selectable(
                            shared,
                            ElementId::Name(format!("{run}-0").into()),
                            run,
                            0,
                            why,
                        ),
                    ),
                    None => theme::reading(div()).text_color(theme::ink2()).child(why),
                });
            }
            match &plan {
                Some(Ok(plan)) => {
                    body = body.child(steps_block_in(&plan.steps, &choose("steps")));
                    if let Some(notes) =
                        notes_block_in("Before you run it", &plan.notes, &choose("notes"))
                    {
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
                        body = body.child(steps_block_in(&words.steps, &choose("steps")));
                    }
                    body = body.child(ui::label(
                        format!("It cannot be made as things stand: {why}"),
                        text::CONTROL,
                        theme::neg(),
                    ));
                }
                None => {
                    if !words.steps.is_empty() {
                        body = body.child(steps_block_in(&words.steps, &choose("steps")));
                    }
                    if let Some(notes) =
                        notes_block_in("Before you run it", &words.notes, &choose("notes"))
                    {
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
                            session.send(Command::MediaMakeCancel {
                                media: stop.clone(),
                            });
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
    /// The media as the bin names it: its file's name.
    name: String,
}

/// A make card's title, naming the media `name` as the bin does.
pub fn card_title(state: CardState, name: &str, rolls: u32, take: u32) -> String {
    match state {
        CardState::Asked if rolls > 1 => format!("Make {rolls} takes of {name}?"),
        CardState::Asked => format!("Make {name}?"),
        CardState::Making => format!("Making {name}"),
        CardState::Made if take > 0 => format!("Take {take} of {name} is made"),
        CardState::Made => format!("{name} is made"),
        CardState::Stopped => format!("Stopped making {name}"),
        CardState::Failed => format!("{name} was not made"),
        CardState::Declined => format!("Not made: {name}"),
    }
}

/// A card's key: a making asked for twice is one card.
fn key(ask: &Ask) -> String {
    format!("{}#{}", ask.media, ask.take.unwrap_or(0))
}

impl Makings {
    /// The cards of `asks`, one for each making asked for (one asked twice
    /// is one card), as the conversation draws them under the answer whose
    /// turn asked: chat.rs calls this from its turn, once it has said
    /// [`Makings::set_cards_in_turns`].
    pub fn cards_for(&mut self, asks: &[Ask], cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut unique: Vec<Ask> = Vec::new();
        for ask in asks {
            if !ask.media.is_empty() && !unique.iter().any(|known| key(known) == key(ask)) {
                unique.push(ask.clone());
            }
        }
        self.cards(unique, cx)
    }

    /// The conversation's selections: its cards' plans can be chosen and
    /// copied with its words (Copy, ⌘C).
    pub fn share_selections(&mut self, shared: selectable::Shared) {
        self.selections = Some(shared);
    }

    /// Whether the conversation draws each turn's cards itself
    /// ([`Makings::cards_for`]): this view then draws only the asks no turn
    /// of the conversation open holds yet, those heard as Kerf made them.
    pub fn set_cards_in_turns(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.cards_in_turns != on {
            self.cards_in_turns = on;
            cx.notify();
        }
    }

    fn cards(&mut self, asks: Vec<Ask>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let connected = session.state.connection == crate::store::Connection::Connected;
        let busy = session.state.work.busy();
        let shown: Vec<(Ask, Facts)> = asks
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
                    name: session.media_name(&ask.media),
                };
                (ask, facts)
            })
            .collect();
        shown
            .into_iter()
            .map(|(ask, facts)| self.card(&ask, facts, cx))
            .collect()
    }
}

impl Render for Makings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let mut asks = self.asks(session);
        if self.cards_in_turns {
            // Those a turn holds are drawn under it.
            if let Some(open) = session.state.open_conversation.as_ref() {
                asks.retain(|ask| {
                    !open
                        .items
                        .iter()
                        .flat_map(|turn| turn.asks.iter())
                        .any(|held| key(held) == key(ask))
                });
            }
        }
        let cards = self.cards(asks, cx);
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

/// One node of a shot's graph, as the bridge's `recipe` lays it out; or,
/// in a model's workflow, one of the workflow's own nodes (kind `part`).
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

impl GraphNode {
    /// A step of the shot: a line of its steps.
    pub fn is_step(&self) -> bool {
        self.line.is_some_and(|line| line > 0)
    }

    /// One of a model's workflow's own nodes.
    pub fn is_part(&self) -> bool {
        self.kind == "part"
    }

    /// A model its entry runs with kerf-comfy: it has a workflow to open.
    pub fn has_workflow(&self) -> bool {
        self.kind == "model" && self.holds["runner"]["kind"] == "comfy"
    }

    /// A model not set up on this computer, or a file that is not there.
    pub fn is_missing(&self) -> bool {
        if self.kind == "model" {
            return self.holds["setUp"] == Value::Bool(false);
        }
        self.holds["there"] == Value::Bool(false)
    }
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

/// A media's steps as a graph (the bridge's `recipe`), or a model's
/// workflow as one ([`Workflow::read`]).
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
    /// A model's workflow: its nodes are `part`s.
    pub parts: bool,
}

impl Recipe {
    /// Reads a `recipe` event's graph, leniently. An edge to or from what is
    /// not there is left out.
    pub fn read(event: &RecipeEvent) -> Self {
        let field = |key: &str| event.graph.get(key).unwrap_or(&Value::Null);
        let words = |key: &str| field(key).as_str().unwrap_or_default().to_owned();
        let said = |key: &str| {
            field(key)
                .as_str()
                .filter(|said| !said.is_empty())
                .map(str::to_owned)
        };
        let nodes: Vec<GraphNode> = field("nodes")
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
        let there = |id: &str| nodes.iter().any(|node| node.id == id);
        let edges = field("edges")
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
                    .filter(|edge| there(&edge.from) && there(&edge.to))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            media: words("media"),
            recipe: words("recipe"),
            document: field("document").as_bool().unwrap_or(false),
            name: words("name"),
            text: words("text"),
            nodes,
            edges,
            error: said("error"),
            cannot: said("cannot"),
            notes: strings(field("notes")),
            models_error: event.models_error.clone().filter(|said| !said.is_empty()),
            parts: false,
        }
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// What reads into `id`, in the order the steps give it.
    pub fn reads_into(&self, id: &str) -> Vec<&GraphEdge> {
        self.edges.iter().filter(|edge| edge.to == id).collect()
    }

    /// Whether its steps read, so a making can be asked for.
    pub fn reads(&self) -> bool {
        self.error.is_none() && !self.nodes.is_empty()
    }
}

/// The workflow a model is run with, where its program is kerf-comfy (the
/// bridge's `workflow`): every node and input of the ComfyUI graph, as a
/// graph of `part`s, and the blanks the shot's steps fill.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workflow {
    pub media: String,
    /// The `generate` line it is of, from 1.
    pub line: usize,
    pub model: String,
    pub name: String,
    /// `own` (the shot's own copy), `file` (the models entry's), or `bundled`.
    pub origin: String,
    /// The shot's own copy, where it has one.
    pub file: String,
    pub about: String,
    /// Its text as read: what an edit says it changes.
    pub text: String,
    /// Each blank by name: what fills it (`says`), what the line gives for it now (`given`).
    pub blanks: serde_json::Map<String, Value>,
    pub graph: Recipe,
}

/// A workflow node's id in the graph.
pub fn part_id(id: &str) -> String {
    format!("part:{id}")
}

impl Workflow {
    /// Reads a `workflow` event's fields, leniently.
    pub fn read(event: &Map<String, Value>) -> Self {
        let field = |key: &str| event.get(key).unwrap_or(&Value::Null);
        let words = |key: &str| field(key).as_str().unwrap_or_default().to_owned();
        let (media, model, name, origin, file) = (
            words("media"),
            words("model"),
            words("name"),
            words("origin"),
            words("file"),
        );
        let mut graph = Recipe {
            media: media.clone(),
            name: model.clone(),
            recipe: if origin == "own" {
                file.clone()
            } else {
                name.clone()
            },
            text: words("text"),
            parts: true,
            ..Recipe::default()
        };
        for node in field("nodes").as_array().into_iter().flatten() {
            let id = value_text(&node["id"]);
            if id.is_empty() {
                continue;
            }
            // By what ComfyUI's own editor calls it where its export says, else by its kind.
            let class = node["class"].as_str().unwrap_or_default().to_owned();
            let title = node["title"]
                .as_str()
                .filter(|title| !title.is_empty())
                .map_or_else(|| class.clone(), str::to_owned);
            let detail = if title == class { String::new() } else { class };
            graph.nodes.push(GraphNode {
                id: part_id(&id),
                kind: "part".into(),
                title,
                detail,
                gives: "file".into(),
                line: None,
                holds: node.clone(),
            });
        }
        let mut edges = Vec::new();
        for node in &graph.nodes {
            for input in node.holds["inputs"].as_array().into_iter().flatten() {
                let Some(from) = input["link"].as_array().and_then(|link| link.first()) else {
                    continue;
                };
                let from = part_id(&value_text(from));
                if graph.node(&from).is_some() {
                    edges.push(GraphEdge {
                        from,
                        to: node.id.clone(),
                        port: input["name"].as_str().unwrap_or_default().to_owned(),
                        nth: 0,
                        gives: "file".into(),
                    });
                }
            }
        }
        graph.edges = edges;
        Self {
            media,
            line: field("line").as_u64().unwrap_or(0) as usize,
            model,
            name,
            origin,
            file,
            about: words("about"),
            text: words("text"),
            blanks: field("blanks").as_object().cloned().unwrap_or_default(),
            graph,
        }
    }

    /// The shot's own copy, which its line names.
    pub fn is_own(&self) -> bool {
        self.origin == "own"
    }

    /// What fills blank `name`, and what the line gives for it now.
    pub fn blank_says(&self, name: &str) -> String {
        let blank = self.blanks.get(name).cloned().unwrap_or_default();
        let says = capitalised(blank["says"].as_str().unwrap_or_default());
        if says.is_empty() {
            return says;
        }
        match &blank["given"] {
            Value::String(given) => format!("{says}. Now: {given}"),
            // A file or a signal it gives none of leaves the node out.
            _ if matches!(blank["by"].as_str(), Some("file" | "signal" | "moves")) => {
                format!("{says}. The line gives none, so the node is left out")
            }
            _ => says,
        }
    }
}

fn capitalised(words: &str) -> String {
    let mut letters = words.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// What a workflow node's input is: a link from another node, a blank the
/// shot's steps fill, or a value of the workflow's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Link,
    Blank,
    Value,
}

pub fn input_kind(input: &Value) -> InputKind {
    if input.get("link").is_some_and(|link| !link.is_null()) {
        return InputKind::Link;
    }
    if input["blanks"]
        .as_array()
        .is_some_and(|blanks| !blanks.is_empty())
    {
        InputKind::Blank
    } else {
        InputKind::Value
    }
}

/// A workflow's value as a row of it shows it: words as they are, a number
/// as it is written, a list or a table as JSON.
pub fn shown_value(value: &Value) -> String {
    match value {
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(whole), _) => whole.to_string(),
            (None, Some(real)) if real.fract() == 0.0 && real.abs() < 1e15 => {
                format!("{}", real as i64)
            }
            (None, Some(real)) => format!("{real}"),
            _ => number.to_string(),
        },
        other => value_text(other),
    }
}

// ------------------------------------------------------------------ wiring

/// The names `to` could read what `from` gives under, the first the most
/// likely: none where it cannot read it at all. A step reads what a step
/// before it made, never one after; a guide reads a sequence; a model reads
/// a signal, a clip's way, the cast and styles as such, a picture in motion
/// as a guide (or another of the guides it says it reads), a reference or
/// its first frame, and a still as its first or last frame or a reference.
/// In a model's workflow, a node's output is read by another's input that
/// is a link already and takes what it gives.
pub fn ports_for(from: &GraphNode, to: &GraphNode) -> Vec<String> {
    if from.is_part() || to.is_part() {
        if from.kind != to.kind || from.id == to.id {
            return Vec::new();
        }
        let gives = strings(&from.holds["outputs"]);
        return to.holds["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|input| input_kind(input) == InputKind::Link)
            .filter(|input| {
                let takes = input["type"].as_str().unwrap_or_default();
                gives.is_empty() || takes.is_empty() || gives.iter().any(|gives| gives == takes)
            })
            .map(|input| input["name"].as_str().unwrap_or_default().to_owned())
            .collect();
    }
    if from.id == to.id || !to.is_step() {
        return Vec::new();
    }
    if from.is_step() && from.line >= to.line {
        return Vec::new();
    }
    let one = |port: &str| vec![port.to_owned()];
    if to.kind == "render" {
        return if from.kind == "sequence" {
            one("sequence")
        } else {
            Vec::new()
        };
    }
    if to.kind != "model" {
        return Vec::new();
    }
    match from.kind.as_str() {
        "signal" => return one("signal"),
        "moves" => return one("moves"),
        "cast" => return one("cast"),
        "style" => return one("style"),
        "sequence" | "takes" => return Vec::new(),
        _ => {}
    }
    match from.gives.as_str() {
        "video" => {
            let mut ports = one("guide");
            // The guides it reads beside the first, by what each is of.
            ports.extend(
                strings(&to.holds["reads"])
                    .into_iter()
                    .filter(|read| read.starts_with("guide_")),
            );
            // As a first frame it is where it ended: the shot goes on from it.
            ports.extend(["ref".to_owned(), "first".to_owned()]);
            ports
        }
        "image" => ["first", "last", "ref"].map(str::to_owned).to_vec(),
        _ => one("ref"),
    }
}

/// What a step's line writes to read `from`: a clip by its name, one of the
/// cast or a style by its name, a file by its path, a sequence by its id, a
/// step's file by the name it has in braces. Nothing for the takes.
pub fn value_of(from: &GraphNode) -> String {
    let said = |key: &str| from.holds[key].as_str().unwrap_or_default().to_owned();
    match from.kind.as_str() {
        "signal" | "moves" => said("clip"),
        "cast" | "style" => said("name"),
        "file" => said("path"),
        "sequence" => said("sequence"),
        "takes" => String::new(),
        _ => {
            // A program may make several: the first of them.
            let makes = match &from.holds["makes"] {
                Value::Array(names) => names.first().and_then(Value::as_str),
                other => other.as_str(),
            }
            .unwrap_or_default();
            if makes.is_empty() {
                String::new()
            } else {
                format!("{{{makes}}}")
            }
        }
    }
}

/// What a model makes of what comes in as `port`.
pub fn port_says(port: &str) -> String {
    if let Some(of) = port.strip_prefix("guide_") {
        return format!("another picture in motion to follow, of its {of}");
    }
    match port {
        "guide" => "a picture in motion to follow",
        "first" => "the first frame",
        "last" => "the last frame",
        "ref" => "who or what is in it, or how it looks",
        "signal" => "when, and how much",
        "moves" => "where a thing goes",
        "cast" => "who or what it shows",
        "style" => "how it looks",
        "sequence" => "what the guide is drawn from",
        _ => "",
    }
    .to_owned()
}

/// Whether `to` makes use of what comes in as `port`: a model that has
/// said what it reads reads only that, and the cast and styles always.
pub fn reads_port(to: &GraphNode, port: &str) -> bool {
    if to.kind != "model" || to.holds.get("reads").is_none_or(Value::is_null) {
        return true;
    }
    port == "cast" || port == "style" || strings(&to.holds["reads"]).iter().any(|read| read == port)
}

/// Which word of `port` on `to`'s line a new reading is: a name a line
/// gives once is replaced (0); one it may give again (a reference, a
/// signal, a clip's way, the cast, a style) is one more.
pub fn wire_nth(recipe: &Recipe, to: &str, port: &str) -> usize {
    if !matches!(port, "ref" | "signal" | "moves" | "cast" | "style") {
        return 0;
    }
    recipe
        .reads_into(to)
        .iter()
        .filter(|edge| edge.port == port)
        .count()
}

/// Which output of workflow node `from` input `input` of `to` reads: the
/// first that gives what the input takes.
pub fn link_output(from: &GraphNode, to: &GraphNode, input: &str) -> usize {
    let takes = to.holds["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|known| known["name"] == input)
        .and_then(|known| known["type"].as_str())
        .unwrap_or_default();
    strings(&from.holds["outputs"])
        .iter()
        .position(|gives| gives == takes)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ the canvas's geometry

/// A node's card, and the room between the cards for the edges to turn in.
const CARD_W: f32 = 208.0;
const CARD_H: f32 = 56.0;
/// What a model's line of its seed and size adds to its card.
const CARD_MODEL: f32 = 18.0;
/// What a signal's curve or a clip's path adds to its card.
const CARD_CURVE: f32 = 36.0;
const COLUMN_GAP: f32 = 112.0;
const ROW_GAP: f32 = 20.0;
/// A node of a model's workflow: its name over a row an input; and between
/// its columns, as its rows name what comes in.
const PART_W: f32 = 232.0;
const PART_HEAD: f32 = 28.0;
const PART_ROW: f32 = 20.0;
const PART_GAP: f32 = 64.0;
/// The canvas's dots, drawn while they are this far apart at least.
const GRID: f32 = 24.0;
const GRID_LEAST: f32 = 10.0;
/// What floats over the canvas keeps this far from its edges.
const PAD: f32 = 12.0;
/// How near the pointer has to be to a port to be on it, on the screen.
const PORT_REACH: f32 = 9.0;
/// A press that moves less than this is a click.
const DRAG_FROM: f32 = 3.0;
/// The panel at most, less where the window is smaller; its head; the
/// column that says what the chosen node holds.
const GRAPH_MOST: (f32, f32) = (1480.0, 900.0);
const HEAD: f32 = 48.0;
const INSPECTOR: f32 = 300.0;
/// The legend of what the edges carry, at the canvas's foot.
const LEGEND: f32 = 28.0;
/// A field's name, before it; a workflow input's longer one.
const FIELD_LABEL: f32 = 76.0;
const INPUT_LABEL: f32 = 112.0;
/// A take's picture in its row.
const TAKE_PICTURE: (f32, f32) = (57.0, 32.0);
/// A signal's curve, drawn to be drawn on.
const CURVE_HEIGHT: f32 = 112.0;
const CURVE_INSET: f32 = 8.0;
/// How far a drawn signal's line may miss a frame between its keys.
const CURVE_WITHIN: f64 = 0.02;
/// The widest a picture in the column is, and the tallest.
const PICTURE_MOST: (f32, f32) = (INSPECTOR - 28.0, 160.0);
/// What the canvas's zoom goes up or down by a press.
pub const ZOOM_STEP: f32 = 1.2;
/// The least and the most the canvas is drawn at.
pub const ZOOM_LEAST: f32 = 0.35;
pub const ZOOM_MOST: f32 = 2.0;
/// The smallest a graph is shown at by itself: smaller than this its words
/// are not read, so a graph too large for the canvas starts at its left at
/// this size, to be moved along, and Fit takes all of it in. Edges' names
/// are not drawn under it either.
pub const ZOOM_READ: f32 = 0.7;

/// The graph panel's size in a window `viewport` big: [`GRAPH_MOST`],
/// less where the window is smaller, with the
/// modal's margin all round (`ui::modal_bounds`), so the whole panel, its
/// foot among it, shows in the box the window gives it however small the
/// window. Its inspector column scrolls what does not fit.
fn graph_size(viewport: (f32, f32)) -> (f32, f32) {
    let placed = ui::modal_bounds(
        gpui::size(px(GRAPH_MOST.0), px(GRAPH_MOST.1)),
        gpui::size(px(viewport.0), px(viewport.1)),
    );
    (f32::from(placed.size.width), f32::from(placed.size.height))
}

/// Whether a card draws something under its lines: a signal's number over
/// its time, or the path a clip is led along.
pub fn draws_curve(node: &GraphNode) -> bool {
    match node.kind.as_str() {
        "signal" => node.holds["values"]
            .as_array()
            .is_some_and(|values| !values.is_empty()),
        "moves" => node.holds["path"]
            .as_str()
            .is_some_and(|path| !path.is_empty()),
        _ => false,
    }
}

/// A card's size, before what it reads makes it taller: a workflow's node
/// is its name and a row for each input; a model says a third thing; a
/// signal or a clip's way draws under its lines.
pub fn card_size(node: &GraphNode) -> (f32, f32) {
    if node.is_part() {
        let rows = node.holds["inputs"].as_array().map_or(0, Vec::len);
        return (PART_W, PART_HEAD + PART_ROW * rows as f32 + 8.0);
    }
    let mut height = CARD_H;
    if node.kind == "model" {
        height += CARD_MODEL;
    }
    if draws_curve(node) {
        height += CARD_CURVE;
    }
    (CARD_W, height)
}

/// A card's height: room for its lines, and for each thing it reads to
/// come in apart (a workflow's node has a row for each already).
pub fn card_height(node: &GraphNode, inputs: usize) -> f32 {
    let (_, height) = card_size(node);
    if node.is_part() {
        return height;
    }
    height.max(16.0 + 24.0 * inputs as f32)
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
            let card = &recipe.nodes[*node];
            let pitch = card_size(card).0 + if card.is_part() { PART_GAP } else { COLUMN_GAP };
            places.insert(card.id.clone(), (at as f32 * pitch, y.round()));
            y += height + ROW_GAP;
        }
    }
    places
}

/// Where each card is: as laid out, or where the person put it.
pub fn placed_over(
    recipe: &Recipe,
    placed: &HashMap<String, (f32, f32)>,
) -> HashMap<String, (f32, f32)> {
    let mut places = arrange(recipe);
    for (id, place) in places.iter_mut() {
        if let Some(put) = placed.get(id) {
            *place = *put;
        }
    }
    places
}

/// A card's rectangle on the scene: left, top, width, height.
pub fn card_rect(
    recipe: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    id: &str,
) -> Option<[f32; 4]> {
    let node = recipe.node(id)?;
    let (x, y) = places.get(id)?;
    let (width, _) = card_size(node);
    let height = card_height(node, recipe.reads_into(id).len());
    Some([*x, *y, width, height])
}

/// Where an edge leaves `id`, on the scene: its right edge, at its middle;
/// a workflow's node beside its name, over its rows.
pub fn out_port(
    recipe: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    id: &str,
) -> Option<(f32, f32)> {
    let [x, y, width, height] = card_rect(recipe, places, id)?;
    if recipe.node(id).is_some_and(GraphNode::is_part) {
        return Some((x + width, y + PART_HEAD / 2.0 + 2.0));
    }
    Some((x + width, y + height / 2.0))
}

/// Where `edge` comes into its card, on the scene: at the row of the input
/// it is, in a workflow; else spread down its left edge in the order the
/// steps give what it reads.
pub fn in_port(
    recipe: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    edge: &GraphEdge,
) -> Option<(f32, f32)> {
    let [x, y, _, height] = card_rect(recipe, places, &edge.to)?;
    let to = recipe.node(&edge.to)?;
    if to.is_part() {
        let row = to.holds["inputs"]
            .as_array()
            .and_then(|inputs| inputs.iter().rposition(|input| input["name"] == edge.port))
            .unwrap_or(0);
        return Some((x, y + PART_HEAD + PART_ROW * (row as f32 + 0.5)));
    }
    let into = recipe.reads_into(&edge.to);
    let index = into
        .iter()
        .rposition(|other| {
            other.from == edge.from && other.port == edge.port && other.nth == edge.nth
        })
        .unwrap_or(0);
    Some((
        x,
        y + height * (index as f32 + 0.5) / into.len().max(1) as f32,
    ))
}

/// How the canvas is looked at: where on it a point of the scene is drawn
/// is `offset + point * zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub zoom: f32,
    pub offset: (f32, f32),
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: (0.0, 0.0),
        }
    }
}

impl Camera {
    pub fn to_view(&self, (x, y): (f32, f32)) -> (f32, f32) {
        (self.offset.0 + x * self.zoom, self.offset.1 + y * self.zoom)
    }

    pub fn to_scene(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let zoom = self.zoom.max(f32::EPSILON);
        ((x - self.offset.0) / zoom, (y - self.offset.1) / zoom)
    }
}

/// What every card covers of the scene: left, top, right, bottom.
pub fn extent(recipe: &Recipe, places: &HashMap<String, (f32, f32)>) -> Option<[f32; 4]> {
    let mut all: Option<[f32; 4]> = None;
    for node in &recipe.nodes {
        let Some([x, y, width, height]) = card_rect(recipe, places, &node.id) else {
            continue;
        };
        all = Some(match all {
            None => [x, y, x + width, y + height],
            Some([left, top, right, bottom]) => [
                left.min(x),
                top.min(y),
                right.max(x + width),
                bottom.max(y + height),
            ],
        });
    }
    all
}

/// The canvas `room` big, looking at cards covering `extent`: all of them,
/// in the middle of the room the legend leaves, no larger than they are; or,
/// short of the `whole` of them (Fit), a graph too large to read whole at
/// its left, at a size that is read ([`ZOOM_READ`]).
pub fn settle(extent: Option<[f32; 4]>, room: (f32, f32), whole: bool) -> Camera {
    let Some([left, top, right, bottom]) = extent else {
        return Camera {
            zoom: 1.0,
            offset: ((room.0 / 2.0).round(), (room.1 / 2.0).round()),
        };
    };
    // Room round the cards, and the legend under them.
    let area = [
        32.0,
        24.0,
        room.0 - 32.0,
        room.1 - (LEGEND + 2.0 * PAD + 8.0),
    ];
    let (width, height) = ((area[2] - area[0]).max(1.0), (area[3] - area[1]).max(1.0));
    let (all_w, all_h) = ((right - left).max(1.0), (bottom - top).max(1.0));
    let zoom = 1.0f32
        .min(width / all_w)
        .min(height / all_h)
        .clamp(ZOOM_LEAST, ZOOM_MOST);
    let centre = ((area[0] + area[2]) / 2.0, (area[1] + area[3]) / 2.0);
    let middle = ((left + right) / 2.0, (top + bottom) / 2.0);
    let mut offset = (centre.0 - middle.0 * zoom, centre.1 - middle.1 * zoom);
    let mut zoom = zoom;
    if !whole && zoom < ZOOM_READ {
        // Too wide or too tall to be read whole: where it starts, at a size that is.
        zoom = ZOOM_READ;
        let wide = all_w * zoom > width;
        let tall = all_h * zoom > height;
        offset = (
            if wide {
                area[0] - left * zoom
            } else {
                centre.0 - middle.0 * zoom
            },
            if tall {
                area[1] - top * zoom
            } else {
                centre.1 - middle.1 * zoom
            },
        );
    }
    Camera {
        zoom,
        offset: (offset.0.round(), offset.1.round()),
    }
}

/// `camera` zoomed by `factor` about `about`, a point of the canvas: what is
/// under it stays under it. The zoom stays between [`ZOOM_LEAST`] and
/// [`ZOOM_MOST`].
pub fn zoom_about(camera: Camera, factor: f32, about: (f32, f32)) -> Camera {
    let zoom = (camera.zoom * factor).clamp(ZOOM_LEAST, ZOOM_MOST);
    let scene = camera.to_scene(about);
    Camera {
        zoom,
        offset: (about.0 - scene.0 * zoom, about.1 - scene.1 * zoom),
    }
}

/// Which way an arrow key goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    Left,
    Right,
    Up,
    Down,
}

/// The card an arrow goes to from `from`: the nearest that way, by the
/// middles of the cards (`centres`), one well off to the side counting for
/// more; the first card when none is chosen.
pub fn nearest_toward(
    centres: &[(String, (f32, f32))],
    from: Option<&str>,
    toward: Toward,
) -> Option<String> {
    let Some(start) = from.and_then(|from| centres.iter().find(|(id, _)| id == from)) else {
        return centres.first().map(|(id, _)| id.clone());
    };
    let across = matches!(toward, Toward::Left | Toward::Right);
    let sign = if matches!(toward, Toward::Left | Toward::Up) {
        -1.0
    } else {
        1.0
    };
    let mut best: Option<(&str, f32)> = None;
    for (id, centre) in centres {
        if id == &start.0 {
            continue;
        }
        let by = (centre.0 - start.1 .0, centre.1 - start.1 .1);
        let along = if across { by.0 } else { by.1 } * sign;
        let aside = if across { by.1 } else { by.0 }.abs();
        if along <= 1.0 || aside > along * 2.0 + 40.0 {
            continue;
        }
        let far = along + aside * 2.0;
        if best.is_none_or(|(_, nearest)| far < nearest) {
            best = Some((id, far));
        }
    }
    best.map(|(id, _)| id.to_owned())
}

// ------------------------------------------------------------------ a signal drawn

/// A signal's values as a line across `rect` (left, top, width, height):
/// 0 at its foot and 1 at its top, the first value at its left and the last
/// at its right; one value is a level across the whole of it.
pub fn signal_line(values: &[f64], rect: [f32; 4]) -> Vec<(f32, f32)> {
    let [left, top, width, height] = rect;
    if values.is_empty() || width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let last = values.len() - 1;
    let mut line: Vec<(f32, f32)> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let along = if last == 0 {
                0.0
            } else {
                index as f32 / last as f32
            };
            let value = if value.is_finite() {
                value.clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            (left + along * width, top + height - value * height)
        })
        .collect();
    if last == 0 {
        line.push((left + width, line[0].1));
    }
    line
}

/// The frame and the value under `at` on a curve of `frames` frames drawn
/// across `rect`: the nearest frame, and 0 at the foot to 1 at the top, each
/// held to the curve.
pub fn signal_under(rect: [f32; 4], frames: usize, at: (f32, f32)) -> (usize, f64) {
    let [left, top, width, height] = rect;
    let last = frames.max(1) - 1;
    let along = if width > 0.0 {
        ((at.0 - left) / width).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let frame = ((along * last as f32).round() as usize).min(last);
    let value = if height > 0.0 {
        (1.0 - (at.1 - top) / height).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (frame, f64::from(value))
}

/// A drawing goes on to `value` at `frame`: every frame the pointer passed
/// since the `last` it was at lies on the line between the two.
pub fn draw_signal(
    drawn: &mut BTreeMap<usize, f64>,
    last: Option<usize>,
    frame: usize,
    value: f64,
) {
    if let Some(last) = last.filter(|last| *last != frame) {
        let start = drawn.get(&last).copied().unwrap_or(value);
        let span = frame as f64 - last as f64;
        let (low, high) = (last.min(frame), last.max(frame));
        for at in low + 1..high {
            let share = (at as f64 - last as f64) / span;
            drawn.insert(at, start + (value - start) * share);
        }
    }
    drawn.insert(frame, value);
}

/// The keys a drawing is kept as: from its first frame on, a frame is a
/// key where a straight line from the last key to the next frame would miss
/// a frame between by more than `within`; and its last frame. Each is
/// linear, its value to a thousandth.
pub fn signal_keys(drawn: &BTreeMap<usize, f64>, within: f64) -> Vec<Value> {
    let frames: Vec<(usize, f64)> = drawn.iter().map(|(at, value)| (*at, *value)).collect();
    let key = |(at, value): (usize, f64)| json!({"at": at, "value": (value * 1000.0).round() / 1000.0, "interp": "linear"});
    let Some(first) = frames.first() else {
        return Vec::new();
    };
    let mut keys = vec![key(*first)];
    let mut kept = 0;
    for next in 2..frames.len() {
        let (a, b) = (frames[kept], frames[next]);
        let span = (b.0 - a.0) as f64;
        let misses = (kept + 1..next).any(|between| {
            let on = a.1 + (b.1 - a.1) * (frames[between].0 - a.0) as f64 / span;
            (on - frames[between].1).abs() > within
        });
        if misses {
            kept = next - 1;
            keys.push(key(frames[kept]));
        }
    }
    if frames.len() > 1 {
        keys.push(key(frames[frames.len() - 1]));
    }
    keys
}

/// Where `at` is on a picture drawn at `frame` (left, top, width, height),
/// as shares of its width and height from its top left; none off it.
pub fn point_on(frame: [f32; 4], at: (f32, f32)) -> Option<(f64, f64)> {
    let [left, top, width, height] = frame;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let inside = at.0 >= left && at.0 <= left + width && at.1 >= top && at.1 <= top + height;
    inside.then(|| {
        (
            f64::from(((at.0 - left) / width).clamp(0.0, 1.0)),
            f64::from(((at.1 - top) / height).clamp(0.0, 1.0)),
        )
    })
}

/// Where a picture `size` big is drawn inside `room` (left, top, width,
/// height): whole, at its own shape, in the middle.
pub fn fitted_in(room: [f32; 4], size: (u32, u32)) -> [f32; 4] {
    let [left, top, width, height] = room;
    if size.0 == 0 || size.1 == 0 {
        return room;
    }
    let scale = (width / size.0 as f32).min(height / size.1 as f32);
    let (w, h) = (size.0 as f32 * scale, size.1 as f32 * scale);
    [left + (width - w) / 2.0, top + (height - h) / 2.0, w, h]
}

// ------------------------------------------------------------------ where cards were put
//
// The cards' places are kept with the editor's own settings
// (`Prefs::graph_places`), never in the recipe.

/// The key a shot's cards are kept under: its project and its media.
pub fn placement_key(project: Option<&Path>, media: &str) -> String {
    let project = project.map(|project| project.display().to_string());
    format!("{}\nmedia {media}", project.unwrap_or_default())
}

/// The key a model's workflow's cards are kept under: another arrangement
/// than its shot's.
pub fn workflow_key(shot: &str, workflow: &str) -> String {
    format!("{shot}\nworkflow {workflow}")
}

fn load_places(session: &Entity<Session>, key: &str, cx: &App) -> HashMap<String, (f32, f32)> {
    session.read(cx).prefs.graph_places_of(key)
}

fn save_places(
    session: &Entity<Session>,
    key: &str,
    places: &HashMap<String, (f32, f32)>,
    cx: &mut App,
) {
    session.update(cx, |session, _| {
        session.prefs.keep_graph_places(key, places);
        session.save_prefs();
    });
}

// ------------------------------------------------------------------ what is said of a take

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

// ------------------------------------------------------------------ starting on a graph

/// A shot's graph to start on, and what is chosen in it: a node's id, or
/// `workflow:<line>` for the workflow of the model on that line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphStart {
    pub media: String,
    pub node: Option<String>,
}

/// Takes `--graph <media>` and `--graph-node <id>` (or `--graph=…`,
/// `--graph-node=…`) out of `args`: the graph the editor starts on, once
/// its project has said that media. A `--graph-node` without `--graph`, or
/// a flag with nothing after it, is refused.
pub fn graph_start(args: &mut Vec<String>) -> Result<Option<GraphStart>, String> {
    let mut media = None;
    let mut node = None;
    let mut kept = Vec::with_capacity(args.len());
    let mut words = std::mem::take(args).into_iter();
    while let Some(word) = words.next() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag == "--graph" || flag == "--graph-node" => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (word.clone(), None),
        };
        let slot = match flag.as_str() {
            "--graph" => &mut media,
            "--graph-node" => &mut node,
            _ => {
                kept.push(word);
                continue;
            }
        };
        let value = inline
            .or_else(|| words.next())
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} wants a value"))?;
        *slot = Some(value);
    }
    *args = kept;
    match (media, node) {
        (Some(media), node) => Ok(Some(GraphStart { media, node })),
        (None, Some(_)) => Err("--graph-node wants --graph".to_owned()),
        (None, None) => Ok(None),
    }
}

/// The node the next graph opened on its media chooses, from `--graph-node`.
static START: std::sync::Mutex<Option<GraphStart>> = std::sync::Mutex::new(None);

/// Opens the graph `start` names (with `open`, which puts it over its
/// window) once `session`'s project has said its media, choosing its node,
/// or opening its model's workflow, once the bridge has said the graph.
/// `app::open_graph_when_up` calls it for `--graph` as the window opens.
pub fn open_when_ready(
    start: GraphStart,
    session: &Entity<Session>,
    open: impl Fn(SharedString, &mut App) + 'static,
    cx: &mut App,
) {
    if let Ok(mut pending) = START.lock() {
        *pending = Some(start.clone());
    }
    let opened = std::rc::Rc::new(std::cell::Cell::new(false));
    cx.observe(session, move |session, cx| {
        if opened.get() || session.read(cx).timeline().source(&start.media).is_none() {
            return;
        }
        opened.set(true);
        open(start.media.clone().into(), cx);
    })
    .detach();
}

/// The node `--graph-node` chose for `media`'s graph, taken once.
fn start_node(media: &str) -> Option<String> {
    let mut pending = START.lock().ok()?;
    if pending.as_ref().is_some_and(|start| start.media == media) {
        return pending.take().and_then(|start| start.node);
    }
    None
}

// ------------------------------------------------------------------ cards

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
        "moves" => Icon::Pen,
        "cast" | "style" => Icon::Chat,
        "takes" => Icon::Media,
        _ => Icon::Dot,
    }
}

/// A model's seed, length, size and what else its line sets, as its card's
/// third line says them.
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
    for knob in holds["knobs"].as_array().into_iter().flatten() {
        if !knob["value"].is_null() {
            said.push(format!(
                "{} {}",
                knob["name"].as_str().unwrap_or_default(),
                value_text(&knob["value"])
            ));
        }
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
    tip: Option<String>,
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
        tip: None,
    }
}

/// The fields of a node, by its kind: what its line sets. A guide's Draws
/// is chosen from a menu, and a workflow's values are set by what each is.
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
            // A part of a longer shot says where in its sequences' time it starts.
            if !holds["from"].is_null() {
                wants.push(want("from", "From", &holds["from"], "the sequence's start"));
            }
            for knob in holds["knobs"].as_array().into_iter().flatten() {
                let name = knob["name"].as_str().unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                let comes = value_text(&knob["default"]);
                let tip = if comes.is_empty() {
                    format!("{name}, as the line sets it; emptied, it is taken off the line")
                } else {
                    format!("{name}: {comes} as the model comes; emptied, it is that again")
                };
                wants.push(Want {
                    tip: Some(tip),
                    ..want(name, name, &knob["value"], &comes)
                });
            }
            wants.push(want(ADD, "Add", &Value::Null, "name=value"));
            wants
        }
        "render" => vec![
            want("sequence", "Sequence", &holds["sequence"], ""),
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
        "join" => {
            let lap = holds["lap"].as_u64().filter(|lap| *lap > 0);
            vec![Want {
                value: lap.map(|lap| lap.to_string()).unwrap_or_default(),
                ..want("lap", "Lap", &Value::Null, "0 frames")
            }]
        }
        "part" => holds["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|input| input_kind(input) == InputKind::Value)
            .filter(|input| {
                let held = &input["value"];
                (held.is_string() || held.is_number())
                    && input["choices"]
                        .as_array()
                        .is_none_or(|choices| choices.is_empty())
            })
            .map(|input| {
                let name = input["name"].as_str().unwrap_or_default();
                let held = &input["value"];
                let (least, most) = (&input["min"], &input["max"]);
                let tip =
                    (held.is_number() && (least.is_number() || most.is_number())).then(|| {
                        let end = |end: &Value| {
                            if end.is_number() {
                                shown_value(end)
                            } else {
                                "any".to_owned()
                            }
                        };
                        format!("{} to {}", end(least), end(most))
                    });
                Want {
                    key: name.to_owned(),
                    label: name.to_owned(),
                    value: shown_value(held),
                    placeholder: String::new(),
                    multiline: false,
                    tip,
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// What a guide draws, and its name.
const PASSES: [(&str, &str); 5] = [
    ("picture", "Picture"),
    ("depth", "Depth"),
    ("relative", "Relative depth"),
    ("ids", "Ids"),
    ("normals", "Normals"),
];

/// What a value typed into a workflow's number input sets it to: the
/// number, where it reads as one the server takes there; none otherwise,
/// and the field shows what it was.
pub fn part_number(typed: &str, least: Option<f64>, most: Option<f64>) -> Option<f64> {
    let value: f64 = typed
        .trim()
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite())?;
    if least.is_some_and(|least| value < least) || most.is_some_and(|most| value > most) {
        return None;
    }
    Some(value)
}

/// A prompt as its line holds it: one line, its spaces single.
pub fn one_line(words: &str) -> String {
    words.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A seed a model's sampler takes: a whole number under 2^31.
pub fn another_seed() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()),
    );
    (hasher.finish() % (1u64 << 31)) as u32
}

// ------------------------------------------------------------------ pictures of files

/// A picture of a file the graph shows (a take, the guide a step drew for
/// one, a picture a step reads, the frame a clip is pointed on), made by
/// FFmpeg off the window's thread as the bin's posters are, and kept by the
/// file's path and when it last changed.
enum Still {
    Asked,
    Ready(Arc<RenderImage>),
    Failed,
}

/// What a picture of `path` is kept under: its path and when it last
/// changed, so a take made again, or a frame drawn again, is read again.
fn still_key(path: &Path) -> String {
    let changed = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis());
    format!("{}\u{0}{changed}", path.display())
}

/// How many FFmpegs make a graph's pictures at once, and how long one may
/// run before it is stopped: as the bin's posters are made (artwork.rs),
/// so a graph of many takes neither holds every thread of the background
/// executor nor keeps one for good on a file FFmpeg hangs on.
const STILL_WORKERS: usize = 2;
const STILL_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The files a graph wants pictures of, started a few at a time in the
/// order they were asked for. A file waits once, however often it is asked
/// for while it waits.
#[derive(Debug, Default)]
pub struct PictureQueue {
    waiting: std::collections::VecDeque<PathBuf>,
    running: usize,
    most: usize,
}

impl PictureQueue {
    /// A queue that runs at most `most` at once (at least one).
    pub fn new(most: usize) -> Self {
        Self {
            waiting: std::collections::VecDeque::new(),
            running: 0,
            most: most.max(1),
        }
    }

    /// `path` waits its turn, unless it is waiting already.
    pub fn push(&mut self, path: PathBuf) {
        if !self.waiting.contains(&path) {
            self.waiting.push_back(path);
        }
    }

    /// The next file to start, while fewer than the most run: it is
    /// counted as running until [`PictureQueue::ended`].
    pub fn start(&mut self) -> Option<PathBuf> {
        if self.running >= self.most {
            return None;
        }
        let next = self.waiting.pop_front()?;
        self.running += 1;
        Some(next)
    }

    /// One that was started has ended, however it went.
    pub fn ended(&mut self) {
        self.running = self.running.saturating_sub(1);
    }

    /// How many are running.
    pub fn running(&self) -> usize {
        self.running
    }

    /// How many wait.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

/// What `command` writes before it ends, or None when it cannot start or
/// runs past `limit`, when it is stopped. Its standard input is nothing
/// and what it says on its standard error is not kept.
pub fn output_within(
    mut command: std::process::Command,
    limit: std::time::Duration,
) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Mutex, PoisonError};
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for each picture.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().ok()?;
    let Some(mut out) = child.stdout.take() else {
        // Stopped and reaped, if it has not ended already: it is not read.
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let child = Arc::new(Mutex::new(child));
    let stopped = Arc::new(AtomicBool::new(false));
    let (done, finished) = mpsc::channel::<()>();
    let watchdog = {
        let (child, stopped) = (child.clone(), stopped.clone());
        std::thread::spawn(move || {
            if finished.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout) {
                stopped.store(true, Ordering::Relaxed);
                // One that has just ended needs no stopping.
                let _ = child.lock().unwrap_or_else(PoisonError::into_inner).kill();
            }
        })
    };
    let mut written = Vec::new();
    let read = out.read_to_end(&mut written);
    // The watchdog may have returned already, having stopped it; it only
    // kills, which does not panic. The wait only reaps: what it wrote is
    // the answer, however it ended.
    let _ = done.send(());
    let _ = watchdog.join();
    let _ = child.lock().unwrap_or_else(PoisonError::into_inner).wait();
    (read.is_ok() && !stopped.load(Ordering::Relaxed)).then_some(written)
}

/// A picture of the file at `path`, as large as the column shows one at
/// most, twice over for a sharp screen: a still as it is, a picture in
/// motion by a frame that stands for it. None for a file FFmpeg cannot
/// read, or does not read within [`STILL_LIMIT`].
fn decode_still(path: &Path) -> Option<Arc<RenderImage>> {
    let ffmpeg = crate::artwork::ffmpeg_bin();
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let still = matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff"
    );
    let scale = "scale=w=640:h=360:force_original_aspect_ratio=decrease:flags=lanczos";
    let filter = if still {
        scale.to_owned()
    } else {
        format!("{scale},thumbnail")
    };
    let mut command = std::process::Command::new(ffmpeg);
    command
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-an", "-sn", "-vf", &filter, "-frames:v", "1"])
        .args([
            "-f",
            "image2pipe",
            "-c:v",
            "pam",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ]);
    let written = output_within(command, STILL_LIMIT)?;
    let picture = crate::artwork::parse_pam_stream(&written)
        .into_iter()
        .next()?;
    crate::artwork::render_image(&picture)
}

// ------------------------------------------------------------------ the graph's view

actions!(
    shot_graph,
    [FitGraph, ZoomIn, ZoomOut, NodeLeft, NodeRight, NodeUp, NodeDown]
);

/// The key context the canvas answers in, while it has the keys.
const CANVAS: &str = "ShotGraph";

/// The canvas's keys: F or 0 fit the whole graph, + or = and - zoom about
/// its middle, the arrows go to the nearest card that way. Bound again
/// whenever a graph opens and finds them missing (a keymap bound clears them).
fn bind_graph_keys(cx: &mut App) {
    let bound = gpui::Keystroke::parse("f").is_ok_and(|key| {
        cx.all_bindings_for_input(&[key])
            .iter()
            .any(|binding| binding.action().partial_eq(&FitGraph))
    });
    if bound {
        return;
    }
    let at = Some(CANVAS);
    cx.bind_keys([
        KeyBinding::new("f", FitGraph, at),
        KeyBinding::new("0", FitGraph, at),
        KeyBinding::new("=", ZoomIn, at),
        KeyBinding::new("shift-=", ZoomIn, at),
        KeyBinding::new("+", ZoomIn, at),
        KeyBinding::new("-", ZoomOut, at),
        KeyBinding::new("left", NodeLeft, at),
        KeyBinding::new("right", NodeRight, at),
        KeyBinding::new("up", NodeUp, at),
        KeyBinding::new("down", NodeDown, at),
    ]);
}

/// A drag under way on the canvas.
enum Drag {
    None,
    /// The canvas, panned from where it was; `middle`: by the middle
    /// button, from anywhere, cards too.
    Canvas {
        from: Point<Pixels>,
        offset: (f32, f32),
        moved: bool,
        middle: bool,
    },
    /// A card, moved from where it was.
    Card {
        id: String,
        from: Point<Pixels>,
        place: (f32, f32),
        moved: bool,
    },
    /// An edge drawn from a card's port, to the pointer; over a card that
    /// could read it.
    Wire {
        from: String,
        at: (f32, f32),
        over: Option<String>,
    },
    /// A signal's curve, drawn by hand.
    Curve,
}

/// How far reading a clip from footage has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PointStage {
    #[default]
    Idle,
    /// Its first frame is being drawn.
    Drawing,
    /// It is drawn, to be pointed on.
    Pointing,
    /// What was pointed at is followed through the clip.
    Following,
    /// The bridge said how that went.
    Said,
}

/// Reading a clip from footage: which clip, for its way (`moves`) or its
/// matte (`matte`); the picture pointed on; the points on the thing and
/// off it, as shares of the picture; what the bridge said.
#[derive(Clone, Debug, Default)]
struct Pointing {
    clip: String,
    of: Option<ShotOf>,
    stage: PointStage,
    picture: Option<PathBuf>,
    on: Vec<(f64, f64)>,
    off: Vec<(f64, f64)>,
    says: String,
    failed: bool,
}

/// A signal's curve being drawn, or drawn and not yet heard back.
struct Drawing {
    clip: String,
    frames: usize,
    drawn: BTreeMap<usize, f64>,
    last: Option<usize>,
    /// The pointer is still down.
    held: bool,
}

/// What opens over the graph and gives it back as it was: a making's plan,
/// Cast and Styles, Settings at its Models.
enum Over {
    Plan(Entity<MakePlan>),
    Cast(Entity<crate::cast::CastView>),
    Settings(Entity<crate::settings::SettingsView>),
}

/// Which menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuKind {
    Several,
    AddStep,
    Port,
    Pass,
    Sound,
    Choice,
}

impl MenuKind {
    /// How wide its menu is.
    fn width(self) -> f32 {
        match self {
            MenuKind::Port => 420.0,
            MenuKind::Several | MenuKind::Pass => 220.0,
            MenuKind::AddStep | MenuKind::Sound | MenuKind::Choice => 240.0,
        }
    }
}

struct GraphMenu {
    kind: MenuKind,
    menu: Entity<ui::Menu>,
    at: Point<Pixels>,
    _closed: Subscription,
}

/// What a field sets: a name on its line, a `name=value` added to it, or a
/// value of a node of a model's workflow (a number held to what the
/// server takes there).
#[derive(Clone, Debug, PartialEq)]
enum FieldKind {
    Line,
    Add,
    Part {
        node: String,
        number: bool,
        least: Option<f64>,
        most: Option<f64>,
    },
}

/// A field of the chosen node, the line it was made for, and what it last
/// showed of the steps: what is typed is set as the person leaves it, on
/// that line, whatever node has been chosen since.
struct Field {
    key: String,
    label: String,
    line: usize,
    shown: String,
    input: Entity<TextInput>,
    multiline: bool,
    kind: FieldKind,
    tip: Option<String>,
}

/// The chosen node's fields.
struct Inspecting {
    /// The node they are of.
    node: String,
    /// The node, its line and its fields' names: the same shape keeps its fields.
    shape: String,
    fields: Vec<Field>,
    _subscriptions: Vec<Subscription>,
}

/// What a field's commit does, worked out before the graph is changed.
enum Commit {
    Nothing,
    Line {
        line: usize,
        key: String,
        value: String,
    },
    Part {
        node: String,
        input: String,
        value: Value,
    },
    Revert(String),
}

/// Holding the takes to their guide.
#[derive(Clone, Debug, PartialEq)]
enum Holding {
    Idle,
    Under,
    Read,
    Failed(String),
}

/// An edge as it is drawn on the canvas.
struct EdgeLine {
    start: (f32, f32),
    end: (f32, f32),
    color: Rgba,
    dashed: bool,
}

/// Where an edge leaves and where it arrives, in the view, and its color:
/// the rings drawn there over the cards' edges.
type EdgeEnds = ((f32, f32), (f32, f32), Rgba);

/// A shot's graph, over the window: its steps and what they read, wired
/// left to right on a dotted canvas, a chosen node's settings at the right,
/// and its making and takes. A model run with kerf-comfy opens into its
/// ComfyUI workflow, on the same canvas.
pub struct ShotGraph {
    session: Entity<Session>,
    media: SharedString,
    focus_handle: FocusHandle,
    /// The canvas's own, for its keys.
    canvas_focus: FocusHandle,
    recipe: Option<Recipe>,
    /// The model's workflow shown in place of the shot's graph.
    workflow: Option<Workflow>,
    /// The line of the model whose workflow is asked for or shown; 0 for none.
    workflow_line: usize,
    /// Why the graph could not be read, before any was.
    refused: Option<String>,
    /// Why the last change was refused.
    said: Option<String>,
    /// The revision the graph was last asked for at.
    asked: Option<u64>,
    /// What the shot's cards are kept under, by its project and media.
    shot_key: String,
    /// Where the person put a card, by its node, over where it was laid out,
    /// in what is shown (the shot's graph or its model's workflow).
    placed: HashMap<String, (f32, f32)>,
    camera: Camera,
    /// The canvas looks at all of the graph, and goes on doing so as the
    /// canvas's size or the nodes change, until it is panned or zoomed.
    fitted: bool,
    /// Fit asked for all of it, however small.
    whole: bool,
    /// The canvas's size, as last drawn.
    room: (f32, f32),
    drag: Drag,
    hovered_port: Option<String>,
    selected: Option<String>,
    inspecting: Option<Inspecting>,
    /// The field that had the keys when the graph was last drawn: left
    /// since, what it says is set.
    typing_in: Option<String>,
    menu: Option<GraphMenu>,
    /// Each take's reading beside its guide, the whole of what was said.
    readings: HashMap<u32, TakeRead>,
    best: Option<u32>,
    reading: Holding,
    pointing: Pointing,
    drawing: Option<Drawing>,
    /// Each file's picture, under the key of the file as it was read.
    stills: HashMap<PathBuf, (String, Still)>,
    /// The files whose pictures are asked for and not yet being made.
    queue: PictureQueue,
    /// Pictures let go of, for the window to take out of its atlas.
    dropped: Vec<Arc<RenderImage>>,
    over: Option<Over>,
    _over: Option<Subscription>,
    /// What `--graph-node` chose, once the graph is read.
    pending_node: Option<String>,
    /// Where the canvas, a signal's curve and a picture pointed on were last drawn.
    canvas_bounds: Rc<Cell<Bounds<Pixels>>>,
    curve_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The inspector's column, for its fades.
    inspector_scroll: gpui::ScrollHandle,
    point_bounds: Rc<Cell<Bounds<Pixels>>>,
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
        bind_graph_keys(cx);
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &session,
                window,
                |this, _, event: &BridgeEvent, window, cx| this.heard(&event.0, window, cx),
            ),
            // Its pictures go out of the window's atlas with it: each graph
            // opened makes its own again.
            cx.on_release(|graph, cx| {
                for image in graph.let_go_of_pictures() {
                    cx.drop_image(image, None);
                }
            }),
        ];
        let shot_key = placement_key(session.read(cx).project.as_deref(), &media);
        let placed = load_places(&session, &shot_key, cx);
        let pending_node = start_node(&media);
        let canvas_focus = cx.focus_handle();
        let mut graph = Self {
            session,
            media,
            focus_handle: cx.focus_handle(),
            canvas_focus,
            recipe: None,
            workflow: None,
            workflow_line: 0,
            refused: None,
            said: None,
            asked: None,
            shot_key,
            placed,
            camera: Camera::default(),
            fitted: true,
            whole: false,
            room: (0.0, 0.0),
            drag: Drag::None,
            hovered_port: None,
            selected: None,
            inspecting: None,
            typing_in: None,
            menu: None,
            readings: HashMap::new(),
            best: None,
            reading: Holding::Idle,
            pointing: Pointing::default(),
            drawing: None,
            stills: HashMap::new(),
            queue: PictureQueue::new(STILL_WORKERS),
            dropped: Vec::new(),
            over: None,
            _over: None,
            pending_node,
            canvas_bounds: Rc::default(),
            curve_bounds: Rc::default(),
            inspector_scroll: gpui::ScrollHandle::new(),
            point_bounds: Rc::default(),
            _subscriptions: subscriptions,
        };
        graph.request(cx);
        graph
    }

    fn request(&mut self, cx: &mut Context<Self>) {
        let media = self.media.to_string();
        let revision = self.session.update(cx, |session, _| {
            session.send(Command::RecipeGet { media });
            session.revision()
        });
        self.asked = Some(revision);
    }

    fn request_workflow(&mut self, cx: &mut Context<Self>) {
        if self.workflow_line == 0 {
            return;
        }
        let command = Command::WorkflowGet {
            media: self.media.to_string(),
            line: self.workflow_line,
        };
        self.session.update(cx, |session, _| session.send(command));
    }

    fn heard(&mut self, event: &Event, window: &mut Window, cx: &mut Context<Self>) {
        // What is said of this shot's media; a recipe and a workflow say it among their fields.
        let media = match event {
            Event::Recipe(recipe) => recipe.graph.get("media").and_then(Value::as_str),
            Event::Workflow(fields) => fields.get("media").and_then(Value::as_str),
            Event::Error(error) => error.media.as_deref(),
            Event::TakeRead(read) => Some(read.media.as_str()),
            Event::TakesRead(read) => Some(read.media.as_str()),
            Event::MediaMade { media, .. } | Event::MediaMakeCancelled { media } => {
                Some(media.as_str())
            }
            _ => None,
        };
        let ours = media == Some(self.media.as_str());
        match event {
            Event::Recipe(recipe) if ours => self.set_recipe(recipe, window, cx),
            Event::Workflow(fields) if ours => {
                let line = fields.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
                if line > 0 && line == self.workflow_line {
                    self.set_workflow(Workflow::read(fields), window, cx);
                }
            }
            Event::Error(error) => {
                let command = error.command.as_deref().unwrap_or_default();
                let message = error.message.clone();
                match command {
                    "workflow" if ours => {
                        self.said = Some(message);
                        // Asked for and refused: the shot's graph stays, and its
                        // model's Open workflow asks again.
                        if self.workflow.is_none() {
                            self.workflow_line = 0;
                        }
                    }
                    "recipe" if ours => {
                        if self.recipe.is_some() {
                            self.said = Some(message);
                        } else {
                            self.refused = Some(message);
                        }
                    }
                    "takes_read" if ours => self.reading = Holding::Failed(message),
                    _ if command.starts_with("shot_") => {
                        self.followed(error.clip.as_deref(), error.of, message, true)
                    }
                    "media_make" if ours => self.request(cx),
                    _ => return,
                }
            }
            Event::TakeRead(read) if ours => {
                self.readings.insert(read.take, read.clone());
            }
            Event::TakesRead(read) if ours => {
                self.best = read.best.filter(|best| *best > 0);
                self.reading = Holding::Read;
            }
            // A clip read from footage: the card that asked knows its clip.
            Event::ShotPicture { clip, of, path, .. } => {
                let pointing = &mut self.pointing;
                let asked = *clip == pointing.clip
                    && pointing.of == Some(*of)
                    && pointing.stage == PointStage::Drawing;
                if !asked {
                    return;
                }
                pointing.picture = Some(path.clone());
                pointing.stage = PointStage::Pointing;
                self.want_pictures(cx);
            }
            Event::ShotFollowed { clip, of, says, .. } => {
                self.followed(Some(clip.as_str()), Some(*of), says.clone(), false);
            }
            // Its takes are other than they were.
            Event::MediaMade { .. } | Event::MediaMakeCancelled { .. } if ours => self.request(cx),
            // The project changed: a signal's keys, a clip's way, a take chosen.
            Event::State { snapshot } => {
                let revision = Some(snapshot.revision);
                if revision != self.asked {
                    self.request(cx);
                    if self.workflow.is_some() {
                        self.request_workflow(cx);
                    }
                }
            }
            Event::Models { .. } | Event::Cast(_) => self.request(cx),
            Event::Connection { .. } if self.recipe.is_none() => self.request(cx),
            _ => return,
        }
        cx.notify();
    }

    /// What the bridge said of following a clip, or why it could not: a
    /// picture that could not be drawn leaves nothing to point on; a thing
    /// that could not be followed leaves the picture, to point again. The
    /// clip and what it was read for are as the bridge named them, if it did.
    fn followed(&mut self, clip: Option<&str>, of: Option<ShotOf>, says: String, failed: bool) {
        let pointing = &mut self.pointing;
        let named = clip.is_some();
        let ours = clip == Some(pointing.clip.as_str()) && of.is_some() && of == pointing.of;
        let waiting = matches!(pointing.stage, PointStage::Drawing | PointStage::Following);
        if !(ours || (!named && waiting)) {
            return;
        }
        pointing.stage = if failed && pointing.picture.is_some() {
            PointStage::Pointing
        } else {
            PointStage::Said
        };
        pointing.says = says;
        pointing.failed = failed;
        pointing.on.clear();
        pointing.off.clear();
    }

    /// The graph shown: the model's workflow, or the shot's.
    fn shown(&self) -> Option<&Recipe> {
        match &self.workflow {
            Some(workflow) => Some(&workflow.graph),
            None => self.recipe.as_ref(),
        }
    }

    /// What the cards shown are kept under.
    fn places_key(&self) -> String {
        match &self.workflow {
            Some(workflow) => workflow_key(&self.shot_key, &workflow.name),
            None => self.shot_key.clone(),
        }
    }

    fn places(&self) -> HashMap<String, (f32, f32)> {
        self.shown()
            .map(|graph| placed_over(graph, &self.placed))
            .unwrap_or_default()
    }

    /// The canvas looks at all of what is shown, or where a graph too large
    /// to read whole starts.
    fn settle(&mut self) {
        let extent = self.shown().and_then(|graph| extent(graph, &self.places()));
        self.camera = settle(extent, self.room, self.whole);
        self.fitted = true;
    }

    fn fit(&mut self, cx: &mut Context<Self>) {
        self.whole = true;
        self.settle();
        cx.notify();
    }

    fn zoom_by(&mut self, factor: f32, about: (f32, f32), cx: &mut Context<Self>) {
        let camera = zoom_about(self.camera, factor, about);
        if camera != self.camera {
            self.camera = camera;
            self.fitted = false;
            cx.notify();
        }
    }

    /// The middle of the canvas.
    fn middle(&self) -> (f32, f32) {
        (self.room.0 / 2.0, self.room.1 / 2.0)
    }

    /// A point of the window on the canvas.
    fn local(&self, at: Point<Pixels>) -> (f32, f32) {
        let origin = self.canvas_bounds.get().origin;
        (f32::from(at.x - origin.x), f32::from(at.y - origin.y))
    }

    fn set_recipe(&mut self, event: &RecipeEvent, window: &mut Window, cx: &mut Context<Self>) {
        let recipe = Recipe::read(event);
        let ids = |recipe: &Recipe| -> Vec<String> {
            recipe.nodes.iter().map(|node| node.id.clone()).collect()
        };
        let before = self.recipe.as_ref().map(ids).unwrap_or_default();
        let now = ids(&recipe);
        // A recipe's next version is the same shot, by its name: the view stays.
        let another = self
            .recipe
            .as_ref()
            .is_none_or(|old| old.media != recipe.media || old.name != recipe.name);
        let edited = event.edited;
        // A step just added is the one to set next.
        if let Some(line) = event.added {
            if let Some(node) = recipe.nodes.iter().find(|node| node.line == Some(line)) {
                self.selected = Some(node.id.clone());
            }
        }
        let pending = self.pending_node.take();
        self.recipe = Some(recipe);
        self.refused = None;
        self.want_pictures(cx);
        // A curve drawn is the project's now, and drawn as it says.
        if self.drawing.as_ref().is_some_and(|drawing| !drawing.held) {
            self.drawing = None;
        }
        if let Some(node) = pending {
            match node
                .strip_prefix("workflow:")
                .map(|line| line.parse::<usize>())
            {
                Some(Ok(line)) => self.open_workflow(line, window, cx),
                Some(Err(_)) => {}
                None => self.selected = Some(node),
            }
        }
        // While a workflow is only asked for, the shot's graph is the one
        // shown, and follows its steps as ever.
        if self.workflow.is_some() {
            // The line a workflow is of was changed: what fills its blanks,
            // and which file it is, are read again.
            if edited {
                self.request_workflow(cx);
            }
            return;
        }
        if self
            .selected
            .as_deref()
            .is_some_and(|id| self.recipe.as_ref().and_then(|r| r.node(id)).is_none())
        {
            self.selected = None;
        }
        if another {
            self.fitted = true;
            self.whole = false;
        }
        if self.fitted && (another || before != now) {
            self.settle();
        }
        self.inspect(window, cx);
    }

    fn set_workflow(&mut self, workflow: Workflow, window: &mut Window, cx: &mut Context<Self>) {
        let ids = |graph: &Recipe| -> Vec<String> {
            graph.nodes.iter().map(|node| node.id.clone()).collect()
        };
        let before = self.workflow.as_ref().map(|old| ids(&old.graph));
        let another = self
            .workflow
            .as_ref()
            .is_none_or(|old| old.line != workflow.line || old.name != workflow.name);
        let changed = before.is_none_or(|before| before != ids(&workflow.graph));
        // Another workflow takes the fields' place, so what they say is set
        // first, on what they were made for. The same one said again (what
        // the server takes, read late; the project's next revision) leaves
        // a field being typed in to the person until they leave it.
        if another {
            self.commit_fields(cx);
        }
        self.workflow = Some(workflow);
        if another {
            self.selected = None;
            self.placed = load_places(&self.session, &self.places_key(), cx);
            self.fitted = true;
            self.whole = false;
        }
        if self
            .selected
            .as_deref()
            .is_some_and(|id| self.shown().and_then(|graph| graph.node(id)).is_none())
        {
            self.selected = None;
        }
        if self.fitted && (another || changed) {
            self.settle();
        }
        self.inspect(window, cx);
    }

    /// Goes into the workflow of the model on `line`: asked for, and shown
    /// on the canvas once the bridge says it.
    fn open_workflow(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        if line == 0 {
            return;
        }
        self.commit_fields(cx);
        self.workflow_line = line;
        self.said = None;
        self.request_workflow(cx);
        window.focus(&self.canvas_focus);
        cx.notify();
    }

    /// Back from a model's workflow to the shot's graph, on the model it is of.
    fn close_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let line = std::mem::take(&mut self.workflow_line);
        self.workflow = None;
        self.said = None;
        self.placed = load_places(&self.session, &self.shot_key, cx);
        self.selected = self.recipe.as_ref().and_then(|recipe| {
            recipe
                .nodes
                .iter()
                .find(|node| node.kind == "model" && node.line == Some(line))
                .map(|node| node.id.clone())
        });
        self.fitted = true;
        self.whole = false;
        self.settle();
        self.inspect(window, cx);
        window.focus(&self.canvas_focus);
        cx.notify();
    }

    /// The graph gives the window back, what was typed set first.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        cx.emit(DismissEvent);
    }

    /// A change to the steps, as they read when the graph was drawn.
    fn edit(&mut self, change: RecipeEdit, cx: &mut Context<Self>) {
        let Some(recipe) = &self.recipe else {
            return;
        };
        let change = RecipeEdit {
            media: self.media.to_string(),
            was: recipe.text.clone(),
            ..change
        };
        self.said = None;
        self.session
            .update(cx, |session, _| session.send(Command::RecipeEdit(change)));
        cx.notify();
    }

    /// A change to the model's workflow, kept in the shot's own copy of it.
    fn edit_workflow(&mut self, change: WorkflowEdit, cx: &mut Context<Self>) {
        let Some(workflow) = &self.workflow else {
            return;
        };
        let change = WorkflowEdit {
            media: self.media.to_string(),
            line: workflow.line,
            was: workflow.text.clone(),
            ..change
        };
        self.said = None;
        self.session
            .update(cx, |session, _| session.send(Command::WorkflowEdit(change)));
        cx.notify();
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != id {
            // What is typed sets its own node's line as it gives way.
            self.commit_fields(cx);
            self.selected = id;
            self.said = None;
            self.inspect(window, cx);
        }
        cx.notify();
    }

    /// The chosen node's fields: made again for another node, or another
    /// shape of the same, else set to what the steps now say, except where
    /// a person is typing.
    ///
    /// Nothing is set here. A person's own way to another node (choosing
    /// it, going back from a workflow) sets what the fields say before it
    /// comes here; what comes here from the bridge (the steps or the
    /// workflow said again) may have moved the node to another line, or
    /// taken it out, so what was typed for it is not set on the line it was
    /// typed for: it goes on in the node's new fields, under the same
    /// names, to be set on the node's line as the person leaves it.
    fn inspect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self
            .selected
            .as_ref()
            .and_then(|id| self.shown()?.node(id))
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
                if field.kind == FieldKind::Add {
                    continue;
                }
                let focused = field.input.read(cx).focus_handle(cx).is_focused(window);
                let typing = focused && typed(field, cx) != field.shown;
                field.shown = want.value.clone();
                // What is being typed is the person's until they leave it.
                if !typing && field.input.read(cx).text() != want.value {
                    let value = want.value.clone();
                    field
                        .input
                        .update(cx, |input, cx| input.set_text(value, cx));
                }
            }
            return;
        }
        let carried = self
            .inspecting
            .take()
            .filter(|old| old.node == node.id)
            .map(|old| carried_typing(&old.fields, window, cx))
            .unwrap_or_default();
        let part_node = node.holds["id"].as_str().map(str::to_owned);
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for want in wants {
            let kind = if want.key == ADD {
                FieldKind::Add
            } else if let Some(part) = part_node.as_ref().filter(|_| node.is_part()) {
                let input = node.holds["inputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|input| input["name"] == want.key.as_str())
                    .cloned()
                    .unwrap_or_default();
                FieldKind::Part {
                    node: part.clone(),
                    number: input["value"].is_number(),
                    least: input["min"].as_f64(),
                    most: input["max"].as_f64(),
                }
            } else {
                FieldKind::Line
            };
            let (says, focused) = match carried.get(&want.key) {
                Some((typed, focused)) => (typed.clone(), *focused),
                None => (want.value.clone(), false),
            };
            let input = cx.new(|cx| {
                let input = TextInput::new(cx, want.placeholder.clone());
                let mut input = if want.multiline {
                    input.multiline().max_height(px(88.0))
                } else {
                    input.with_style(text::TIMECODE)
                };
                input.set_text(says, cx);
                input
            });
            if focused {
                window.focus(&input.read(cx).focus_handle(cx));
            }
            let key = want.key.clone();
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.commit(&key, cx),
                    // Esc puts back what the steps say, and leaves the field.
                    InputEvent::Cancel => {
                        this.revert(&key, cx);
                        window.focus(&this.canvas_focus);
                    }
                    // What is pasted is one line, as the step's line is.
                    InputEvent::Changed => {
                        if key == "prompt" && input.read(cx).text().contains('\n') {
                            let one = one_line(input.read(cx).text());
                            input.update(cx, |input, cx| input.set_text(one, cx));
                        }
                    }
                },
            ));
            let focus = input.read(cx).focus_handle(cx);
            let key = want.key.clone();
            subscriptions
                .push(cx.on_blur(&focus, window, move |this, _, cx| this.commit(&key, cx)));
            fields.push(Field {
                key: want.key,
                label: want.label,
                line,
                shown: want.value,
                input,
                multiline: want.multiline,
                kind,
                tip: want.tip,
            });
        }
        self.inspecting = Some(Inspecting {
            node: node.id.clone(),
            shape,
            fields,
            _subscriptions: subscriptions,
        });
    }

    /// A field left since the graph was last drawn, by Tab, a click
    /// elsewhere or Esc, sets what it says: followed as the graph is drawn,
    /// so that it holds whether the window is the active one or not.
    fn follow_typing(&mut self, window: &Window, cx: &mut Context<Self>) {
        let focused = self.inspecting.as_ref().and_then(|inspecting| {
            inspecting
                .fields
                .iter()
                .find(|field| field.input.read(cx).focus_handle(cx).is_focused(window))
                .map(|field| field.key.clone())
        });
        if self.typing_in != focused {
            if let Some(left) = self.typing_in.take() {
                self.commit(&left, cx);
            }
            self.typing_in = focused;
        }
    }

    /// Sets what was typed in each field the person has not left yet.
    fn commit_fields(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<String> = self
            .inspecting
            .as_ref()
            .map(|inspecting| {
                inspecting
                    .fields
                    .iter()
                    .map(|field| field.key.clone())
                    .collect()
            })
            .unwrap_or_default();
        for key in keys {
            self.commit(&key, cx);
        }
    }

    /// Field `key` set to what it says, on the line it was made for: the
    /// name taken off the line when it says nothing; a `name=value` added;
    /// a workflow's value set, a number only where the server takes it.
    fn commit(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(field) = self
            .inspecting
            .as_mut()
            .and_then(|inspecting| inspecting.fields.iter_mut().find(|field| field.key == key))
        else {
            return;
        };
        let now = typed(field, cx);
        let commit = match &field.kind {
            FieldKind::Add => {
                let Some((name, value)) = now.split_once('=') else {
                    return;
                };
                let (name, value) = (name.trim().to_owned(), value.trim().to_owned());
                if name.is_empty() || field.line == 0 {
                    return;
                }
                field.input.update(cx, |input, cx| input.clear(cx));
                if value.is_empty() {
                    Commit::Nothing
                } else {
                    Commit::Line {
                        line: field.line,
                        key: name,
                        value,
                    }
                }
            }
            _ if now == field.shown => Commit::Nothing,
            FieldKind::Line if field.line == 0 => Commit::Nothing,
            FieldKind::Line => {
                field.shown = now.clone();
                Commit::Line {
                    line: field.line,
                    key: field.key.clone(),
                    value: now,
                }
            }
            FieldKind::Part {
                node,
                number,
                least,
                most,
            } => {
                if !number {
                    field.shown = now.clone();
                    Commit::Part {
                        node: node.clone(),
                        input: field.key.clone(),
                        value: json!(now),
                    }
                } else {
                    match part_number(&now, *least, *most) {
                        Some(value) => {
                            field.shown = now;
                            let value = if value.fract() == 0.0 && value.abs() < 1e15 {
                                json!(value as i64)
                            } else {
                                json!(value)
                            };
                            Commit::Part {
                                node: node.clone(),
                                input: field.key.clone(),
                                value,
                            }
                        }
                        // A number stays a number, and one the server does
                        // not take there is not sent to be refused.
                        None => Commit::Revert(field.shown.clone()),
                    }
                }
            }
        };
        match commit {
            Commit::Nothing => {}
            Commit::Line { line, key, value } => {
                // The picture is what a guide draws when its line names no pass.
                if value.is_empty() {
                    self.edit(
                        RecipeEdit {
                            line,
                            key,
                            nth: 0,
                            ..RecipeEdit::default()
                        },
                        cx,
                    );
                } else {
                    self.edit(
                        RecipeEdit {
                            line,
                            key,
                            nth: 0,
                            value: Some(value),
                            ..RecipeEdit::default()
                        },
                        cx,
                    );
                }
            }
            Commit::Part { node, input, value } => self.edit_workflow(
                WorkflowEdit {
                    node,
                    input,
                    value,
                    ..WorkflowEdit::default()
                },
                cx,
            ),
            Commit::Revert(was) => {
                if let Some(field) = self.field(key) {
                    field.input.update(cx, |input, cx| input.set_text(was, cx));
                }
            }
        }
    }

    /// Field `key` back to what the steps say.
    fn revert(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(field) = self.field(key) {
            let shown = if field.kind == FieldKind::Add {
                String::new()
            } else {
                field.shown.clone()
            };
            field
                .input
                .update(cx, |input, cx| input.set_text(shown, cx));
        }
    }

    fn field(&self, key: &str) -> Option<&Field> {
        self.inspecting
            .as_ref()?
            .fields
            .iter()
            .find(|field| field.key == key)
    }

    /// Another seed, written at once.
    fn roll_seed(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.field("seed") else {
            return;
        };
        let rolled = another_seed().to_string();
        field
            .input
            .update(cx, |input, cx| input.set_text(rolled, cx));
        self.commit("seed", cx);
    }

    fn press_canvas(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        // A double-click on the canvas takes in the whole graph.
        if event.click_count >= 2 {
            self.drag = Drag::None;
            self.fit(cx);
            return;
        }
        self.drag = Drag::Canvas {
            from: event.position,
            offset: self.camera.offset,
            moved: false,
            middle: false,
        };
        cx.notify();
    }

    /// The middle button pressed anywhere on the canvas, over a card too:
    /// a drag pans it; let go, nothing is chosen.
    fn press_middle(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Middle || !matches!(self.drag, Drag::None) {
            return;
        }
        window.focus(&self.canvas_focus);
        self.drag = Drag::Canvas {
            from: event.position,
            offset: self.camera.offset,
            moved: false,
            middle: true,
        };
        cx.notify();
    }

    /// The middle button let go: its pan ends.
    fn middle_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.drag, Drag::Canvas { middle: true, .. }) {
            self.drag = Drag::None;
            cx.notify();
        }
    }

    fn press_card(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        // A double-click goes into a model that has a workflow behind it.
        if event.click_count >= 2 {
            let line = self
                .shown()
                .and_then(|graph| graph.node(id))
                .filter(|node| node.has_workflow() && self.workflow.is_none())
                .and_then(|node| node.line);
            if let Some(line) = line {
                self.drag = Drag::None;
                self.open_workflow(line, window, cx);
                return;
            }
        }
        let place = self.places().get(id).copied().unwrap_or((0.0, 0.0));
        self.drag = Drag::Card {
            id: id.to_owned(),
            from: event.position,
            place,
            moved: false,
        };
        self.select(Some(id.to_owned()), window, cx);
    }

    fn press_port(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        self.drag = Drag::Wire {
            from: id.to_owned(),
            at: self.local(event.position),
            over: None,
        };
        cx.notify();
    }

    /// The card under a point of the canvas, the topmost.
    fn card_at(&self, at: (f32, f32)) -> Option<String> {
        let graph = self.shown()?;
        let places = self.places();
        graph.nodes.iter().rev().find_map(|node| {
            let [x, y, width, height] = card_rect(graph, &places, &node.id)?;
            let (left, top) = self.camera.to_view((x, y));
            let zoom = self.camera.zoom;
            let inside = at.0 >= left
                && at.0 <= left + width * zoom
                && at.1 >= top
                && at.1 <= top + height * zoom;
            inside.then(|| node.id.clone())
        })
    }

    fn on_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.drag, Drag::None) {
            return;
        }
        let held = if matches!(self.drag, Drag::Canvas { middle: true, .. }) {
            MouseButton::Middle
        } else {
            MouseButton::Left
        };
        if event.pressed_button != Some(held) {
            self.drag = Drag::None;
            if let Some(drawing) = self.drawing.as_mut() {
                drawing.held = false;
            }
            cx.notify();
            return;
        }
        let local = self.local(event.position);
        let zoom = self.camera.zoom.max(0.01);
        let over_card = match &self.drag {
            Drag::Wire { from, .. } => {
                let from = from.clone();
                self.card_at(local).filter(|over| {
                    let graph = self.shown();
                    let (Some(from), Some(to)) = (
                        graph.and_then(|graph| graph.node(&from)),
                        graph.and_then(|graph| graph.node(over)),
                    ) else {
                        return false;
                    };
                    !ports_for(from, to).is_empty()
                })
            }
            _ => None,
        };
        match &mut self.drag {
            Drag::None => {}
            Drag::Canvas {
                from,
                offset,
                moved,
                ..
            } => {
                let (dx, dy) = (
                    f32::from(event.position.x - from.x),
                    f32::from(event.position.y - from.y),
                );
                if !*moved && dx.hypot(dy) < DRAG_FROM {
                    return;
                }
                *moved = true;
                self.camera.offset = (offset.0 + dx, offset.1 + dy);
                self.fitted = false;
            }
            Drag::Card {
                id,
                from,
                place,
                moved,
            } => {
                let (dx, dy) = (
                    f32::from(event.position.x - from.x),
                    f32::from(event.position.y - from.y),
                );
                if !*moved && dx.hypot(dy) < DRAG_FROM {
                    return;
                }
                *moved = true;
                // A card goes where it is dragged, on whole points.
                self.placed.insert(
                    id.clone(),
                    ((place.0 + dx / zoom).round(), (place.1 + dy / zoom).round()),
                );
            }
            Drag::Wire { at, over, .. } => {
                *at = local;
                *over = over_card;
            }
            Drag::Curve => self.draw_curve(event.position),
        }
        cx.notify();
    }

    fn on_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::None => return,
            // A press on the canvas that did not pan it chooses nothing: the shot itself.
            Drag::Canvas {
                moved: false,
                middle: false,
                ..
            } => self.select(None, window, cx),
            Drag::Canvas { .. } => {}
            Drag::Card { moved: true, .. } => {
                save_places(&self.session, &self.places_key(), &self.placed, cx)
            }
            Drag::Card { .. } => {}
            Drag::Wire {
                from,
                over: Some(to),
                ..
            } => self.wire(&from, &to, event.position, window, cx),
            Drag::Wire { .. } => {}
            Drag::Curve => self.drawn(cx),
        }
        cx.notify();
    }

    /// An edge drawn from `from` let go over `to`: what it gives is written
    /// on `to`'s line under the name it reads it by, asked first where
    /// several would; in a workflow, an input of `to` re-pointed at it.
    fn wire(
        &mut self,
        from: &str,
        to: &str,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(graph) = self.shown() else {
            return;
        };
        let (Some(from), Some(to)) = (graph.node(from).cloned(), graph.node(to).cloned()) else {
            return;
        };
        let ports = ports_for(&from, &to);
        if ports.is_empty() {
            return;
        }
        let this = cx.weak_entity();
        if from.is_part() {
            if let [port] = ports.as_slice() {
                self.link(&from, &to, port, cx);
                return;
            }
            let items = ports
                .iter()
                .map(|port| {
                    let (this, from, to, port) =
                        (this.clone(), from.clone(), to.clone(), port.clone());
                    ui::MenuItem::new(port.clone()).on_choose(move |_, cx| {
                        let _ = this.update(cx, |graph, cx| graph.link(&from, &to, &port, cx));
                    })
                })
                .collect();
            let header = format!("{} reads {} at", to.title, from.title);
            self.open_menu(MenuKind::Port, items, Some(header), at, window, cx);
            return;
        }
        let value = value_of(&from);
        if value.is_empty() {
            return;
        }
        if let [port] = ports.as_slice() {
            self.read_port(&to.id, port, &value, cx);
            return;
        }
        // More than one name would read it: which is the person's to say.
        let said_reads = to.holds.get("reads").is_some_and(|reads| !reads.is_null());
        let items = ports
            .iter()
            .map(|port| {
                // What the model has said it has no use for is still the
                // line's to say, and is said so.
                let says = if said_reads && !strings(&to.holds["reads"]).contains(port) {
                    "the model has no use for it".to_owned()
                } else {
                    port_says(port)
                };
                let (this, to, port, value) =
                    (this.clone(), to.id.clone(), port.clone(), value.clone());
                ui::MenuItem::new(port.clone())
                    .key(says)
                    .on_choose(move |_, cx| {
                        let _ =
                            this.update(cx, |graph, cx| graph.read_port(&to, &port, &value, cx));
                    })
            })
            .collect();
        let header = format!("{} reads {} as", to.title, from.title);
        self.open_menu(MenuKind::Port, items, Some(header), at, window, cx);
    }

    /// `to`'s line given `value` under `port`: replaced, or one more where
    /// the name is one a line may give again.
    fn read_port(&mut self, to: &str, port: &str, value: &str, cx: &mut Context<Self>) {
        let Some(recipe) = &self.recipe else {
            return;
        };
        let Some(line) = recipe.node(to).and_then(|node| node.line) else {
            return;
        };
        let nth = wire_nth(recipe, to, port);
        self.edit(
            RecipeEdit {
                line,
                key: port.to_owned(),
                nth,
                value: Some(value.to_owned()),
                ..RecipeEdit::default()
            },
            cx,
        );
    }

    /// Input `input` of workflow node `to` re-pointed at `from`'s output
    /// that gives what it takes.
    fn link(&mut self, from: &GraphNode, to: &GraphNode, input: &str, cx: &mut Context<Self>) {
        let output = link_output(from, to, input);
        let node = value_text(&to.holds["id"]);
        let source = value_text(&from.holds["id"]);
        self.edit_workflow(
            WorkflowEdit {
                node,
                input: input.to_owned(),
                link: Some((source, output as u64)),
                ..WorkflowEdit::default()
            },
            cx,
        );
    }

    fn open_menu(
        &mut self,
        kind: MenuKind,
        items: Vec<ui::MenuItem>,
        header: Option<String>,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = kind.width();
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(width);
            match header {
                Some(header) => menu.header(header),
                None => menu,
            }
        });
        let closed = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some(GraphMenu {
            kind,
            menu,
            at,
            _closed: closed,
        });
        cx.notify();
    }

    /// Puts away the menu open, if any: true when it was of `kind`, which a
    /// second press on what opened it puts away and opens no other.
    fn close_menu(&mut self, kind: MenuKind, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.menu.take() else {
            return false;
        };
        open.menu.update(cx, |menu, cx| menu.close(window, cx));
        cx.notify();
        open.kind == kind
    }

    /// The head's menus open from a press, and a second press puts them away.
    fn toggle_menu(
        &mut self,
        kind: MenuKind,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_menu(kind, window, cx) {
            return;
        }
        let this = cx.weak_entity();
        let (items, header): (Vec<ui::MenuItem>, &str) = match kind {
            MenuKind::Several => (
                // Several takes of the same shot to choose among, each with its seeds one on.
                [2u32, 3, 4]
                    .into_iter()
                    .map(|rolls| {
                        let this = this.clone();
                        ui::MenuItem::new(format!("{rolls} Takes…")).on_choose(move |window, cx| {
                            let _ = this.update(cx, |graph, cx| graph.open_plan(rolls, window, cx));
                        })
                    })
                    .collect(),
                "Each seed one on from the last",
            ),
            MenuKind::AddStep => (
                [
                    ("guide", "Guide, drawn from a sequence"),
                    ("model", "Model, asked for a file"),
                    ("join", "Join, parts set end to end"),
                ]
                .into_iter()
                .map(|(kind, words)| {
                    let this = this.clone();
                    ui::MenuItem::new(words).on_choose(move |_, cx| {
                        let _ = this.update(cx, |graph, cx| {
                            graph.edit(
                                RecipeEdit {
                                    add: Some(kind.to_owned()),
                                    ..RecipeEdit::default()
                                },
                                cx,
                            )
                        });
                    })
                })
                .collect(),
                if self.recipe.as_ref().is_some_and(|recipe| !recipe.document) {
                    "A new line of the recipe"
                } else {
                    "A new step of the shot"
                },
            ),
            _ => return,
        };
        self.open_menu(kind, items, Some(header.to_owned()), at, window, cx);
    }

    /// Make, or Several: what it would run is asked first, over the graph,
    /// which stays as it is under it and follows the making after a yes.
    fn open_plan(&mut self, rolls: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let session = self.session.clone();
        let media = self.media.clone();
        let plan = cx.new(|cx| MakePlan::new(session, media, rolls, window, cx));
        let closed = cx.subscribe_in(&plan, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_over(window, cx)
        });
        window.focus(&plan.focus_handle(cx));
        self.over = Some(Over::Plan(plan));
        self._over = Some(closed);
        cx.notify();
    }

    /// Cast and Styles…, over the graph.
    fn open_cast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let cast = cx.new(|cx| crate::cast::CastView::new(self.session.clone(), window, cx));
        let closed = cx.subscribe_in(&cast, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_over(window, cx)
        });
        window.focus(&cast.focus_handle(cx));
        self.over = Some(Over::Cast(cast));
        self._over = Some(closed);
        cx.notify();
    }

    /// Settings, at its Models, over the graph.
    fn open_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let session = self.session.clone();
        let settings = cx.new(|cx| {
            crate::settings::SettingsView::new_at(
                session,
                crate::settings::Page::Models,
                window,
                cx,
            )
        });
        let closed = cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &crate::settings::SettingsEvent, window, cx| match event {
                crate::settings::SettingsEvent::Dismiss => this.close_over(window, cx),
            },
        );
        window.focus(&settings.focus_handle(cx));
        self.over = Some(Over::Settings(settings));
        self._over = Some(closed);
        cx.notify();
    }

    fn close_over(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.over.take().is_some() {
            self._over = None;
            // A keymap chosen in Settings bound the window's keys again
            // without the graph's.
            bind_keys(cx);
            window.focus(&self.canvas_focus);
            // Its takes may be other than they were.
            self.request(cx);
            cx.notify();
        }
    }

    fn arrow(&mut self, toward: Toward, window: &mut Window, cx: &mut Context<Self>) {
        let Some(graph) = self.shown() else {
            return;
        };
        let places = self.places();
        let centres: Vec<(String, (f32, f32))> = graph
            .nodes
            .iter()
            .filter_map(|node| {
                let [x, y, width, height] = card_rect(graph, &places, &node.id)?;
                Some((node.id.clone(), (x + width / 2.0, y + height / 2.0)))
            })
            .collect();
        if let Some(next) = nearest_toward(&centres, self.selected.as_deref(), toward) {
            self.select(Some(next), window, cx);
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let at = self.local(event.position);
        let zooms = event.modifiers.control || event.modifiers.platform;
        match event.delta {
            // Two fingers on a trackpad move the canvas.
            ScrollDelta::Pixels(delta) if !zooms => {
                self.camera.offset = (
                    self.camera.offset.0 + f32::from(delta.x),
                    self.camera.offset.1 + f32::from(delta.y),
                );
                self.fitted = false;
                cx.notify();
            }
            // A wheel, or either with Ctrl or ⌘, zooms about the pointer.
            ScrollDelta::Pixels(delta) => self.zoom_by(1.0015f32.powf(f32::from(delta.y)), at, cx),
            ScrollDelta::Lines(lines) => self.zoom_by(1.0015f32.powf(lines.y * 120.0), at, cx),
        }
    }

    // ------------------------------------------------------------ a signal drawn by hand

    fn start_curve(
        &mut self,
        clip: String,
        frames: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        self.drawing = Some(Drawing {
            clip,
            frames: frames.max(1),
            drawn: BTreeMap::new(),
            last: None,
            held: true,
        });
        self.drag = Drag::Curve;
        self.draw_curve(at);
        cx.notify();
    }

    fn draw_curve(&mut self, at: Point<Pixels>) {
        let bounds = self.curve_bounds.get();
        let rect = curve_plot(bounds);
        let at = (f32::from(at.x), f32::from(at.y));
        let Some(drawing) = self.drawing.as_mut() else {
            return;
        };
        let (frame, value) = signal_under(rect, drawing.frames, at);
        draw_signal(&mut drawing.drawn, drawing.last, frame, value);
        drawing.last = Some(frame);
    }

    /// Let go: what was drawn is the clip's value, keyed, in one edit; a
    /// single press is not a drawing. What was drawn stays shown until the
    /// project has it and says its curve again.
    fn drawn(&mut self, cx: &mut Context<Self>) {
        let Some(drawing) = self.drawing.as_mut() else {
            return;
        };
        drawing.held = false;
        if drawing.drawn.len() < 2 {
            self.drawing = None;
            return;
        }
        let keys = signal_keys(&drawing.drawn, CURVE_WITHIN);
        let op = json!({"op": "set", "clip": drawing.clip, "props": {"value": {"keys": keys}}});
        self.session.update(cx, |session, cx| {
            session.edit(vec![op]);
            cx.notify();
        });
    }

    // ------------------------------------------------------------ reading a clip from footage

    fn ask_picture(&mut self, clip: String, of: ShotOf, cx: &mut Context<Self>) {
        self.pointing = Pointing {
            clip: clip.clone(),
            of: Some(of),
            stage: PointStage::Drawing,
            ..Pointing::default()
        };
        self.session.update(cx, |session, _| {
            session.send(Command::ShotPicture { clip, of })
        });
        cx.notify();
    }

    fn point(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.pointing.stage != PointStage::Pointing {
            return;
        }
        let Some(size) = self
            .pointing
            .picture
            .as_deref()
            .and_then(|path| self.still(path))
            .map(|image| crate::artwork::image_size(&image))
        else {
            return;
        };
        let bounds = self.point_bounds.get();
        let room = [
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ];
        let Some(point) = point_on(
            fitted_in(room, size),
            (f32::from(event.position.x), f32::from(event.position.y)),
        ) else {
            return;
        };
        let right = event.button == MouseButton::Right;
        let pointing = &mut self.pointing;
        if pointing.of == Some(ShotOf::Moves) {
            // One point: pressed, it is followed at once.
            if right {
                return;
            }
            pointing.on = vec![point];
            self.follow(cx);
            return;
        }
        if right || event.modifiers.alt {
            pointing.off.push(point);
        } else {
            pointing.on.push(point);
        }
        cx.notify();
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let pointing = &mut self.pointing;
        if pointing.on.is_empty() {
            return;
        }
        // Points are only pressed on a picture asked for one way or the other.
        let Some(of) = pointing.of else {
            return;
        };
        pointing.stage = PointStage::Following;
        pointing.says.clear();
        pointing.failed = false;
        let points = |points: &[(f64, f64)]| -> Vec<[f64; 2]> {
            points.iter().map(|(x, y)| [*x, *y]).collect()
        };
        let command = Command::ShotFollow(ShotFollow {
            clip: pointing.clip.clone(),
            of,
            points: points(&pointing.on),
            outside: points(&pointing.off),
        });
        self.session.update(cx, |session, _| session.send(command));
        cx.notify();
    }

    // ------------------------------------------------------------ pictures

    /// The file at `path` in the project.
    fn in_project(&self, path: &str, cx: &App) -> Option<PathBuf> {
        if path.is_empty() {
            return None;
        }
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Some(path);
        }
        self.session
            .read(cx)
            .project
            .as_ref()
            .map(|project| project.join(path))
    }

    /// Every picture the graph made and has not let go of yet, let go of
    /// now: for the window to take out of its atlas as the graph closes.
    fn let_go_of_pictures(&mut self) -> Vec<Arc<RenderImage>> {
        let mut pictures = std::mem::take(&mut self.dropped);
        pictures.extend(
            self.stills
                .drain()
                .filter_map(|(_, (_, still))| match still {
                    Still::Ready(image) => Some(image),
                    Still::Asked | Still::Failed => None,
                }),
        );
        pictures
    }

    /// The picture of `path`, once made.
    fn still(&self, path: &Path) -> Option<Arc<RenderImage>> {
        match self.stills.get(path) {
            Some((_, Still::Ready(image))) => Some(image.clone()),
            _ => None,
        }
    }

    /// Asks for the pictures what is shown wants: each take, what each step
    /// drew for the take that shows, a picture a step reads, and the frame
    /// a clip is pointed on. Only files that are there; one that changed
    /// since its picture was made is read again. Asked as the graph or the
    /// frame is said, not as it is drawn.
    fn want_pictures(&mut self, cx: &mut Context<Self>) {
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Some(recipe) = &self.recipe {
            for node in &recipe.nodes {
                let holds = &node.holds;
                let mut named = vec![holds["made"]["path"].as_str().unwrap_or_default()];
                if node.kind == "file" && node.gives == "image" {
                    named.push(
                        holds["inProject"]
                            .as_str()
                            .or_else(|| holds["path"].as_str())
                            .unwrap_or_default(),
                    );
                }
                for take in holds["takes"].as_array().into_iter().flatten() {
                    named.push(take["path"].as_str().unwrap_or_default());
                }
                paths.extend(
                    named
                        .into_iter()
                        .filter_map(|path| self.in_project(path, cx)),
                );
            }
        }
        paths.extend(self.pointing.picture.clone());
        for path in paths {
            if !path.is_file() {
                continue;
            }
            let key = still_key(&path);
            if self
                .stills
                .get(&path)
                .is_some_and(|(known, _)| *known == key)
            {
                continue;
            }
            // What was made of the file before it changed is let go of.
            if let Some((_, Still::Ready(image))) = self.stills.remove(&path) {
                self.dropped.push(image);
            }
            self.stills.insert(path.clone(), (key, Still::Asked));
            self.queue.push(path);
        }
        self.make_pictures(cx);
    }

    /// Starts making the pictures that wait, while fewer than
    /// [`STILL_WORKERS`] are being made; each that ends starts the next.
    fn make_pictures(&mut self, cx: &mut Context<Self>) {
        while let Some(path) = self.queue.start() {
            // The file as it was last asked for; one made already, or let
            // go of, is not made again.
            let key = match self.stills.get(&path) {
                Some((key, Still::Asked)) => key.clone(),
                _ => {
                    self.queue.ended();
                    continue;
                }
            };
            cx.spawn(async move |this, cx| {
                let file = path.clone();
                let made = cx
                    .background_executor()
                    .spawn(async move { decode_still(&file) })
                    .await;
                let _ = this.update(cx, |graph, cx| {
                    graph.queue.ended();
                    // Kept only if the file is still as it was asked for.
                    if graph
                        .stills
                        .get(&path)
                        .is_some_and(|(known, _)| *known == key)
                    {
                        graph
                            .stills
                            .insert(path, (key, made.map_or(Still::Failed, Still::Ready)));
                        cx.notify();
                    }
                    graph.make_pictures(cx);
                });
            })
            .detach();
        }
    }

    // ------------------------------------------------------------ the head

    fn making<'a>(&self, session: &'a Session) -> Option<&'a Making> {
        session.state.makings.get(self.media.as_ref())
    }

    /// How the shot stands, the most pressing thing first, and what its tip says.
    fn status(&self, making: Option<&Making>) -> (String, Rgba, String) {
        if let Some(making) = making {
            let said = if making.line.is_empty() {
                format!("Making take {}…", making.take)
            } else {
                format!("Making take {}: {}", making.take, making.line)
            };
            return (said.clone(), theme::activity::gpu(), said);
        }
        let Some(recipe) = &self.recipe else {
            return match &self.refused {
                Some(why) => ("Cannot be shown".into(), theme::neg(), why.clone()),
                None => ("Reading its steps…".into(), theme::ink3(), String::new()),
            };
        };
        if let Some(error) = &recipe.error {
            let said = if recipe.document {
                "The steps do not read"
            } else {
                "The recipe does not read"
            };
            return (said.into(), theme::neg(), error.clone());
        }
        if let Some(cannot) = &recipe.cannot {
            return (
                "Cannot be made as things stand".into(),
                theme::neg(),
                cannot.clone(),
            );
        }
        let takes = recipe.node("takes");
        if let Some(wants) = takes.and_then(|takes| takes.holds["wants"].as_str()) {
            let said = format!("Wants making: {wants}");
            return (said.clone(), theme::warn(), said);
        }
        let said = takes.map(|takes| takes.detail.clone()).unwrap_or_default();
        (said.clone(), theme::ink3(), said)
    }

    fn head(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let making = self.making(session).cloned();
        let connected = session.state.connection == crate::store::Connection::Connected;
        let shot_name = session.media_name(&self.media);
        let (status, status_color, status_tip) = self.status(making.as_ref());
        let name = self
            .recipe
            .as_ref()
            .map(|recipe| recipe.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or(shot_name);
        let path = match (&self.workflow, &self.recipe) {
            (Some(workflow), _) => format!(
                "{} · {}",
                workflow.model,
                if workflow.is_own() {
                    &workflow.file
                } else {
                    &workflow.name
                }
            ),
            (None, Some(recipe)) if recipe.document => "steps in the document".to_owned(),
            (None, Some(recipe)) => recipe.recipe.clone(),
            (None, None) => String::new(),
        };
        let reads = self.recipe.as_ref().is_some_and(Recipe::reads);
        let document = self.recipe.as_ref().is_none_or(|recipe| recipe.document);
        let has_takes = self
            .recipe
            .as_ref()
            .and_then(|recipe| recipe.node("takes"))
            .and_then(|takes| takes.holds["takes"].as_array())
            .is_some_and(|takes| !takes.is_empty());
        let can = connected && reads;
        let open_menu = self.menu.as_ref().map(|open| open.kind);
        let why = if !connected {
            "Kerf is not connected".to_owned()
        } else if self.recipe.is_none() {
            "Its steps are still being read".to_owned()
        } else if document {
            "The steps do not read".to_owned()
        } else {
            "The recipe does not read".to_owned()
        };
        let mut head = div()
            .h(px(HEAD))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(if self.workflow.is_some() { 10.0 } else { 16.0 }))
            .pr(px(12.0))
            .border_b_1()
            .border_color(theme::line());
        if self.workflow.is_some() {
            head = head.child(
                icon_button("graph-back", Icon::ChevronLeft)
                    .tooltip("Back to the shot's graph")
                    .on_click(cx.listener(|this, _, window, cx| this.close_workflow(window, cx))),
            );
        }
        head = head
            .child(Icon::Media.view(16.0, theme::ink2()))
            .child(
                ui::line(name, text::TITLE, theme::ink())
                    .flex_none()
                    .max_w(px(240.0)),
            )
            .when(!path.is_empty(), |head| {
                head.child(
                    ui::line(path, text::MONO_SMALL, theme::ink3())
                        .flex_shrink()
                        .min_w_0()
                        .max_w(px(320.0)),
                )
            })
            .child(ui::tip_on(
                div()
                    .id("graph-status")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_end()
                    .child(ui::line(status, text::CONTROL, status_color)),
                (!status_tip.is_empty()).then(|| ui::Tip::new(status_tip)),
            ));
        if making.is_some() {
            head = head.child(
                ui::button("graph-stop", "Stop")
                    .icon(Icon::Stop)
                    .tooltip("Stop making: its step is asked to end, and no take is kept")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let media = this.media.to_string();
                        this.session.update(cx, |session, _| {
                            session.send(Command::MediaMakeCancel { media });
                        });
                    })),
            );
        } else {
            if self.workflow.is_none() {
                // A step is added to the shot's steps, not to a model's workflow, and to ones that read.
                let can_add = reads;
                head = head.child(
                    ui::trigger(
                        "graph-add-step-trigger",
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if can_add {
                                this.commit_fields(cx);
                                let at = event.position + point(px(0.0), px(16.0));
                                this.toggle_menu(MenuKind::AddStep, at, window, cx);
                            }
                        }),
                    )
                    .child(
                        ui::button("graph-add-step", "Add step")
                            .ghost()
                            .icon(Icon::Plus)
                            .disabled(!can_add)
                            .when(open_menu != Some(MenuKind::AddStep), |button| {
                                button
                                    .tooltip("A new step, written to read as it stands, then set in its card")
                                    .reason(why.clone())
                            }),
                    ),
                );
            }
            head = head
                .child(
                    ui::trigger(
                        "graph-several-trigger",
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if can {
                                this.commit_fields(cx);
                                let at = event.position + point(px(0.0), px(16.0));
                                this.toggle_menu(MenuKind::Several, at, window, cx);
                            }
                        }),
                    )
                    .child(
                        ui::button("graph-several", "Several")
                            .ghost()
                            .icon(Icon::ChevronDown)
                            .disabled(!can)
                            .when(open_menu != Some(MenuKind::Several), |button| {
                                button
                                    .tooltip("Make two to four takes to choose among")
                                    .reason(why.clone())
                            }),
                    ),
                )
                .child(
                    ui::button("graph-make", if has_takes { "Make again" } else { "Make" })
                        .primary()
                        .disabled(!can)
                        .tooltip(if document {
                            "Run its steps and write a new take; what it runs is shown first"
                        } else {
                            "Run the recipe and write a new take; what it runs is shown first"
                        })
                        .reason(why)
                        .on_click(cx.listener(|this, _, window, cx| this.open_plan(1, window, cx))),
                );
        }
        head.child(
            icon_button("graph-close", Icon::Close)
                .tooltip("Close")
                .key("Esc")
                .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
        )
        .into_any_element()
    }

    // ------------------------------------------------------------ the canvas

    /// The canvas: its dots, the edges and the cards, panned and zoomed as
    /// a whole, the legend and the zoom's buttons at its foot.
    fn canvas(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let room =
            div()
                .id("graph-canvas")
                .key_context(CANVAS)
                .track_focus(&self.canvas_focus)
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
                .bg(theme::well())
                .on_action(cx.listener(|this, _: &FitGraph, _, cx| this.fit(cx)))
                .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                    let middle = this.middle();
                    this.zoom_by(ZOOM_STEP, middle, cx)
                }))
                .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                    let middle = this.middle();
                    this.zoom_by(1.0 / ZOOM_STEP, middle, cx)
                }))
                .on_action(cx.listener(|this, _: &NodeLeft, window, cx| {
                    this.arrow(Toward::Left, window, cx)
                }))
                .on_action(cx.listener(|this, _: &NodeRight, window, cx| {
                    this.arrow(Toward::Right, window, cx)
                }))
                .on_action(
                    cx.listener(|this, _: &NodeUp, window, cx| this.arrow(Toward::Up, window, cx)),
                )
                .on_action(cx.listener(|this, _: &NodeDown, window, cx| {
                    this.arrow(Toward::Down, window, cx)
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.press_canvas(event, window, cx)
                    }),
                )
                // Ahead of the cards: a middle drag pans from over one too.
                .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.press_middle(event, window, cx)
                }))
                .on_scroll_wheel(
                    cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.wheel(event, cx)),
                );
        let bounds = self.canvas_bounds.clone();
        let keep_bounds = canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
            .absolute()
            .inset_0();
        let Some(graph) = self.shown().cloned() else {
            return self.waiting(room.child(keep_bounds), cx).into_any_element();
        };
        if graph.nodes.is_empty() {
            let why = graph
                .error
                .clone()
                .unwrap_or_else(|| "There are no steps to show.".into());
            let said = if graph.document {
                format!("These steps do not read: {why}")
            } else {
                why
            };
            return room
                .child(keep_bounds)
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .child(ui::label(said, text::BODY, theme::neg()).max_w(px(480.0)))
                .into_any_element();
        }
        let places = self.places();
        let camera = self.camera;
        let zoom = camera.zoom;
        let view = |at: (f32, f32)| camera.to_view(at);
        let selected = self.selected.clone();
        let touches = |edge: &GraphEdge| {
            selected
                .as_deref()
                .is_none_or(|id| edge.from == id || edge.to == id)
        };
        // Edges under the cards. With a node chosen, what does not touch it
        // falls back; what a model has said it has no use for goes to it dashed.
        let edges: Vec<EdgeLine> = graph
            .edges
            .iter()
            .filter_map(|edge| {
                let start = view(out_port(&graph, &places, &edge.from)?);
                let end = view(in_port(&graph, &places, edge)?);
                let color = gives_color(&edge.gives);
                let dashed = graph
                    .node(&edge.to)
                    .is_some_and(|to| !reads_port(to, &edge.port));
                Some(EdgeLine {
                    start,
                    end,
                    color: if touches(edge) {
                        color
                    } else {
                        theme::with_alpha(color, 0.35)
                    },
                    dashed,
                })
            })
            .collect();
        // Where each edge leaves and arrives, over the cards' edges.
        let ends: Vec<EdgeEnds> = graph
            .edges
            .iter()
            .filter_map(|edge| {
                Some((
                    view(out_port(&graph, &places, &edge.from)?),
                    view(in_port(&graph, &places, edge)?),
                    gives_color(&edge.gives),
                ))
            })
            .collect();
        // By what name the step reads each, beside where it comes in; too small to read, none is.
        let labels: Vec<AnyElement> = if zoom < ZOOM_READ {
            Vec::new()
        } else {
            graph
                .edges
                .iter()
                .filter(|edge| {
                    graph
                        .node(&edge.to)
                        .is_some_and(|to| matches!(to.kind.as_str(), "model" | "run" | "join"))
                })
                .filter_map(|edge| {
                    let end = view(in_port(&graph, &places, edge)?);
                    Some(port_label(&edge.port, end, touches(edge)))
                })
                .collect()
        };
        let grid = GRID * zoom;
        let dots = (
            camera.offset.0.rem_euclid(grid),
            camera.offset.1.rem_euclid(grid),
        );
        let session = self.session.read(cx);
        let making = self.making(session).cloned();
        if making.is_some() && !motion::reduced() && self.workflow.is_none() {
            window.request_animation_frame();
        }
        let wire_over = match &self.drag {
            Drag::Wire { over, .. } => over.clone(),
            _ => None,
        };
        let cards: Vec<AnyElement> = graph
            .nodes
            .iter()
            .filter_map(|node| {
                let [x, y, width, height] = card_rect(&graph, &places, &node.id)?;
                let at = view((x, y));
                let size = (width * zoom, height * zoom);
                let over = wire_over.as_deref() == Some(node.id.as_str());
                Some(if node.is_part() {
                    self.part_card(node, &graph, at, size, zoom, cx)
                } else {
                    self.card(node, at, size, making.as_ref(), over, cx)
                })
            })
            .collect();
        // The ports, a little larger under the pointer: an edge is drawn from one.
        let ports: Vec<AnyElement> = graph
            .nodes
            .iter()
            .filter(|node| node.kind != "takes")
            .filter_map(|node| {
                let at = view(out_port(&graph, &places, &node.id)?);
                let id = node.id.clone();
                let hover_id = node.id.clone();
                Some(
                    div()
                        .id(ElementId::Name(format!("port-{}", node.id).into()))
                        .absolute()
                        .left(px(at.0 - PORT_REACH))
                        .top(px(at.1 - PORT_REACH))
                        .size(px(2.0 * PORT_REACH))
                        .rounded_full()
                        .cursor(CursorStyle::Crosshair)
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            let now = hovered.then(|| hover_id.clone());
                            if this.hovered_port != now {
                                this.hovered_port = now;
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.press_port(&id, event, window, cx);
                            }),
                        )
                        .into_any_element(),
                )
            })
            .collect();
        let lit_port = match &self.drag {
            Drag::Wire { from, .. } => Some((from.clone(), true)),
            _ => self.hovered_port.clone().map(|id| (id, false)),
        };
        let lit = lit_port.and_then(|(id, held)| {
            let node = graph.node(&id)?;
            Some((
                view(out_port(&graph, &places, &id)?),
                gives_color(&node.gives),
                held,
            ))
        });
        // The edge being drawn, from its port to the pointer, in what it would carry.
        let wire = match &self.drag {
            Drag::Wire { from, at, over } => graph.node(from).and_then(|node| {
                Some((
                    view(out_port(&graph, &places, from)?),
                    *at,
                    gives_color(&node.gives),
                    over.is_none(),
                ))
            }),
            _ => None,
        };
        let card_fill = theme::card();
        room.child(
            canvas(
                {
                    let bounds = self.canvas_bounds.clone();
                    move |area, _, _| bounds.set(area)
                },
                move |bounds, _, window, _| {
                    let origin = bounds.origin;
                    let (width, height) =
                        (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    // The canvas's grid: a dot a step, while they are far enough apart to be dots.
                    if grid >= GRID_LEAST {
                        let dot = theme::with_alpha(theme::ink3(), 0.22);
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
                    }
                    for edge in &edges {
                        // It leaves and arrives level, and turns in the room between the columns.
                        let reach = (48.0 * zoom).max((edge.end.0 - edge.start.0).abs() * 0.6);
                        let at = |(x, y): (f32, f32)| origin + point(px(x), px(y));
                        let mut path = PathBuilder::stroke(px(1.5));
                        if edge.dashed {
                            path = path.dash_array(&[px(3.0), px(3.0)]);
                        }
                        path.move_to(at(edge.start));
                        path.cubic_bezier_to(
                            at(edge.end),
                            at((edge.start.0 + reach, edge.start.1)),
                            at((edge.end.0 - reach, edge.end.1)),
                        );
                        if let Ok(path) = path.build() {
                            window.paint_path(path, edge.color);
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .children(cards)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let origin = bounds.origin;
                    let at = |(x, y): (f32, f32)| origin + point(px(x), px(y));
                    for (start, end, color) in &ends {
                        paint_ring(window, at(*start), 3.5 * zoom, card_fill, Some(*color));
                        paint_ring(window, at(*end), 3.5 * zoom, *color, None);
                    }
                    if let Some((port, color, held)) = lit {
                        let fill = if held { color } else { card_fill };
                        paint_ring(window, at(port), 5.0 * zoom.max(0.8), fill, Some(color));
                    }
                    if let Some((start, end, color, held)) = wire {
                        let reach = 32f32.max((end.0 - start.0).abs() * 0.5);
                        let mut path = PathBuilder::stroke(px(1.5));
                        // Until it is over something that could read it, it is only being held.
                        if held {
                            path = path.dash_array(&[px(3.0), px(3.0)]);
                        }
                        path.move_to(at(start));
                        path.cubic_bezier_to(
                            at(end),
                            at((start.0 + reach, start.1)),
                            at((end.0 - reach, end.1)),
                        );
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .children(labels)
        .children(ports)
        .child(self.legend(&graph, cx))
        .into_any_element()
    }

    /// The canvas before there is a graph: what it waits on, or why there is none.
    fn waiting(
        &self,
        room: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let reading = if self.workflow_line > 0 {
            "Reading the model's workflow…".to_owned()
        } else {
            format!("Reading how {} is made…", self.media)
        };
        let said = match &self.refused {
            Some(why) => ui::label(why.clone(), text::BODY, theme::neg()),
            None => ui::label(reading, text::BODY, theme::ink3()),
        };
        let mut column = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .max_w(px(480.0))
            // A press on what it says, or its button, is not one on the canvas.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(said);
        if self
            .refused
            .as_deref()
            .is_some_and(|why| why.contains("recipe file") || why.contains("kerf carry"))
        {
            column = column.child(
                ui::button("graph-carry", "Carry Steps into the Document…")
                    .on_click(cx.listener(|this, _, window, cx| this.open_plan(1, window, cx))),
            );
        }
        room.flex()
            .items_center()
            .justify_center()
            .p(px(24.0))
            .child(column)
    }

    /// The legend of what the edges carry, or of what a workflow's rows
    /// are, and beside it the zoom's buttons. They sit on the canvas, but a
    /// press on them is theirs: it neither pans the canvas, nor chooses the
    /// shot, nor counts towards a double-click that fits the graph.
    fn legend(&self, graph: &Recipe, cx: &mut Context<Self>) -> AnyElement {
        let card = || {
            div()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(10.0))
                .h(px(LEGEND))
                .rounded(px(radius::CARD))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::line2())
        };
        let sample = |color: Rgba| div().w(px(16.0)).h(px(1.5)).rounded_full().bg(color);
        let legend = if graph.parts {
            let fills = if self.recipe.as_ref().is_some_and(|recipe| !recipe.document) {
                "the recipe fills"
            } else {
                "the steps fill"
            };
            card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(sample(gives_color("file")))
                        .child(ui::label("link", text::META, theme::ink2())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .px(px(5.0))
                                .rounded(px(radius::SMALL))
                                .bg(theme::accent_wash())
                                .child(ui::label("{blank}", text::MONO_SMALL, theme::accent_ink())),
                        )
                        .child(ui::label(fills, text::META, theme::ink2())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(ui::label("value", text::MONO_SMALL, theme::ink2()))
                        .child(ui::label("set here", text::META, theme::ink2())),
                )
        } else {
            // The four a shot is told by, always; words and other files where the graph has them.
            let mut shown: Vec<&str> = vec!["video", "image", "path", "signal"];
            for more in ["words", "file"] {
                if graph.edges.iter().any(|edge| edge.gives == more) {
                    shown.push(more);
                }
            }
            card().children(shown.into_iter().map(|gives| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(sample(gives_color(gives)))
                    .child(ui::label(gives, text::META, theme::ink2()))
            }))
        };
        let zoom = card()
            .gap(px(0.0))
            .px(px(4.0))
            .child(
                ui::icon_button("graph-zoom-out", Icon::ZoomOut)
                    .focusable(false)
                    .tooltip("Zoom out")
                    .key("-")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let middle = this.middle();
                        this.zoom_by(1.0 / ZOOM_STEP, middle, cx)
                    })),
            )
            .child(
                ui::icon_button("graph-zoom-in", Icon::ZoomIn)
                    .focusable(false)
                    .tooltip("Zoom in")
                    .key("+")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let middle = this.middle();
                        this.zoom_by(ZOOM_STEP, middle, cx)
                    })),
            )
            .child(
                ui::button("graph-fit", "Fit")
                    .ghost()
                    .small()
                    .focusable(false)
                    .tooltip("The whole graph in view")
                    .key("F")
                    .on_click(cx.listener(|this, _, _, cx| this.fit(cx))),
            );
        div()
            .absolute()
            .left(px(PAD))
            .bottom(px(PAD))
            .flex()
            .gap(px(8.0))
            .child(legend)
            .child(zoom)
            .into_any_element()
    }

    fn card(
        &self,
        node: &GraphNode,
        at: (f32, f32),
        size: (f32, f32),
        making: Option<&Making>,
        wire_over: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let zoom = self.camera.zoom;
        let z = |points: f32| px(points * zoom);
        let color = gives_color(&node.gives);
        let selected = self.selected.as_deref() == Some(node.id.as_str());
        let id = node.id.clone();
        let striped = making.filter(|_| node.kind == "takes");
        let detail = match striped {
            Some(making) => format!("Making take {} · {}", making.take, clock(making.since)),
            None if node.kind == "model" && node.detail.trim().is_empty() => "no prompt".to_owned(),
            None => one_line(&node.detail),
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
        // Under the lines: a signal's number over its clip, or the path a clip is led along.
        let under = if node.kind == "model" {
            CARD_H - 9.0 + CARD_MODEL
        } else {
            CARD_H - 9.0
        };
        let curve = draws_curve(node).then(|| card_curve(node, color, under, zoom));
        div()
            .id(ElementId::Name(format!("node-{}", node.id).into()))
            .absolute()
            .left(px(at.0))
            .top(px(at.1))
            .w(px(size.0))
            .h(px(size.1))
            .rounded(z(radius::PANEL))
            .overflow_hidden()
            .bg(theme::card())
            .when(wire_over, |card| {
                card.border(px(1.5)).border_color(theme::accent_ink())
            })
            .when(!wire_over, |card| {
                card.border_1().border_color(if selected {
                    theme::accent()
                } else {
                    theme::line2()
                })
            })
            .when(!selected && !wire_over, |card| {
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
            .children(curve)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.press_card(&id, event, window, cx);
                }),
            )
            .into_any_element()
    }

    /// A node of a model's workflow: its name and number, over a row an
    /// input: a link comes in at its row; a blank is the steps', in the
    /// accent's ink on a wash of it; a value is the workflow's own, plain.
    fn part_card(
        &self,
        node: &GraphNode,
        _graph: &Recipe,
        at: (f32, f32),
        size: (f32, f32),
        zoom: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let z = |points: f32| px(points * zoom);
        let selected = self.selected.as_deref() == Some(node.id.as_str());
        let id = node.id.clone();
        let number = format!("#{}", value_text(&node.holds["id"]));
        let rows = node.holds["inputs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|input| {
                let name = input["name"].as_str().unwrap_or_default().to_owned();
                let kind = input_kind(&input);
                let value = shown_value(&input["value"]);
                div()
                    .h(z(PART_ROW))
                    .flex()
                    .items_center()
                    .gap(z(10.0))
                    .px(z(10.0))
                    .child(
                        zoomed(
                            ui::line(
                                name,
                                text::META,
                                if kind == InputKind::Link {
                                    theme::ink2()
                                } else {
                                    theme::ink3()
                                },
                            ),
                            text::META,
                            zoom,
                        )
                        .flex_shrink()
                        .min_w_0()
                        .max_w(z(PART_W * 0.5)),
                    )
                    .child(div().flex_1())
                    .when(kind == InputKind::Blank, |row| {
                        row.child(
                            div()
                                .px(z(5.0))
                                .rounded(z(radius::SMALL))
                                .bg(theme::accent_wash())
                                .min_w_0()
                                .child(zoomed(
                                    ui::line(value.clone(), text::MONO_SMALL, theme::accent_ink()),
                                    text::MONO_SMALL,
                                    zoom,
                                )),
                        )
                    })
                    .when(kind == InputKind::Value, |row| {
                        row.child(
                            zoomed(
                                ui::line(value.clone(), text::MONO_SMALL, theme::ink2()),
                                text::MONO_SMALL,
                                zoom,
                            )
                            .min_w_0(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        div()
            .id(ElementId::Name(format!("node-{}", node.id).into()))
            .absolute()
            .left(px(at.0))
            .top(px(at.1))
            .w(px(size.0))
            .h(px(size.1))
            .rounded(z(radius::CARD))
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
            .child(
                div()
                    .h(z(PART_HEAD))
                    .flex()
                    .items_center()
                    .gap(z(8.0))
                    .px(z(10.0))
                    .pt(z(4.0))
                    .child(
                        zoomed(
                            ui::line(node.title.clone(), text::CONTROL_STRONG, theme::ink()),
                            text::CONTROL_STRONG,
                            zoom,
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(zoomed(
                        ui::label(number, text::MONO_SMALL, theme::ink3()),
                        text::MONO_SMALL,
                        zoom,
                    )),
            )
            .child(div().mx(px(1.0)).h(px(1.0)).bg(theme::line()))
            .children(rows)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.press_card(&id, event, window, cx);
                }),
            )
            .into_any_element()
    }

    // ------------------------------------------------------------ the column

    /// The column at the right: the chosen node and its settings, the shot,
    /// or the model's workflow.
    fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div().flex().flex_col().gap(px(8.0)).p(px(14.0));
        let session = self.session.read(cx);
        let making = self.making(session).is_some();
        let connected = session.state.connection == crate::store::Connection::Connected;
        let node = self
            .selected
            .as_ref()
            .and_then(|id| self.shown()?.node(id))
            .cloned();
        if let Some(said) = &self.said {
            column = column.child(ui::label(said.clone(), text::CONTROL, theme::neg()));
        }
        match (self.shown().cloned(), node) {
            (None, _) => {
                column = column.child(ui::label(self.media.to_string(), text::TITLE, theme::ink()));
            }
            (Some(_), None) if self.workflow.is_some() => {
                column = column.child(self.workflow_column(cx));
            }
            (Some(recipe), None) => {
                column = column.child(self.shot_column(&recipe, making, connected, cx));
            }
            (Some(graph), Some(node)) => {
                let color = gives_color(&node.gives);
                let tag = if node.is_part() {
                    format!("#{}", value_text(&node.holds["id"]))
                } else {
                    node.gives.clone()
                };
                column = column.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .when(!node.is_part(), |head| {
                            head.child(kind_icon(&node.kind).view(16.0, color))
                        })
                        .child(
                            ui::line(node.title.clone(), text::TITLE, theme::ink())
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(ui::label(tag, text::MONO_SMALL, theme::ink3())),
                );
                column = column.child(self.node_column(&graph, &node, making, connected, cx));
                if let Some(line) = node.line.filter(|_| !node.is_part()) {
                    column = column.child(
                        div().flex().pt(px(8.0)).child(
                            ui::button("graph-remove-step", "Delete step")
                                .ghost()
                                .icon(Icon::Trash)
                                .disabled(making)
                                .tooltip(format!(
                                    "Take line {line} out of the shot's steps. A step whose file a later one reads, and the last, stay until that is changed"
                                ))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit(
                                        RecipeEdit {
                                            line,
                                            remove: true,
                                            ..RecipeEdit::default()
                                        },
                                        cx,
                                    );
                                    this.select(None, window, cx);
                                })),
                        ),
                    );
                }
            }
        }
        // Scrolling with no bar, it fades where a row is cut and while
        // there is more below.
        div()
            .relative()
            .w(px(INSPECTOR))
            .flex_none()
            .h_full()
            .child(
                div()
                    .id("graph-inspector")
                    .track_scroll(&self.inspector_scroll)
                    .size_full()
                    .overflow_y_scroll()
                    .bg(theme::chrome())
                    .border_l_1()
                    .border_color(theme::line())
                    .child(column),
            )
            .child(ui::scroll_fade(&self.inspector_scroll, theme::chrome()))
            .into_any_element()
    }

    /// No node chosen: the shot itself, and what is worth knowing before it is made.
    fn shot_column(
        &self,
        recipe: &Recipe,
        making: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
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
            column = column.child(note(format!("{said}: {error}"), theme::neg()));
        }
        if let Some(cannot) = &recipe.cannot {
            column = column.child(note(
                format!("It cannot be made as things stand: {cannot}"),
                theme::neg(),
            ));
        }
        if let Some(error) = &recipe.models_error {
            column = column.child(note(error.clone(), theme::neg()));
        }
        if let Some(notes) = notes_block("Before it is made", &recipe.notes) {
            column = column.child(notes);
        }
        if recipe.error.is_none() {
            column = column.child(note(
                if recipe.document {
                    "Choose a node to see and set what it holds. A change is written into the shot's steps in the document, as an edit that is undone as any other; Make then writes a new take."
                } else {
                    "Choose a node to see and set what it holds. A change is written into the recipe, which keeps its comments and its order; Make then writes a new take."
                }
                .to_owned(),
                theme::ink2(),
            ));
            column = column.child(ui::label(
                "Drag a card to move it, and from the dot at its right to wire it into another. The wheel or ⌘ and two fingers zoom; F fits the whole graph.",
                text::META,
                theme::ink3(),
            ));
        }
        if let Some(takes) = recipe.node("takes") {
            column = column.child(self.takes(takes, making, connected, cx));
        }
        column
    }

    /// No node of a workflow chosen: the workflow itself, and what fills its blanks.
    fn workflow_column(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(8.0));
        let Some(workflow) = &self.workflow else {
            return column;
        };
        column = column
            .child(ui::label(workflow.name.clone(), text::TITLE, theme::ink()))
            .child(note(
                format!("The ComfyUI workflow model {} is run with.", workflow.model),
                theme::ink3(),
            ));
        if !workflow.about.is_empty() {
            column = column.child(note(workflow.about.clone(), theme::ink2()));
        }
        if workflow.is_own() {
            column = column.child(note(
                format!(
                    "This shot's own copy, {}, which its line names. It is what this shot is made with; nothing on the ComfyUI server is changed.",
                    workflow.file
                ),
                theme::ink2(),
            ));
            let line = workflow.line;
            column = column.child(
                div().flex().child(
                    ui::button("graph-workflow-drop", "Use it as it comes")
                        .ghost()
                        .tooltip("Take workflow= off the line; the copy stays in the project")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit(
                                RecipeEdit {
                                    line,
                                    key: "workflow".to_owned(),
                                    nth: 0,
                                    ..RecipeEdit::default()
                                },
                                cx,
                            )
                        })),
                ),
            );
        } else {
            let from = if workflow.origin == "file" {
                ", from the file your models entry names"
            } else {
                " with kerf-comfy"
            };
            column = column.child(note(
                format!(
                    "As it comes{from}. Set a value in it and this shot keeps a copy with that change, in the project, which its line then runs. Nothing on the ComfyUI server, and nothing of kerf-comfy, is changed."
                ),
                theme::ink2(),
            ));
        }
        if !workflow.blanks.is_empty() {
            column = column.child(section("Blanks the steps fill"));
            for name in workflow.blanks.keys() {
                column = column
                    .child(ui::label(
                        format!("{{{name}}}"),
                        text::TIMECODE,
                        theme::accent_ink(),
                    ))
                    .child(note(workflow.blank_says(name), theme::ink3()));
            }
        }
        column.child(note(
            "Choose a node to see its inputs. A blank is set on the step's line; any other value is set here.".to_owned(),
            theme::ink3(),
        ))
    }

    /// What a node is, by its kind, and its settings.
    fn node_column(
        &self,
        graph: &Recipe,
        node: &GraphNode,
        making: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let holds = &node.holds;
        let missing = holds["there"] == Value::Bool(false);
        let mut column = div().flex().flex_col().gap(px(8.0));
        match node.kind.as_str() {
            "part" => return column.child(self.part_column(graph, node, cx)),
            "model" => return column.child(self.model_column(graph, node, cx)),
            "render" => {
                column = column
                    .child(note(
                        "Kerf draws a sequence of this project into a file a model follows, with no sound.".into(),
                        theme::ink2(),
                    ))
                    .children(self.fields_named(&["sequence"]))
                    .child(self.pass_row(node, cx))
                    .children(self.fields_named(&["size", "rate", "matte", "from", "seconds"]));
                // A matte is one clip in white on black: of footage, as much of it as a mask cuts out.
                if let Some(matte) = holds["matte"].as_str().filter(|matte| !matte.is_empty()) {
                    column = column.child(self.pointing(matte, ShotOf::Matte, cx));
                }
                // The guide as it was drawn for the take that shows: what the model was handed.
                let made = &holds["made"];
                let drawn = made["path"]
                    .as_str()
                    .and_then(|path| self.in_project(path, cx))
                    .and_then(|path| self.still(&path));
                if let Some(image) = drawn {
                    column = column
                        .child(section(&format!(
                            "As drawn for take {}",
                            made["take"].as_u64().unwrap_or(0)
                        )))
                        .child(picture_box(image));
                }
            }
            "join" => {
                column = column
                    .child(note(
                        "Kerf sets the parts it reads end to end into one file: a shot longer than a model makes at once, made a part at a time.".into(),
                        theme::ink2(),
                    ))
                    .children(self.fields_named(&["lap"]))
                    .child(note(
                        "Lap is how many frames each part after the first shares with the one before it, which are left out: 1 where a part goes on from the last frame of the one before.".into(),
                        theme::ink3(),
                    ));
            }
            "sequence" => {
                column = column
                    .child(note(
                        node.detail.clone(),
                        if missing { theme::neg() } else { theme::ink2() },
                    ))
                    .child(note(
                        "A sequence of this project, which the guide is drawn from. Edit it on the timeline.".into(),
                        theme::ink3(),
                    ));
            }
            "file" => {
                // A picture shows; any file says where it is.
                let shown = holds["inProject"]
                    .as_str()
                    .or_else(|| holds["path"].as_str())
                    .and_then(|path| self.in_project(path, cx))
                    .filter(|_| node.gives == "image")
                    .and_then(|path| self.still(&path));
                if let Some(image) = shown {
                    column = column.child(picture_box(image));
                }
                column = column.child(ui::label(
                    value_text(&holds["path"]),
                    text::TIMECODE,
                    theme::ink2(),
                ));
                if missing {
                    column = column.child(note(
                        "This file is not there. The steps cannot run until it is.".into(),
                        theme::neg(),
                    ));
                }
            }
            "signal" => column = column.child(self.signal_column(node, cx)),
            "moves" => {
                let said = if missing {
                    node.detail.clone()
                } else {
                    format!(
                        "The model is handed where this clip is on each frame of {}: its place as the sequence draws it, so a camera that moves and what hides it are in it. Move the clip, or its path on the picture, to change it.",
                        holds["sequence"].as_str().unwrap_or("its sequence")
                    )
                };
                column = column.child(note(
                    said,
                    if missing { theme::neg() } else { theme::ink2() },
                ));
                if !missing {
                    let clip = holds["clip"].as_str().unwrap_or_default();
                    column = column.child(self.pointing(clip, ShotOf::Moves, cx));
                }
            }
            "cast" | "style" => {
                column = column.child(note(
                    node.detail.clone(),
                    if missing { theme::neg() } else { theme::ink2() },
                ));
                let refs = strings(&holds["refs"]);
                if !refs.is_empty() {
                    column =
                        column.child(ui::label(refs.join("\n"), text::MONO_SMALL, theme::ink3()));
                }
                column = column.child(
                    div().flex().child(
                        ui::button("graph-cast", "Cast and Styles…")
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_cast(window, cx)),
                            ),
                    ),
                );
            }
            "takes" => return column.child(self.takes(node, making, connected, cx)),
            _ => {
                if !node.detail.is_empty() {
                    column = column.child(note(node.detail.clone(), theme::ink2()));
                }
            }
        }
        column.child(self.reads(graph, node, cx))
    }

    /// The rows of the fields named, in that order.
    fn fields_named(&self, keys: &[&str]) -> Vec<AnyElement> {
        keys.iter()
            .filter_map(|key| self.field(key))
            .map(|field| field_row(field, None).into_any_element())
            .collect()
    }

    fn model_column(&self, graph: &Recipe, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let holds = &node.holds;
        let mut column = div().flex().flex_col().gap(px(8.0));
        if let Some(about) = holds["about"].as_str().filter(|about| !about.is_empty()) {
            column = column.child(note(about.to_owned(), theme::ink2()));
        }
        if node.is_missing() {
            // Named by the steps, and by nothing of the person's: it cannot be asked.
            column = column
                .child(note(
                    format!(
                        "No model named {} is set up on this computer. A model is a program your own models file names.",
                        holds["model"].as_str().unwrap_or_default()
                    ),
                    theme::warn(),
                ))
                .child(
                    div().flex().child(
                        ui::button("graph-models", "Models…")
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.open_models(window, cx))),
                    ),
                );
        }
        column = column.child(section("Prompt"));
        if let Some(prompt) = self.field("prompt") {
            column = column.child(field_box(prompt));
        }
        // The seed, and another at a press: the same shot, rolled again.
        if let Some(seed) = self.field("seed") {
            let die = ui::icon_button("graph-seed-roll", Icon::Dice)
                .tooltip("Another seed")
                .on_click(cx.listener(|this, _, _, cx| this.roll_seed(cx)));
            column = column.child(field_row(seed, Some(die.into_any_element())));
        }
        column = column.children(self.fields_named(&["seconds", "size", "from"]));
        // The knobs: what the line sets for this model alone, then what the
        // model has said it takes, each as it comes until the line says otherwise.
        let knobs: Vec<&str> = holds["knobs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|knob| knob["name"].as_str())
            .collect();
        column = column
            .child(section("Knobs"))
            .children(self.fields_named(&knobs));
        column = column.children(self.fields_named(&[ADD]));
        // A model its entry runs with kerf-comfy has a workflow behind it.
        if node.has_workflow() {
            let own = holds["workflow"].as_str().filter(|own| !own.is_empty());
            let says = match own {
                Some(own) => format!("{own}, this shot's own copy."),
                None => format!(
                    "{}, as it comes with kerf-comfy.",
                    value_text(&holds["runner"]["workflow"])
                ),
            };
            let line = node.line.unwrap_or(0);
            column = column
                .child(section("Workflow"))
                .child(note(says, theme::ink2()))
                .child(
                    div().flex().child(
                        ui::button("graph-open-workflow", "Open workflow")
                            .tooltip(
                                "Every node and input of the ComfyUI graph this model is run with",
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_workflow(line, window, cx)
                            })),
                    ),
                );
        }
        column.child(self.reads(graph, node, cx))
    }

    /// A guide's Draws, chosen from a menu with the one it draws ticked.
    fn pass_row(&self, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let pass = node.holds["pass"]
            .as_str()
            .filter(|pass| !pass.is_empty())
            .unwrap_or("picture")
            .to_owned();
        let shown = PASSES
            .iter()
            .find(|(id, _)| *id == pass)
            .map_or(pass.clone(), |(_, name)| (*name).to_owned());
        let line = node.line.unwrap_or(0);
        let trigger = ui::trigger(
            "graph-pass-trigger",
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.commit_fields(cx);
                if this.close_menu(MenuKind::Pass, window, cx) {
                    return;
                }
                let this_entity = cx.weak_entity();
                let items = PASSES
                    .iter()
                    .map(|(id, name)| {
                        let (this, id, chosen) = (this_entity.clone(), *id, *id == pass);
                        ui::MenuItem::new(*name)
                            .checked(chosen)
                            .on_choose(move |_, cx| {
                                if chosen || line == 0 {
                                    return;
                                }
                                let _ = this.update(cx, |graph, cx| {
                                    // The picture is what a line draws when it names no pass.
                                    let value = (id != "picture").then(|| id.to_owned());
                                    graph.edit(
                                        RecipeEdit {
                                            line,
                                            key: "pass".to_owned(),
                                            nth: 0,
                                            value,
                                            ..RecipeEdit::default()
                                        },
                                        cx,
                                    )
                                });
                            })
                    })
                    .collect();
                let at = event.position + point(px(0.0), px(16.0));
                this.open_menu(MenuKind::Pass, items, None, at, window, cx);
            }),
        )
        .child(
            ui::button("graph-pass", shown)
                .ghost()
                .small()
                .icon(Icon::ChevronDown),
        );
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                ui::line("Draws", text::CONTROL, theme::ink2())
                    .w(px(FIELD_LABEL))
                    .flex_none(),
            )
            .child(trigger)
    }

    /// A signal's curve, drawn across to draw it anew, and its moments read
    /// from a sound of its sequence.
    fn signal_column(&self, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let holds = &node.holds;
        let mut column = div().flex().flex_col().gap(px(8.0));
        if node.is_missing() {
            return column.child(note(node.detail.clone(), theme::neg()));
        }
        column = column.child(note(
            "A number from 0 to 1 over the shot's time. The prompt says what it is of; this says when, and how much.".into(),
            theme::ink2(),
        ));
        let values: Vec<f64> = holds["values"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|value| value.as_f64().unwrap_or(0.0))
            .collect();
        let frames = holds["frames"].as_u64().unwrap_or(1).max(1) as usize;
        let clip = holds["clip"].as_str().unwrap_or_default().to_owned();
        let drawn: Option<Vec<(usize, f64)>> = self
            .drawing
            .as_ref()
            .filter(|drawing| drawing.clip == clip)
            .map(|drawing| {
                drawing
                    .drawn
                    .iter()
                    .map(|(at, value)| (*at, *value))
                    .collect()
            });
        let bounds = self.curve_bounds.clone();
        let press_clip = clip.clone();
        column = column.child(ui::tip_on(
            div()
                .id("graph-signal-curve")
                .h(px(CURVE_HEIGHT))
                .w_full()
                .rounded(px(radius::CONTROL))
                .bg(theme::well())
                .border_1()
                .border_color(theme::line2())
                .cursor(CursorStyle::Crosshair)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.start_curve(press_clip.clone(), frames, event.position, window, cx);
                    }),
                )
                .child(
                    canvas(
                        move |area, _, _| bounds.set(area),
                        move |area, _, window, _| {
                            paint_signal(window, area, &values, frames, drawn.as_deref())
                        },
                    )
                    .size_full(),
                ),
            Some(ui::Tip::new(
                "Drag across to draw the signal: 0 at the foot, 1 at the top",
            )),
        ));
        column = column.child(note(
            format!(
                "{}. Drag across it to draw it anew; it is the clip's value in the project, and is undone as any edit.",
                node.detail
            ),
            theme::ink3(),
        ));
        // Its moments read from a sound of its sequence: 1 for a few frames
        // from where each sound starts.
        let sounds = holds["sounds"].as_array().cloned().unwrap_or_default();
        if sounds.is_empty() {
            return column;
        }
        let sound_open = self
            .menu
            .as_ref()
            .is_some_and(|open| open.kind == MenuKind::Sound);
        let trigger = ui::trigger(
            "graph-sound-trigger",
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                if this.close_menu(MenuKind::Sound, window, cx) {
                    return;
                }
                let this_entity = cx.weak_entity();
                let items = sounds
                    .iter()
                    .map(|sound| {
                        let of = sound["clip"].as_str().unwrap_or_default().to_owned();
                        let name = sound["name"]
                            .as_str()
                            .filter(|name| !name.is_empty())
                            .map_or_else(|| of.clone(), str::to_owned);
                        let (this, clip) = (this_entity.clone(), clip.clone());
                        ui::MenuItem::new(name).on_choose(move |_, cx| {
                            let command = Command::SignalFromSound {
                                clip: clip.clone(),
                                of: of.clone(),
                            };
                            // A send refused is said by the session itself.
                            let _ = this.update(cx, |graph, cx| {
                                graph.session.update(cx, |session, _| session.send(command))
                            });
                        })
                    })
                    .collect();
                let at = event.position + point(px(0.0), px(16.0));
                this.open_menu(
                    MenuKind::Sound,
                    items,
                    Some("Where this sound's sounds start".to_owned()),
                    at,
                    window,
                    cx,
                );
            }),
        )
        .child(
            ui::button("graph-signal-sound", "From a sound")
                .ghost()
                .icon(Icon::ChevronDown)
                .when(!sound_open, |button| {
                    button.tooltip("Set it to 1 for three frames from each place a sound of its sequence starts, the beats of a track or a ball on a floor, and 0 between")
                }),
        );
        column.child(div().flex().child(trigger))
    }

    /// Reading `clip` from footage, for its way (`moves`) or its matte
    /// (`matte`): its first frame drawn, pointed on, and followed.
    fn pointing(&self, clip: &str, of: ShotOf, cx: &mut Context<Self>) -> gpui::Div {
        let moves = of == ShotOf::Moves;
        let mine = self.pointing.clip == clip && self.pointing.of == Some(of);
        let stage = if mine {
            self.pointing.stage
        } else {
            PointStage::Idle
        };
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(section("From footage"));
        if mine && !self.pointing.says.is_empty() {
            column = column.child(note(
                self.pointing.says.clone(),
                if self.pointing.failed {
                    theme::neg()
                } else {
                    theme::ink2()
                },
            ));
        }
        match stage {
            PointStage::Pointing | PointStage::Following => {
                let following = stage == PointStage::Following;
                let image = self
                    .pointing
                    .picture
                    .as_deref()
                    .and_then(|path| self.still(path));
                column = column.child(self.point_picture(image, following, cx));
                if following {
                    column = column.child(note(
                        if moves {
                            "Following it through the clip…"
                        } else {
                            "Cutting it out through the clip…"
                        }
                        .into(),
                        theme::ink3(),
                    ));
                } else if moves {
                    column = column.child(note(
                        "Press on what the clip should follow: a corner or a mark, or a thing that stands out from what is behind it. The clip itself is not in this picture.".into(),
                        theme::ink3(),
                    ));
                } else {
                    column = column
                        .child(note(
                            "Press on what to cut out, in as many places as tell it from the rest; hold Alt, or use the other button, on what is taken for it and is not it.".into(),
                            theme::ink3(),
                        ))
                        .child(
                            div().flex().child(
                                ui::button("graph-cut-out", "Cut out")
                                    .ghost()
                                    .disabled(self.pointing.on.is_empty())
                                    .tooltip("Mask the clip to what the points are on, through the whole clip: its matte is then that thing in white")
                                    .on_click(cx.listener(|this, _, _, cx| this.follow(cx))),
                            ),
                        );
                }
            }
            PointStage::Drawing => {
                column = column.child(note(
                    "Drawing the clip's first frame…".into(),
                    theme::ink3(),
                ));
            }
            PointStage::Idle | PointStage::Said => {
                let clip = clip.to_owned();
                column = column.child(
                    div().flex().child(
                        ui::button(
                            if moves {
                                "graph-follow-picture"
                            } else {
                                "graph-cut-picture"
                            },
                            if moves {
                                "Follow the picture"
                            } else {
                                "Cut out of the picture"
                            },
                        )
                        .ghost()
                        .tooltip(if moves {
                            "Point at something in the footage under this clip, and have the clip follow it for as long as it runs: its x and y, keyed"
                        } else {
                            "Point at a thing this clip shows, and mask the clip to it as it moves and changes shape: the matte is then that thing"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_picture(clip.clone(), of, cx)
                        })),
                    ),
                );
            }
        }
        column
    }

    /// The clip's first frame to point on: a press marks what it should
    /// follow, or a point on the thing (a ring); Alt or the other button one
    /// off it (a cross).
    fn point_picture(
        &self,
        image: Option<Arc<RenderImage>>,
        following: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = PICTURE_MOST.0;
        let height = image
            .as_ref()
            .map(|image| crate::artwork::image_size(image))
            .filter(|size| size.0 > 0)
            .map_or(width * 9.0 / 16.0, |(w, h)| width * h as f32 / w as f32);
        let bounds = self.point_bounds.clone();
        let (on, off) = (self.pointing.on.clone(), self.pointing.off.clone());
        let path_ink = theme::playhead();
        let off_ink = theme::neg();
        div()
            .id("graph-point-picture")
            .w(px(width))
            .h(px(height))
            .rounded(px(radius::CONTROL))
            .overflow_hidden()
            .bg(theme::well())
            .when(!following, |picture| picture.cursor(CursorStyle::Crosshair))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.point(event, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.point(event, cx)
                }),
            )
            .child(
                canvas(
                    move |area, _, _| bounds.set(area),
                    move |area, _, window, _| {
                        let Some(image) = image.clone() else {
                            return;
                        };
                        let size = crate::artwork::image_size(&image);
                        let room = [
                            f32::from(area.origin.x),
                            f32::from(area.origin.y),
                            f32::from(area.size.width),
                            f32::from(area.size.height),
                        ];
                        let [left, top, w, h] = fitted_in(room, size);
                        let placed =
                            Bounds::new(point(px(left), px(top)), gpui::size(px(w), px(h)));
                        if let Err(error) =
                            window.paint_image(placed, gpui::Corners::default(), image, 0, false)
                        {
                            log::warn!("the frame to point on did not draw: {error}");
                        }
                        if following {
                            window.paint_quad(fill(placed, gpui::rgba(0x00000060)));
                        }
                        let place = |(x, y): (f64, f64)| {
                            point(px(left + x as f32 * w), px(top + y as f32 * h))
                        };
                        // A point on the thing as a ring, one off it as a cross:
                        // told by shape as well as by hue.
                        for at in &on {
                            let at = place(*at);
                            paint_ring_line(window, at, 5.0, gpui::rgba(0x000000a0), 4.0);
                            paint_ring_line(window, at, 5.0, path_ink, 2.0);
                        }
                        for at in &off {
                            let at = place(*at);
                            paint_cross(window, at, gpui::rgba(0x000000a0), 4.0);
                            paint_cross(window, at, off_ink, 2.0);
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    /// A node of a model's workflow: each input, a link said, a blank and
    /// what fills it, a value set by a switch, a menu or a field.
    fn part_column(&self, graph: &Recipe, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(8.0));
        if !node.detail.is_empty() {
            column = column.child(ui::label(
                node.detail.clone(),
                text::MONO_SMALL,
                theme::ink3(),
            ));
        }
        let part = value_text(&node.holds["id"]);
        let row = |name: &str, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    ui::line(name.to_owned(), text::CONTROL, theme::ink2())
                        .w(px(INPUT_LABEL))
                        .flex_none(),
                )
                .child(div().flex_1().min_w_0().child(control))
        };
        for (index, input) in node.holds["inputs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            let name = input["name"].as_str().unwrap_or_default().to_owned();
            match input_kind(&input) {
                InputKind::Link => {
                    // Where it comes from: the graph's own shape, said; wired, it is re-pointed.
                    let from = input["link"]
                        .as_array()
                        .and_then(|link| link.first())
                        .map(value_text)
                        .unwrap_or_default();
                    let title = graph
                        .node(&part_id(&from))
                        .map_or("a node".to_owned(), |node| node.title.clone());
                    column = column.child(row(
                        &name,
                        ui::label(format!("← {title} #{from}"), text::CONTROL, theme::ink3())
                            .into_any_element(),
                    ));
                }
                InputKind::Blank => {
                    // The steps' to fill: marked, with what fills it and what the line gives now.
                    column = column.child(row(
                        &name,
                        ui::label(
                            shown_value(&input["value"]),
                            text::TIMECODE,
                            theme::accent_ink(),
                        )
                        .into_any_element(),
                    ));
                    let says: Vec<String> = strings(&input["blanks"])
                        .iter()
                        .filter_map(|blank| {
                            let says = self.workflow.as_ref()?.blank_says(blank);
                            (!says.is_empty()).then_some(says)
                        })
                        .collect();
                    if !says.is_empty() {
                        column = column.child(note(says.join("\n"), theme::ink3()));
                    }
                }
                InputKind::Value => {
                    let held = input["value"].clone();
                    if let Value::Bool(on) = held {
                        let (part, input_name) = (part.clone(), name.clone());
                        let this = cx.weak_entity();
                        column = column.child(row(
                            &name,
                            ui::switch(
                                SharedString::from(format!("graph-switch-{index}")),
                                on,
                                move |on, _, cx| {
                                    let change = WorkflowEdit {
                                        node: part.clone(),
                                        input: input_name.clone(),
                                        value: Value::Bool(on),
                                        ..WorkflowEdit::default()
                                    };
                                    let _ = this
                                        .update(cx, |graph, cx| graph.edit_workflow(change, cx));
                                },
                            )
                            .into_any_element(),
                        ));
                        continue;
                    }
                    if !held.is_string() && !held.is_number() {
                        // A list or a table the workflow sets whole: shown, and the file's to change.
                        column = column.child(row(
                            &name,
                            ui::label(shown_value(&held), text::TIMECODE, theme::ink2())
                                .into_any_element(),
                        ));
                        continue;
                    }
                    let choices = input["choices"].as_array().cloned().unwrap_or_default();
                    if !choices.is_empty() {
                        // One of the values the server takes there: chosen, not typed.
                        let (part, input_name) = (part.clone(), name.clone());
                        let trigger = ui::trigger(
                            ElementId::Name(format!("graph-choice-trigger-{index}").into()),
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                if this.close_menu(MenuKind::Choice, window, cx) {
                                    return;
                                }
                                let this_entity = cx.weak_entity();
                                let items = choices
                                    .iter()
                                    .map(|choice| {
                                        let chosen = *choice == held;
                                        let (this, part, input_name, choice) = (
                                            this_entity.clone(),
                                            part.clone(),
                                            input_name.clone(),
                                            choice.clone(),
                                        );
                                        ui::MenuItem::new(shown_value(&choice))
                                            .checked(chosen)
                                            .on_choose(move |_, cx| {
                                                if chosen {
                                                    return;
                                                }
                                                let change = WorkflowEdit {
                                                    node: part.clone(),
                                                    input: input_name.clone(),
                                                    value: choice.clone(),
                                                    ..WorkflowEdit::default()
                                                };
                                                let _ = this.update(cx, |graph, cx| {
                                                    graph.edit_workflow(change, cx)
                                                });
                                            })
                                    })
                                    .collect();
                                let at = event.position + point(px(0.0), px(16.0));
                                this.open_menu(MenuKind::Choice, items, None, at, window, cx);
                            }),
                        )
                        .child(
                            ui::button(
                                ElementId::Name(format!("graph-choice-{index}").into()),
                                shown_value(&input["value"]),
                            )
                            .ghost()
                            .small()
                            .icon(Icon::ChevronDown),
                        );
                        column = column.child(row(&name, trigger.into_any_element()));
                        continue;
                    }
                    if let Some(field) = self.field(&name) {
                        let mut control = field_box(field);
                        if let Some(tip) = field.tip.clone() {
                            control = control
                                .child(ui::label(tip, text::META, theme::ink3()).pt(px(2.0)));
                        }
                        column = column.child(row(&name, control.into_any_element()));
                    }
                }
            }
        }
        column
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
                                    this.edit(
                                        RecipeEdit {
                                            line,
                                            key: port.clone(),
                                            nth,
                                            ..RecipeEdit::default()
                                        },
                                        cx,
                                    );
                                },
                            )),
                        )
                    }),
            );
        }
        reads
    }

    /// The shot's takes: each kept, by its picture, the one the shot is
    /// ticked, another chosen with a press; held to their guide on asking.
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
                .gap(px(8.0))
                .min_h(px(if picture.is_some() {
                    TAKE_PICTURE.1 + 8.0
                } else {
                    28.0
                }))
                .px(px(if picture.is_some() { 4.0 } else { 8.0 }))
                .py(px(2.0))
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
                                    crate::artwork::paint_fitted(
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
            column = column.child(ui::tip_on(row, Some(ui::Tip::new(tip))));
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
            ),
        );
        let said = match &self.reading {
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
        };
        if let Some((said, color)) = said {
            column = column.child(note(said, color));
        }
        column.child(ui::label(
            "Every take is kept. Make again writes the next beside these.",
            text::META,
            theme::ink3(),
        ))
    }

    /// What opens over the graph, on a dim of it. The plan and Cast and
    /// Styles are as large as they make themselves; Settings fills what the
    /// window would give it ([`ui::modal_bounds`]), here of the graph's room
    /// (`room`, as large as the graph is drawn).
    fn over_box(&self, over: &Over, room: (f32, f32), cx: &mut Context<Self>) -> AnyElement {
        let (view, sized): (AnyElement, Option<gpui::Size<Pixels>>) = match over {
            Over::Plan(plan) => (plan.clone().into_any_element(), None),
            Over::Cast(cast) => (cast.clone().into_any_element(), None),
            Over::Settings(settings) => {
                let (width, height) = crate::settings::MOST;
                let placed =
                    ui::modal_bounds(size(px(width), px(height)), size(px(room.0), px(room.1)));
                (settings.clone().into_any_element(), Some(placed.size))
            }
        };
        let rise = scene::PAGE_RISE;
        div()
            .id("graph-over")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::with_alpha(theme::overlay_screen(), 0.35))
            .occlude()
            // A press on the dim gives the graph back.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_over(window, cx)
                }),
            )
            .child(
                div()
                    .id("graph-over-panel")
                    .rounded(px(radius::PANEL))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme::line2())
                    .shadow_lg()
                    .when_some(sized, |panel, sized| {
                        panel.w(sized.width).h(sized.height).flex().flex_col()
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(view)
                    .with_animation(
                        "graph-over-rise",
                        motion::animation(scene::PAGE),
                        move |panel, t| panel.opacity(t).mt(px(rise * (1.0 - t))),
                    ),
            )
            .into_any_element()
    }
}

/// What a person typed into `fields` and has not set yet, by each field's
/// name, and whether it has the keys: what the node's fields made again
/// start with. A field that says what the steps say has nothing to carry;
/// Add, anything in it.
fn carried_typing(fields: &[Field], window: &Window, cx: &App) -> HashMap<String, (String, bool)> {
    fields
        .iter()
        .filter(|field| {
            if field.kind == FieldKind::Add {
                !field.input.read(cx).text().is_empty()
            } else {
                typed(field, cx) != field.shown
            }
        })
        .map(|field| {
            let input = field.input.read(cx);
            let focused = input.focus_handle(cx).is_focused(window);
            (field.key.clone(), (input.text().to_owned(), focused))
        })
        .collect()
}

/// What a field says now, as its line would hold it: a prompt one line.
fn typed(field: &Field, cx: &App) -> String {
    let words = field.input.read(cx).text();
    if field.key == "prompt" {
        one_line(words)
    } else {
        words.trim().to_owned()
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

/// A note in the column.
fn note(words: String, color: Rgba) -> gpui::Div {
    ui::label(words, text::CONTROL, color)
}

/// A field's box: a well, with its words.
fn field_box(field: &Field) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .px(px(8.0))
        .py(px(5.0))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line2())
        .when(field.multiline, |input| input.min_h(px(88.0)))
        .child(field.input.clone())
}

/// A field's name and its box, and what follows the box (a seed's die).
fn field_row(field: &Field, after: Option<AnyElement>) -> gpui::Div {
    let mut name = ui::line(field.label.clone(), text::CONTROL, theme::ink2())
        .w(px(FIELD_LABEL))
        .flex_none();
    if let Some(tip) = &field.tip {
        let tip = tip.clone();
        name = div().child(ui::tipped(
            ElementId::Name(format!("graph-field-tip-{}", field.key).into()),
            ui::Tip::new(tip),
            name,
        ));
    }
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(name)
        .child(field_box(field))
        .children(after)
}

/// A picture in the column, at its own shape, as wide as the column at most.
fn picture_box(image: Arc<RenderImage>) -> gpui::Div {
    let (width, height) = crate::artwork::image_size(&image);
    let scale = if width == 0 || height == 0 {
        1.0
    } else {
        (PICTURE_MOST.0 / width as f32).min(PICTURE_MOST.1 / height as f32)
    };
    div()
        .w(px(width as f32 * scale))
        .h(px(height as f32 * scale))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    crate::artwork::paint_fitted(
                        window,
                        bounds,
                        image.clone(),
                        false,
                        px(radius::CONTROL),
                    )
                },
            )
            .size_full(),
        )
}

/// An edge's name beside where it comes into its card, on the canvas's
/// own colour, so an edge that passes behind it does not strike it through.
fn port_label(port: &str, end: (f32, f32), touches: bool) -> AnyElement {
    const MOST: f32 = 160.0;
    div()
        .absolute()
        .left(px(end.0 - 9.0 - MOST))
        .top(px(end.1 - 19.0))
        .w(px(MOST))
        .h(px(14.0))
        .flex()
        .justify_end()
        .child(
            div()
                .px(px(4.0))
                .h(px(14.0))
                .flex()
                .items_center()
                .rounded(px(4.0))
                .bg(theme::with_alpha(theme::well(), 0.86))
                .child(
                    theme::type_style(div(), text::META)
                        .line_height(px(14.0))
                        .text_color(if touches { theme::ink3() } else { theme::dis() })
                        .whitespace_nowrap()
                        .child(port.to_owned()),
                ),
        )
        .into_any_element()
}

/// A signal's curve or a clip's path drawn under a card's lines, `under`
/// points down it.
fn card_curve(node: &GraphNode, color: Rgba, under: f32, zoom: f32) -> AnyElement {
    let holds = &node.holds;
    let values: Vec<f64> = holds["values"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|value| value.as_f64().unwrap_or(0.0))
        .collect();
    let aspect = holds["aspect"].as_f64().unwrap_or(16.0 / 9.0).max(0.1);
    let path: Option<Vec<(f64, f64)>> = holds["path"]
        .as_str()
        .and_then(crate::stage::overlay::parse_path)
        .map(|points| {
            if points.len() >= 2 {
                crate::stage::overlay::MotionPath::new(points, aspect).samples()
            } else {
                points
            }
        });
    let signal = node.kind == "signal";
    let ground = theme::with_alpha(theme::surround_dim(), 0.6);
    let node_fill = theme::card();
    div()
        .absolute()
        .left(px(12.0 * zoom))
        .top(px((under + 3.0) * zoom))
        .w(px((CARD_W - 24.0) * zoom))
        .h(px((CARD_CURVE - 12.0) * zoom))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                    let (width, height) =
                        (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(px(left - 4.0 * zoom), px(top - 3.0 * zoom)),
                                size(px(width + 8.0 * zoom), px(height + 6.0 * zoom)),
                            ),
                            ground,
                        )
                        .corner_radii(px(radius::SMALL * zoom)),
                    );
                    if signal {
                        let line = signal_line(&values, [left, top, width, height]);
                        paint_under(window, &line, top + height, theme::with_alpha(color, 0.16));
                        paint_line(window, &line, color, 1.25);
                        return;
                    }
                    let Some(path) = &path else {
                        return;
                    };
                    // In frame widths across and frame heights down, as the clip's x and y are.
                    let drawn: Vec<(f64, f64)> =
                        path.iter().map(|(x, y)| (x * aspect, *y)).collect();
                    let (mut low, mut high) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
                    for (x, y) in &drawn {
                        low = (low.0.min(*x), low.1.min(*y));
                        high = (high.0.max(*x), high.1.max(*y));
                    }
                    let fit = (f64::from(width) / (high.0 - low.0).max(1e-3))
                        .min(f64::from(height) / (high.1 - low.1).max(1e-3));
                    let middle = ((low.0 + high.0) / 2.0, (low.1 + high.1) / 2.0);
                    let line: Vec<(f32, f32)> = drawn
                        .iter()
                        .map(|(x, y)| {
                            (
                                left + width / 2.0 + ((x - middle.0) * fit) as f32,
                                top + height / 2.0 + ((y - middle.1) * fit) as f32,
                            )
                        })
                        .collect();
                    paint_line(window, &line, color, 1.25);
                    if let (Some(first), Some(last)) = (line.first(), line.last()) {
                        paint_ring(
                            window,
                            point(px(first.0), px(first.1)),
                            2.5,
                            node_fill,
                            Some(color),
                        );
                        paint_ring(window, point(px(last.0), px(last.1)), 2.5, color, None);
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// The plot of a curve drawn `bounds` big: inside its edge.
fn curve_plot(bounds: Bounds<Pixels>) -> [f32; 4] {
    [
        f32::from(bounds.origin.x) + CURVE_INSET,
        f32::from(bounds.origin.y) + CURVE_INSET,
        f32::from(bounds.size.width) - 2.0 * CURVE_INSET,
        f32::from(bounds.size.height) - 2.0 * CURVE_INSET,
    ]
}

/// A signal's curve: its floor, middle and top as hairlines, its values
/// washed underneath, and a curve being drawn over it in ink, the one it
/// was falling back to 45%.
fn paint_signal(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    values: &[f64],
    frames: usize,
    drawn: Option<&[(usize, f64)]>,
) {
    let rect = curve_plot(bounds);
    let [left, top, width, height] = rect;
    for share in [0.0f32, 0.5, 1.0] {
        let y = (top + height - height * share).round();
        window.paint_quad(fill(
            Bounds::new(point(px(left), px(y)), size(px(width), px(1.0))),
            theme::line(),
        ));
    }
    let ink = theme::warn();
    let line = signal_line(values, rect);
    let drawing = drawn.is_some_and(|drawn| !drawn.is_empty());
    if !line.is_empty() {
        paint_under(window, &line, top + height, theme::with_alpha(ink, 0.16));
        let was = if drawing {
            theme::with_alpha(ink, 0.45)
        } else {
            ink
        };
        paint_line(window, &line, was, 1.5);
    }
    let Some(drawn) = drawn.filter(|drawn| !drawn.is_empty()) else {
        return;
    };
    let last = frames.max(2) - 1;
    let points: Vec<(f32, f32)> = drawn
        .iter()
        .map(|(at, value)| {
            (
                left + width * *at as f32 / last as f32,
                top + height - height * *value as f32,
            )
        })
        .collect();
    if points.len() == 1 {
        paint_ring(
            window,
            point(px(points[0].0), px(points[0].1)),
            1.5,
            theme::ink(),
            None,
        );
    } else {
        paint_line(window, &points, theme::ink(), 1.5);
    }
}

/// A line through `points`.
fn paint_line(window: &mut Window, points: &[(f32, f32)], color: Rgba, width: f32) {
    if points.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(point(px(points[0].0), px(points[0].1)));
    for (x, y) in &points[1..] {
        path.line_to(point(px(*x), px(*y)));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// The room under a line down to `floor`, washed.
fn paint_under(window: &mut Window, points: &[(f32, f32)], floor: f32, color: Rgba) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return;
    };
    let mut path = PathBuilder::fill();
    path.move_to(point(px(first.0), px(floor)));
    for (x, y) in points {
        path.line_to(point(px(*x), px(*y)));
    }
    path.line_to(point(px(last.0), px(floor)));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// A dot `radius` round at `at`, filled, with an edge in `edge` if any.
fn paint_ring(
    window: &mut Window,
    at: Point<Pixels>,
    radius: f32,
    fill_color: Rgba,
    edge: Option<Rgba>,
) {
    let bounds = Bounds::new(
        at - point(px(radius), px(radius)),
        size(px(2.0 * radius), px(2.0 * radius)),
    );
    let mut quad = fill(bounds, fill_color).corner_radii(px(radius));
    if let Some(edge) = edge {
        quad = quad.border_widths(px(1.5)).border_color(edge);
    }
    window.paint_quad(quad);
}

/// A ring `radius` round at `at`, its line `width` wide.
fn paint_ring_line(window: &mut Window, at: Point<Pixels>, radius: f32, color: Rgba, width: f32) {
    let outer = radius + width / 2.0;
    let bounds = Bounds::new(
        at - point(px(outer), px(outer)),
        size(px(2.0 * outer), px(2.0 * outer)),
    );
    window.paint_quad(
        fill(bounds, gpui::transparent_black())
            .corner_radii(px(outer))
            .border_widths(px(width))
            .border_color(color),
    );
}

/// A cross at `at`, its arms 4 points out.
fn paint_cross(window: &mut Window, at: Point<Pixels>, color: Rgba, width: f32) {
    for (from, to) in [((-4.0, -4.0), (4.0, 4.0)), ((-4.0, 4.0), (4.0, -4.0))] {
        let mut path = PathBuilder::stroke(px(width));
        path.move_to(at + point(px(from.0), px(from.1)));
        path.line_to(at + point(px(to.0), px(to.1)));
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

/// `element`'s type set in `style` at `zoom`.
fn zoomed(element: gpui::Div, style: theme::TypeStyle, zoom: f32) -> gpui::Div {
    element
        .text_size(px(style.px * zoom))
        .line_height(px(style.line * zoom))
}

impl EventEmitter<DismissEvent> for ShotGraph {}

impl Focusable for ShotGraph {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.canvas_focus.clone()
    }
}

impl Render for ShotGraph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (width, height) = graph_size((f32::from(viewport.width), f32::from(viewport.height)));
        let room = (width - INSPECTOR - 1.0, height - HEAD - 1.0);
        if room != self.room {
            self.room = room;
            if self.fitted {
                self.settle();
            }
        }
        for image in std::mem::take(&mut self.dropped) {
            // gpui 0.2.2 answers Ok always: one not in the atlas is no matter.
            let _ = window.drop_image(image);
        }
        self.follow_typing(window, cx);
        let head = self.head(cx);
        let canvas = self.canvas(window, cx);
        let inspector = self.inspector(cx);
        // A menu, over all of it.
        let menu = self.menu.as_ref().map(|open| {
            deferred(
                anchored()
                    .position(open.at)
                    .snap_to_window_with_margin(px(8.0))
                    .child(open.menu.clone()),
            )
            .with_priority(2)
        });
        let over = self
            .over
            .as_ref()
            .map(|over| self.over_box(over, (width, height), cx));
        let covered = self.over.is_some();
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _, cx| this.dismiss(cx)))
            // A press on the dim around the panel gives the window back.
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                if !covered && this.menu.is_none() {
                    this.dismiss(cx)
                }
            }))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::middle_up))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::middle_up))
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
            .children(over)
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
}
