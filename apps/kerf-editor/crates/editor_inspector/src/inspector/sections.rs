//! The Inspector's sections and their rows: the clip's head (its name,
//! kind and times), a generator's parameters and the clip's properties in
//! their groups, the caption and the masks; and a parameter's row, its
//! label with its doc, and its value as its kind edits it.

use super::catalog::{catalog, Kind, Param};
use super::clip::{held, name_id, Field, Held, Inspected, Target};
use super::fields::SWATCH;
use super::values::{humanize, swatch_color, value_words};
use super::{field_frame, InspectorView};
use editor_state::model::{EditParam, ViewParam};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, height, radius, space, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnyElement, ClickEvent, Context, ElementId, Focusable, MouseButton,
    MouseDownEvent, Window,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A row's label.
pub(super) const LABEL: f32 = 84.0;

/// The clip properties of a picture, in groups. What the catalog has that
/// no group names goes under More.
const VIDEO_GROUPS: &[Group] = &[
    (
        "Transform",
        &[
            "x",
            "y",
            "scale",
            "scale_x",
            "scale_y",
            "rotation",
            "skew",
            "skew_axis",
            "anchor_x",
            "anchor_y",
            "shutter",
        ],
        true,
    ),
    ("Compositing", &["opacity", "blend"], true),
    ("Crop", &["crop_l", "crop_r", "crop_t", "crop_b"], true),
    ("Border", &["border", "border_color"], false),
    (
        "3D",
        &[
            "three_d",
            "z",
            "depth",
            "bevel",
            "bevel_profile",
            "rotate_x",
            "rotate_y",
        ],
        false,
    ),
    ("Motion path", &["path", "along", "orient"], false),
];

/// A title's parameters, in groups.
const TITLE_GROUPS: &[Group] = &[
    (
        "",
        &["text", "font", "size", "weight", "style", "color", "align"],
        true,
    ),
    ("Outline", &["outline", "outline_width"], false),
    (
        "Reveal",
        &[
            "reveal",
            "reveal_by",
            "reveal_soft",
            "from_x",
            "from_y",
            "from_scale",
            "from_rotation",
            "from_opacity",
        ],
        false,
    ),
    (
        "Font axes",
        &["wght", "wdth", "slnt", "opsz", "ital", "features"],
        false,
    ),
];

/// A group's name ("" for none), its members, and whether it is open
/// before it is opened or closed by hand.
type Group = (&'static str, &'static [&'static str], bool);

/// `params` in `groups`, those no group names under More. With no groups,
/// all of them in one with no name.
fn grouped<'a>(params: &'a [Param], groups: &[Group]) -> Vec<(&'static str, bool, Vec<&'a Param>)> {
    let mut found: Vec<(&'static str, bool, Vec<&'a Param>)> = groups
        .iter()
        .map(|(name, members, open)| {
            let members = members
                .iter()
                .filter_map(|member| params.iter().find(|param| param.name == *member))
                .collect();
            (*name, *open, members)
        })
        .collect();
    let rest: Vec<&Param> = params
        .iter()
        .filter(|param| {
            !groups
                .iter()
                .any(|(_, members, _)| members.contains(&param.name.as_str()))
        })
        .collect();
    if !rest.is_empty() {
        found.push((
            if groups.is_empty() { "" } else { "More" },
            groups.is_empty(),
            rest,
        ));
    }
    found.retain(|(_, _, members)| !members.is_empty());
    found
}

impl InspectorView {
    /// The clip's head: its name to rename it, its kind and track, its
    /// times, and how many more clips are chosen.
    pub(super) fn head(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let focused = self.name.focus_handle(cx).is_focused(window);
        let timecode = |seconds: f64| editor_state::model::timecode(seconds, clip.fps);
        let times = [
            ("In", timecode(clip.start)),
            ("Out", timecode(clip.start + clip.duration)),
            ("Length", timecode(clip.duration)),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(space::S8))
            .p(px(space::S12))
            .child(field_frame(&self.name, focused, height::H28))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S6))
                    .min_w_0()
                    .child(ui::line(clip.kind.clone(), text::META, theme::ink2()).min_w_0())
                    .child(ui::label("·", text::META, theme::ink3()))
                    .child(ui::line(clip.track.clone(), text::META, theme::ink3()))
                    .when(clip.locked, |row| {
                        row.child(Icon::Lock.view(12.0, theme::ink3()))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(space::S8))
                    .children(times.into_iter().map(|(words, time)| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(ui::label(words, text::META, theme::ink3()))
                            .child(ui::line(time, text::TIMECODE, theme::ink()))
                    })),
            )
            .when(clip.others > 0, |head| {
                let more = if clip.others == 1 {
                    "1 more clip is chosen".to_owned()
                } else {
                    format!("{} more clips are chosen", clip.others)
                };
                head.child(ui::label(more, text::META, theme::ink3()))
            })
    }

    /// A generator's parameters, in their groups (a title's).
    pub(super) fn generator_section(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fx = clip.generator.as_deref()?;
        let entry = catalog().fx.get(fx);
        let params = entry
            .map(|entry| entry.params.as_slice())
            .unwrap_or_default();
        let groups = if fx == "title" { TITLE_GROUPS } else { &[] };
        let mut section = div()
            .flex()
            .flex_col()
            .child(section_head(humanize(fx), None, None));
        if params.is_empty() {
            let words = entry.map_or("Nothing to set.", |entry| entry.blurb());
            section = section.child(note(words.to_owned()));
        }
        let rows = self.group_rows(
            "generator",
            clip,
            Target::Clip,
            params,
            groups,
            &clip.props,
            window,
            cx,
        );
        Some(section.children(rows).into_any_element())
    }

    /// The clip's properties, in their groups (a picture's).
    pub(super) fn clip_section(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let domain = clip.domain.as_deref()?;
        let params = catalog().props.get(domain)?;
        let groups = if domain == "video" { VIDEO_GROUPS } else { &[] };
        let rows = self.group_rows(
            "clip",
            clip,
            Target::Clip,
            params,
            groups,
            &clip.props,
            window,
            cx,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .child(section_head("Clip".to_owned(), None, None))
                .children(rows)
                .into_any_element(),
        )
    }

    /// The rows of `params` in their groups, a group's head opening or
    /// closing it. A group is open at first when it says so or when any
    /// of its parameters is set.
    #[expect(
        clippy::too_many_arguments,
        reason = "each section hands over its own name, parameters and groups, and where they are set"
    )]
    fn group_rows(
        &self,
        section: &str,
        clip: &Inspected,
        target: Target,
        params: &[Param],
        groups: &[Group],
        values: &BTreeMap<String, ViewParam>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        for (name, open_at_first, members) in grouped(params, groups) {
            let key = format!("{section}/{name}");
            let any_set = members.iter().any(|param| {
                values
                    .get(&param.name)
                    .is_some_and(|value| !matches!(value, ViewParam::Value(Value::Null)))
            });
            let open = name.is_empty()
                || self
                    .groups
                    .get(&key)
                    .copied()
                    .unwrap_or(open_at_first || any_set);
            if !name.is_empty() {
                rows.push(self.group_head(name, key, open, cx));
            }
            if open {
                for param in members {
                    let field = Field::new(target.clone(), &param.name);
                    rows.push(self.param_row(
                        clip,
                        &field,
                        param,
                        values.get(&param.name),
                        window,
                        cx,
                    ));
                }
            }
        }
        rows
    }

    fn group_head(
        &self,
        name: &str,
        key: String,
        open: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(name_id(format!("group-{key}")))
            .flex()
            .items_center()
            .gap(px(space::S4))
            .h(px(height::H24))
            .px(px(space::S8))
            .cursor_pointer()
            .text_color(theme::ink3())
            .hover(|style| style.text_color(theme::ink2()))
            .child(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                }
                .view(12.0, theme::ink3()),
            )
            .child(theme::type_style(div(), text::META_STRONG).child(name.to_owned()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.groups.insert(key.clone(), !open);
                cx.notify();
            }))
            .into_any_element()
    }

    /// A parameter's row: its label, and its value as its kind edits it;
    /// keyframed, only said.
    pub(super) fn param_row(
        &self,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        set: Option<&ViewParam>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let held = held(param, set, clip.at);
        let clip_level = field.target == Target::Clip;
        let label = self.row_label(clip, field, param, &held, cx);
        let row = div()
            .flex()
            .items_center()
            .gap(px(space::S8))
            .min_h(px(height::H28))
            .px(px(space::S12))
            .child(label);
        if held.keyed {
            let words = value_words(param, &held.value, clip_level);
            return row
                .child(keyed(words, param.kind == Kind::Color))
                .into_any_element();
        }
        match param.kind {
            Kind::Number => match held.value.as_f64() {
                Some(value) => {
                    self.number_row(row, clip, field, param, value, held.changed, window, cx)
                }
                None => row
                    .child(ui::line(
                        value_words(param, &held.value, clip_level),
                        text::CONTROL,
                        theme::ink2(),
                    ))
                    .into_any_element(),
            },
            Kind::Bool => {
                let on = held.value.as_bool().unwrap_or(false);
                let this = cx.entity().downgrade();
                let (clip_id, key) = (clip.id.clone(), field.clone());
                row.child(div().flex_1())
                    .child(ui::switch(
                        format!("switch-{}-{:?}-{}", clip.id, field.target, field.name),
                        on,
                        move |on, _, cx| {
                            this.update(cx, |view, cx| {
                                view.set_param(&clip_id, &key, EditParam::Value(json!(on)), cx)
                            })
                            .ok();
                        },
                    ))
                    .into_any_element()
            }
            Kind::Choice => self.choice_row(
                row,
                clip,
                field,
                param,
                held.value.as_str().unwrap_or_default(),
                window,
                cx,
            ),
            Kind::Color => self.color_row(
                row,
                clip,
                field,
                held.value.as_str().unwrap_or("#000000"),
                window,
                cx,
            ),
            Kind::Text => match self.texts.get(field) {
                Some(entry) => {
                    let focused = entry.input.focus_handle(cx).is_focused(window);
                    row.child(field_frame(&entry.input, focused, height::H24).flex_1())
                        .into_any_element()
                }
                None => row
                    .child(ui::line(
                        value_words(param, &held.value, clip_level),
                        text::CONTROL,
                        theme::ink2(),
                    ))
                    .into_any_element(),
            },
        }
    }

    /// A row's label: its doc on rest, and a double press gives back its
    /// default when it is set.
    fn row_label(
        &self,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        held: &Held,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let resettable = held.changed && !held.keyed;
        let mut tip = param.doc.trim().to_owned();
        if resettable {
            if !tip.is_empty() {
                tip.push('\n');
            }
            tip.push_str("Double-click to reset.");
        }
        let (clip_id, key) = (clip.id.clone(), field.clone());
        let label = div()
            .id(field.id("label"))
            .w(px(LABEL))
            .flex_none()
            .min_w_0()
            .child(ui::line(
                humanize(&param.name),
                text::CONTROL,
                theme::ink2(),
            ))
            .when(resettable, |label| {
                label.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        if event.click_count >= 2 {
                            this.set_param(&clip_id, &key, EditParam::Default, cx);
                        }
                    }),
                )
            });
        ui::tip_on(label, (!tip.is_empty()).then(|| ui::Tip::new(tip))).into_any_element()
    }

    /// The clip's masks, a row each; a press chooses one, here and on the
    /// picture (`Session::chosen_mask`), and the one chosen there is
    /// chosen here.
    pub(super) fn masks_section(
        &self,
        clip: &Inspected,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if clip.masks.is_empty() {
            return None;
        }
        let chosen = self.session.read(cx).chosen_mask.clone();
        let rows = clip.masks.iter().enumerate().map(|(index, mask)| {
            let name = mask.name.clone();
            let is_chosen = chosen.as_deref() == Some(name.as_str());
            let choose = name.clone();
            let mode = humanize(if mask.mode.is_empty() {
                "add"
            } else {
                mask.mode.as_str()
            });
            let outline = &mask.outline;
            let shape = if outline.rect.is_some() {
                "Rectangle"
            } else if outline.ellipse.is_some() {
                "Ellipse"
            } else if outline.points.is_some() {
                "Path"
            } else {
                "Shape"
            };
            div()
                .id(ElementId::NamedInteger(
                    "inspector-mask".into(),
                    index as u64,
                ))
                .flex()
                .items_center()
                .gap(px(space::S8))
                .h(px(height::H28))
                .px(px(space::S12))
                .cursor_pointer()
                .when(is_chosen, |row| row.bg(theme::on()))
                .when(!is_chosen, |row| {
                    row.hover(|style| style.bg(theme::raised()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let name = Some(choose.clone());
                    this.session.update(cx, |session, cx| {
                        if session.chosen_mask != name {
                            session.chosen_mask = name;
                            cx.notify();
                        }
                    });
                }))
                .child(
                    ui::line(name, text::TIMECODE, theme::ink())
                        .w(px(LABEL))
                        .flex_none(),
                )
                .child(
                    ui::line(format!("{shape} · {mode}"), text::CONTROL, theme::ink2())
                        .flex_1()
                        .min_w_0(),
                )
                .when(mask.inverted, |row| {
                    row.child(ui::chip("Inverted", theme::ink2()))
                })
        });
        Some(
            div()
                .flex()
                .flex_col()
                .child(section_head(
                    "Masks".to_owned(),
                    Some(clip.masks.len()),
                    None,
                ))
                .children(rows)
                .into_any_element(),
        )
    }

    /// A caption's words.
    pub(super) fn caption_section(&self, clip: &Inspected) -> Option<AnyElement> {
        let words = clip.caption.clone()?;
        Some(
            div()
                .flex()
                .flex_col()
                .child(section_head("Caption".to_owned(), None, None))
                .child(
                    ui::label(words, text::BODY, theme::ink())
                        .px(px(space::S12))
                        .pb(px(space::S8)),
                )
                .into_any_element(),
        )
    }
}

/// A section's head: its name, how many it has, and what it offers.
pub(super) fn section_head(
    title: String,
    count: Option<usize>,
    trailing: Option<AnyElement>,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::S6))
        .h(px(height::H32))
        .pl(px(space::S12))
        .pr(px(space::S8))
        .mt(px(space::S4))
        .child(ui::label(title, text::CONTROL_STRONG, theme::ink()))
        .when_some(count, |head, count| {
            head.child(ui::label(count.to_string(), text::META, theme::ink3()))
        })
        .child(div().flex_1())
        .children(trailing)
}

/// A line of words in a section, in ink3.
pub(super) fn note(words: String) -> gpui::Div {
    ui::label(words, text::META, theme::ink3())
        .px(px(space::S12))
        .py(px(space::S4))
}

/// A keyframed value: it is set on the timeline's keys, not here.
fn keyed(words: String, colour: bool) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(space::S6))
        .child(
            theme::type_style(div(), text::META)
                .flex_none()
                .h(px(height::H20 - 2.0))
                .px(px(space::S6))
                .flex()
                .items_center()
                .rounded(px(radius::SMALL))
                .bg(theme::accent_wash())
                .text_color(theme::accent_ink())
                .child("Keyframed"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::S6))
                .min_w_0()
                .when(colour, |value| {
                    value.child(
                        div()
                            .size(px(SWATCH - 4.0))
                            .flex_none()
                            .rounded(px(radius::SMALL))
                            .border_1()
                            .border_color(theme::line2())
                            .bg(swatch_color(&words)),
                    )
                })
                .child(ui::line(words, text::TIMECODE, theme::ink2())),
        )
}
