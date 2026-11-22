//! The makings Kerf asks for in the conversation, as cards under the
//! answer whose turn asked ([`Makings`]): what it asks to make and the
//! plan, with Make and Not now; then the making as it goes, with Stop; once
//! made, Look at it. How a card stands is worked out from what is known of
//! its media ([`card_state`]).

use super::plan::{
    ask_words, notes_block_in, refusal, steps_block_in, takes_words, AskWords, Choose, Plan,
};
use crate::icons::Icon;
use crate::making::clock;
use crate::model::Ask;
use crate::motion;
use crate::selectable;
use crate::session::{BridgeEvent, Session};
use crate::store::Making;
use crate::theme::{self, motion as scene, radius, text};
use crate::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, Context, ElementId, Entity, Rgba, Subscription,
    Window,
};
use kerf_bridge::{Command, Event};
use std::collections::{HashMap, HashSet};

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
        let status = card_status(state, &progress, making.as_ref(), take);
        let shown = Shown {
            key: card_key.clone(),
            rolls,
            take,
            state,
            asked,
            again,
            plan_open,
            made: progress.take,
            connected,
            busy,
            refused: matches!(plan, Some(Err(_))),
        };
        let body = self.card_body(&shown, &words, plan.as_ref(), status);
        let footer = self.card_footer(ask, &shown, cx);
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

    /// A card's words: why Kerf asks, the plan's steps and notes (or why it
    /// cannot be made) while the plan is unfolded, then how the making
    /// stands.
    fn card_body(
        &self,
        shown: &Shown,
        words: &AskWords,
        plan: Option<&Result<Plan, String>>,
        status: Option<(String, Rgba)>,
    ) -> gpui::Div {
        let mut body = div().flex().flex_col().gap(px(10.0));
        // In the conversation, the plan's words can be chosen and copied.
        let choose = |part: &str| -> Choose {
            self.selections
                .as_ref()
                .map(|shared| (shared, format!("plan-{}-{part}", shown.key).into()))
        };
        if shown.plan_open {
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
            match plan {
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
                    if shown.asked {
                        body =
                            body.child(ui::label("Reading the plan…", text::META, theme::ink3()));
                    }
                }
            }
        }
        if let Some((words, color)) = status {
            body = body.child(ui::label(words, text::CONTROL, color));
        }
        body
    }

    /// A card's foot: the plan folded or unfolded, then Not now and Make
    /// while it waits on a yes (Make again once stopped, failed or
    /// declined), Stop while it is made, and Look at it once it is.
    fn card_footer(&self, ask: &Ask, shown: &Shown, cx: &mut Context<Self>) -> gpui::Div {
        let card_key = &shown.key;
        let plan_open = shown.plan_open;
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
        if shown.asked {
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
        if shown.asked || shown.again {
            let label = if shown.asked && shown.rolls > 1 {
                format!("Make {}", shown.rolls)
            } else {
                "Make".to_owned()
            };
            let (ask, take) = (ask.clone(), shown.take);
            footer = footer.child(
                ui::button(ElementId::Name(format!("make-{card_key}").into()), label)
                    .primary()
                    .disabled(!shown.connected || shown.refused)
                    .on_click(cx.listener(move |this, _, _, cx| this.make(&ask, take, cx))),
            );
        }
        if shown.state == CardState::Making {
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
        if shown.state == CardState::Made && shown.made > 0 {
            let look = format!(
                "Take {} of {} is made. Look at it and tell me how it came out against what I asked for.",
                shown.made, ask.media
            );
            footer = footer.child(
                ui::button(
                    ElementId::Name(format!("look-{card_key}").into()),
                    "Look at it",
                )
                .icon(Icon::Eye)
                .disabled(shown.busy || !shown.connected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session.update(cx, |session, cx| {
                        session.ask(&look);
                        cx.notify();
                    });
                })),
            );
        }
        footer
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

/// How a card is drawn, worked out from its ask and its [`Facts`].
struct Shown {
    /// The card's key ([`key`]), which its elements' ids are made of.
    key: String,
    rolls: u32,
    /// The first take its making writes: the plan's, else the ask's.
    take: u32,
    state: CardState,
    /// Waiting on a yes.
    asked: bool,
    /// Stopped, failed or declined: to be made again.
    again: bool,
    plan_open: bool,
    /// The take its making is of, as far as the cards know.
    made: u32,
    connected: bool,
    busy: bool,
    /// There is no plan, and why is said: it cannot be made as things stand.
    refused: bool,
}

/// How a card's making stands, in words and their colour: its take, clock
/// and step's line while it is made; why it failed; or that it was stopped.
fn card_status(
    state: CardState,
    progress: &Progress,
    making: Option<&Making>,
    take: u32,
) -> Option<(String, Rgba)> {
    match state {
        CardState::Making => {
            let elapsed = making
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
    }
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
