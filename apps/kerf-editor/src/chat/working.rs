//! While Kerf works, under your request ([`ChatView::working`]): "Working"
//! with a band of light passing over it, the time so far and the step it
//! is on; its last steps; the first cut it is making, as a card at work;
//! the footage it starts on; three dots until it says anything; and its
//! answer as it streams. And what the view follows of the turn for them,
//! as the session changes ([`ChatView::follow_turn`]) and as it draws
//! ([`ChatView::follow_newest`], [`ChatView::follow_clock`]).

use super::picture::{cut_frames, poster_layer, PictureState, CARD_CROSSFADE, CARD_DONE};
use super::script::script_summary;
use super::ChatView;
use crate::icons::Icon;
use crate::model::TimelineShow;
use crate::motion;
use crate::store::{Step, StepState};
use crate::theme::{self, motion as scene, text};
use crate::ui;
use gpui::{
    div, linear_color_stop, linear_gradient, prelude::*, px, AnimationExt, AnyElement, App,
    Context, ElementId, Window,
};
use std::time::{Duration, Instant};

/// A footage tile's size (`theme::layout`).
pub use crate::theme::layout::FOOTAGE_TILE;
/// The drafting dots' beat (`theme::motion`).
pub use crate::theme::motion::DRAFT_BEAT;
/// The most footage tiles that gather.
pub const FOOTAGE_MOST: usize = 8;

/// The clock beside "Working": minutes and seconds so far, "0:07", "12:40".
pub fn work_clock(seconds: f64) -> String {
    let whole = if seconds.is_finite() {
        seconds.max(0.0).floor() as u64
    } else {
        0
    };
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// Which of the three drafting dots is lit `elapsed` into the pulse; None
/// under Reduce Motion, when all three hold still.
pub fn draft_dot(elapsed: Duration, reduced: bool) -> Option<usize> {
    if reduced {
        return None;
    }
    Some((elapsed.as_millis() / DRAFT_BEAT.as_millis().max(1)) as usize % 3)
}

impl ChatView {
    /// Under Reduce Motion, the working clock's one timer: it draws the
    /// conversation again once each `CLOCK_FRAME` while Kerf works, and
    /// stops when it does not (or motion is on again: the row's own frames
    /// move the clock then). Asking again while it runs starts no other.
    fn keep_clock(&mut self, cx: &mut Context<Self>) {
        if self.clock.is_some() {
            return;
        }
        self.clock = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(scene::CLOCK_FRAME).await;
            let going = this
                .update(cx, |this, cx| {
                    let busy = this.session.read(cx).state.work.busy();
                    if busy && motion::reduced() {
                        cx.notify();
                    } else {
                        this.clock = None;
                    }
                    busy && motion::reduced()
                })
                .unwrap_or(false);
            if !going {
                break;
            }
        }));
    }

    /// As the conversation draws the working row under Reduce Motion, the
    /// clock's timer is kept ([`Self::keep_clock`]). Reduce Motion is read
    /// as it draws: Settings turns it on or off with only the window drawn
    /// again to say so.
    pub(super) fn follow_clock(&mut self, cx: &mut Context<Self>) {
        if motion::reduced() && self.working_shows(cx) {
            self.keep_clock(cx);
        }
    }

    /// Whether the working row shows: a turn under way that has started,
    /// in the conversation open (the list draws it there only).
    fn working_shows(&self, cx: &App) -> bool {
        let session = self.session.read(cx);
        let work = &session.state.work;
        work.busy() && work.started.is_some() && work.conversation == session.conversation
    }

    /// What Kerf is doing now, under your request: "Working", the time so
    /// far and the step it is on; its last steps; the first cut it is
    /// making, as a card at work; the footage it starts on; three dots
    /// until it says anything; and its answer as it streams.
    pub(super) fn working(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let started = self.session.read(cx).state.work.started?;
        // The clock goes on once a second (under Reduce Motion, from the
        // timer `follow_clock` keeps as this draws); the light 30 times a
        // second while it shows.
        if !motion::reduced() {
            window.request_animation_frame();
        }
        let session = self.session.read(cx);
        let work = &session.state.work;
        let status = if self.stopping {
            "Stopping…".to_owned()
        } else {
            work.status.clone()
        };
        let head = working_head(started, status);
        let steps = work
            .steps
            .iter()
            .rev()
            .take(4)
            .rev()
            .cloned()
            .collect::<Vec<_>>();
        let answer = work.answer.clone();
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(head);
        if !steps.is_empty() {
            column = column.child(recent_steps(steps));
        }
        // The first cut it is making: its card, at work.
        if let Some(card) = self.building_card(window, cx) {
            column = column.child(card);
        }
        // The footage it will choose from.
        if let Some(tiles) = self.footage_sheet(cx) {
            column = column.child(tiles);
        }
        if answer.trim().is_empty() {
            // Before it has said anything, three dots in turn.
            column = column.child(drafting_dots(draft_dot(
                started.elapsed(),
                motion::reduced(),
            )));
        } else {
            column = column.child(self.prose(&answer, "live", theme::ink(), cx));
        }
        Some(column.into_any_element())
    }

    /// Follows the turn and the first cut it makes as the session changes,
    /// before the conversation is drawn again: footage is wanted as a turn
    /// starts on footage with no cut, gathers once its posters have come,
    /// and goes when it ends; the cut being made is followed (its newest
    /// clip as it draws, [`ChatView::follow_newest`]), and when it is done
    /// it says so.
    pub(super) fn follow_turn(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let started = session.state.work.started;
        if started != self.turn_seen {
            self.turn_seen = started;
            self.footage = None;
            self.taken.clear();
            let timeline = session.timeline();
            self.footage_wanted = started.is_some()
                && !motion::reduced()
                && session.picture_open != Some(true)
                && !timeline.has_clips()
                && !timeline.sources.is_empty();
        }
        if started.is_none() {
            self.footage = None;
            self.footage_wanted = false;
        }
        let cutting = session
            .cutting()
            .then(|| {
                session
                    .first_cut()
                    .map(|(proposal, _)| proposal.proposal_id.clone())
            })
            .flatten();
        match cutting {
            Some(id) => self.building = Some(id),
            None => {
                if let Some(was) = self.building.take() {
                    self.built = Some((was, Instant::now()));
                    self.newest = None;
                    self.turning = None;
                }
            }
        }
        if self
            .built
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= CARD_DONE)
        {
            self.built = None;
        }
        // The picture went back from the stage: the card has it again. (A
        // card just pressed, the picture not yet in, keeps it.)
        let open = session.picture_open;
        if self.picture_seen == Some(true) && open == Some(false) {
            self.stage_proposal = None;
        }
        self.picture_seen = open;
        self.gather_footage(cx);
    }

    /// The newest clip a building cut's picture shows, as its posters come
    /// while the working row shows it: a newer one turns in over the last
    /// (at once under Reduce Motion) until `CARD_CROSSFADE` has gone by, or
    /// the poster it turns from has.
    ///
    /// As the conversation draws, in the same frame as the picture that
    /// shows the newest poster come: a poster lands before the session says
    /// so, and the picture never shows it whole before it turns in.
    pub(super) fn follow_newest(&mut self, cx: &App) {
        if self.building.is_none() || !self.working_shows(cx) {
            return;
        }
        let session = self.session.read(cx);
        let Some((_, cut)) = session.first_cut() else {
            return;
        };
        let (frames, _) = cut_frames(cut);
        let newest = frames
            .iter()
            .rev()
            .find(|(_, source)| session.artwork.poster(source).is_some())
            .map(|(_, source)| source.clone());
        if newest != self.newest {
            if let (Some(old), false) = (self.newest.take(), motion::reduced()) {
                self.turning = Some((old, Instant::now()));
            }
            self.newest = newest;
        }
        if self.turning.as_ref().is_some_and(|(before, at)| {
            at.elapsed() >= CARD_CROSSFADE || session.artwork.poster(before).is_none()
        }) {
            self.turning = None;
        }
    }

    /// The footage a turn wants gathers while the working row shows, once
    /// its posters have come.
    fn gather_footage(&mut self, cx: &App) {
        if !self.footage_wanted || self.footage.is_some() || !self.working_shows(cx) {
            return;
        }
        let session = self.session.read(cx);
        let timeline = session.timeline();
        let posters: Vec<String> = timeline
            .sources
            .iter()
            .filter(|source| session.artwork.poster(&source.id).is_some())
            .take(FOOTAGE_MOST)
            .map(|source| source.id.clone())
            .collect();
        // A folder just brought in has its posters still to come.
        if !timeline.sources.is_empty() && posters.len() >= timeline.sources.len().min(FOOTAGE_MOST)
        {
            self.footage = Some((posters, Instant::now()));
            self.footage_wanted = false;
        }
    }

    /// The card of the first cut Kerf is making: its picture at work, its
    /// title, and nothing to do with it yet.
    fn building_card(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.building.clone()?;
        let (title, cut) = {
            let session = self.session.read(cx);
            let (proposal, cut) = session.first_cut()?;
            (
                script_summary(&proposal.script, proposal.version),
                cut.clone(),
            )
        };
        let state = self.building_state(&id, cx);
        let picture = self.cut_picture(&id, &cut, state, window, cx);
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(theme::space::S8))
                .p(px(10.0))
                .rounded(px(theme::radius::CARD))
                .bg(theme::raised())
                .border_1()
                .border_color(theme::line())
                .child(picture)
                .child(div().px(px(theme::space::S4)).child(ui::line(
                    title,
                    text::BODY_NAME,
                    theme::ink(),
                )))
                .with_animation(
                    "building-card",
                    motion::animation(scene::APPEAR),
                    |card, t| card.opacity(t),
                )
                .into_any_element(),
        )
    }

    /// How a building cut's picture stands: at work, the newest clip placed
    /// turning in over the last ([`ChatView::follow_newest`]).
    fn building_state(&self, id: &str, cx: &App) -> PictureState {
        let mut state = PictureState {
            working: true,
            finished: None,
            ..self.picture_state(id, cx)
        };
        if let Some((before, at)) = &self.turning {
            let t = at.elapsed().as_secs_f32() / CARD_CROSSFADE.as_secs_f32();
            let session = self.session.read(cx);
            if let Some(poster) = session.artwork.poster(before).filter(|_| t < 1.0) {
                state.before = Some((poster, motion::ease(t)));
            }
        }
        state
    }

    /// The footage Kerf starts on, as named poster tiles popping in one
    /// after another; it goes when the turn ends.
    fn footage_sheet(&self, cx: &App) -> Option<AnyElement> {
        let (ids, _) = self.footage.as_ref()?;
        let session = self.session.read(cx);
        let timeline = session.timeline();
        let tiles = ids.iter().enumerate().filter_map(|(index, id)| {
            let poster = session.artwork.poster(id)?;
            let name = timeline.media_name(id);
            let delay = scene::POP_STAGGER.as_secs_f32() * index as f32;
            let pop = scene::POP.as_secs_f32();
            let whole = Duration::from_secs_f32(delay + pop);
            // Gone on from here to become a clip, its place stays.
            let taken = self.taken.contains(id);
            let (tiles, key) = (self.tiles.clone(), id.clone());
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S4))
                    .w(px(FOOTAGE_TILE.0))
                    .when(taken, |tile| tile.invisible())
                    .child(
                        div()
                            .relative()
                            .w(px(FOOTAGE_TILE.0))
                            .h(px(FOOTAGE_TILE.1))
                            .rounded(px(theme::radius::SMALL))
                            .overflow_hidden()
                            .bg(theme::well())
                            .child(poster_layer(poster, theme::radius::SMALL))
                            // Where it is, for what goes on from it.
                            .child(
                                gpui::canvas(
                                    move |bounds, _, _| {
                                        tiles.borrow_mut().insert(key.clone(), bounds);
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            ),
                    )
                    .child(ui::line(name, text::META, theme::ink2()))
                    .with_animation(
                        ElementId::Name(format!("footage-{id}").into()),
                        gpui::Animation::new(motion::duration(whole).max(Duration::from_millis(1))),
                        move |tile, t| {
                            let local = ((t * (delay + pop) - delay) / pop).clamp(0.0, 1.0);
                            let eased = motion::ease(local);
                            tile.opacity(eased)
                                .mt(px(scene::FOOTAGE_RISE * (1.0 - eased)))
                        },
                    ),
            )
        });
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(theme::space::S12))
                .children(tiles)
                .into_any_element(),
        )
    }

    /// Where media `source`'s footage tile shows in the window, as last
    /// laid out, while it is in sight and has not gone on.
    pub fn footage_tile_bounds(&self, source: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        if self.taken.contains(source) {
            return None;
        }
        let bounds = self.tiles.borrow().get(source).copied()?;
        self.in_sight(bounds).then_some(bounds)
    }

    /// Footage `source` has gone on from its tile (to become a clip): its
    /// place stays, empty, until the sheet goes.
    pub fn take_footage(&mut self, source: &str, cx: &mut Context<Self>) {
        if self.taken.insert(source.to_owned()) {
            cx.notify();
        }
    }
}

/// "Working" with a band of light passing over it once each
/// `WORK_SHIMMER` (still under Reduce Motion), the time since `started`,
/// and the step Kerf is on (`status`).
fn working_head(started: Instant, status: String) -> gpui::Div {
    let clock = format!(" · {}", work_clock(started.elapsed().as_secs_f64()));
    let phase = motion::phase(started, scene::WORK_SHIMMER);
    let mut label = ui::label("Working", text::CONTROL_STRONG, theme::ink2())
        .relative()
        .flex_none()
        .overflow_hidden();
    if let Some(at) = motion::shimmer_at(phase, 1.0).filter(|_| !motion::reduced()) {
        // A band of light, brightest at its middle, passing over the word.
        let half = |rising: bool| {
            let (from, to) = if rising { (0.0, 0.28) } else { (0.28, 0.0) };
            div().h_full().w(gpui::relative(0.5)).bg(linear_gradient(
                90.0,
                linear_color_stop(theme::with_alpha(theme::ink(), from), 0.0),
                linear_color_stop(theme::with_alpha(theme::ink(), to), 1.0),
            ))
        };
        label = label.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(gpui::relative(at - 0.2))
                .w(gpui::relative(0.4))
                .flex()
                .child(half(true))
                .child(half(false)),
        );
    }
    div()
        .flex()
        .items_center()
        .min_w_0()
        .child(label)
        .child(ui::label(clock, text::MONO_SMALL, theme::ink3()).flex_none())
        .when(!status.trim().is_empty(), |row| {
            row.child(
                ui::line(status, text::CONTROL, theme::ink3())
                    .ml(px(theme::space::S8))
                    .flex_1()
                    .min_w_0(),
            )
        })
}

/// Kerf's last steps on a hairline, the latest in ink and one that failed
/// in brick.
fn recent_steps(steps: Vec<Step>) -> gpui::Div {
    let latest = steps.len() - 1;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::space::S4))
        .pl(px(10.0))
        .border_l_1()
        .border_color(theme::line2())
        .children(steps.into_iter().enumerate().map(|(index, step)| {
            let (icon, color) = match step.state {
                StepState::Running => (Icon::Dot, theme::activity::kerf()),
                StepState::Done => (Icon::Check, theme::ink3()),
                StepState::Failed => (Icon::Close, theme::neg()),
            };
            let ink = match step.state {
                StepState::Failed => theme::neg(),
                _ if index == latest => theme::ink(),
                _ => theme::ink2(),
            };
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S6))
                .child(icon.view(12.0, color))
                .child(ui::line(step.text, text::CONTROL, ink))
        }))
}

/// Before Kerf has said anything, three dots lit in turn (`lit`; all three
/// under Reduce Motion).
fn drafting_dots(lit: Option<usize>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(3.0))
        .h(px(16.0))
        .pl(px(theme::space::S2))
        .children((0..3).map(|dot| {
            let full = lit.is_none_or(|lit| lit == dot);
            div()
                .size(px(3.0))
                .rounded_full()
                .bg(theme::ink3())
                .opacity(if full { 1.0 } else { 0.45 })
        }))
}
