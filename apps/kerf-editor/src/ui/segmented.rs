//! A segmented control: one of a few chosen, its fill sliding to the segment
//! chosen, the arrows choosing while it has the keys.

use super::{control, hovered, keyed, SegmentNext, SegmentPrevious, Tip, CONTROL_SEGMENTED};
use crate::motion::Tween;
use crate::theme::{self, motion as scene, radius, text};
use gpui::{
    div, prelude::*, px, App, Component, Div, ElementId, Entity, SharedString, Stateful, Window,
};
use std::rc::Rc;

/// A segmented control: one of a few, the chosen one filled. Choosing
/// another slides the fill to it over `SEGMENT` (at once under Reduce
/// Motion, and in place when it first shows). Tab reaches it, and the
/// arrows choose. Each segment can say more in a tip ([`Segmented::tips`])
/// and be greyed when it cannot be chosen ([`Segmented::disabled`]).
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<SharedString>,
    options: &[(T, &'static str)],
    chosen: T,
    choose: impl Fn(T, &mut Window, &mut App) + 'static,
) -> Segmented<T> {
    Segmented {
        id: id.into(),
        options: options
            .iter()
            .map(|(value, words)| (*value, SharedString::from(*words)))
            .collect(),
        chosen,
        choose: Rc::new(choose),
        tips: Vec::new(),
        disabled: Vec::new(),
        focusable: true,
    }
}

/// What choosing a segment of a [`Segmented`] does, given its value.
type ChooseHandler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub struct Segmented<T> {
    id: SharedString,
    options: Vec<(T, SharedString)>,
    chosen: T,
    choose: ChooseHandler<T>,
    tips: Vec<Option<SharedString>>,
    disabled: Vec<T>,
    focusable: bool,
}

impl<T: Copy + PartialEq + 'static> Segmented<T> {
    /// What each segment's tip says, in the options' order (Settings'
    /// permission segments carry their summaries).
    pub fn tips(mut self, tips: &[&str]) -> Self {
        self.tips = tips
            .iter()
            .map(|tip| (!tip.is_empty()).then(|| SharedString::from(tip.to_string())))
            .collect();
        self
    }

    /// Greys `value`'s segment: it cannot be chosen now.
    pub fn disabled(mut self, value: T) -> Self {
        self.disabled.push(value);
        self
    }

    /// Whether Tab reaches it (it does unless a view keeps its own stops).
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }
}

impl<T: Copy + PartialEq + 'static> IntoElement for Segmented<T> {
    type Element = Component<Self>;

    fn into_element(self) -> Self::Element {
        Component::new(self)
    }
}

/// The step from segment `from` to the next one that can be chosen,
/// forward or back; none past either end.
pub fn segment_step(enabled: &[bool], from: usize, forward: bool) -> Option<usize> {
    if forward {
        (from + 1..enabled.len()).find(|at| enabled[*at])
    } else {
        (0..from.min(enabled.len())).rev().find(|at| enabled[*at])
    }
}

/// Where a segmented control's fill is: on the segment chosen when it
/// last stood still, sliding from where it was to a newly chosen one.
struct Fill {
    chosen: Option<usize>,
    /// Its span (left, width) as it began to slide, and its slide.
    from: Option<(f32, f32)>,
    slide: Tween,
    /// Each segment's span, from the track's inner left, as laid out last.
    spans: Vec<(f32, f32)>,
}

impl Fill {
    /// Its span now, `t` of the way from where it began to the chosen one.
    fn span(&self, t: f32) -> Option<(f32, f32)> {
        let to = *self.spans.get(self.chosen?)?;
        let from = self.from.unwrap_or(to);
        Some((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t))
    }
}

/// Inside the track: its padding, the segments' height.
const TRACK_PAD: f32 = 2.0;
const SEGMENT_TALL: f32 = 24.0;

impl<T: Copy + PartialEq + 'static> Segmented<T> {
    /// The track: the fill while it slides (`span`, from the track's inner
    /// left), then each segment, filled itself when chosen while the fill
    /// stands still. Each segment's span goes into `fill` as it is laid out.
    fn track(
        &self,
        fill: &Entity<Fill>,
        chosen: Option<usize>,
        enabled: &[bool],
        span: Option<(f32, f32)>,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let slid = span.is_some();
        let count = self.options.len();
        let spans_into = fill.clone();
        let mut track = div()
            .relative()
            .flex()
            .flex_none()
            .p(px(TRACK_PAD))
            .rounded(px(radius::CONTROL + 1.0))
            .bg(theme::raised())
            .on_children_prepainted(move |bounds, _, cx| {
                // The segments come after the fill when it slides.
                let segments = &bounds[bounds.len().saturating_sub(count)..];
                let Some(first) = segments.first() else {
                    return;
                };
                let left = f32::from(first.left());
                let spans = segments
                    .iter()
                    .map(|b| (f32::from(b.left()) - left, f32::from(b.size.width)))
                    .collect();
                spans_into.update(cx, |fill, _| fill.spans = spans);
            });
        if let Some((left, width)) = span {
            track = track.child(
                div()
                    .absolute()
                    .top(px(TRACK_PAD))
                    .left(px(TRACK_PAD + left))
                    .w(px(width))
                    .h(px(SEGMENT_TALL))
                    .rounded(px(radius::SMALL + 1.0))
                    .bg(theme::on()),
            );
        }
        for index in 0..count {
            track = track.child(self.segment(index, chosen, enabled, slid, window, cx));
        }
        track
    }

    /// Segment `index`: its words, in the stronger type while it is chosen
    /// (filled too while the fill is not sliding, `slid`), its tip, and a
    /// press that chooses it while it can be chosen.
    fn segment(
        &self,
        index: usize,
        chosen: Option<usize>,
        enabled: &[bool],
        slid: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let (value, words) = self.options[index].clone();
        let selected = Some(index) == chosen;
        let can = enabled[index];
        let choose = self.choose.clone();
        let segment_id = ElementId::Name(format!("{}-{index}", self.id).into());
        let segment_state = control(&segment_id, window, cx);
        let ink = if !can {
            theme::dis()
        } else if selected {
            theme::ink()
        } else {
            theme::ink2()
        };
        let segment = theme::type_style(
            div(),
            if selected {
                text::CONTROL_STRONG
            } else {
                text::CONTROL
            },
        )
        .id(segment_id)
        .relative()
        .px(px(10.0))
        .h(px(SEGMENT_TALL))
        .flex()
        .flex_col()
        .justify_center()
        .items_center()
        .rounded(px(radius::SMALL + 1.0))
        .text_color(ink)
        .when(selected && !slid, |segment| segment.bg(theme::on()))
        .child(words.clone())
        // As wide as its words are when chosen, so choosing does not
        // change its width.
        .child(
            theme::type_style(div(), text::CONTROL_STRONG)
                .h(px(0.0))
                .overflow_hidden()
                .child(words),
        )
        .when(can, |segment| {
            segment
                .cursor_pointer()
                .when(!selected, |segment| {
                    segment.hover(|style| style.text_color(theme::ink()))
                })
                .on_click(move |_, window, cx| choose(value, window, cx))
        });
        let tip = self.tips.get(index).cloned().flatten().map(Tip::new);
        hovered(segment, &segment_state, tip, None, window, cx)
    }
}

impl<T: Copy + PartialEq + 'static> RenderOnce for Segmented<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(self.id.clone());
        let state = control(&id, window, cx);
        let chosen = self
            .options
            .iter()
            .position(|(value, _)| *value == self.chosen);
        let enabled: Rc<Vec<bool>> = Rc::new(
            self.options
                .iter()
                .map(|(value, _)| !self.disabled.contains(value))
                .collect(),
        );
        let fill = window.use_keyed_state(id.clone(), cx, |_, _| Fill {
            chosen,
            from: None,
            slide: Tween::at(1.0),
            spans: Vec::new(),
        });
        // A new choice slides the fill from where it is now.
        let slide = fill.update(cx, |fill, _| {
            if fill.chosen != chosen {
                let t = fill.slide.value();
                fill.from = fill.span(t);
                fill.chosen = chosen;
                fill.slide = Tween::at(0.0);
                fill.slide.go(1.0, scene::SEGMENT);
            }
            fill.slide.clone()
        });
        let t = slide.sample(window);
        let span = if slide.running() {
            fill.read(cx).span(t)
        } else {
            None
        };
        let track = self.track(&fill, chosen, &enabled, span, window, cx);
        let (forward, back) = (self.choose.clone(), self.choose.clone());
        let values: Rc<Vec<T>> = Rc::new(self.options.iter().map(|(value, _)| *value).collect());
        let (forward_values, back_values) = (values.clone(), values);
        let (forward_enabled, back_enabled) = (enabled.clone(), enabled);
        let from = chosen.unwrap_or(0);
        let element = div()
            .id(id)
            .flex_none()
            .rounded(px(radius::CONTROL + 1.0))
            .child(track)
            .on_action(move |_: &SegmentNext, window, cx| {
                let next = if chosen.is_none() {
                    forward_enabled.iter().position(|can| *can)
                } else {
                    segment_step(&forward_enabled, from, true)
                };
                if let Some(next) = next {
                    forward(forward_values[next], window, cx);
                }
            })
            .on_action(move |_: &SegmentPrevious, window, cx| {
                if let Some(next) = segment_step(&back_enabled, from, false) {
                    back(back_values[next], window, cx);
                }
            });
        keyed(
            element,
            &state,
            CONTROL_SEGMENTED,
            self.focusable,
            false,
            radius::CONTROL + 1.0,
            false,
            window,
            cx,
            None,
        )
    }
}
