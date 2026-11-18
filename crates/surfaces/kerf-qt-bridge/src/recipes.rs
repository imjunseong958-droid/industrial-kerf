//! How a shot is made, for the editor's graph: a media's steps laid out with
//! what they read ([`kerf_engine::recipe::graph`]), and a person's change to
//! them.
//!
//! Steps the document holds under the media are changed by an edit, as any
//! of the document is: the change becomes `step` lines that kerfd applies,
//! one revision that undo takes back. Steps in a recipe's file are changed
//! in its text, with every comment and every other line as it was
//! ([`kerf_engine::recipe::text`]). The editor sends the same change for
//! both: the steps' text as it read them, and the line to change.
//!
//! Nothing here runs a step. A making is `media_make`, which shows what
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
    /// A step to add, by its kind: `guide`, `model`, or `join`
    /// ([`kerf_shots::graph::step`]).
    #[serde(default)]
    add: Option<String>,
    /// Take the step on `line` out.
    #[serde(default)]
    remove: bool,
    /// Go back to the recipe this take was made from ([`kerf_shots::takes::restore`]).
    #[serde(default)]
    restore: Option<u32>,
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

/// What a change to a media's steps comes to.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Changed {
    /// The line a step was added on, where one was.
    added: Option<usize>,
    /// The lines of the edit that makes it, for steps the document holds: empty for a
    /// recipe's file, which is written already.
    script: Vec<String>,
}

/// Make `edit` to its media's steps. A recipe's file is written in place. Steps the
/// document holds are not changed here: what would change them is given back as the
/// lines of an edit, for kerfd to apply.
fn change(project: &Project, directory: &Path, edit: &Edit, models: &Models) -> Result<Changed, String> {
    let (home, there) = kerf_shots::graph::recipe(project, directory, &edit.media)?;
    let in_document = home == kerf_engine::recipe::Home::Document;
    // What the editor read is what the document has: steps changed since are left alone.
    if in_document && there != edit.was {
        return Err(format!(
            "the steps of `{}` were changed since they were read; they are left as that left them",
            edit.media
        ));
    }
    if let Some(take) = edit.restore {
        if !in_document {
            return kerf_shots::takes::restore(project, directory, &edit.media, take, &edit.was)
                .map(|()| Changed::default());
        }
        let restored = kerf_shots::takes::restored(project, directory, &edit.media, take)?;
        if restored.text == edit.was && restored.files.is_empty() {
            return Err(format!("the steps are as take {take} was made from them already"));
        }
        for (file, _, held) in &restored.files {
            kerf_shots::workflow::write(file, held)?;
        }
        return Ok(Changed {
            added: None,
            script: kerf_shots::graph::step_lines(&edit.media, &edit.was, &restored.text)?,
        });
    }
    let now = match &edit.text {
        Some(text) => text.clone(),
        None if edit.remove => kerf_shots::graph::removed(&edit.was, edit.line)?,
        None if edit.add.is_some() => {
            let kind = edit.add.as_deref().unwrap_or_default();
            let (after, line) = kerf_shots::graph::step(project, models, &edit.was, kind)?;
            kerf_shots::graph::added(&edit.was, after, &line)?
        }
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
    let added = edit
        .add
        .as_ref()
        .and_then(|_| kerf_shots::graph::added_at(&edit.was, &now));
    if in_document {
        return Ok(Changed {
            added,
            script: kerf_shots::graph::step_lines(&edit.media, &edit.was, &now)?,
        });
    }
    text::write(project, directory, &edit.media, &edit.was, &now).map_err(|error| error.to_string())?;
    Ok(Changed {
        added,
        script: Vec::new(),
    })
}

/// Have kerfd run `lines`, an edit of the person's own: one revision, which undo takes back.
pub(crate) fn apply(client: &kerf_runtime::RuntimeClient, lines: &[String]) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    let revision = client.get_state().map_err(|error| error.message)?.revision;
    client
        .apply_script_request("ui", &crate::key(), &lines.join("\n"), revision)
        .map(|_| ())
        .map_err(|error| error.message)
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
    let (models, models_error) = models();
    // Steps the document holds change by an edit, which kerfd applies: the graph shown
    // after is of the project as that left it.
    let changed = change(&project, &shared.directory, edit, &models)
        .and_then(|changed| apply(client, &changed.script).map(|()| changed));
    let project = match &changed {
        Ok(changed) if !changed.script.is_empty() => match self::project(client) {
            Ok(project) => project,
            Err(why) => return failed(tx, &edit.media, why),
        },
        _ => project,
    };
    let shown = event(&project, &shared.directory, &edit.media, &models, models_error);
    let added = match changed {
        Ok(changed) => changed.added,
        Err(why) => {
            failed(tx, &edit.media, why);
            None
        }
    };
    match shown {
        Ok(mut event) => {
            event["edited"] = json!(true);
            // The line of the step that was added, for what shows the graph to choose it.
            if let Some(line) = added {
                event["added"] = json!(line);
            }
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
            add: None,
            remove: false,
            restore: None,
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
        change(&project, dir, &set("seed", Some("8")), &models).unwrap();
        let now = fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap();
        assert_eq!(now, RECIPE.replace("seed=7", "seed=8"));
        // The editor's copy is now old: its next change is refused, and the file kept.
        let stale = change(&project, dir, &set("seed", Some("9")), &models).unwrap_err();
        assert!(stale.contains("changed by another program"), "{stale}");
        assert_eq!(fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(), now);
        // A knob taken off the line, and the whole text set.
        let mut off = set("strength", None);
        off.was = now.clone();
        change(&project, dir, &off, &models).unwrap();
        let now = fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap();
        assert!(now.ends_with("seed=8\n") && now.contains("# The opening shot."));
        let whole = Edit {
            text: Some(RECIPE.into()),
            was: now,
            ..set("", None)
        };
        change(&project, dir, &whole, &models).unwrap();
        assert_eq!(fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(), RECIPE);
        // What a line cannot take is refused as a making would refuse it.
        let refused = change(&project, dir, &set("seed", Some("soon")), &models).unwrap_err();
        assert!(refused.contains("`seed=soon` is not a whole number"), "{refused}");
        // A step added, by its kind, and taken out again: the file as it was.
        let was = fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap();
        let add = Edit { add: Some("guide".into()), was: was.clone(), ..set("", None) };
        // Before the last step and what is written over it, which stays its own.
        assert_eq!(change(&project, dir, &add, &models).unwrap().added, Some(2));
        let more = fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap();
        assert!(more.starts_with("kerf-recipe 1\nrender s1 {guide.mp4} pass=depth\n# The opening shot.\ngenerate model=dawn"), "{more}");
        let out = Edit { remove: true, line: 2, was: more, ..set("", None) };
        assert_eq!(change(&project, dir, &out, &models).unwrap(), Changed::default());
        assert_eq!(fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(), was);
        let last = Edit { remove: true, line: 3, was: was.clone(), ..set("", None) };
        assert!(change(&project, dir, &last, &models).unwrap_err().contains("does not read"));
        // A models file that does not read is said with the graph, which is still shown.
        let shown = event(&project, dir, "m1", &models, Some("it is not JSON".into())).unwrap();
        assert_eq!(shown["modelsError"], "it is not JSON");
        assert!(event(&project, dir, "m9", &models, None).unwrap_err().contains("no media `m9`"));
    }

    /// Steps the document holds are shown as a recipe's are, and a change to them is the
    /// lines of an edit: nothing is written here, and no file says how the media is made.
    #[test]
    fn steps_the_document_holds_are_shown_as_a_graph_and_changed_by_an_edit() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=96f video=320x180\n  generate model=dawn prompt=\"a boat leaves\" seed=7 strength=0.6\nsequence s1 rate=24 size=320x180\n",
        )
        .unwrap();
        let models = Models::default();
        let shown = event(&project, dir, "m1", &models, None).unwrap();
        assert_eq!(shown["document"], true);
        assert_eq!((shown["recipe"].as_str(), shown["name"].as_str()), (Some(""), Some("m1")));
        let was = shown["text"].as_str().unwrap().to_owned();
        assert_eq!(was, "kerf-recipe 1\ngenerate model=dawn prompt=\"a boat leaves\" seed=7 strength=0.6\n");
        assert_eq!(shown["nodes"][0]["line"], 2);
        let on = |line: usize| Edit { was: was.clone(), line, ..set("", None) };

        // One name on one step is that step set, by its number.
        let seeded = Edit { key: "seed".into(), value: Some("8".into()), ..on(2) };
        assert_eq!(
            change(&project, dir, &seeded, &models).unwrap(),
            Changed {
                added: None,
                script: vec!["step m1 set 1 generate model=dawn prompt=\"a boat leaves\" seed=8 strength=0.6".into()],
            }
        );
        // A step added, by its kind: every step written again, and the line it is on said.
        let guide = Edit { add: Some("guide".into()), ..on(0) };
        let added = change(&project, dir, &guide, &models).unwrap();
        assert_eq!(added.added, Some(2));
        assert_eq!(
            added.script,
            [
                "step m1 remove 1",
                "step m1 add render s1 {guide.mp4} pass=depth",
                "step m1 add generate model=dawn prompt=\"a boat leaves\" seed=7 strength=0.6",
            ]
        );
        // The lines make the change, as kerfd applies them.
        let edited = kerf_lang::apply_script(&project, kerf_model::Catalog::builtin(), &added.script.join("\n"))
            .unwrap()
            .project;
        assert_eq!(edited.media("m1").unwrap().steps.len(), 2);
        // What a step cannot take is refused as a making would refuse it, and a program to
        // run has no place in the document.
        let wrong = Edit { key: "seed".into(), value: Some("soon".into()), ..on(2) };
        assert!(change(&project, dir, &wrong, &models).unwrap_err().contains("`seed=soon` is not a whole number"));
        let ran = Edit { text: Some("kerf-recipe 1\nrun vork render --out {out}\n".into()), ..on(0) };
        assert!(change(&project, dir, &ran, &models).unwrap_err().contains("`run` is a recipe file's alone"));
        // Steps changed since the editor read them are left as that left them.
        let stale = Edit { was: "kerf-recipe 1\ngenerate model=dawn\n".into(), ..seeded };
        assert!(change(&project, dir, &stale, &models).unwrap_err().contains("were changed since they were read"));
        // Nothing was written: the document is where the steps are.
        assert!(!dir.join("shots").exists());
    }
}
