//! The chip itself, in the toolbar: the branch and how far it is from the
//! remote, what a sync is doing while it runs, a dot for edits not
//! committed or a sync that failed; its own Tab stop.

use super::said::stage_words;
use super::{Collab, Menu};
use crate::motion;
use crate::theme::{self, height, motion as scene, radius, text};
use crate::ui;
use gpui::{
    canvas, point, prelude::*, px, Context, Div, PathBuilder, Rgba, SharedString, Stateful, Window,
};
use kerf_bridge::event::CollabStatus;
use std::time::Instant;

impl Collab {
    /// The chip: where the project stands, a dot for edits not committed
    /// or a sync that failed, and its tip while its menu is closed.
    pub(super) fn chip(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let status = self.status.clone().unwrap_or_default();
        let busy = self.busy();
        let open = self.menu != Menu::Closed;
        let conflicted = status.git.conflicted;
        let ink = if conflicted {
            theme::neg()
        } else {
            theme::ink2()
        };
        let chip = self.standing(
            self.chip_frame(open, ink, window, cx),
            &status,
            busy,
            ink,
            window,
        );
        let mark = if self.failure.last().is_some() && !busy {
            Some(theme::neg())
        } else if status.git.changed && !busy && !conflicted {
            Some(theme::ink2())
        } else {
            None
        };
        let tooltip: SharedString = match (self.failure.last(), status.git.changed) {
            (Some(failure), _) => format!("Sync failed: {failure}").into(),
            _ if conflicted => "A merge is unfinished; finish it with git".into(),
            (None, true) => "Edits not committed yet".into(),
            _ => "Sync".into(),
        };
        // Its menu says all the tip would, so the tip waits while it is open.
        ui::tip_on(
            chip.children(mark.map(|color| ui::dot(color, 6.0))),
            (!open).then(|| ui::Tip::new(tooltip)),
        )
    }

    /// The chip's frame, the kit's trigger: another menu open goes on its
    /// press, and pressed again, its own menu goes rather than opening anew.
    fn chip_frame(
        &self,
        open: bool,
        ink: Rgba,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        theme::type_style(
            ui::trigger(
                "collab-chip",
                cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| this.toggle(window, cx)),
            ),
            text::CONTROL,
        )
        .flex()
        .flex_none()
        .items_center()
        .gap(px(theme::space::S6))
        .h(px(height::H24))
        .px(px(theme::space::S8))
        .rounded(px(radius::CONTROL))
        .border_1()
        .border_color(theme::line())
        .cursor_pointer()
        .text_color(ink)
        .when(open, |chip| chip.bg(theme::on()))
        .when(!open, |chip| chip.hover(|style| style.bg(theme::raised())))
        // A Tab stop of its own: ringed while it has the keys, Enter or
        // Space opens its menu; a press leaves the keys where they were.
        .relative()
        .track_focus(&self.chip_focus)
        .key_context(ui::CONTROL)
        .capture_any_mouse_down(|_, window, _| window.prevent_default())
        .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
        .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, _| {
            let keystroke = &event.keystroke;
            if matches!(keystroke.key.as_str(), "enter" | "space")
                && !keystroke.modifiers.modified()
            {
                this.chip_armed = true;
            }
        }))
        .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
            if event.is_keyboard() && std::mem::take(&mut this.chip_armed) {
                this.toggle(window, cx);
            }
        }))
        .when(self.chip_focus.is_focused(window), |chip| {
            chip.child(ui::ring(radius::CONTROL))
        })
    }

    /// What the chip says: what a sync is doing while it runs, that the
    /// project is not shared, or its branch and how far it is from the
    /// remote, and a merge left unfinished.
    fn standing(
        &self,
        chip: Stateful<Div>,
        status: &CollabStatus,
        busy: bool,
        ink: Rgba,
        window: &Window,
    ) -> Stateful<Div> {
        let conflicted = status.git.conflicted;
        if busy {
            chip.child(spinner(self.busy_since, theme::activity::kerf(), window))
                .child(stage_words(self.stage.as_ref()))
        } else if !status.git.repository {
            chip.child(branch_glyph(theme::ink3())).child(ui::label(
                "Not shared",
                text::CONTROL,
                theme::ink2(),
            ))
        } else {
            // Live sync's glyph is sage: connected.
            let glyph = if status.live { theme::pos() } else { ink };
            chip.child(branch_glyph(glyph))
                .child(
                    ui::line(
                        status
                            .git
                            .branch
                            .clone()
                            .unwrap_or_else(|| "detached".into()),
                        text::CONTROL,
                        ink,
                    )
                    .max_w(px(140.0)),
                )
                .when(status.git.ahead > 0, |chip| {
                    chip.child(ui::label(
                        format!("↑{}", status.git.ahead),
                        text::CONTROL,
                        ink,
                    ))
                })
                .when(status.git.behind > 0, |chip| {
                    chip.child(ui::label(
                        format!("↓{}", status.git.behind),
                        text::CONTROL,
                        ink,
                    ))
                })
                .when(conflicted, |chip| {
                    chip.child(ui::label("Merge unfinished", text::CONTROL, theme::neg()))
                })
        }
    }
}

/// A branch drawn as a line with a fork off it, in `color`.
fn branch_glyph(color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |area, _, window, _| {
            let scale = f32::from(area.size.width) / 16.0;
            let at = |x: f32, y: f32| {
                point(area.origin.x + px(x * scale), area.origin.y + px(y * scale))
            };
            let mut builder = PathBuilder::stroke(px(1.3));
            let ring = |builder: &mut PathBuilder, x: f32, y: f32| {
                let r = 1.9;
                builder.move_to(at(x + r, y));
                builder.arc_to(
                    point(px(r * scale), px(r * scale)),
                    px(0.0),
                    false,
                    true,
                    at(x - r, y),
                );
                builder.arc_to(
                    point(px(r * scale), px(r * scale)),
                    px(0.0),
                    false,
                    true,
                    at(x + r, y),
                );
            };
            ring(&mut builder, 5.0, 3.5);
            ring(&mut builder, 5.0, 12.5);
            ring(&mut builder, 11.5, 4.5);
            builder.move_to(at(5.0, 5.4));
            builder.line_to(at(5.0, 10.6));
            builder.move_to(at(11.5, 6.4));
            builder.cubic_bezier_to(at(5.0, 10.6), at(11.5, 9.0), at(5.0, 8.0));
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(14.0))
    .flex_none()
}

/// A ring turning once each [`scene::WORK_PULSE`] while work runs; still
/// under Reduce Motion.
fn spinner(since: Instant, color: Rgba, window: &Window) -> impl IntoElement {
    let phase = motion::phase(since, scene::WORK_PULSE);
    if !motion::reduced() {
        window.request_animation_frame();
    }
    canvas(
        |_, _, _| (),
        move |area, _, window, _| {
            let size = f32::from(area.size.width.min(area.size.height));
            let r = size / 2.0 - 1.5;
            let centre = point(
                area.origin.x + px(size / 2.0),
                area.origin.y + px(size / 2.0),
            );
            let at = |angle: f32| {
                point(
                    centre.x + px(r * angle.cos()),
                    centre.y + px(r * angle.sin()),
                )
            };
            let start = phase * std::f32::consts::TAU;
            let end = start + std::f32::consts::PI * 1.5;
            let mut track = PathBuilder::stroke(px(1.5));
            track.move_to(at(0.0));
            track.arc_to(
                point(px(r), px(r)),
                px(0.0),
                false,
                true,
                at(std::f32::consts::PI),
            );
            track.arc_to(point(px(r), px(r)), px(0.0), false, true, at(0.0));
            if let Ok(path) = track.build() {
                window.paint_path(path, theme::track());
            }
            let mut arc = PathBuilder::stroke(px(1.5));
            arc.move_to(at(start));
            arc.arc_to(point(px(r), px(r)), px(0.0), true, true, at(end));
            if let Ok(path) = arc.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(12.0))
    .flex_none()
}
