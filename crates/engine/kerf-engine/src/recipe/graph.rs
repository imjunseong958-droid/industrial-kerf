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
use super::{stored, text::step_lines, within, Recipe, Step};
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
    /// Steps the document holds under the media have no recipe to name: `recipe`
    /// is then empty, and `document` says so.
    pub media: String,
    pub recipe: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub document: bool,
    pub name: String,
    /// The steps' text: as the recipe's file holds it, or written out from the
    /// document, a step a line after the first.
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
            let inside = within(building.folder, file, "file a step names").ok();
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
                    // The clips of its sequence that play a sound, whose moments its keys
                    // can be read from: each once, by its id and the name it shows.
                    let mut sounds: Vec<Json> = Vec::new();
                    for heard in sequence.tracks.iter().flat_map(|track| &track.clips) {
                        let plays = match &heard.content {
                            Content::Media { media, .. } => project
                                .media(media.as_str())
                                .is_some_and(|media| media.audio.is_some()),
                            _ => false,
                        };
                        // A picture and its sound are two clips of one recording: its sound's.
                        let track_of = sequence
                            .tracks
                            .iter()
                            .find(|track| track.clips.iter().any(|clip| clip.id == heard.id));
                        let on_sound = track_of.is_some_and(|track| track.kind == kerf_model::TrackKind::Audio);
                        if plays && on_sound {
                            sounds.push(json!({
                                "clip": heard.id.as_str(),
                                "name": heard.name.clone().unwrap_or_else(|| heard.id.to_string()),
                            }));
                        }
                    }
                    holds.insert("sounds".to_owned(), json!(sounds));
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

/// The steps that make media `id` of `project`, kept in `directory`, as a
/// graph. Steps that do not end in the media's file are a graph with no nodes
/// that says why (`error`); ones that read and cannot be made say that
/// (`cannot`) and are laid out all the same. Nothing is run.
pub fn graph(project: &Project, directory: &Path, id: &str, models: &Models) -> Result<Graph> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let text = super::written(project, id)?;
    let mut graph = Graph {
        media: id.to_owned(),
        // The steps are the document's own: no file is named, and the shot goes by its media.
        recipe: String::new(),
        document: true,
        name: id.to_owned(),
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
    let folder = std::path::PathBuf::new();
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
        };
        last = Some(node);
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
