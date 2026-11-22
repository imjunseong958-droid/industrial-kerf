//! What runs, and how the picture says so: a light round just outside its
//! edge (`Hue`: blue for Kerf, amber for the computer, amber winning) and
//! the chip at its top left (`Said`), saying a making first, then a
//! render, then Kerf. Both fade in, and out when it ends, the chip keeping
//! its last words as it goes; under Reduce Motion no light runs and the
//! ring is whole and steady.

use super::picture::Placement;
use super::StageView;
use editor_ui::motion;
use editor_ui::theme::{self, layout, motion as scene, text};
use editor_ui::ui;
use gpui::{
    div, point, prelude::*, px, quad, size, App, BorderStyle, Bounds, Context, Corners,
    PathBuilder, Pixels, Point, Rgba, SharedString, TextRun, Window,
};
use std::time::Instant;

/// Which of the two lights runs round the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hue {
    Kerf,
    Computer,
}

impl Hue {
    pub(super) fn color(self) -> Rgba {
        match self {
            Hue::Kerf => theme::activity::kerf(),
            Hue::Computer => theme::activity::gpu(),
        }
    }
}

/// What the chip over the picture says: its words, cut short where it has
/// no room; how a making stands after them, never cut; and its tip, the
/// whole of it, while a making runs.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Said {
    pub(super) words: String,
    pub(super) stands: Option<String>,
    pub(super) tip: Option<String>,
}

impl Said {
    fn words(words: String) -> Self {
        Self {
            words,
            stands: None,
            tip: None,
        }
    }
}

/// Whether the chip says Kerf is cutting rather than looking: Kerf works
/// (`busy`) on a cut it proposes for an empty program (no clips of its own,
/// a proposed timeline there).
pub fn kerf_cutting(busy: bool, program_has_clips: bool, proposal_timeline: bool) -> bool {
    busy && !program_has_clips && proposal_timeline
}

/// How clear the working chip's dot is `phase` (0..1) of the way through
/// a pulse (`WORK_PULSE`): full at either end, down to 35% half way.
pub fn pulse_level(phase: f32) -> f32 {
    let phase = if phase.is_finite() {
        phase.clamp(0.0, 1.0)
    } else {
        0.0
    };
    1.0 - 0.65 * (1.0 - (phase * 2.0 - 1.0).abs())
}

/// How wide `words` are, set on one line in `style`.
fn line_width(window: &Window, words: &str, style: theme::TypeStyle) -> f32 {
    let words: SharedString = words.to_owned().into();
    let run = TextRun {
        len: words.len(),
        font: theme::ui_font(style.weight),
        color: theme::ink().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(words, px(style.px), &[run], None)
            .width,
    )
}

/// `words` cut short with an ellipsis to fit `room` points, set in `style`.
fn cut_to(cx: &App, words: &str, room: f32, style: theme::TypeStyle) -> SharedString {
    let font = theme::ui_font(style.weight);
    let mut runs = vec![TextRun {
        len: words.len(),
        font: font.clone(),
        color: theme::ink().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    cx.text_system()
        .line_wrapper(font, px(style.px))
        .truncate_line(words.to_owned().into(), px(room.max(0.0)), "…", &mut runs)
}

impl StageView {
    /// What runs, and the light that says so: a making wins over a render,
    /// which wins over Kerf.
    pub(super) fn activity(&self, cx: &App) -> (Option<Hue>, Option<Said>) {
        let session = self.session.read(cx);
        let state = &session.state;
        if !state.makings.is_empty() {
            let makings: Vec<(String, editor_state::store::Making)> = state
                .makings
                .iter()
                .map(|(media, making)| {
                    // As the band names it: no content hash, a shot by its name.
                    let name = Some(session.media_name(media))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| media.clone());
                    (name, making.clone())
                })
                .collect();
            let said = editor_state::chip::following(&makings).map(|following| Said {
                words: following.said,
                stands: Some(following.stands),
                tip: Some(following.tip),
            });
            return (Some(Hue::Computer), said);
        }
        // A change reel's previews and a first cut's are the reel's and the
        // cut card's to say, not the chip's too; an export still is.
        let quiet = state.preview.rendering() && (session.reel().is_some() || session.cutting());
        if let Some(rendering) = state.rendering().filter(|_| !quiet) {
            return (Some(Hue::Computer), Some(Said::words(rendering)));
        }
        if state.work.busy() {
            let cutting = kerf_cutting(
                true,
                session.timeline().has_clips(),
                state.snapshot.proposal_timeline.is_some(),
            );
            let words = if cutting {
                "Kerf is cutting"
            } else {
                "Kerf is looking"
            };
            return (Some(Hue::Kerf), Some(Said::words(words.to_owned())));
        }
        (None, None)
    }

    /// Follows what runs: the chip keeps what it said while it fades out,
    /// its clock goes on while a making runs, and the light fades in as
    /// something starts (running from then) and out as it all ends. What
    /// the chip says now, worked out once a frame.
    pub(super) fn follow_activity(&mut self, cx: &mut Context<Self>) -> Option<Said> {
        let (hue, chip) = self.activity(cx);
        // The chip keeps what it said while it fades out.
        if chip.is_some() {
            self.said = chip.clone();
        }
        self.tick_makings(cx);
        if hue != self.hue {
            if let Some(hue) = hue {
                self.last_hue = hue;
                if self.hue.is_none() {
                    self.edge_since = Instant::now();
                }
            }
            self.hue = hue;
            self.edge
                .go(if hue.is_some() { 1.0 } else { 0.0 }, scene::EDGE_FADE);
        }
        chip
    }

    /// The chip's clock goes on once a second while a making runs, even
    /// when nothing else asks for frames.
    fn tick_makings(&mut self, cx: &mut Context<Self>) {
        if self.clock.is_some() || self.session.read(cx).state.makings.is_empty() {
            return;
        }
        self.clock = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(scene::CLOCK_FRAME).await;
            let going = this.update(cx, |this, cx| {
                let going = !this.session.read(cx).state.makings.is_empty();
                if !going {
                    this.clock = None;
                }
                cx.notify();
                going
            });
            if !matches!(going, Ok(true)) {
                break;
            }
        }));
    }

    /// The chip at the picture's top left, saying what runs (`chip`, else
    /// what it said last as it fades out with the light, `edge`); none
    /// while the preview's card says what renders (`card_says`).
    pub(super) fn work_chip(
        &self,
        placed: &Placement,
        chip: Option<Said>,
        edge: f32,
        card_says: bool,
        window: &mut Window,
        cx: &App,
    ) -> Option<impl IntoElement + use<>> {
        let picture = placed.picture;
        let left = picture.origin.x;
        let top = picture.origin.y;
        let chip = chip
            .or_else(|| self.said.clone())
            .filter(|_| edge > 0.001 && !card_says)?;
        let color = self.last_hue.color();
        // Its dot pulses once each WORK_PULSE; held still under
        // Reduce Motion. The light round the picture asks for the
        // frames meanwhile.
        let dot = if motion::reduced() {
            1.0
        } else {
            window.request_animation_frame();
            pulse_level(motion::phase(self.edge_since, scene::WORK_PULSE))
        };
        // As wide as its words, up to CHIP_MOST and inside the
        // picture: what a making last said is cut short there, and
        // its tip says the whole of it.
        let most =
            (f32::from(picture.size.width) - 2.0 * layout::CHIP_IN).clamp(0.0, layout::CHIP_MOST);
        // The chip is as wide as its words, up to `most`; past that
        // its words are cut short with an ellipsis.
        let stands = chip.stands.map(|stands| format!("· {stands}"));
        let after = stands.as_deref().map_or(0.0, |stands| {
            6.0 + line_width(window, stands, text::CONTROL)
        });
        let words = line_width(window, &chip.words, text::CONTROL);
        let width = (2.0 * 10.0 + 6.0 + 6.0 + words + after).ceil().min(most);
        let shown = cut_to(
            cx,
            &chip.words,
            width - 2.0 * 10.0 - 6.0 - 6.0 - after,
            text::CONTROL,
        );
        let body = theme::type_style(div(), text::CONTROL)
            .id("stage-chip")
            .flex()
            .items_center()
            .gap(px(theme::space::S6))
            .h(px(24.0))
            .w(px(width))
            .px(px(10.0))
            .rounded(px(theme::radius::PANEL))
            .overflow_hidden()
            .bg(theme::activity::chip())
            .text_color(theme::ink())
            .child(ui::dot(color, 6.0).opacity(dot).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(shown),
            )
            .children(stands.map(|stands| {
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(theme::ink2())
                    .child(stands)
            }));
        Some(
            div()
                .absolute()
                .left(left + px(layout::CHIP_IN))
                .top(top + px(if placed.phone { 36.0 } else { layout::CHIP_IN }))
                .child(ui::tip_on(body, chip.tip.map(ui::Tip::new)))
                .opacity(edge.max(0.0)),
        )
    }
}

/// The point `share` (0..1) of the way round `bounds`, clockwise from its top left.
fn round_point(bounds: Bounds<Pixels>, share: f32) -> Point<Pixels> {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let perimeter = 2.0 * (width + height);
    let mut along = share.rem_euclid(1.0) * perimeter;
    let origin = bounds.origin;
    if along < width {
        return point(origin.x + px(along), origin.y);
    }
    along -= width;
    if along < height {
        return point(origin.x + px(width), origin.y + px(along));
    }
    along -= height;
    if along < width {
        return point(origin.x + px(width - along), origin.y + px(height));
    }
    along -= width;
    point(origin.x, origin.y + px(height - along))
}

/// The light round the picture: the rest of the ring faint, and a head with
/// a tail running round once each `EDGE_TURN`.
pub(super) fn paint_edge_light(
    window: &mut Window,
    placement: Placement,
    color: Rgba,
    opacity: f32,
    since: Instant,
) {
    let out = px(scene::EDGE_GAP + scene::EDGE_WIDTH / 2.0);
    let ring = Bounds::new(
        point(
            placement.picture.origin.x - out,
            placement.picture.origin.y - out,
        ),
        size(
            placement.picture.size.width + out * 2.0,
            placement.picture.size.height + out * 2.0,
        ),
    );
    // Quiet under the light; whole and steady under Reduce Motion, where
    // no light runs to say that something works.
    let still = motion::reduced();
    let rest = if still {
        scene::EDGE_STILL
    } else {
        scene::EDGE_REST
    };
    window.paint_quad(quad(
        ring,
        Corners::all(placement.picture_radius + out),
        theme::with_alpha(color, 0.0),
        px(scene::EDGE_WIDTH),
        theme::with_alpha(color, rest * opacity),
        BorderStyle::Solid,
    ));
    if still {
        return;
    }
    let head = motion::phase(since, scene::EDGE_TURN);
    let pieces = 28;
    for piece in 0..pieces {
        let from = head - scene::EDGE_TAIL * (1.0 - piece as f32 / pieces as f32);
        let to = head - scene::EDGE_TAIL * (1.0 - (piece + 1) as f32 / pieces as f32);
        let strength = (piece + 1) as f32 / pieces as f32;
        let mut path = PathBuilder::stroke(px(scene::EDGE_WIDTH));
        path.move_to(round_point(ring, from));
        // A piece that turns a corner goes by it.
        let corner_from = (from.rem_euclid(1.0) * 4.0).floor();
        let corner_to = (to.rem_euclid(1.0) * 4.0).floor();
        if corner_from != corner_to {
            let middle = (from + to) / 2.0;
            path.line_to(round_point(ring, middle));
        }
        path.line_to(round_point(ring, to));
        if let Ok(path) = path.build() {
            window.paint_path(path, theme::with_alpha(color, opacity * strength));
        }
    }
}
