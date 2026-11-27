//! The panel at the window's right: the picture (and the chosen clip's
//! inspector beside it), the Media band down from under it, the timeline.
//! How it opens and goes, and each of its panes with it; how its height is
//! shared between them by the picture's shape ([`panel_heights`]); and where
//! the stage puts the picture in its slot ([`picture_in`]).

use super::flights::PaneGoes;
use super::{Regions, Workspace};
use editor_ui::motion::Tween;
use editor_ui::theme::{self, layout, motion as scene};
use gpui::{canvas, div, point, prelude::*, px, size, AnyElement, Bounds, Context, Pixels, Window};
use std::time::Duration;

/// Where the stage was laid out in the window, and where the panel's left
/// edge was drawn then and was to come to rest: kept as the stage is laid
/// out, for the next frame and for what flies from the picture.
#[derive(Clone, Copy)]
pub(super) struct StageSlot {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) panel_left: (f32, f32),
}

impl Workspace {
    /// The panel's button: both the picture and the timeline come in, or
    /// the panel goes with whatever it showed.
    pub fn set_panel(&mut self, open: bool, cx: &mut Context<Self>) {
        if open {
            self.open_panel(true, true, scene::PANEL, cx);
            return;
        }
        // The picture goes back into its cut's card, while that is in sight;
        // what else flew into the panel is let go where it is.
        self.send_picture_back(cx);
        self.let_flights_go(PaneGoes::Panel);
        self.panel_open = false;
        self.panel_by_hand = true;
        self.panel_shown.go(0.0, scene::PANEL);
        if self.media_open {
            self.media_open = false;
            self.media_shown.go(0.0, scene::PANEL);
        }
        self.settle_soon(cx);
        self.tell_picture_open(cx);
    }

    /// Whether the picture shows: the panel open with the picture in it.
    fn picture_in_view(&self) -> bool {
        self.panel_open && self.picture_shown.target() > 0.5
    }

    /// The session knows whether the picture shows: a request is stamped
    /// with the playhead, and a cut's card says "On the stage", only while
    /// it does. However the panel came or went, it is told once, when what
    /// changed it is done with: as it then stands.
    pub(super) fn tell_picture_open(&self, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let session = self.session.clone();
        cx.defer(move |cx| {
            let Some(picture) = this.upgrade().map(|this| this.read(cx).picture_in_view()) else {
                return;
            };
            session.update(cx, |session, cx| {
                if session.picture_open != Some(picture) {
                    session.picture_open = Some(picture);
                    cx.notify();
                }
            });
        });
    }

    /// Opens the panel over `duration` with what is asked for in it: with
    /// the panel closed only that comes in, and has the whole panel; open
    /// already, it joins what shows.
    pub(super) fn open_panel(
        &mut self,
        picture: bool,
        timeline: bool,
        duration: Duration,
        cx: &mut Context<Self>,
    ) {
        let shown = (
            self.picture_shown.target() > 0.5,
            self.timeline_shown.target() > 0.5,
        );
        let (show_picture, show_timeline) =
            panes_opening(self.panel_open, shown, (picture, timeline));
        if !self.panel_open {
            // Coming in anew, nothing in it is growing or going back.
            self.grow = None;
            self.going_back = None;
        }
        for (tween, show, was) in [
            (&mut self.picture_shown, show_picture, shown.0),
            (&mut self.timeline_shown, show_timeline, shown.1),
        ] {
            let target = if show { 1.0 } else { 0.0 };
            if !self.panel_open {
                // It comes in as it will be: nothing in it moves meanwhile.
                tween.set(target);
            } else if show != was {
                tween.go(target, scene::PANEL);
            }
        }
        self.panel_open = true;
        self.panel_by_hand = false;
        self.panel_shown.go(1.0, duration);
        self.tell_picture_open(cx);
        cx.notify();
    }

    pub(super) fn set_media(&mut self, open: bool, cx: &mut Context<Self>) {
        self.media_open = open;
        if open && !self.panel_open {
            // With the panel closed, the panel comes in with the band in it,
            // the picture and the timeline with it.
            self.media_shown.set(1.0);
            self.open_panel(true, true, scene::PANEL, cx);
        } else {
            self.media_shown
                .go(if open { 1.0 } else { 0.0 }, scene::PANEL);
        }
        if !open {
            self.settle_soon(cx);
        }
        cx.notify();
    }

    /// Looking at something (a timecode, a Preview, a mask tool) opens the
    /// picture: with the panel closed, the picture alone.
    pub(super) fn reveal_picture(&mut self, cx: &mut Context<Self>) {
        if !self.panel_open || self.picture_shown.target() < 0.5 {
            self.open_panel(true, false, scene::PANEL, cx);
        }
    }

    /// Takes out mask tool `tool` on the picture, or puts it down; the
    /// picture comes in for it. Not while something is open over the window.
    pub(super) fn toggle_mask_tool(
        &mut self,
        tool: editor_stage::stage::MaskTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal.is_some() {
            return;
        }
        let taken = self
            .stage
            .update(cx, |stage, cx| stage.toggle_tool(tool, window, cx));
        if taken && self.stage.read(cx).mask_tool().is_some() {
            self.reveal_picture(cx);
        }
    }

    /// One of the panel's two, the picture or the timeline, on or off;
    /// asked for with the panel closed, it comes in alone.
    pub(super) fn toggle_pane(&mut self, picture: bool, cx: &mut Context<Self>) {
        if !self.panel_open {
            self.open_panel(picture, !picture, scene::PANEL, cx);
            return;
        }
        let (this, other) = if picture {
            (&self.picture_shown, &self.timeline_shown)
        } else {
            (&self.timeline_shown, &self.picture_shown)
        };
        let showing = this.target() > 0.5;
        if showing && other.target() < 0.5 {
            // The last of the two going closes the panel.
            self.set_panel(false, cx);
            return;
        }
        if picture && showing {
            self.send_picture_back(cx);
        }
        if showing {
            self.let_flights_go(if picture {
                PaneGoes::Picture
            } else {
                PaneGoes::Timeline
            });
        }
        let this = if picture {
            &mut self.picture_shown
        } else {
            &mut self.timeline_shown
        };
        this.go(if showing { 0.0 } else { 1.0 }, scene::PANEL);
        // The timeline gone gives the keys back; come, it takes them.
        self.settle_soon(cx);
        self.tell_picture_open(cx);
    }

    /// Where the stage's slot is once the panel has come to rest, as
    /// `regions` lay the window out now: what the picture grows into.
    pub(super) fn picture_slot(&self, regions: &Regions) -> Bounds<Pixels> {
        let shown = |tween: &Tween| tween.target().clamp(0.0, 1.0);
        let laid = panel_layout(
            regions.panel_width - 8.0,
            regions.content_height - 8.0,
            regions.aspect,
            (
                shown(&self.picture_shown),
                shown(&self.timeline_shown),
                shown(&self.media_shown),
            ),
            regions.inspecting,
        );
        Bounds::new(
            point(
                px(regions.rest_left),
                px(layout::TOOLBAR + regions.bars_height),
            ),
            size(px(laid.stage_width.max(1.0)), px(laid.picture)),
        )
    }

    /// The panel, beside the conversation as `regions` lay it out (as wide
    /// as its room and what it reaches over the conversation, less its
    /// gutter): the picture (and the inspector), the Media band, the
    /// timeline. `shows` is how much of the stage and of the timeline is
    /// drawn (both whole, but while the picture grows or goes back into its
    /// card).
    pub(super) fn panel(
        &self,
        regions: &Regions,
        shows: (f32, f32),
        window: &Window,
    ) -> AnyElement {
        let width = regions.panel_width + regions.reach - 8.0;
        let height = regions.content_height - 8.0;
        let (inspecting, aspect) = (regions.inspecting, regions.aspect);
        let panel_left = (regions.panel_left, regions.rest_left);
        let picture = self.picture_shown.sample(window);
        let timeline = self.timeline_shown.sample(window);
        let media = self.media_shown.sample(window);
        let PanelLayout {
            picture: picture_height,
            band,
            timeline: timeline_height,
            ..
        } = panel_layout(
            width,
            height,
            aspect,
            (picture, timeline, media),
            inspecting,
        );
        let (stage_shows, timeline_shows) = shows;
        div()
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(picture_height))
                    .w_full()
                    .flex_none()
                    .flex()
                    .overflow_hidden()
                    .when(stage_shows < 1.0, |row| row.opacity(stage_shows))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .relative()
                            .child(self.stage.clone())
                            // Where the stage is, for a frame to fly from,
                            // and where the panel was as it was laid out.
                            .child({
                                let slot = self.stage_slot.clone();
                                canvas(
                                    move |bounds, _, _| {
                                        slot.set(Some(StageSlot { bounds, panel_left }))
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0()
                            }),
                    )
                    .when(inspecting, |row| {
                        row.child(
                            div()
                                .w(px(layout::INSPECTOR))
                                .flex_none()
                                .h_full()
                                .border_l_1()
                                .border_color(theme::line())
                                .child(self.inspector.clone()),
                        )
                    }),
            )
            .child(
                // The band comes down out from under the picture. Put away
                // and at rest, nothing in it is a Tab stop.
                div()
                    .h(px(band))
                    .w_full()
                    .flex_none()
                    .overflow_hidden()
                    .relative()
                    .when(!self.media_open && media <= 0.0, |band| band.invisible())
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(layout::MEDIA_BAND))
                            .child(self.media.clone()),
                    ),
            )
            .child(
                div()
                    .h(px(timeline_height))
                    .w_full()
                    .flex_none()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(theme::line())
                    .when(timeline_shows < 1.0, |lanes| lanes.opacity(timeline_shows))
                    .child(self.timeline.clone()),
            )
            .into_any_element()
    }
}

/// What the panel shows once something in it is asked for: `asked` (the
/// picture, the timeline) alone when the panel is closed (`open` false),
/// else what it `shown` with what is asked joining it. Neither asked for
/// opens both.
pub fn panes_opening(open: bool, shown: (bool, bool), asked: (bool, bool)) -> (bool, bool) {
    let asked = if asked == (false, false) {
        (true, true)
    } else {
        asked
    };
    if open {
        (shown.0 || asked.0, shown.1 || asked.1)
    } else {
        asked
    }
}

/// Where the stage puts a `width` × `height` picture within `slot` (the
/// stage, its transport under it): in its room, inset as the stage insets
/// its device, at its own aspect, centred.
pub fn picture_within(slot: Bounds<Pixels>, width: u32, height: u32) -> Bounds<Pixels> {
    let aspect = if width == 0 || height == 0 {
        16.0 / 9.0
    } else {
        width as f32 / height as f32
    };
    picture_in(slot, aspect)
}

/// [`picture_within`] for a picture `aspect` wide to its height.
pub fn picture_in(slot: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let area_height = (f32::from(slot.size.height) - layout::TRANSPORT).max(1.0);
    let inset = STAGE_MARGIN + STAGE_BEZEL;
    let room_width = (f32::from(slot.size.width) - inset * 2.0).max(1.0);
    let room_height = (area_height - inset * 2.0).max(1.0);
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let (shown_width, shown_height) = if room_width / room_height > aspect {
        (room_height * aspect, room_height)
    } else {
        (room_width, room_width / aspect)
    };
    Bounds::new(
        point(
            slot.origin.x + px((f32::from(slot.size.width) - shown_width) / 2.0),
            slot.origin.y + px((area_height - shown_height) / 2.0),
        ),
        size(px(shown_width), px(shown_height)),
    )
}

/// How tall the timeline is under the picture in a panel `height` tall, with
/// the Media band `band` tall between them and the stage `width` wide, split by
/// the picture's shape (`aspect`, width over height): under a wide picture
/// [`layout::TIMELINE_WIDE_SHARE`] of the height, at least
/// [`layout::TIMELINE_WIDE_LEAST`], and what the picture cannot use at that
/// width; under a vertical or square one [`layout::TIMELINE_UPRIGHT_SHARE`], at
/// least [`layout::TIMELINE_UPRIGHT_LEAST`]. The band takes the timeline's top
/// until that would leave it under [`layout::MEDIA_BAND_TIMELINE_LEAST`]; then
/// the picture gives way. The transport, under the picture, counts as the
/// timeline's.
pub fn timeline_split(height: f32, band: f32, width: f32, aspect: f32) -> f32 {
    let height = height.max(0.0);
    let band = band.clamp(0.0, height);
    let wide = aspect >= layout::WIDE;
    let (share, least) = if wide {
        (layout::TIMELINE_WIDE_SHARE, layout::TIMELINE_WIDE_LEAST)
    } else {
        (
            layout::TIMELINE_UPRIGHT_SHARE,
            layout::TIMELINE_UPRIGHT_LEAST,
        )
    };
    // The picture's slot, with its air; under it the timeline and its transport.
    let mut slot = height - (height * share).max(least).min(height);
    if wide {
        let air = (STAGE_MARGIN + STAGE_BEZEL) * 2.0;
        slot = slot.min((width - air).max(0.0) / aspect + air);
    }
    if band > 0.0 {
        slot = slot.min(height - band - layout::MEDIA_BAND_TIMELINE_LEAST);
    }
    let picture = (slot.max(0.0) + layout::TRANSPORT).min(height - band);
    (height - band - picture).max(0.0)
}

/// How a panel `height` tall, its picture's row `width` wide showing a
/// picture `aspect` wide to its height, is shared with the picture, the
/// timeline and the Media band shown `(picture, timeline, media)` (each
/// 0..1, as far as they have come): the picture's height, the band's and
/// the timeline's. The timeline's share follows the picture's shape
/// ([`timeline_split`]); the picture takes the rest; one of the two alone
/// has all of it.
pub fn panel_heights(
    height: f32,
    width: f32,
    aspect: f32,
    (picture, timeline, media): (f32, f32, f32),
) -> (f32, f32, f32) {
    let band = layout::MEDIA_BAND * media;
    let timeline_full = (height - band).max(0.0);
    let timeline_tall = timeline_split(height, band, width, aspect);
    let timeline_height = timeline_tall + (timeline_full - timeline_tall) * (1.0 - picture);
    let timeline_height = timeline_height * timeline;
    let picture_height = (height - band - timeline_height).max(0.0);
    (picture_height, band, timeline_height)
}

/// Where the panel's panes go: the stage's width, and the heights of the
/// picture's row (the transport at its foot), the Media band and the
/// timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLayout {
    pub stage_width: f32,
    pub picture: f32,
    pub band: f32,
    pub timeline: f32,
}

/// How a panel `width` × `height` is laid out ([`panel_heights`]), with
/// the chosen clip's inspector beside the picture or not (`inspecting`).
/// The heights are split by the picture's row as wide as the panel, the
/// inspector in it or not: it takes its side of the row from the stage,
/// and the picture is drawn smaller in what is left. Choosing a clip, or
/// letting it go, moves neither the transport nor the lanes under the
/// pointer, so the next press lands where it was aimed.
pub fn panel_layout(
    width: f32,
    height: f32,
    aspect: f32,
    shown: (f32, f32, f32),
    inspecting: bool,
) -> PanelLayout {
    let (picture, band, timeline) = panel_heights(height, width, aspect, shown);
    let inspector = if inspecting { layout::INSPECTOR } else { 0.0 };
    PanelLayout {
        stage_width: (width - inspector).max(0.0),
        picture,
        band,
        timeline,
    }
}

/// The stage's air round its device, and the device's bezel.
const STAGE_MARGIN: f32 = 24.0;
const STAGE_BEZEL: f32 = 8.0;
