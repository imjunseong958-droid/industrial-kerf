//! The stage: the picture, in the frame of the device it is made for (a
//! laptop for 16:9 and wider, a phone for vertical and square), with its
//! transport under it.
//!
//! Nothing moves the picture's colours: the room around it stays neutral.
//! While something works a light runs round just outside the picture's
//! edge, blue for Kerf and amber for the computer, amber winning; the chip
//! at its top left says what runs. Both fade in, and out when it ends;
//! under Reduce Motion the ring is still.

use crate::icons::Icon;
use crate::model;
use crate::motion::{self, Tween};
use crate::session::Session;
use crate::theme::{self, layout, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::{
    canvas, div, point, prelude::*, px, quad, size, App, BorderStyle, Bounds, Context, Corners,
    Entity, PathBuilder, Pixels, Point, Rgba, SharedString, Window,
};
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

/// Which of the two lights runs round the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hue {
    Kerf,
    Computer,
}

impl Hue {
    fn color(self) -> Rgba {
        match self {
            Hue::Kerf => theme::activity::kerf(),
            Hue::Computer => theme::activity::gpu(),
        }
    }
}

/// Where the picture and its device sit within the stage.
#[derive(Clone, Copy, Debug)]
struct Placement {
    device: Bounds<Pixels>,
    picture: Bounds<Pixels>,
    device_radius: Pixels,
    picture_radius: Pixels,
    phone: bool,
}

fn place(area: Bounds<Pixels>, aspect: f32) -> Placement {
    let phone = aspect < 1.2;
    let margin = px(24.0);
    let bezel = if phone { px(9.0) } else { px(8.0) };
    let room = size(
        (area.size.width - margin * 2.0).max(px(40.0)),
        (area.size.height - margin * 2.0).max(px(40.0)),
    );
    // The picture fits the room less the bezel, at its own aspect.
    let inner = size(room.width - bezel * 2.0, room.height - bezel * 2.0);
    let (width, height) = if f32::from(inner.width) / f32::from(inner.height) > aspect {
        (inner.height * aspect, inner.height)
    } else {
        (inner.width, inner.width / aspect)
    };
    let picture = Bounds::new(
        point(
            area.origin.x + (area.size.width - width) / 2.0,
            area.origin.y + (area.size.height - height) / 2.0,
        ),
        size(width, height),
    );
    let device = Bounds::new(
        point(picture.origin.x - bezel, picture.origin.y - bezel),
        size(width + bezel * 2.0, height + bezel * 2.0),
    );
    let (device_radius, picture_radius) = if phone {
        let radius = (width.min(height) * 0.14).min(px(46.0));
        (radius + bezel, radius)
    } else {
        (px(14.0), px(6.0))
    };
    Placement {
        device,
        picture,
        device_radius,
        picture_radius,
        phone,
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

pub struct StageView {
    session: Entity<Session>,
    area: Rc<Cell<Option<Bounds<Pixels>>>>,
    hue: Option<Hue>,
    /// The light's hue as it fades out.
    last_hue: Hue,
    edge: Tween,
    edge_since: Instant,
}

impl StageView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            area: Rc::new(Cell::new(None)),
            hue: None,
            last_hue: Hue::Kerf,
            edge: Tween::at(0.0),
            edge_since: Instant::now(),
        }
    }

    /// What runs, and the light that says so.
    fn activity(&self, cx: &App) -> (Option<Hue>, Option<String>) {
        let state = &self.session.read(cx).state;
        if let Some(rendering) = state.rendering() {
            return (Some(Hue::Computer), Some(rendering));
        }
        if let Some(making) = state.making_label() {
            return (Some(Hue::Computer), Some(making));
        }
        if state.work.busy() {
            let words = if state.agent_clips.is_empty() {
                "Kerf is looking"
            } else {
                "Kerf is cutting"
            };
            return (Some(Hue::Kerf), Some(words.to_owned()));
        }
        (None, None)
    }

    fn transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let fps = session.timeline().rate();
        let position = model::timecode(session.state.position, fps);
        let length = model::timecode(session.state.program_length(), fps);
        let playing = session.state.playing;
        let preview = session.state.preview.clone();
        let preview_control = match preview.status.as_str() {
            "required" => Some(
                ui::button("render-preview", "Render preview")
                    .small()
                    .icon(Icon::Sparkle)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, _| session.preview(None))
                    }))
                    .into_any_element(),
            ),
            "running" => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(ui::chip(
                        format!("Rendering {:.0}%", preview.progress.unwrap_or(0.0) * 100.0),
                        theme::activity::gpu(),
                    ))
                    .child(ui::button("stop-preview", "Stop").small().ghost().on_click(
                        cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(json!({"type": "preview_stop"}));
                            })
                        }),
                    ))
                    .into_any_element(),
            ),
            "ready" | "live" if preview.proposal.is_some() => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(ui::chip("Previewing the proposal", theme::pos()))
                    .child(
                        ui::button("exit-preview", "Back to the cut")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.update(cx, |session, _| {
                                    session.send(json!({"type": "preview_exit"}));
                                })
                            })),
                    )
                    .into_any_element(),
            ),
            "unavailable" => preview
                .message
                .clone()
                .map(|message| ui::chip(message, theme::ink3()).into_any_element()),
            _ => None,
        };
        div()
            .h(px(layout::TRANSPORT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(12.0))
            .child(
                icon_button("previous-frame", Icon::StepBack)
                    .tooltip("Previous frame")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::commands::RunCommand::new("playback.previousFrame")),
                            cx,
                        );
                    })),
            )
            .child(
                icon_button("play", if playing { Icon::Pause } else { Icon::Play })
                    .tooltip(if playing { "Pause" } else { "Play" })
                    .color(theme::ink())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session
                            .update(cx, |session, _| session.toggle_playback())
                    })),
            )
            .child(
                icon_button("next-frame", Icon::StepForward)
                    .tooltip("Next frame")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::commands::RunCommand::new("playback.nextFrame")),
                            cx,
                        );
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(8.0))
                    .child(ui::label(position, text::TIMECODE_STRONG, theme::ink()))
                    .child(ui::label("/", text::TIMECODE, theme::ink3()))
                    .child(ui::label(length, text::TIMECODE, theme::ink3())),
            )
            .child(div().flex_1())
            .children(preview_control)
    }
}

impl Render for StageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (hue, chip) = self.activity(cx);
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
        let edge = self.edge.sample(window);
        if edge > 0.0 && !motion::reduced() {
            window.request_animation_frame();
        }
        let session = self.session.read(cx);
        let timeline = session.timeline();
        let picture = session
            .picture
            .as_ref()
            .map(|picture| (picture.image.clone(), picture.width, picture.height));
        let aspect = match (&picture, timeline.width > 0) {
            (_, true) => timeline.aspect(),
            (Some((_, width, height)), false) if *height > 0 => *width as f32 / *height as f32,
            _ => 16.0 / 9.0,
        };
        let caption = session.state.caption.clone();
        let stand_in: Option<SharedString> = if picture.is_some() {
            None
        } else if !session.state.has_snapshot {
            Some("Connecting to kerfd…".into())
        } else if !timeline.has_clips() {
            Some("No picture yet".into())
        } else {
            Some(session.project_name().into())
        };

        // Overlays sit on the picture as it was placed last frame.
        // The canvas is placed in window coordinates; the overlays in the stage's own.
        let placed = self
            .area
            .get()
            .map(|area| place(Bounds::new(point(px(0.0), px(0.0)), area.size), aspect));
        let area_cell = self.area.clone();
        let hue_color = self.last_hue.color();
        let since = self.edge_since;
        let image = picture.as_ref().map(|(image, _, _)| image.clone());

        let stage = canvas(
            move |bounds, window, _| {
                if area_cell.get() != Some(bounds) {
                    area_cell.set(Some(bounds));
                    window.refresh();
                }
                place(bounds, aspect)
            },
            move |_, placement: Placement, window, _| {
                // The device: a bezel, a hairline round it.
                window.paint_quad(quad(
                    placement.device,
                    Corners::all(placement.device_radius),
                    gpui::rgb(0x0a0a0a),
                    px(1.0),
                    theme::line2(),
                    BorderStyle::Solid,
                ));
                window.paint_quad(quad(
                    placement.picture,
                    Corners::all(placement.picture_radius),
                    theme::overlay_screen(),
                    px(0.0),
                    theme::line(),
                    BorderStyle::Solid,
                ));
                if let Some(image) = image.clone() {
                    let _ = window.paint_image(
                        placement.picture,
                        Corners::all(placement.picture_radius),
                        image,
                        0,
                        false,
                    );
                }
                if placement.phone {
                    // The phone's island.
                    let island = size(
                        placement.picture.size.width * 0.28,
                        px(18.0).min(placement.picture.size.height * 0.03),
                    );
                    let origin = point(
                        placement.picture.center().x - island.width / 2.0,
                        placement.picture.origin.y + px(8.0),
                    );
                    window.paint_quad(quad(
                        Bounds::new(origin, island),
                        Corners::all(island.height / 2.0),
                        gpui::rgb(0x000000),
                        px(0.0),
                        theme::line(),
                        BorderStyle::Solid,
                    ));
                }
                if edge > 0.001 {
                    paint_edge_light(window, placement, hue_color, edge, since);
                }
            },
        )
        .absolute()
        .size_full();

        let mut overlays = div().absolute().inset_0();
        if let Some(placed) = placed {
            let picture = placed.picture;
            let left = picture.origin.x;
            let top = picture.origin.y;
            if let Some(chip) = chip.clone() {
                let color = self.last_hue.color();
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left + px(12.0))
                        .top(top + px(if placed.phone { 36.0 } else { 12.0 }))
                        .child(
                            theme::type_style(div(), text::CONTROL)
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .h(px(24.0))
                                .px(px(10.0))
                                .rounded(px(12.0))
                                .bg(theme::activity::chip())
                                .text_color(theme::ink())
                                .child(ui::dot(color, 6.0))
                                .child(chip),
                        )
                        .opacity(edge.max(0.0)),
                );
            }
            if let Some(stand_in) = stand_in {
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left)
                        .top(top)
                        .w(picture.size.width)
                        .h(picture.size.height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::label(stand_in, text::BODY, theme::ink3())),
                );
            }
            if !caption.trim().is_empty() {
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left)
                        .w(picture.size.width)
                        .top(top + picture.size.height * 0.82)
                        .flex()
                        .justify_center()
                        .child(
                            theme::type_style(div(), text::CAPTION)
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(theme::radius::SMALL))
                                .bg(theme::with_alpha(gpui::rgb(0x000000), 0.72))
                                .text_color(gpui::rgb(0xffffff))
                                .child(caption),
                        ),
                );
            }
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::surround())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(stage)
                    .child(overlays),
            )
            .child(self.transport(cx))
    }
}

/// The light round the picture: the rest of the ring faint, and a head with
/// a tail running round once each `EDGE_TURN`.
fn paint_edge_light(
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
    window.paint_quad(quad(
        ring,
        Corners::all(placement.picture_radius + out),
        theme::with_alpha(color, 0.0),
        px(scene::EDGE_WIDTH),
        theme::with_alpha(color, scene::EDGE_REST * opacity),
        BorderStyle::Solid,
    ));
    if motion::reduced() {
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
