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

use crate::names::{guide_named, names_a_file, names_a_signal, MOVES, MOVES_BOX, MOVES_VIDEO};
use crate::workflow::Workflow;
use serde_json::{json, Map, Value};
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
                            _ => return Err("the workflow is not JSON: a list is not closed".to_owned()),
                        }
                    }
                }
            }
            Some(b'"') => {
                self.string()?;
            }
            Some(_) => {
                // A number, true, false, or null: up to what ends it.
                while self
                    .next()
                    .is_some_and(|byte| !matches!(byte, b',' | b'}' | b']') && !byte.is_ascii_whitespace())
                {
                    self.at += 1;
                }
                if self.at == start {
                    return Err(format!("the workflow is not JSON: a value was expected at byte {start}"));
                }
            }
            None => return Err("the workflow is not JSON: it ends where a value was expected".to_owned()),
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
        serde_json::from_str(&text[at.clone()]).map_err(|error| format!("the workflow is not JSON: {error}"))
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
                            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
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
fn filled(name: &str, workflow: &Workflow) -> (&'static str, String, Option<Value>) {
    let (by, says) = match name {
        "prompt" => ("line", "the line's prompt, with the cast and styles it names".to_owned()),
        "seed" => ("line", "the line's seed, or one picked where it gives none".to_owned()),
        "width" | "height" => ("line", "the line's size, brought to what the model takes".to_owned()),
        "frames" => ("line", "the line's seconds, as frames of the model's".to_owned()),
        "fps" => ("kerf", "the rate the model makes frames at, or its guide's for one made of its guide".to_owned()),
        "no_picture" => ("kerf", "a small black picture, for a node that will not run without one".to_owned()),
        MOVES | MOVES_VIDEO | MOVES_BOX => ("moves", "where the clips the line names with moves= go".to_owned()),
        name if names_a_signal(name) => ("signal", "a signal the line names with signal=".to_owned()),
        name if guide_named(name).is_some() => ("file", format!("the guide the line gives as {name}=")),
        name if names_a_file(name) => (
            "file",
            format!(
                "the file the line gives as {}=",
                if name.starts_with("ref") { "ref" } else { name }
            ),
        ),
        name => {
            return match workflow.set.get(name) {
                Some(value) => (
                    "set",
                    format!("{} unless the line says {name}=", shown(value)),
                    Some(value.clone()),
                ),
                None => ("line", format!("what the line says as {name}=, which it has to"), None),
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

/// The workflow `text` holds, for an editor to show: what it makes, and
/// each node with each of its inputs, in the file's order, an input being a
/// `link` to a node's output, or a `value` with the `blanks` it holds, if
/// any. `blanks` at the top says of each blank what fills it. A text that is
/// no workflow is refused with why.
pub fn described(text: &str) -> Result<Value, String> {
    let workflow = Workflow::parse(text)?;
    let nodes = nodes(text)?;
    let mut blanks = Map::new();
    let described: Vec<Value> = nodes
        .iter()
        .map(|node| {
            let inputs: Vec<Value> = node
                .inputs
                .iter()
                .map(|input| match link(&input.value) {
                    Some((to, output)) => json!({"name": input.name, "link": [to, output]}),
                    None => {
                        let named = blanks_of(&input.value);
                        for name in &named {
                            if !blanks.contains_key(name) {
                                let (by, says, default) = filled(name, &workflow);
                                let mut blank = json!({"by": by, "says": says});
                                if let Some(default) = default {
                                    blank["default"] = default;
                                }
                                blanks.insert(name.clone(), blank);
                            }
                        }
                        let mut said = json!({"name": input.name, "value": input.value});
                        if !named.is_empty() {
                            said["blanks"] = json!(named);
                        }
                        said
                    }
                })
                .collect();
            let mut said = json!({"id": node.id, "class": node.class, "inputs": inputs});
            if let Some(title) = &node.title {
                said["title"] = json!(title);
            }
            said
        })
        .collect();
    let mut said = json!({
        "about": workflow.about,
        "frames": {"most": workflow.frames.most, "step": workflow.frames.step},
        "size": [workflow.size.0, workflow.size.1],
        "nodes": described,
        "blanks": blanks,
    });
    if workflow.fps > 0.0 {
        said["fps"] = json!(workflow.fps);
    }
    Ok(said)
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
                found.inputs.iter().map(|input| input.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        })?;
    if link(&named.value).is_some() {
        return Err(format!(
            "`{input}` of node {node} is a link to another node, which is the graph's shape and is not set here"
        ));
    }
    if let Some(blank) = blanks_of(&named.value).first() {
        return Err(format!(
            "`{input}` of node {node} is the blank `{{{blank}}}`, which the recipe fills: set it on the `generate` line"
        ));
    }
    let written = match (&named.value, value) {
        (Value::String(_), Value::String(_)) | (Value::Bool(_), Value::Bool(_)) => value.clone(),
        (Value::Number(was), Value::Number(now)) => {
            let whole = was.is_i64() || was.is_u64();
            match now.as_f64() {
                // A whole number stays one: a node that counts steps takes no half.
                Some(number) if whole && number.fract() != 0.0 => {
                    return Err(format!("`{input}` of node {node} is a whole number, and {number} is not"))
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
pub fn set_link(text: &str, node: &str, input: &str, from: &str, output: u64) -> Result<String, String> {
    let nodes = nodes(text)?;
    let find = |id: &str| nodes.iter().find(|found| found.id == id);
    let found = find(node).ok_or_else(|| format!("the workflow has no node `{node}`"))?;
    let named = found.inputs.iter().find(|found| found.name == input).ok_or_else(|| {
        format!(
            "node {node}, a {}, has no input `{input}`; it has {}",
            found.class,
            found.inputs.iter().map(|input| input.name.as_str()).collect::<Vec<_>>().join(", ")
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

    const SMALL: &str = r#"{
  "kerf-comfy": 1,
  "about": "A small shot.",
  "fps": 16,
  "frames": { "most": 81, "step": 4 },
  "size": [832, 480],
  "set": { "steps": 20, "negative": "blurry" },
  "workflow": {
    "4": { "class_type": "CLIPTextEncode", "_meta": { "title": "Prompt" },
           "inputs": { "text": "{prompt}, {style_words}", "clip": ["2", 0] } },
    "8": { "class_type": "KSampler",
           "inputs": { "seed": "{seed}", "steps": "{steps}", "cfg": 6.0, "sampler_name": "uni_pc",
                       "denoise": 1, "tiled": false, "positive": ["4", 0], "note": "braces {Like This} stand" } },
    "6": { "class_type": "LoadVideo", "inputs": { "file": "{guide}" } }
  }
}
"#;

    #[test]
    fn a_link_is_pointed_at_another_node_and_the_graph_stays_one() {
        let text = r#"{"kerf-comfy": 1, "fps": 16, "frames": {"most": 81, "step": 4}, "size": [832, 480],
  "workflow": {
    "1": {"class_type": "UNETLoader", "inputs": {"unet_name": "a.safetensors"}},
    "2": {"class_type": "LoraLoaderModelOnly", "inputs": {"model": ["1", 0], "lora_name": "b.safetensors"}},
    "3": {"class_type": "KSampler", "inputs": {"model": [
        "1",
        0
      ], "seed": "{seed}", "steps": 20}}
  }
}"#;
        // The sampler reads the lora's model in place of the loader's: that link alone changes.
        let now = set_link(text, "3", "model", "2", 0).unwrap();
        assert_eq!(now, text.replace("[\n        \"1\",\n        0\n      ]", "[\"2\",0]"));
        assert_eq!(described(&now).unwrap()["nodes"][2]["inputs"][0]["link"], json!(["2", 0]));
        // What would have the graph go round, a node that is not there, and what is no link.
        let chained = set_link(&now, "2", "model", "3", 0).unwrap_err();
        assert!(chained.contains("the graph would go round"), "{chained}");
        assert!(set_link(text, "3", "model", "3", 0).unwrap_err().contains("cannot read its own output"));
        assert!(set_link(text, "3", "model", "9", 0).unwrap_err().contains("no node `9` to read from"));
        assert!(set_link(text, "3", "steps", "1", 0).unwrap_err().contains("is not a link"));
        assert!(set_link(text, "3", "seed", "1", 0).unwrap_err().contains("is not a link"));
        assert!(set_link(text, "3", "latent", "1", 0).unwrap_err().contains("has no input `latent`"));
    }

    #[test]
    fn a_workflow_is_said_node_by_node_in_the_files_order() {
        let said = described(SMALL).unwrap();
        assert_eq!((said["about"].as_str(), said["fps"].as_f64()), (Some("A small shot."), Some(16.0)));
        let ids: Vec<&str> = said["nodes"].as_array().unwrap().iter().map(|node| node["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["4", "8", "6"]);
        assert_eq!(said["nodes"][0]["title"], "Prompt");
        assert_eq!(said["nodes"][1]["class"], "KSampler");
        // Each input, in the order the node has them: a link, a blank, or a value.
        let inputs = said["nodes"][1]["inputs"].as_array().unwrap();
        let names: Vec<&str> = inputs.iter().map(|input| input["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["seed", "steps", "cfg", "sampler_name", "denoise", "tiled", "positive", "note"]);
        assert_eq!(inputs[0], json!({"name": "seed", "value": "{seed}", "blanks": ["seed"]}));
        assert_eq!(inputs[2], json!({"name": "cfg", "value": 6.0}));
        assert_eq!(inputs[6], json!({"name": "positive", "link": ["4", 0]}));
        // Braces round what is no blank's name stand for themselves.
        assert_eq!(inputs[7], json!({"name": "note", "value": "braces {Like This} stand"}));
        assert_eq!(said["nodes"][0]["inputs"][0]["blanks"], json!(["prompt", "style_words"]));
        // What fills each blank: the line, a file it hands over, or what the workflow sets.
        let blanks = &said["blanks"];
        assert_eq!(blanks["prompt"]["by"], "line");
        assert_eq!(blanks["guide"], json!({"by": "file", "says": "the file the line gives as guide="}));
        assert_eq!(blanks["steps"], json!({"by": "set", "says": "20 unless the line says steps=", "default": 20}));
        assert_eq!(blanks["style_words"]["says"], "what the line says as style_words=, which it has to");
    }

    #[test]
    fn a_value_is_set_where_it_stands_and_the_rest_reads_as_it_did() {
        let now = set_input(SMALL, "8", "sampler_name", &json!("euler")).unwrap();
        assert_eq!(now, SMALL.replace("\"uni_pc\"", "\"euler\""));
        let now = set_input(SMALL, "8", "cfg", &json!(4.5)).unwrap();
        assert_eq!(now, SMALL.replace("\"cfg\": 6.0", "\"cfg\": 4.5"));
        // A whole number stays whole, however it is given, and takes no half.
        let now = set_input(SMALL, "8", "denoise", &json!(2.0)).unwrap();
        assert_eq!(now, SMALL.replace("\"denoise\": 1", "\"denoise\": 2"));
        assert!(set_input(SMALL, "8", "denoise", &json!(0.5)).unwrap_err().contains("is a whole number"));
        let now = set_input(SMALL, "8", "tiled", &json!(true)).unwrap();
        assert_eq!(now, SMALL.replace("\"tiled\": false", "\"tiled\": true"));
        // A text with a quote in it is written as JSON writes it.
        let now = set_input(SMALL, "8", "sampler_name", &json!("a \"b\"")).unwrap();
        assert!(now.contains(r#""sampler_name": "a \"b\"""#));
        assert_eq!(described(&now).unwrap()["nodes"][1]["inputs"][3]["value"], "a \"b\"");
    }

    #[test]
    fn a_link_a_blank_and_another_kind_are_refused_with_why() {
        let refused = |node: &str, input: &str, value: Value| set_input(SMALL, node, input, &value).unwrap_err();
        assert!(refused("8", "positive", json!("x")).contains("is a link to another node"));
        assert!(refused("8", "steps", json!(30)).contains("is the blank `{steps}`, which the recipe fills"));
        assert!(refused("4", "text", json!("a boat")).contains("is the blank `{prompt}`"));
        assert!(refused("8", "cfg", json!("six")).contains("is a number, and what it is given is a text"));
        assert!(refused("9", "cfg", json!(1)).contains("has no node `9`"));
        assert!(refused("8", "scheduler", json!("simple")).contains("has no input `scheduler`; it has seed, steps"));
        assert!(described("{\"kerf-comfy\": 1").is_err());
    }

    #[test]
    fn every_workflow_that_comes_with_the_program_is_said_whole() {
        for (name, text) in BUNDLED {
            let said = described(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            let workflow = Workflow::parse(text).unwrap();
            // Every blank the workflow has is said, with what fills it.
            let named: Vec<&String> = said["blanks"].as_object().unwrap().keys().collect();
            let mut have: Vec<String> = workflow.blanks().into_iter().collect();
            have.sort();
            assert_eq!(named, have.iter().collect::<Vec<_>>(), "{name}");
            assert!(!said["nodes"].as_array().unwrap().is_empty(), "{name}");
            // A value of it set and set back is the text it was.
            let node = said["nodes"].as_array().unwrap().iter().find_map(|node| {
                let input = node["inputs"].as_array()?.iter().find(|input| {
                    input.get("blanks").is_none() && input["value"].is_string()
                })?;
                Some((node["id"].as_str()?.to_owned(), input["name"].as_str()?.to_owned(), input["value"].clone()))
            });
            if let Some((id, input, was)) = node {
                let changed = set_input(text, &id, &input, &json!("something else")).unwrap();
                assert_ne!(changed, *text, "{name}");
                assert_eq!(set_input(&changed, &id, &input, &was).unwrap(), *text, "{name}");
            }
        }
    }
}
