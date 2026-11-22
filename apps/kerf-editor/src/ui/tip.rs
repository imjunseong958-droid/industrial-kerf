//! The kit's tips: what one says, how big its card is and where it stands,
//! when the next shows at once, and how it is drawn over everything; and the
//! tip on what is not a control of the kit (`tipped`, `tip_on`, `tip_zone`).

use super::{control, hovered, label, Control};
use crate::motion;
use crate::theme::{self, layout, motion as scene, radius, text, TypeStyle};
use gpui::{
    canvas, div, point, prelude::*, px, size, AnyElement, AnyTooltip, App, Bounds, Div, ElementId,
    Entity, Hsla, Pixels, Point, SharedString, Size, Stateful, TextRun, Window,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

/// What a control's tip says: its name (or why it cannot be pressed now),
/// and its key, set after the words in mono and dimmer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tip {
    pub words: SharedString,
    pub key: Option<SharedString>,
}

impl Tip {
    pub fn new(words: impl Into<SharedString>) -> Self {
        Self {
            words: words.into(),
            key: None,
        }
    }

    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The key, if there is one.
    pub fn maybe_key(mut self, key: Option<impl Into<SharedString>>) -> Self {
        self.key = key.map(Into::into);
        self
    }

    /// Its words line by line (a tile's says its name, its file and what a
    /// double-click does, a line each); its key keeps to one line.
    fn lines(&self) -> Vec<SharedString> {
        if !self.words.contains('\n') {
            return vec![self.words.clone()];
        }
        self.words
            .split('\n')
            .map(|line| SharedString::from(line.to_owned()))
            .collect()
    }

    fn flat_key(&self) -> Option<SharedString> {
        self.key.as_ref().map(|key| {
            if key.contains('\n') {
                key.replace('\n', " ").into()
            } else {
                key.clone()
            }
        })
    }
}

/// The tip a control shows: while it can be pressed, its name; greyed,
/// why not (else its name). Its key either way.
pub fn tip_for(
    name: Option<&SharedString>,
    key: Option<&SharedString>,
    disabled: bool,
    reason: Option<&SharedString>,
) -> Option<Tip> {
    let words = if disabled { reason.or(name) } else { name }?;
    Some(Tip::new(words.clone()).maybe_key(key.cloned()))
}

/// The card a tip is drawn on, `width` wide (or as wide as its words): one
/// line, its key after its words; or its lines one under another (wrapped
/// past `TIP_MOST`), the key after the first.
fn tip_card(tip: &Tip, width: Option<f32>) -> Div {
    let card = theme::type_style(div(), text::CONTROL)
        .flex_none()
        .px(px(theme::space::S8))
        .when_some(width, |card, width| card.w(px(width)))
        .rounded(px(radius::CONTROL))
        .bg(theme::card())
        .border_1()
        .border_color(theme::line2())
        .text_color(theme::ink())
        .shadow_md();
    let key = tip
        .flat_key()
        .map(|key| label(key, text::MONO_SMALL, theme::ink3()).flex_none());
    let lines = tip.lines();
    let wraps = width.is_some_and(|width| width >= layout::TIP_MOST);
    if lines.len() == 1 && !wraps {
        return card
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .h(px(layout::TIP_HEIGHT))
            .whitespace_nowrap()
            .child(tip.words.clone())
            .children(key);
    }
    let pad = (layout::TIP_HEIGHT - text::CONTROL.line) / 2.0;
    let mut lines = lines.into_iter();
    let first = div()
        .flex()
        .items_start()
        .gap(px(theme::space::S8))
        .child(div().flex_1().min_w_0().children(lines.next()))
        .children(key);
    card.flex()
        .flex_col()
        .py(px(pad))
        .child(first)
        .children(lines.map(|line| div().child(line)))
}

/// How wide `words` are set in `style`.
fn measure(words: &SharedString, style: TypeStyle, window: &Window) -> f32 {
    if words.is_empty() {
        return 0.0;
    }
    let font = if style.mono {
        theme::mono_font(style.weight)
    } else {
        theme::ui_font(style.weight)
    };
    let run = TextRun {
        len: words.len(),
        font,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(words.clone(), px(style.px), &[run], None);
    f32::from(line.width).ceil()
}

/// How many rows `words` take set in `style` within `wide`.
fn rows_within(words: &SharedString, style: TypeStyle, wide: f32, window: &Window) -> usize {
    let font = if style.mono {
        theme::mono_font(style.weight)
    } else {
        theme::ui_font(style.weight)
    };
    let run = TextRun {
        len: words.len(),
        font,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_text(words.clone(), px(style.px), &[run], Some(px(wide)), None)
        .ok()
        .and_then(|lines| lines.first().map(|line| line.wrap_boundaries().len() + 1))
        .unwrap_or(1)
}

/// How big `tip`'s card is: as wide as its widest line (at most
/// `TIP_MOST`, wrapping past it), a row of `CONTROL` type for each line.
fn tip_size(tip: &Tip, window: &Window) -> Size<Pixels> {
    let pad = theme::space::S8 * 2.0 + 2.0;
    let key = tip.flat_key().map_or(0.0, |key| {
        theme::space::S8 + measure(&key, text::MONO_SMALL, window)
    });
    let lines = tip.lines();
    let widths: Vec<f32> = lines
        .iter()
        .enumerate()
        .map(|(at, line)| measure(line, text::CONTROL, window) + if at == 0 { key } else { 0.0 })
        .collect();
    let widest = widths.iter().copied().fold(0.0, f32::max);
    let width = (pad + widest).min(layout::TIP_MOST);
    if lines.len() == 1 && pad + widest <= layout::TIP_MOST {
        return size(px(width), px(layout::TIP_HEIGHT));
    }
    let inner = width - pad;
    let rows: usize = lines
        .iter()
        .zip(&widths)
        .enumerate()
        .map(|(at, (line, wide))| {
            if *wide <= inner {
                1
            } else {
                let room = if at == 0 { inner - key } else { inner };
                rows_within(line, text::CONTROL, room.max(1.0), window)
            }
        })
        .sum();
    size(px(width), px(tip_height(rows)))
}

/// How tall a tip of `rows` rows is: `TIP_HEIGHT` for one, a row of
/// `CONTROL` type more for each after it.
pub fn tip_height(rows: usize) -> f32 {
    layout::TIP_HEIGHT + text::CONTROL.line * rows.saturating_sub(1) as f32
}

/// Where a tip `tip` big goes for a control at `control` in a window
/// `window` big: centred on the control, `TIP_EDGE` in
/// from the window's sides at least, `TIP_GAP` under it, or over it when
/// the control is low in the window.
pub fn tip_origin(
    control: Bounds<Pixels>,
    tip: Size<Pixels>,
    window: Size<Pixels>,
) -> Point<Pixels> {
    let (width, edge) = (f32::from(tip.width), layout::TIP_EDGE);
    let centred =
        (f32::from(control.left()) + f32::from(control.size.width) / 2.0 - width / 2.0).round();
    let left = centred
        .min(f32::from(window.width) - edge - width)
        .max(edge);
    let tall = f32::from(tip.height);
    let over = f32::from(control.top()) - layout::TIP_GAP - tall;
    let under = f32::from(control.bottom()) + layout::TIP_GAP;
    // Low in the window it goes over its control, as it does where one of
    // several lines would run past the window's foot and there is room over.
    let above = (f32::from(control.bottom()) > layout::TIP_FLIP_BELOW
        && f32::from(control.top()) > layout::TIP_FLIP_ABOVE)
        || (under + tall > f32::from(window.height) - edge && over >= edge);
    let top = if above {
        f32::from(control.top()) - layout::TIP_GAP - f32::from(tip.height)
    } else {
        f32::from(control.bottom()) + layout::TIP_GAP
    };
    point(px(left), px(top.round()))
}

/// What a tip stands by for a control at `control`, the pointer at
/// `pointer`, in a window `window` big: the control; or, for one as tall as
/// half the window or more (the sidebar's edge), the row of it the
/// pointer is on, so the tip shows by the pointer rather than off the
/// window's foot.
pub fn tip_anchor(
    control: Bounds<Pixels>,
    pointer: Point<Pixels>,
    window: Size<Pixels>,
) -> Bounds<Pixels> {
    if control.size.height * 2.0 < window.height {
        return control;
    }
    let tall = px(layout::TIP_HEIGHT);
    let top = (pointer.y - tall / 2.0)
        .max(control.top())
        .min(control.bottom() - tall);
    Bounds::new(point(control.left(), top), size(control.size.width, tall))
}

/// Whether a tip asked for now shows at once: one shows still
/// (`showing`), or the last went less than `TIP_QUICK` before `now`.
pub fn tip_is_quick(showing: usize, last_hidden: Option<Instant>, now: Instant) -> bool {
    showing > 0
        || last_hidden.is_some_and(|at| now.saturating_duration_since(at) < scene::TIP_QUICK)
}

thread_local! {
    /// How many tips show now, and when the last one went: one warm-up for
    /// the whole app, shared by the kit's tips and GPUI's own, so the next
    /// tip after any other shows at once.
    static TIPS: Cell<(usize, Option<Instant>)> = const { Cell::new((0, None)) };
}

/// A tip shows: the next asked for shows at once.
pub(super) fn tip_shown() {
    TIPS.with(|tips| {
        let (showing, last) = tips.get();
        tips.set((showing + 1, last));
    });
}

/// A tip went at `now`: the next asked for within `TIP_QUICK` shows at once.
pub(super) fn tip_gone(now: Instant) {
    TIPS.with(|tips| {
        let (showing, _) = tips.get();
        tips.set((showing.saturating_sub(1), Some(now)));
    });
}

/// Whether a tip asked for now shows at once, as the app's tips stand.
pub(super) fn tips_are_warm(now: Instant) -> bool {
    let (showing, last) = TIPS.with(Cell::get);
    tip_is_quick(showing, last, now)
}

/// How far a fade begun `since` has got, 0..1, eased; none under Reduce Motion.
fn faded(since: Instant, now: Instant) -> f32 {
    let duration = motion::duration(scene::FADE);
    if duration.is_zero() {
        return 1.0;
    }
    let t = now.saturating_duration_since(since).as_secs_f32() / duration.as_secs_f32();
    motion::ease(t.clamp(0.0, 1.0))
}

/// The tip over everything, while it shows or fades out. It is drawn as
/// the window draws GPUI's own tooltips, after everything else (menus and
/// panels drawn on top of the window among them), so a control inside a
/// menu or a popover has its tip over it too; it stands where
/// [`tip_origin`] puts it, not by the pointer.
pub(super) fn tip_layer(
    state: &Entity<Control>,
    tip: &Tip,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let now = Instant::now();
    let (shown, hiding, bounds) = {
        let control = state.read(cx);
        (control.tip.shown, control.tip.hiding, control.tip.bounds)
    };
    let opacity = match (shown, hiding) {
        (Some((since, true)), _) => faded(since, now),
        (Some((_, false)), _) => 1.0,
        (None, Some(since)) => {
            let gone = faded(since, now);
            // The window draws one tip: one going gives way at once to
            // one that shows (the pointer went on to another control).
            // Which gives way is settled as the window draws, when every
            // tip that went in one move of the pointer has gone.
            let another = TIPS.with(Cell::get).0 > 0;
            if gone >= 1.0 || another {
                state.update(cx, |control, _| control.tip.hiding = None);
                return None;
            }
            1.0 - gone
        }
        (None, None) => return None,
    };
    if opacity < 1.0 || hiding.is_some() {
        window.request_animation_frame();
    }
    let tip_size = tip_size(tip, window);
    let origin = tip_origin(
        tip_anchor(bounds?, window.mouse_position(), window.viewport_size()),
        tip_size,
        window.viewport_size(),
    );
    let tip = tip.clone();
    let width = f32::from(tip_size.width);
    Some(
        canvas(
            move |_, window, cx| {
                let view = cx.new(|_| TipCard {
                    tip,
                    width,
                    opacity,
                });
                // The window sets a tooltip one point past where it is told.
                window.set_tooltip(AnyTooltip {
                    view: view.into(),
                    mouse_position: origin - point(px(1.0), px(1.0)),
                    check_visible_and_update: Rc::new(|_, _, _| true),
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
        .into_any_element(),
    )
}

/// A kit's tip as the window draws it over everything: its card, `width`
/// wide, at `opacity`.
struct TipCard {
    tip: Tip,
    width: f32,
    opacity: f32,
}

impl Render for TipCard {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        tip_card(&self.tip, Some(self.width)).opacity(self.opacity)
    }
}

/// `child` with a tip: its name and key, shown as a control of the kit's
/// is (after a rest, or at once after another; fading). The toolbar's
/// buttons that are not of the kit, and rows that say more on a rest, use it.
pub fn tipped(id: impl Into<ElementId>, tip: Tip, child: impl IntoElement) -> Tipped {
    Tipped {
        id: id.into(),
        tip,
        child: child.into_any_element(),
    }
}

#[derive(IntoElement)]
pub struct Tipped {
    id: ElementId,
    tip: Tip,
    child: AnyElement,
}

impl RenderOnce for Tipped {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = control(&self.id, window, cx);
        let element = div()
            .id(self.id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .child(self.child);
        hovered(element, &state, Some(self.tip), None, window, cx)
    }
}

/// `element` itself with the kit's tip `tip` (none: no tip), shown as a
/// control's is: after a rest, or at once after another tip; fading in and
/// out; its key in mono; gone when a panel opens over it. Its words are
/// read each time it is drawn, so a tip that shows says what the element
/// says now. In place of GPUI's own `tooltip`, with no box round it: the
/// element keeps its own place and size. It takes the element's hover
/// (`on_hover`), which must have none of its own.
pub fn tip_on(element: Stateful<Div>, tip: Option<Tip>) -> TipOn {
    TipOn { element, tip }
}

#[derive(IntoElement)]
pub struct TipOn {
    element: Stateful<Div>,
    tip: Option<Tip>,
}

impl RenderOnce for TipOn {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut element = self.element;
        let Some(tip) = self.tip else {
            return element;
        };
        let Some(id) = element.interactivity().element_id.clone() else {
            return element;
        };
        let state = control(&id, window, cx);
        hovered(element, &state, Some(tip), None, window, cx)
    }
}

/// The kit's tip `tip` over the whole of the element it is a child of (it
/// fills it, `absolute`, and takes no press from it), shown as
/// [`tip_on`]'s is. For an element that keeps a hover of its own, or whose
/// end is far from where its tip is known. None: no tip.
pub fn tip_zone(id: impl Into<ElementId>, tip: Option<Tip>) -> TipZone {
    TipZone { id: id.into(), tip }
}

#[derive(IntoElement)]
pub struct TipZone {
    id: ElementId,
    tip: Option<Tip>,
}

impl RenderOnce for TipZone {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let zone = div()
            .id(self.id.clone())
            .absolute()
            .top_0()
            .left_0()
            .size_full();
        let Some(tip) = self.tip else {
            return zone;
        };
        let state = control(&self.id, window, cx);
        hovered(zone, &state, Some(tip), None, window, cx)
    }
}
