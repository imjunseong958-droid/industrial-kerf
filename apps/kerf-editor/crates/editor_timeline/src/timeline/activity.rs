//! What a clip says of its media's making, when it is doing more than
//! standing ready ([`Activity`]): moving amber stripes while it is made
//! (still under Reduce Motion), its clock or "Queued", "Not made",
//! "Offline" in a pill at its foot ([`activity_label`]); and the clock
//! that ticks it once a second while any media is being made.

use super::canvas::{masked, shape, Fonts, Pen};
use super::lanes::Lanes;
use super::TimelineView;
use editor_state::store::{EditorState, MediaActivity};
use editor_ui::motion;
use editor_ui::theme::{self, radius, text};
use gpui::{
    point, px, quad, App, BorderStyle, Bounds, Context, PathBuilder, Pixels, Rgba, SharedString,
    Window,
};

/// What a media's clips say it is doing, when it is doing more than
/// standing ready.
#[derive(Clone)]
pub(super) struct Activity {
    pub(super) media: String,
    pub(super) state: MediaActivity,
    /// The stripes' phase, while it is being made.
    pub(super) phase: f32,
    pub(super) label: SharedString,
}

impl Activity {
    pub(super) fn making(&self) -> bool {
        matches!(self.state, MediaActivity::Making { .. })
    }

    /// Dashed: waiting its turn, or with no file to show.
    pub(super) fn dashed(&self) -> bool {
        self.state.absent() || matches!(self.state, MediaActivity::Queued { .. })
    }
}

/// What a clip says of its media's making, holding still or not: "Making
/// take 2 · 0:41", "Queued · 2 ahead", "Not made", "Offline".
pub fn activity_label(activity: &MediaActivity) -> Option<String> {
    Some(match activity {
        MediaActivity::Ready => return None,
        MediaActivity::Making { take, since } => {
            let elapsed = since.elapsed().as_secs();
            let clock = format!("{}:{:02}", elapsed / 60, elapsed % 60);
            if *take > 1 {
                format!("Making take {take} · {clock}")
            } else {
                format!("Making · {clock}")
            }
        }
        MediaActivity::Queued { ahead: Some(ahead) } if *ahead > 0 => {
            format!("Queued · {ahead} ahead")
        }
        MediaActivity::Queued { .. } => "Queued".into(),
        MediaActivity::NotMade => "Not made".into(),
        MediaActivity::Offline => "Offline".into(),
    })
}

/// What the media on `lanes` are doing, as `state` says, each media once:
/// where it is more than standing ready, with its stripes' phase now.
pub(super) fn activities(lanes: &Lanes, state: &EditorState) -> Vec<Activity> {
    let mut activities: Vec<Activity> = Vec::new();
    for clip in lanes.rows.iter().flat_map(|row| &row.clips) {
        let Some(media) = clip.source.as_ref().filter(|_| !clip.card()) else {
            continue;
        };
        if activities.iter().any(|known| known.media == *media) {
            continue;
        }
        let activity = state.media_activity(media);
        if let Some(label) = activity_label(&activity) {
            let phase = match &activity {
                MediaActivity::Making { since, .. } => {
                    motion::phase(*since, theme::motion::STRIPE_CYCLE)
                }
                _ => 0.0,
            };
            activities.push(Activity {
                media: media.clone(),
                state: activity,
                phase,
                label: label.into(),
            });
        }
    }
    activities
}

impl TimelineView {
    /// Keeps a making's clock going once a second, which Reduce Motion
    /// leaves as all that moves, while any media is being made.
    pub(super) fn tick_makings(&mut self, cx: &mut Context<Self>) {
        if self.clock.is_some() || self.session.read(cx).state.makings.is_empty() {
            return;
        }
        self.clock = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(theme::motion::CLOCK_FRAME)
                .await;
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
}

/// What a clip's media is doing, in a pill at its foot: the making's
/// clock, "Queued", "Not made" (ink2), "Offline" (brick).
#[expect(
    clippy::too_many_arguments,
    reason = "painting takes the window, where the canvas is and each measure it draws with"
)]
pub(super) fn paint_activity(
    activity: &Activity,
    pen: Pen,
    width: f32,
    (x0, x1): (f32, f32),
    (top, tall): (f32, f32),
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let ink = match activity.state {
        MediaActivity::Offline => theme::neg(),
        MediaActivity::NotMade => theme::ink2(),
        _ => theme::activity::gpu(),
    };
    let style = if activity.making() {
        text::MONO_SMALL
    } else {
        text::META
    };
    let font = if activity.making() {
        &fonts.mono
    } else {
        &fonts.ui
    };
    let line = shape(window, activity.label.clone(), style, font, ink);
    let across = f32::from(line.width) + 8.0;
    let right = x1.min(width) - 4.0;
    let left = right - across;
    if left < x0.max(0.0) + 4.0 || x1 - x0 < 24.0 {
        return;
    }
    // Under the name in a clip tall enough, else at its middle.
    let pill_top = if tall >= 30.0 {
        top + tall - 18.0
    } else {
        top + (tall - 16.0) / 2.0
    };
    let pill = pen.rect(left, pill_top, across, 16.0);
    masked(window, pen.rect(x0, top, x1 - x0, tall), |window| {
        window.paint_quad(quad(
            pill,
            px(radius::SMALL),
            theme::activity::chip(),
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
        if let Err(error) = line.paint(
            pen.at(left + 4.0, pill_top + (16.0 - style.line) / 2.0),
            px(style.line),
            window,
            cx,
        ) {
            log::warn!("a clip's activity did not draw: {error}");
        }
    });
}

/// Bands leaning right, half a space wide, a space apart, moved on by
/// `phase` of a space, inside `area`.
pub(super) fn stripes(window: &mut Window, area: Bounds<Pixels>, phase: f32, color: Rgba) {
    let space = theme::motion::STRIPE_SPACE;
    let (left, top) = (f32::from(area.origin.x), f32::from(area.origin.y));
    let (right, bottom) = (
        left + f32::from(area.size.width),
        top + f32::from(area.size.height),
    );
    let tall = bottom - top;
    if tall <= 0.0 || right <= left {
        return;
    }
    let mut builder = PathBuilder::fill();
    let mut x = left - tall - space + phase.fract() * space;
    while x < right + space {
        builder.move_to(point(px(x), px(bottom)));
        builder.line_to(point(px(x + space / 2.0), px(bottom)));
        builder.line_to(point(px(x + space / 2.0 + tall), px(top)));
        builder.line_to(point(px(x + tall), px(top)));
        builder.close();
        x += space;
    }
    if let Ok(path) = builder.build() {
        masked(window, area, |window| window.paint_path(path, color));
    }
}
