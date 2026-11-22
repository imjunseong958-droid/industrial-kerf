//! Reading a clip from footage in a shot's graph, for its way (`moves`) or
//! its matte (`matte`): the clip's first frame drawn, pointed on (on the
//! thing, or off it), followed through the clip, and what the bridge said.

use super::inspector::{note, section};
use super::paint::{paint_cross, paint_ring_line};
use super::pictures::PICTURE_MOST;
use super::ShotGraph;
use crate::theme::{self, radius};
use crate::ui;
use gpui::{
    canvas, div, fill, point, prelude::*, px, AnyElement, Bounds, Context, CursorStyle,
    MouseButton, MouseDownEvent, RenderImage,
};
use kerf_bridge::{Command, ShotFollow, ShotOf};
use std::path::PathBuf;
use std::sync::Arc;

/// How far reading a clip from footage has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PointStage {
    #[default]
    Idle,
    /// Its first frame is being drawn.
    Drawing,
    /// It is drawn, to be pointed on.
    Pointing,
    /// What was pointed at is followed through the clip.
    Following,
    /// The bridge said how that went.
    Said,
}

/// Reading a clip from footage: which clip, for its way (`moves`) or its
/// matte (`matte`); the picture pointed on; the points on the thing and
/// off it, as shares of the picture; what the bridge said.
#[derive(Clone, Debug, Default)]
pub(super) struct Pointing {
    pub(super) clip: String,
    pub(super) of: Option<ShotOf>,
    pub(super) stage: PointStage,
    pub(super) picture: Option<PathBuf>,
    on: Vec<(f64, f64)>,
    off: Vec<(f64, f64)>,
    says: String,
    failed: bool,
}

/// Where `at` is on a picture drawn at `frame` (left, top, width, height),
/// as shares of its width and height from its top left; none off it.
pub fn point_on(frame: [f32; 4], at: (f32, f32)) -> Option<(f64, f64)> {
    let [left, top, width, height] = frame;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let inside = at.0 >= left && at.0 <= left + width && at.1 >= top && at.1 <= top + height;
    inside.then(|| {
        (
            f64::from(((at.0 - left) / width).clamp(0.0, 1.0)),
            f64::from(((at.1 - top) / height).clamp(0.0, 1.0)),
        )
    })
}

/// Where a picture `size` big is drawn inside `room` (left, top, width,
/// height): whole, at its own shape, in the middle.
pub fn fitted_in(room: [f32; 4], size: (u32, u32)) -> [f32; 4] {
    let [left, top, width, height] = room;
    if size.0 == 0 || size.1 == 0 {
        return room;
    }
    let scale = (width / size.0 as f32).min(height / size.1 as f32);
    let (w, h) = (size.0 as f32 * scale, size.1 as f32 * scale);
    [left + (width - w) / 2.0, top + (height - h) / 2.0, w, h]
}

impl ShotGraph {
    /// What the bridge said of following a clip, or why it could not: a
    /// picture that could not be drawn leaves nothing to point on; a thing
    /// that could not be followed leaves the picture, to point again. The
    /// clip and what it was read for are as the bridge named them, if it did.
    pub(super) fn followed(
        &mut self,
        clip: Option<&str>,
        of: Option<ShotOf>,
        says: String,
        failed: bool,
    ) {
        let pointing = &mut self.pointing;
        let named = clip.is_some();
        let ours = clip == Some(pointing.clip.as_str()) && of.is_some() && of == pointing.of;
        let waiting = matches!(pointing.stage, PointStage::Drawing | PointStage::Following);
        if !(ours || (!named && waiting)) {
            return;
        }
        pointing.stage = if failed && pointing.picture.is_some() {
            PointStage::Pointing
        } else {
            PointStage::Said
        };
        pointing.says = says;
        pointing.failed = failed;
        pointing.on.clear();
        pointing.off.clear();
    }

    fn ask_picture(&mut self, clip: String, of: ShotOf, cx: &mut Context<Self>) {
        self.pointing = Pointing {
            clip: clip.clone(),
            of: Some(of),
            stage: PointStage::Drawing,
            ..Pointing::default()
        };
        self.session.update(cx, |session, _| {
            session.send(Command::ShotPicture { clip, of })
        });
        cx.notify();
    }

    fn point(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.pointing.stage != PointStage::Pointing {
            return;
        }
        let Some(size) = self
            .pointing
            .picture
            .as_deref()
            .and_then(|path| self.still(path))
            .map(|image| crate::artwork::image_size(&image))
        else {
            return;
        };
        let bounds = self.point_bounds.get();
        let room = [
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ];
        let Some(point) = point_on(
            fitted_in(room, size),
            (f32::from(event.position.x), f32::from(event.position.y)),
        ) else {
            return;
        };
        let right = event.button == MouseButton::Right;
        let pointing = &mut self.pointing;
        if pointing.of == Some(ShotOf::Moves) {
            // One point: pressed, it is followed at once.
            if right {
                return;
            }
            pointing.on = vec![point];
            self.follow(cx);
            return;
        }
        if right || event.modifiers.alt {
            pointing.off.push(point);
        } else {
            pointing.on.push(point);
        }
        cx.notify();
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let pointing = &mut self.pointing;
        if pointing.on.is_empty() {
            return;
        }
        // Points are only pressed on a picture asked for one way or the other.
        let Some(of) = pointing.of else {
            return;
        };
        pointing.stage = PointStage::Following;
        pointing.says.clear();
        pointing.failed = false;
        let points = |points: &[(f64, f64)]| -> Vec<[f64; 2]> {
            points.iter().map(|(x, y)| [*x, *y]).collect()
        };
        let command = Command::ShotFollow(ShotFollow {
            clip: pointing.clip.clone(),
            of,
            points: points(&pointing.on),
            outside: points(&pointing.off),
        });
        self.session.update(cx, |session, _| session.send(command));
        cx.notify();
    }

    /// Reading `clip` from footage, for its way (`moves`) or its matte
    /// (`matte`): its first frame drawn, pointed on, and followed.
    pub(super) fn pointing(&self, clip: &str, of: ShotOf, cx: &mut Context<Self>) -> gpui::Div {
        let moves = of == ShotOf::Moves;
        let mine = self.pointing.clip == clip && self.pointing.of == Some(of);
        let stage = if mine {
            self.pointing.stage
        } else {
            PointStage::Idle
        };
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(section("From footage"));
        if mine && !self.pointing.says.is_empty() {
            column = column.child(note(
                self.pointing.says.clone(),
                if self.pointing.failed {
                    theme::neg()
                } else {
                    theme::ink2()
                },
            ));
        }
        match stage {
            PointStage::Pointing | PointStage::Following => {
                let following = stage == PointStage::Following;
                let image = self
                    .pointing
                    .picture
                    .as_deref()
                    .and_then(|path| self.still(path));
                column = column.child(self.point_picture(image, following, cx));
                if following {
                    column = column.child(note(
                        if moves {
                            "Following it through the clip…"
                        } else {
                            "Cutting it out through the clip…"
                        }
                        .into(),
                        theme::ink3(),
                    ));
                } else if moves {
                    column = column.child(note(
                        "Press on what the clip should follow: a corner or a mark, or a thing that stands out from what is behind it. The clip itself is not in this picture.".into(),
                        theme::ink3(),
                    ));
                } else {
                    column = column
                        .child(note(
                            "Press on what to cut out, in as many places as tell it from the rest; hold Alt, or use the other button, on what is taken for it and is not it.".into(),
                            theme::ink3(),
                        ))
                        .child(
                            div().flex().child(
                                ui::button("graph-cut-out", "Cut out")
                                    .ghost()
                                    .disabled(self.pointing.on.is_empty())
                                    .tooltip("Mask the clip to what the points are on, through the whole clip: its matte is then that thing in white")
                                    .on_click(cx.listener(|this, _, _, cx| this.follow(cx))),
                            ),
                        );
                }
            }
            PointStage::Drawing => {
                column = column.child(note(
                    "Drawing the clip's first frame…".into(),
                    theme::ink3(),
                ));
            }
            PointStage::Idle | PointStage::Said => {
                let clip = clip.to_owned();
                column = column.child(
                    div().flex().child(
                        ui::button(
                            if moves {
                                "graph-follow-picture"
                            } else {
                                "graph-cut-picture"
                            },
                            if moves {
                                "Follow the picture"
                            } else {
                                "Cut out of the picture"
                            },
                        )
                        .ghost()
                        .tooltip(if moves {
                            "Point at something in the footage under this clip, and have the clip follow it for as long as it runs: its x and y, keyed"
                        } else {
                            "Point at a thing this clip shows, and mask the clip to it as it moves and changes shape: the matte is then that thing"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_picture(clip.clone(), of, cx)
                        })),
                    ),
                );
            }
        }
        column
    }

    /// The clip's first frame to point on: a press marks what it should
    /// follow, or a point on the thing (a ring); Alt or the other button one
    /// off it (a cross).
    fn point_picture(
        &self,
        image: Option<Arc<RenderImage>>,
        following: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = PICTURE_MOST.0;
        let height = image
            .as_ref()
            .map(|image| crate::artwork::image_size(image))
            .filter(|size| size.0 > 0)
            .map_or(width * 9.0 / 16.0, |(w, h)| width * h as f32 / w as f32);
        let bounds = self.point_bounds.clone();
        let (on, off) = (self.pointing.on.clone(), self.pointing.off.clone());
        let path_ink = theme::playhead();
        let off_ink = theme::neg();
        div()
            .id("graph-point-picture")
            .w(px(width))
            .h(px(height))
            .rounded(px(radius::CONTROL))
            .overflow_hidden()
            .bg(theme::well())
            .when(!following, |picture| picture.cursor(CursorStyle::Crosshair))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.point(event, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.point(event, cx)
                }),
            )
            .child(
                canvas(
                    move |area, _, _| bounds.set(area),
                    move |area, _, window, _| {
                        let Some(image) = image.clone() else {
                            return;
                        };
                        let size = crate::artwork::image_size(&image);
                        let room = [
                            f32::from(area.origin.x),
                            f32::from(area.origin.y),
                            f32::from(area.size.width),
                            f32::from(area.size.height),
                        ];
                        let [left, top, w, h] = fitted_in(room, size);
                        let placed =
                            Bounds::new(point(px(left), px(top)), gpui::size(px(w), px(h)));
                        if let Err(error) =
                            window.paint_image(placed, gpui::Corners::default(), image, 0, false)
                        {
                            log::warn!("the frame to point on did not draw: {error}");
                        }
                        if following {
                            window.paint_quad(fill(placed, theme::pointing::following_dim()));
                        }
                        let place = |(x, y): (f64, f64)| {
                            point(px(left + x as f32 * w), px(top + y as f32 * h))
                        };
                        // A point on the thing as a ring, one off it as a cross:
                        // told by shape as well as by hue.
                        for at in &on {
                            let at = place(*at);
                            paint_ring_line(window, at, 5.0, theme::pointing::mark_shadow(), 4.0);
                            paint_ring_line(window, at, 5.0, path_ink, 2.0);
                        }
                        for at in &off {
                            let at = place(*at);
                            paint_cross(window, at, theme::pointing::mark_shadow(), 4.0);
                            paint_cross(window, at, off_ink, 2.0);
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}
