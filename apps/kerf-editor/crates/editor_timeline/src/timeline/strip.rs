//! The strip over the lanes: the transport while the window shows no
//! picture, the timecode where the playhead is (or the pointer skims) and
//! the cut's length, the tools; at its right Snap and the zoom
//! (− slider + and Fit). As much shows as fits ([`StripFit`]), from how
//! wide it and its pieces were last laid out.

use super::drag::Gesture;
use super::tools::Tool;
use super::viewport::{zoom_share, ZOOM_STEP};
use super::{TimelineView, ZOOM_REPEAT, ZOOM_REPEAT_DELAY};
use editor_session::commands::RunCommand;
use editor_stage::stage;
use editor_state::model;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, layout, radius, text};
use editor_ui::ui;
use gpui::{
    canvas, div, prelude::*, px, AnimationExt, App, Context, ElementId, MouseButton,
    MouseDownEvent, MouseUpEvent,
};
use serde_json::Value;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

/// Points between what the strip holds.
const STRIP_GAP: f32 = 8.0;
/// Points before the strip's tools, past the cut's length.
const TOOLS_SPACE: f32 = 8.0;
/// Points from the strip's last control to its edge, and to its first
/// without the transport.
const STRIP_END: f32 = 6.0;
const STRIP_START: f32 = 12.0;
/// The zoom slider's width on the strip.
const SLIDER: f32 = 72.0;

/// Whether the timeline's strip carries the transport: while the window
/// shows no picture (View ▸ Picture off, `Session::picture_open` false),
/// so the transport stays at hand with the picture hidden. Until the
/// window says (None) the stage has it.
pub fn strip_has_transport(picture_open: Option<bool>) -> bool {
    picture_open == Some(false)
}

/// What the timeline's strip shows of what it can let go. Too narrow for
/// all of it (a narrow window, the transport in it), it lets go first of
/// the zoom slider (− and + stay), then of the transport's Go to start and
/// Go to end (Home and End still reach them) and the transport's wide gap,
/// then of the cut's length and the space before the tools. What still
/// does not fit is cut off at the tools' end: Snap, − and +, and Fit stay
/// whole, so each can still be pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripFit {
    pub slider: bool,
    pub transport_ends: bool,
    pub length: bool,
}

impl StripFit {
    pub const ALL: StripFit = StripFit {
        slider: true,
        transport_ends: true,
        length: true,
    };

    /// Each step, from all of it, as they are let go.
    pub const STEPS: [StripFit; 4] = [
        StripFit::ALL,
        StripFit {
            slider: false,
            ..StripFit::ALL
        },
        StripFit {
            slider: false,
            transport_ends: false,
            length: true,
        },
        StripFit {
            slider: false,
            transport_ends: false,
            length: false,
        },
    ];

    /// How much wider the strip is with this shown than with all of it let
    /// go: `transport`, whether it carries the transport (three buttons
    /// always, five with its ends); `length`, how wide the cut's length is
    /// set.
    pub fn width(self, transport: bool, length: f32) -> f32 {
        let button = theme::height::H24 + layout::STRIP_TRANSPORT_SPACING;
        let mut width = 0.0;
        if transport {
            width += 3.0 * button - layout::STRIP_TRANSPORT_SPACING + STRIP_GAP;
            if self.transport_ends {
                width += 2.0 * button + layout::STRIP_TRANSPORT_GAP - STRIP_GAP;
            }
        }
        if self.slider {
            width += SLIDER + STRIP_GAP;
        }
        if self.length {
            width += length + STRIP_GAP + TOOLS_SPACE + STRIP_GAP;
        }
        width
    }

    /// What fits in `room`, from what it shows now: the most of it whose
    /// width, over `rest` (what the strip needs with all of it let go),
    /// fits; showing more than now only with [`layout::STRIP_REGROW`] to
    /// spare. With none fitting, all of it let go.
    pub fn within(self, room: f32, rest: f32, transport: bool, length: f32) -> StripFit {
        let now = Self::STEPS
            .iter()
            .position(|step| *step == self)
            .unwrap_or(0);
        Self::STEPS
            .into_iter()
            .enumerate()
            .find(|(step, fit)| {
                let spare = if *step < now {
                    layout::STRIP_REGROW
                } else {
                    0.0
                };
                rest + fit.width(transport, length) + spare <= room
            })
            .map_or(Self::STEPS[3], |(_, fit)| fit)
    }
}

/// What the strip showed, and whether it carried the transport, as it was
/// laid out at the widths [`StripWidths`] keeps.
type Laid = (StripFit, bool);

/// How wide the strip, what it holds on its left and on its right (each
/// as wide as its pieces would be, cut off or not), and the cut's length
/// were last laid out, and what it showed then.
#[derive(Clone)]
pub(super) struct StripWidths {
    room: Rc<Cell<f32>>,
    left: Rc<Cell<f32>>,
    right: Rc<Cell<f32>>,
    length: Rc<Cell<f32>>,
    laid: Rc<Cell<Laid>>,
}

impl Default for StripWidths {
    fn default() -> Self {
        Self {
            room: Rc::default(),
            left: Rc::default(),
            right: Rc::default(),
            length: Rc::default(),
            laid: Rc::new(Cell::new((StripFit::ALL, false))),
        }
    }
}

/// Over its parent, a canvas that keeps the parent's width in `cell`; a
/// new width draws the strip again with it. With `laid`, it keeps what
/// the strip shows at that width in its cell too.
fn width_canvas(cell: Rc<Cell<f32>>, laid: Option<(Rc<Cell<Laid>>, Laid)>) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            if let Some((kept, shows)) = &laid {
                kept.set(*shows);
            }
            let width = f32::from(bounds.size.width);
            if (cell.get() - width).abs() > 0.1 {
                cell.set(width);
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl TimelineView {
    /// Why each of the strip's transport controls cannot run now, asked of
    /// the registry again only when what it is asked about changes, not on
    /// every frame played; and each time the strip takes the transport
    /// back, one more coming, so it fades in anew. Followed as the timeline
    /// draws, while the strip carries the transport.
    pub(super) fn follow_transport(&mut self, cx: &App) {
        let has_transport = strip_has_transport(self.session.read(cx).picture_open);
        if has_transport != self.strip_transport.0 {
            self.strip_transport.0 = has_transport;
            if has_transport {
                self.strip_transport.1 += 1;
            }
        }
        if !has_transport {
            return;
        }
        let context = self.transport_context(cx);
        if self
            .transport_asked
            .as_ref()
            .is_none_or(|(asked, _)| *asked != context)
        {
            let reasons = stage::transport_reasons(&context);
            self.transport_asked = Some((context, reasons));
        }
    }

    /// What the transport's reasons are asked of: the command context, the
    /// playhead left out.
    fn transport_context(&self, cx: &App) -> Value {
        let mut context = self.session.read(cx).command_context();
        if let Some(context) = context.as_object_mut() {
            context.remove("playheadFrame");
        }
        context
    }

    /// Why each transport control cannot run now: as asked as the
    /// timeline began to draw this frame ([`Self::follow_transport`]).
    fn transport_reasons(&self) -> HashMap<&'static str, String> {
        self.transport_asked
            .as_ref()
            .map(|(_, reasons)| reasons.clone())
            .unwrap_or_default()
    }

    /// The transport, in the strip while the picture pane is closed: Go to
    /// start, Previous frame, Play (Pause while playing), Next frame, Go to
    /// end, each its command ([`stage::TRANSPORT`]), its key in its tip,
    /// greyed with the registry's reason while it cannot run
    /// ([`stage::transport_reasons`], [`stage::transport_tip`]), as the
    /// stage's are. It fades in over `FADE` each time it comes. Without
    /// `ends` (a narrow strip, [`StripFit`]) it has no Go to start and Go
    /// to end, and the strip's own gap to the timecode.
    fn strip_transport(&self, ends: bool, cx: &Context<Self>) -> impl IntoElement {
        let reasons = self.transport_reasons();
        let playing = self.session.read(cx).state.playing;
        let buttons: Vec<_> = stage::TRANSPORT
            .into_iter()
            .filter(|(id, _, _)| ends || !matches!(*id, "playback.goToStart" | "playback.goToEnd"))
            .map(|(id, name, icon)| {
                let play = id == "playback.toggle";
                let (name, icon) = if play && playing {
                    ("Pause", Icon::Pause)
                } else {
                    (name, icon)
                };
                let reason = reasons.get(id).cloned();
                let mut button = ui::icon_button(
                    ElementId::Name(format!("timeline-transport-{id}").into()),
                    icon,
                )
                .size(theme::height::H24)
                .tooltip(name)
                .disabled(reason.is_some())
                .focusable(false)
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand { id: id.into() }), cx);
                });
                // Play in ink at rest, so it stands out from the rest of
                // the transport.
                if play {
                    button = button.color(theme::ink());
                }
                if let Some(key) = self.keys_of(id, cx) {
                    button = button.key(key);
                }
                if let Some(reason) = reason {
                    button = button.reason(stage::transport_tip(name, Some(&reason)));
                }
                button
            })
            .collect();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(layout::STRIP_TRANSPORT_SPACING))
            // To the timecode, the transport's wide gap, the strip's own
            // gap counted in.
            .when(ends, |transport| {
                transport.pr(px(layout::STRIP_TRANSPORT_GAP - STRIP_GAP))
            })
            .children(buttons)
            .with_animation(
                ("timeline-transport", self.strip_transport.1),
                motion::animation(theme::motion::FADE),
                |transport, t| transport.opacity(t),
            )
    }

    /// What the strip shows now ([`StripFit`]): from what it showed as it
    /// was last laid out, how wide it was and how wide its pieces were then
    /// (less those that come and go), with the transport in it or not.
    fn strip_shows(&self, has_transport: bool) -> StripFit {
        let widths = &self.strip_widths;
        let (shown, carried) = widths.laid.get();
        let length_width = widths.length.get();
        let room = widths.room.get();
        if room <= 0.0 {
            return shown;
        }
        // Its pieces as last laid out, less those that come and go then.
        let rest = widths.left.get() + widths.right.get() - shown.width(carried, length_width);
        let start = if has_transport {
            layout::STRIP_EDGE
        } else {
            STRIP_START
        };
        let room = room - start - STRIP_END - 2.0 * STRIP_GAP;
        shown.within(room, rest, has_transport, length_width)
    }

    /// The strip over the lanes: the transport (while the picture pane is
    /// closed), the timecode where the playhead is and the cut's length,
    /// the tools; at its right Snap and the zoom. As much shows as fits
    /// ([`StripFit`]), from how wide it and its pieces were last laid out.
    pub(super) fn strip(&self, shown: f64, skimming: bool, cx: &Context<Self>) -> impl IntoElement {
        let has_transport = strip_has_transport(self.session.read(cx).picture_open);
        let shows = self.strip_shows(has_transport);
        let widths = self.strip_widths.clone();
        let left = self.strip_left(shown, skimming, has_transport, shows, cx);
        let right = self.strip_right(shows, cx);
        div()
            .h(px(layout::STRIP))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .gap(px(STRIP_GAP))
            .pl(px(if has_transport {
                layout::STRIP_EDGE
            } else {
                STRIP_START
            }))
            .pr(px(STRIP_END))
            .bg(theme::chrome())
            .border_b_1()
            .border_color(theme::line())
            .child(width_canvas(
                widths.room.clone(),
                Some((widths.laid.clone(), (shows, has_transport))),
            ))
            .child(
                div()
                    .flex()
                    .flex_shrink()
                    .min_w_0()
                    .overflow_hidden()
                    .child(left),
            )
            .child(div().flex_1())
            .child(right)
    }

    /// The strip's left: with the picture pane closed, the transport first;
    /// then where the playhead is (`shown`, in ink2 where the pointer
    /// skims), the cut's length, the tools. Cut off at the tools' end where
    /// it must be.
    fn strip_left(
        &self,
        shown: f64,
        skimming: bool,
        has_transport: bool,
        shows: StripFit,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let widths = &self.strip_widths;
        let transport = has_transport.then(|| self.strip_transport(shows.transport_ends, cx));
        let lanes = self.lanes.clone();
        let this = cx.entity().downgrade();
        let tips: Vec<String> = [Tool::Selection, Tool::Razor]
            .into_iter()
            .map(|tool| match self.keys_of(tool.command(), cx) {
                Some(key) => format!("{}  ·  {key}", tool.name()),
                None => tool.name().to_owned(),
            })
            .collect();
        let tips: Vec<&str> = tips.iter().map(String::as_str).collect();
        let tools = ui::segmented(
            "timeline-tools",
            &[(Tool::Selection, "Selection"), (Tool::Razor, "Razor")],
            self.tool,
            move |tool, _, cx| {
                let _ = this.update(cx, |view, cx| view.set_tool(tool, cx));
            },
        )
        .tips(&tips)
        .focusable(false);
        let other_tool = !matches!(self.tool, Tool::Selection | Tool::Razor);
        let tool_key = self.keys_of(self.tool.command(), cx);
        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(STRIP_GAP))
            .child(width_canvas(widths.left.clone(), None))
            .children(transport)
            .child(ui::label(
                model::timecode(shown, lanes.fps),
                text::TIMECODE_STRONG,
                if skimming {
                    theme::ink2()
                } else {
                    theme::ink()
                },
            ))
            .when(shows.length, |left| {
                left.child(
                    ui::label(
                        model::timecode(lanes.end, lanes.fps),
                        text::TIMECODE,
                        theme::ink3(),
                    )
                    .relative()
                    .flex_none()
                    .child(width_canvas(widths.length.clone(), None)),
                )
            })
            .when(lanes.proposed, |left| {
                left.child(ui::chip("Proposed cut", theme::pos()))
            })
            .when(shows.length, |left| {
                left.child(div().w(px(TOOLS_SPACE)).flex_none())
            })
            .child(tools)
            .when(other_tool, |left| {
                left.child(ui::tipped(
                    "timeline-tool-other",
                    ui::Tip::new(format!("{} tool", self.tool.name())).maybe_key(tool_key),
                    ui::chip(self.tool.name(), theme::accent_ink()),
                ))
            })
    }

    /// The strip's right: Snap, then − (the slider while it fits) + and Fit.
    fn strip_right(&self, shows: StripFit, cx: &Context<Self>) -> impl IntoElement {
        let snap_key = self.keys_of("sequence.snap", cx);
        let zoom_in = editor_state::shortcuts::shown_keys("Ctrl+=");
        let zoom_out = editor_state::shortcuts::shown_keys("Ctrl+-");
        let (fit, most) = self.zoom_range();
        let share = zoom_share(self.view.pps, fit, most);
        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(STRIP_GAP))
            .child(width_canvas(self.strip_widths.right.clone(), None))
            .child(ui::tipped(
                "timeline-snap-tip",
                ui::Tip::new("Snap").maybe_key(snap_key),
                strip_button("timeline-snap", "Snap", self.snap).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.snap = !this.snap;
                        cx.notify();
                    },
                )),
            ))
            .child(ui::hairline(false).h(px(14.0)))
            .child(ui::tipped(
                "timeline-zoom-out-tip",
                ui::Tip::new("Zoom out").key(zoom_out),
                self.zoom_button("timeline-zoom-out", "\u{2212}", 1.0 / ZOOM_STEP, cx),
            ))
            .when(shows.slider, |right| {
                right.child(ui::tipped(
                    "timeline-zoom-slider-tip",
                    ui::Tip::new("Timeline zoom"),
                    self.slider(share, cx),
                ))
            })
            .child(ui::tipped(
                "timeline-zoom-in-tip",
                ui::Tip::new("Zoom in").key(zoom_in),
                self.zoom_button("timeline-zoom-in", "+", ZOOM_STEP, cx),
            ))
            .child(
                strip_button("timeline-fit", "Fit", self.zoom.is_none()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.zoom = None;
                        this.left = 0.0;
                        cx.notify();
                    },
                )),
            )
    }

    /// The strip's − or +: one step of zoom by `factor` a click (on its
    /// release), and held down, a step after [`ZOOM_REPEAT_DELAY`] and
    /// every [`ZOOM_REPEAT`] after it until it is let go or the pointer
    /// leaves it.
    fn zoom_button(
        &self,
        id: &'static str,
        words: &'static str,
        factor: f32,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        strip_button(id, words, false)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| this.hold_zoom(factor, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.zoom_held = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.zoom_held = None),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                if !*hovered {
                    this.zoom_held = None;
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.zoom_held = None;
                this.zoom_by(factor, None);
                cx.notify();
            }))
    }

    /// − or + pressed: while it is held, a step of zoom after
    /// [`ZOOM_REPEAT_DELAY`] and every [`ZOOM_REPEAT`] after that. Letting
    /// go (or leaving it) drops the task, which stops it.
    fn hold_zoom(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.zoom_held = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ZOOM_REPEAT_DELAY).await;
            loop {
                let held = this.update(cx, |this, cx| {
                    this.zoom_by(factor, None);
                    cx.notify();
                });
                if held.is_err() {
                    break;
                }
                cx.background_executor().timer(ZOOM_REPEAT).await;
            }
        }));
    }

    /// The zoom's slider: a rail, its fill to the knob, the knob; pressed
    /// or dragged, the zoom follows it on a log scale.
    fn slider(&self, share: f32, cx: &Context<Self>) -> impl IntoElement {
        let cell = self.slider.clone();
        let knob = 10.0;
        let travel = SLIDER - knob;
        let at = share * travel;
        div()
            .id("timeline-zoom-slider")
            .relative()
            .flex_none()
            .w(px(SLIDER))
            .h(px(22.0))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.gesture = Some(Gesture::Slider);
                    this.slide_zoom(event.position, cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(knob / 2.0))
                    .right(px(knob / 2.0))
                    .top(px(10.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(theme::track()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(knob / 2.0))
                    .w(px(at))
                    .top(px(10.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(theme::ink3()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(at))
                    .top(px(6.0))
                    .size(px(knob))
                    .rounded_full()
                    .bg(theme::ink())
                    .border_1()
                    .border_color(theme::line2()),
            )
    }
}

/// A small button of words on the strip, held down when `on`.
fn strip_button(id: &'static str, words: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    strip_button_owned(id.into(), words, on)
}

pub(super) fn strip_button_owned(
    id: ElementId,
    words: &'static str,
    on: bool,
) -> gpui::Stateful<gpui::Div> {
    theme::type_style(div(), text::CONTROL)
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(22.0))
        .min_w(px(22.0))
        .px(px(7.0))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .text_color(if on { theme::ink() } else { theme::ink2() })
        .when(on, |button| button.bg(theme::on()))
        .when(!on, |button| {
            button.hover(|style| style.bg(theme::raised()).text_color(theme::ink()))
        })
        .child(words)
}
