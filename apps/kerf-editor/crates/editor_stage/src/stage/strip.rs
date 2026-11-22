//! The change reel's strip ([`reel_strip`]), floating over the picture
//! while what a proposal changed plays.

use editor_session::session::Session;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, text};
use editor_ui::ui::{self, icon_button};
use gpui::{div, prelude::*, px, AnimationExt, AnyElement, App, Entity, Window};

/// The change reel's strip, to float over the picture while it plays:
/// "2 / 5", what the segment is with the seconds left ("· getting the
/// picture" while it waits, "Watched every change" at its end), previous
/// and next, Before|After, and Exit. None while no reel plays. The window
/// places it over the picture's foot (`StageView`).
pub fn reel_strip(session: &Entity<Session>, cx: &App) -> Option<AnyElement> {
    let reel = session.read(cx).reel()?;
    let playhead = session.read(cx).state.position;
    let count = reel.segments.len();
    let index = reel.index;
    let side = reel.side;
    let words = reel.words(playhead);
    let step = |forward: bool| {
        let session = session.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            session.update(cx, |session, cx| {
                session.reel_step(forward);
                cx.notify();
            })
        }
    };
    let sides = {
        let session = session.clone();
        ui::segmented(
            "reel-sides",
            &[
                (editor_session::session::ReelSide::Before, "Before"),
                (editor_session::session::ReelSide::After, "After"),
            ],
            side,
            move |side, _, cx| {
                session.update(cx, |session, cx| {
                    session.reel_side(side);
                    cx.notify();
                })
            },
        )
        .tips(&["The program as it is", "What the proposal makes"])
    };
    let exit = {
        let session = session.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            session.update(cx, |session, cx| {
                session.reel_stop();
                cx.notify();
            })
        }
    };
    Some(
        div()
            .id("change-reel")
            .h(px(40.0))
            .pl(px(theme::space::S12))
            .pr(px(theme::space::S8))
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .rounded(px(theme::radius::PANEL))
            .bg(theme::with_alpha(theme::chrome(), 0.94))
            .border_1()
            .border_color(theme::line2())
            .occlude()
            .child(ui::label(
                format!("{} / {count}", index + 1),
                text::TIMECODE,
                theme::accent_ink(),
            ))
            .child(
                ui::line(words, text::CONTROL, theme::ink2())
                    .flex_1()
                    .min_w_0(),
            )
            .child(
                icon_button("reel-previous", Icon::ChevronLeft)
                    .tooltip("Previous change")
                    .disabled(index == 0)
                    .on_click(step(false)),
            )
            .child(
                icon_button("reel-next", Icon::ChevronRight)
                    .tooltip("Next change")
                    .disabled(index + 1 >= count)
                    .on_click(step(true)),
            )
            .child(sides)
            .child(
                ui::button("reel-exit", "Exit")
                    .small()
                    .ghost()
                    .on_click(exit),
            )
            .with_animation(
                "change-reel-in",
                motion::animation(scene::FADE),
                |strip, t| strip.opacity(t),
            )
            .into_any_element(),
    )
}
