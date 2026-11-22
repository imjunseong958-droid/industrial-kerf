//! The sidebar's edge: dragged, it sets how wide the sidebar is; pressed,
//! it puts the sidebar away after a double-click's wait; pressed twice,
//! its width goes back. Nothing shows until the pointer is over it.

use super::{SidebarEvent, SidebarView};
use crate::theme::{self, layout, motion as scene};
use crate::ui;
use gpui::{
    canvas, div, prelude::*, px, Context, DispatchPhase, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point,
};
use std::time::Instant;

/// Whether a press on the sidebar's edge at `now` is the second of a
/// double-click: the platform counts it so, or the press before it was let
/// go less than [`scene::DOUBLE_CLICK`] ago.
pub fn is_second_press(last_click: Option<Instant>, now: Instant, click_count: usize) -> bool {
    click_count >= 2
        || last_click.is_some_and(|at| now.saturating_duration_since(at) < scene::DOUBLE_CLICK)
}

/// What the sidebar's edge's tooltip says.
pub const EDGE_TIP: &str = "Drag to resize · click to hide · double-click to reset";

/// A press on the sidebar's edge, while it is held.
pub(super) struct EdgePress {
    from: Point<Pixels>,
    /// It moved past a press: it is a drag.
    moved: bool,
    /// The second press of a double-click: the width went back.
    second: bool,
}

impl SidebarView {
    /// A press on the edge: the second of a double-click puts the width
    /// back, and stops the first from putting the sidebar away.
    fn edge_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let second = is_second_press(self.edge_clicked, Instant::now(), event.click_count);
        if second {
            self.edge_hide = None;
            self.edge_clicked = None;
            self.session.update(cx, |session, cx| {
                session.prefs.sidebar_width = layout::SIDEBAR;
                session.save_prefs();
                cx.notify();
            });
        }
        self.edge_press = Some(EdgePress {
            from: event.position,
            moved: false,
            second,
        });
        cx.notify();
    }

    /// The pointer moved with the edge held: past a press, the sidebar is
    /// as wide as the pointer is far in, with no motion.
    fn edge_moved(&mut self, at: Point<Pixels>, window_width: Pixels, cx: &mut Context<Self>) {
        let Some(press) = self.edge_press.as_mut() else {
            return;
        };
        if !press.moved && f32::from((at.x - press.from.x).abs()) < layout::DRAG_START {
            return;
        }
        press.moved = true;
        self.edge_hide = None;
        self.edge_clicked = None;
        let most = layout::sidebar_most(f32::from(window_width));
        let width = f32::from(at.x).clamp(layout::SIDEBAR_LEAST, most);
        self.session.update(cx, |session, cx| {
            if (session.prefs.sidebar_width - width).abs() > f32::EPSILON {
                session.prefs.sidebar_width = width;
                cx.notify();
            }
        });
        cx.notify();
    }

    /// The edge let go: a drag keeps its width; a press waits
    /// `DOUBLE_CLICK` for a second, and with none puts the sidebar away.
    fn edge_released(&mut self, cx: &mut Context<Self>) {
        let Some(press) = self.edge_press.take() else {
            return;
        };
        if press.moved {
            self.session.read(cx).save_prefs();
        } else if !press.second {
            self.edge_clicked = Some(Instant::now());
            self.edge_hide = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(scene::DOUBLE_CLICK).await;
                let _ = this.update(cx, |this, cx| {
                    this.edge_clicked = None;
                    cx.emit(SidebarEvent::Hide);
                });
            }));
        }
        cx.notify();
    }

    /// The sidebar's edge: dragged, it sets how wide the sidebar is;
    /// pressed, it puts it away; pressed twice, the width goes back.
    /// Nothing shows until the pointer is over it; then a line, in the
    /// accent while it is dragged.
    pub(super) fn edge(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pressed = self.edge_press.is_some();
        let dragging = self.edge_press.as_ref().is_some_and(|press| press.moved);
        let line = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(layout::SIDEBAR_GRIP / 2.0 - 1.0))
            .w(px(2.0))
            .bg(if dragging {
                theme::accent()
            } else {
                theme::lineh()
            })
            .when(!pressed, |line| {
                line.invisible()
                    .group_hover("sidebar-edge", |line| line.visible())
            });
        let mut edge = div()
            .id("sidebar-edge")
            .group("sidebar-edge")
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(-layout::SIDEBAR_GRIP / 2.0))
            .w(px(layout::SIDEBAR_GRIP))
            .cursor_col_resize()
            .child(line)
            .child(ui::tip_zone(
                "sidebar-edge-tip",
                (!pressed).then(|| ui::Tip::new(EDGE_TIP)),
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.edge_down(event, cx)),
            )
            // Let go over it or anywhere else.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.edge_released(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.edge_released(cx)),
            );
        if pressed {
            // While it is held, the pointer is followed wherever it goes.
            let this = cx.weak_entity();
            edge = edge.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble {
                                let width = window.viewport_size().width;
                                let _ = this.update(cx, |this, cx| {
                                    this.edge_moved(event.position, width, cx)
                                });
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            );
        }
        edge
    }
}
