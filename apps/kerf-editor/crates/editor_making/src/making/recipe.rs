//! A shot's graph as the bridge reads it: its nodes and the edges of what
//! each reads ([`Recipe`]); and what may read what, under which name.

use kerf_bridge::event::graph::{Gives, Kind, ModelNode, TakesNode, What};
use kerf_bridge::event::Recipe as RecipeEvent;

/// One node of a shot's graph, as the bridge's `recipe` lays it out.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub gives: Gives,
    /// The step's line, from 1, for a step.
    pub line: Option<usize>,
    /// What else it holds, by its kind.
    pub what: What,
}

impl GraphNode {
    /// What it is in a shot's graph.
    pub fn kind(&self) -> Kind {
        self.what.kind()
    }

    pub fn is(&self, kind: Kind) -> bool {
        self.kind() == kind
    }

    /// A step of the shot: a line of its steps.
    pub fn is_step(&self) -> bool {
        self.line.is_some_and(|line| line > 0)
    }

    /// What its `generate` line asks, for a model.
    pub fn model(&self) -> Option<&ModelNode> {
        match &self.what {
            What::Model(model) => Some(model),
            _ => None,
        }
    }

    /// The shot's takes, for the node that lists them.
    pub fn takes(&self) -> Option<&TakesNode> {
        match &self.what {
            What::Takes(takes) => Some(takes),
            _ => None,
        }
    }

    /// A model not set up on this computer, or a file that is not there.
    pub fn is_missing(&self) -> bool {
        match &self.what {
            What::Model(model) => !model.set_up,
            what => !what.there(),
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

/// A media's steps as a graph (the bridge's `recipe`).
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
                what: node.what.clone(),
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

/// The names `to` could read what `from` gives under, the first the most
/// likely: none where it cannot read it at all. A step reads what a step
/// before it made, never one after; a guide reads a sequence; a model reads
/// a signal, a clip's way, the cast and styles as such, a picture in motion
/// as a guide (or another of the guides it says it reads), a reference or
/// its first frame, and a still as its first or last frame or a reference.
pub fn ports_for(from: &GraphNode, to: &GraphNode) -> Vec<String> {
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
        Kind::Signal => return one("signal"),
        Kind::Moves => return one("moves"),
        Kind::Cast => return one("cast"),
        Kind::Style => return one("style"),
        Kind::Sequence | Kind::Takes => return Vec::new(),
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
    let what = &from.what;
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
