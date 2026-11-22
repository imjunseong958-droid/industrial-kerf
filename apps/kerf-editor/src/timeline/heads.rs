//! The track heads at the left of the lanes: the rate over them, and for
//! each track its target chip, its name, Solo for sound, its mute or eye,
//! its lock and the line under it that resizes it; under the last an
//! add-track row; and the track switches they press.

use super::drag::Gesture;
use super::heights::ResizeDrag;
use super::lanes::{kind_colour, Row};
use super::strip::strip_button_owned;
use super::{TimelineEvent, TimelineView};
use crate::icons::Icon;
use crate::model::{self, TrackKind};
use crate::theme::{self, layout, radius, text};
use crate::ui;
use gpui::{
    div, point, prelude::*, px, App, Context, CursorStyle, ElementId, MouseButton, MouseDownEvent,
    Rgba, SharedString,
};
use kerf_bridge::Command;
use serde_json::json;

/// Points either side of the line under a head that take a resize.
const GRIP: f32 = 3.0;
/// The add-track row under the last track.
pub(super) const ADD_ROW: f32 = 28.0;

impl TimelineView {
    pub(super) fn heads(&self, cx: &Context<Self>) -> impl IntoElement {
        let lanes = self.lanes.clone();
        let scroll_y = self.view.scroll_y;
        let rate = lanes.fps;
        let rate = if (rate - rate.round()).abs() < 0.005 {
            format!("{} fps", rate.round())
        } else {
            format!("{rate:.2} fps")
        };
        let targets = self.targets_now(cx);
        let read_only = lanes.proposed || self.offline(cx);
        let rows: Vec<_> = lanes
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.head(index, row, scroll_y, read_only, &targets, cx))
            .collect();
        let add_top = lanes.height - scroll_y;
        div()
            .w(px(layout::TRACK_HEADER))
            .flex_none()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(theme::chrome())
            .border_r_1()
            .border_color(theme::line())
            .on_scroll_wheel(cx.listener(Self::heads_wheel))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(layout::RULER))
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(ui::label(rate, text::META, theme::ink3())),
            )
            .child(
                div()
                    .absolute()
                    .top(px(layout::RULER))
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .overflow_hidden()
                    .children(rows)
                    // Under the last track: a row whose + adds one.
                    .when(!read_only, |heads| heads.child(self.add_row(add_top, cx))),
            )
    }

    /// Under the last track, `top` points down the heads: a row whose +
    /// opens the add-track menu.
    fn add_row(&self, top: f32, cx: &Context<Self>) -> impl IntoElement {
        let add_hover = self.add_hover;
        div()
            .id("timeline-add-row")
            .absolute()
            .top(px(top))
            .left_0()
            .right_0()
            .h(px(ADD_ROW))
            .flex()
            .items_center()
            .pl(px(6.0))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.add_hover = *hovered;
                cx.notify();
            }))
            .child(ui::tipped(
                "timeline-add-track-tip",
                ui::Tip::new("Add track"),
                div()
                    .id("timeline-add-track")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(26.0))
                    .h(px(18.0))
                    .rounded(px(radius::SMALL))
                    .border_1()
                    .border_color(if add_hover {
                        theme::lineh()
                    } else {
                        theme::line()
                    })
                    .cursor_pointer()
                    .child(ui::label(
                        "+",
                        text::CONTROL_STRONG,
                        if add_hover {
                            theme::ink()
                        } else {
                            theme::ink3()
                        },
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            let at = point(event.position.x, event.position.y + px(10.0));
                            this.add_track_menu(at, false, window, cx);
                            window.prevent_default();
                        }),
                    ),
            ))
    }

    /// Track `row`'s head, the `index`th, its lanes scrolled `scroll_y`
    /// down: its target chip, its name, Solo, its switch and its lock,
    /// and the line under it (while it can be pressed, not `read_only`).
    fn head(
        &self,
        index: usize,
        row: &Row,
        scroll_y: f32,
        read_only: bool,
        targets: &[String],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let label_ink = if row.quiet() {
            theme::ink3()
        } else {
            theme::ink2()
        };
        let control = (row.height - 6.0).clamp(16.0, 22.0);
        let lock = self.head_lock(row, control, read_only, cx);
        let switch = self.head_switch(row, control, read_only, cx);
        let solo = self.head_solo(row, control, cx);
        let target = self.head_target(row, read_only, targets, label_ink, cx);
        let grip = self.head_grip(row, cx);
        div()
            .id(ElementId::Name(format!("track-head-{}", row.id).into()))
            .absolute()
            .top(px(row.top - scroll_y))
            .left_0()
            .right_0()
            .h(px(row.height))
            .flex()
            .items_center()
            .gap(px(2.0))
            .pl(px(6.0))
            .pr(px(4.0))
            .border_b_1()
            .border_color(theme::line())
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.head_menu(index, event.position, window, cx);
                    window.prevent_default();
                }),
            )
            .child(target)
            .child(div().w(px(4.0)).flex_none())
            // Its name, when it is more than its badge says.
            .child(
                ui::line(
                    if row.label == row.badge {
                        SharedString::default()
                    } else {
                        row.label.clone()
                    },
                    text::META,
                    label_ink,
                )
                .flex_1()
                .min_w_0(),
            )
            .children(solo)
            .child(switch)
            .child(lock)
            .when(!read_only, |head| head.child(grip))
    }

    /// A head's lock: its clips chosen, never dragged.
    fn head_lock(
        &self,
        row: &Row,
        control: f32,
        read_only: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let (track, locked) = (row.id.clone(), row.locked);
        ui::icon_button(
            ElementId::Name(format!("track-lock-{}", row.id).into()),
            Icon::Lock,
        )
        .size(control)
        .on(locked)
        .disabled(read_only)
        .tooltip(if locked { "Unlock track" } else { "Lock track" })
        .on_click(cx.listener(move |this, _, _, cx| this.set_track(&track, "locked", !locked, cx)))
    }

    /// A head's switch: Mute for sound, the eye for the rest.
    fn head_switch(
        &self,
        row: &Row,
        control: f32,
        read_only: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        if row.audio() {
            let (track, muted) = (row.id.clone(), row.muted);
            ui::icon_button(
                ElementId::Name(format!("track-mute-{}", row.id).into()),
                Icon::Mute,
            )
            .size(control)
            .on(muted)
            .disabled(read_only)
            .tooltip(if muted { "Unmute" } else { "Mute" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "muted", !muted, cx)),
            )
        } else {
            let (track, enabled) = (row.id.clone(), row.enabled);
            ui::icon_button(
                ElementId::Name(format!("track-eye-{}", row.id).into()),
                Icon::Eye,
            )
            .size(control)
            .color(if enabled { theme::ink3() } else { theme::dis() })
            .disabled(read_only)
            .tooltip(if enabled { "Hide track" } else { "Show track" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "enabled", !enabled, cx)),
            )
        }
    }

    /// Solo, for sound: what playback hears alone, for this session;
    /// nothing to hear while kerfd is not connected.
    fn head_solo(&self, row: &Row, control: f32, cx: &Context<Self>) -> Option<impl IntoElement> {
        let offline = self.offline(cx);
        row.audio().then(|| {
            let track = row.id.clone();
            let on = self.solo.contains(&row.id);
            ui::tipped(
                ElementId::Name(format!("track-solo-tip-{}", row.id).into()),
                ui::Tip::new("Solo (monitoring only)"),
                if offline {
                    // Greyed, with no hover and nothing to press.
                    theme::type_style(div(), text::CONTROL)
                        .id(ElementId::Name(format!("track-solo-{}", row.id).into()))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .h(px(control))
                        .min_w(px(control))
                        .rounded(px(radius::CONTROL))
                        .text_color(theme::dis())
                        .child("S")
                } else {
                    strip_button_owned(
                        ElementId::Name(format!("track-solo-{}", row.id).into()),
                        "S",
                        on,
                    )
                    .h(px(control))
                    .min_w(px(control))
                    .px(px(0.0))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_solo(&track, cx)))
                },
            )
        })
    }

    /// The kind chip, a target for picture and sound: where source edits
    /// land. Its tip says whether it is targeted.
    fn head_target(
        &self,
        row: &Row,
        read_only: bool,
        targets: &[String],
        label_ink: Rgba,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let targetable = row.kind != TrackKind::Subtitle && !read_only;
        let targeted = targetable && targets.contains(&row.id);
        let colour = kind_colour(row.kind);
        let chip = {
            let track = row.id.clone();
            div()
                .id(ElementId::Name(format!("track-target-{}", row.id).into()))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(26.0))
                .h(px((row.height - 8.0).clamp(14.0, 18.0)))
                .rounded(px(radius::SMALL))
                .border_1()
                .border_color(if targeted {
                    colour
                } else {
                    theme::with_alpha(colour, if row.quiet() { 0.3 } else { 0.6 })
                })
                .when(targeted, |chip| chip.bg(theme::with_alpha(colour, 0.85)))
                .child(ui::label(
                    row.badge.clone(),
                    text::BADGE,
                    if targeted {
                        theme::surround()
                    } else {
                        label_ink
                    },
                ))
                .when(targetable, |chip| {
                    chip.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_target(&track, cx)))
                })
        };
        let chip_tip = if !targetable {
            format!("{} · {}", row.badge, row.label)
        } else if targeted {
            format!("{} · Targeted", row.badge)
        } else {
            format!("{} · Target track", row.badge)
        };
        ui::tipped(
            ElementId::Name(format!("track-target-tip-{}", row.id).into()),
            ui::Tip::new(chip_tip),
            chip,
        )
    }

    /// The line under a head: dragged, it resizes the track (Option: every
    /// track of its kind).
    fn head_grip(&self, row: &Row, cx: &Context<Self>) -> impl IntoElement {
        let (track, kind, height) = (row.id.clone(), row.kind, row.height);
        let words = format!(
            "Drag to resize {}. Option-drag resizes every {} track; Option-scroll over the heads resizes every track.",
            row.badge,
            if row.audio() { "sound" } else { "picture" }
        );
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(GRIP + 1.0))
            .child(ui::tipped(
                ElementId::Name(format!("track-grip-tip-{}", row.id).into()),
                ui::Tip::new(words),
                div()
                    .id(ElementId::Name(format!("track-grip-{}", row.id).into()))
                    .w(px(layout::TRACK_HEADER))
                    .h(px(GRIP + 1.0))
                    .cursor(CursorStyle::ResizeUpDown)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, _| {
                            this.gesture = Some(Gesture::Resize(ResizeDrag {
                                track: track.clone(),
                                kind,
                                every: event.modifiers.alt,
                                anchor: f32::from(event.position.y),
                                from: height,
                            }));
                        }),
                    ),
            ))
    }

    /// Switches a track's `switch` (`locked`, `muted`, `enabled`,
    /// `syncLocked`) to `value`.
    pub(super) fn set_track(
        &mut self,
        track: &str,
        switch: &str,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        if self.lanes.proposed || self.offline(cx) {
            return;
        }
        let mut op = json!({"op": "set-track", "track": track});
        op[switch] = json!(value);
        self.session.update(cx, |session, cx| {
            session.edit(vec![op]);
            cx.notify();
        });
    }

    /// The targets as they stand: the person's, else the first unlocked
    /// track of each kind.
    fn targets_now(&self, cx: &App) -> Vec<String> {
        match &self.targets {
            Some(targets) => targets.clone(),
            None => model::default_targets(self.session.read(cx).timeline()),
        }
    }

    /// Track `id` targeted in place of its kind's other, or not targeted.
    fn toggle_target(&mut self, id: &str, cx: &mut Context<Self>) {
        let targets =
            model::toggle_target(self.session.read(cx).timeline(), &self.targets_now(cx), id);
        self.targets = Some(targets);
        cx.emit(TimelineEvent::Targets(self.targets.clone()));
        cx.notify();
    }

    fn toggle_solo(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.solo.iter().position(|known| known == id) {
            Some(at) => {
                self.solo.remove(at);
            }
            None => self.solo.push(id.to_owned()),
        }
        self.send_solo(cx);
        cx.notify();
    }

    /// Tells kerfd which sound tracks playback hears alone (none: all).
    pub(super) fn send_solo(&mut self, cx: &mut Context<Self>) {
        let tracks = self.solo.clone();
        self.session.update(cx, |session, _| {
            session.send(Command::Solo { tracks });
        });
    }
}
