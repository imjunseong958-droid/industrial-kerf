//! How one clip is painted on its lane ([`paint_clip`]): its ground under
//! its kind's edge colour, what it holds of its media (a filmstrip, a
//! waveform) or a signal's curve, its making's stripes and pill, its name
//! and what goes with it, a sound's rubber band, its outline, the edge
//! under the pointer or being trimmed, and Kerf's ring while it looks.

use super::activity::{paint_activity, stripes, Activity};
use super::canvas::{clip_box, masked, shape, Fonts, Frame, Pen};
use super::drag::Zone;
use super::gain::{band_line, gain_drawn, paint_band};
use super::hover::Hover;
use super::lane_ink;
use super::lanes::{ClipBox, ClipKind, Row};
use super::signal::{paint_drawn, paint_signal};
use crate::model::{self, ClipShow};
use crate::theme::{self, radius, text};
use gpui::{fill, px, quad, App, BorderStyle, Bounds, Corners, Pixels, Window};

/// Points of the colour edge along a clip's top.
const CLIP_EDGE: f32 = 2.0;
/// Points of the dark bar a name stands on over a filmstrip or a waveform.
const NAME_BAR: f32 = 16.0;

/// What a clip shows of its media under its name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Art {
    None,
    Strip,
    Wave,
}

/// One clip as it is painted: where its box stands on the canvas, and how
/// it shows (quieted, chosen, pointed at, its media's making).
struct ClipPaint<'a> {
    frame: &'a Frame,
    pen: Pen,
    row: &'a Row,
    clip: &'a ClipBox,
    /// Its box, kept within a little of the canvas.
    left: f32,
    top: f32,
    across: f32,
    tall: f32,
    /// Its ends as drawn, past the canvas or not.
    x0: f32,
    x1: f32,
    body: Bounds<Pixels>,
    /// All of it but the edge along its top; all of it with no file.
    under_edge: Bounds<Pixels>,
    quiet: bool,
    selected: bool,
    hover: Option<&'a Hover>,
    activity: Option<&'a Activity>,
    /// Its media is not made, or its file is gone: a hole.
    absent: bool,
}

// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_clip(
    frame: &Frame,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    clip: &ClipBox,
    (start, duration): (f64, f64),
    trimming: Option<Zone>,
    head_shift: f64,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(paint) = ClipPaint::new(frame, pen, width, row, y, clip, (start, duration)) else {
        return;
    };
    paint.ground(window);
    let art = paint.media(duration, head_shift, window, cx);
    paint.making(window);
    paint.signal(window);
    paint_name(
        frame,
        pen,
        width,
        row,
        clip,
        (paint.x0, paint.x1),
        (paint.top, paint.tall),
        art,
        paint.quiet,
        duration,
        fonts,
        window,
        cx,
    );
    if let Some(activity) = paint.activity {
        paint_activity(
            activity,
            pen,
            width,
            (paint.x0, paint.x1),
            (paint.top, paint.tall),
            fonts,
            window,
            cx,
        );
    }
    paint.band(window);
    paint.outline(window);
    paint.edge_bar(trimming, window);
    paint.looked_at(window);
}

impl<'a> ClipPaint<'a> {
    /// Clip `clip` on `row` drawn at `y`, from `start` for `duration`
    /// seconds; None when it is off the canvas.
    fn new(
        frame: &'a Frame,
        pen: Pen,
        width: f32,
        row: &'a Row,
        y: f32,
        clip: &'a ClipBox,
        (start, duration): (f64, f64),
    ) -> Option<Self> {
        let view = &frame.view;
        let (left, top, across, tall) = clip_box(view, width, row, y, start, start + duration)?;
        let x0 = view.x(start);
        let x1 = view.x(start + duration).max(x0 + 2.0);
        let body = pen.rect(left, top, across, tall);
        let quiet = !clip.enabled || row.quiet();
        let selected = !row.previewed && frame.selection.contains(&clip.id);
        let hover = frame
            .hover
            .as_ref()
            .filter(|hover| hover.clip == clip.id && !row.previewed);
        let activity = frame.activity(clip);
        let absent = activity.is_some_and(|activity| activity.state.absent());
        let under_edge = if absent {
            body
        } else {
            pen.rect(left, top + CLIP_EDGE, across, tall - CLIP_EDGE)
        };
        Some(Self {
            frame,
            pen,
            row,
            clip,
            left,
            top,
            across,
            tall,
            x0,
            x1,
            body,
            under_edge,
            quiet,
            selected,
            hover,
            activity,
            absent,
        })
    }

    /// The edge colour fills it, and its ground covers all but its top;
    /// with no file, only a well's ground, all of it.
    fn ground(&self, window: &mut Window) {
        let clip = self.clip;
        let ground = if self.absent {
            theme::well()
        } else if clip.changed {
            theme::mix(theme::card(), theme::pos(), 0.16)
        } else {
            theme::card()
        };
        let (ground, edge) = if self.quiet {
            (
                theme::mix(theme::well(), ground, 0.55),
                theme::mix(theme::well(), clip.edge, 0.35),
            )
        } else {
            (ground, clip.edge)
        };
        let corner = px(radius::SMALL);
        if !self.absent {
            window.paint_quad(quad(
                self.body,
                corner,
                edge,
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
        }
        let bottom = Corners {
            top_left: px(if self.absent { radius::SMALL } else { 0.0 }),
            top_right: px(if self.absent { radius::SMALL } else { 0.0 }),
            bottom_right: corner,
            bottom_left: corner,
        };
        window.paint_quad(quad(
            self.under_edge,
            bottom,
            ground,
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
    }

    /// What it holds: frames along a picture, a sound's waveform, covered
    /// while it is quieted. A leading trim being dragged moves the in point
    /// with it (`head_shift`). Gives what it painted.
    fn media(&self, duration: f64, head_shift: f64, window: &mut Window, cx: &mut App) -> Art {
        let (clip, row, pen) = (self.clip, self.row, self.pen);
        let (x0, x1, top, tall) = (self.x0, self.x1, self.top, self.tall);
        let mut art = Art::None;
        if let (false, false, Some(source)) = (self.absent, clip.card(), clip.source.as_deref()) {
            let source_at = |offset: f64| clip.clip.source_at(offset + head_shift).unwrap_or(0.0);
            let inside = pen.rect(x0, top + CLIP_EDGE, x1 - x0, tall - CLIP_EDGE);
            if row.audio() {
                let peaks = self.frame.session.read(cx).artwork.peaks(source);
                if let Some(peaks) = peaks.filter(|peaks| !peaks.is_empty()) {
                    let wave = pen.rect(x0, top + CLIP_EDGE + 1.0, x1 - x0, tall - CLIP_EDGE - 2.0);
                    let gain = 10f64.powf(clip.clip.gain_db() / 20.0);
                    masked(window, self.under_edge, |window| {
                        crate::artwork::paint_waveform(
                            window,
                            wave,
                            &peaks,
                            source_at(0.0),
                            source_at(duration),
                            lane_ink::peak(row.room),
                            lane_ink::rms(row.room),
                            gain,
                        )
                    });
                    art = Art::Wave;
                }
            } else {
                let artwork = &self.frame.session.read(cx).artwork;
                let painted = masked(window, self.under_edge, |window| {
                    crate::artwork::paint_filmstrip(
                        window,
                        inside,
                        artwork,
                        source,
                        self.frame.view.pps,
                        source_at,
                    )
                });
                if painted {
                    art = Art::Strip;
                }
            }
            if art != Art::None && self.quiet {
                window.paint_quad(fill(
                    self.under_edge,
                    theme::with_alpha(theme::well(), 0.55),
                ));
            }
        }
        art
    }

    /// Amber stripes moving over it while its media is being made.
    fn making(&self, window: &mut Window) {
        if let Some(activity) = self.activity.filter(|activity| activity.making()) {
            stripes(
                window,
                self.under_edge,
                activity.phase,
                theme::activity::gpu_stripe(),
            );
        }
    }

    /// A signal's number over its time, filled under as a sound's level is,
    /// and what is being drawn over it until the document has it.
    fn signal(&self, window: &mut Window) {
        let (clip, pen, frame) = (self.clip, self.pen, self.frame);
        let (x0, x1, top, tall) = (self.x0, self.x1, self.top, self.tall);
        if clip.kind == ClipKind::Generator && !clip.clip.signal.is_empty() && x1 - x0 >= 4.0 {
            let inside = (x0, top + 2.0, x1 - x0, tall - 4.0);
            masked(window, self.under_edge, |window| {
                paint_signal(&clip.clip.signal, inside, pen, window);
                if let Some((_, drawn)) = frame.signal.as_ref().filter(|(id, _)| *id == clip.id) {
                    paint_drawn(drawn, clip.duration_frames, inside, pen, window);
                }
            });
        }
    }

    /// A sound clip's volume rubber band: at 0 dB with no keys it shows
    /// only where it can be dragged (pointed at, chosen, dragged).
    fn band(&self, window: &mut Window) {
        let (clip, row, frame) = (self.clip, self.row, self.frame);
        if row.audio() && !clip.card() && !row.previewed {
            let (keys, values, keyed) = gain_drawn(clip, frame.lanes.fps, frame.gain.as_ref());
            let dragged = frame.gain.as_ref().is_some_and(|gain| gain.clip == clip.id);
            let level = values.first().copied().unwrap_or(0.0);
            // At 0 dB with no keys it shows only where it can be dragged.
            let shown =
                keyed || level.abs() > 0.05 || self.hover.is_some() || self.selected || dragged;
            if shown {
                let (points, _) = band_line(
                    &keys,
                    &values,
                    keyed,
                    self.x0,
                    self.x1 - self.x0,
                    self.top,
                    self.tall,
                    frame.view.pps,
                );
                let pen = self.pen;
                masked(window, self.body, |window| {
                    paint_band(&points, keyed, pen, window)
                });
            }
        }
    }

    /// Its outline: ink chosen (it reads over the frames); dashed while it
    /// waits its turn (amber) or has no file (a hole); amber being made;
    /// sage when an open proposal changes it; brighter under the pointer.
    fn outline(&self, window: &mut Window) {
        let (clip, activity) = (self.clip, self.activity);
        let dashed = activity.is_some_and(Activity::dashed);
        let (outline, outline_width, style) = if self.selected {
            (theme::ink(), 1.5, BorderStyle::Solid)
        } else if dashed {
            let queued = activity.is_some_and(|activity| !activity.state.absent());
            (
                if queued {
                    theme::activity::gpu()
                } else {
                    theme::lineh()
                },
                1.0,
                BorderStyle::Dashed,
            )
        } else if activity.is_some_and(Activity::making) {
            (theme::activity::gpu(), 1.0, BorderStyle::Solid)
        } else if clip.changed || clip.proposed {
            (theme::pos(), 1.0, BorderStyle::Solid)
        } else if self.hover.is_some() {
            (theme::lineh(), 1.0, BorderStyle::Solid)
        } else {
            (theme::line2(), 1.0, BorderStyle::Solid)
        };
        window.paint_quad(quad(
            self.body,
            px(radius::SMALL),
            gpui::transparent_black(),
            px(outline_width),
            outline,
            style,
        ));
    }

    /// The edge under the pointer, or the edge being trimmed (`trimming`).
    fn edge_bar(&self, trimming: Option<Zone>, window: &mut Window) {
        let edge_bar = trimming
            .or(self
                .hover
                .filter(|hover| !hover.band && self.frame.tool.trims_edges())
                .map(|hover| hover.zone))
            .filter(|zone| *zone != Zone::Body);
        if let Some(zone) = edge_bar {
            let x = if zone == Zone::In {
                self.x0
            } else {
                self.x1 - 2.0
            };
            let (pen, top, tall) = (self.pen, self.top, self.tall);
            masked(window, self.body, |window| {
                window.paint_quad(fill(pen.rect(x, top, 2.0, tall), theme::ink()));
            });
        }
    }

    /// Kerf looking at it rings it.
    fn looked_at(&self, window: &mut Window) {
        let looked_at = self
            .frame
            .kerf
            .as_ref()
            .is_some_and(|(_, clips)| clips.contains(&self.clip.id));
        if looked_at {
            let ring = self.pen.rect(
                self.left - 2.0,
                self.top - 2.0,
                self.across + 4.0,
                self.tall + 4.0,
            );
            window.paint_quad(quad(
                ring,
                px(radius::SMALL + 2.0),
                gpui::transparent_black(),
                px(1.5),
                theme::activity::kerf(),
                BorderStyle::Solid,
            ));
        }
    }
}

/// A clip's name and what goes with it: a red dot when a check found a
/// problem in it, its speed or a title's text after it, an fx mark when it
/// has effects. Over a filmstrip it stands on a dark bar; over a waveform
/// it is shadowed; a caption's is in its middle; with nothing under it,
/// its length goes under it in a tall clip.
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_name(
    frame: &Frame,
    pen: Pen,
    width: f32,
    row: &Row,
    clip: &ClipBox,
    (x0, x1): (f32, f32),
    (top, tall): (f32, f32),
    art: Art,
    quiet: bool,
    duration: f64,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let start = view.time(x0);
    // Kept in sight while its start is scrolled off, and clear of a
    // transition coming into it.
    let into = row
        .transitions
        .iter()
        .filter(|(from, to)| *from <= start + 1e-6 && *to > start + 1e-6)
        .map(|(_, to)| view.x(*to) + 4.0)
        .fold(0.0, f32::max);
    let label_x = (x0.max(0.0) + 6.0).max(into);
    let room = x1.min(width) - label_x - 4.0;
    if room <= 10.0 {
        return;
    }
    let bar_top = top + CLIP_EDGE;
    let bar_tall = NAME_BAR.min(tall - CLIP_EDGE);
    if art == Art::Strip {
        let bar = pen.rect(
            x0.max(-2.0),
            bar_top,
            x1.min(width + 2.0) - x0.max(-2.0),
            bar_tall,
        );
        window.paint_quad(fill(bar, lane_ink::name_bar()));
    }
    let ink = if quiet { theme::dis() } else { theme::ink() };
    let label_y = if clip.kind == ClipKind::Caption || (art == Art::None && tall < 30.0) {
        top + (tall - text::META.line) / 2.0 + 1.0
    } else {
        bar_top + (bar_tall - text::META.line) / 2.0 + 1.0
    };
    let name = shape(window, clip.label.clone(), text::META, &fonts.ui, ink);
    let shadow = (art == Art::Wave).then(|| {
        shape(
            window,
            clip.label.clone(),
            text::META,
            &fonts.ui,
            lane_ink::shadow(),
        )
    });
    let secondary = (!clip.secondary.is_empty()).then(|| {
        shape(
            window,
            clip.secondary.clone(),
            text::META,
            &fonts.ui,
            theme::ink3(),
        )
    });
    let fx = clip.fx.then(|| {
        shape(
            window,
            "fx".into(),
            text::MONO_SMALL,
            &fonts.mono,
            theme::ink3(),
        )
    });
    let length = (art == Art::None
        && clip.kind != ClipKind::Caption
        && tall >= 36.0
        && room > f32::from(name.width) + 8.0)
        .then(|| {
            let ink = if quiet { theme::dis() } else { theme::ink3() };
            shape(
                window,
                model::length(duration).into(),
                text::MONO_SMALL,
                &fonts.mono,
                ink,
            )
        });
    masked(window, pen.rect(label_x, top, room, tall), |window| {
        let mut x = label_x;
        if clip.qc {
            let dot = pen.rect(x, label_y + (text::META.line - 6.0) / 2.0, 6.0, 6.0);
            window.paint_quad(quad(
                dot,
                px(3.0),
                theme::neg(),
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
            x += 10.0;
        }
        if let Some(shadow) = &shadow {
            if let Err(error) = shadow.paint(
                pen.at(x + 1.0, label_y + 1.0),
                px(text::META.line),
                window,
                cx,
            ) {
                log::warn!("a clip name's shadow did not draw: {error}");
            }
        }
        if let Err(error) = name.paint(pen.at(x, label_y), px(text::META.line), window, cx) {
            log::warn!("a clip's name did not draw: {error}");
        }
        x += f32::from(name.width) + 6.0;
        if let Some(secondary) = &secondary {
            if let Err(error) = secondary.paint(pen.at(x, label_y), px(text::META.line), window, cx)
            {
                log::warn!("what follows a clip's name did not draw: {error}");
            }
            x += f32::from(secondary.width) + 6.0;
        }
        if let Some(fx) = &fx {
            if let Err(error) = fx.paint(pen.at(x, label_y), px(text::MONO_SMALL.line), window, cx)
            {
                log::warn!("a clip's fx mark did not draw: {error}");
            }
        }
        if let Some(length) = &length {
            if let Err(error) = length.paint(
                pen.at(label_x, label_y + 15.0),
                px(text::MONO_SMALL.line),
                window,
                cx,
            ) {
                log::warn!("a clip's length did not draw: {error}");
            }
        }
    });
}
