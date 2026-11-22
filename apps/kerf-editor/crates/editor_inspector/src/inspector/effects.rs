//! The clip's effects: a card each, folded to its head or open on its
//! parameters, switched off or removed; and Add Effect…, the list of the
//! effects the clip takes, by category, to search and add one from.

use super::catalog::catalog;
use super::clip::{name_id, Field, Inspected, Target};
use super::sections::{note, section_head};
use super::values::same_words;
use super::{field_frame, InspectorView};
use editor_ui::icons::Icon;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::motion;
use editor_ui::theme::{self, height, radius, space, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, ClickEvent, Context, Div, Entity, Focusable,
    Stateful, Subscription, Window,
};
use kerf_bridge::event::ViewEffect;
use serde_json::json;

/// The list Add Effect… opens.
pub(super) struct Adding {
    search: Entity<TextInput>,
    opened: u64,
    _subscription: Subscription,
}

impl InspectorView {
    fn open_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = cx.new(|cx| TextInput::new(cx, "Search effects").with_style(text::CONTROL));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => cx.notify(),
                InputEvent::Submit => {
                    // Enter adds the first that matches.
                    let first = this.inspected(cx).and_then(|clip| {
                        let query = this.adding.as_ref()?.search.read(cx).text().to_owned();
                        let domain = clip.domain.clone()?;
                        catalog()
                            .effects(&domain, &query)
                            .first()
                            .map(|entry| (clip.id, entry.id.clone()))
                    });
                    if let Some((clip, fx)) = first {
                        this.add_effect(&clip, &fx, window, cx);
                    }
                }
                InputEvent::Cancel => this.close_adding(window, cx),
            },
        );
        search.read(cx).focus(window);
        self.addings_opened += 1;
        self.adding = Some(Adding {
            search,
            opened: self.addings_opened,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.adding.take().is_some() {
            window.blur();
        }
        cx.notify();
    }

    fn add_effect(&mut self, clip: &str, fx: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(vec![json!({"op": "fx-add", "clip": clip, "fx": fx})], cx);
        self.close_adding(window, cx);
    }

    /// The section: Add Effect…, and a card for each effect in the stack.
    pub(super) fn effects_section(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        clip.domain.as_deref()?;
        let add = ui::button("add-effect", "Add Effect…")
            .small()
            .ghost()
            .icon(Icon::Plus)
            .on_click(cx.listener(|this, _, window, cx| this.open_adding(window, cx)))
            .into_any_element();
        let mut section = div().flex().flex_col().child(section_head(
            "Effects".to_owned(),
            Some(clip.effects.len()),
            Some(add),
        ));
        if clip.effects.is_empty() {
            section = section.child(note("None yet.".to_owned()));
        }
        for (index, effect) in clip.effects.iter().enumerate() {
            section = section.child(self.effect_card(clip, index, effect, window, cx));
        }
        Some(section.into_any_element())
    }

    fn effect_card(
        &self,
        clip: &Inspected,
        index: usize,
        effect: &ViewEffect,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fx = effect.fx.as_str();
        let entry = catalog().fx.get(fx);
        let name = entry.map_or_else(|| fx.to_owned(), |entry| entry.name.clone());
        let enabled = effect.enabled;
        let folded = self.folded.contains(&index);
        let head = self.effect_head(clip, index, name, enabled, folded, cx);
        let mut card = div()
            .mx(px(space::S8))
            .mb(px(space::S6))
            .pb(px(if folded { 0.0 } else { space::S4 }))
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(theme::line())
            .bg(theme::card())
            .child(head);
        if folded {
            return card.into_any_element();
        }
        match entry {
            None => card = card.child(note(format!("“{fx}” is not in this version of Kerf."))),
            Some(entry) if entry.params.is_empty() => {
                card = card.child(note("Nothing to set.".to_owned()))
            }
            Some(entry) => {
                let values = &effect.params;
                for param in &entry.params {
                    let field = Field::new(Target::Effect(index), &param.name);
                    card = card.child(self.param_row(
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
        card.into_any_element()
    }

    /// An effect card's head: fold, its name (dim while off), its switch
    /// and Remove.
    fn effect_head(
        &self,
        clip: &Inspected,
        index: usize,
        name: String,
        enabled: bool,
        folded: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let clip_id = clip.id.clone();
        let toggle = move |on: bool, _: &mut Window, cx: &mut gpui::App| {
            let op = json!({"op": "fx-set", "clip": clip_id, "index": index, "enabled": on});
            this.update(cx, |view, cx| view.edit(vec![op], cx)).ok();
        };
        let clip_id = clip.id.clone();
        div()
            .flex()
            .items_center()
            .gap(px(space::S6))
            .h(px(height::H32))
            .pl(px(space::S4))
            .pr(px(space::S6))
            .child(
                ui::icon_button(
                    name_id(format!("fold-{index}")),
                    if folded {
                        Icon::ChevronRight
                    } else {
                        Icon::ChevronDown
                    },
                )
                .size(height::H20)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.folded.remove(&index) {
                        this.folded.insert(index);
                    }
                    cx.notify();
                })),
            )
            .child(
                ui::line(
                    name,
                    text::CONTROL_STRONG,
                    if enabled { theme::ink() } else { theme::ink3() },
                )
                .flex_1()
                .min_w_0(),
            )
            .child(ui::switch(
                format!("effect-on-{}-{index}", clip.id),
                enabled,
                toggle,
            ))
            .child(
                ui::icon_button(name_id(format!("remove-effect-{index}")), Icon::Trash)
                    .size(height::H24)
                    .tooltip("Remove")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // The cards after it move up a place.
                        this.folded.clear();
                        this.edit(
                            vec![json!({"op": "fx-remove", "clip": clip_id, "index": index})],
                            cx,
                        );
                    })),
            )
    }

    /// The effects a clip takes, by category, to add one.
    pub(super) fn adding_list(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let adding = self.adding.as_ref()?;
        let domain = clip.domain.clone()?;
        let query = adding.search.read(cx).text().to_owned();
        let focused = adding.search.focus_handle(cx).is_focused(window);
        let list = self.effect_entries(clip, &domain, &query, cx);
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .bg(theme::chrome())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S6))
                        .p(px(space::S8))
                        .child(Icon::Search.view(14.0, theme::ink3()))
                        .child(field_frame(&adding.search, focused, height::H28).flex_1())
                        .child(
                            ui::icon_button("close-effects", Icon::Close)
                                .size(height::H24)
                                .tooltip("Close")
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.close_adding(window, cx)
                                    }),
                                ),
                        ),
                )
                .child(ui::hairline(true))
                .child(list)
                // It fades in where it is; Esc or its close takes it away at once.
                .with_animation(
                    ("add-effect", adding.opened),
                    motion::animation(theme::motion::FADE),
                    |list, t| list.opacity(t),
                )
                .into_any_element(),
        )
    }

    /// The effects of `domain` whose name, id or category has `query` in
    /// it, under their categories, each added when pressed.
    fn effect_entries(
        &self,
        clip: &Inspected,
        domain: &str,
        query: &str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let found = catalog().effects(domain, query);
        let mut list = div()
            .id("effect-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .py(px(space::S4));
        if found.is_empty() {
            list = list.child(note("No effect matches.".to_owned()).py(px(space::S12)));
        }
        let mut category = None;
        for (index, entry) in found.into_iter().enumerate() {
            if category != Some(entry.category.as_str()) {
                category = Some(entry.category.as_str());
                let words = if entry.category.is_empty() {
                    "Other"
                } else {
                    entry.category.as_str()
                };
                list = list.child(
                    ui::label(words.to_owned(), text::META_STRONG, theme::ink3())
                        .px(px(space::S12))
                        .pt(px(space::S8))
                        .pb(px(space::S2)),
                );
            }
            let (clip_id, fx) = (clip.id.clone(), entry.id.clone());
            let blurb = entry.blurb();
            list = list.child(
                div()
                    .id(("effect-entry", index))
                    .px(px(space::S12))
                    .py(px(space::S4))
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::raised()))
                    .child(ui::line(entry.name.clone(), text::CONTROL, theme::ink()))
                    .when(!same_words(blurb, &entry.name), |row| {
                        row.child(ui::line(blurb.to_owned(), text::META, theme::ink3()))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.add_effect(&clip_id, &fx, window, cx)
                    })),
            );
        }
        list
    }
}
