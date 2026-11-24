//! A workflow as an editor shows it, and one input of it set in its text.
//!
//! A workflow is a ComfyUI graph with blanks ([`crate::workflow`]). An
//! editor that lets a person see how a model is run shows every node of it
//! and every input of each, and says of each input what it is: a link to
//! another node, a blank a request fills, or a value the workflow itself
//! sets, which is the person's to change:
//!
//! ```json
//! "8": { "class_type": "KSampler", "inputs": {
//!   "model": ["1", 0],          a link
//!   "seed": "{seed}",           a blank: the line's seed
//!   "steps": "{steps}",         a blank: 20 unless the line says steps=
//!   "sampler_name": "uni_pc"    a value, to set here
//! } }
//! ```
//!
//! [`described`] says that, with the nodes and their inputs in the order
//! the file has them. [`set_input`] changes one value in the file's text
//! and nothing else of it: the copy a person keeps of a workflow that comes
//! with kerf-comfy then differs from it in what they changed, and the
//! rest reads as it was written. A link and a blank are not set here: a
//! link is the graph's shape, and a blank is the recipe's to fill.
//!
//! Nothing here asks a server. ComfyUI is another program, asked over HTTP
//! by kerf-comfy when a shot is made; this reads a file.

use crate::names::{
    guide_named, names_a_file, names_a_signal, names_points, MOVES, MOVES_BOX, MOVES_VIDEO,
};
use crate::workflow::Workflow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Number, Value};
use std::ops::Range;

/// A JSON text read only as far as where each thing in it is.
struct Scan<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Scan<'a> {
    fn space(&mut self) {
        let rest = &self.text[self.at..];
        self.at += rest.len() - rest.trim_start().len();
    }

    fn next(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.space();
        if self.next() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!(
                "the workflow is not JSON: `{}` was expected at byte {}",
                byte as char, self.at
            ))
        }
    }

    /// Where the string that starts here is, quotes and all.
    fn string(&mut self) -> Result<Range<usize>, String> {
        self.space();
        let start = self.at;
        self.expect(b'"')?;
        loop {
            match self.next() {
                Some(b'\\') => self.at += 2,
                Some(b'"') => {
                    self.at += 1;
                    return Ok(start..self.at);
                }
                Some(_) => self.at += 1,
                None => return Err("the workflow is not JSON: a text is not closed".to_owned()),
            }
        }
    }

    /// Where the value that starts here is, whatever it is.
    fn value(&mut self) -> Result<Range<usize>, String> {
        self.space();
        let start = self.at;
        match self.next() {
            Some(b'{') => self.object(&mut |scan, _| scan.value().map(drop))?,
            Some(b'[') => {
                self.at += 1;
                self.space();
                if self.next() == Some(b']') {
                    self.at += 1;
                } else {
                    loop {
                        self.value()?;
                        self.space();
                        match self.next() {
                            Some(b',') => self.at += 1,
                            Some(b']') => {
                                self.at += 1;
                                break;
                            }
                            _ => {
                                return Err(
                                    "the workflow is not JSON: a list is not closed".to_owned()
                                )
                            }
                        }
                    }
                }
            }
            Some(b'"') => {
                self.string()?;
            }
            Some(_) => {
                // A number, true, false, or null: up to what ends it.
                while self.next().is_some_and(|byte| {
                    !matches!(byte, b',' | b'}' | b']') && !byte.is_ascii_whitespace()
                }) {
                    self.at += 1;
                }
                if self.at == start {
                    return Err(format!(
                        "the workflow is not JSON: a value was expected at byte {start}"
                    ));
                }
            }
            None => {
                return Err(
                    "the workflow is not JSON: it ends where a value was expected".to_owned(),
                )
            }
        }
        Ok(start..self.at)
    }

    /// Read the object that starts here: `each` is handed each key, with
    /// the scan at its value, which it reads.
    fn object(
        &mut self,
        each: &mut dyn FnMut(&mut Self, String) -> Result<(), String>,
    ) -> Result<(), String> {
        self.expect(b'{')?;
        self.space();
        if self.next() == Some(b'}') {
            self.at += 1;
            return Ok(());
        }
        loop {
            let key = self.string()?;
            let key: String = serde_json::from_str(&self.text[key])
                .map_err(|error| format!("the workflow is not JSON: {error}"))?;
            self.expect(b':')?;
            each(self, key)?;
            self.space();
            match self.next() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(());
                }
                _ => return Err("the workflow is not JSON: an object is not closed".to_owned()),
            }
        }
    }
}

/// One input of a node: its name, what the file has for it, and where in the file.
struct Input {
    name: String,
    value: Value,
    at: Range<usize>,
}

/// One node of the graph, as the file has it.
struct Node {
    id: String,
    class: String,
    title: Option<String>,
    inputs: Vec<Input>,
}

/// The nodes of the workflow `text` holds, and their inputs, in the file's order.
fn nodes(text: &str) -> Result<Vec<Node>, String> {
    let parse = |at: &Range<usize>| -> Result<Value, String> {
        serde_json::from_str(&text[at.clone()])
            .map_err(|error| format!("the workflow is not JSON: {error}"))
    };
    let mut found = Vec::new();
    let mut scan = Scan { text, at: 0 };
    scan.object(&mut |scan, key| {
        if key != "workflow" {
            return scan.value().map(drop);
        }
        scan.object(&mut |scan, id| {
            let mut node = Node {
                id,
                class: String::new(),
                title: None,
                inputs: Vec::new(),
            };
            scan.object(&mut |scan, field| match field.as_str() {
                "class_type" => {
                    let at = scan.value()?;
                    node.class = parse(&at)?.as_str().unwrap_or_default().to_owned();
                    Ok(())
                }
                // What ComfyUI's own editor calls the node, where its export says.
                "_meta" => {
                    let at = scan.value()?;
                    node.title = parse(&at)?["title"].as_str().map(str::to_owned);
                    Ok(())
                }
                "inputs" => scan.object(&mut |scan, name| {
                    let at = scan.value()?;
                    node.inputs.push(Input {
                        name,
                        value: parse(&at)?,
                        at,
                    });
                    Ok(())
                }),
                _ => scan.value().map(drop),
            })?;
            found.push(node);
            Ok(())
        })
    })?;
    Ok(found)
}

/// The node an input is linked to and which of its outputs, if it is a link: `["7", 0]`.
fn link(value: &Value) -> Option<(&str, u64)> {
    match value.as_array()?.as_slice() {
        [Value::String(node), Value::Number(output)] => Some((node, output.as_u64()?)),
        _ => None,
    }
}

/// The names of the blanks `value` holds, in the order they are in.
fn blanks_of(value: &Value) -> Vec<String> {
    fn each(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::String(text) => {
                let mut rest = text.as_str();
                while let Some(open) = rest.find('{') {
                    let after = &rest[open + 1..];
                    let Some(close) = after.find('}') else { break };
                    let name = &after[..close];
                    let named = name.starts_with(|first: char| first.is_ascii_lowercase())
                        && name.chars().all(|character| {
                            character.is_ascii_lowercase()
                                || character.is_ascii_digit()
                                || character == '_'
                        });
                    if named && !found.iter().any(|have| have == name) {
                        found.push(name.to_owned());
                    }
                    rest = if named { &after[close + 1..] } else { after };
                }
            }
            Value::Array(items) => items.iter().for_each(|item| each(item, found)),
            Value::Object(fields) => fields.values().for_each(|field| each(field, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    each(value, &mut found);
    found
}

/// What fills the blank `name`, as a person is told it; and the value the
/// workflow sets for it, where it is one a line can replace.
fn filled(name: &str, workflow: &Workflow) -> (By, String, Option<Value>) {
    let (by, says) = match name {
        "prompt" => (
            By::Line,
            "the line's prompt, with the cast and styles it names".to_owned(),
        ),
        "seed" => (
            By::Line,
            "the line's seed, or one picked where it gives none".to_owned(),
        ),
        "width" | "height" => (
            By::Line,
            "the line's size, brought to what the model takes".to_owned(),
        ),
        "frames" => (
            By::Line,
            "the line's seconds, as frames of the model's".to_owned(),
        ),
        "fps" => (
            By::Kerf,
            "the rate the model makes frames at, or its guide's for one made of its guide"
                .to_owned(),
        ),
        "no_picture" => (
            By::Kerf,
            "a small black picture, for a node that will not run without one".to_owned(),
        ),
        MOVES | MOVES_VIDEO | MOVES_BOX => (
            By::Moves,
            "where the clips the line names with moves= go".to_owned(),
        ),
        name if names_points(name) => {
            // A line's to give as any value it sets, and none until it does.
            return (
                By::Set,
                format!(
                    "the points the line gives as {name}=\"x,y x,y\", each across and down the guide's first frame from 0 to 1; none unless it does"
                ),
                workflow.set.get(name).cloned(),
            );
        }
        name if names_a_signal(name) => (
            By::Signal,
            "a signal the line names with signal=".to_owned(),
        ),
        name if guide_named(name).is_some() => {
            (By::File, format!("the guide the line gives as {name}="))
        }
        name if names_a_file(name) => (
            By::File,
            format!(
                "the file the line gives as {}=",
                if name.starts_with("ref") { "ref" } else { name }
            ),
        ),
        name => {
            return match workflow.set.get(name) {
                Some(value) => (
                    By::Set,
                    format!("{} unless the line says {name}=", shown(value)),
                    Some(value.clone()),
                ),
                None => (
                    By::Line,
                    format!("what the line says as {name}=, which it has to"),
                    None,
                ),
            }
        }
    };
    (by, says, None)
}

/// A value as a person reads it.
fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// What fills a blank.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum By {
    /// What the step's line says of it: its prompt, its seed, a name it has to give.
    #[default]
    Line,
    /// Kerf itself.
    Kerf,
    /// Where the clips the line names with `moves=` go.
    Moves,
    /// A value the workflow sets unless the line says otherwise.
    Set,
    /// A signal the line names.
    Signal,
    /// A file the line gives.
    File,
}

/// One blank of a workflow: what fills it, in words too, and the value the workflow sets
/// for it where a line can replace it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Blank {
    pub by: By,
    pub says: String,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub default: Option<Value>,
    /// What the step's line gives for it now, as it writes it. [`described`] reads a file
    /// and knows no step: whoever shows the workflow of one says it.
    pub given: Option<String>,
}

/// A workflow's blanks by name, in the order its nodes first hold them: the order a
/// person meets them in. Written as an object, a field a blank; read back in the order
/// the object has them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blanks(Vec<(String, Blank)>);

impl Blanks {
    pub fn get(&self, name: &str) -> Option<&Blank> {
        self.iter()
            .find_map(|(named, blank)| (named == name).then_some(blank))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Blank)> {
        self.0.iter().map(|(name, blank)| (name.as_str(), blank))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut Blank)> {
        self.0
            .iter_mut()
            .map(|(name, blank)| (name.as_str(), blank))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The blanks' names, in their order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|(name, _)| name)
    }

    /// Add blank `name` after the ones there are, unless it is one of them.
    fn add(&mut self, name: &str, blank: impl FnOnce() -> Blank) {
        if self.get(name).is_none() {
            self.0.push((name.to_owned(), blank()));
        }
    }
}

impl Serialize for Blanks {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter())
    }
}

impl<'de> Deserialize<'de> for Blanks {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Read;
        impl<'de> serde::de::Visitor<'de> for Read {
            type Value = Blanks;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("blanks by name")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Blanks, A::Error> {
                let mut blanks = Blanks::default();
                while let Some((name, blank)) = map.next_entry::<String, Blank>()? {
                    blanks.0.push((name, blank));
                }
                Ok(blanks)
            }
        }
        deserializer.deserialize_map(Read)
    }
}

/// A value that is there, `null` too: a workflow may set a blank to it.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// One input of a node: a `link` to output `link.1` of node `link.0`, or a `value` with
/// the `blanks` it holds, if any.
///
/// What a server takes there is not the file's to say, and [`described`] leaves it out:
/// whoever has asked the server fills in what another node hands a link (`type`), the
/// values to choose among (`choices`), or a number's `min`, `max` and `step`, each as the
/// server wrote it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DescribedInput {
    pub name: String,
    pub link: Option<(String, u64)>,
    pub value: Value,
    pub blanks: Vec<String>,
    #[serde(rename = "type")]
    pub takes: Option<String>,
    pub choices: Option<Vec<Value>>,
    pub min: Option<Number>,
    pub max: Option<Number>,
    pub step: Option<Number>,
}

impl Serialize for DescribedInput {
    /// A link is written without a value, and a value without a link: `null` is a value
    /// a workflow may hold.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("name", &self.name)?;
        match &self.link {
            Some(link) => map.serialize_entry("link", link)?,
            None => {
                map.serialize_entry("value", &self.value)?;
                if !self.blanks.is_empty() {
                    map.serialize_entry("blanks", &self.blanks)?;
                }
            }
        }
        if let Some(takes) = &self.takes {
            map.serialize_entry("type", takes)?;
        }
        if let Some(choices) = &self.choices {
            map.serialize_entry("choices", choices)?;
        }
        for (name, bound) in [("min", &self.min), ("max", &self.max), ("step", &self.step)] {
            if let Some(bound) = bound {
                map.serialize_entry(name, bound)?;
            }
        }
        map.end()
    }
}

/// One node of a workflow: its id in the file, its kind, what ComfyUI's own editor calls
/// it where its export says, and its inputs in the file's order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DescribedNode {
    pub id: String,
    pub class: String,
    pub inputs: Vec<DescribedInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What a node of its kind gives at each output, as a server says it: filled in by
    /// whoever has asked one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Value>>,
}

/// How many frames a workflow's model makes: the most in one shot, and one frame and a
/// multiple of `step`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DescribedFrames {
    pub most: u32,
    pub step: u32,
}

/// A workflow, for an editor to show: see [`described`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Described {
    /// What it makes, in a line.
    pub about: String,
    pub frames: DescribedFrames,
    /// The picture its model makes, across and down.
    pub size: (u32, u32),
    pub nodes: Vec<DescribedNode>,
    /// Each blank by its name.
    pub blanks: Blanks,
    /// The rate its model makes frames at: left out where the workflow says none.
    #[serde(skip_serializing_if = "not_positive")]
    pub fps: f64,
}

fn not_positive(fps: &f64) -> bool {
    *fps <= 0.0 || fps.is_nan()
}

/// The workflow `text` holds, for an editor to show: what it makes, and
/// each node with each of its inputs, in the file's order, an input being a
/// `link` to a node's output, or a `value` with the `blanks` it holds, if
/// any. `blanks` at the top says of each blank what fills it. A text that is
/// no workflow is refused with why.
pub fn described(text: &str) -> Result<Described, String> {
    let workflow = Workflow::parse(text)?;
    let nodes = nodes(text)?;
    let mut blanks = Blanks::default();
    let described = nodes
        .iter()
        .map(|node| {
            let inputs = node
                .inputs
                .iter()
                .map(|input| match link(&input.value) {
                    Some((to, output)) => DescribedInput {
                        name: input.name.clone(),
                        link: Some((to.to_owned(), output)),
                        ..DescribedInput::default()
                    },
                    None => {
                        let named = blanks_of(&input.value);
                        for name in &named {
                            blanks.add(name, || {
                                let (by, says, default) = filled(name, &workflow);
                                Blank {
                                    by,
                                    says,
                                    default,
                                    given: None,
                                }
                            });
                        }
                        DescribedInput {
                            name: input.name.clone(),
                            value: input.value.clone(),
                            blanks: named,
                            ..DescribedInput::default()
                        }
                    }
                })
                .collect();
            DescribedNode {
                id: node.id.clone(),
                class: node.class.clone(),
                inputs,
                title: node.title.clone(),
                outputs: None,
            }
        })
        .collect();
    Ok(Described {
        about: workflow.about.clone(),
        frames: DescribedFrames {
            most: workflow.frames.most,
            step: workflow.frames.step,
        },
        size: workflow.size,
        nodes: described,
        blanks,
        fps: workflow.fps,
    })
}

/// `text` with input `input` of node `node` set to `value`, and nothing
/// else of it changed. Only a value the workflow itself sets is set here: a
/// number, a text, or a yes or no, for another of its kind. A link is the
/// graph's shape and a blank is the recipe's to fill, so neither is. What
/// the change makes is read as a workflow before it is given back.
pub fn set_input(text: &str, node: &str, input: &str, value: &Value) -> Result<String, String> {
    let nodes = nodes(text)?;
    let found = nodes
        .iter()
        .find(|found| found.id == node)
        .ok_or_else(|| format!("the workflow has no node `{node}`"))?;
    let named = found
        .inputs
        .iter()
        .find(|found| found.name == input)
        .ok_or_else(|| {
            format!(
                "node {node}, a {}, has no input `{input}`; it has {}",
                found.class,
                found
                    .inputs
                    .iter()
                    .map(|input| input.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    if link(&named.value).is_some() {
        return Err(format!(
            "`{input}` of node {node} is a link to another node, which is the graph's shape and is not set here"
        ));
    }
    if let Some(blank) = blanks_of(&named.value).first() {
        return Err(format!(
            "`{input}` of node {node} is the blank `{{{blank}}}`, which the step fills: set it on the `generate` line"
        ));
    }
    let written = match (&named.value, value) {
        (Value::String(_), Value::String(_)) | (Value::Bool(_), Value::Bool(_)) => value.clone(),
        (Value::Number(was), Value::Number(now)) => {
            let whole = was.is_i64() || was.is_u64();
            match now.as_f64() {
                // A whole number stays one: a node that counts steps takes no half.
                Some(number) if whole && number.fract() != 0.0 => {
                    return Err(format!(
                        "`{input}` of node {node} is a whole number, and {number} is not"
                    ))
                }
                Some(number) if whole => json!(number as i64),
                _ => value.clone(),
            }
        }
        (was, _) => {
            let kind = |value: &Value| match value {
                Value::String(_) => "a text",
                Value::Number(_) => "a number",
                Value::Bool(_) => "a yes or no",
                Value::Null => "nothing",
                Value::Array(_) => "a list",
                Value::Object(_) => "an object",
            };
            return Err(format!(
                "`{input}` of node {node} is {}, and what it is given is {}",
                kind(was),
                kind(value)
            ));
        }
    };
    let mut now = String::with_capacity(text.len() + 16);
    now.push_str(&text[..named.at.start]);
    now.push_str(&written.to_string());
    now.push_str(&text[named.at.end..]);
    Workflow::parse(&now)?;
    Ok(now)
}

/// `text` with input `input` of node `node` read from output `output` of node `from`, in
/// place of the node it was linked to, and nothing else of it changed. Only an input that
/// is a link is linked again: a value and a blank are set, not wired. A node is not linked
/// to itself or to one that reads it, however far on, since a graph that goes round has no
/// order to run in. What kind of thing an output gives is the server's to say, and is held
/// by whoever asks it; here the graph only has to stay a graph.
pub fn set_link(
    text: &str,
    node: &str,
    input: &str,
    from: &str,
    output: u64,
) -> Result<String, String> {
    let nodes = nodes(text)?;
    let find = |id: &str| nodes.iter().find(|found| found.id == id);
    let found = find(node).ok_or_else(|| format!("the workflow has no node `{node}`"))?;
    let named = found
        .inputs
        .iter()
        .find(|found| found.name == input)
        .ok_or_else(|| {
            format!(
                "node {node}, a {}, has no input `{input}`; it has {}",
                found.class,
                found
                    .inputs
                    .iter()
                    .map(|input| input.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    if link(&named.value).is_none() {
        return Err(format!(
            "`{input}` of node {node} is not a link to another node: a value is set, and a blank is the recipe's to fill"
        ));
    }
    if find(from).is_none() {
        return Err(format!("the workflow has no node `{from}` to read from"));
    }
    // Whether `at` reads `node`, itself or through what it reads.
    fn reads(nodes: &[Node], at: &str, node: &str, seen: &mut Vec<String>) -> bool {
        if at == node {
            return true;
        }
        if seen.iter().any(|seen| seen == at) {
            return false;
        }
        seen.push(at.to_owned());
        nodes
            .iter()
            .find(|found| found.id == at)
            .is_some_and(|found| {
                found
                    .inputs
                    .iter()
                    .filter_map(|input| link(&input.value))
                    .any(|(to, _)| reads(nodes, to, node, seen))
            })
    }
    if reads(&nodes, from, node, &mut Vec::new()) {
        return Err(if from == node {
            format!("node {node} cannot read its own output")
        } else {
            format!("node {from} reads node {node}, so node {node} cannot read it back: the graph would go round")
        });
    }
    let mut now = String::with_capacity(text.len() + 8);
    now.push_str(&text[..named.at.start]);
    now.push_str(&json!([from, output]).to_string());
    now.push_str(&text[named.at.end..]);
    Workflow::parse(&now)?;
    Ok(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::BUNDLED;

    #[test]
    fn every_workflow_that_comes_with_the_program_is_said_whole() {
        for (name, text) in BUNDLED {
            let said = described(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            let workflow = Workflow::parse(text).unwrap();
            // Every blank the workflow has is said, with what fills it.
            let mut named: Vec<&str> = said.blanks.names().collect();
            named.sort_unstable();
            let mut have: Vec<String> = workflow.blanks().into_iter().collect();
            have.sort();
            assert_eq!(named, have, "{name}");
            assert!(!said.nodes.is_empty(), "{name}");
            // A value of it set and set back is the text it was.
            let node = said.nodes.iter().find_map(|node| {
                let input = node
                    .inputs
                    .iter()
                    .find(|input| input.blanks.is_empty() && input.value.is_string())?;
                Some((node.id.clone(), input.name.clone(), input.value.clone()))
            });
            if let Some((id, input, was)) = node {
                let changed = set_input(text, &id, &input, &json!("something else")).unwrap();
                assert_ne!(changed, *text, "{name}");
                assert_eq!(
                    set_input(&changed, &id, &input, &was).unwrap(),
                    *text,
                    "{name}"
                );
            }
        }
        // Points are the line's to give, as a value it sets, and said as points.
        let matte =
            described(BUNDLED.iter().find(|(name, _)| *name == "matte").unwrap().1).unwrap();
        let inside = matte.blanks.get("inside").unwrap();
        assert_eq!((inside.by, &inside.default), (By::Set, &Some(json!(""))));
        assert!(
            inside
                .says
                .starts_with("the points the line gives as inside="),
            "{inside:?}"
        );
    }
}
