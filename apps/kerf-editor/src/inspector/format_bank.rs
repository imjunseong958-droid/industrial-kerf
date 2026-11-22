//! The toolbar's format bank: the sequence's shape, chosen from a few
//! (16:9, 9:16, 1:1, 4:5), as one Tab stop whose Left and Right choose the
//! format beside the one it is.

use crate::session::Session;
use crate::theme::{self, height, radius, space, text};
use crate::ui;
use gpui::{canvas, div, prelude::*, px, ClickEvent, Context, Entity, SharedString, Window};
use serde_json::json;

/// The shapes a sequence is offered in: what it is called, its ratio, and its size.
const FORMATS: [(&str, &str, u32, u32); 4] = [
    ("Landscape", "16:9", 1920, 1080),
    ("Vertical", "9:16", 1080, 1920),
    ("Square", "1:1", 1080, 1080),
    ("Portrait", "4:5", 1080, 1350),
];

/// The format beside `chosen` in the toolbar's bank (Right: `forward`,
/// else Left): the next one, none past either end; from a size of its own
/// (None), the first or the last.
pub fn format_beside(chosen: Option<usize>, forward: bool) -> Option<usize> {
    match chosen {
        Some(at) if forward => (at + 1 < FORMATS.len()).then_some(at + 1),
        Some(at) => at.checked_sub(1).filter(|before| *before < FORMATS.len()),
        None if forward => Some(0),
        None => Some(FORMATS.len() - 1),
    }
}

/// Which of the bank's formats a sequence `width` × `height` is.
pub fn format_index(width: u32, height: u32) -> Option<usize> {
    FORMATS
        .iter()
        .position(|(_, _, w, h)| *w == width && *h == height)
}

/// The sequence's shape, chosen from a few (16:9, 9:16, 1:1, 4:5), in the toolbar.
/// The one the snapshot has is filled; another size fills none and says so.
///
/// It is one Tab stop, between the title and the sync chip: ringed while it
/// has the keys, Left and Right choose the format beside the one it is.
pub struct FormatBank {
    session: Entity<Session>,
    focus: gpui::FocusHandle,
}

impl FormatBank {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// Left (`forward` false) or Right from the format it is: the one
    /// beside it, none past either end; from a size of its own, the first
    /// or the last.
    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let timeline = self.session.read(cx).timeline();
        let chosen = format_index(timeline.width.unwrap_or(0), timeline.height.unwrap_or(0));
        if let Some(next) = format_beside(chosen, forward) {
            self.choose(next, cx);
        }
    }

    /// The sequence becomes format `index` (Landscape 16:9, Vertical 9:16,
    /// Square, Portrait): its segment pressed, or the registry's
    /// `sequence.setFormat.169`/`.916` (⌥1, ⌥2), which the workspace runs.
    /// Nothing while kerfd is connecting or gone: the bank shows disabled
    /// then, since a format chosen then would never reach kerfd.
    pub fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((_, _, width, height)) = FORMATS.get(index).copied() else {
            return;
        };
        if !self.session.read(cx).connected() {
            return;
        }
        self.session.update(cx, |session, cx| {
            let timeline = session.timeline();
            if timeline.width.unwrap_or(0) == width && timeline.height.unwrap_or(0) == height {
                return;
            }
            let mut op = json!({"op": "set-sequence", "width": width, "height": height});
            if let Some(sequence) = timeline.sequence.clone() {
                op["sequence"] = json!(sequence);
            }
            session.edit(vec![op]);
            cx.notify();
        });
    }
}

impl Render for FormatBank {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let timeline = session.timeline();
        let (width, height) = (timeline.width.unwrap_or(0), timeline.height.unwrap_or(0));
        if !session.has_project() || !session.state.has_snapshot || width == 0 || height == 0 {
            return div().into_any_element();
        }
        let focused = self.focus.is_focused(window);
        let noted = self.focus.clone();
        let chosen = format_index(width, height);
        // kerfd connecting or gone: the bank takes no presses and shows so.
        let enabled = session.connected();
        let custom: Option<SharedString> = chosen
            .is_none()
            .then(|| format!("Custom · {width} × {height}").into());
        let bank =
            div()
                .id("format-bank")
                .relative()
                .track_focus(&self.focus)
                .key_context(ui::CONTROL_SEGMENTED)
                // The pointer leaves the keys where they were: the ring is the keyboard's.
                .capture_any_mouse_down(|_, window, _| window.prevent_default())
                .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
                .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
                .on_action(cx.listener(|this, _: &ui::SegmentNext, _, cx| this.step(true, cx)))
                .on_action(cx.listener(|this, _: &ui::SegmentPrevious, _, cx| this.step(false, cx)))
                .flex()
                .flex_none()
                .p(px(space::S2))
                .gap(px(space::S2))
                .rounded(px(radius::CONTROL + 1.0))
                .bg(theme::raised())
                .children(FORMATS.iter().enumerate().map(|(index, format)| {
                    segment(index, *format, chosen == Some(index), enabled, cx)
                }))
                .when(focused, |bank| bank.child(ui::ring(radius::CONTROL + 1.0)))
                // Folded away to nothing (the toolbar too narrow), Tab passes it over.
                .child(
                    canvas(
                        move |bounds, window, _| ui::note_shown(&noted, bounds, window),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                );
        ui::tip_on(bank, custom.map(ui::Tip::new)).into_any_element()
    }
}

/// One of the bank's formats: its ratio, filled while chosen, its name and
/// size in its tip; pressed, the sequence takes it.
fn segment(
    index: usize,
    (name, ratio, w, h): (&'static str, &'static str, u32, u32),
    selected: bool,
    enabled: bool,
    cx: &Context<FormatBank>,
) -> impl IntoElement {
    let tip: SharedString = format!("{name} {ratio} · {w}×{h}").into();
    let segment = theme::type_style(
        div(),
        if selected {
            text::CONTROL_STRONG
        } else {
            text::CONTROL
        },
    )
    .id(("format", index))
    .px(px(space::S8))
    .h(px(height::H24))
    .flex()
    .items_center()
    .rounded(px(radius::SMALL + 1.0))
    .text_color(if !enabled {
        theme::dis()
    } else if selected {
        theme::ink()
    } else {
        theme::ink2()
    })
    .when(selected, |segment| {
        // Disabled, the chosen fill is a step off the
        // track, so which format it is still shows.
        segment.bg(if enabled {
            theme::on()
        } else {
            theme::mix(theme::raised(), theme::on(), 0.3)
        })
    })
    .when(!selected && enabled, |segment| {
        segment.hover(|style| style.text_color(theme::ink()))
    })
    // Over a segment, its own tip and not the bank's.
    .block_mouse_except_scroll()
    .child(ratio)
    .when(enabled, |segment| {
        segment
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.choose(index, cx)))
    });
    ui::tip_on(segment, Some(ui::Tip::new(tip)))
}
