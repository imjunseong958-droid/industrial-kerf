//! The chosen node's fields: what each sets on its step's line (or on a
//! node of a model's workflow), made again for another node or another
//! shape of it, carrying over what a person is typing, and set as they
//! leave it.

use super::ShotGraph;
use crate::making::{
    input_kind, plain, shown_number, shown_value, value_text, GraphNode, Holds, InputKind,
};
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::theme::{self, radius, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, ElementId, Entity, Focusable, Subscription,
    Window,
};
use kerf_bridge::event::graph::What;
use kerf_bridge::{RecipeEdit, WorkflowEdit};
use serde_json::{json, Value};
use std::collections::HashMap;

/// A field's name, before it; a workflow input's longer one.
pub(super) const FIELD_LABEL: f32 = 76.0;
pub(super) const INPUT_LABEL: f32 = 112.0;

/// A field the chosen node's column shows: a name on its line to set.
#[derive(Clone, Debug, PartialEq)]
struct Want {
    key: String,
    label: String,
    value: String,
    placeholder: String,
    multiline: bool,
    tip: Option<String>,
}

/// Any other name goes to the model as it is written: `name=value`.
pub(super) const ADD: &str = "+add";

fn want(key: &str, label: &str, value: impl Into<String>, placeholder: &str) -> Want {
    Want {
        key: key.to_owned(),
        label: label.to_owned(),
        value: value.into(),
        placeholder: placeholder.to_owned(),
        multiline: false,
        tip: None,
    }
}

/// What a line says of a name, or nothing where it says none.
fn said(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

/// The fields of a node, by its kind: what its line sets. A guide's Draws
/// is chosen from a menu, and a workflow's values are set by what each is.
fn wants_of(node: &GraphNode) -> Vec<Want> {
    match &node.holds {
        Holds::Shot(What::Model(model)) => {
            let mut wants = vec![Want {
                multiline: true,
                ..want(
                    "prompt",
                    "Prompt",
                    model.prompt.as_str(),
                    "What the file should show",
                )
            }];
            wants.push(want(
                "seed",
                "Seed",
                model.seed.map(|seed| seed.to_string()).unwrap_or_default(),
                "the model picks one",
            ));
            let longest = match model.model_seconds {
                None => "as the model makes".to_owned(),
                Some(seconds) => format!("up to {}", plain(seconds)),
            };
            wants.push(want(
                "seconds",
                "Seconds",
                model.seconds.map(plain).unwrap_or_default(),
                &longest,
            ));
            let size = model.model_size.as_deref().unwrap_or("the model's own");
            wants.push(want("size", "Size", said(&model.size), size));
            // A part of a longer shot says where in its sequences' time it starts.
            if let Some(from) = model.from {
                wants.push(want("from", "From", plain(from), "the sequence's start"));
            }
            for knob in &model.knobs {
                let name = knob.name.as_str();
                if name.is_empty() {
                    continue;
                }
                let comes = knob.default.as_ref().map(value_text).unwrap_or_default();
                let tip = if comes.is_empty() {
                    format!("{name}, as the line sets it; emptied, it is taken off the line")
                } else {
                    format!("{name}: {comes} as the model comes; emptied, it is that again")
                };
                wants.push(Want {
                    tip: Some(tip),
                    ..want(name, name, said(&knob.value), &comes)
                });
            }
            wants.push(want(ADD, "Add", "", "name=value"));
            wants
        }
        Holds::Shot(What::Render(render)) => vec![
            want("sequence", "Sequence", render.sequence.as_str(), ""),
            want("size", "Size", said(&render.size), "the sequence's"),
            want("rate", "Rate", said(&render.rate), "the sequence's"),
            want(
                "matte",
                "Matte of",
                said(&render.matte),
                "a clip, alone in white",
            ),
            want("from", "From", said(&render.from), "its start"),
            want("seconds", "Seconds", said(&render.seconds), "to its end"),
        ],
        Holds::Shot(What::Join(join)) => {
            let lap = Some(join.lap).filter(|lap| *lap > 0);
            vec![want(
                "lap",
                "Lap",
                lap.map(|lap| lap.to_string()).unwrap_or_default(),
                "0 frames",
            )]
        }
        Holds::Part(part) => part
            .inputs
            .iter()
            .filter(|input| input_kind(input) == InputKind::Value)
            .filter(|input| {
                let held = &input.value;
                (held.is_string() || held.is_number())
                    && input.choices.as_ref().is_none_or(Vec::is_empty)
            })
            .map(|input| {
                let held = &input.value;
                let (least, most) = (&input.min, &input.max);
                let tip = (held.is_number() && (least.is_some() || most.is_some())).then(|| {
                    let end = |end: &Option<serde_json::Number>| {
                        end.as_ref().map_or_else(|| "any".to_owned(), shown_number)
                    };
                    format!("{} to {}", end(least), end(most))
                });
                Want {
                    key: input.name.clone(),
                    label: input.name.clone(),
                    value: shown_value(held),
                    placeholder: String::new(),
                    multiline: false,
                    tip,
                }
            })
            .collect(),
        Holds::Shot(_) => Vec::new(),
    }
}

/// What a value typed into a workflow's number input sets it to: the
/// number, where it reads as one the server takes there; none otherwise,
/// and the field shows what it was.
pub fn part_number(typed: &str, least: Option<f64>, most: Option<f64>) -> Option<f64> {
    let value: f64 = typed
        .trim()
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite())?;
    if least.is_some_and(|least| value < least) || most.is_some_and(|most| value > most) {
        return None;
    }
    Some(value)
}

/// A prompt as its line holds it: one line, its spaces single.
pub fn one_line(words: &str) -> String {
    words.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A seed a model's sampler takes: a whole number under 2^31.
pub fn another_seed() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()),
    );
    (hasher.finish() % (1u64 << 31)) as u32
}

/// What a field sets: a name on its line, a `name=value` added to it, or a
/// value of a node of a model's workflow (a number held to what the
/// server takes there).
#[derive(Clone, Debug, PartialEq)]
enum FieldKind {
    Line,
    Add,
    Part {
        node: String,
        number: bool,
        least: Option<f64>,
        most: Option<f64>,
    },
}

/// A field of the chosen node, the line it was made for, and what it last
/// showed of the steps: what is typed is set as the person leaves it, on
/// that line, whatever node has been chosen since.
pub(super) struct Field {
    key: String,
    label: String,
    line: usize,
    shown: String,
    input: Entity<TextInput>,
    multiline: bool,
    kind: FieldKind,
    pub(super) tip: Option<String>,
}

/// The chosen node's fields.
pub(super) struct Inspecting {
    /// The node they are of.
    node: String,
    /// The node, its line and its fields' names: the same shape keeps its fields.
    shape: String,
    fields: Vec<Field>,
    _subscriptions: Vec<Subscription>,
}

/// What a field's commit does, worked out before the graph is changed.
enum Commit {
    Nothing,
    Line {
        line: usize,
        key: String,
        value: String,
    },
    Part {
        node: String,
        input: String,
        value: Value,
    },
    Revert(String),
}

impl ShotGraph {
    /// The chosen node's fields: made again for another node, or another
    /// shape of the same, else set to what the steps now say, except where
    /// a person is typing.
    ///
    /// Nothing is set here. A person's own way to another node (choosing
    /// it, going back from a workflow) sets what the fields say before it
    /// comes here; what comes here from the bridge (the steps or the
    /// workflow said again) may have moved the node to another line, or
    /// taken it out, so what was typed for it is not set on the line it was
    /// typed for: it goes on in the node's new fields, under the same
    /// names, to be set on the node's line as the person leaves it.
    pub(super) fn inspect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self
            .selected
            .as_ref()
            .and_then(|id| self.shown()?.node(id))
            .cloned();
        let Some(node) = node else {
            self.inspecting = None;
            return;
        };
        let wants = wants_of(&node);
        let line = node.line.unwrap_or(0);
        let names: Vec<&str> = wants.iter().map(|want| want.key.as_str()).collect();
        let shape = format!("{}\n{line}\n{}", node.id, names.join("\n"));
        if let Some(inspecting) = self
            .inspecting
            .as_mut()
            .filter(|inspecting| inspecting.shape == shape)
        {
            for (field, want) in inspecting.fields.iter_mut().zip(&wants) {
                if field.kind == FieldKind::Add {
                    continue;
                }
                let focused = field.input.read(cx).focus_handle(cx).is_focused(window);
                let typing = focused && typed(field, cx) != field.shown;
                field.shown = want.value.clone();
                // What is being typed is the person's until they leave it.
                if !typing && field.input.read(cx).text() != want.value {
                    let value = want.value.clone();
                    field
                        .input
                        .update(cx, |input, cx| input.set_text(value, cx));
                }
            }
            return;
        }
        let carried = self
            .inspecting
            .take()
            .filter(|old| old.node == node.id)
            .map(|old| carried_typing(&old.fields, window, cx))
            .unwrap_or_default();
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for want in wants {
            let kind = if want.key == ADD {
                FieldKind::Add
            } else if let Some(part) = node.part() {
                let input = part.inputs.iter().find(|input| input.name == want.key);
                let bound = |bound: &Option<serde_json::Number>| {
                    bound.as_ref().and_then(serde_json::Number::as_f64)
                };
                FieldKind::Part {
                    node: part.id.clone(),
                    number: input.is_some_and(|input| input.value.is_number()),
                    least: input.and_then(|input| bound(&input.min)),
                    most: input.and_then(|input| bound(&input.max)),
                }
            } else {
                FieldKind::Line
            };
            let (says, focused) = match carried.get(&want.key) {
                Some((typed, focused)) => (typed.clone(), *focused),
                None => (want.value.clone(), false),
            };
            let input = cx.new(|cx| {
                let input = TextInput::new(cx, want.placeholder.clone());
                let mut input = if want.multiline {
                    input.multiline().max_height(px(88.0))
                } else {
                    input.with_style(text::TIMECODE)
                };
                input.set_text(says, cx);
                input
            });
            if focused {
                window.focus(&input.read(cx).focus_handle(cx));
            }
            let key = want.key.clone();
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.commit(&key, cx),
                    // Esc puts back what the steps say, and leaves the field.
                    InputEvent::Cancel => {
                        this.revert(&key, cx);
                        window.focus(&this.canvas_focus);
                        this.follow_typing(window, cx);
                    }
                    // What is pasted is one line, as the step's line is.
                    InputEvent::Changed => {
                        if key == "prompt" && input.read(cx).text().contains('\n') {
                            let one = one_line(input.read(cx).text());
                            input.update(cx, |input, cx| input.set_text(one, cx));
                        }
                    }
                },
            ));
            let focus = input.read(cx).focus_handle(cx);
            let key = want.key.clone();
            subscriptions
                .push(cx.on_blur(&focus, window, move |this, _, cx| this.commit(&key, cx)));
            fields.push(Field {
                key: want.key,
                label: want.label,
                line,
                shown: want.value,
                input,
                multiline: want.multiline,
                kind,
                tip: want.tip,
            });
        }
        self.inspecting = Some(Inspecting {
            node: node.id.clone(),
            shape,
            fields,
            _subscriptions: subscriptions,
        });
    }

    /// A field left since the keys were last followed, by Tab, a click
    /// elsewhere or Esc, sets what it says. Followed once each press or key
    /// in the graph has run ([`ShotGraph::keys_may_move`]), as Esc leaves a
    /// field, as a menu gives the keys back and as the bridge is heard, so
    /// that it holds whether the window is the active one or not: GPUI says
    /// a field is left (`on_blur`) only in the active window.
    pub(super) fn follow_typing(&mut self, window: &Window, cx: &mut Context<Self>) {
        let focused = self.inspecting.as_ref().and_then(|inspecting| {
            inspecting
                .fields
                .iter()
                .find(|field| field.input.read(cx).focus_handle(cx).is_focused(window))
                .map(|field| field.key.clone())
        });
        if self.typing_in != focused {
            if let Some(left) = self.typing_in.take() {
                self.commit(&left, cx);
            }
            self.typing_in = focused;
        }
    }

    /// A press or a key in the graph may take the keys out of a field: once
    /// all it does has run, what the field left says is set
    /// ([`ShotGraph::follow_typing`]).
    pub(super) fn keys_may_move(&self, window: &Window, cx: &mut Context<Self>) {
        cx.defer_in(window, |graph, window, cx| graph.follow_typing(window, cx));
    }

    /// Sets what was typed in each field the person has not left yet.
    pub(super) fn commit_fields(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<String> = self
            .inspecting
            .as_ref()
            .map(|inspecting| {
                inspecting
                    .fields
                    .iter()
                    .map(|field| field.key.clone())
                    .collect()
            })
            .unwrap_or_default();
        for key in keys {
            self.commit(&key, cx);
        }
    }

    /// Field `key` set to what it says, on the line it was made for: the
    /// name taken off the line when it says nothing; a `name=value` added;
    /// a workflow's value set, a number only where the server takes it.
    fn commit(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(field) = self
            .inspecting
            .as_mut()
            .and_then(|inspecting| inspecting.fields.iter_mut().find(|field| field.key == key))
        else {
            return;
        };
        let now = typed(field, cx);
        let commit = match &field.kind {
            FieldKind::Add => {
                let Some((name, value)) = now.split_once('=') else {
                    return;
                };
                let (name, value) = (name.trim().to_owned(), value.trim().to_owned());
                if name.is_empty() || field.line == 0 {
                    return;
                }
                field.input.update(cx, |input, cx| input.clear(cx));
                if value.is_empty() {
                    Commit::Nothing
                } else {
                    Commit::Line {
                        line: field.line,
                        key: name,
                        value,
                    }
                }
            }
            _ if now == field.shown => Commit::Nothing,
            FieldKind::Line if field.line == 0 => Commit::Nothing,
            FieldKind::Line => {
                field.shown = now.clone();
                Commit::Line {
                    line: field.line,
                    key: field.key.clone(),
                    value: now,
                }
            }
            FieldKind::Part {
                node,
                number,
                least,
                most,
            } => {
                if !number {
                    field.shown = now.clone();
                    Commit::Part {
                        node: node.clone(),
                        input: field.key.clone(),
                        value: json!(now),
                    }
                } else {
                    match part_number(&now, *least, *most) {
                        Some(value) => {
                            field.shown = now;
                            let value = if value.fract() == 0.0 && value.abs() < 1e15 {
                                json!(value as i64)
                            } else {
                                json!(value)
                            };
                            Commit::Part {
                                node: node.clone(),
                                input: field.key.clone(),
                                value,
                            }
                        }
                        // A number stays a number, and one the server does
                        // not take there is not sent to be refused.
                        None => Commit::Revert(field.shown.clone()),
                    }
                }
            }
        };
        match commit {
            Commit::Nothing => {}
            Commit::Line { line, key, value } => {
                // The picture is what a guide draws when its line names no pass.
                if value.is_empty() {
                    self.edit(
                        RecipeEdit {
                            line,
                            key,
                            nth: 0,
                            ..RecipeEdit::default()
                        },
                        cx,
                    );
                } else {
                    self.edit(
                        RecipeEdit {
                            line,
                            key,
                            nth: 0,
                            value: Some(value),
                            ..RecipeEdit::default()
                        },
                        cx,
                    );
                }
            }
            Commit::Part { node, input, value } => self.edit_workflow(
                WorkflowEdit {
                    node,
                    input,
                    value,
                    ..WorkflowEdit::default()
                },
                cx,
            ),
            Commit::Revert(was) => {
                if let Some(field) = self.field(key) {
                    field.input.update(cx, |input, cx| input.set_text(was, cx));
                }
            }
        }
    }

    /// Field `key` back to what the steps say.
    fn revert(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(field) = self.field(key) {
            let shown = if field.kind == FieldKind::Add {
                String::new()
            } else {
                field.shown.clone()
            };
            field
                .input
                .update(cx, |input, cx| input.set_text(shown, cx));
        }
    }

    pub(super) fn field(&self, key: &str) -> Option<&Field> {
        self.inspecting
            .as_ref()?
            .fields
            .iter()
            .find(|field| field.key == key)
    }

    /// Another seed, written at once.
    pub(super) fn roll_seed(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.field("seed") else {
            return;
        };
        let rolled = another_seed().to_string();
        field
            .input
            .update(cx, |input, cx| input.set_text(rolled, cx));
        self.commit("seed", cx);
    }
}

/// What a person typed into `fields` and has not set yet, by each field's
/// name, and whether it has the keys: what the node's fields made again
/// start with. A field that says what the steps say has nothing to carry;
/// Add, anything in it.
fn carried_typing(fields: &[Field], window: &Window, cx: &App) -> HashMap<String, (String, bool)> {
    fields
        .iter()
        .filter(|field| {
            if field.kind == FieldKind::Add {
                !field.input.read(cx).text().is_empty()
            } else {
                typed(field, cx) != field.shown
            }
        })
        .map(|field| {
            let input = field.input.read(cx);
            let focused = input.focus_handle(cx).is_focused(window);
            (field.key.clone(), (input.text().to_owned(), focused))
        })
        .collect()
}

/// What a field says now, as its line would hold it: a prompt one line.
fn typed(field: &Field, cx: &App) -> String {
    let words = field.input.read(cx).text();
    if field.key == "prompt" {
        one_line(words)
    } else {
        words.trim().to_owned()
    }
}

/// A field's box: a well, with its words.
pub(super) fn field_box(field: &Field) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .px(px(theme::space::S8))
        .py(px(5.0))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line2())
        .when(field.multiline, |input| input.min_h(px(88.0)))
        .child(field.input.clone())
}

/// A field's name and its box, and what follows the box (a seed's die).
pub(super) fn field_row(field: &Field, after: Option<AnyElement>) -> gpui::Div {
    let mut name = ui::line(field.label.clone(), text::CONTROL, theme::ink2())
        .w(px(FIELD_LABEL))
        .flex_none();
    if let Some(tip) = &field.tip {
        let tip = tip.clone();
        name = div().child(ui::tipped(
            ElementId::Name(format!("graph-field-tip-{}", field.key).into()),
            ui::Tip::new(tip),
            name,
        ));
    }
    div()
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .child(name)
        .child(field_box(field))
        .children(after)
}
