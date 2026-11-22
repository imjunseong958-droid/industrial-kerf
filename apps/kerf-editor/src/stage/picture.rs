//! Where the picture sits on the stage and how it is painted: placed in the
//! frame of its device (a laptop for a wide picture, a phone for the rest),
//! the frame drawn whole at its own shape or at the viewer's zoom, the side
//! of a change reel wiping away and the Before still sweeping over it, the
//! phone's island, the safe zones, and over it all the chosen clip's
//! overlays, what a mask tool draws and the light round the edge.

use super::activity::paint_edge_light;
use super::editing::listen;
use super::overlay::Pt;
use super::paint::{paint_drawn, paint_overlays};
use super::reel::paint_compare;
use super::shown::{Drawn, Shown};
use super::StageView;
use crate::model::TimelineShow;
use crate::session::Session;
use crate::theme::{self, over_picture};
use gpui::{
    canvas, fill, point, prelude::*, px, quad, size, App, BorderStyle, Bounds, ContentMask,
    Context, Corners, CursorStyle, Pixels, Point, RenderImage, Rgba, WeakEntity, Window,
};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// Action safe: a line this share of the picture in from each edge.
const ACTION_SAFE: f32 = 0.035;
/// Title safe, shown by ticks at its corners: this share in.
pub(super) const TITLE_SAFE: f32 = 0.05;
/// Points of each title-safe tick.
const SAFE_TICK: f32 = 16.0;
/// Points across the cross at the picture's centre.
const SAFE_CROSS: f32 = 12.0;

/// Width over height of what `session`'s picture shows: the sequence's
/// format; a first cut proposed on an empty program brings its own; with
/// neither, the frame's, else 16:9. The workspace splits the panel by it.
pub fn shown_aspect(session: &Session) -> f32 {
    let timeline = session.timeline();
    let proposed = session
        .state
        .snapshot
        .proposal_timeline
        .as_ref()
        .filter(|proposal| {
            !timeline.has_clips()
                && proposal.width.unwrap_or(0) > 0
                && proposal.height.unwrap_or(0) > 0
        });
    match (
        proposed,
        session.picture.as_ref(),
        timeline.width.unwrap_or(0) > 0 && timeline.height.unwrap_or(0) > 0,
    ) {
        (Some(proposal), _, _) => proposal.aspect(),
        (None, _, true) => timeline.aspect(),
        (None, Some(picture), false) if picture.height > 0 => {
            picture.width as f32 / picture.height as f32
        }
        _ => 16.0 / 9.0,
    }
}

/// The frame's own pixels, for the zoom: the sequence's, else the picture's.
pub(super) fn frame_size(session: &Session) -> (u32, u32) {
    let timeline = session.timeline();
    if timeline.width.unwrap_or(0) > 0 && timeline.height.unwrap_or(0) > 0 {
        (timeline.width.unwrap_or(0), timeline.height.unwrap_or(0))
    } else {
        session
            .picture
            .as_ref()
            .map_or((0, 0), |picture| (picture.width, picture.height))
    }
}

// ---------------------------------------------------------------- placement

/// Where the picture and its device sit within the stage.
#[derive(Clone, Copy, Debug)]
pub(super) struct Placement {
    pub(super) device: Bounds<Pixels>,
    pub(super) picture: Bounds<Pixels>,
    pub(super) device_radius: Pixels,
    pub(super) picture_radius: Pixels,
    pub(super) phone: bool,
}

/// Whether a picture of `aspect` (width over height) is wide: framed as a
/// laptop, its caption one line. Below it (vertical, square and near
/// square) it is framed as a phone and its caption wraps over balanced
/// lines.
pub fn wide(aspect: f32) -> bool {
    aspect >= WIDE_FROM
}

/// Where a picture's aspect starts to be wide.
pub const WIDE_FROM: f32 = 1.2;

pub(super) fn place(area: Bounds<Pixels>, aspect: f32) -> Placement {
    let phone = !wide(aspect);
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

/// A frame `width` × `height` drawn whole within `bounds`, centred: never
/// stretched to another shape.
pub(super) fn contain(bounds: Bounds<Pixels>, width: u32, height: u32) -> Bounds<Pixels> {
    if width == 0 || height == 0 {
        return bounds;
    }
    let scale = (f32::from(bounds.size.width) / width as f32)
        .min(f32::from(bounds.size.height) / height as f32);
    let shown = size(px(width as f32 * scale), px(height as f32 * scale));
    Bounds::new(
        point(
            bounds.origin.x + (bounds.size.width - shown.width) / 2.0,
            bounds.origin.y + (bounds.size.height - shown.height) / 2.0,
        ),
        shown,
    )
}

/// The zooms the viewer offers: Fit (0), then shares of the frame's own pixels.
pub const ZOOMS: [f32; 4] = [0.0, 0.5, 1.0, 2.0];

/// A zoom as its menu names it: "Fit", "50%".
pub fn zoom_words(zoom: f32) -> String {
    if zoom <= 0.0 {
        "Fit".to_owned()
    } else {
        format!("{}%", (zoom * 100.0).round() as i64)
    }
}

/// Where a frame `frame` pixels big is drawn over the picture `card` at
/// `zoom`: the card itself at Fit (or with no frame size); else that share
/// of its pixels, one frame pixel a device pixel at 100% (`scale` device
/// pixels to the point), centred on the card.
pub fn zoomed(card: Bounds<Pixels>, frame: (u32, u32), zoom: f32, scale: f32) -> Bounds<Pixels> {
    let (width, height) = frame;
    if zoom <= 0.0 || width == 0 || height == 0 {
        return card;
    }
    let scale = scale.max(1.0);
    let shown = size(
        px(width as f32 * zoom / scale),
        px(height as f32 * zoom / scale),
    );
    Bounds::new(
        point(
            card.center().x - shown.width / 2.0,
            card.center().y - shown.height / 2.0,
        ),
        shown,
    )
}

pub(super) fn to_pt(at: Point<Pixels>) -> Pt {
    (f64::from(f32::from(at.x)), f64::from(f32::from(at.y)))
}

pub(super) fn to_point((x, y): Pt) -> Point<Pixels> {
    point(px(x as f32), px(y as f32))
}

pub(super) fn apart(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

// ------------------------------------------------------------------ canvas

/// What the picture's canvas paints this frame, worked out as the stage
/// draws.
pub(super) struct Painting {
    /// The shape the picture is placed at.
    pub(super) aspect: f32,
    /// The frame's own pixels, and the viewer's zoom on them.
    pub(super) frame_size: (u32, u32),
    pub(super) zoom: f32,
    /// The frame shown, and its pixels.
    pub(super) frame: Option<(Arc<RenderImage>, u32, u32)>,
    /// A poster stands in whole over it: the frame is not drawn.
    pub(super) covering: bool,
    /// The frame wiping away as a reel's side changed, and how far.
    pub(super) wipe: Option<(Arc<RenderImage>, f32)>,
    /// The reel's Before still, and where its line is across the picture.
    pub(super) compare: Option<(Arc<RenderImage>, f32)>,
    pub(super) safe_zones: bool,
    /// The chosen clip's overlays, and what is chosen of them.
    pub(super) shown: Option<Rc<Shown>>,
    pub(super) chosen_mask: Option<String>,
    pub(super) chosen_point: Option<usize>,
    /// What a mask tool has drawn so far.
    pub(super) in_hand: Drawn,
    /// How clear the light round the picture is, its hue, and since when
    /// it runs.
    pub(super) edge: f32,
    pub(super) hue_color: Rgba,
    pub(super) since: Instant,
    /// The cursor while a drag holds the pointer.
    pub(super) dragging: Option<CursorStyle>,
}

impl StageView {
    /// The picture as last placed, in the window's coordinates: where the
    /// workspace's export flight leaves from (`Workspace::fly_export_home`).
    pub fn picture_bounds(&self) -> Option<Bounds<Pixels>> {
        self.picture.get()
    }

    /// The canvas the picture is painted on, filling the stage. As it is
    /// laid out it keeps where the stage, the picture and the frame are,
    /// and what overlays it shows, for a press and the pointer to hit; the
    /// pictures let go of since it last drew leave the atlas then, as
    /// nothing drawn from now on shows them.
    pub(super) fn picture_canvas(
        &self,
        painting: Painting,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let area_cell = self.area.clone();
        let picture_cell = self.picture.clone();
        let canvas_cell = self.canvas.clone();
        let shown_cell = self.shown.clone();
        let retired = self.retired.clone();
        let (aspect, frame_size, zoom) = (painting.aspect, painting.frame_size, painting.zoom);
        let shown = painting.shown.clone();
        let this = cx.entity().downgrade();
        canvas(
            move |bounds, window, cx| {
                for image in retired.take() {
                    cx.drop_image(image, Some(window));
                }
                if area_cell.get() != Some(bounds) {
                    area_cell.set(Some(bounds));
                    window.refresh();
                }
                let placement = place(bounds, aspect);
                picture_cell.set(Some(placement.picture));
                let drawn = zoomed(placement.picture, frame_size, zoom, window.scale_factor());
                canvas_cell.set(Some(drawn));
                *shown_cell.borrow_mut() = shown;
                (placement, drawn)
            },
            move |bounds, (placement, drawn): (Placement, Bounds<Pixels>), window, cx| {
                paint_picture(bounds, placement, drawn, painting, &this, window, cx);
            },
        )
        .absolute()
        .size_full()
    }
}

/// Paints the device, the frame in it (or nothing under a poster standing
/// in whole), the reel's wipe and Before still, the phone's island, the
/// safe zones, the chosen clip's overlays and what a mask tool draws, and
/// the light round the edge; a drag in hand holds the pointer.
fn paint_picture(
    bounds: Bounds<Pixels>,
    placement: Placement,
    drawn: Bounds<Pixels>,
    painting: Painting,
    this: &WeakEntity<StageView>,
    window: &mut Window,
    cx: &mut App,
) {
    let Painting {
        zoom,
        frame,
        covering,
        wipe,
        compare,
        safe_zones,
        shown,
        chosen_mask,
        chosen_point,
        in_hand,
        edge,
        hue_color,
        since,
        dragging,
        ..
    } = painting;
    let framed = frame.is_some();
    // The device: a bezel, a hairline round it.
    window.paint_quad(quad(
        placement.device,
        Corners::all(placement.device_radius),
        over_picture::bezel(),
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
    // A poster standing in: under the cut Kerf builds it covers
    // the frame; with none, it is the picture until one comes.
    if let (Some((image, width, height)), false) = (frame, covering) {
        // Whole, at its own shape; zoomed, cut to the picture.
        if zoom > 0.0 {
            window.with_content_mask(
                Some(ContentMask {
                    bounds: placement.picture,
                }),
                |window| {
                    if let Err(error) = window.paint_image(
                        contain(drawn, width, height),
                        Corners::default(),
                        image,
                        0,
                        false,
                    ) {
                        log::warn!("the viewer's picture did not draw: {error}");
                    }
                },
            );
        } else {
            let card = contain(placement.picture, width, height);
            if let Err(error) = window.paint_image(
                card,
                Corners::all(placement.picture_radius),
                image,
                0,
                false,
            ) {
                log::warn!("the viewer's picture did not draw: {error}");
            }
            // What was showing as the side changed, wiping away
            // to the right as what shows now comes in.
            if let Some((from, t)) = &wipe {
                let edge = card.left() + card.size.width * *t;
                let kept = Bounds::from_corners(point(edge, card.top()), card.bottom_right());
                window.with_content_mask(Some(ContentMask { bounds: kept }), |window| {
                    if let Err(error) = window.paint_image(
                        card,
                        Corners::all(placement.picture_radius),
                        from.clone(),
                        0,
                        false,
                    ) {
                        log::warn!("the picture wiped away did not draw: {error}");
                    }
                });
                window.paint_quad(fill(
                    Bounds::new(
                        point(edge - px(1.0), card.top()),
                        size(px(2.0), card.size.height),
                    ),
                    over_picture::line(),
                ));
            }
            // What it was, left of the moving line; what it is, right of it.
            if let Some((still, at)) = &compare {
                paint_compare(window, card, placement.picture_radius, still.clone(), *at);
            }
        }
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
            over_picture::island(),
            px(0.0),
            theme::line(),
            BorderStyle::Solid,
        ));
    }
    // Over a picture, not over the words standing in for one.
    if safe_zones && framed {
        window.with_content_mask(
            Some(ContentMask {
                bounds: placement.picture,
            }),
            |window| paint_safe_zones(window, drawn),
        );
    }
    if let Some(shown) = &shown {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            paint_overlays(
                window,
                cx,
                drawn,
                shown,
                chosen_mask.as_deref(),
                chosen_point,
            );
            paint_drawn(window, drawn, shown, &in_hand);
        });
    }
    if edge > 0.001 {
        paint_edge_light(window, placement, hue_color, edge, since);
    }
    if let Some(cursor) = dragging {
        window.set_window_cursor_style(cursor);
        listen(this, window);
    }
}

/// The broadcast safe zones: action safe, a 1 pt line just inside a rect
/// 3.5% in from every edge; title safe, 5% in, shown by 16 pt ticks at its
/// corners; and a 12 pt cross at the centre.
fn paint_safe_zones(window: &mut Window, picture: Bounds<Pixels>) {
    let (x, y) = (f32::from(picture.origin.x), f32::from(picture.origin.y));
    let (w, h) = (
        f32::from(picture.size.width),
        f32::from(picture.size.height),
    );
    if w < 2.0 * SAFE_TICK || h < 2.0 * SAFE_TICK {
        return;
    }
    let rect = |left: f32, top: f32, across: f32, down: f32| {
        Bounds::new(
            point(px(left.round()), px(top.round())),
            size(px(across.round().max(1.0)), px(down.round().max(1.0))),
        )
    };
    let (ax, ay) = (x + w * ACTION_SAFE, y + h * ACTION_SAFE);
    let (aw, ah) = (w * (1.0 - 2.0 * ACTION_SAFE), h * (1.0 - 2.0 * ACTION_SAFE));
    for line in [
        rect(ax, ay, aw, 1.0),
        rect(ax, ay + ah - 1.0, aw, 1.0),
        rect(ax, ay + 1.0, 1.0, ah - 2.0),
        rect(ax + aw - 1.0, ay + 1.0, 1.0, ah - 2.0),
    ] {
        window.paint_quad(fill(line, theme::lineh()));
    }
    let (tx, ty) = (x + w * TITLE_SAFE, y + h * TITLE_SAFE);
    let (right, bottom) = (x + w * (1.0 - TITLE_SAFE), y + h * (1.0 - TITLE_SAFE));
    let k = SAFE_TICK;
    let tick = over_picture::tick();
    for line in [
        rect(tx, ty, k, 1.0),
        rect(tx, ty + 1.0, 1.0, k - 1.0),
        rect(right - k, ty, k, 1.0),
        rect(right - 1.0, ty + 1.0, 1.0, k - 1.0),
        rect(tx, bottom - 1.0, k, 1.0),
        rect(tx, bottom - k, 1.0, k - 1.0),
        rect(right - k, bottom - 1.0, k, 1.0),
        rect(right - 1.0, bottom - k, 1.0, k - 1.0),
    ] {
        window.paint_quad(fill(line, tick));
    }
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let half = SAFE_CROSS / 2.0;
    window.paint_quad(fill(rect(cx - half, cy, SAFE_CROSS, 1.0), tick));
    window.paint_quad(fill(rect(cx, cy - half, 1.0, SAFE_CROSS), tick));
}
