//! A shot's graph as the bridge reads it: its nodes and the edges of what
//! each reads ([`Recipe`]), or a model's ComfyUI workflow as a graph of its
//! own nodes ([`Workflow`]); and what may read what, under which name.

use crate::making::{strings, value_text};
use kerf_bridge::event::Recipe as RecipeEvent;
use serde_json::{Map, Value};

/// One node of a shot's graph, as the bridge's `recipe` lays it out; or,
/// in a model's workflow, one of the workflow's own nodes (kind `part`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub gives: String,
    /// The step's line, from 1, for a step.
    pub line: Option<usize>,
    /// The whole node, for what else it holds by its kind.
    pub holds: Value,
}

impl GraphNode {
    /// A step of the shot: a line of its steps.
    pub fn is_step(&self) -> bool {
        self.line.is_some_and(|line| line > 0)
    }

    /// One of a model's workflow's own nodes.
    pub fn is_part(&self) -> bool {
        self.kind == "part"
    }

    /// A model its entry runs with kerf-comfy: it has a workflow to open.
    pub fn has_workflow(&self) -> bool {
        self.kind == "model" && self.holds["runner"]["kind"] == "comfy"
    }

    /// A model not set up on this computer, or a file that is not there.
    pub fn is_missing(&self) -> bool {
        if self.kind == "model" {
            return self.holds["setUp"] == Value::Bool(false);
        }
        self.holds["there"] == Value::Bool(false)
    }
}

/// One reading: what `from` gives, read by `to` under the name `port`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub port: String,
    pub nth: usize,
    pub gives: String,
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
    /// A model's workflow: its nodes are `part`s.
    pub parts: bool,
}

impl Recipe {
    /// Reads a `recipe` event's graph, leniently. An edge to or from what is
    /// not there is left out.
    pub fn read(event: &RecipeEvent) -> Self {
        let field = |key: &str| event.graph.get(key).unwrap_or(&Value::Null);
        let words = |key: &str| field(key).as_str().unwrap_or_default().to_owned();
        let said = |key: &str| {
            field(key)
                .as_str()
                .filter(|said| !said.is_empty())
                .map(str::to_owned)
        };
        let nodes: Vec<GraphNode> = field("nodes")
            .as_array()
            .map(|nodes| {
                nodes
                    .iter()
                    .map(|node| GraphNode {
                        id: node["id"].as_str().unwrap_or_default().to_owned(),
                        kind: node["kind"].as_str().unwrap_or_default().to_owned(),
                        title: node["title"].as_str().unwrap_or_default().to_owned(),
                        detail: node["detail"].as_str().unwrap_or_default().to_owned(),
                        gives: node["gives"].as_str().unwrap_or_default().to_owned(),
                        line: node["line"].as_u64().map(|line| line as usize),
                        holds: node.clone(),
                    })
                    .filter(|node| !node.id.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let there = |id: &str| nodes.iter().any(|node| node.id == id);
        let edges = field("edges")
            .as_array()
            .map(|edges| {
                edges
                    .iter()
                    .map(|edge| GraphEdge {
                        from: edge["from"].as_str().unwrap_or_default().to_owned(),
                        to: edge["to"].as_str().unwrap_or_default().to_owned(),
                        port: edge["port"].as_str().unwrap_or_default().to_owned(),
                        nth: edge["nth"].as_u64().unwrap_or(0) as usize,
                        gives: edge["gives"].as_str().unwrap_or_default().to_owned(),
                    })
                    .filter(|edge| there(&edge.from) && there(&edge.to))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            media: words("media"),
            recipe: words("recipe"),
            document: field("document").as_bool().unwrap_or(false),
            name: words("name"),
            text: words("text"),
            nodes,
            edges,
            error: said("error"),
            cannot: said("cannot"),
            notes: strings(field("notes")),
            models_error: event.models_error.clone().filter(|said| !said.is_empty()),
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
/// graph of `part`s, and the blanks the shot's steps fill.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workflow {
    pub media: String,
    /// The `generate` line it is of, from 1.
    pub line: usize,
    pub model: String,
    pub name: String,
    /// `own` (the shot's own copy), `file` (the models entry's), or `bundled`.
    pub origin: String,
    /// The shot's own copy, where it has one.
    pub file: String,
    pub about: String,
    /// Its text as read: what an edit says it changes.
    pub text: String,
    /// Each blank by name: what fills it (`says`), what the line gives for it now (`given`).
    pub blanks: serde_json::Map<String, Value>,
    pub graph: Recipe,
}

/// A workflow node's id in the graph.
pub fn part_id(id: &str) -> String {
    format!("part:{id}")
}

impl Workflow {
    /// Reads a `workflow` event's fields, leniently.
    pub fn read(event: &Map<String, Value>) -> Self {
        let field = |key: &str| event.get(key).unwrap_or(&Value::Null);
        let words = |key: &str| field(key).as_str().unwrap_or_default().to_owned();
        let (media, model, name, origin, file) = (
            words("media"),
            words("model"),
            words("name"),
            words("origin"),
            words("file"),
        );
        let mut graph = Recipe {
            media: media.clone(),
            name: model.clone(),
            recipe: if origin == "own" {
                file.clone()
            } else {
                name.clone()
            },
            text: words("text"),
            parts: true,
            ..Recipe::default()
        };
        for node in field("nodes").as_array().into_iter().flatten() {
            let id = value_text(&node["id"]);
            if id.is_empty() {
                continue;
            }
            // By what ComfyUI's own editor calls it where its export says, else by its kind.
            let class = node["class"].as_str().unwrap_or_default().to_owned();
            let title = node["title"]
                .as_str()
                .filter(|title| !title.is_empty())
                .map_or_else(|| class.clone(), str::to_owned);
            let detail = if title == class { String::new() } else { class };
            graph.nodes.push(GraphNode {
                id: part_id(&id),
                kind: "part".into(),
                title,
                detail,
                gives: "file".into(),
                line: None,
                holds: node.clone(),
            });
        }
        let mut edges = Vec::new();
        for node in &graph.nodes {
            for input in node.holds["inputs"].as_array().into_iter().flatten() {
                let Some(from) = input["link"].as_array().and_then(|link| link.first()) else {
                    continue;
                };
                let from = part_id(&value_text(from));
                if graph.node(&from).is_some() {
                    edges.push(GraphEdge {
                        from,
                        to: node.id.clone(),
                        port: input["name"].as_str().unwrap_or_default().to_owned(),
                        nth: 0,
                        gives: "file".into(),
                    });
                }
            }
        }
        graph.edges = edges;
        Self {
            media,
            line: field("line").as_u64().unwrap_or(0) as usize,
            model,
            name,
            origin,
            file,
            about: words("about"),
            text: words("text"),
            blanks: field("blanks").as_object().cloned().unwrap_or_default(),
            graph,
        }
    }

    /// The shot's own copy, which its line names.
    pub fn is_own(&self) -> bool {
        self.origin == "own"
    }

    /// What fills blank `name`, and what the line gives for it now.
    pub fn blank_says(&self, name: &str) -> String {
        let blank = self.blanks.get(name).cloned().unwrap_or_default();
        let says = capitalised(blank["says"].as_str().unwrap_or_default());
        if says.is_empty() {
            return says;
        }
        match &blank["given"] {
            Value::String(given) => format!("{says}. Now: {given}"),
            // A file or a signal it gives none of leaves the node out.
            _ if matches!(blank["by"].as_str(), Some("file" | "signal" | "moves")) => {
                format!("{says}. The line gives none, so the node is left out")
            }
            _ => says,
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

pub fn input_kind(input: &Value) -> InputKind {
    if input.get("link").is_some_and(|link| !link.is_null()) {
        return InputKind::Link;
    }
    if input["blanks"]
        .as_array()
        .is_some_and(|blanks| !blanks.is_empty())
    {
        InputKind::Blank
    } else {
        InputKind::Value
    }
}

/// A workflow's value as a row of it shows it: words as they are, a number
/// as it is written, a list or a table as JSON.
pub fn shown_value(value: &Value) -> String {
    match value {
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(whole), _) => whole.to_string(),
            (None, Some(real)) if real.fract() == 0.0 && real.abs() < 1e15 => {
                format!("{}", real as i64)
            }
            (None, Some(real)) => format!("{real}"),
            _ => number.to_string(),
        },
        other => value_text(other),
    }
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
        if from.kind != to.kind || from.id == to.id {
            return Vec::new();
        }
        let gives = strings(&from.holds["outputs"]);
        return to.holds["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|input| input_kind(input) == InputKind::Link)
            .filter(|input| {
                let takes = input["type"].as_str().unwrap_or_default();
                gives.is_empty() || takes.is_empty() || gives.iter().any(|gives| gives == takes)
            })
            .map(|input| input["name"].as_str().unwrap_or_default().to_owned())
            .collect();
    }
    if from.id == to.id || !to.is_step() {
        return Vec::new();
    }
    if from.is_step() && from.line >= to.line {
        return Vec::new();
    }
    let one = |port: &str| vec![port.to_owned()];
    if to.kind == "render" {
        return if from.kind == "sequence" {
            one("sequence")
        } else {
            Vec::new()
        };
    }
    if to.kind != "model" {
        return Vec::new();
    }
    match from.kind.as_str() {
        "signal" => return one("signal"),
        "moves" => return one("moves"),
        "cast" => return one("cast"),
        "style" => return one("style"),
        "sequence" | "takes" => return Vec::new(),
        _ => {}
    }
    match from.gives.as_str() {
        "video" => {
            let mut ports = one("guide");
            // The guides it reads beside the first, by what each is of.
            ports.extend(
                strings(&to.holds["reads"])
                    .into_iter()
                    .filter(|read| read.starts_with("guide_")),
            );
            // As a first frame it is where it ended: the shot goes on from it.
            ports.extend(["ref".to_owned(), "first".to_owned()]);
            ports
        }
        "image" => ["first", "last", "ref"].map(str::to_owned).to_vec(),
        _ => one("ref"),
    }
}

/// What a step's line writes to read `from`: a clip by its name, one of the
/// cast or a style by its name, a file by its path, a sequence by its id, a
/// step's file by the name it has in braces. Nothing for the takes.
pub fn value_of(from: &GraphNode) -> String {
    let said = |key: &str| from.holds[key].as_str().unwrap_or_default().to_owned();
    match from.kind.as_str() {
        "signal" | "moves" => said("clip"),
        "cast" | "style" => said("name"),
        "file" => said("path"),
        "sequence" => said("sequence"),
        "takes" => String::new(),
        _ => {
            // A program may make several: the first of them.
            let makes = match &from.holds["makes"] {
                Value::Array(names) => names.first().and_then(Value::as_str),
                other => other.as_str(),
            }
            .unwrap_or_default();
            if makes.is_empty() {
                String::new()
            } else {
                format!("{{{makes}}}")
            }
        }
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
    if to.kind != "model" || to.holds.get("reads").is_none_or(Value::is_null) {
        return true;
    }
    port == "cast" || port == "style" || strings(&to.holds["reads"]).iter().any(|read| read == port)
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
    let takes = to.holds["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|known| known["name"] == input)
        .and_then(|known| known["type"].as_str())
        .unwrap_or_default();
    strings(&from.holds["outputs"])
        .iter()
        .position(|gives| gives == takes)
        .unwrap_or(0)
}
