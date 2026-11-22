//! A first cut's picture on its card ([`ChatView::cut_picture`]): the
//! poster of the clip it shows, how long it is and that it plays; while
//! Kerf makes it, light round it and "Cutting"; just done, "Done". Moving
//! over it runs through its clips; pressed, the picture grows out of it to
//! the stage ([`ChatEvent::GrowPicture`]), and goes back into it while the
//! card is in sight ([`in_sight`]).

use super::cards::previewing;
use super::rim::{paint_rim, paint_ring};
use super::{ChatEvent, ChatView};
use crate::icons::Icon;
use crate::model::{self, Timeline, TimelineShow};
use crate::motion;
use crate::theme::{self, motion as scene, text};
use crate::ui;
use gpui::{
    div, linear_color_stop, linear_gradient, point, prelude::*, px, AnyElement, App, Context,
    ElementId, RenderImage, Window,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A cut card's timings and sizes, kept in the theme.
pub use crate::theme::layout::{
    CARD_PICTURE_RING, CARD_PICTURE_TALLEST, CARD_PICTURE_WIDEST, CARD_PLAY_MARK,
};
pub use crate::theme::motion::{
    CARD_CROSSFADE, CARD_DONE, CARD_DONE_FADE, CARD_MARKS_AFTER, CARD_MARKS_FADE, CARD_RING,
    CARD_TURN,
};

/// A first cut's clips, in time order, on its first picture track that
/// has any: where each starts and the media it shows; and how long the
/// whole cut is, every track counted.
pub fn cut_frames(cut: &Timeline) -> (Vec<(f64, String)>, f64) {
    let duration = cut
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.end())
        .fold(0.0, f64::max);
    let mut frames: Vec<(f64, String)> = cut
        .tracks
        .iter()
        .find(|track| track.kind == model::TrackKind::Video && !track.clips.is_empty())
        .map(|track| {
            track
                .clips
                .iter()
                .map(|clip| {
                    (
                        clip.timeline_start,
                        clip.source_id.clone().unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    frames.sort_by(|a, b| a.0.total_cmp(&b.0));
    (frames, duration)
}

/// Which of a cut's frames (in time order, each with whether its poster
/// has come) its picture shows: while Kerf works, the newest with a
/// poster; else the one under the pointer (`scrub`, 0..1 across), or the
/// first. None until a poster has come.
pub fn shown_frame(
    frames: &[(f64, bool)],
    duration: f64,
    scrub: Option<f32>,
    working: bool,
) -> Option<usize> {
    if working {
        return frames.iter().rposition(|(_, poster)| *poster);
    }
    if !frames.iter().any(|(_, poster)| *poster) {
        return None;
    }
    let at = scrub.map_or(0.0, |scrub| f64::from(scrub.clamp(0.0, 0.999)) * duration);
    let mut shown = 0;
    for (index, (start, poster)) in frames.iter().enumerate() {
        if *start <= at && *poster {
            shown = index;
        }
    }
    Some(shown)
}

/// The length badge on a cut's picture: "0:42", "12:05".
pub fn length_badge(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// How a finished cut's picture stands `since` it was done: how much it
/// is still at work (its shade and ring going out over `CARD_RING`), the
/// ring's light, Done's opacity (gone over the last `CARD_DONE_FADE`), and
/// the length and play mark's (coming after `CARD_MARKS_AFTER`). Past
/// `CARD_DONE` it simply stands, its marks on. As drawn, see
/// [`finish_drawn`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    pub work: f32,
    pub ring: f32,
    pub chip: f32,
    pub marks: f32,
}

pub fn finishing(since: Duration) -> Finish {
    let ramp = |from: Duration, length: Duration| {
        if length.is_zero() {
            return if since >= from { 1.0 } else { 0.0 };
        }
        (since.saturating_sub(from).as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
    };
    if since >= CARD_DONE {
        return Finish {
            work: 0.0,
            ring: 0.0,
            chip: 0.0,
            marks: 1.0,
        };
    }
    let out = ramp(Duration::ZERO, CARD_RING);
    Finish {
        work: 1.0 - out,
        ring: 1.0 - out,
        chip: 1.0 - ramp(CARD_DONE - CARD_DONE_FADE, CARD_DONE_FADE),
        marks: ramp(CARD_MARKS_AFTER, CARD_MARKS_FADE),
    }
}

/// How a finished cut's picture is drawn `since` it was done: as
/// [`finishing`] has it, or under Reduce Motion (`reduced`) simply as it
/// stands at the end, at once, with no frames to wait for.
pub fn finish_drawn(since: Duration, reduced: bool) -> Finish {
    finishing(if reduced { CARD_DONE } else { since })
}

/// The picture's box on its card: as wide as it may be at its shape, an
/// upright one held to the tallest.
pub fn card_picture_size(aspect: f32) -> (f32, f32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let mut width = CARD_PICTURE_WIDEST;
    let mut height = width / aspect;
    if height > CARD_PICTURE_TALLEST {
        height = CARD_PICTURE_TALLEST;
        width = height * aspect;
    }
    (width, height)
}

/// How a cut's picture is drawn now.
#[derive(Clone, Debug, Default)]
pub(super) struct PictureState {
    /// Kerf is making it.
    pub(super) working: bool,
    /// It finished this long ago (Done, then its marks).
    pub(super) finished: Option<Duration>,
    /// Its picture is on the stage, from here.
    pub(super) on_stage: bool,
    /// The pointer's place across it.
    pub(super) scrub: Option<f32>,
    /// The poster it showed before the newest clip, and how far the
    /// newest has come over it (0..1).
    pub(super) before: Option<(Arc<RenderImage>, f32)>,
}

/// How a cut's picture is drawn this frame, from how it stands.
struct PictureLook {
    /// The poster of the clip it shows, once one has come.
    shown: Option<Arc<RenderImage>>,
    /// How long the whole cut is.
    duration: f64,
    /// Its box on the card, at the cut's shape.
    width: f32,
    height: f32,
    reduced: bool,
    /// How it stands since it was done ([`finish_drawn`]).
    finish: Option<Finish>,
    /// How much it is at work (its shade, the line under it), how much its
    /// chip shows, and its length and play mark.
    work: f32,
    chip: f32,
    marks: f32,
    /// Whether it moves by itself, and so asks for frames.
    moving: bool,
}

impl ChatView {
    /// How cut `id`'s picture stands now: just done, on the stage, under
    /// the pointer.
    pub(super) fn picture_state(&self, id: &str, cx: &App) -> PictureState {
        let session = self.session.read(cx);
        PictureState {
            on_stage: session.picture_open == Some(true)
                && self
                    .stage_proposal
                    .as_deref()
                    .is_some_and(|shown| self.book.same(shown, id)),
            scrub: self
                .scrub
                .as_ref()
                .filter(|(over, _)| over == id)
                .map(|(_, across)| *across),
            finished: self
                .built
                .as_ref()
                .filter(|(built, _)| self.book.same(built, id))
                .map(|(_, at)| at.elapsed()),
            ..PictureState::default()
        }
    }

    /// How cut `cut`'s picture is drawn as it stands (`state`): the poster
    /// it shows, its box, and how far it is through being done.
    fn picture_look(&self, cut: &Timeline, state: &PictureState, cx: &App) -> PictureLook {
        let session = self.session.read(cx);
        let artwork = &session.artwork;
        let (frames, duration) = cut_frames(cut);
        let posters: Vec<Option<Arc<RenderImage>>> = frames
            .iter()
            .map(|(_, source)| artwork.poster(source))
            .collect();
        let shown = shown_frame(
            &frames
                .iter()
                .zip(&posters)
                .map(|((at, _), poster)| (*at, poster.is_some()))
                .collect::<Vec<_>>(),
            duration,
            state.scrub,
            state.working,
        )
        .and_then(|index| posters.get(index).cloned().flatten());
        let (width, height) = card_picture_size(cut.aspect());
        let reduced = motion::reduced();
        let finish = state.finished.map(|since| finish_drawn(since, reduced));
        let work = if state.working {
            1.0
        } else {
            finish.map_or(0.0, |finish| finish.work)
        };
        let moving = !reduced
            && (state.working
                || finish.is_some_and(|finish| finish.chip > 0.0 || finish.marks < 1.0));
        let chip = if state.working {
            1.0
        } else {
            finish.map_or(0.0, |finish| finish.chip)
        };
        let marks = if state.working {
            0.0
        } else {
            finish.map_or(1.0, |finish| finish.marks)
        };
        PictureLook {
            shown,
            duration,
            width,
            height,
            reduced,
            finish,
            work,
            chip,
            marks,
            moving,
        }
    }

    /// A first cut's picture (`cut` is its timeline), for its card: the
    /// poster of the clip it shows, how long it is and a play mark; while
    /// Kerf makes it, light round it and "Cutting"; just done, "Done".
    /// Moving over it runs through its clips; pressing it brings the
    /// picture in.
    pub(super) fn cut_picture(
        &self,
        id: &str,
        cut: &Timeline,
        state: PictureState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let look = self.picture_look(cut, &state, cx);
        if look.moving || state.before.is_some() {
            window.request_animation_frame();
        }
        let radius = theme::radius::CARD;
        let mut sized = div().w(px(look.width)).max_w_full();
        // Its height follows its width at the cut's shape, narrowed or not.
        sized.style().aspect_ratio = Some(look.width / look.height.max(1.0));
        let picture = sized
            .id(ElementId::Name(format!("cut-picture-{id}").into()))
            .relative()
            .rounded(px(radius))
            .overflow_hidden()
            .bg(theme::overlay_screen())
            .children(poster_layers(
                look.shown.as_ref(),
                state.before.as_ref(),
                radius,
            ))
            // The pointer's place, a line at its foot.
            .children(
                state
                    .scrub
                    .filter(|_| !state.working && !state.on_stage)
                    .map(scrub_line),
            )
            // At work the picture sits a little back.
            .children((look.work > 0.0).then(|| work_shade(look.work)))
            .children(stage_or_marks(state.on_stage, look.marks, look.duration))
            // What Kerf is doing, at its top left: hopping dots and Cutting,
            // then a tick and Done.
            .children((look.chip > 0.0 && !state.on_stage).then(|| {
                work_chip(
                    state.working,
                    look.chip,
                    look.reduced,
                    self.started_at.elapsed(),
                )
            }))
            // Where it was laid out, for the pointer's place over it.
            .child({
                let (pictures, key) = (self.pictures.clone(), id.to_owned());
                gpui::canvas(
                    move |bounds, _, _| {
                        pictures.borrow_mut().insert(key.clone(), bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
            });
        let tipped = !state.working && !state.on_stage;
        let picture = picture.when(tipped, |picture| scrubbed(picture, id, cx));
        // Round it, the light: running while Kerf works, lit once and going
        // out as it is done; under it, a line of light runs while it works.
        let ring = look.finish.map_or(0.0, |finish| finish.ring);
        div()
            .relative()
            .flex()
            .flex_col()
            .p(px(CARD_PICTURE_RING))
            .pb(px(CARD_PICTURE_RING + 4.0))
            .child(if tipped {
                ui::tipped(
                    ElementId::Name(format!("cut-picture-tip-{id}").into()),
                    ui::Tip::new("Open on the stage"),
                    picture,
                )
                .into_any_element()
            } else {
                picture.into_any_element()
            })
            .children((look.work > 0.0 && !look.reduced).then(|| {
                foot_line(
                    look.work,
                    motion::phase(self.started_at, scene::WORK_SHIMMER),
                )
            }))
            .child(rim(state.working, look.reduced, self.started_at, ring))
            .into_any_element()
    }

    /// The picture of cut `id` comes in from its card: the stage shows it
    /// (rendered when it is not yet), growing out of the card's picture
    /// (the window's [`ChatEvent::GrowPicture`]), and the card says it is
    /// there.
    fn open_cut(&mut self, id: String, _window: &mut Window, cx: &mut Context<Self>) {
        self.stage_proposal = Some(id.clone());
        self.scrub = None;
        self.session.update(cx, |session, cx| {
            let showing = session.state.preview.proposal.as_deref() == Some(id.as_str())
                && previewing(&session.state.preview.status);
            if !showing {
                session.preview(Some(&id));
            }
            cx.notify();
        });
        cx.emit(ChatEvent::GrowPicture { proposal: id });
        cx.notify();
    }

    /// The cut whose picture went to the stage from its card, while it is there.
    pub fn stage_proposal(&self) -> Option<&str> {
        self.stage_proposal.as_deref()
    }

    /// Where the visible part of the conversation was last laid out.
    pub(super) fn in_sight(&self, bounds: gpui::Bounds<gpui::Pixels>) -> bool {
        let view = self.scroll.bounds();
        view.size.width > px(0.0) && in_sight(bounds, view)
    }

    /// Where cut `proposal`'s card shows its picture in the window, as last
    /// laid out, while it is in sight: what the stage grows out of, and
    /// what it goes back into.
    pub fn cut_picture_bounds(&self, proposal: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        let pictures = self.pictures.borrow();
        let bounds = pictures.get(proposal).copied().or_else(|| {
            pictures
                .iter()
                .find(|(id, _)| self.book.same(id, proposal))
                .map(|(_, bounds)| *bounds)
        })?;
        self.in_sight(bounds).then_some(bounds)
    }

    /// The poster cut `proposal`'s card shows now (the clip under the
    /// pointer, else its first), to stand in as its picture grows.
    pub fn cut_poster(&self, proposal: &str, cx: &App) -> Option<Arc<RenderImage>> {
        let session = self.session.read(cx);
        let (shown, cut) = session.first_cut()?;
        if !self.book.same(&shown.proposal_id, proposal) {
            return None;
        }
        let (frames, duration) = cut_frames(cut);
        let posters: Vec<Option<Arc<RenderImage>>> = frames
            .iter()
            .map(|(_, source)| session.artwork.poster(source))
            .collect();
        let scrub = self
            .scrub
            .as_ref()
            .filter(|(over, _)| self.book.same(over, proposal))
            .map(|(_, across)| *across);
        let shown = shown_frame(
            &frames
                .iter()
                .zip(&posters)
                .map(|((at, _), poster)| (*at, poster.is_some()))
                .collect::<Vec<_>>(),
            duration,
            scrub,
            false,
        )?;
        posters.get(shown).cloned().flatten()
    }
}

/// Whether `bounds` lies whole inside the conversation's visible part
/// `view` (a point either way spared): only then does a picture go into it
/// or come out of it, so none flies to or from a place out of sight.
pub fn in_sight(bounds: gpui::Bounds<gpui::Pixels>, view: gpui::Bounds<gpui::Pixels>) -> bool {
    let spare = px(1.0);
    bounds.left() >= view.left() - spare
        && bounds.right() <= view.right() + spare
        && bounds.top() >= view.top() - spare
        && bounds.bottom() <= view.bottom() + spare
}

/// The picture: the poster it shows (the one before fading under the
/// newest while a clip turns), else a wash in the room's greys and the
/// accent, never black.
fn poster_layers(
    shown: Option<&Arc<RenderImage>>,
    before: Option<&(Arc<RenderImage>, f32)>,
    radius: f32,
) -> Vec<AnyElement> {
    match (shown, before) {
        (Some(poster), Some((before, t))) => vec![
            poster_layer(before.clone(), radius).into_any_element(),
            div()
                .absolute()
                .inset_0()
                .opacity(*t)
                .child(poster_layer(poster.clone(), radius))
                .into_any_element(),
        ],
        (Some(poster), None) => vec![poster_layer(poster.clone(), radius).into_any_element()],
        (None, _) => vec![div()
            .absolute()
            .inset_0()
            .bg(theme::cut::ground())
            .child(div().absolute().inset_0().bg(linear_gradient(
                135.0,
                linear_color_stop(theme::cut::glow_from(), 0.0),
                linear_color_stop(theme::cut::glow_to(), 1.0),
            )))
            .into_any_element()],
    }
}

/// The pointer's place over the picture, `scrub` of the way across: a line
/// at its foot.
fn scrub_line(scrub: f32) -> gpui::Div {
    div()
        .absolute()
        .left_0()
        .bottom_0()
        .h(px(2.0))
        .w(gpui::relative(scrub))
        .bg(theme::cut::scrub())
}

/// At work the picture sits a little back, by `work`.
fn work_shade(work: f32) -> gpui::Div {
    div()
        .absolute()
        .inset_0()
        .bg(theme::with_alpha(gpui::black().into(), 0.22 * work))
}

/// Over the picture: on the stage now, what is left here says so; else
/// how long the cut is (`duration`), at its foot, and that it plays, at
/// `marks`.
fn stage_or_marks(on_stage: bool, marks: f32, duration: f64) -> Vec<AnyElement> {
    if on_stage {
        // It is on the stage now: what is left here says so.
        vec![div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::cut::on_stage())
            .child(ui::label("On the stage", text::CONTROL, theme::ink2()))
            .into_any_element()]
    } else if marks > 0.0 {
        // How long it is, at its foot, and that it plays.
        vec![
            div()
                .absolute()
                .right(px(6.0))
                .bottom(px(6.0))
                .h(px(18.0))
                .px(px(5.0))
                .flex()
                .items_center()
                .rounded(px(theme::radius::SMALL))
                .bg(theme::cut::length_plate())
                .opacity(marks)
                .child(ui::label(
                    length_badge(duration),
                    text::MONO_SMALL,
                    gpui::white().into(),
                ))
                .into_any_element(),
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .opacity(marks)
                .child(
                    div()
                        .size(px(CARD_PLAY_MARK))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme::cut::play_mark())
                        .child(Icon::Play.view(16.0, gpui::white().into())),
                )
                .into_any_element(),
        ]
    } else {
        Vec::new()
    }
}

/// What Kerf is doing, at the picture's top left, at `chip`: while it
/// works, dots hopping in turn (`since` the view began; still under Reduce
/// Motion) and Cutting; then a tick and Done.
fn work_chip(working: bool, chip: f32, reduced: bool, since: Duration) -> gpui::Div {
    let dots = (0..3).map(|dot| {
        let t = if reduced {
            0.0
        } else {
            ((since.as_secs_f32() / scene::CHIP_HOP.as_secs_f32())
                - dot as f32 * scene::CHIP_HOP_STAGGER
                + 1.0)
                .rem_euclid(1.0)
        };
        let hop = if t < 0.6 {
            (t / 0.6 * std::f32::consts::PI).sin()
        } else {
            0.0
        };
        div()
            .size(px(4.0))
            .mt(px(-scene::CHIP_HOP_RISE * hop))
            .rounded_full()
            .bg(theme::accent_ink())
            .opacity(0.5 + 0.5 * hop)
    });
    div()
        .absolute()
        .left(px(8.0))
        .top(px(8.0))
        .h(px(22.0))
        .pl(px(theme::space::S8))
        .pr(px(9.0))
        .flex()
        .items_center()
        .gap(px(theme::space::S6))
        .rounded_full()
        .bg(theme::cut::work_chip())
        .opacity(chip)
        .child(if working {
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S2))
                .children(dots)
                .into_any_element()
        } else {
            Icon::Check
                .view(12.0, theme::accent_ink())
                .into_any_element()
        })
        .child(ui::label(
            if working { "Cutting" } else { "Done" },
            text::META,
            gpui::white().into(),
        ))
}

/// The picture of cut `id` answers the pointer: moving over it runs
/// through its clips (the view's `scrub`), leaving it lets that go, and
/// pressing it opens the cut on the stage.
fn scrubbed(
    picture: gpui::Stateful<gpui::Div>,
    id: &str,
    cx: &Context<ChatView>,
) -> gpui::Stateful<gpui::Div> {
    let scrub_id = id.to_owned();
    let leave_id = id.to_owned();
    let open_id = id.to_owned();
    picture
        .cursor_pointer()
        .on_mouse_move(
            cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                let Some(bounds) = this.pictures.borrow().get(&scrub_id).copied() else {
                    return;
                };
                let width = f32::from(bounds.size.width);
                let across = if width > 0.0 && bounds.contains(&event.position) {
                    Some(
                        ((f32::from(event.position.x - bounds.origin.x)) / width).clamp(0.0, 0.999),
                    )
                } else {
                    None
                };
                let now = across.map(|across| (scrub_id.clone(), across));
                if this.scrub != now {
                    this.scrub = now;
                    cx.notify();
                }
            }),
        )
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            if !*hovered && this.scrub.as_ref().is_some_and(|(id, _)| *id == leave_id) {
                this.scrub = None;
                cx.notify();
            }
        }))
        .on_click(
            cx.listener(move |this, _, window, cx| this.open_cut(open_id.clone(), window, cx)),
        )
}

/// Round the picture, the light: running round while Kerf works (still
/// under Reduce Motion), else the ring lit once as it is done, at `ring`,
/// going out.
fn rim(working: bool, reduced: bool, started: Instant, ring: f32) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let inner = gpui::Bounds {
                origin: point(
                    bounds.origin.x + px(CARD_PICTURE_RING / 2.0),
                    bounds.origin.y + px(CARD_PICTURE_RING / 2.0),
                ),
                size: gpui::size(
                    bounds.size.width - px(CARD_PICTURE_RING),
                    bounds.size.height - px(CARD_PICTURE_RING + 7.0),
                ),
            };
            let radius = theme::radius::CARD + CARD_PICTURE_RING / 2.0;
            if working {
                let turn = (!reduced).then(|| motion::phase(started, CARD_TURN));
                paint_rim(inner, radius, turn, window);
            } else {
                paint_ring(inner, radius, ring, window);
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Under the picture while Kerf works, at `work`, a line of light running
/// along its foot, `at` of the way through each `WORK_SHIMMER`.
fn foot_line(work: f32, at: f32) -> gpui::Div {
    div()
        .relative()
        .h(px(2.0))
        .mt(px(3.0))
        .mx(px(CARD_PICTURE_RING))
        .overflow_hidden()
        .opacity(work)
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(gpui::relative(at * 1.4 - 0.4))
                .w(gpui::relative(0.4))
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(theme::cut::foot_line_from(), 0.0),
                    linear_color_stop(theme::cut::foot_line_to(), 1.0),
                )),
        )
}

/// A poster filling the picture's box, cut to its shape.
pub(super) fn poster_layer(poster: Arc<RenderImage>, radius: f32) -> impl IntoElement {
    gpui::img(poster)
        .absolute()
        .inset_0()
        .size_full()
        .object_fit(gpui::ObjectFit::Cover)
        .rounded(px(radius))
}
