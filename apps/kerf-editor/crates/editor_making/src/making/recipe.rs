//! A shot's graph as the bridge reads it: its nodes and the edges of what
//! each reads ([`Recipe`]), or a model's ComfyUI workflow as a graph of its
//! own nodes ([`Workflow`]); and what may read what, under which name.

use crate::making::value_text;
use kerf_bridge::event::graph::{Gives, Kind, ModelNode, RunnerKind, TakesNode, What};
use kerf_bridge::event::workflow::{Blank, By, DescribedInput, DescribedNode, OriginKind, Shown};
use kerf_bridge::event::Recipe as RecipeEvent;
use serde_json::Value;
use std::collections::BTreeMap;

/// What a node is: one of a shot's graph, with what the bridge says of one of its kind,
/// or one of a model's workflow's own nodes.
#[derive(Clone, Debug, PartialEq)]
pub enum Holds {
    Shot(What),
    Part(DescribedNode),
}

/// One node of a shot's graph, as the bridge's `recipe` lays it out; or,
/// in a model's workflow, one of the workflow's own nodes (a part).
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub gives: Gives,
    /// The step's line, from 1, for a step.
    pub line: Option<usize>,
    /// What else it holds, by its kind.
    pub holds: Holds,
}

impl GraphNode {
    /// What it is in a shot's graph: none for one of a workflow's own nodes.
    pub fn kind(&self) -> Option<Kind> {
        match &self.holds {
            Holds::Shot(what) => Some(what.kind()),
            Holds::Part(_) => None,
        }
    }

    pub fn is(&self, kind: Kind) -> bool {
        self.kind() == Some(kind)
    }

    /// A step of the shot: a line of its steps.
    pub fn is_step(&self) -> bool {
        self.line.is_some_and(|line| line > 0)
    }

    /// One of a model's workflow's own nodes.
    pub fn is_part(&self) -> bool {
        matches!(self.holds, Holds::Part(_))
    }

    /// The workflow's own node it is, for a part.
    pub fn part(&self) -> Option<&DescribedNode> {
        match &self.holds {
            Holds::Part(part) => Some(part),
            Holds::Shot(_) => None,
        }
    }

    /// What its `generate` line asks, for a model.
    pub fn model(&self) -> Option<&ModelNode> {
        match &self.holds {
            Holds::Shot(What::Model(model)) => Some(model),
            _ => None,
        }
    }

    /// The shot's takes, for the node that lists them.
    pub fn takes(&self) -> Option<&TakesNode> {
        match &self.holds {
            Holds::Shot(What::Takes(takes)) => Some(takes),
            _ => None,
        }
    }

    /// A model its entry runs with kerf-comfy: it has a workflow to open.
    pub fn has_workflow(&self) -> bool {
        self.model()
            .and_then(|model| model.runner.as_ref())
            .is_some_and(|runner| runner.kind == RunnerKind::Comfy)
    }

    /// A model not set up on this computer, or a file that is not there.
    pub fn is_missing(&self) -> bool {
        match &self.holds {
            Holds::Shot(What::Model(model)) => !model.set_up,
            Holds::Shot(what) => !what.there(),
            Holds::Part(_) => false,
        }
    }
}

/// One reading: what `from` gives, read by `to` under the name `port`.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub port: String,
    pub nth: usize,
    pub gives: Gives,
}

/// What goes along an edge, as its badge says it.
pub fn gives_word(gives: Gives) -> &'static str {
    match gives {
        Gives::Video => "video",
        Gives::Image => "image",
        Gives::Signal => "signal",
        Gives::Path => "path",
        Gives::Words => "words",
        Gives::File => "file",
    }
}

/// A media's steps as a graph (the bridge's `recipe`), or a model's
/// workflow as one ([`Workflow::read`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recipe {
    pub media: String,
    pub recipe: String,
    pub document: bool,
    pub name: String,
    /// The steps as text, as the bridge read them: what an edit says it changes.
    pub text: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub error: Option<String>,
    pub cannot: Option<String>,
    pub notes: Vec<String>,
    pub models_error: Option<String>,
    /// A model's workflow: its nodes are parts.
    pub parts: bool,
}

impl Recipe {
    /// Reads a `recipe` event's graph. A node with no id, and an edge to or from what is
    /// not there, is left out.
    pub fn read(event: &RecipeEvent) -> Self {
        let graph = &event.graph;
        let said = |said: &Option<String>| said.clone().filter(|said| !said.is_empty());
        let nodes: Vec<GraphNode> = graph
            .nodes
            .iter()
            .filter(|node| !node.id.is_empty())
            .map(|node| GraphNode {
                id: node.id.clone(),
                title: node.title.clone(),
                detail: node.detail.clone(),
                gives: node.gives,
                line: node.line,
                holds: Holds::Shot(node.what.clone()),
            })
            .collect();
        let there = |id: &str| nodes.iter().any(|node| node.id == id);
        let edges = graph
            .edges
            .iter()
            .filter(|edge| there(&edge.from) && there(&edge.to))
            .map(|edge| GraphEdge {
                from: edge.from.clone(),
                to: edge.to.clone(),
                port: edge.port.clone(),
                nth: edge.nth,
                gives: edge.gives,
            })
            .collect();
        Self {
            media: graph.media.clone(),
            recipe: graph.recipe.clone(),
            document: graph.document,
            name: graph.name.clone(),
            text: graph.text.clone(),
            nodes,
            edges,
            error: said(&graph.error),
            cannot: said(&graph.cannot),
            notes: graph.notes.clone(),
            models_error: said(&event.models_error),
            parts: false,
        }
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// What reads into `id`, in the order the steps give it.
    pub fn reads_into(&self, id: &str) -> Vec<&GraphEdge> {
        self.edges.iter().filter(|edge| edge.to == id).collect()
    }

    /// Whether its steps read, so a making can be asked for.
    pub fn reads(&self) -> bool {
        self.error.is_none() && !self.nodes.is_empty()
    }
}

/// The workflow a model is run with, where its program is kerf-comfy (the
/// bridge's `workflow`): every node and input of the ComfyUI graph, as a
/// graph of parts, and the blanks the shot's steps fill.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workflow {
    pub media: String,
    /// The `generate` line it is of, from 1.
    pub line: usize,
    pub model: String,
    pub name: String,
    /// The shot's own copy, the models entry's file, or one that comes with kerf-comfy.
    pub origin: OriginKind,
    /// The shot's own copy, where it has one.
    pub file: String,
    pub about: String,
    /// Its text as read: what an edit says it changes.
    pub text: String,
    /// Each blank by name: what fills it (`says`), what the line gives for it now (`given`).
    pub blanks: BTreeMap<String, Blank>,
    /// The blanks' names, in the order the workflow's nodes first hold them.
    pub blanks_as_held: Vec<String>,
    pub graph: Recipe,
}

/// A workflow node's id in the graph.
pub fn part_id(id: &str) -> String {
    format!("part:{id}")
}

impl Workflow {
    /// Reads a `workflow` event. A node with no id is left out, and so is a link from one
    /// that is not there.
    pub fn read(event: &Shown) -> Self {
        let file = event.file.clone().unwrap_or_default();
        let mut graph = Recipe {
            media: event.media.clone(),
            name: event.model.clone(),
            recipe: if event.origin == OriginKind::Own {
                file.clone()
            } else {
                event.name.clone()
            },
            text: event.text.clone(),
            parts: true,
            ..Recipe::default()
        };
        for node in &event.described.nodes {
            if node.id.is_empty() {
                continue;
            }
            // By what ComfyUI's own editor calls it where its export says, else by its kind.
            let class = node.class.clone();
            let title = node
                .title
                .clone()
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| class.clone());
            let detail = if title == class { String::new() } else { class };
            graph.nodes.push(GraphNode {
                id: part_id(&node.id),
                title,
                detail,
                gives: Gives::File,
                line: None,
                holds: Holds::Part(node.clone()),
            });
        }
        let mut edges = Vec::new();
        for node in &graph.nodes {
            for input in node.part().into_iter().flat_map(|part| &part.inputs) {
                let Some((from, _)) = &input.link else {
                    continue;
                };
                let from = part_id(from);
                if graph.node(&from).is_some() {
                    edges.push(GraphEdge {
                        from,
                        to: node.id.clone(),
                        port: input.name.clone(),
                        nth: 0,
                        gives: Gives::File,
                    });
                }
            }
        }
        graph.edges = edges;
        Self {
            media: event.media.clone(),
            line: event.line,
            model: event.model.clone(),
            name: event.name.clone(),
            origin: event.origin,
            file,
            about: event.described.about.clone(),
            text: event.text.clone(),
            blanks: event.described.blanks.clone(),
            blanks_as_held: event
                .described
                .blanks_as_held()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            graph,
        }
    }

    /// The shot's own copy, which its line names.
    pub fn is_own(&self) -> bool {
        self.origin == OriginKind::Own
    }

    /// What fills blank `name`, and what the line gives for it now.
    pub fn blank_says(&self, name: &str) -> String {
        let Some(blank) = self.blanks.get(name) else {
            return String::new();
        };
        let says = capitalised(&blank.says);
        if says.is_empty() {
            return says;
        }
        match &blank.given {
            Some(given) => format!("{says}. Now: {given}"),
            // A file or a signal it gives none of leaves the node out.
            None if matches!(blank.by, By::File | By::Signal | By::Moves) => {
                format!("{says}. The line gives none, so the node is left out")
            }
            None => says,
        }
    }
}

fn capitalised(words: &str) -> String {
    let mut letters = words.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// What a workflow node's input is: a link from another node, a blank the
/// shot's steps fill, or a value of the workflow's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Link,
    Blank,
    Value,
}

pub fn input_kind(input: &DescribedInput) -> InputKind {
    if input.link.is_some() {
        InputKind::Link
    } else if input.blanks.is_empty() {
        InputKind::Value
    } else {
        InputKind::Blank
    }
}

/// A workflow's value as a row of it shows it: words as they are, a number
/// as it is written, a list or a table as JSON.
pub fn shown_value(value: &Value) -> String {
    match value {
        Value::Number(number) => shown_number(number),
        other => value_text(other),
    }
}

/// A number of a workflow as a row of it shows it: as it is written, a whole one without
/// a point.
pub fn shown_number(number: &serde_json::Number) -> String {
    match (number.as_i64(), number.as_f64()) {
        (Some(whole), _) => whole.to_string(),
        (None, Some(real)) if real.fract() == 0.0 && real.abs() < 1e15 => {
            format!("{}", real as i64)
        }
        (None, Some(real)) => format!("{real}"),
        _ => number.to_string(),
    }
}

/// What a workflow node gives at each output, as a server named them in words.
fn outputs_of(part: &DescribedNode) -> Vec<&str> {
    part.outputs
        .iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

// ------------------------------------------------------------------ wiring

/// The names `to` could read what `from` gives under, the first the most
/// likely: none where it cannot read it at all. A step reads what a step
/// before it made, never one after; a guide reads a sequence; a model reads
/// a signal, a clip's way, the cast and styles as such, a picture in motion
/// as a guide (or another of the guides it says it reads), a reference or
/// its first frame, and a still as its first or last frame or a reference.
/// In a model's workflow, a node's output is read by another's input that
/// is a link already and takes what it gives.
pub fn ports_for(from: &GraphNode, to: &GraphNode) -> Vec<String> {
    if from.is_part() || to.is_part() {
        let (Some(gives), Some(takes)) = (from.part(), to.part()) else {
            return Vec::new();
        };
        if from.id == to.id {
            return Vec::new();
        }
        let gives = outputs_of(gives);
        return takes
            .inputs
            .iter()
            .filter(|input| input_kind(input) == InputKind::Link)
            .filter(|input| {
                let takes = input.takes.as_deref().unwrap_or_default();
                gives.is_empty() || takes.is_empty() || gives.contains(&takes)
            })
            .map(|input| input.name.clone())
            .collect();
    }
    if from.id == to.id || !to.is_step() {
        return Vec::new();
    }
    if from.is_step() && from.line >= to.line {
        return Vec::new();
    }
    let one = |port: &str| vec![port.to_owned()];
    if to.is(Kind::Render) {
        return if from.is(Kind::Sequence) {
            one("sequence")
        } else {
            Vec::new()
        };
    }
    let Some(model) = to.model() else {
        return Vec::new();
    };
    match from.kind() {
        Some(Kind::Signal) => return one("signal"),
        Some(Kind::Moves) => return one("moves"),
        Some(Kind::Cast) => return one("cast"),
        Some(Kind::Style) => return one("style"),
        Some(Kind::Sequence | Kind::Takes) => return Vec::new(),
        _ => {}
    }
    match from.gives {
        Gives::Video => {
            let mut ports = one("guide");
            // The guides it reads beside the first, by what each is of.
            ports.extend(
                model
                    .reads
                    .iter()
                    .flatten()
                    .filter(|read| read.starts_with("guide_"))
                    .cloned(),
            );
            // As a first frame it is where it ended: the shot goes on from it.
            ports.extend(["ref".to_owned(), "first".to_owned()]);
            ports
        }
        Gives::Image => ["first", "last", "ref"].map(str::to_owned).to_vec(),
        _ => one("ref"),
    }
}

/// What a step's line writes to read `from`: a clip by its name, one of the
/// cast or a style by its name, a file by its path, a sequence by its id, a
/// step's file by the name it has in braces. Nothing for the takes.
pub fn value_of(from: &GraphNode) -> String {
    let Holds::Shot(what) = &from.holds else {
        return String::new();
    };
    match what {
        What::Signal(signal) => signal.clip.clone(),
        What::Moves(moves) => moves.clip.clone(),
        What::Cast(named) | What::Style(named) => named.name.clone(),
        What::File(file) => file.path.clone(),
        What::Sequence(sequence) => sequence.sequence.clone(),
        What::Takes(_) => String::new(),
        What::Render(_) | What::Model(_) | What::Join(_) => match what.makes() {
            Some(makes) if !makes.is_empty() => format!("{{{makes}}}"),
            _ => String::new(),
        },
    }
}

/// What a model makes of what comes in as `port`.
pub fn port_says(port: &str) -> String {
    if let Some(of) = port.strip_prefix("guide_") {
        return format!("another picture in motion to follow, of its {of}");
    }
    match port {
        "guide" => "a picture in motion to follow",
        "first" => "the first frame",
        "last" => "the last frame",
        "ref" => "who or what is in it, or how it looks",
        "signal" => "when, and how much",
        "moves" => "where a thing goes",
        "cast" => "who or what it shows",
        "style" => "how it looks",
        "sequence" => "what the guide is drawn from",
        _ => "",
    }
    .to_owned()
}

/// Whether `to` makes use of what comes in as `port`: a model that has
/// said what it reads reads only that, and the cast and styles always.
pub fn reads_port(to: &GraphNode, port: &str) -> bool {
    let Some(reads) = to.model().and_then(|model| model.reads.as_ref()) else {
        return true;
    };
    port == "cast" || port == "style" || reads.iter().any(|read| read == port)
}

/// Which word of `port` on `to`'s line a new reading is: a name a line
/// gives once is replaced (0); one it may give again (a reference, a
/// signal, a clip's way, the cast, a style) is one more.
pub fn wire_nth(recipe: &Recipe, to: &str, port: &str) -> usize {
    if !matches!(port, "ref" | "signal" | "moves" | "cast" | "style") {
        return 0;
    }
    recipe
        .reads_into(to)
        .iter()
        .filter(|edge| edge.port == port)
        .count()
}

/// Which output of workflow node `from` input `input` of `to` reads: the
/// first that gives what the input takes.
pub fn link_output(from: &GraphNode, to: &GraphNode, input: &str) -> usize {
    let takes = to
        .part()
        .and_then(|part| part.inputs.iter().find(|known| known.name == input))
        .and_then(|known| known.takes.as_deref())
        .unwrap_or_default();
    from.part()
        .map(outputs_of)
        .unwrap_or_default()
        .iter()
        .position(|gives| *gives == takes)
        .unwrap_or(0)
}
