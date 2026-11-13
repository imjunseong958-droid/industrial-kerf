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
//! A workflow that reads a picture in motion and makes a file of it, its
//! depth or the camera that filmed it, says `"of": "guide"` and needs no
//! `fps`. What it makes is of its guide frame for frame: `{fps}` is the
//! guide's own rate, `{frames}` each frame it has up to `frames.most`, and
//! `{width}` and `{height}` its own size, down to `size` by area.

use crate::request::{names_a_file, names_a_signal, names_what_is_handed};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The version of workflow file this program reads.
const WORKFLOW_VERSION: u32 = 1;

/// The workflows that come with this program, by name.
pub const BUNDLED: [(&str, &str); 6] = [
    ("wan-vace", include_str!("../workflows/wan-vace.json")),
    ("depth", include_str!("../workflows/depth.json")),
    ("set", include_str!("../workflows/set.json")),
    ("depth-whole", include_str!("../workflows/depth-whole.json")),
    (
        "cameras",
        include_str!("../workflows/cameras.json"),
    ),
    ("set-views", include_str!("../workflows/set-views.json")),
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
}

/// A workflow as its file says it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Workflow {
    #[serde(rename = "kerf-comfy")]
    version: u32,
    /// What it makes, in a line.
    #[serde(default)]
    pub about: String,
    /// What it makes a file of, frame for frame: `guide`, or nothing for a
    /// model that makes a shot at a rate and a size of its own.
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
    #[serde(rename = "workflow")]
    graph: Map<String, Value>,
}

fn one() -> u32 {
    1
}

impl Workflow {
    /// The workflow `name` names: a file, or one that comes with this program.
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
        if let Some(of) = workflow.of.as_deref().filter(|of| *of != "guide") {
            return Err(format!("`of` is `guide` or is left out, not `{of}`"));
        }
        if !workflow.of_guide() && !(workflow.fps.is_finite() && workflow.fps > 0.0) {
            return Err("`fps` is not a rate".to_owned());
        }
        if workflow.frames.most == 0 || workflow.frames.step == 0 {
            return Err("`frames` needs a `most` and a `step` of at least 1".to_owned());
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
        Ok(workflow)
    }

    /// Whether what it makes is of its guide, frame for frame, and not a
    /// shot at a rate and a size of the model's own.
    pub fn of_guide(&self) -> bool {
        self.of.is_some()
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
        if self.of_guide() {
            // It has no rate and no size of its own to draw a guide at.
            let mut takes = Map::new();
            if !self.about.is_empty() {
                takes.insert("about".to_owned(), Value::from(self.about.clone()));
            }
            takes.insert("of".to_owned(), Value::from("guide"));
            takes.insert(
                "frames".to_owned(),
                serde_json::json!({"most": self.frames.most}),
            );
            takes.insert("reads".to_owned(), Value::from(vec!["guide"]));
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

    /// The graph to send, with `values` in its blanks. A node with a blank
    /// for a file or a signal that `values` has not got is left out, and so is each
    /// node that `required` says cannot do without one left out; an input
    /// that can loses its link.
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
                let cut: Vec<String> = inputs
                    .iter()
                    .filter(|(_, value)| link(value).is_some_and(|to| gone.contains(to)))
                    .map(|(name, _)| name.clone())
                    .collect();
                if cut.is_empty() {
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
        Workflow::named("wan-vace").unwrap()
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
                    ["prompt", "seed", "width", "height", "frames", "fps"]
                        .contains(&blank.as_str())
                        || names_a_file(&blank)
                        || workflow.set.contains_key(&blank),
                    "{name}: nothing fills {{{blank}}}"
                );
            }
        }
        // One that makes a file of its guide reads a guide, and has no rate of its own.
        for name in [
            "depth",
            "set",
            "depth-whole",
            "cameras",
            "set-views",
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
        // Depth Anything 3 finds a camera's path only with its weights as 32-bit floats.
        for name in ["cameras", "set-views"] {
            let multiview = Workflow::named(name).unwrap();
            assert!(multiview
                .graph
                .values()
                .any(|node| node["class_type"] == "LoadDA3Model"
                    && node["inputs"]["weight_dtype"] == "fp32"));
        }
        // MoGe refines three times unless a line says otherwise.
        let moge = Workflow::named("depth").unwrap();
        assert_eq!(moge.takes()["more"]["refine_steps"], json!(3));
        let missing = Workflow::named("no-such").unwrap_err();
        assert!(missing.contains("wan-vace"), "{missing}");
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
        assert!(takes["about"].as_str().unwrap().starts_with("Wan 2.1 VACE"));
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
            let vace = graph["9"]["inputs"].as_object().unwrap().clone();
            (graph.as_object().unwrap().len(), vace)
        };
        // No reference: its loader goes, and the link to it.
        let (count, vace) = nodes(&[("guide", "kerf-1.mp4")]);
        assert_eq!(count, 15);
        assert!(vace.contains_key("control_video") && !vace.contains_key("reference_image"));
        // No guide: its loader goes, and what takes it apart, which cannot do without it.
        let (count, vace) = nodes(&[("ref", "kerf-2.png")]);
        assert_eq!(count, 14);
        assert!(!vace.contains_key("control_video") && vace.contains_key("reference_image"));
        // Neither: the prompt alone.
        let (count, vace) = nodes(&[]);
        assert_eq!(count, 13);
        assert!(!vace.contains_key("control_video") && !vace.contains_key("reference_image"));
        assert_eq!(vace["positive"], json!(["4", 0]));
        // A server that cannot say what a node needs stops it.
        let failed = workflow
            .graph(&values(&[]), &mut |_| Err("the server is away".to_owned()))
            .unwrap_err();
        assert_eq!(failed, "the server is away");
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
