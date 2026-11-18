//! How a shot is made, for the editor's graph: a media's recipe laid out as
//! its steps and what they read ([`kerf_engine::recipe::graph`]), and a
//! person's change to it written back into its text, with every comment and
//! every other line as it was ([`kerf_engine::recipe::text`]).
//!
//! Nothing here runs a recipe. A making is `media_make`, which shows what
//! would run and asks first.

use crate::Shared;
use kerf_engine::recipe::model::Models;
use kerf_engine::recipe::text;
use kerf_model::Project;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc::SyncSender;

/// A change to a media's recipe: one name on one line, or with `text` the
/// whole of it.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Edit {
    media: String,
    /// The recipe as the editor read it. A file that holds anything else
    /// was changed since, and is left alone.
    was: String,
    /// The whole recipe, in place of `was`.
    #[serde(default)]
    text: Option<String>,
    /// The line, from 1, and the name on it to set: the `nth` word of that
    /// name, to `value`, or taken off the line where there is no value.
    #[serde(default)]
    line: usize,
    #[serde(default)]
    key: String,
    #[serde(default)]
    nth: usize,
    #[serde(default)]
    value: Option<String>,
}

/// The person's models, and why their file does not read where it does not:
/// a graph is laid out all the same, with no model set up.
fn models() -> (Models, Option<String>) {
    match Models::load() {
        Ok(models) => (models, None),
        Err(error) => (Models::default(), Some(error.to_string())),
    }
}

/// The `recipe` event of media `id`: its recipe as a graph.
pub(crate) fn event(
    project: &Project,
    directory: &Path,
    id: &str,
    models: &Models,
    models_error: Option<String>,
) -> Result<Value, String> {
    let mut event = kerf_shots::graph::shown(project, directory, id, models)?;
    event["type"] = json!("recipe");
    if let Some(error) = models_error {
        event["modelsError"] = json!(error);
    }
    Ok(event)
}

/// Make `edit` to its media's recipe, in its file.
fn change(project: &Project, directory: &Path, edit: &Edit) -> Result<(), String> {
    let now = match &edit.text {
        Some(text) => text.clone(),
        None => {
            let word = kerf_shots::graph::Word {
                line: edit.line,
                key: &edit.key,
                nth: edit.nth,
                value: edit.value.as_deref(),
            };
            kerf_shots::graph::changed(&edit.was, word)?
        }
    };
    text::write(project, directory, &edit.media, &edit.was, &now).map_err(|error| error.to_string())
}

/// The project as kerfd has it.
fn project(client: &kerf_runtime::RuntimeClient) -> Result<Project, String> {
    client
        .get_document(false)
        .and_then(|(_, text)| kerf_runtime::parse_document(&text))
        .map_err(|error| error.message)
}

/// What cannot be shown or changed of media `id`'s recipe: an `error` that
/// names the media, so that what shows its graph can say so there.
fn failed(tx: &SyncSender<Value>, id: &str, why: String) {
    crate::emit(
        tx,
        json!({"type": "error", "message": why, "command": "recipe", "media": id}),
    );
}

/// Answer `recipe` for media `id`: how it is made, as a graph.
pub(crate) fn get(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    id: &str,
    tx: &SyncSender<Value>,
) {
    let (models, models_error) = models();
    match project(client)
        .and_then(|project| event(&project, &shared.directory, id, &models, models_error))
    {
        Ok(event) => crate::emit(tx, event),
        Err(why) => failed(tx, id, why),
    }
}

/// Make `edit` and answer `recipe` with the graph as it now is. A change
/// that is refused is an `error` with why, followed by the graph as the
/// file has it, so that what shows it goes back to what is true.
pub(crate) fn edit(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    edit: &Edit,
    tx: &SyncSender<Value>,
) {
    let project = match project(client) {
        Ok(project) => project,
        Err(why) => return failed(tx, &edit.media, why),
    };
    let refused = change(&project, &shared.directory, edit).err();
    let (models, models_error) = models();
    let shown = event(&project, &shared.directory, &edit.media, &models, models_error);
    if let Some(why) = refused {
        failed(tx, &edit.media, why);
    }
    match shown {
        Ok(mut event) => {
            event["edited"] = json!(true);
            crate::emit(tx, event);
        }
        Err(why) => failed(tx, &edit.media, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const RECIPE: &str = "kerf-recipe 1\n# The opening shot.\ngenerate model=dawn prompt=\"a boat leaves\" seed=7 strength=0.6\n";

    fn opening(directory: &Path) -> Project {
        fs::create_dir_all(directory.join("shots")).unwrap();
        fs::write(directory.join("shots/open.kerfrecipe"), RECIPE).unwrap();
        kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=96f video=320x180 recipe=\"shots/open.kerfrecipe\"\nsequence s1 rate=24 size=320x180\n",
        )
        .unwrap()
    }

    fn set(key: &str, value: Option<&str>) -> Edit {
        Edit {
            media: "m1".into(),
            was: RECIPE.into(),
            text: None,
            line: 3,
            key: key.into(),
            nth: 0,
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn a_recipe_is_shown_as_a_graph_and_changed_in_its_text() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let project = opening(dir);
        let models = Models::default();
        let shown = event(&project, dir, "m1", &models, None).unwrap();
        assert_eq!(shown["type"], "recipe");
        assert_eq!((shown["media"].as_str(), shown["name"].as_str()), (Some("m1"), Some("open")));
        assert_eq!(shown["text"], RECIPE);
        let kinds: Vec<&str> = shown["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["model", "takes"]);
        assert_eq!(shown["nodes"][0]["line"], 3);
        // No model is set up, which is why it cannot be made, and is said.
        assert!(shown["cannot"].as_str().unwrap().contains("no model `dawn` is set up"));

        // One name on one line: the comment and the rest of the line stay.
        change(&project, dir, &set("seed", Some("8"))).unwrap();
        let now = fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap();
        assert_eq!(now, RECIPE.replace("seed=7", "seed=8"));
        // The editor's copy is now old: its next change is refused, and the file kept.
        let stale = change(&project, dir, &set("seed", Some("9"))).unwrap_err();
        assert!(stale.contains("changed by another program"), "{stale}");
        assert_eq!(fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(), now);
        // A knob taken off the line, and the whole text set.
        let mut off = set("strength", None);
        off.was = now.clone();
        change(&project, dir, &off).unwrap();
        let now = fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap();
        assert!(now.ends_with("seed=8\n") && now.contains("# The opening shot."));
        let whole = Edit {
            text: Some(RECIPE.into()),
            was: now,
            ..set("", None)
        };
        change(&project, dir, &whole).unwrap();
        assert_eq!(fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(), RECIPE);
        // What a line cannot take is refused as a making would refuse it.
        let refused = change(&project, dir, &set("seed", Some("soon"))).unwrap_err();
        assert!(refused.contains("`seed=soon` is not a whole number"), "{refused}");
        // A models file that does not read is said with the graph, which is still shown.
        let shown = event(&project, dir, "m1", &models, Some("it is not JSON".into())).unwrap();
        assert_eq!(shown["modelsError"], "it is not JSON");
        assert!(event(&project, dir, "m9", &models, None).unwrap_err().contains("no media `m9`"));
    }
}
