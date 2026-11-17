//! A recipe as a graph: its steps and what they read, wired together.
//!
//! A recipe is lines, and what one line makes a later one reads: a guide
//! drawn from a sequence, a picture one model makes that the next starts
//! from, a signal, the way a clip goes. Read as text that has to be held in
//! the head. Here it is laid out as what it is, for an editor to draw: a
//! node for each step and for each thing of the project a step reads, and
//! an edge for each reading, with what goes along it.
//!
//! ```text
//! render court {depth.mp4} pass=depth size=832x480 rate=16
//! generate model=still prompt="an orange basketball" seed=3 out={ball.png}
//! generate model=video prompt="it bounces" guide={depth.mp4} ref={ball.png} moves=ball signal=bounce
//! ```
//!
//! is the sequence `court` into a depth guide into the video model's `guide`,
//! the still model's picture into its `ref`, the clip `ball` into its `moves`,
//! the signal `bounce` into its `signal`, and the video model into the shot's
//! takes.
//!
//! Nothing is run to say it, and nothing here changes the recipe: a change
//! is made to its text ([`super::text`]) and the graph is read again.

use super::model::{Ask, Models, Source};
use super::signal::Signal;
use super::{folder_of, inside_project, stored, text::step_lines, versioned, within, Recipe, Step};
use crate::error::{EngineError, Result};
use kerf_compose::Pass;
use kerf_model::{Content, Param, Project, Value};
use serde::Serialize;
use serde_json::{json, Map, Value as Json};
use std::collections::BTreeMap;
use std::path::Path;

/// What goes along an edge, and what a node gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Gives {
    /// A picture in motion: a sequence, a guide, a take.
    Video,
    /// A still picture.
    Image,
    /// A number over time.
    Signal,
    /// Where a thing goes.
    Path,
    /// Words, with or without pictures: one of the cast, a style.
    Words,
    /// Any other file: a mesh, a camera's path, a scene.
    File,
}

impl Gives {
    /// What a file of this name holds, by its extension.
    fn of_file(name: &str) -> Self {
        let extension = Path::new(name)
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "mp4" | "mov" | "m4v" | "mkv" | "webm" | "avi" | "mxf" | "gif" => Self::Video,
            "png" | "jpg" | "jpeg" | "webp" | "tif" | "tiff" | "bmp" | "exr" => Self::Image,
            _ => Self::File,
        }
    }
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A sequence of the project a step draws.
    Sequence,
    /// A `render` step: Kerf drawing a sequence into a file.
    Render,
    /// A `generate` step: a model asked for a file.
    Model,
    /// A `run` step: a program.
    Run,
    /// A `join` step: parts of a longer shot set end to end.
    Join,
    /// A file of the project a step reads.
    File,
    /// A signal clip of the project.
    Signal,
    /// A clip of the project whose way across the frame a model is handed.
    Moves,
    /// One of the project's cast.
    Cast,
    /// One of the project's styles.
    Style,
    /// The shot's takes: what the last step writes.
    Takes,
}

/// One node of a recipe's graph.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Node {
    /// What it is told apart by, which stays the same while the recipe's
    /// other lines change: a step by the file it makes, what is read by its
    /// name.
    pub id: String,
    pub kind: Kind,
    /// What it is called, and a line on what it holds.
    pub title: String,
    pub detail: String,
    pub gives: Gives,
    /// The line of the recipe it is, for a step, counted from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// What else there is to say of one of its kind.
    #[serde(flatten)]
    pub holds: Map<String, Json>,
}

/// One reading: what `from` gives, read by `to` under the name `port`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// The name the reading step gives it: `guide`, `ref`, `signal`.
    pub port: String,
    /// Which of the words of that name on the line it is, from 0.
    pub nth: usize,
    pub gives: Gives,
}

/// A media's recipe, laid out.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Graph {
    /// The media made, its recipe as the document names it, and the shot's name.
    pub media: String,
    pub recipe: String,
    pub name: String,
    /// The recipe's text, as its file holds it.
    pub text: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Why the text does not read as a recipe, where it does not: there are
    /// then no nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Why it cannot be made as things stand, where it cannot: a model that
    /// is not set up, a file that is not there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cannot: Option<String>,
    /// What is worth knowing before it is made, as a plan says it.
    pub notes: Vec<String>,
}

/// A sequence's size, rate, and length, as a line under its name.
fn sequence_line(sequence: &kerf_model::Sequence) -> String {
    format!(
        "{}×{} · {} fps · {} s",
        sequence.width,
        sequence.height,
        super::plain(sequence.rate.fps()),
        super::plain(sequence.duration().seconds())
    )
}

/// What a `render` step is called, by what it draws.
fn render_title(pass: Pass, matte: Option<&str>) -> String {
    match (pass, matte) {
        (_, Some(clip)) => format!("Matte of {clip}"),
        (Pass::Picture, None) => "Guide".to_owned(),
        (Pass::Depth, None) => "Depth guide".to_owned(),
        (Pass::Relative, None) => "Relative depth guide".to_owned(),
        (Pass::Ids, None) => "Ids guide".to_owned(),
        (Pass::Normals, None) => "Normals guide".to_owned(),
    }
}

/// The node ids of a graph being built: a second step that makes a file of
/// the same name, or a second run, is told from the first.
#[derive(Default)]
struct Ids(BTreeMap<String, usize>);

impl Ids {
    fn of(&mut self, id: String) -> String {
        let count = self.0.entry(id.clone()).or_default();
        *count += 1;
        match *count {
            1 => id,
            more => format!("{id}#{more}"),
        }
    }
}

/// The graph being built.
struct Building<'a> {
    project: &'a Project,
    directory: &'a Path,
    /// The recipe's folder, from the project's.
    folder: &'a Path,
    models: &'a Models,
    ids: Ids,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    /// The step that made each file made so far, by the file's name in braces.
    makers: BTreeMap<String, String>,
    /// What the media's own file holds, which `{out}` stands for.
    out: Gives,
}

impl Building<'_> {
    /// What the file a step makes under `name` holds: by its extension, and
    /// for `{out}` by the media's own.
    fn made(&self, name: &str) -> Gives {
        if name == super::OUT {
            self.out
        } else {
            Gives::of_file(name)
        }
    }

    /// The node for something of the project that is read, added once: a
    /// second reading of it is an edge from the same node.
    fn read(&mut self, id: String, make: impl FnOnce(&Self) -> Node) -> String {
        if !self.nodes.iter().any(|node| node.id == id) {
            let node = make(self);
            // What is read stands before the steps that read it.
            self.nodes.push(node);
        }
        id
    }

    fn edge(&mut self, from: String, to: &str, port: &str, nth: usize, gives: Gives) {
        self.edges.push(Edge {
            from,
            to: to.to_owned(),
            port: port.to_owned(),
            nth,
            gives,
        });
    }

    /// The node a file a step is handed comes from, and what it holds: the
    /// step that made it, or a file of the project.
    fn source(&mut self, source: &Source) -> (String, Gives) {
        match source {
            Source::Made(name) => (
                self.makers
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| format!("made:{name}")),
                self.made(name),
            ),
            Source::File(file) => (self.file(file), Gives::of_file(file)),
        }
    }

    /// The node of a file of the project, named as the recipe names it.
    fn file(&mut self, file: &str) -> String {
        self.read(format!("file:{file}"), |building| {
            let inside = within(building.folder, file, "file the recipe needs").ok();
            let there = inside
                .as_ref()
                .is_some_and(|inside| building.directory.join(inside).is_file());
            let path = Path::new(file);
            let mut holds = Map::new();
            holds.insert("path".to_owned(), json!(file));
            if let Some(inside) = &inside {
                holds.insert("inProject".to_owned(), json!(stored(inside)));
            }
            holds.insert("there".to_owned(), json!(there));
            Node {
                id: format!("file:{file}"),
                kind: Kind::File,
                title: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.to_owned()),
                detail: if there {
                    inside
                        .as_ref()
                        .and_then(|inside| inside.parent())
                        .map(stored)
                        .unwrap_or_default()
                } else {
                    "not there".to_owned()
                },
                gives: Gives::of_file(file),
                line: None,
                holds,
            }
        })
    }

    /// The node of a sequence of the project a step draws.
    fn sequence(&mut self, id: &str) -> String {
        self.read(format!("sequence:{id}"), |building| {
            let sequence = building.project.sequence(id);
            let mut holds = Map::new();
            holds.insert("sequence".to_owned(), json!(id));
            holds.insert("there".to_owned(), json!(sequence.is_some()));
            Node {
                id: format!("sequence:{id}"),
                kind: Kind::Sequence,
                title: sequence
                    .map(|sequence| sequence.name.trim())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(id)
                    .to_owned(),
                detail: sequence.map_or_else(
                    || "not a sequence of this project".to_owned(),
                    sequence_line,
                ),
                gives: Gives::Video,
                line: None,
                holds,
            }
        })
    }

    /// The node of a signal clip a `generate` line names.
    fn signal(&mut self, clip: &str) -> String {
        self.read(format!("signal:{clip}"), |building| {
            let project = building.project;
            let mut holds = Map::new();
            holds.insert("clip".to_owned(), json!(clip));
            let at = project.locate(clip);
            let read = Signal::read(project, clip);
            let detail = match (&read, at) {
                (Ok(_), Some(at)) => {
                    let sequence = &project.sequences[at.sequence];
                    let found = project.clip_at(at);
                    let rate = sequence.rate;
                    let frames = rate.frames_round(found.duration).max(1);
                    let value = match &found.content {
                        Content::Generator { params, .. } => params.get(kerf_model::SIGNAL_VALUE),
                        _ => None,
                    };
                    // What it comes to along its clip, a number a frame, as the document
                    // would have a model handed it.
                    let step = (frames as usize).div_ceil(POINTS).max(1);
                    let at_frame = |frame: i64| match value
                        .map(|value| value.value_at(rate.time_of(frame)))
                    {
                        Some(Value::Number(number)) if number.is_finite() => {
                            (number.clamp(0.0, 1.0) * 1e4).round() / 1e4
                        }
                        _ => 0.0,
                    };
                    let mut values: Vec<f64> = (0..frames).step_by(step).map(at_frame).collect();
                    if (frames - 1) % step as i64 != 0 {
                        values.push(at_frame(frames - 1));
                    }
                    holds.insert("sequence".to_owned(), json!(sequence.id.as_str()));
                    holds.insert("frames".to_owned(), json!(frames));
                    holds.insert("fps".to_owned(), json!(rate.fps()));
                    holds.insert("values".to_owned(), json!(values));
                    holds.insert("there".to_owned(), json!(true));
                    format!(
                        "{} · {frames} frames at {}",
                        sequence.id,
                        super::plain(rate.fps())
                    )
                }
                (Err(why), _) => {
                    holds.insert("there".to_owned(), json!(false));
                    why.clone()
                }
                (Ok(_), None) => unreachable!("a signal that reads is a clip"),
            };
            Node {
                id: format!("signal:{clip}"),
                kind: Kind::Signal,
                title: at
                    .and_then(|at| project.clip_at(at).name.clone())
                    .unwrap_or_else(|| clip.to_owned()),
                detail,
                gives: Gives::Signal,
                line: None,
                holds,
            }
        })
    }

    /// The node of a clip whose way across the frame a `generate` line hands over.
    fn moves(&mut self, clip: &str) -> String {
        self.read(format!("moves:{clip}"), |building| {
            let project = building.project;
            let mut holds = Map::new();
            holds.insert("clip".to_owned(), json!(clip));
            let at = project.locate(clip);
            holds.insert("there".to_owned(), json!(at.is_some()));
            let detail = match at {
                Some(at) => {
                    let sequence = &project.sequences[at.sequence];
                    let found = project.clip_at(at);
                    holds.insert("sequence".to_owned(), json!(sequence.id.as_str()));
                    holds.insert(
                        "aspect".to_owned(),
                        json!(f64::from(sequence.width) / f64::from(sequence.height.max(1))),
                    );
                    // The path it is led along, where it has one: its points, as its
                    // line writes them.
                    if let Some(Param::Static(Value::Text(path))) = found.props.get("path") {
                        if !path.trim().is_empty() {
                            holds.insert("path".to_owned(), json!(path));
                        }
                    }
                    format!("where it goes in {}", sequence.id)
                }
                None => project.no_clip(clip),
            };
            Node {
                id: format!("moves:{clip}"),
                kind: Kind::Moves,
                title: at
                    .and_then(|at| project.clip_at(at).name.clone())
                    .unwrap_or_else(|| clip.to_owned()),
                detail,
                gives: Gives::Path,
                line: None,
                holds,
            }
        })
    }

    /// The node of one of the project's cast or styles.
    fn cast(&mut self, kind: super::cast::Kind, name: &str, cast: Option<&super::cast::Cast>) -> String {
        let word = kind.word();
        self.read(format!("{word}:{name}"), |_| {
            let entry = cast.and_then(|cast| cast.of(kind).find(|(named, _)| *named == name));
            let mut holds = Map::new();
            holds.insert("name".to_owned(), json!(name));
            holds.insert("there".to_owned(), json!(entry.is_some()));
            if let Some((_, entry)) = entry {
                holds.insert("refs".to_owned(), json!(entry.references));
            }
            Node {
                id: format!("{word}:{name}"),
                kind: match kind {
                    super::cast::Kind::Cast => Kind::Cast,
                    super::cast::Kind::Style => Kind::Style,
                },
                title: name.to_owned(),
                detail: entry.map_or_else(
                    || format!("not in {}", super::cast::FILE),
                    |(_, entry)| entry.words.clone(),
                ),
                gives: Gives::Words,
                line: None,
                holds,
            }
        })
    }

    /// The node of a `generate` step, and an edge for each thing it reads.
    fn model(&mut self, ask: &Ask, line: usize, cast: Option<&super::cast::Cast>) -> String {
        let id = self.ids.of(format!("made:{}", ask.out));
        let takes = self.models.takes(&ask.model);
        let mut holds = Map::new();
        holds.insert("model".to_owned(), json!(ask.model));
        holds.insert(
            "setUp".to_owned(),
            json!(self.models.command(&ask.model).is_ok()),
        );
        holds.insert("prompt".to_owned(), json!(ask.prompt));
        holds.insert("seed".to_owned(), json!(ask.seed));
        holds.insert("seconds".to_owned(), json!(ask.seconds));
        holds.insert("from".to_owned(), json!(ask.from));
        holds.insert(
            "size".to_owned(),
            json!(ask.size.map(|(width, height)| format!("{width}x{height}"))),
        );
        holds.insert("makes".to_owned(), json!(ask.out));
        holds.insert(
            super::model::WORKFLOW.to_owned(),
            json!(ask.workflow.as_ref().map(|workflow| match workflow {
                Source::File(file) => file.clone(),
                Source::Made(name) => format!("{{{name}}}"),
            })),
        );
        // What the line sets for this model alone, then what the model has said it
        // takes and the line leaves as it comes.
        let mut knobs: Vec<Json> = ask
            .more
            .iter()
            .map(|(name, value)| {
                json!({"name": name, "value": value,
                    "default": takes.and_then(|takes| takes.more.get(name))})
            })
            .collect();
        if let Some(takes) = takes {
            knobs.extend(
                takes
                    .more
                    .iter()
                    .filter(|(name, _)| !ask.more.contains_key(*name))
                    .map(|(name, default)| json!({"name": name, "value": null, "default": default})),
            );
            if let Some(about) = &takes.about {
                holds.insert("about".to_owned(), json!(about));
            }
            if let Some(reads) = &takes.reads {
                holds.insert("reads".to_owned(), json!(reads));
            }
            if let Some((width, height)) = takes.size {
                holds.insert("modelSize".to_owned(), json!(format!("{width}x{height}")));
            }
            if let Some(rate) = takes.rate {
                holds.insert("modelRate".to_owned(), json!(rate));
            }
            if let Some(seconds) = takes.seconds {
                holds.insert("modelSeconds".to_owned(), json!(seconds));
            }
        }
        holds.insert("knobs".to_owned(), json!(knobs));
        // What it reads first, so that each stands before it in the list, as it does on the page.
        if let Some(guide) = &ask.guide {
            let (from, gives) = self.source(guide);
            self.edge(from, &id, "guide", 0, gives);
        }
        for (name, guide) in &ask.guides {
            let (from, gives) = self.source(guide);
            self.edge(from, &id, &format!("guide_{name}"), 0, gives);
        }
        if let Some(first) = &ask.first {
            let (from, gives) = self.source(first);
            self.edge(from, &id, "first", 0, gives);
        }
        if let Some(last) = &ask.last {
            let (from, gives) = self.source(last);
            self.edge(from, &id, "last", 0, gives);
        }
        for (nth, reference) in ask.references.iter().enumerate() {
            let (from, gives) = self.source(reference);
            self.edge(from, &id, "ref", nth, gives);
        }
        if let Some(workflow) = &ask.workflow {
            let (from, gives) = self.source(workflow);
            self.edge(from, &id, super::model::WORKFLOW, 0, gives);
        }
        for (nth, clip) in ask.moves.iter().enumerate() {
            let from = self.moves(clip);
            self.edge(from, &id, super::model::MOVES, nth, Gives::Path);
        }
        for (nth, clip) in ask.signals.iter().enumerate() {
            let from = self.signal(clip);
            self.edge(from, &id, super::model::SIGNAL, nth, Gives::Signal);
        }
        for (nth, name) in ask.cast.iter().enumerate() {
            let from = self.cast(super::cast::Kind::Cast, name, cast);
            self.edge(from, &id, "cast", nth, Gives::Words);
        }
        for (nth, name) in ask.styles.iter().enumerate() {
            let from = self.cast(super::cast::Kind::Style, name, cast);
            self.edge(from, &id, "style", nth, Gives::Words);
        }
        let gives = self.made(&ask.out);
        self.nodes.push(Node {
            id: id.clone(),
            kind: Kind::Model,
            title: ask.model.clone(),
            detail: ask.prompt.clone(),
            gives,
            line: Some(line),
            holds,
        });
        self.makers.insert(ask.out.clone(), id.clone());
        id
    }
}

/// The most points a signal's curve is given as: a long clip is read every few frames.
const POINTS: usize = 512;

/// The recipe media `id` of `project`, kept in `directory`, names, as a
/// graph. A text that does not read as a recipe is a graph with no nodes
/// that says why (`error`); one that reads and cannot be made says that
/// (`cannot`) and is laid out all the same. Nothing is run.
pub fn graph(project: &Project, directory: &Path, id: &str, models: &Models) -> Result<Graph> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let name = media.recipe.as_deref().ok_or_else(|| {
        EngineError::msg(format!("media `{id}` names no recipe, so there is no graph of how it is made"))
    })?;
    let file = inside_project(directory, name, "recipe")?;
    let text = std::fs::read_to_string(&file).map_err(|error| {
        EngineError::msg(format!("cannot read the recipe {}: {error}", file.display()))
    })?;
    let mut graph = Graph {
        media: id.to_owned(),
        recipe: name.to_owned(),
        name: versioned(Path::new(name)).0,
        text,
        nodes: Vec::new(),
        edges: Vec::new(),
        error: None,
        cannot: None,
        notes: Vec::new(),
    };
    let recipe = match Recipe::parse(&graph.text) {
        Ok(recipe) => recipe,
        Err(error) => {
            graph.error = Some(error.to_string());
            return Ok(graph);
        }
    };
    let folder = folder_of(name);
    let cast = super::cast::Cast::read(directory).ok();
    let mut building = Building {
        project,
        directory,
        folder: &folder,
        models,
        ids: Ids::default(),
        nodes: Vec::new(),
        edges: Vec::new(),
        makers: BTreeMap::new(),
        out: Gives::of_file(&media.path),
    };
    let lines = step_lines(&graph.text);
    let mut runs = Vec::new();
    let mut last = None;
    for (step, line) in recipe.steps.iter().zip(lines) {
        let node = match step {
            Step::Render {
                sequence,
                to,
                size,
                rate,
                pass,
                matte,
                from: starts,
                seconds,
            } => {
                let from = building.sequence(sequence);
                let id = building.ids.of(format!("made:{to}"));
                let mut at = Vec::new();
                if let Some((width, height)) = size {
                    at.push(format!("{width}×{height}"));
                }
                if let Some(rate) = rate {
                    at.push(format!("{} fps", super::plain(rate.fps())));
                }
                let mut holds = Map::new();
                holds.insert("sequence".to_owned(), json!(sequence));
                holds.insert("makes".to_owned(), json!(to));
                holds.insert(
                    "pass".to_owned(),
                    json!(if *pass == Pass::Picture { "picture" } else { pass.name() }),
                );
                holds.insert(
                    "size".to_owned(),
                    json!(size.map(|(width, height)| format!("{width}x{height}"))),
                );
                holds.insert("rate".to_owned(), json!(rate.map(|rate| rate.to_string())));
                holds.insert("matte".to_owned(), json!(matte));
                holds.insert("from".to_owned(), json!(starts.map(super::plain)));
                holds.insert("seconds".to_owned(), json!(seconds.map(super::plain)));
                // The part of the sequence it draws, where it is not the whole.
                match (starts, seconds) {
                    (None, None) => {}
                    (starts, Some(seconds)) => at.push(format!(
                        "{} s from {} s",
                        super::plain(*seconds),
                        super::plain(starts.unwrap_or(0.0))
                    )),
                    (Some(starts), None) => at.push(format!("from {} s", super::plain(*starts))),
                }
                building.nodes.push(Node {
                    id: id.clone(),
                    kind: Kind::Render,
                    title: render_title(*pass, matte.as_deref()),
                    detail: if at.is_empty() {
                        "at the sequence's size and rate".to_owned()
                    } else {
                        at.join(" · ")
                    },
                    gives: building.made(to),
                    line: Some(line),
                    holds,
                });
                building.edge(from, &id, "sequence", 0, Gives::Video);
                building.makers.insert(to.clone(), id.clone());
                id
            }
            Step::Generate(ask) => building.model(ask, line, cast.as_ref()),
            Step::Join { parts, to, lap } => {
                let id = building.ids.of(format!("made:{to}"));
                let mut holds = Map::new();
                holds.insert("makes".to_owned(), json!(to));
                holds.insert("parts".to_owned(), json!(parts));
                holds.insert("lap".to_owned(), json!(lap));
                building.nodes.push(Node {
                    id: id.clone(),
                    kind: Kind::Join,
                    title: "Join".to_owned(),
                    detail: match lap {
                        0 => format!("{} parts, end to end", parts.len()),
                        1 => format!("{} parts, each going on from the last frame of the one before", parts.len()),
                        lap => format!("{} parts, each {lap} frames into the one before", parts.len()),
                    },
                    gives: Gives::Video,
                    line: Some(line),
                    holds,
                });
                for (nth, part) in parts.iter().enumerate() {
                    if let Some(maker) = building.makers.get(part).cloned() {
                        let gives = building.made(part);
                        building.edge(maker, &id, "part", nth, gives);
                    }
                }
                building.makers.insert(to.clone(), id.clone());
                id
            }
            Step::Run { program, arguments } => {
                // A name in braces that a step before made is read; any other is made here.
                let named: Vec<String> = arguments
                    .iter()
                    .flat_map(|argument| super::stands_for(argument))
                    .collect();
                let makes: Vec<&String> = named
                    .iter()
                    .filter(|name| !building.makers.contains_key(*name))
                    .collect();
                let id = building.ids.of(match makes.first() {
                    Some(first) => format!("made:{first}"),
                    None => "run".to_owned(),
                });
                let mut holds = Map::new();
                holds.insert("program".to_owned(), json!(program));
                holds.insert("arguments".to_owned(), json!(arguments));
                holds.insert("makes".to_owned(), json!(makes));
                building.nodes.push(Node {
                    id: id.clone(),
                    kind: Kind::Run,
                    title: Path::new(program)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| program.clone()),
                    detail: super::shown(arguments.iter().cloned()),
                    gives: makes
                        .first()
                        .map_or(Gives::File, |first| building.made(first)),
                    line: Some(line),
                    holds,
                });
                let mut read = 0;
                for name in &named {
                    if let Some(maker) = building.makers.get(name).cloned() {
                        let gives = building.made(name);
                        building.edge(maker, &id, "reads", read, gives);
                        read += 1;
                    }
                }
                for name in makes {
                    building.makers.insert(name.clone(), id.clone());
                }
                runs.push((id.clone(), arguments.clone()));
                id
            }
        };
        last = Some(node);
    }
    // A file a recipe `needs` is one a program reads: it goes to the runs that name it,
    // and to every run where none does.
    for (nth, needed) in recipe.needs.iter().enumerate() {
        let from = building.file(needed);
        let naming: Vec<&String> = runs
            .iter()
            .filter(|(_, arguments)| arguments.iter().any(|argument| argument.contains(needed.as_str())))
            .map(|(id, _)| id)
            .collect();
        let to: Vec<String> = if naming.is_empty() {
            runs.iter().map(|(id, _)| id.clone()).collect()
        } else {
            naming.into_iter().cloned().collect()
        };
        for run in to {
            building.edge(from.clone(), &run, "needs", nth, Gives::of_file(needed));
        }
    }
    // What the last step writes is the shot: its takes.
    let made = super::takes::takes(project, directory, media).unwrap_or_default();
    let chosen = made.iter().find(|take| take.chosen).map(|take| take.number);
    let wants = super::unmade(project, directory, media).map(|unmade| unmade.reason());
    // What each step made on the way to the take the shot is, or to the last where it is
    // none: a guide as it was drawn, to look at beside the take.
    if let Some(shown) = chosen.or(made.last().map(|take| take.number)) {
        for node in &mut building.nodes {
            let file = node
                .holds
                .get("makes")
                .and_then(Json::as_str)
                .filter(|makes| *makes != super::OUT)
                .and_then(|makes| super::takes::step_file(directory, media, shown, makes));
            if let Some(file) = file {
                node.holds.insert("made".to_owned(), json!({"take": shown, "path": file}));
            }
        }
    }
    let mut holds = Map::new();
    holds.insert("media".to_owned(), json!(id));
    holds.insert(
        "takes".to_owned(),
        made.iter()
            .map(|take| {
                json!({"number": take.number, "path": take.stored, "chosen": take.chosen,
                    "rolled": take.rolled, "changed": take.changed, "kept": take.kept})
            })
            .collect(),
    );
    holds.insert("wants".to_owned(), json!(wants));
    let gives = building.out;
    building.nodes.push(Node {
        id: "takes".to_owned(),
        kind: Kind::Takes,
        title: graph.name.clone(),
        detail: match (chosen, made.len()) {
            (_, 0) => "no take yet".to_owned(),
            (Some(chosen), all) => format!("take {chosen} of {all}"),
            (None, 1) => "1 take, none chosen".to_owned(),
            (None, all) => format!("{all} takes, none chosen"),
        },
        gives,
        line: None,
        holds,
    });
    if let Some(last) = last {
        building.edge(last, "takes", "take", 0, gives);
    }
    graph.nodes = building.nodes;
    graph.edges = building.edges;
    match super::plan(project, directory, id, models) {
        Ok(plan) => graph.notes = plan.notes,
        Err(error) => graph.cannot = Some(error.to_string()),
    }
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project with a court, a ball that moves across it, and a signal, whose media
    /// `shot` the recipe `text` makes, kept in a folder with the picture the recipe names.
    fn court(text: &str) -> (tempfile::TempDir, Project) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("shots")).unwrap();
        std::fs::create_dir_all(folder.path().join("refs")).unwrap();
        std::fs::write(folder.path().join("refs/court.png"), "a picture").unwrap();
        std::fs::write(folder.path().join("shots/bounce.kerfrecipe"), text).unwrap();
        let project = kerf_lang::parse_document(
            "kerf 1\nproject name=\"court\"\n\
             media shot path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480 recipe=\"shots/bounce.kerfrecipe\"\n\
             sequence main rate=24 size=1920x1080 audio=48000\n\
             sequence court name=\"The court\" rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip floor gen=solid@1 at=0f dur=96f\n\
             \x20     set three_d=true\n\
             \x20 track V2 video\n\
             \x20   clip ball gen=solid@1 at=0f dur=96f\n\
             \x20     set path=\"-0.4,-0.3 0,0.3 0.4,-0.2\" along={0f:0 95f:1}\n\
             \x20 track V3 video\n\
             \x20   clip bounce gen=signal@1 at=0f dur=96f\n\
             \x20     set value={0f:0 47f:1 95f:0}\n",
            kerf_model::Catalog::builtin(),
        )
        .expect("a project")
        .project;
        (folder, project)
    }

    const BOUNCE: &str = "kerf-recipe 1\n\
        # A basketball bouncing on a court.\n\
        render court {depth.mp4} pass=depth size=832x480 rate=16\n\
        generate model=still prompt=\"an orange basketball\" seed=3 out={ball.png}\n\
        generate model=video prompt=\"it bounces\" guide={depth.mp4} ref={ball.png} ref=../refs/court.png moves=ball signal=bounce seconds=4 seed=7 strength=0.8\n";

    #[test]
    fn a_longer_shot_is_laid_out_as_its_parts_and_their_join() {
        let (folder, project) = court(
            "kerf-recipe 1\n\
             render court {d1.mp4} pass=depth seconds=5.0625\n\
             render court {d2.mp4} pass=depth from=5 seconds=5.0625\n\
             generate model=video prompt=\"it bounces\" guide={d1.mp4} out={a.mp4}\n\
             generate model=video prompt=\"it bounces\" guide={d2.mp4} first={a.mp4} from=5 out={b.mp4}\n\
             join {a.mp4} {b.mp4} {out} lap=1\n",
        );
        let graph = graph(&project, folder.path(), "shot", &models(folder.path())).unwrap();
        let second = node(&graph, "made:d2.mp4");
        assert_eq!(second.detail, "5.0625 s from 5 s");
        assert_eq!((second.holds["from"].as_str(), second.holds["seconds"].as_str()), (Some("5"), Some("5.0625")));
        assert_eq!(node(&graph, "made:b.mp4").holds["from"], 5.0);
        let join = node(&graph, "made:out");
        assert_eq!(join.kind, Kind::Join);
        assert_eq!(join.line, Some(6));
        assert_eq!(join.detail, "2 parts, each going on from the last frame of the one before");
        assert_eq!(join.holds["lap"], 1);
        let edges: Vec<String> = graph
            .edges
            .iter()
            .filter(|edge| ["first", "part", "take"].contains(&edge.port.as_str()))
            .map(|edge| format!("{} -> {} {}[{}]", edge.from, edge.to, edge.port, edge.nth))
            .collect();
        assert_eq!(
            edges,
            [
                "made:a.mp4 -> made:b.mp4 first[0]",
                "made:a.mp4 -> made:out part[0]",
                "made:b.mp4 -> made:out part[1]",
                "made:out -> takes take[0]",
            ]
        );
    }

    #[test]
    fn a_step_says_what_it_made_on_the_way_to_the_take_that_shows() {
        let (folder, project) = court(BOUNCE);
        let dir = folder.path();
        let made = |graph: &Graph, id: &str| node(graph, id).holds.get("made").cloned();
        // No take yet: nothing was drawn.
        let none = graph(&project, dir, "shot", &models(dir)).unwrap();
        assert_eq!(made(&none, "made:depth.mp4"), None);
        // Two takes, the guide of each kept in its working folder; the shot is none of them,
        // so the last shows.
        for take in ["001", "002"] {
            let work = dir.join(format!("shots/bounce.takes/{take}.work"));
            std::fs::create_dir_all(&work).unwrap();
            std::fs::write(work.join("depth.mp4"), "a guide").unwrap();
            std::fs::write(dir.join(format!("shots/bounce.takes/{take}.mp4")), "a take").unwrap();
        }
        let shown = graph(&project, dir, "shot", &models(dir)).unwrap();
        assert_eq!(
            made(&shown, "made:depth.mp4"),
            Some(json!({"take": 2, "path": "shots/bounce.takes/002.work/depth.mp4"}))
        );
        // A step whose file is not there says nothing, and the shot's own file is its takes.
        assert_eq!(made(&shown, "made:ball.png"), None);
        assert_eq!(made(&shown, "made:out"), None);
    }

    fn models(folder: &Path) -> Models {
        let file = folder.join("models.json");
        std::fs::write(
            &file,
            r#"{"still": {"run": ["still", "{request}"]},
                "video": {"run": ["video", "{request}"],
                  "takes": {"about": "follows depth", "size": [832, 480], "rate": 16, "seconds": 5,
                    "reads": ["guide", "ref", "signal", "moves"],
                    "more": {"steps": 20, "strength": 1.0}}}}"#,
        )
        .unwrap();
        Models::read(&file).unwrap()
    }

    fn node<'a>(graph: &'a Graph, id: &str) -> &'a Node {
        graph
            .nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap_or_else(|| panic!("no node {id} among {:?}", graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>()))
    }

    fn edges(graph: &Graph) -> Vec<String> {
        graph
            .edges
            .iter()
            .map(|edge| {
                format!(
                    "{} -> {} {}{} {:?}",
                    edge.from,
                    edge.to,
                    edge.port,
                    if edge.nth > 0 { format!("[{}]", edge.nth) } else { String::new() },
                    edge.gives
                )
            })
            .collect()
    }

    #[test]
    fn a_recipe_is_its_steps_and_what_they_read_wired() {
        let (folder, project) = court(BOUNCE);
        let graph = graph(&project, folder.path(), "shot", &models(folder.path())).unwrap();
        assert_eq!((graph.name.as_str(), graph.recipe.as_str()), ("bounce", "shots/bounce.kerfrecipe"));
        assert_eq!(graph.text, BOUNCE);
        assert_eq!(graph.error, None);
        assert_eq!(
            edges(&graph),
            [
                "sequence:court -> made:depth.mp4 sequence Video",
                "made:depth.mp4 -> made:out guide Video",
                "made:ball.png -> made:out ref Image",
                "file:../refs/court.png -> made:out ref[1] Image",
                "moves:ball -> made:out moves Path",
                "signal:bounce -> made:out signal Signal",
                "made:out -> takes take Video",
            ]
        );
        // A step is found by the line it is, and says what its line holds.
        let depth = node(&graph, "made:depth.mp4");
        assert_eq!((depth.kind, depth.line), (Kind::Render, Some(3)));
        assert_eq!((depth.title.as_str(), depth.detail.as_str()), ("Depth guide", "832×480 · 16 fps"));
        assert_eq!(depth.holds["pass"], "depth");
        let still = node(&graph, "made:ball.png");
        assert_eq!((still.kind, still.line, still.gives), (Kind::Model, Some(4), Gives::Image));
        assert_eq!((still.holds["seed"].as_u64(), still.holds["prompt"].as_str()), (Some(3), Some("an orange basketball")));
        let video = node(&graph, "made:out");
        assert_eq!((video.line, video.gives), (Some(5), Gives::Video));
        assert_eq!(video.holds["setUp"], true);
        // What the line sets, with what the model has when it does not; then what it leaves.
        assert_eq!(
            video.holds["knobs"],
            json!([{"name": "strength", "value": "0.8", "default": 1.0},
                   {"name": "steps", "value": null, "default": 20}])
        );
        assert_eq!(video.holds["reads"], json!(["guide", "ref", "signal", "moves"]));
        // What is read of the project says what it is.
        let sequence = node(&graph, "sequence:court");
        assert_eq!((sequence.title.as_str(), sequence.detail.as_str()), ("The court", "1920×1080 · 24 fps · 4 s"));
        let signal = node(&graph, "signal:bounce");
        assert_eq!(signal.detail, "court · 96 frames at 24");
        let values = signal.holds["values"].as_array().unwrap();
        assert_eq!(values.len(), 96);
        assert_eq!((values[0].as_f64(), values[47].as_f64(), values[95].as_f64()), (Some(0.0), Some(1.0), Some(0.0)));
        let moves = node(&graph, "moves:ball");
        assert_eq!(moves.holds["path"], "-0.4,-0.3 0,0.3 0.4,-0.2");
        assert_eq!(moves.detail, "where it goes in court");
        let file = node(&graph, "file:../refs/court.png");
        assert_eq!((file.title.as_str(), file.detail.as_str(), &file.holds["inProject"]), ("court.png", "refs", &json!("refs/court.png")));
        // No take yet, and it wants one.
        let takes = node(&graph, "takes");
        assert_eq!((takes.title.as_str(), takes.detail.as_str()), ("bounce", "no take yet"));
        assert_eq!(takes.holds["wants"], "its file is not there yet");
        assert_eq!(graph.cannot, None);
    }

    #[test]
    fn what_cannot_be_made_is_laid_out_and_says_why() {
        let (folder, project) = court(&BOUNCE.replace("signal=bounce", "signal=bounce cast=Ada"));
        let none = Models::read(&folder.path().join("none.json")).unwrap();
        let graph = graph(&project, folder.path(), "shot", &none).unwrap();
        assert_eq!(node(&graph, "made:out").holds["setUp"], false);
        let ada = node(&graph, "cast:Ada");
        assert_eq!((ada.kind, ada.detail.as_str()), (Kind::Cast, "not in cast.kerfcast"));
        assert!(graph.cannot.is_some());
        // A text that is no recipe has no nodes and says its line.
        std::fs::write(folder.path().join("shots/bounce.kerfrecipe"), "kerf-recipe 1\nrender court\n").unwrap();
        let graph = super::graph(&project, folder.path(), "shot", &none).unwrap();
        assert!(graph.nodes.is_empty() && graph.edges.is_empty());
        assert!(graph.error.as_deref().unwrap().starts_with("line 2 of the recipe"));
        assert_eq!(graph.text, "kerf-recipe 1\nrender court\n");
    }

    #[test]
    fn a_program_reads_what_was_made_and_what_the_recipe_needs() {
        let (folder, project) = court(
            "kerf-recipe 1\nneeds scene.json look.cube\nrun vork scene.json --out {rough.mp4}\nrun grade {rough.mp4} --out {out}\n",
        );
        let graph = graph(&project, folder.path(), "shot", &models(folder.path())).unwrap();
        assert_eq!(
            edges(&graph),
            [
                "made:rough.mp4 -> made:out reads Video",
                "file:scene.json -> made:rough.mp4 needs File",
                "file:look.cube -> made:rough.mp4 needs[1] File",
                "file:look.cube -> made:out needs[1] File",
                "made:out -> takes take Video",
            ]
        );
        assert_eq!(node(&graph, "made:rough.mp4").title, "vork");
        assert_eq!(node(&graph, "file:scene.json").detail, "not there");
    }

    #[test]
    fn a_workflow_a_line_names_is_a_file_the_model_reads() {
        let (folder, project) = court(&BOUNCE.replace("strength=0.8", "strength=0.8 workflow=bounce.video.comfy.json"));
        std::fs::write(folder.path().join("shots/bounce.video.comfy.json"), "{}").unwrap();
        let graph = graph(&project, folder.path(), "shot", &models(folder.path())).unwrap();
        assert!(edges(&graph).contains(&"file:bounce.video.comfy.json -> made:out workflow File".to_owned()));
        let file = node(&graph, "file:bounce.video.comfy.json");
        assert_eq!((file.detail.as_str(), &file.holds["inProject"]), ("shots", &json!("shots/bounce.video.comfy.json")));
        assert_eq!(node(&graph, "made:out").holds["workflow"], "bounce.video.comfy.json");
        // A line that names none says so.
        assert_eq!(node(&graph, "made:ball.png").holds["workflow"], Json::Null);
        assert_eq!(graph.cannot, None);
    }

    #[test]
    fn a_graph_is_json_an_editor_reads() {
        let (folder, project) = court(BOUNCE);
        let graph = graph(&project, folder.path(), "shot", &models(folder.path())).unwrap();
        let value = serde_json::to_value(&graph).unwrap();
        assert_eq!(value["nodes"][0]["kind"], "sequence");
        assert_eq!(value["nodes"][0]["gives"], "video");
        assert_eq!(value["edges"][0], json!({"from": "sequence:court", "to": "made:depth.mp4", "port": "sequence", "nth": 0, "gives": "video"}));
        assert!(value.get("error").is_none());
    }
}
