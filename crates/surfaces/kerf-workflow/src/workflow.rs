//! A workflow: a ComfyUI graph with blanks, and what the model it runs makes.
//!
//! ```json
//! {
//!   "kerf-comfy": 1,
//!   "fps": 16,
//!   "frames": { "most": 81, "step": 4 },
//!   "size": [832, 480],
//!   "multiple": 16,
//!   "set": { "steps": 20, "negative": "blurry" },
//!   "workflow": {
//!     "4": { "class_type": "CLIPTextEncode", "inputs": { "clip": ["2", 0], "text": "{prompt}" } },
//!     "6": { "class_type": "LoadVideo", "inputs": { "file": "{guide}" } }
//!   }
//! }
//! ```
//!
//! `workflow` is a graph as ComfyUI exports it for its API. A blank is a name
//! in braces: `{prompt}`, `{seed}`, `{width}`, `{height}`, `{frames}`, and
//! `{fps}` are the request's, `{guide}`, `{first}`, `{last}`, and `{ref}` the
//! files it gives, as the server names them once sent, `{signal}` a signal
//! it gives, as a number for each frame asked for (`0,0.25,0.5`), and any other is a
//! value `set` holds, which the same name on a `generate` line replaces
//! (`steps=30`). A value that is a blank and nothing else takes what fills
//! it as it is, a number as a number; a blank in a longer text is filled in
//! as text.
//!
//! A node that reads a file or a signal the request does not give is left
//! out, and with it whatever cannot do without it, so one workflow serves a
//! shot with a guide, with a reference, with both, or with neither.
//!
//! Where the server lets a node do without an input that this workflow's
//! node is nothing without, a second guide laid over the first, `without`
//! says so, and which node stands in for one left out:
//!
//! ```json
//! "without": { "22": { "wants": ["control_video"], "reads": "9" } }
//! ```
//!
//! Node 22 is left out when its `control_video` is, and what read node 22
//! then reads node 9, output for output. `"given": ["first"]` leaves a node
//! out when the request hands over no `first`, for one that reads nothing of
//! it and is still only of use beside it; `"given": ["prompt"]` leaves one
//! out when the line has no words, for a node that would take none as words
//! all the same. So a workflow can lay one thing over another, each in a
//! node of its own, and still serve a shot that gives only some of them.
//!
//! A workflow whose `frames.most` is 1 makes a still: it needs no `fps`,
//! and what it makes is a picture, saved by a node that saves one.
//!
//! A workflow that reads a picture in motion and makes a file of it, its
//! depth or the camera that filmed it, says `"of": "guide"` and needs no
//! `fps`. What it makes is of its guide frame for frame: `{fps}` is the
//! guide's own rate, `{frames}` each frame it has up to `frames.most`, and
//! `{width}` and `{height}` its own size, down to `size` by area.
//!
//! A workflow that reads a file that is not a picture, a scene as splats,
//! and makes another file of it says `"of": "ref"`: the file is the line's
//! `ref=`, sent as it is, and the workflow has no rate, size, or frames to
//! ask for. `{no_picture}` is a small black picture on the server, for a
//! node that loads a file and will not run without pictures beside it.

use crate::names::{guide_named, names_a_file, names_a_signal, names_what_is_handed, MOVES, MOVES_BOX, MOVES_VIDEO};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The version of workflow file this program reads.
const WORKFLOW_VERSION: u32 = 1;

/// The blank a line's words fill, which a node can be nothing without.
const PROMPT: &str = "prompt";

/// The workflows that come with kerf-comfy, by name.
pub const BUNDLED: [(&str, &str); 15] = [
    ("shot", include_str!("../workflows/shot.json")),
    ("shot-timed", include_str!("../workflows/shot-timed.json")),
    ("shot-into", include_str!("../workflows/shot-into.json")),
    ("shot-led", include_str!("../workflows/shot-led.json")),
    ("shot-directed", include_str!("../workflows/shot-directed.json")),
    ("still", include_str!("../workflows/still.json")),
    ("depth", include_str!("../workflows/depth.json")),
    ("set", include_str!("../workflows/set.json")),
    (
        "cameras",
        include_str!("../workflows/cameras.json"),
    ),
    ("set-views", include_str!("../workflows/set-views.json")),
    ("set-splats", include_str!("../workflows/set-splats.json")),
    ("moves-shown", include_str!("../workflows/moves-shown.json")),
    ("matte", include_str!("../workflows/matte.json")),
    ("matte-shown", include_str!("../workflows/matte-shown.json")),
    ("plate", include_str!("../workflows/plate.json")),
];

/// What fills a workflow's blanks, by name.
pub type Values = BTreeMap<String, Value>;

/// The inputs a kind of node cannot do without, as the server says.
pub type Required<'a> = dyn FnMut(&str) -> Result<BTreeSet<String>, String> + 'a;

/// How many frames the model makes.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct Frames {
    /// The most it makes in one shot.
    pub most: u32,
    /// It makes one frame and a multiple of this many: 4 for 5, 9, … 81.
    pub step: u32,
    /// The fewest frames of a guide it makes a file of: 2 for one that reads them as
    /// views of one scene, which a frame alone is not.
    #[serde(default = "one")]
    pub least: u32,
}

/// A workflow as its file says it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Workflow {
    #[serde(rename = "kerf-comfy")]
    version: u32,
    /// What it makes, in a line.
    #[serde(default)]
    pub about: String,
    /// What it makes a file of: `guide`, frame for frame, `ref`, a file that
    /// is not a picture, or nothing for a model that makes a shot at a rate
    /// and a size of its own.
    #[serde(default)]
    of: Option<String>,
    /// The rate the model makes frames at.
    #[serde(default)]
    pub fps: f64,
    pub frames: Frames,
    /// The picture the model makes, and by its area the largest asked of it.
    pub size: (u32, u32),
    /// Each side is a multiple of this.
    #[serde(default = "one")]
    pub multiple: u32,
    /// The values its other blanks take unless the request says otherwise.
    #[serde(default)]
    pub set: BTreeMap<String, Value>,
    /// The nodes that go when an input of theirs does, by their id.
    #[serde(default)]
    without: BTreeMap<String, Without>,
    #[serde(rename = "workflow")]
    graph: Map<String, Value>,
}

/// What a node is left out for, and what stands in for it then.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Without {
    /// The inputs it is nothing without, though the server lets it run
    /// without them: with one of them left out, so is it.
    #[serde(default)]
    wants: Vec<String>,
    /// What a request hands over that it is nothing without, by the blank's
    /// name (`first`, `guide`), though none of its own inputs reads it: with
    /// one of them not given, it is left out. `prompt` is the line's words,
    /// not given where it has none.
    #[serde(default)]
    given: Vec<String>,
    /// The node that what read it reads in its place, output for output.
    /// With none, what read it loses its link, or goes with it.
    #[serde(default)]
    reads: Option<String>,
}

fn one() -> u32 {
    1
}

impl Workflow {
    /// The workflow `name` names: a file, or one that comes with kerf-comfy.
    pub fn named(name: &str) -> Result<Self, String> {
        if Path::new(name).is_file() {
            let text = std::fs::read_to_string(name)
                .map_err(|error| format!("cannot read the workflow {name}: {error}"))?;
            return Self::parse(&text).map_err(|error| format!("the workflow {name}: {error}"));
        }
        let (_, text) = BUNDLED.iter().find(|(bundled, _)| *bundled == name).ok_or_else(|| {
            format!(
                "no workflow `{name}`: it is neither a file nor one of those that come with kerf-comfy ({})",
                BUNDLED.map(|(bundled, _)| bundled).join(", ")
            )
        })?;
        Self::parse(text).map_err(|error| format!("the workflow {name}: {error}"))
    }

    /// Read a workflow's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let workflow: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if workflow.version != WORKFLOW_VERSION {
            return Err(format!(
                "it is `kerf-comfy` {}, and this program reads {WORKFLOW_VERSION}",
                workflow.version
            ));
        }
        if let Some(of) = workflow
            .of
            .as_deref()
            .filter(|of| !["guide", "ref"].contains(of))
        {
            return Err(format!(
                "`of` is `guide`, `ref`, or is left out, not `{of}`"
            ));
        }
        // A still, one frame, has no rate either.
        if workflow.of.is_none() && workflow.frames.most > 1 && !(workflow.fps.is_finite() && workflow.fps > 0.0) {
            return Err("`fps` is not a rate".to_owned());
        }
        if workflow.frames.most == 0 || workflow.frames.step == 0 {
            return Err("`frames` needs a `most` and a `step` of at least 1".to_owned());
        }
        if workflow.frames.least == 0 || workflow.frames.least > workflow.frames.most {
            return Err("`frames` has a `least` from 1 to its `most`".to_owned());
        }
        if workflow.size.0 == 0 || workflow.size.1 == 0 || workflow.multiple == 0 {
            return Err("`size` and `multiple` cannot be 0".to_owned());
        }
        for (id, node) in &workflow.graph {
            if !(node["class_type"].is_string() && node["inputs"].is_object()) {
                return Err(format!(
                    "node {id} of `workflow` is not a node as ComfyUI exports one for its API, with a `class_type` and `inputs`"
                ));
            }
        }
        for (id, without) in &workflow.without {
            let Some(node) = workflow.graph.get(id) else {
                return Err(format!("`without` names node {id}, which `workflow` has not got"));
            };
            for wanted in &without.wants {
                if link(&node["inputs"][wanted]).is_none() {
                    return Err(format!(
                        "`without` says node {id} wants `{wanted}`, which is not an input of it linked to a node"
                    ));
                }
            }
            for name in &without.given {
                if !names_what_is_handed(name) && name != PROMPT {
                    return Err(format!(
                        "`without` says node {id} wants `{name}` given, which is not something a request hands over"
                    ));
                }
            }
            if let Some(stand) = &without.reads {
                if stand == id || !workflow.graph.contains_key(stand) {
                    return Err(format!(
                        "`without` has node {stand} stand in for node {id}: it is another node of `workflow`"
                    ));
                }
            }
        }
        Ok(workflow)
    }

    /// The node that stands in for `id` among those not in `gone`, if one does.
    fn stand_in<'a>(&'a self, id: &str, gone: &BTreeSet<String>) -> Option<&'a str> {
        let mut at = id;
        // No further than there are nodes, so that two that name each other end.
        for _ in 0..self.without.len() {
            let stand = self.without.get(at)?.reads.as_deref()?;
            if !gone.contains(stand) {
                return Some(stand);
            }
            at = stand;
        }
        None
    }

    /// Whether what it makes is of its guide, frame for frame, and not a
    /// shot at a rate and a size of the model's own.
    pub fn of_guide(&self) -> bool {
        self.of.as_deref() == Some("guide")
    }

    /// Whether what it makes is of a file that is not a picture, the line's `ref=`.
    pub fn of_file(&self) -> bool {
        self.of.as_deref() == Some("ref")
    }

    /// The frames to read of a guide that has `guide` of them: each, up to
    /// the most the workflow reads at once and to `seconds` of them at the
    /// guide's `fps` where that is asked.
    pub fn frames_of(&self, guide: u32, seconds: Option<f64>, fps: f64) -> u32 {
        let asked = seconds.map_or(u32::MAX, |seconds| (seconds * fps).round().max(1.0) as u32);
        guide.min(self.frames.most).min(asked).max(1)
    }

    /// The size to ask for: the one `asked`, or the model's own, with each
    /// side a multiple the model takes. One with more area than the model's
    /// own comes down to it, keeping its shape.
    pub fn size(&self, asked: Option<(u32, u32)>) -> (u32, u32) {
        let (width, height) = asked.unwrap_or(self.size);
        let most = f64::from(self.size.0) * f64::from(self.size.1);
        let area = f64::from(width) * f64::from(height);
        let scale = if area > most {
            (most / area).sqrt()
        } else {
            1.0
        };
        let side = |side: u32| {
            let multiples = (f64::from(side) * scale / f64::from(self.multiple)).round() as u32;
            multiples.max(1) * self.multiple
        };
        (side(width), side(height))
    }

    /// The frames to ask for: `seconds` of them at the model's rate, or as
    /// long as the guide is, or the most the model makes. It is never past
    /// the most, and always a count the model takes, the next one up.
    pub fn frames(&self, seconds: Option<f64>, guide: Option<f64>) -> u32 {
        let step = self.frames.step;
        let most = (self.frames.most.max(1) - 1) / step * step + 1;
        let Some(seconds) = seconds.or(guide) else {
            return most;
        };
        let wanted = (seconds * self.fps).round().clamp(1.0, f64::from(most)) as u32;
        ((wanted - 1).div_ceil(step) * step + 1).min(most)
    }

    /// What the model this workflow runs takes, as a person's models file
    /// says it under `takes` (`docs/shots.md`): the picture it makes, its
    /// rate, the longest shot, the files a `generate` line gives that it
    /// reads, and the other names a line can set, each with the value it
    /// has when the line does not.
    pub fn takes(&self) -> Value {
        let blanks = self.blanks();
        if let Some(of) = self.of.as_deref() {
            // It has no rate and no size of its own to draw a guide at.
            let mut takes = Map::new();
            if !self.about.is_empty() {
                takes.insert("about".to_owned(), Value::from(self.about.clone()));
            }
            takes.insert("of".to_owned(), Value::from(of));
            if self.of_guide() {
                let mut frames = serde_json::json!({"most": self.frames.most});
                if self.frames.least > 1 {
                    frames["least"] = Value::from(self.frames.least);
                }
                takes.insert("frames".to_owned(), frames);
            }
            let mut reads = vec![of];
            // A guide beside the first, by its own name: the mask of what to take out of it.
            reads.extend(
                blanks
                    .iter()
                    .filter(|name| guide_named(name).is_some())
                    .map(String::as_str),
            );
            if blanks.contains(MOVES) {
                reads.push(MOVES);
            }
            takes.insert("reads".to_owned(), Value::from(reads));
            let more: Map<String, Value> = self
                .set
                .iter()
                .filter(|(name, _)| blanks.contains(*name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            if !more.is_empty() {
                takes.insert("more".to_owned(), Value::Object(more));
            }
            return Value::Object(takes);
        }
        let most = self.frames(None, None);
        let mut reads: Vec<&str> = ["guide", "first", "last"]
            .into_iter()
            .filter(|name| blanks.contains(*name))
            .collect();
        // A guide beside the first, by its own name.
        reads.extend(
            blanks
                .iter()
                .filter(|name| guide_named(name).is_some())
                .map(String::as_str),
        );
        let references = blanks
            .iter()
            .filter(|name| name.starts_with("ref") && names_a_file(name))
            .count();
        if references > 0 {
            reads.push("ref");
        }
        let signals = blanks.iter().filter(|name| names_a_signal(name)).count();
        if signals > 0 {
            reads.push("signal");
        }
        if blanks.contains(MOVES)
            || blanks.contains(MOVES_VIDEO)
            || blanks.contains(MOVES_BOX)
        {
            reads.push(MOVES);
        }
        let more: Map<String, Value> = self
            .set
            .iter()
            .filter(|(name, _)| blanks.contains(*name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let mut takes = Map::new();
        if !self.about.is_empty() {
            takes.insert("about".to_owned(), Value::from(self.about.clone()));
        }
        takes.insert(
            "size".to_owned(),
            Value::from(vec![self.size.0, self.size.1]),
        );
        if most == 1 {
            // One frame: a still, which has no rate and no length to say.
            takes.insert("still".to_owned(), Value::from(true));
        } else {
            // A whole rate is written whole.
            takes.insert(
                "rate".to_owned(),
                if self.fps.fract() == 0.0 {
                    Value::from(self.fps as u64)
                } else {
                    Value::from(self.fps)
                },
            );
            takes.insert(
                "seconds".to_owned(),
                Value::from(f64::from(most) / self.fps),
            );
            takes.insert(
                "frames".to_owned(),
                serde_json::json!({"most": most, "step": self.frames.step}),
            );
        }
        takes.insert("reads".to_owned(), Value::from(reads));
        if references > 1 {
            takes.insert("refs".to_owned(), Value::from(references));
        }
        if signals > 1 {
            takes.insert("signals".to_owned(), Value::from(signals));
        }
        if !more.is_empty() {
            takes.insert("more".to_owned(), Value::Object(more));
        }
        Value::Object(takes)
    }

    /// The line's names for the inputs of node `node` that `said`, the
    /// server's word on why it failed, names: `confidence_threshold` is the node's,
    /// `confidence=` the line's. In the order the node has them.
    pub fn line_names(&self, node: &str, said: &str) -> Vec<(String, String)> {
        let Some(inputs) = self
            .graph
            .get(node)
            .and_then(|node| node["inputs"].as_object())
        else {
            return Vec::new();
        };
        inputs
            .iter()
            .filter(|(input, _)| said.contains(input.as_str()))
            .filter_map(|(input, value)| {
                let (name, _) = blanks(value.as_str()?).next()?;
                Some((input.clone(), name.to_owned()))
            })
            .collect()
    }

    /// The names of every blank the graph has.
    pub fn blanks(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for node in self.graph.values() {
            each_text(&node["inputs"], &mut |text| {
                names.extend(blanks(text).map(|(name, _)| name.to_owned()));
            });
        }
        names
    }

    /// The nodes nothing else of the graph reads, by their ids: those that save what it
    /// makes, or show it.
    pub fn ends(&self) -> BTreeSet<String> {
        let mut read = BTreeSet::new();
        for node in self.graph.values() {
            for input in node["inputs"].as_object().into_iter().flat_map(Map::values) {
                read.extend(link(input).map(str::to_owned));
            }
        }
        self.graph
            .keys()
            .filter(|id| !read.contains(*id))
            .cloned()
            .collect()
    }

    /// The names of what a request hands over that the graph has a blank for and `values`
    /// has not got: what a line could give and did not.
    pub fn not_given(&self, values: &Values) -> Vec<String> {
        self.blanks()
            .into_iter()
            .filter(|name| names_what_is_handed(name) && !values.contains_key(name))
            .collect()
    }

    /// The kinds of node its graph has, each once.
    pub fn classes(&self) -> BTreeSet<String> {
        self.graph
            .values()
            .filter_map(|node| node["class_type"].as_str().map(str::to_owned))
            .collect()
    }

    /// The graph to send, with `values` in its blanks. A node with a blank
    /// for a file or a signal that `values` has not got is left out, and so is each
    /// node that `required`, or the workflow's own `without`, says cannot do
    /// without one left out; an input that can loses its link, or reads the
    /// node that stands in for the one left out.
    pub fn graph(&self, values: &Values, required: &mut Required<'_>) -> Result<Value, String> {
        let mut graph = self.graph.clone();
        let mut gone: BTreeSet<String> = graph
            .iter()
            .filter(|(_, node)| {
                let mut missing = false;
                each_text(&node["inputs"], &mut |text| {
                    missing |= blanks(text)
                        .any(|(name, _)| names_what_is_handed(name) && !values.contains_key(name));
                });
                missing
            })
            .map(|(id, _)| id.clone())
            .collect();
        // And each the workflow says is nothing without something the request has not got:
        // a file it does not hand over, or words it has none of.
        let given = |name: &String| values.get(name).is_some_and(|value| value.as_str() != Some(""));
        gone.extend(
            self.without
                .iter()
                .filter(|(_, without)| !without.given.iter().all(given))
                .map(|(id, _)| id.clone()),
        );
        // What a kind of node cannot do without, asked once for each kind.
        let mut needs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        loop {
            graph.retain(|id, _| !gone.contains(id));
            let mut more = Vec::new();
            for (id, node) in &mut graph {
                let class = node["class_type"].as_str().unwrap_or_default().to_owned();
                let Some(inputs) = node["inputs"].as_object_mut() else {
                    continue;
                };
                // What read a node left out reads the one that stands in for it, where one does.
                for value in inputs.values_mut() {
                    let stand = link(value)
                        .filter(|to| gone.contains(*to))
                        .and_then(|to| self.stand_in(to, &gone));
                    if let Some(stand) = stand {
                        value[0] = Value::from(stand);
                    }
                }
                let cut: Vec<String> = inputs
                    .iter()
                    .filter(|(_, value)| link(value).is_some_and(|to| gone.contains(to)))
                    .map(|(name, _)| name.clone())
                    .collect();
                if cut.is_empty() {
                    continue;
                }
                // What this workflow says the node is nothing without, whatever the server lets it.
                let wanted = self
                    .without
                    .get(id)
                    .is_some_and(|without| cut.iter().any(|name| without.wants.contains(name)));
                if wanted {
                    more.push(id.clone());
                    continue;
                }
                if !needs.contains_key(&class) {
                    needs.insert(class.clone(), required(&class)?);
                }
                if cut.iter().any(|name| needs[&class].contains(name)) {
                    more.push(id.clone());
                } else {
                    inputs.retain(|name, _| !cut.contains(name));
                }
            }
            if more.is_empty() {
                break;
            }
            gone.extend(more);
        }
        for node in graph.values_mut() {
            fill(&mut node["inputs"], values)?;
        }
        Ok(Value::Object(graph))
    }
}

/// The node an input is linked to, if it is a link: `["7", 0]`.
fn link(value: &Value) -> Option<&str> {
    match value.as_array()?.as_slice() {
        [Value::String(node), Value::Number(_)] => Some(node),
        _ => None,
    }
}

/// Hand each text in `value` to `read`.
fn each_text(value: &Value, read: &mut dyn FnMut(&str)) {
    match value {
        Value::String(text) => read(text),
        Value::Array(items) => items.iter().for_each(|item| each_text(item, read)),
        Value::Object(fields) => fields.values().for_each(|field| each_text(field, read)),
        _ => {}
    }
}

/// The blanks in `text`, each with where it is: a name of small letters,
/// digits, and `_` in braces. Braces around anything else stand for
/// themselves.
fn blanks(text: &str) -> impl Iterator<Item = (&str, std::ops::Range<usize>)> {
    text.match_indices('{').filter_map(move |(open, _)| {
        let rest = &text[open + 1..];
        let name = &rest[..rest.find('}')?];
        let named = name.starts_with(|first: char| first.is_ascii_lowercase())
            && name.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
            });
        named.then(|| (name, open..open + name.len() + 2))
    })
}

/// Put `values` in every blank under `value`.
fn fill(value: &mut Value, values: &Values) -> Result<(), String> {
    // A link is a node's id and an output, never a blank.
    if link(value).is_some() {
        return Ok(());
    }
    match value {
        Value::String(text) => {
            let found: Vec<(String, std::ops::Range<usize>)> = blanks(text)
                .map(|(name, at)| (name.to_owned(), at))
                .collect();
            let filled = |name: &str| {
                values.get(name).ok_or_else(|| {
                    format!(
                        "the workflow has a blank `{{{name}}}` that nothing fills: the request has no `{name}=`, and the workflow's `set` has none"
                    )
                })
            };
            match found.as_slice() {
                [] => {}
                [(name, at)] if at.len() == text.len() => *value = filled(name)?.clone(),
                _ => {
                    let mut out = String::new();
                    let mut from = 0;
                    for (name, at) in &found {
                        out.push_str(&text[from..at.start]);
                        match filled(name)? {
                            Value::String(word) => out.push_str(word),
                            other => out.push_str(&other.to_string()),
                        }
                        from = at.end;
                    }
                    out.push_str(&text[from..]);
                    *text = out;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                fill(item, values)?;
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                fill(field, values)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// What `text`, written on a `generate` line, is as a value: of the kind
/// `like` is, the workflow's own value for it, or a number if it reads as
/// one where the workflow has none.
pub fn value_of(name: &str, text: &str, like: Option<&Value>) -> Result<Value, String> {
    let number = || {
        text.parse::<i64>().map(Value::from).ok().or_else(|| {
            text.parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .map(Value::from)
        })
    };
    match like {
        Some(Value::Number(_)) => number().ok_or_else(|| {
            format!("`{name}={text}` is not a number, which this workflow takes for `{name}`")
        }),
        Some(Value::Bool(_)) => text
            .parse::<bool>()
            .map(Value::from)
            .map_err(|_| format!("`{name}={text}` is not `true` or `false`")),
        Some(_) => Ok(Value::from(text)),
        None => Ok(number().unwrap_or_else(|| Value::from(text))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundled() -> Workflow {
        Workflow::named("shot").unwrap()
    }

    /// What the server says of the kinds of node the bundled workflow links.
    fn required(class: &str) -> Result<BTreeSet<String>, String> {
        let names: &[&str] = match class {
            "GetVideoComponents" => &["video"],
            "WanVaceToVideo" => &[
                "positive",
                "negative",
                "vae",
                "width",
                "height",
                "length",
                "batch_size",
                "strength",
            ],
            other => return Err(format!("nothing links {other} to a node left out")),
        };
        Ok(names.iter().map(|name| (*name).to_owned()).collect())
    }

    fn values(files: &[(&str, &str)]) -> Values {
        let workflow = bundled();
        let mut values: Values = workflow.set.clone();
        values.extend([
            ("prompt".to_owned(), json!("a boat, at dawn")),
            ("seed".to_owned(), json!(7)),
            ("width".to_owned(), json!(832)),
            ("height".to_owned(), json!(480)),
            ("frames".to_owned(), json!(65)),
            ("fps".to_owned(), json!(workflow.fps)),
        ]);
        values.extend(
            files
                .iter()
                .map(|(name, file)| ((*name).to_owned(), json!(file))),
        );
        values
    }

    #[test]
    fn every_bundled_workflow_reads() {
        for (name, _) in BUNDLED {
            let workflow = Workflow::named(name).unwrap();
            // Each blank is the request's, a file's, or one `set` fills.
            for blank in workflow.blanks() {
                assert!(
                    ["prompt", "seed", "width", "height", "frames", "fps", "no_picture"]
                        .contains(&blank.as_str())
                        || names_what_is_handed(&blank)
                        || workflow.set.contains_key(&blank),
                    "{name}: nothing fills {{{blank}}}"
                );
            }
        }
        // One makes a still: a frame, with no rate or length to say, of a reference where given.
        let still = Workflow::named("still").unwrap();
        let takes = still.takes();
        assert_eq!(takes["still"], true);
        assert_eq!(takes["reads"], json!(["ref"]));
        assert!(takes.get("rate").is_none() && takes.get("seconds").is_none() && takes.get("frames").is_none());
        assert_eq!((still.frames(None, None), still.frames(Some(4.0), None)), (1, 1));
        assert!(still.graph.values().any(|node| node["class_type"] == "SaveImage"));
        // A workflow of more than one frame still has to say its rate.
        let unrated = include_str!("../workflows/shot.json").replacen("\"fps\": 16,", "", 1);
        assert_eq!(Workflow::parse(&unrated).unwrap_err(), "`fps` is not a rate");

        // One reads all four at once, and serves a line that gives any of them.
        let directed = Workflow::named("shot-directed").unwrap();
        assert_eq!(directed.takes()["reads"], json!(["guide", "first", "ref", "signal", "moves"]));
        assert_eq!(directed.takes()["more"]["moves_strength"], json!(1.0));
        let mut needs = |class: &str| -> Result<BTreeSet<String>, String> {
            let names: &[&str] = match class {
                "GetVideoComponents" => &["video"],
                "ImageToMask" => &["image", "channel"],
                "ImageCompositeMasked" => &["destination", "source", "x", "y", "resize_source"],
                "ImageScale" => &["image", "upscale_method", "width", "height", "crop"],
                "ImageFromBatch" => &["image", "batch_index", "length"],
                "ImageBatch" => &["image1", "image2"],
                "WanVaceToVideo" => &["positive", "negative", "vae", "width", "height", "length", "batch_size", "strength"],
                other => return Err(format!("nothing links {other} to a node left out")),
            };
            Ok(names.iter().map(|name| (*name).to_owned()).collect())
        };
        let mut sent = |files: &[(&str, &str)]| {
            let mut given = values(files);
            given.extend(directed.set.clone());
            directed.graph(&given, &mut needs).unwrap()
        };
        let all = sent(&[("guide", "g.mp4"), ("first", "f.png"), ("ref", "r.png"), ("signal_video", "s.mp4"), ("moves_video", "m.mp4")]);
        assert_eq!(all.as_object().unwrap().len(), directed.graph.len());
        // With a first frame the guide starts as that picture, kept: the rest of it is followed.
        assert_eq!(all["9"]["inputs"]["control_video"], json!(["33", 0]));
        assert_eq!(all["9"]["inputs"]["control_masks"], json!(["38", 0]));
        assert_eq!(all["32"]["inputs"]["image"], json!(["24", 0]));
        // No first frame: the guide as it is, and nothing kept of it.
        let whole = sent(&[("guide", "g.mp4"), ("ref", "r.png"), ("signal_video", "s.mp4"), ("moves_video", "m.mp4")]);
        assert!(whole.get("33").is_none() && whole.get("38").is_none() && whole.get("30").is_none());
        assert_eq!(whole["9"]["inputs"]["control_video"], json!(["24", 0]));
        assert!(whole["9"]["inputs"].get("control_masks").is_none());
        // A first frame and no signal: the part of the guide after its first frame, ungated.
        let on = sent(&[("guide", "g.mp4"), ("first", "f.png")]);
        assert_eq!(on["32"]["inputs"]["image"], json!(["7", 0]));
        assert_eq!(on["9"]["inputs"]["control_video"], json!(["33", 0]));
        // A first frame with no guide has nothing to go on along: it is not kept either.
        let unled = sent(&[("first", "f.png"), ("ref", "r.png")]);
        assert!(unled.get("38").is_none() && unled["9"]["inputs"].get("control_video").is_none());
        assert_eq!(all["12"]["inputs"]["positive"], json!(["22", 0]));
        // No signal: the guide is followed all through, with nothing laid over it in grey.
        let unsignalled = sent(&[("guide", "g.mp4"), ("ref", "r.png"), ("moves_video", "m.mp4")]);
        assert!(unsignalled.get("24").is_none());
        assert_eq!(unsignalled["9"]["inputs"]["control_video"], json!(["7", 0]));
        // No moves: what `shot` sends, the sampler reading the guide's node.
        let unmoved = sent(&[("guide", "g.mp4"), ("ref", "r.png")]);
        assert!(unmoved.get("22").is_none() && unmoved.get("20").is_none());
        assert_eq!(unmoved["12"]["inputs"]["latent_image"], json!(["9", 2]));
        assert_eq!(unmoved["13"]["inputs"]["trim_amount"], json!(["9", 3]));
        // A signal with no guide has nothing to let go: the prompt and the layout.
        let unguided = sent(&[("signal_video", "s.mp4"), ("moves_video", "m.mp4")]);
        assert!(unguided.get("24").is_none() && unguided["9"]["inputs"].get("control_video").is_none());
        assert_eq!(unguided["22"]["inputs"]["control_video"], json!(["21", 0]));
        // One that makes a file of its guide reads a guide, and has no rate of its own.
        for name in [
            "depth",
            "set",
            "cameras",
            "set-views",
            "matte",
            "matte-shown",
        ] {
            let workflow = Workflow::named(name).unwrap();
            assert!(
                workflow.of_guide() && workflow.blanks().contains("guide"),
                "{name}"
            );
            let takes = workflow.takes();
            assert_eq!(takes["of"], "guide");
            assert_eq!(takes["reads"], json!(["guide"]));
            assert!(
                takes.get("rate").is_none() && takes.get("size").is_none(),
                "{name}"
            );
            assert!(!takes["about"].as_str().unwrap().is_empty(), "{name}");
        }
        // One that makes a file of a file reads it as the line's `ref=`, and asks for no frames.
        let of_file = Workflow::named("set-splats").unwrap();
        assert!(of_file.of_file() && !of_file.of_guide());
        let takes = of_file.takes();
        assert_eq!(takes["of"], "ref");
        assert_eq!(takes["reads"], json!(["ref"]));
        assert!(takes.get("frames").is_none() && takes.get("rate").is_none());
        // One that draws where clips go over its guide reads both.
        let shown = Workflow::named("moves-shown").unwrap();
        assert_eq!(shown.takes()["reads"], json!(["guide", "moves"]));
        // A camera's path is found only with the model's weights as 32-bit floats, and so is
        // the depth the frames agree on when they are read together.
        for name in ["cameras", "set-views", "depth"] {
            let multiview = Workflow::named(name).unwrap();
            assert!(multiview
                .graph
                .values()
                .any(|node| node["class_type"] == "LoadDA3Model"
                    && node["inputs"]["weight_dtype"] == "fp32"));
        }
        // There is one depth, of the frames read together, and one file of each reading: a
        // camera's path is a text, and its depth is depth's.
        let depth = Workflow::named("depth").unwrap();
        assert!(depth.graph.values().any(|node| node["inputs"]["mode"] == "multiview"));
        assert!(depth.classes().contains("SaveVideo") && !depth.classes().contains("SaveText"));
        let cameras = Workflow::named("cameras").unwrap();
        assert!(cameras.classes().contains("SaveText") && !cameras.classes().contains("SaveVideo"));
        assert!(Workflow::named("depth-whole").is_err());
        // A camera and a set in its space are found of frames seen beside each other, so of
        // two at the least; a depth, and a frame alone as a set, are had of one.
        for (name, least) in [("cameras", 2), ("set-views", 2), ("depth", 1), ("set", 1), ("matte", 1)] {
            let workflow = Workflow::named(name).unwrap();
            assert_eq!(workflow.frames.least, least, "{name}");
            assert_eq!(workflow.takes()["frames"].get("least").is_some(), least > 1, "{name}");
        }
        let missing = Workflow::named("no-such").unwrap_err();
        assert!(missing.contains("shot"), "{missing}");
    }

    /// What the server says of the kinds of node the workflows that find a thing in footage,
    /// and the one that makes a part of it anew, link.
    fn finding(class: &str) -> Result<BTreeSet<String>, String> {
        let names: &[&str] = match class {
            "GetVideoComponents" => &["video"],
            "ImageFromBatch" => &["image", "batch_index", "length"],
            "SAM3_Detect" => &["model", "image", "threshold", "refine_iterations", "individual_masks"],
            "SAM3_VideoTrack" => &["images", "model", "detection_threshold", "max_objects", "detect_interval"],
            "SAM3_TrackToMask" => &["track_data", "object_indices"],
            "SAM3_TrackPreview" => &["track_data", "opacity", "fps"],
            "ImageToMask" => &["image", "channel"],
            "GrowMask" => &["mask", "expand", "tapered_corners"],
            "MaskToImage" => &["mask"],
            "ImageBlur" => &["image", "blur_radius", "sigma"],
            "ImageCompositeMasked" => &["destination", "source", "x", "y", "resize_source"],
            "CreateVideo" => &["images", "fps"],
            "SaveVideo" => &["video", "filename_prefix", "format"],
            "WanVaceToVideo" => &["positive", "negative", "vae", "width", "height", "length", "batch_size", "strength"],
            other => return Err(format!("nothing links {other} to a node left out")),
        };
        Ok(names.iter().map(|name| (*name).to_owned()).collect())
    }

    #[test]
    fn a_matte_is_found_by_the_lines_words_or_by_points_and_never_by_neither() {
        for name in ["matte", "matte-shown"] {
            let matte = Workflow::named(name).unwrap();
            // The node that makes of what was followed what the workflow saves.
            let last = if name == "matte" { "6" } else { "20" };
            let sent = |given: &[(&str, &str)]| {
                let mut filled = values(&[("guide", "g.mp4")]);
                filled.extend(matte.set.clone());
                // As `kerf-comfy` hands them: points a line does not give are not there.
                filled.remove("inside");
                filled.remove("outside");
                filled.extend(given.iter().map(|(name, value)| ((*name).to_owned(), json!(value))));
                matte.graph(&filled, &mut finding).unwrap()
            };
            // Words alone: nothing reads points, and what is followed is what the words find.
            let words = sent(&[("prompt", "black pickup truck")]);
            for gone in ["11", "12", "13", "14"] {
                assert!(words.get(gone).is_none(), "{name}: node {gone}");
            }
            assert_eq!(words["5"]["inputs"]["conditioning"], json!(["4", 0]));
            assert_eq!(words[last]["inputs"]["track_data"], json!(["5", 0]));
            // Points alone: the thing they are on in the first frame is followed, and no words
            // are read, an empty text being words to the model all the same.
            let points = sent(&[("prompt", ""), ("inside", "[{\"x\":458,\"y\":384}]")]);
            assert!(points.get("4").is_none() && points.get("12").is_none(), "{name}");
            assert_eq!(points["13"]["inputs"]["positive_coords"], json!(["11", 0]));
            assert!(points["13"]["inputs"].get("negative_coords").is_none());
            assert_eq!(points["14"]["inputs"]["initial_mask"], json!(["13", 0]));
            assert_eq!(points[last]["inputs"]["track_data"], json!(["14", 0]));
            // Both: the points say which thing, so the words are not asked as well.
            let both = sent(&[("prompt", "truck"), ("inside", "[]"), ("outside", "[]")]);
            assert_eq!(both["13"]["inputs"]["negative_coords"], json!(["12", 0]));
            assert_eq!(both[last]["inputs"]["track_data"], json!(["14", 0]));
            assert!(both["14"]["inputs"].get("conditioning").is_none());
            // Neither: the node that follows is sent with nothing to find, and the server refuses it.
            let neither = sent(&[("prompt", "")]);
            assert!(neither.get("4").is_none() && neither.get("13").is_none());
            assert!(neither["5"]["inputs"].get("conditioning").is_none());
            assert_eq!(neither[last]["inputs"]["track_data"], json!(["5", 0]));
            // A line sets how many things, and of a matte which of them.
            let more = &matte.takes()["more"];
            assert_eq!((&more["objects"], &more["inside"]), (&json!(4), &json!("")));
            assert_eq!(more.get("which").is_some(), name == "matte");
        }
        // What is shown is saved by the node that draws it, which is all the workflow saves.
        let shown = Workflow::named("matte-shown").unwrap();
        assert!(!shown.classes().contains("SaveVideo") && shown.classes().contains("SAM3_TrackPreview"));
    }

    #[test]
    fn what_is_made_anew_in_a_part_is_laid_back_over_the_guide_through_the_mask() {
        let into = Workflow::named("shot-into").unwrap();
        assert_eq!(into.takes()["reads"], json!(["guide", "guide_mask"]));
        assert_eq!((&into.takes()["more"]["grow"], &into.takes()["more"]["feather"]), (&json!(12), &json!(9)));
        let sent = |files: &[(&str, &str)]| {
            let mut given = values(files);
            given.extend(into.set.clone());
            into.graph(&given, &mut finding).unwrap()
        };
        // A guide and its mask: the model is told the mask grown, and its picture is laid over
        // the guide's own through that mask, softened.
        let whole = sent(&[("guide", "g.mp4"), ("guide_mask", "m.mp4")]);
        assert_eq!(whole.as_object().unwrap().len(), into.graph.len());
        assert_eq!(whole["25"]["inputs"]["expand"], json!(12));
        assert_eq!(whole["9"]["inputs"]["control_masks"], json!(["25", 0]));
        assert_eq!(whole["29"]["inputs"]["destination"], json!(["7", 0]));
        assert_eq!(whole["29"]["inputs"]["source"], json!(["14", 0]));
        assert_eq!(whole["29"]["inputs"]["mask"], json!(["28", 0]));
        assert_eq!(whole["15"]["inputs"]["images"], json!(["29", 0]));
        // No mask: nothing is kept, and the model's picture is the take.
        let unmasked = sent(&[("guide", "g.mp4")]);
        for gone in ["17", "18", "19", "25", "26", "27", "28"] {
            assert!(unmasked.get(gone).is_none(), "node {gone}");
        }
        assert!(unmasked["29"]["inputs"].get("mask").is_none());
        assert!(unmasked["9"]["inputs"].get("control_masks").is_none());
        // No guide: there is nothing to lay it over, and what is saved is the model's own.
        let unguided = sent(&[]);
        assert!(unguided.get("29").is_none() && unguided.get("21").is_none());
        assert_eq!(unguided["15"]["inputs"]["images"], json!(["14", 0]));
        assert!(unguided["9"]["inputs"].get("control_video").is_none());
    }

    #[test]
    fn a_plate_is_of_its_guide_and_its_mask_and_nothing_without_the_mask() {
        let plate = Workflow::named("plate").unwrap();
        assert!(plate.of_guide());
        // It reads a second guide beside the first, as a shot that makes a part anew does.
        let takes = plate.takes();
        assert_eq!(takes["reads"], json!(["guide", "guide_mask"]));
        assert_eq!(takes["frames"], json!({"most": 197}));
        let more = &takes["more"];
        assert_eq!((&more["grow"], &more["reach"], &more["refine"]), (&json!(8), &json!(0), &json!(false)));
        // What it saves is the one node nothing reads.
        assert_eq!(plate.ends(), BTreeSet::from(["66".to_owned()]));
        let mut needs = |class: &str| -> Result<BTreeSet<String>, String> {
            let names: &[&str] = match class {
                "GetVideoComponents" => &["video"],
                "ImageToMask" => &["image", "channel"],
                "GrowMask" => &["mask", "expand", "tapered_corners"],
                "MaskToImage" => &["mask"],
                "ImageFromBatch" => &["image", "batch_index", "length"],
                "RepeatImageBatch" => &["image", "amount"],
                "ImageBatch" => &["image1", "image2"],
                "ImageScale" => &["image", "upscale_method", "width", "height", "crop"],
                "ThresholdMask" => &["mask", "value"],
                "ImageBlur" => &["image", "blur_radius", "sigma"],
                "VOIDInpaintConditioning" => {
                    &["positive", "negative", "vae", "video", "quadmask", "width", "height", "length", "batch_size"]
                }
                "CFGGuider" => &["model", "positive", "negative", "cfg"],
                "SamplerCustomAdvanced" => &["noise", "guider", "sampler", "sigmas", "latent_image"],
                "VAEDecode" => &["samples", "vae"],
                "VOIDWarpedNoise" => &["optical_flow", "video", "width", "height", "length", "batch_size"],
                "VOIDWarpedNoiseSource" => &["warped_noise"],
                "ComfySwitchNode" => &["switch"],
                "ImageCompositeMasked" => &["destination", "source", "x", "y", "resize_source"],
                "CreateVideo" => &["images", "fps"],
                "SaveVideo" => &["video", "filename_prefix", "format"],
                other => return Err(format!("nothing links {other} to a node left out")),
            };
            Ok(names.iter().map(|name| (*name).to_owned()).collect())
        };
        let mut sent = |files: &[(&str, &str)]| {
            let mut given = values(files);
            given.extend(plate.set.clone());
            (plate.not_given(&given), plate.graph(&given, &mut needs).unwrap())
        };
        // Both given: all of it, the model told the mask grown, and its picture laid back
        // through the mask grown that much further again.
        let (wanted, whole) = sent(&[("guide", "g.mp4"), ("guide_mask", "m.mp4")]);
        assert!(wanted.is_empty());
        assert_eq!(whole.as_object().unwrap().len(), plate.graph.len());
        assert_eq!(whole["34"]["inputs"]["quadmask"], json!(["22", 0]));
        assert_eq!((&whole["6"]["inputs"]["expand"], &whole["8"]["inputs"]["expand"]), (&json!(8), &json!(0)));
        assert_eq!(whole["64"]["inputs"]["destination"], json!(["2", 0]));
        // The model takes five frames and a multiple of eight, and a guide has what it has:
        // it is read with its last frame held to the next such count, and cut back to its own.
        assert_eq!(whole["12"]["inputs"]["expression"], "ceil(max(a - 5, 0) / 8) * 8 + 5");
        assert_eq!(whole["12"]["inputs"]["values.a"], json!(65));
        assert_eq!(whole["34"]["inputs"]["length"], json!(["12", 1]));
        assert_eq!(whole["60"]["inputs"]["length"], json!(65));
        // One pass, or the second over it, is a value of the line's, read where the picture
        // is chosen: the pass not chosen is not run.
        assert_eq!(whole["58"]["inputs"], json!({"switch": false, "on_true": ["57", 0], "on_false": ["46", 0]}));
        // No mask: the model has nothing to take out, and nothing is left that saves.
        let (wanted, unmasked) = sent(&[("guide", "g.mp4")]);
        assert_eq!(wanted, ["guide_mask"]);
        assert!(plate.ends().iter().all(|end| unmasked.get(end).is_none()));
        assert!(unmasked.get("34").is_none() && unmasked.get("64").is_none());
    }

    #[test]
    fn the_files_the_workflows_load_are_the_ones_a_server_is_said_to_hold() {
        // `docs/shots.md`, What the server holds: a workflow that loads another file, or
        // one that no longer loads one, changes that table with this.
        let mut loaded: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for (name, _) in BUNDLED {
            for value in Workflow::named(name).unwrap().set.values() {
                if let Some(file) = value.as_str().filter(|file| file.ends_with(".safetensors")) {
                    loaded.entry(file.to_owned()).or_default().push(name);
                }
            }
        }
        // Every shot is made by one model, with its picture's coder and its words' reader.
        let shots = vec!["shot", "shot-timed", "shot-into", "shot-led", "shot-directed", "still"];
        let held: BTreeMap<String, Vec<&str>> = [
            ("wan2.1_vace_1.3B_fp16.safetensors", shots.clone()),
            ("wan_2.1_vae.safetensors", shots.clone()),
            ("umt5_xxl_fp16.safetensors", shots),
            ("depth_anything_3_base.safetensors", vec!["depth", "cameras", "set-views"]),
            ("moge_3_vitl_fp16.safetensors", vec!["set"]),
            ("sam3.1_multiplex_fp16.safetensors", vec!["matte", "matte-shown"]),
            // The model that takes a thing out, in two passes, with its own coder, its own
            // words' reader, and the flow its second pass is led by.
            ("void_pass1.safetensors", vec!["plate"]),
            ("void_pass2.safetensors", vec!["plate"]),
            ("cogvideox_vae.safetensors", vec!["plate"]),
            ("t5xxl_fp16.safetensors", vec!["plate"]),
            ("raft_large_C_T_SKHT_V2-ff5fadd5.safetensors", vec!["plate"]),
        ]
        .into_iter()
        .map(|(file, by)| (file.to_owned(), by))
        .collect();
        assert_eq!(loaded, held);
    }

    #[test]
    fn a_workflow_says_what_its_model_takes() {
        let takes = bundled().takes();
        assert_eq!(takes["size"], json!([832, 480]));
        assert_eq!(takes["rate"], json!(16));
        // 81 frames at 16 a second.
        assert_eq!(takes["seconds"], json!(5.0625));
        assert_eq!(takes["frames"], json!({"most": 81, "step": 4}));
        assert_eq!(takes["reads"], json!(["guide", "ref"]));
        assert_eq!(takes["more"]["steps"], json!(20));
        assert_eq!(takes["more"]["strength"], json!(1.0));
        assert!(takes["about"].as_str().unwrap().starts_with("A shot that follows a guide"));
        assert!(takes.get("refs").is_none(), "one reference needs no count");

        // A model that reads first and last frames and two references, at a rate that is not
        // whole, and that makes fewer frames than `most` says where its step does not reach it.
        let other = Workflow::parse(
            r#"{
                "kerf-comfy": 1, "fps": 23.976, "frames": {"most": 50, "step": 8}, "size": [1280, 720],
                "set": {"steps": 30, "unused": 1},
                "workflow": {
                    "1": {"class_type": "A", "inputs": {"a": "{first}", "b": "{last}", "c": "{ref}", "d": "{ref2}", "e": "{steps}", "f": "{prompt}"}}
                }
            }"#,
        )
        .unwrap();
        let takes = other.takes();
        assert_eq!(takes["rate"], json!(23.976));
        assert_eq!(takes["frames"], json!({"most": 49, "step": 8}));
        assert_eq!(takes["reads"], json!(["first", "last", "ref"]));
        assert_eq!(takes["refs"], json!(2));
        // What `set` holds with no blank to fill is nothing a line can set.
        assert_eq!(takes["more"], json!({"steps": 30}));
        assert!(takes.get("about").is_none());
    }

    #[test]
    fn what_is_wrong_with_a_workflow_is_said() {
        let wrong = |text: &str| Workflow::parse(text).unwrap_err();
        let whole = |fields: &str| {
            format!(
                r#"{{"kerf-comfy": 1, "fps": 16, "frames": {{"most": 81, "step": 4}}, "size": [832, 480], {fields}}}"#
            )
        };
        assert!(Workflow::parse(&whole(r#""workflow": {}"#)).is_ok());
        assert!(wrong(
            &whole(r#""workflow": {}"#).replace("\"kerf-comfy\": 1", "\"kerf-comfy\": 3")
        )
        .contains("this program reads 1"));
        assert!(
            wrong(&whole(r#""workflow": {}"#).replace("\"fps\": 16", "\"fps\": 0"))
                .contains("not a rate")
        );
        assert!(
            wrong(&whole(r#""workflow": {}"#).replace("\"step\": 4", "\"step\": 0"))
                .contains("at least 1")
        );
        for least in ["0", "82"] {
            assert!(wrong(&whole(r#""workflow": {}"#).replace("\"step\": 4", &format!("\"step\": 4, \"least\": {least}")))
                .contains("`least` from 1 to its `most`"));
        }
        // With no rate it makes a file of its guide, and of nothing else.
        let of = |of: &str| {
            whole(r#""workflow": {}"#).replace("\"fps\": 16", &format!("\"of\": \"{of}\""))
        };
        assert!(Workflow::parse(&of("guide")).unwrap().of_guide());
        assert!(wrong(&of("first")).contains("`of` is `guide`"));
        // A graph saved from ComfyUI's canvas is not the one it exports for its API.
        assert!(wrong(&whole(r#""workflow": {"nodes": [], "links": []}"#))
            .contains("exports one for its API"));
    }

    #[test]
    fn a_size_is_one_the_model_takes() {
        let workflow = bundled();
        assert_eq!(workflow.size(None), (832, 480));
        // Smaller is as asked, to the multiple; larger comes down to the model's area.
        assert_eq!(workflow.size(Some((416, 240))), (416, 240));
        assert_eq!(workflow.size(Some((480, 832))), (480, 832));
        assert_eq!(workflow.size(Some((500, 300))), (496, 304));
        assert_eq!(workflow.size(Some((1920, 1080))), (848, 480));
        assert_eq!(workflow.size(Some((1080, 1920))), (480, 848));
        assert_eq!(workflow.size(Some((3, 3))), (16, 16));
    }

    #[test]
    fn frames_are_a_count_the_model_takes() {
        let workflow = bundled();
        // Nothing asked, and no guide: the most.
        assert_eq!(workflow.frames(None, None), 81);
        // Four seconds is 64 frames at 16 a second, and the model takes 65: never less than asked.
        assert_eq!(workflow.frames(Some(4.0), None), 65);
        assert_eq!(workflow.frames(Some(4.0), Some(2.0)), 65);
        assert_eq!(workflow.frames(None, Some(2.0)), 33);
        assert_eq!(workflow.frames(Some(0.01), None), 1);
        assert_eq!(workflow.frames(Some(5.0), None), 81);
        // Past the most is the most.
        assert_eq!(workflow.frames(Some(30.0), None), 81);
        let odd = Workflow::parse(
            r#"{"kerf-comfy": 1, "fps": 24, "frames": {"most": 100, "step": 8}, "size": [64, 64], "workflow": {}}"#,
        )
        .unwrap();
        assert_eq!(odd.frames(None, None), 97);
        assert_eq!(odd.frames(Some(1.0), None), 25);
    }

    #[test]
    fn a_file_of_a_guide_has_the_guides_frames_and_its_size() {
        let reader = Workflow::named("depth").unwrap();
        // Each frame the guide has, up to what is asked and to the most read at once.
        assert_eq!(reader.frames_of(65, None, 16.0), 65);
        assert_eq!(reader.frames_of(65, Some(2.0), 16.0), 32);
        assert_eq!(reader.frames_of(100_000, None, 24.0), reader.frames.most);
        assert_eq!(reader.frames_of(0, None, 24.0), 1);
        // The guide's own size, and one past the largest read comes down in its shape.
        assert_eq!(reader.size(Some((832, 480))), (832, 480));
        assert_eq!(reader.size(Some((3840, 2160))), (1920, 1080));
        assert_eq!(reader.size(Some((853, 481))), (854, 482));
    }

    #[test]
    fn a_blank_takes_its_value_as_it_is_and_in_a_text_as_text() {
        let workflow = bundled();
        let graph = workflow
            .graph(
                &values(&[("guide", "kerf-1.mp4"), ("ref", "kerf-2.png")]),
                &mut required,
            )
            .unwrap();
        assert_eq!(graph["4"]["inputs"]["text"], "a boat, at dawn");
        assert_eq!(graph["6"]["inputs"]["file"], "kerf-1.mp4");
        assert_eq!(graph["8"]["inputs"]["image"], "kerf-2.png");
        // A number stays a number, and a link stays a link.
        assert_eq!(graph["9"]["inputs"]["width"], json!(832));
        assert_eq!(graph["9"]["inputs"]["length"], json!(65));
        assert_eq!(graph["9"]["inputs"]["control_video"], json!(["7", 0]));
        assert_eq!(graph["12"]["inputs"]["seed"], json!(7));
        assert_eq!(graph["12"]["inputs"]["steps"], json!(20));
        assert_eq!(graph["15"]["inputs"]["fps"], json!(16.0));
        assert_eq!(
            graph["1"]["inputs"]["unet_name"],
            "wan2.1_vace_1.3B_fp16.safetensors"
        );
        assert_eq!(graph.as_object().unwrap().len(), 16);

        let mut filled = json!({"a": "{prompt}, shot on {frames} frames", "b": "{\"k\": 1} {Seed} {}", "c": ["{seed}", 2, 3]});
        fill(&mut filled, &values(&[])).unwrap();
        assert_eq!(filled["a"], "a boat, at dawn, shot on 65 frames");
        assert_eq!(filled["b"], "{\"k\": 1} {Seed} {}");
        assert_eq!(filled["c"], json!([7, 2, 3]));
        let unfilled = fill(&mut json!("{denoise}"), &values(&[])).unwrap_err();
        assert!(
            unfilled.contains("blank `{denoise}` that nothing fills"),
            "{unfilled}"
        );
    }

    #[test]
    fn a_node_whose_file_is_not_given_goes_with_what_cannot_do_without_it() {
        let workflow = bundled();
        let nodes = |files: &[(&str, &str)]| {
            let graph = workflow.graph(&values(files), &mut required).unwrap();
            let shot = graph["9"]["inputs"].as_object().unwrap().clone();
            (graph.as_object().unwrap().len(), shot)
        };
        // No reference: its loader goes, and the link to it.
        let (count, shot) = nodes(&[("guide", "kerf-1.mp4")]);
        assert_eq!(count, 15);
        assert!(shot.contains_key("control_video") && !shot.contains_key("reference_image"));
        // No guide: its loader goes, and what takes it apart, which cannot do without it.
        let (count, shot) = nodes(&[("ref", "kerf-2.png")]);
        assert_eq!(count, 14);
        assert!(!shot.contains_key("control_video") && shot.contains_key("reference_image"));
        // Neither: the prompt alone.
        let (count, shot) = nodes(&[]);
        assert_eq!(count, 13);
        assert!(!shot.contains_key("control_video") && !shot.contains_key("reference_image"));
        assert_eq!(shot["positive"], json!(["4", 0]));
        // A server that cannot say what a node needs stops it.
        let failed = workflow
            .graph(&values(&[]), &mut |_| Err("the server is away".to_owned()))
            .unwrap_err();
        assert_eq!(failed, "the server is away");
    }

    #[test]
    fn a_node_the_workflow_says_is_nothing_without_an_input_goes_and_another_stands_in() {
        // A second guide laid over the first, in a node of its own: the server lets that node
        // run with no guide, which would lay nothing over it at full strength.
        let laid = |without: &str| {
            Workflow::parse(&format!(
                r#"{{
                "kerf-comfy": 1, "fps": 16, "frames": {{"most": 81, "step": 4}}, "size": [832, 480],
                {without}
                "workflow": {{
                    "4": {{"class_type": "CLIPTextEncode", "inputs": {{"text": "{{prompt}}"}}}},
                    "6": {{"class_type": "LoadVideo", "inputs": {{"file": "{{guide}}"}}}},
                    "7": {{"class_type": "GetVideoComponents", "inputs": {{"video": ["6", 0]}}}},
                    "9": {{"class_type": "WanVaceToVideo", "inputs": {{"positive": ["4", 0], "control_video": ["7", 0]}}}},
                    "20": {{"class_type": "LoadVideo", "inputs": {{"file": "{{moves_video}}"}}}},
                    "21": {{"class_type": "GetVideoComponents", "inputs": {{"video": ["20", 0]}}}},
                    "22": {{"class_type": "WanVaceToVideo", "inputs": {{"positive": ["9", 0], "control_video": ["21", 0]}}}},
                    "12": {{"class_type": "KSampler", "inputs": {{"positive": ["22", 0], "latent_image": ["22", 2]}}}}
                }}
            }}"#
            ))
        };
        let workflow = laid(r#""without": {"22": {"wants": ["control_video"], "reads": "9"}, "9": {"wants": ["control_video"], "reads": "4"}},"#).unwrap();
        let graph = |files: &[(&str, &str)]| {
            let mut values = Values::new();
            values.insert("prompt".to_owned(), json!("a ball"));
            values.extend(files.iter().map(|(name, file)| ((*name).to_owned(), json!(file))));
            workflow.graph(&values, &mut required).unwrap()
        };
        // Both given: as written.
        let both = graph(&[("guide", "g.mp4"), ("moves_video", "m.mp4")]);
        assert_eq!(both.as_object().unwrap().len(), 8);
        assert_eq!(both["12"]["inputs"]["positive"], json!(["22", 0]));
        // No moves: the node that lays them over goes, and the sampler reads the one before it,
        // output for output.
        let guided = graph(&[("guide", "g.mp4")]);
        assert!(guided.get("22").is_none() && guided.get("21").is_none());
        assert_eq!(guided["12"]["inputs"]["positive"], json!(["9", 0]));
        assert_eq!(guided["12"]["inputs"]["latent_image"], json!(["9", 2]));
        // No guide: the first goes, and the second reads what it read.
        let moved = graph(&[("moves_video", "m.mp4")]);
        assert!(moved.get("9").is_none());
        assert_eq!(moved["22"]["inputs"]["positive"], json!(["4", 0]));
        // Neither: one stand-in after another, to the prompt.
        let alone = graph(&[]);
        assert_eq!(alone.as_object().unwrap().len(), 2);
        assert_eq!(alone["12"]["inputs"]["positive"], json!(["4", 0]));
        // What it says is held to the graph it says it of.
        for (without, why) in [
            (r#""without": {"30": {}},"#, "names node 30"),
            (r#""without": {"22": {"wants": ["strength"]}},"#, "wants `strength`, which is not an input of it linked to a node"),
            (r#""without": {"22": {"reads": "22"}},"#, "has node 22 stand in for node 22"),
            (r#""without": {"22": {"reads": "31"}},"#, "has node 31 stand in for node 22"),
            (r#""without": {"22": {"given": ["steps"]}},"#, "wants `steps` given, which is not something a request hands over"),
        ] {
            let refused = laid(without).unwrap_err();
            assert!(refused.contains(why), "{refused}");
        }
    }

    #[test]
    fn a_node_that_is_nothing_without_words_goes_when_the_line_has_none() {
        let worded = Workflow::parse(
            r#"{
                "kerf-comfy": 1, "of": "guide", "frames": {"most": 81, "step": 1}, "size": [832, 480],
                "without": {"1": {"given": ["prompt"]}},
                "workflow": {
                    "1": {"class_type": "Words", "inputs": {"text": "{prompt}"}},
                    "2": {"class_type": "Finder", "inputs": {"words": ["1", 0], "count": 4}}
                }
            }"#,
        )
        .unwrap();
        let mut needs = |_: &str| Ok(BTreeSet::from(["count".to_owned()]));
        let mut sent = |prompt: &str| {
            let values: Values = [("prompt".to_owned(), json!(prompt))].into();
            worded.graph(&values, &mut needs).unwrap()
        };
        assert_eq!(sent("a ball")["2"]["inputs"]["words"], json!(["1", 0]));
        // No words: an empty text would still be words to the node that reads them.
        let silent = sent("");
        assert!(silent.get("1").is_none());
        assert_eq!(silent["2"]["inputs"], json!({"count": 4}));
    }

    #[test]
    fn a_signal_fills_its_blank_or_its_node_goes() {
        let cued = Workflow::parse(
            r#"{
                "kerf-comfy": 1, "fps": 16, "frames": {"most": 81, "step": 4}, "size": [832, 480],
                "workflow": {
                    "1": {"class_type": "Words", "inputs": {"text": "{prompt}"}},
                    "2": {"class_type": "Values", "inputs": {"values": "{signal}"}},
                    "3": {"class_type": "Sampler", "inputs": {"words": ["1", 0], "cue": ["2", 0]}}
                }
            }"#,
        )
        .unwrap();
        assert_eq!(cued.takes()["reads"], json!(["signal"]));
        assert!(cued.takes().get("signals").is_none(), "one needs no count");
        let mut needs = |_: &str| Ok(BTreeSet::from(["words".to_owned()]));
        let mut values: Values = [("prompt".to_owned(), json!("a wave"))].into();
        // No signal: the node that reads one goes, and the link to it.
        let without = cued.graph(&values, &mut needs).unwrap();
        assert!(without.get("2").is_none());
        assert_eq!(without["3"]["inputs"], json!({"words": ["1", 0]}));
        // One given: its numbers, as text.
        values.insert("signal".to_owned(), json!("0,0.5,1"));
        let with = cued.graph(&values, &mut needs).unwrap();
        assert_eq!(with["2"]["inputs"]["values"], "0,0.5,1");
        assert_eq!(with["3"]["inputs"]["cue"], json!(["2", 0]));
    }

    #[test]
    fn a_value_written_on_the_line_is_of_the_workflows_kind() {
        assert_eq!(
            value_of("steps", "30", Some(&json!(20))).unwrap(),
            json!(30)
        );
        assert_eq!(
            value_of("cfg", "4.5", Some(&json!(6.0))).unwrap(),
            json!(4.5)
        );
        assert_eq!(
            value_of("verbose", "true", Some(&json!(false))).unwrap(),
            json!(true)
        );
        assert_eq!(
            value_of("negative", "12", Some(&json!("blurry"))).unwrap(),
            json!("12")
        );
        assert_eq!(value_of("lora", "0.8", None).unwrap(), json!(0.8));
        assert_eq!(value_of("lora", "soft", None).unwrap(), json!("soft"));
        assert!(value_of("steps", "many", Some(&json!(20)))
            .unwrap_err()
            .contains("not a number"));
        assert!(value_of("verbose", "yes", Some(&json!(false)))
            .unwrap_err()
            .contains("`true` or `false`"));
    }
}
