//! What travels over the window ([`Flight`]): a frame of the picture going
//! home to the project's row (Manual export, Going home), the clips of a cut
//! Kerf builds flying from their footage to the timeline, the picture's
//! stand-in growing out of what it came from, the picture going back into its
//! card, and what is let go as its landing goes. Where each is at a moment
//! (the `*_flight_at` curves), where it lands as that is laid out now, and
//! how it is drawn over everything ([`flown`]).

use super::panel::{picture_in, picture_within, StageSlot};
use super::{Regions, Workspace};
use crate::motion;
use crate::theme::motion::{STAND_IN_MOST, STAND_IN_RELEASE, TIMELINE_LEAD};
use crate::theme::{self, layout, motion as scene, text};
use crate::ui;
use gpui::{
    div, img, point, prelude::*, px, size, AnyElement, App, Bounds, Context, ObjectFit, Pixels,
    RenderImage, SharedString, Window,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Something travelling over the window, from where it leaves to where it
/// lands, an `img` over everything ([`Workspace::flights_now`]).
pub(super) struct Flight {
    image: Arc<RenderImage>,
    /// `image` is a frame of the picture (the stage's own), which leaves
    /// the atlas once it has landed unless the stage still shows it or
    /// another flight carries it; else a poster, the artwork's, which stays.
    frame: bool,
    from: Bounds<Pixels>,
    to: Bounds<Pixels>,
    /// When it leaves: later than it was asked for when it is staggered.
    started: Instant,
    kind: FlightKind,
}

/// What goes from the panel: all of it, or one of its two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneGoes {
    Panel,
    Picture,
    Timeline,
}

/// What a flight is, which says how it goes and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum FlightKind {
    /// Manual export: a frame of the picture, fading as it goes
    /// ([`export_flight_at`]).
    Export,
    /// Going home: the picture itself, whole and clear until it lands
    /// ([`home_flight_at`]).
    Home,
    /// A clip of a cut being built, from its footage tile (or over the
    /// composer) to where the timeline draws it, ringed and named
    /// ([`clip_flight_at`]); it lands where the clip is drawn now.
    Clip { clip: String, name: SharedString },
    /// The picture's stand-in growing out of what it came from into the
    /// picture, whole on black ([`grow_flight_at`]); it lands where the
    /// picture is to be, and holds there while a picture is coming, until
    /// it comes (the frame shown is no longer `frame`), a clip lands over
    /// it, or nothing is coming: then, `released`, it fades.
    Grow {
        frame: Option<usize>,
        released: Option<Instant>,
    },
    /// The picture going back into cut `proposal`'s card, whole on black
    /// ([`back_flight_at`]); it lands where the card's picture is drawn.
    Back { proposal: String },
    /// A flight (`was`, drawn as it was) whose landing went away before
    /// it got there, a pane going: it holds where it had got to and fades
    /// there from how clear it was ([`let_go_flight_at`]).
    LetGo { was: Box<FlightKind>, clear: f32 },
}

impl FlightKind {
    /// Where it is `elapsed` after it left: how far along and how clear;
    /// None once it has gone.
    pub fn at(&self, elapsed: Duration) -> Option<(f32, f32)> {
        match self {
            FlightKind::Export => export_flight_at(elapsed),
            FlightKind::Home => home_flight_at(elapsed),
            FlightKind::Clip { .. } => clip_flight_at(elapsed),
            FlightKind::Grow { released, .. } => grow_flight_at(
                elapsed,
                released.map(|at| elapsed.saturating_sub(at.elapsed())),
            ),
            FlightKind::Back { .. } => back_flight_at(elapsed),
            FlightKind::LetGo { clear, .. } => let_go_flight_at(elapsed, *clear),
        }
    }

    /// Whether it flies on as `goes` goes from the panel: a clip lands in
    /// the timeline and the picture's stand-in in the picture, so each is
    /// let go when that goes; the rest land outside the panel (a frame
    /// going home, the picture going back into its card) or are let go.
    pub fn flies_on(&self, goes: PaneGoes) -> bool {
        match self {
            FlightKind::Clip { .. } => goes == PaneGoes::Picture,
            FlightKind::Grow { .. } => goes == PaneGoes::Timeline,
            FlightKind::Export
            | FlightKind::Home
            | FlightKind::Back { .. }
            | FlightKind::LetGo { .. } => true,
        }
    }
}

/// A clip a cut being built placed, waiting to fly to where the timeline
/// draws it.
pub(super) struct PendingClip {
    placed: PlacedClip,
    asked: Instant,
}

/// What is drawn of a flight this frame.
pub(super) struct Drawn {
    bounds: Bounds<Pixels>,
    opacity: f32,
    image: Arc<RenderImage>,
    kind: FlightKind,
}

impl Workspace {
    /// Manual export: a frame of the picture flies from the stage to the
    /// project's row in the sidebar, fading as it goes. Not under Reduce
    /// Motion, while Kerf works, with no picture showing, or with the
    /// sidebar away (the row is not there to land on).
    pub(super) fn fly_export_home(&mut self, cx: &mut Context<Self>) {
        if motion::reduced()
            || !self.sidebar_open
            || self.sidebar_gives_way
            || !self.panel_open
            || self.picture_shown.target() < 0.5
        {
            return;
        }
        let session = self.session.read(cx);
        if session.state.work.busy() {
            return;
        }
        let Some(picture) = session.picture.as_ref() else {
            return;
        };
        // From the picture where the stage placed it (else where it would
        // be in its slot), to the project's row where the sidebar laid it.
        let Some(from) = self.stage.read(cx).picture_bounds().or_else(|| {
            self.stage_slot
                .get()
                .map(|slot| picture_within(slot.bounds, picture.width, picture.height))
        }) else {
            return;
        };
        let to = self
            .sidebar
            .read(cx)
            .project_row_bounds()
            .map(landing_in)
            .unwrap_or_else(project_row);
        self.flights.push(Flight {
            image: picture.image.clone(),
            frame: true,
            from,
            to,
            started: Instant::now(),
            kind: FlightKind::Export,
        });
        cx.notify();
    }

    /// Going home: Kerf rendered the whole cut itself in the turn that ended,
    /// so the picture shrinks into the project's row in the sidebar and the
    /// panel goes, the conversation taking the room back. Not under Reduce
    /// Motion (nothing moves; the panel stays), nor with no picture showing or
    /// the sidebar away (no row to go into).
    pub(super) fn go_home(&mut self, cx: &mut Context<Self>) {
        if motion::reduced()
            || !self.sidebar_open
            || self.sidebar_gives_way
            || !self.panel_open
            || self.picture_shown.target() < 0.5
            || self.modal.is_some()
        {
            return;
        }
        let Some(row) = self.sidebar.read(cx).project_row_bounds() else {
            return;
        };
        let session = self.session.read(cx);
        let Some(picture) = session.picture.as_ref() else {
            return;
        };
        let Some(from) = self.stage.read(cx).picture_bounds().or_else(|| {
            self.stage_slot
                .get()
                .map(|slot| picture_within(slot.bounds, picture.width, picture.height))
        }) else {
            return;
        };
        let flight = Flight {
            image: picture.image.clone(),
            frame: true,
            from,
            to: landing_in(row),
            started: Instant::now(),
            kind: FlightKind::Home,
        };
        // What flew into the panel has nowhere to land now.
        self.let_flights_go(PaneGoes::Panel);
        self.flights.push(flight);
        // The panel goes as the picture does, and stays gone: the snapshot
        // the turn's end brings (and any after it) does not open it again,
        // so it does not come back the moment it went. Asked for, it comes.
        self.panel_open = false;
        self.panel_by_hand = true;
        self.panel_shown.go(0.0, scene::STAGE);
        if self.media_open {
            self.media_open = false;
            self.media_shown.go(0.0, scene::STAGE);
        }
        self.settle_soon(cx);
        self.tell_picture_open(cx);
    }

    /// What flies this frame: each flight where it has got to and how
    /// clear, `to` following what it lands on (a clip where the timeline
    /// draws it now, the picture where it is to be, a card where the
    /// conversation lays it). One that has landed and faded goes; a frame
    /// it carried leaves the atlas unless the stage still shows it. Frames
    /// are asked for while any flies.
    pub(super) fn flights_now(
        &mut self,
        picture_home: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Drawn> {
        let now = Instant::now();
        let mut drawn = Vec::new();
        // What was called off goes as what has landed does.
        let mut landed = std::mem::take(&mut self.called_off);
        // A clip of the cut has landed over the picture's stand-in.
        let covered = self.flights.iter().any(|flight| {
            matches!(flight.kind, FlightKind::Clip { .. })
                && now.saturating_duration_since(flight.started) >= scene::FLIGHT
        });
        for flight in std::mem::take(&mut self.flights) {
            // Where it lands, as that is now.
            let to = match &flight.kind {
                FlightKind::Clip { clip, .. } => self.timeline.read(cx).clip_bounds(clip),
                FlightKind::Grow { .. } => picture_home,
                FlightKind::Back { proposal } => self.chat.read(cx).cut_picture_bounds(proposal),
                FlightKind::Export | FlightKind::Home | FlightKind::LetGo { .. } => None,
            }
            .unwrap_or(flight.to);
            let mut flight = Flight { to, ..flight };
            self.hold_or_release(&mut flight, covered, cx);
            let Some(elapsed) = now.checked_duration_since(flight.started) else {
                // Not left yet: it waits its turn, unseen.
                window.request_animation_frame();
                self.flights.push(flight);
                continue;
            };
            match flight.kind.at(elapsed) {
                Some((along, opacity)) => {
                    window.request_animation_frame();
                    drawn.push(Drawn {
                        bounds: lerp_bounds(flight.from, flight.to, along),
                        opacity,
                        image: flight.image.clone(),
                        kind: flight.kind.clone(),
                    });
                    self.flights.push(flight);
                }
                None => landed.push(flight),
            }
        }
        for flight in landed {
            if !flight.frame {
                continue;
            }
            let shown = self
                .session
                .read(cx)
                .picture
                .as_ref()
                .is_some_and(|picture| Arc::ptr_eq(&picture.image, &flight.image));
            let flying = self
                .flights
                .iter()
                .any(|other| Arc::ptr_eq(&other.image, &flight.image));
            if !shown && !flying {
                cx.drop_image(flight.image, Some(window));
            }
        }
        drawn
    }

    /// The picture's stand-in, landed, is let go once its picture comes
    /// (the frame shown changed), a clip of the cut lands over it, nothing
    /// is coming any more (no render, Kerf not at work), or it has held for
    /// [`STAND_IN_MOST`].
    fn hold_or_release(&self, flight: &mut Flight, covered: bool, cx: &App) {
        let FlightKind::Grow { frame, released } = &mut flight.kind else {
            return;
        };
        if released.is_some() {
            return;
        }
        let elapsed = flight.started.elapsed();
        if elapsed < scene::STAGE {
            return;
        }
        let session = self.session.read(cx);
        let shown = session
            .picture
            .as_ref()
            .map(|picture| Arc::as_ptr(&picture.image) as usize);
        let came = shown.is_some() && shown != *frame;
        let coming = session.state.preview.rendering() || session.state.work.busy();
        if came || !coming || covered || elapsed >= scene::STAGE + STAND_IN_MOST {
            *released = Some(Instant::now());
        }
    }

    /// Where the picture is to be once the panel has come to rest: where
    /// the stage placed it last frame (`stage`, as it was laid out then),
    /// moved by as far as the panel had still to come, else where the stage
    /// would place it in its slot as `regions` lay the window out now.
    pub(super) fn picture_home(
        &self,
        stage: Option<StageSlot>,
        regions: &Regions,
        cx: &App,
    ) -> Bounds<Pixels> {
        let session = self.session.read(cx);
        let placed = stage.and_then(|drawn| {
            self.stage
                .read(cx)
                .picture_bounds()
                .map(|picture| (picture, drawn.panel_left))
        });
        match placed {
            Some((picture, (drawn, rest))) => Bounds::new(
                point(picture.origin.x + px(rest - drawn), picture.origin.y),
                picture.size,
            ),
            None => picture_in(
                self.picture_slot(regions),
                crate::stage::shown_aspect(session),
            ),
        }
    }

    /// The picture comes in growing out of `origin` (a cut's card, a
    /// footage tile, over the composer), its stand-in `poster` flying from
    /// there to where the picture will be over `STAGE` as the panel comes
    /// in under it, unseen, and the conversation gives way; the timeline
    /// comes in after it. With nowhere to grow from, nothing to stand in,
    /// or under Reduce Motion, the panel simply comes in over `STAGE`.
    fn grow_picture(
        &mut self,
        origin: Option<Bounds<Pixels>>,
        poster: Option<Arc<RenderImage>>,
        stands_for: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let open = self.panel_open && self.picture_shown.target() > 0.5;
        if open {
            self.open_panel(true, true, scene::PANEL, cx);
            return;
        }
        let duration = if self.panel_open {
            scene::PANEL
        } else {
            scene::STAGE
        };
        self.open_panel(true, !self.panel_open, duration, cx);
        let (Some(origin), Some(poster), false) = (origin, poster, motion::reduced()) else {
            return;
        };
        let now = Instant::now();
        self.going_back = None;
        self.call_off_picture();
        let shown = self
            .session
            .read(cx)
            .picture
            .as_ref()
            .map(|picture| picture.image.clone());
        let frame = shown.as_ref().map(|image| Arc::as_ptr(image) as usize);
        // The stand-in is the frame shown when there was no poster for it.
        let is_frame = shown.is_some_and(|image| Arc::ptr_eq(&image, &poster));
        self.flights.push(Flight {
            image: poster,
            frame: is_frame,
            from: origin,
            // Where the picture will be follows as it is laid out.
            to: origin,
            started: now,
            kind: FlightKind::Grow {
                frame,
                released: None,
            },
        });
        self.grow = Some((now, stands_for));
        cx.notify();
    }

    /// A cut's card was pressed: the picture grows out of its picture.
    pub(super) fn grow_from_card(
        &mut self,
        proposal: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chat = self.chat.read(cx);
        let origin = chat.cut_picture_bounds(proposal);
        let poster = chat.cut_poster(proposal, cx).or_else(|| {
            self.session
                .read(cx)
                .picture
                .as_ref()
                .map(|picture| picture.image.clone())
        });
        self.grow_picture(origin, poster, None, cx);
        window.refresh();
    }

    /// A cut arrived with the panel closed: the picture grows out of what
    /// its first clip came from, its card where it shows, else its
    /// footage's tile (which it takes), else over the composer.
    pub(super) fn grow_for_cut(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let cut = session.live_cut();
        let first = cut
            .map(|(_, cut)| cut)
            .into_iter()
            .chain(std::iter::once(session.timeline()))
            .find_map(first_picture_clip);
        let card = cut.and_then(|(proposal, _)| {
            let chat = self.chat.read(cx);
            chat.cut_picture_bounds(&proposal.proposal_id)
                .map(|bounds| (bounds, chat.cut_poster(&proposal.proposal_id, cx)))
        });
        let (origin, poster, stands_for) = match (card, first) {
            (Some((bounds, poster)), _) => (Some(bounds), poster, None),
            (None, Some((clip, source))) => {
                let poster = session.artwork.poster(&source);
                let tile = self.chat.read(cx).footage_tile_bounds(&source);
                let origin = tile.or_else(|| self.chat.read(cx).over_composer());
                if tile.is_some() {
                    self.chat
                        .update(cx, |chat, cx| chat.take_footage(&source, cx));
                }
                (origin, poster, Some(clip))
            }
            (None, None) => (None, None, None),
        };
        self.grow_picture(origin, poster, stands_for, cx);
    }

    /// The picture is put away while its cut's card is in sight: it goes
    /// back into the card's picture over `PANEL` as the panel goes, the
    /// stage not drawn meanwhile. Not under Reduce Motion.
    pub(super) fn send_picture_back(&mut self, cx: &mut Context<Self>) {
        if motion::reduced() || !self.panel_open || self.picture_shown.target() < 0.5 {
            return;
        }
        let chat = self.chat.read(cx);
        let Some(proposal) = chat.stage_proposal().map(str::to_owned) else {
            return;
        };
        let Some(card) = chat.cut_picture_bounds(&proposal) else {
            return;
        };
        let poster = chat.cut_poster(&proposal, cx);
        let session = self.session.read(cx);
        let image = match (session.picture.as_ref(), poster) {
            (Some(picture), _) => Some((picture.image.clone(), true)),
            (None, poster) => poster.map(|poster| (poster, false)),
        };
        let (Some((image, frame)), Some(from)) = (image, self.stage.read(cx).picture_bounds())
        else {
            return;
        };
        let now = Instant::now();
        self.call_off_picture();
        self.grow = None;
        self.flights.push(Flight {
            image,
            frame,
            from,
            to: card,
            started: now,
            kind: FlightKind::Back { proposal },
        });
        self.going_back = Some(now);
        cx.notify();
    }

    /// The picture's stand-in and the picture going back into its card
    /// are called off, for another to take over.
    fn call_off_picture(&mut self) {
        let (off, on): (Vec<Flight>, Vec<Flight>) = std::mem::take(&mut self.flights)
            .into_iter()
            .partition(|flight| {
                matches!(
                    flight.kind,
                    FlightKind::Grow { .. } | FlightKind::Back { .. }
                )
            });
        self.flights = on;
        self.called_off.extend(off);
    }

    /// As `goes` goes from the panel, what was flying into it (a clip into
    /// the timeline, the picture's stand-in into the picture) has nowhere
    /// to land: each is let go where it has got to, fading there
    /// ([`let_go_flight_at`]), and one yet to leave does not. The picture
    /// going before it had grown, the stage is not drawn as it goes (the
    /// timeline still comes in as it was to).
    pub(super) fn let_flights_go(&mut self, goes: PaneGoes) {
        let now = Instant::now();
        let mut stand_in = false;
        for flight in std::mem::take(&mut self.flights) {
            if flight.kind.flies_on(goes) {
                self.flights.push(flight);
                continue;
            }
            stand_in |= matches!(flight.kind, FlightKind::Grow { .. });
            let Some(elapsed) = now.checked_duration_since(flight.started) else {
                self.called_off.push(flight);
                continue;
            };
            match flight.kind.at(elapsed) {
                Some((along, clear)) => {
                    let here = lerp_bounds(flight.from, flight.to, along);
                    let was = Box::new(flight.kind.clone());
                    self.flights.push(Flight {
                        from: here,
                        to: here,
                        started: now,
                        kind: FlightKind::LetGo { was, clear },
                        ..flight
                    });
                }
                None => self.called_off.push(flight),
            }
        }
        if stand_in {
            self.going_back = Some(now);
        }
        if goes != PaneGoes::Picture {
            self.pending_clips.clear();
        }
    }

    /// As Kerf builds a cut, what it placed since it was last seen waits
    /// to fly from its footage tile to its clip ([`placed_since`]). What
    /// was there as the turn began is not new.
    pub(super) fn follow_cut_build(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if !session.state.work.busy() {
            return;
        }
        let Some((_, cut)) = session.live_cut() else {
            return;
        };
        let (placed, seen) = placed_since(session.timeline(), cut, &self.live_seen);
        self.live_seen = seen;
        let now = Instant::now();
        let grown = self.growing().and_then(|(_, clip)| clip.clone());
        self.pending_clips.extend(
            placed
                .into_iter()
                .filter(|placed| Some(&placed.clip) != grown.as_ref())
                .map(|placed| PendingClip { placed, asked: now }),
        );
    }

    /// The clips waiting to fly leave once the picture has grown and the
    /// timeline draws them, one after another `FLIGHT_STAGGER` apart: from
    /// the footage's tile (which they take), else over the composer. Ones
    /// the timeline does not draw in time, or with no poster, do not fly.
    pub(super) fn launch_clips(&mut self, cx: &mut Context<Self>) {
        if self.pending_clips.is_empty() {
            return;
        }
        let growing = self
            .grow
            .as_ref()
            .is_some_and(|(since, _)| since.elapsed() < scene::STAGE);
        let in_view = self.panel_open
            && self.timeline_shown.target() > 0.5
            && self.panel_shown.target() > 0.5
            && !motion::reduced();
        if !in_view {
            self.pending_clips.clear();
            return;
        }
        if growing {
            return;
        }
        let now = Instant::now();
        let busy = self.session.read(cx).state.work.busy();
        let mut leaving = 0u32;
        for pending in std::mem::take(&mut self.pending_clips) {
            let placed = &pending.placed;
            let lane = self.timeline.read(cx).clip_bounds(&placed.clip);
            let poster = placed
                .source
                .as_deref()
                .and_then(|source| self.session.read(cx).artwork.poster(source));
            match (lane, poster) {
                (Some(lane), Some(poster)) => {
                    let tile = placed
                        .source
                        .as_deref()
                        .and_then(|source| self.chat.read(cx).footage_tile_bounds(source));
                    let Some(from) = tile.or_else(|| self.chat.read(cx).over_composer()) else {
                        continue;
                    };
                    if let (Some(source), true) = (placed.source.clone(), tile.is_some()) {
                        self.chat
                            .update(cx, |chat, cx| chat.take_footage(&source, cx));
                    }
                    self.flights.push(Flight {
                        image: poster,
                        frame: false,
                        from,
                        to: lane,
                        started: now + scene::FLIGHT_STAGGER * leaving,
                        kind: FlightKind::Clip {
                            clip: placed.clip.clone(),
                            name: placed.name.clone().into(),
                        },
                    });
                    leaving += 1;
                }
                // The timeline has yet to draw it: it waits while Kerf
                // works, and a little after.
                (None, Some(_)) if busy || pending.asked.elapsed() < scene::FLIGHT_WAIT => {
                    self.pending_clips.push(pending);
                }
                _ => {}
            }
        }
        if leaving > 0 || !self.pending_clips.is_empty() {
            cx.notify();
        }
    }

    /// The picture growing out of what it came from while it still is (its
    /// stand-in landing, the stage showing, the timeline coming in after
    /// it): since when, and the clip it stood in for. Once that is over it
    /// is no longer growing, whenever the window next looks.
    fn growing(&self) -> Option<&(Instant, Option<String>)> {
        self.grow.as_ref().filter(|(since, _)| {
            since.elapsed() < scene::STAGE + scene::FLIGHT_LAND.max(scene::SETTLE)
        })
    }

    /// How much of the stage and of the timeline is drawn now: both whole,
    /// but while the picture grows ([`grow_shows`]) or goes back into its
    /// card (the stage not at all, as the panel goes).
    pub(super) fn panel_shows(&self) -> (f32, f32) {
        let growing = self.growing().map(|(since, _)| grow_shows(since.elapsed()));
        let going_back = self
            .going_back
            .is_some_and(|since| since.elapsed() < scene::PANEL);
        if going_back {
            return (0.0, growing.map_or(1.0, |(_, lanes)| lanes));
        }
        growing.unwrap_or((1.0, 1.0))
    }
}

/// A flight as it is drawn this frame: a frame going home covers its box,
/// rounded as the picture is; a clip's poster covers its box with a ring of
/// Kerf's ink round it and its name at its foot; the picture growing or going
/// back is shown whole on black, as the stage shows it.
pub(super) fn flown(drawn: Drawn) -> AnyElement {
    let Drawn {
        bounds,
        opacity,
        image,
        kind,
    } = drawn;
    // Let go, it is drawn as it was flying.
    let kind = match kind {
        FlightKind::LetGo { was, .. } => *was,
        kind => kind,
    };
    let (radius, whole) = match &kind {
        FlightKind::Clip { .. } => (theme::radius::SMALL, false),
        FlightKind::Grow { .. } | FlightKind::Back { .. } => (theme::radius::CARD, true),
        FlightKind::Export | FlightKind::Home | FlightKind::LetGo { .. } => {
            (theme::radius::CARD, false)
        }
    };
    let fit = if whole {
        ObjectFit::Contain
    } else {
        ObjectFit::Cover
    };
    let flight = div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .opacity(opacity)
        // Cut to its box.
        .overflow_hidden()
        .rounded(px(radius))
        .when(whole, |flight| flight.bg(gpui::black()))
        .child(img(image).size_full().rounded(px(radius)).object_fit(fit));
    match kind {
        FlightKind::Clip { name, .. } => flight
            .when(
                f32::from(bounds.size.height) >= layout::CLIP_FLIGHT_NAMED,
                |flight| {
                    flight.child(
                        div()
                            .absolute()
                            .left(px(theme::space::S4))
                            .bottom(px(theme::space::S4))
                            .max_w(bounds.size.width - px(theme::space::S8))
                            .h(px(17.0))
                            .px(px(theme::space::S4))
                            .flex()
                            .items_center()
                            .rounded(px(theme::radius::SMALL))
                            .bg(gpui::rgba(0x00000099))
                            .child(ui::line(name, text::META, gpui::white().into())),
                    )
                },
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(radius))
                    .border(px(layout::CLIP_FLIGHT_RING))
                    .border_color(theme::accent_ink()),
            )
            .into_any_element(),
        _ => flight.into_any_element(),
    }
}

/// Where a manual export's frame is at `elapsed`: how far along (eased,
/// 0..1) and how clear. It crosses over [`scene::FLIGHT`] fading from 0.9
/// to 0.2, then fades out where it landed over [`scene::FLIGHT_LAND`].
/// None once it has gone.
pub fn export_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    if elapsed >= scene::FLIGHT + scene::FLIGHT_LAND {
        return None;
    }
    let flight = scene::FLIGHT.as_secs_f32();
    let land = scene::FLIGHT_LAND.as_secs_f32();
    let elapsed = elapsed.as_secs_f32();
    let t = (elapsed / flight).clamp(0.0, 1.0);
    let opacity = FLIGHT_FROM + (FLIGHT_TO - FLIGHT_FROM) * t;
    let fading = ((elapsed - flight) / land).clamp(0.0, 1.0);
    Some((motion::ease(t), opacity * (1.0 - fading)))
}

/// Where the picture going home is at `elapsed`: how far along (eased,
/// 0..1) over [`scene::STAGE`], whole and clear, then how clear as it fades
/// where it landed over [`scene::FLIGHT_LAND`]. None once it has gone.
pub fn home_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    if elapsed >= scene::STAGE + scene::FLIGHT_LAND {
        return None;
    }
    let travel = scene::STAGE.as_secs_f32();
    let land = scene::FLIGHT_LAND.as_secs_f32();
    let elapsed = elapsed.as_secs_f32();
    let t = (elapsed / travel).clamp(0.0, 1.0);
    let fading = ((elapsed - travel) / land).clamp(0.0, 1.0);
    Some((motion::ease(t), 1.0 - fading))
}

/// How clear a manual export's frame is as it leaves the picture and as it lands.
const FLIGHT_FROM: f32 = 0.9;
const FLIGHT_TO: f32 = 0.2;

/// Where a clip of a cut being built is at `elapsed` on its way from its
/// footage to the timeline: how far along (eased, over [`scene::FLIGHT`])
/// and how clear (whole, then fading over [`scene::FLIGHT_LAND`] where it
/// landed, over what it became). None once it has gone.
pub fn clip_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    travel_then_fade(elapsed, scene::FLIGHT, scene::FLIGHT_LAND)
}

/// Where the picture's stand-in is at `elapsed` as the picture grows out
/// of what it came from: how far along (eased, over [`scene::STAGE`]),
/// whole, holding where it landed until it is let go (`released`, that
/// long after it left; None: not yet), then fading over
/// [`STAND_IN_RELEASE`] over the picture. None once it has gone.
pub fn grow_flight_at(elapsed: Duration, released: Option<Duration>) -> Option<(f32, f32)> {
    let Some(released) = released else {
        let t = elapsed.as_secs_f32() / scene::STAGE.as_secs_f32();
        return Some((motion::ease(t.clamp(0.0, 1.0)), 1.0));
    };
    let fades_from = released.max(scene::STAGE);
    if elapsed >= fades_from + STAND_IN_RELEASE {
        return None;
    }
    let t = elapsed.as_secs_f32() / scene::STAGE.as_secs_f32();
    let fading =
        (elapsed.as_secs_f32() - fades_from.as_secs_f32()) / STAND_IN_RELEASE.as_secs_f32();
    Some((
        motion::ease(t.clamp(0.0, 1.0)),
        1.0 - fading.clamp(0.0, 1.0),
    ))
}

/// Where the picture is at `elapsed` as it goes back into its card: how
/// far along (eased, over [`scene::PANEL`], as the panel goes), whole,
/// then fading over [`scene::FLIGHT_LAND`] over the card's own.
pub fn back_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    travel_then_fade(elapsed, scene::PANEL, scene::FLIGHT_LAND)
}

/// What flew, let go where it had got to as its landing went away, `clear`
/// as it was then: there (all along), fading from `clear` over
/// [`scene::FLIGHT_LAND`], as one that has landed. None once it has gone.
pub fn let_go_flight_at(elapsed: Duration, clear: f32) -> Option<(f32, f32)> {
    if elapsed >= scene::FLIGHT_LAND {
        return None;
    }
    let fading = elapsed.as_secs_f32() / scene::FLIGHT_LAND.as_secs_f32();
    Some((1.0, clear.clamp(0.0, 1.0) * (1.0 - fading)))
}

/// Eased along over `travel`, whole; then fading over `land`. Under
/// Reduce Motion nothing flies, so this is never asked then.
fn travel_then_fade(elapsed: Duration, travel: Duration, land: Duration) -> Option<(f32, f32)> {
    if elapsed >= travel + land {
        return None;
    }
    let t = elapsed.as_secs_f32() / travel.as_secs_f32().max(f32::EPSILON);
    let fading =
        (elapsed.as_secs_f32() - travel.as_secs_f32()) / land.as_secs_f32().max(f32::EPSILON);
    Some((
        motion::ease(t.clamp(0.0, 1.0)),
        1.0 - fading.clamp(0.0, 1.0),
    ))
}

/// How much of the stage and of the timeline shows `elapsed` after the
/// picture began to grow: the stage once its stand-in has landed, over
/// [`scene::FLIGHT_LAND`]; the timeline after it, from
/// [`TIMELINE_LEAD`] before the picture has grown, over
/// [`scene::SETTLE`].
pub fn grow_shows(elapsed: Duration) -> (f32, f32) {
    let after = |start: Duration, over: Duration| {
        let t = (elapsed.as_secs_f32() - start.as_secs_f32()) / over.as_secs_f32();
        motion::ease(t.clamp(0.0, 1.0))
    };
    (
        after(scene::STAGE, scene::FLIGHT_LAND),
        after(scene::STAGE.saturating_sub(TIMELINE_LEAD), scene::SETTLE),
    )
}

/// A clip a cut being built placed: its id, the footage it plays (whose
/// tile and poster it flies with), its name, and where it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedClip {
    pub clip: String,
    pub source: Option<String>,
    pub name: String,
    pub start: f64,
}

/// What `cut` (a proposal over `program`) placed that `seen` (each clip
/// as "id@end") had not: each picture clip added or moved since, not one
/// taken out and none on a sound track (its sound comes with it), in the
/// order they play; and what is seen now.
pub fn placed_since(
    program: &crate::model::Timeline,
    cut: &crate::model::Timeline,
    seen: &HashSet<String>,
) -> (Vec<PlacedClip>, HashSet<String>) {
    let changes = crate::changes::clip_changes(program, cut);
    let key =
        |change: &crate::changes::ClipChange| format!("{}@{:.4}", change.clip, change.new_end);
    let now: HashSet<String> = changes.iter().map(key).collect();
    let mut placed: Vec<PlacedClip> = changes
        .iter()
        .filter(|change| !change.removed && !change.transition && !seen.contains(&key(change)))
        .filter_map(|change| {
            let (track, clip) = cut.clip(&change.clip)?;
            (track.kind == crate::model::TrackKind::Video).then(|| PlacedClip {
                clip: change.clip.clone(),
                source: clip.source_id.clone(),
                name: change.name.clone(),
                start: change.new_start,
            })
        })
        .collect();
    placed.sort_by(|a, b| a.start.total_cmp(&b.start));
    (placed, now)
}

/// The first clip of `timeline`'s first picture track that has one, and
/// the footage it plays: what the picture grows out of.
fn first_picture_clip(timeline: &crate::model::Timeline) -> Option<(String, String)> {
    timeline
        .tracks
        .iter()
        .filter(|track| track.kind == crate::model::TrackKind::Video)
        .find_map(|track| {
            let clip = track
                .clips
                .iter()
                .min_by(|a, b| a.timeline_start.total_cmp(&b.timeline_start))?;
            Some((clip.id.clone(), clip.source_id.clone()?))
        })
}

/// The rect `along` (0..1) of the way from `from` to `to`.
pub fn lerp_bounds(from: Bounds<Pixels>, to: Bounds<Pixels>, along: f32) -> Bounds<Pixels> {
    let mix = |a: Pixels, b: Pixels| a + (b - a) * along;
    Bounds::new(
        point(
            mix(from.origin.x, to.origin.x),
            mix(from.origin.y, to.origin.y),
        ),
        size(
            mix(from.size.width, to.size.width),
            mix(from.size.height, to.size.height),
        ),
    )
}

/// Where a flight lands on `row` (the project's row in the sidebar): a
/// small frame at the row's start, inside its height.
pub fn landing_in(row: Bounds<Pixels>) -> Bounds<Pixels> {
    let (left, top) = scene::LANDING_INSET;
    let height = (f32::from(row.size.height) - top * 2.0).max(1.0);
    let width = scene::LANDING_WIDTH.min(f32::from(row.size.width)).max(1.0);
    Bounds::new(
        point(row.origin.x + px(left), row.origin.y + px(top)),
        size(px(width), px(height)),
    )
}

/// Where a manual export's frame lands when the sidebar has not laid the
/// project's row out yet: at its folder, under the toolbar, New
/// conversation and Search.
fn project_row() -> Bounds<Pixels> {
    let top = layout::TOOLBAR + theme::height::H32 * 2.0 + 2.0 + 8.0 + 4.0;
    Bounds::new(point(px(12.0), px(top)), size(px(44.0), px(26.0)))
}
