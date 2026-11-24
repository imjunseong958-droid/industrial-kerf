//! A model's ComfyUI workflow in the shot's graph: asked for and shown in
//! the canvas's place, gone back from to the shot's graph, a change to it
//! kept in the shot's own copy, and the column that says what it is and
//! each of its nodes' inputs.

use super::fields::{field_box, INPUT_LABEL};
use super::inspector::{note, section};
use super::layout::load_places;
use super::{MenuKind, ShotGraph};
use crate::making::{input_kind, part_id, shown_value, GraphNode, InputKind, Recipe, Workflow};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, text};
use editor_ui::ui;
use gpui::{
    div, point, prelude::*, px, AnyElement, Context, ElementId, MouseDownEvent, SharedString,
    Window,
};
use kerf_bridge::event::graph::Kind;
use kerf_bridge::event::workflow::OriginKind;
use kerf_bridge::{Command, RecipeEdit, WorkflowEdit};
use serde_json::Value;

impl ShotGraph {
    pub(super) fn request_workflow(&mut self, cx: &mut Context<Self>) {
        if self.workflow_line == 0 {
            return;
        }
        let command = Command::WorkflowGet {
            media: self.media.to_string(),
            line: self.workflow_line,
        };
        self.session.update(cx, |session, _| session.send(command));
    }

    pub(super) fn set_workflow(
        &mut self,
        workflow: Workflow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = |graph: &Recipe| -> Vec<String> {
            graph.nodes.iter().map(|node| node.id.clone()).collect()
        };
        let before = self.workflow.as_ref().map(|old| ids(&old.graph));
        let another = self
            .workflow
            .as_ref()
            .is_none_or(|old| old.line != workflow.line || old.name != workflow.name);
        let changed = before.is_none_or(|before| before != ids(&workflow.graph));
        // Another workflow takes the fields' place, so what they say is set
        // first, on what they were made for. The same one said again (what
        // the server takes, read late; the project's next revision) leaves
        // a field being typed in to the person until they leave it.
        if another {
            self.commit_fields(cx);
        }
        self.workflow = Some(workflow);
        if another {
            self.selected = None;
            self.placed = load_places(&self.session, &self.places_key(), cx);
            self.fitted = true;
            self.whole = false;
        }
        if self
            .selected
            .as_deref()
            .is_some_and(|id| self.shown().and_then(|graph| graph.node(id)).is_none())
        {
            self.selected = None;
        }
        if self.fitted && (another || changed) {
            self.settle();
        }
        self.inspect(window, cx);
    }

    /// Goes into the workflow of the model on `line`: asked for, and shown
    /// on the canvas once the bridge says it.
    pub(super) fn open_workflow(
        &mut self,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if line == 0 {
            return;
        }
        self.commit_fields(cx);
        self.workflow_line = line;
        self.said = None;
        self.request_workflow(cx);
        window.focus(&self.canvas_focus);
        cx.notify();
    }

    /// Back from a model's workflow to the shot's graph, on the model it is of.
    pub(super) fn close_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let line = std::mem::take(&mut self.workflow_line);
        self.workflow = None;
        self.said = None;
        self.placed = load_places(&self.session, &self.shot_key, cx);
        self.selected = self.recipe.as_ref().and_then(|recipe| {
            recipe
                .nodes
                .iter()
                .find(|node| node.is(Kind::Model) && node.line == Some(line))
                .map(|node| node.id.clone())
        });
        self.fitted = true;
        self.whole = false;
        self.settle();
        self.inspect(window, cx);
        window.focus(&self.canvas_focus);
        cx.notify();
    }

    /// A change to the model's workflow, kept in the shot's own copy of it.
    pub(super) fn edit_workflow(&mut self, change: WorkflowEdit, cx: &mut Context<Self>) {
        let Some(workflow) = &self.workflow else {
            return;
        };
        let change = WorkflowEdit {
            media: self.media.to_string(),
            line: workflow.line,
            was: workflow.text.clone(),
            ..change
        };
        self.said = None;
        self.session
            .update(cx, |session, _| session.send(Command::WorkflowEdit(change)));
        cx.notify();
    }

    /// No node of a workflow chosen: the workflow itself, and what fills its blanks.
    pub(super) fn workflow_column(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        let Some(workflow) = &self.workflow else {
            return column;
        };
        column = column
            .child(ui::label(workflow.name.clone(), text::TITLE, theme::ink()))
            .child(note(
                format!("The ComfyUI workflow model {} is run with.", workflow.model),
                theme::ink3(),
            ));
        if !workflow.about.is_empty() {
            column = column.child(note(workflow.about.clone(), theme::ink2()));
        }
        if workflow.is_own() {
            column = column.child(note(
                format!(
                    "This shot's own copy, {}, which its line names. It is what this shot is made with; nothing on the ComfyUI server is changed.",
                    workflow.file
                ),
                theme::ink2(),
            ));
            let line = workflow.line;
            column = column.child(
                div().flex().child(
                    ui::button("graph-workflow-drop", "Use it as it comes")
                        .ghost()
                        .tooltip("Take workflow= off the line; the copy stays in the project")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit(
                                RecipeEdit {
                                    line,
                                    key: "workflow".to_owned(),
                                    nth: 0,
                                    ..RecipeEdit::default()
                                },
                                cx,
                            )
                        })),
                ),
            );
        } else {
            let from = if workflow.origin == OriginKind::File {
                ", from the file your models entry names"
            } else {
                " with kerf-comfy"
            };
            column = column.child(note(
                format!(
                    "As it comes{from}. Set a value in it and this shot keeps a copy with that change, in the project, which its line then runs. Nothing on the ComfyUI server, and nothing of kerf-comfy, is changed."
                ),
                theme::ink2(),
            ));
        }
        if !workflow.blanks.is_empty() {
            column = column.child(section("Blanks the steps fill"));
            for name in workflow.blanks.names() {
                column = column
                    .child(ui::label(
                        format!("{{{name}}}"),
                        text::TIMECODE,
                        theme::accent_ink(),
                    ))
                    .child(note(workflow.blank_says(name), theme::ink3()));
            }
        }
        column.child(note(
            "Choose a node to see its inputs. A blank is set on the step's line; any other value is set here.".to_owned(),
            theme::ink3(),
        ))
    }

    /// A node of a model's workflow: each input, a link said, a blank and
    /// what fills it, a value set by a switch, a menu or a field.
    pub(super) fn part_column(
        &self,
        graph: &Recipe,
        node: &GraphNode,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        if !node.detail.is_empty() {
            column = column.child(ui::label(
                node.detail.clone(),
                text::MONO_SMALL,
                theme::ink3(),
            ));
        }
        let Some(of) = node.part() else {
            return column;
        };
        let part = of.id.clone();
        let row = |name: &str, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .child(
                    ui::line(name.to_owned(), text::CONTROL, theme::ink2())
                        .w(px(INPUT_LABEL))
                        .flex_none(),
                )
                .child(div().flex_1().min_w_0().child(control))
        };
        for (index, input) in of.inputs.iter().enumerate() {
            let name = input.name.clone();
            match input_kind(input) {
                InputKind::Link => {
                    // Where it comes from: the graph's own shape, said; wired, it is re-pointed.
                    let from = input
                        .link
                        .as_ref()
                        .map_or_else(String::new, |(from, _)| from.clone());
                    let title = graph
                        .node(&part_id(&from))
                        .map_or("a node".to_owned(), |node| node.title.clone());
                    column = column.child(row(
                        &name,
                        ui::label(format!("← {title} #{from}"), text::CONTROL, theme::ink3())
                            .into_any_element(),
                    ));
                }
                InputKind::Blank => {
                    // The steps' to fill: marked, with what fills it and what the line gives now.
                    column = column.child(row(
                        &name,
                        ui::label(
                            shown_value(&input.value),
                            text::TIMECODE,
                            theme::accent_ink(),
                        )
                        .into_any_element(),
                    ));
                    let says: Vec<String> = input
                        .blanks
                        .iter()
                        .filter_map(|blank| {
                            let says = self.workflow.as_ref()?.blank_says(blank);
                            (!says.is_empty()).then_some(says)
                        })
                        .collect();
                    if !says.is_empty() {
                        column = column.child(note(says.join("\n"), theme::ink3()));
                    }
                }
                InputKind::Value => {
                    let held = input.value.clone();
                    if let Value::Bool(on) = held {
                        let (part, input_name) = (part.clone(), name.clone());
                        let this = cx.weak_entity();
                        column = column.child(row(
                            &name,
                            ui::switch(
                                SharedString::from(format!("graph-switch-{index}")),
                                on,
                                move |on, _, cx| {
                                    let change = WorkflowEdit {
                                        node: part.clone(),
                                        input: input_name.clone(),
                                        value: Value::Bool(on),
                                        ..WorkflowEdit::default()
                                    };
                                    let _ = this
                                        .update(cx, |graph, cx| graph.edit_workflow(change, cx));
                                },
                            )
                            .into_any_element(),
                        ));
                        continue;
                    }
                    if !held.is_string() && !held.is_number() {
                        // A list or a table the workflow sets whole: shown, and the file's to change.
                        column = column.child(row(
                            &name,
                            ui::label(shown_value(&held), text::TIMECODE, theme::ink2())
                                .into_any_element(),
                        ));
                        continue;
                    }
                    let choices = input.choices.clone().unwrap_or_default();
                    if !choices.is_empty() {
                        // One of the values the server takes there: chosen, not typed.
                        let trigger = self.choice_trigger(index, &part, &name, held, choices, cx);
                        column = column.child(row(&name, trigger));
                        continue;
                    }
                    if let Some(field) = self.field(&name) {
                        let mut control = field_box(field);
                        if let Some(tip) = field.tip.clone() {
                            control = control.child(
                                ui::label(tip, text::META, theme::ink3()).pt(px(theme::space::S2)),
                            );
                        }
                        column = column.child(row(&name, control.into_any_element()));
                    }
                }
            }
        }
        column
    }

    /// A workflow node's input that takes one of the values the server
    /// takes there (`choices`), chosen from a menu with the one it holds
    /// (`held`) ticked.
    fn choice_trigger(
        &self,
        index: usize,
        part: &str,
        name: &str,
        held: Value,
        choices: Vec<Value>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown = shown_value(&held);
        let (part, input_name) = (part.to_owned(), name.to_owned());
        ui::trigger(
            ElementId::Name(format!("graph-choice-trigger-{index}").into()),
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                if this.close_menu(MenuKind::Choice, window, cx) {
                    return;
                }
                let this_entity = cx.weak_entity();
                let items = choices
                    .iter()
                    .map(|choice| {
                        let chosen = *choice == held;
                        let (this, part, input_name, choice) = (
                            this_entity.clone(),
                            part.clone(),
                            input_name.clone(),
                            choice.clone(),
                        );
                        ui::MenuItem::new(shown_value(&choice))
                            .checked(chosen)
                            .on_choose(move |_, cx| {
                                if chosen {
                                    return;
                                }
                                let change = WorkflowEdit {
                                    node: part.clone(),
                                    input: input_name.clone(),
                                    value: choice.clone(),
                                    ..WorkflowEdit::default()
                                };
                                let _ =
                                    this.update(cx, |graph, cx| graph.edit_workflow(change, cx));
                            })
                    })
                    .collect();
                let at = event.position + point(px(0.0), px(16.0));
                this.open_menu(MenuKind::Choice, items, None, at, window, cx);
            }),
        )
        .child(
            ui::button(
                ElementId::Name(format!("graph-choice-{index}").into()),
                shown,
            )
            .ghost()
            .small()
            .icon(Icon::ChevronDown),
        )
        .into_any_element()
    }
}
