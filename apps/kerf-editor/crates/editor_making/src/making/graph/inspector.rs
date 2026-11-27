//! The column at the graph's right: the node chosen and what it holds,
//! set in its fields (a model's prompt, seed and knobs, a guide's Draws),
//! with what reads into it; or, with none chosen, the shot itself and its
//! takes.

use super::cards::{gives_color, kind_icon};
use super::fields::{field_box, field_row, ADD, FIELD_LABEL};
use super::layout::INSPECTOR;
use super::pictures::picture_box;
use super::{MenuKind, ShotGraph};
use crate::making::plan::notes_block;
use crate::making::{gives_word, GraphNode, Recipe};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, text};
use editor_ui::ui::{self, icon_button};
use gpui::{div, point, prelude::*, px, AnyElement, Context, ElementId, MouseDownEvent, Rgba};
use kerf_bridge::event::graph::{Gives, Kind, What};
use kerf_bridge::{RecipeEdit, ShotOf};

/// What a guide draws, and its name.
const PASSES: [(&str, &str); 6] = [
    ("picture", "Picture"),
    ("depth", "Depth"),
    ("relative", "Relative depth"),
    ("ids", "Ids"),
    ("normals", "Normals"),
    ("motion", "Motion"),
];

impl ShotGraph {
    /// The column at the right: the chosen node and its settings, or the shot.
    pub(super) fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .p(px(14.0));
        let session = self.session.read(cx);
        let making = self.making(session).is_some();
        let connected = session.state.connection == editor_state::store::Connection::Connected;
        let node = self
            .selected
            .as_ref()
            .and_then(|id| self.shown()?.node(id))
            .cloned();
        if let Some(said) = &self.said {
            column = column.child(ui::label(said.clone(), text::CONTROL, theme::neg()));
        }
        match (self.shown().cloned(), node) {
            (None, _) => {
                column = column.child(ui::label(self.media.to_string(), text::TITLE, theme::ink()));
            }
            (Some(recipe), None) => {
                column = column.child(self.shot_column(&recipe, making, connected, cx));
            }
            (Some(graph), Some(node)) => {
                let color = gives_color(node.gives);
                let tag = gives_word(node.gives).to_owned();
                column = column.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S8))
                        .child(kind_icon(node.kind()).view(16.0, color))
                        .child(
                            ui::line(node.title.clone(), text::TITLE, theme::ink())
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(ui::label(tag, text::MONO_SMALL, theme::ink3())),
                );
                column = column.child(self.node_column(&graph, &node, making, connected, cx));
                if let Some(line) = node.line {
                    column = column.child(
                        div().flex().pt(px(theme::space::S8)).child(
                            ui::button("graph-remove-step", "Delete step")
                                .ghost()
                                .icon(Icon::Trash)
                                .disabled(making)
                                .tooltip(format!(
                                    "Take line {line} out of the shot's steps. A step whose file a later one reads, and the last, stay until that is changed"
                                ))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit(
                                        RecipeEdit {
                                            line,
                                            remove: true,
                                            ..RecipeEdit::default()
                                        },
                                        cx,
                                    );
                                    this.select(None, window, cx);
                                })),
                        ),
                    );
                }
            }
        }
        // Scrolling with no bar, it fades where a row is cut and while
        // there is more below.
        div()
            .relative()
            .w(px(INSPECTOR))
            .flex_none()
            .h_full()
            .child(
                div()
                    .id("graph-inspector")
                    .track_scroll(&self.inspector_scroll)
                    .size_full()
                    .overflow_y_scroll()
                    .bg(theme::chrome())
                    .border_l_1()
                    .border_color(theme::line())
                    .child(column),
            )
            .child(ui::scroll_fade(&self.inspector_scroll, theme::chrome()))
            .into_any_element()
    }

    /// No node chosen: the shot itself, and what is worth knowing before it is made.
    fn shot_column(
        &self,
        recipe: &Recipe,
        making: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(ui::label(recipe.name.clone(), text::TITLE, theme::ink()))
            .child(ui::label(
                if recipe.document {
                    "steps in the document".to_owned()
                } else {
                    recipe.recipe.clone()
                },
                text::MONO_SMALL,
                theme::ink3(),
            ));
        if let Some(error) = &recipe.error {
            let said = if recipe.document {
                "These steps do not read"
            } else {
                "This recipe does not read"
            };
            column = column.child(note(format!("{said}: {error}"), theme::neg()));
        }
        if let Some(cannot) = &recipe.cannot {
            column = column.child(note(
                format!("It cannot be made as things stand: {cannot}"),
                theme::neg(),
            ));
        }
        if let Some(error) = &recipe.models_error {
            column = column.child(note(error.clone(), theme::neg()));
        }
        if let Some(notes) = notes_block("Before it is made", &recipe.notes) {
            column = column.child(notes);
        }
        if recipe.error.is_none() {
            column = column.child(note(
                if recipe.document {
                    "Choose a node to see and set what it holds. A change is written into the shot's steps in the document, as an edit that is undone as any other; Make then writes a new take."
                } else {
                    "Choose a node to see and set what it holds. A change is written into the recipe, which keeps its comments and its order; Make then writes a new take."
                }
                .to_owned(),
                theme::ink2(),
            ));
            column = column.child(ui::label(
                "Drag a card to move it, and from the dot at its right to wire it into another. The wheel or ⌘ and two fingers zoom; F fits the whole graph.",
                text::META,
                theme::ink3(),
            ));
        }
        if let Some(takes) = recipe.node("takes") {
            column = column.child(self.takes(takes, making, connected, cx));
        }
        column
    }

    /// What a node is, by its kind, and its settings.
    fn node_column(
        &self,
        graph: &Recipe,
        node: &GraphNode,
        making: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        let what = &node.what;
        let missing = !what.there();
        match what {
            What::Model(_) => return column.child(self.model_column(graph, node, cx)),
            What::Render(render) => {
                column = column
                    .child(note(
                        "Kerf draws a sequence of this project into a file a model follows, with no sound.".into(),
                        theme::ink2(),
                    ))
                    .children(self.fields_named(&["sequence"]))
                    .child(self.pass_row(node, cx))
                    .children(self.fields_named(&["size", "rate", "matte", "from", "seconds"]));
                // A matte is one clip in white on black: of footage, as much of it as a mask cuts out.
                if let Some(matte) = render.matte.as_deref().filter(|matte| !matte.is_empty()) {
                    column = column.child(self.pointing(matte, ShotOf::Matte, cx));
                }
                // The guide as it was drawn for the take that shows: what the model was handed.
                let drawn = render.made.as_ref().and_then(|made| {
                    let path = self.in_project(&made.path, cx)?;
                    Some((made.take, self.still(&path)?))
                });
                if let Some((take, image)) = drawn {
                    column = column
                        .child(section(&format!("As drawn for take {take}")))
                        .child(picture_box(image));
                }
            }
            What::Join(_) => {
                column = column
                    .child(note(
                        "Kerf sets the parts it reads end to end into one file: a shot longer than a model makes at once, made a part at a time.".into(),
                        theme::ink2(),
                    ))
                    .children(self.fields_named(&["lap"]))
                    .child(note(
                        "Lap is how many frames each part after the first shares with the one before it, which are left out: 1 where a part goes on from the last frame of the one before.".into(),
                        theme::ink3(),
                    ));
            }
            What::Sequence(_) => {
                column = column
                    .child(note(
                        node.detail.clone(),
                        if missing { theme::neg() } else { theme::ink2() },
                    ))
                    .child(note(
                        "A sequence of this project, which the guide is drawn from. Edit it on the timeline.".into(),
                        theme::ink3(),
                    ));
            }
            What::File(file) => {
                // A picture shows; any file says where it is.
                let shown = self
                    .in_project(file.in_project.as_deref().unwrap_or(&file.path), cx)
                    .filter(|_| node.gives == Gives::Image)
                    .and_then(|path| self.still(&path));
                if let Some(image) = shown {
                    column = column.child(picture_box(image));
                }
                column = column.child(ui::label(file.path.clone(), text::TIMECODE, theme::ink2()));
                if missing {
                    column = column.child(note(
                        "This file is not there. The steps cannot run until it is.".into(),
                        theme::neg(),
                    ));
                }
            }
            What::Signal(_) => column = column.child(self.signal_column(node, cx)),
            What::Moves(moves) => {
                let said = if missing {
                    node.detail.clone()
                } else {
                    format!(
                        "The model is handed where this clip is on each frame of {}: its place as the sequence draws it, so a camera that moves and what hides it are in it. Move the clip, or its path on the picture, to change it.",
                        moves.sequence.as_deref().unwrap_or("its sequence")
                    )
                };
                column = column.child(note(
                    said,
                    if missing { theme::neg() } else { theme::ink2() },
                ));
                if !missing {
                    column = column.child(self.pointing(&moves.clip, ShotOf::Moves, cx));
                }
            }
            What::Cast(named) | What::Style(named) => {
                column = column.child(note(
                    node.detail.clone(),
                    if missing { theme::neg() } else { theme::ink2() },
                ));
                let refs = named.refs.clone().unwrap_or_default();
                if !refs.is_empty() {
                    column =
                        column.child(ui::label(refs.join("\n"), text::MONO_SMALL, theme::ink3()));
                }
                column = column.child(
                    div().flex().child(
                        ui::button("graph-cast", "Cast and Styles…")
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_cast(window, cx)),
                            ),
                    ),
                );
            }
            What::Takes(_) => return column.child(self.takes(node, making, connected, cx)),
        }
        column.child(self.reads(graph, node, cx))
    }

    /// The rows of the fields named, in that order.
    fn fields_named(&self, keys: &[&str]) -> Vec<AnyElement> {
        keys.iter()
            .filter_map(|key| self.field(key))
            .map(|field| field_row(field, None).into_any_element())
            .collect()
    }

    fn model_column(&self, graph: &Recipe, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        let Some(model) = node.model() else {
            return column;
        };
        if let Some(about) = model.about.as_deref().filter(|about| !about.is_empty()) {
            column = column.child(note(about.to_owned(), theme::ink2()));
        }
        if node.is_missing() {
            // Named by the steps, and by nothing of the person's: it cannot be asked.
            column = column
                .child(note(
                    format!(
                        "No model named {} is set up on this computer. A model is a program your own models file names.",
                        model.model
                    ),
                    theme::warn(),
                ))
                .child(
                    div().flex().child(
                        ui::button("graph-models", "Models…")
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.open_models(window, cx))),
                    ),
                );
        }
        column = column.child(section("Prompt"));
        if let Some(prompt) = self.field("prompt") {
            column = column.child(field_box(prompt));
        }
        // The seed, and another at a press: the same shot, rolled again.
        if let Some(seed) = self.field("seed") {
            let die = ui::icon_button("graph-seed-roll", Icon::Dice)
                .tooltip("Another seed")
                .on_click(cx.listener(|this, _, _, cx| this.roll_seed(cx)));
            column = column.child(field_row(seed, Some(die.into_any_element())));
        }
        column = column.children(self.fields_named(&["seconds", "size", "from"]));
        // The knobs: what the line sets for this model alone, then what the
        // model has said it takes, each as it comes until the line says otherwise.
        let knobs: Vec<&str> = model.knobs.iter().map(|knob| knob.name.as_str()).collect();
        column = column
            .child(section("Knobs"))
            .children(self.fields_named(&knobs));
        column = column.children(self.fields_named(&[ADD]));
        column.child(self.reads(graph, node, cx))
    }

    /// A guide's Draws, chosen from a menu with the one it draws ticked.
    fn pass_row(&self, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let pass = match &node.what {
            What::Render(render) if !render.pass.is_empty() => render.pass.clone(),
            _ => "picture".to_owned(),
        };
        let shown = PASSES
            .iter()
            .find(|(id, _)| *id == pass)
            .map_or(pass.clone(), |(_, name)| (*name).to_owned());
        let line = node.line.unwrap_or(0);
        let trigger = ui::trigger(
            "graph-pass-trigger",
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.commit_fields(cx);
                if this.close_menu(MenuKind::Pass, window, cx) {
                    return;
                }
                let this_entity = cx.weak_entity();
                let items = PASSES
                    .iter()
                    .map(|(id, name)| {
                        let (this, id, chosen) = (this_entity.clone(), *id, *id == pass);
                        ui::MenuItem::new(*name)
                            .checked(chosen)
                            .on_choose(move |_, cx| {
                                if chosen || line == 0 {
                                    return;
                                }
                                let _ = this.update(cx, |graph, cx| {
                                    // The picture is what a line draws when it names no pass.
                                    let value = (id != "picture").then(|| id.to_owned());
                                    graph.edit(
                                        RecipeEdit {
                                            line,
                                            key: "pass".to_owned(),
                                            nth: 0,
                                            value,
                                            ..RecipeEdit::default()
                                        },
                                        cx,
                                    )
                                });
                            })
                    })
                    .collect();
                let at = event.position + point(px(0.0), px(16.0));
                this.open_menu(MenuKind::Pass, items, None, at, window, cx);
            }),
        )
        .child(
            ui::button("graph-pass", shown)
                .ghost()
                .small()
                .icon(Icon::ChevronDown),
        );
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .child(
                ui::line("Draws", text::CONTROL, theme::ink2())
                    .w(px(FIELD_LABEL))
                    .flex_none(),
            )
            .child(trigger)
    }

    /// What reads into a node, a row each; a model's can be let go of.
    fn reads(&self, recipe: &Recipe, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let into = recipe.reads_into(&node.id);
        let mut reads = div().flex().flex_col().gap(px(theme::space::S4));
        if into.is_empty() {
            return reads;
        }
        reads = reads.child(section("Reads"));
        let line = node.line.unwrap_or(0);
        for (index, edge) in into.into_iter().enumerate() {
            let from = recipe
                .node(&edge.from)
                .map_or(edge.from.clone(), |from| from.title.clone());
            let (port, nth) = (edge.port.clone(), edge.nth);
            reads = reads.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .h(px(24.0))
                    .child(ui::dot(gives_color(edge.gives), 6.0))
                    .child(
                        ui::line(edge.port.clone(), text::TIMECODE, theme::ink2())
                            .w(px(FIELD_LABEL - 14.0))
                            .flex_none(),
                    )
                    .child(
                        ui::line(from, text::CONTROL, theme::ink())
                            .flex_1()
                            .min_w_0(),
                    )
                    .when(node.is(Kind::Model) && line > 0, |row| {
                        row.child(
                            icon_button(
                                ElementId::Name(format!("let-go-{index}").into()),
                                Icon::Close,
                            )
                            .size(20.0)
                            .tooltip(format!("Take {port} off the line"))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.edit(
                                        RecipeEdit {
                                            line,
                                            key: port.clone(),
                                            nth,
                                            ..RecipeEdit::default()
                                        },
                                        cx,
                                    );
                                },
                            )),
                        )
                    }),
            );
        }
        reads
    }
}

/// A heading over a part of the column.
pub(super) fn section(words: &str) -> gpui::Div {
    div().pt(px(theme::space::S6)).child(ui::label(
        words.to_owned(),
        text::META_STRONG,
        theme::ink3(),
    ))
}

/// A note in the column.
pub(super) fn note(words: String, color: Rgba) -> gpui::Div {
    ui::label(words, text::CONTROL, color)
}
