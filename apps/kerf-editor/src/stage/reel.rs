//! Watching what a proposal changed (the change reel, `Session::reel`):
//! on the After side the program as it was at the segment's start sweeps
//! over the picture left of a moving line (Before|After), tagged at the
//! picture's foot; a side changed wipes away the frame that was showing as
//! the next comes; the reel's strip floats over the picture's top.

use super::StageView;
use crate::model;
use crate::motion;
// The Before|After line's sweep, there and back, and how far across the
// picture it goes; the glow either side of it, the tags at the picture's
// foot, and the change reel's strip over the picture's top, at most so
// wide.
use crate::theme::layout::{COMPARE_TAG_IN, REEL_STRIP_IN, REEL_STRIP_MOST};
use crate::theme::motion::{COMPARE_FROM, COMPARE_GLOW, COMPARE_SWEEP, COMPARE_TO};
use crate::theme::{self, motion as scene, over_picture, text};
use gpui::{
    div, fill, point, prelude::*, px, size, App, Bounds, ContentMask, Context, Corners, Div,
    Pixels, RenderImage, Rgba, Window,
};
use kerf_bridge::event::EffectPreview;
use kerf_bridge::{Command, EffectPreviewsRequest, PreviewEffect};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How far into a segment its Before still is taken, and how wide.
const COMPARE_IN: f64 = 0.04;
const COMPARE_WIDE: u32 = 960;

/// Where the Before|After line is across the picture (a share of its
/// width), `elapsed` into the sweep: from `COMPARE_FROM` to `COMPARE_TO`
/// and back, evenly, once each `COMPARE_SWEEP`; held at `COMPARE_FROM`
/// under Reduce Motion (`still`).
pub fn compare_at(elapsed: Duration, still: bool) -> f32 {
    if still {
        return COMPARE_FROM;
    }
    let period = COMPARE_SWEEP.as_secs_f32();
    let phase = (elapsed.as_secs_f32() / period).fract();
    let there = 1.0 - (phase * 2.0 - 1.0).abs();
    COMPARE_FROM + (COMPARE_TO - COMPARE_FROM) * there
}

/// The bytes of standard base64 `text` (padding optional, no line
/// breaks); None when it does not read.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let text = text.trim_end_matches('=').as_bytes();
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.chunks(4) {
        let mut bits = 0u32;
        for (at, byte) in chunk.iter().enumerate() {
            bits |= value(*byte)? << (18 - 6 * at);
        }
        out.push((bits >> 16) as u8);
        if chunk.len() > 2 {
            out.push((bits >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(bits as u8);
        }
    }
    Some(out)
}

impl StageView {
    /// A picture let go of: the atlas drops it as the stage next draws.
    fn retire(&self, image: Arc<RenderImage>) {
        self.retired.borrow_mut().push(image);
    }

    /// Follows the change reel (`Session::reel`): watching the After side,
    /// the program as it was at the segment's start is asked for, to
    /// sweep over the picture (Before|After); a change of side wipes the
    /// frame that was showing away once the next comes.
    pub(super) fn follow_reel(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let frame = session
            .picture
            .as_ref()
            .map(|picture| picture.image.clone());
        let Some(reel) = session.reel().cloned() else {
            self.reel_side = None;
            self.wipe_armed = None;
            self.compare_key = None;
            if let Some((still, _)) = self.compare.take() {
                self.retire(still);
            }
            if frame.is_some() {
                self.last_frame = frame;
            }
            return;
        };
        // What was last shown wipes away (the picture may have gone as
        // the side changed: the last frame seen, not none).
        if self.reel_side.is_some_and(|side| side != reel.side) {
            self.wipe_armed = self.last_frame.clone().or_else(|| frame.clone());
        }
        self.reel_side = Some(reel.side);
        // A new frame after the side changed starts the wipe.
        let new_frame = match (&frame, &self.last_frame) {
            (Some(now), Some(was)) => !Arc::ptr_eq(now, was),
            (Some(_), None) => true,
            _ => false,
        };
        if new_frame && !motion::reduced() {
            if let Some(from) = self.wipe_armed.take() {
                // It is done once it has crossed the picture (`end_wipe`).
                if let Some((old, _)) = self.wipe.replace((from, Instant::now())) {
                    self.retire(old);
                }
            }
        }
        if frame.is_some() {
            self.last_frame = frame;
        }
        if reel.side != crate::session::ReelSide::After {
            self.compare_key = None;
            if let Some((still, _)) = self.compare.take() {
                self.retire(still);
            }
            return;
        }
        let key = format!("{}/{}", reel.proposal, reel.index);
        if self.compare_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.compare_key = Some(key.clone());
        if let Some((still, _)) = self.compare.take() {
            self.retire(still);
        }
        // The program's picture clip there; a part that is new has nothing
        // to compare with.
        let Some(segment) = reel.segments.get(reel.index) else {
            return;
        };
        let at = segment.before.max(0.0) + COMPARE_IN;
        let session = self.session.read(cx);
        let clip = session
            .timeline()
            .tracks
            .iter()
            .filter(|track| track.kind == model::TrackKind::Video)
            .flat_map(|track| track.clips.iter())
            .find(|clip| clip.timeline_start <= at && at < clip.end())
            .map(|clip| clip.id.clone());
        if let Some(clip) = clip {
            self.session.update(cx, |session, _| {
                session.send(Command::EffectPreviews(EffectPreviewsRequest {
                    clip,
                    at,
                    effects: vec![PreviewEffect::Params {
                        fx: "color.basic".into(),
                        params: serde_json::Map::new(),
                        key: Some(format!("reel-before:{key}")),
                    }],
                    width: COMPARE_WIDE,
                    key: Some("reel".into()),
                }));
            });
        }
    }

    /// Ends the wipe once it has crossed the picture (`STAGE`), as the
    /// stage draws (its frames go on until then, [`Self::wipe_now`]): the
    /// frame it wiped away leaves the atlas, unless it is the one showing.
    pub(super) fn end_wipe(&mut self, cx: &App) {
        let crossed = self
            .wipe
            .as_ref()
            .is_some_and(|(_, since)| *since + scene::STAGE <= Instant::now());
        if !crossed {
            return;
        }
        if let Some((from, _)) = self.wipe.take() {
            let showing = self
                .session
                .read(cx)
                .picture
                .as_ref()
                .is_some_and(|picture| Arc::ptr_eq(&picture.image, &from));
            if !showing {
                self.retire(from);
            }
        }
    }

    /// An `effect_preview` that is the Before still asked for: it sweeps
    /// over the picture from now.
    pub(super) fn hear_compare(&mut self, preview: &EffectPreview, cx: &mut Context<Self>) {
        let Some(key) = self.compare_key.as_deref() else {
            return;
        };
        if preview.key.as_deref() != Some(format!("reel-before:{key}").as_str()) {
            return;
        }
        let Some(rgba) = base64_decode(&preview.rgba) else {
            return;
        };
        if let Some(still) = crate::session::render_image(&rgba, preview.width, preview.height) {
            if let Some((old, _)) = self.compare.replace((still, Instant::now())) {
                self.retire(old);
            }
            cx.notify();
        }
    }

    /// The Before still sweeping now, and where its line is across the
    /// picture; frames are asked for while it moves.
    pub(super) fn compare_now(&self, window: &Window) -> Option<(Arc<RenderImage>, f32)> {
        let still_motion = motion::reduced();
        self.compare.as_ref().map(|(still, since)| {
            if !still_motion {
                window.request_animation_frame();
            }
            (still.clone(), compare_at(since.elapsed(), still_motion))
        })
    }

    /// The frame wiping away now, and how far across it has gone; frames
    /// are asked for until it has crossed the picture, when the stage's
    /// next draw ends it ([`Self::end_wipe`]).
    pub(super) fn wipe_now(&self, window: &Window) -> Option<(Arc<RenderImage>, f32)> {
        let (from, since) = self.wipe.as_ref()?;
        window.request_animation_frame();
        let t = since.elapsed().as_secs_f32() / scene::STAGE.as_secs_f32();
        Some((from.clone(), motion::ease(t.clamp(0.0, 1.0))))
    }

    /// "Before" and "After" at the foot of `picture` (in the stage's own
    /// points), while the still sweeps.
    pub(super) fn compare_tags(&self, picture: Bounds<Pixels>) -> [Div; 2] {
        let left = picture.origin.x;
        let top = picture.origin.y;
        let tag = |words: &'static str, ground: Rgba| {
            theme::type_style(div(), text::META)
                .h(px(22.0))
                .px(px(theme::space::S8))
                .flex()
                .items_center()
                .rounded(px(theme::radius::CONTROL))
                .bg(ground)
                .text_color(over_picture::tag_ink())
                .child(words)
        };
        let foot = top + picture.size.height - px(COMPARE_TAG_IN + 22.0);
        [
            div()
                .absolute()
                .left(left + px(COMPARE_TAG_IN))
                .top(foot)
                .child(tag("Before", over_picture::tag())),
            div()
                .absolute()
                .right(px(f32::from(
                    self.area.get().map_or(Pixels::ZERO, |area| area.size.width)
                        - (left + picture.size.width),
                ) + COMPARE_TAG_IN))
                .top(foot)
                .child(tag("After", theme::accent())),
        ]
    }

    /// The change reel's strip over the top of `picture`, at its corner.
    pub(super) fn reel_strip_over(&self, picture: Bounds<Pixels>, cx: &App) -> Option<Div> {
        let strip = crate::chat::reel_strip(&self.session, cx)?;
        let left = picture.origin.x;
        let top = picture.origin.y;
        let room = self
            .area
            .get()
            .map_or(0.0, |area| f32::from(area.size.width));
        let wide = REEL_STRIP_MOST.min(room - 2.0 * REEL_STRIP_IN).max(0.0);
        let shown = f32::from(picture.size.width);
        let at = if shown >= wide + 2.0 * REEL_STRIP_IN {
            f32::from(left) + REEL_STRIP_IN
        } else {
            f32::from(left) + shown / 2.0 - wide / 2.0
        };
        let at = at.clamp(
            REEL_STRIP_IN,
            (room - REEL_STRIP_IN - wide).max(REEL_STRIP_IN),
        );
        Some(
            div()
                .absolute()
                .left(px(at))
                .top(top + px(REEL_STRIP_IN))
                .w(px(wide))
                .child(strip),
        )
    }
}

/// The change reel's Before still over the picture `card`, left of the
/// line `at` of its width, with a soft glow either side of the line and
/// the line itself in white.
pub(super) fn paint_compare(
    window: &mut Window,
    card: Bounds<Pixels>,
    radius: Pixels,
    still: Arc<RenderImage>,
    at: f32,
) {
    let edge = card.left() + card.size.width * at;
    let before = Bounds::from_corners(card.origin, point(edge, card.bottom()));
    let placed = crate::artwork::fitted(card, crate::artwork::image_size(&still), false);
    window.with_content_mask(Some(ContentMask { bounds: before }), |window| {
        if let Err(error) = window.paint_image(placed, Corners::all(radius), still, 0, false) {
            log::warn!("the change reel's Before still did not draw: {error}");
        }
    });
    let glow = px(COMPARE_GLOW);
    let clear = theme::with_alpha(over_picture::line(), 0.0);
    let lit = theme::with_alpha(over_picture::line(), 0.32);
    window.paint_quad(fill(
        Bounds::new(point(edge - glow, card.top()), size(glow, card.size.height)),
        gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(clear, 0.0),
            gpui::linear_color_stop(lit, 1.0),
        ),
    ));
    window.paint_quad(fill(
        Bounds::new(point(edge, card.top()), size(glow, card.size.height)),
        gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(lit, 0.0),
            gpui::linear_color_stop(clear, 1.0),
        ),
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(edge - px(1.0), card.top()),
            size(px(2.0), card.size.height),
        ),
        over_picture::line(),
    ));
}
