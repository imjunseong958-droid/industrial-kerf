//! A media's recipe as a graph, with what runs each of its models.

use kerf_engine::recipe::model::Models;
use kerf_engine::recipe::text::{self, Change};
use kerf_model::Project;
use serde_json::{json, Value};
use std::path::Path;

/// The recipe of media `id` as a graph: its nodes and edges, its text, what
/// would refuse its making (`cannot`) and what is worth knowing first
/// (`notes`), as [`kerf_engine::recipe::graph`] lays it out. A model its
/// entry runs with a program whose workflow can be opened says so under
/// `runner`: which program, and which workflow.
pub fn shown(project: &Project, directory: &Path, id: &str, models: &Models) -> Result<Value, String> {
    let graph =
        kerf_engine::recipe::graph(project, directory, id, models).map_err(|error| error.to_string())?;
    let mut shown = serde_json::to_value(&graph).map_err(|error| error.to_string())?;
    // Which workflow a model is run with is the person's own models file's to say.
    for node in shown["nodes"].as_array_mut().into_iter().flatten() {
        let workflow = node["model"]
            .as_str()
            .and_then(|model| models.command(model).ok())
            .and_then(crate::workflow::runs);
        if let Some(workflow) = workflow {
            node["runner"] = json!({"kind": "comfy", "workflow": workflow});
        }
    }
    Ok(shown)
}

/// One name on one line of a recipe to set, or with no `value` to take off.
#[derive(Clone, Copy, Debug)]
pub struct Word<'a> {
    /// The line, from 1.
    pub line: usize,
    pub key: &'a str,
    /// Which of the words of that name on the line, from 0.
    pub nth: usize,
    pub value: Option<&'a str>,
}

/// `recipe` with `word` set or taken off, and nothing else of it changed
/// ([`text::changed`]): refused as a making would refuse what it makes.
pub fn changed(recipe: &str, word: Word) -> Result<String, String> {
    let change = match word.value {
        Some(value) => Change::Set {
            key: word.key,
            nth: word.nth,
            value,
        },
        None => Change::Remove {
            key: word.key,
            nth: word.nth,
        },
    };
    text::changed(recipe, word.line, change).map_err(|error| error.to_string())
}

/// The recipe media `id` names: the path the document names it by, and its text.
pub fn recipe(project: &Project, directory: &Path, id: &str) -> Result<(String, String), String> {
    let media = project
        .media(id)
        .ok_or_else(|| format!("no media `{id}` in this project"))?;
    let name = media
        .recipe
        .as_deref()
        .ok_or_else(|| format!("media `{id}` names no recipe"))?;
    // The recipe's own file, named from its own folder.
    let own = Path::new(name)
        .file_name()
        .map(|own| own.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (file, _) =
        kerf_engine::recipe::beside(project, directory, id, &own).map_err(|error| error.to_string())?;
    let text = std::fs::read_to_string(&file)
        .map_err(|error| format!("cannot read the recipe {}: {error}", file.display()))?;
    Ok((name.to_owned(), text))
}
