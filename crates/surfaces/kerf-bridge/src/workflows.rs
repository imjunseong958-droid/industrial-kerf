//! The workflow behind a model, for the editor's graph: a `generate` step
//! whose model a person's entry runs with `kerf-comfy` can be opened as the
//! ComfyUI graph it runs, every node and input of it, and a value in it set.
//!
//! Setting one changes nothing on a ComfyUI server and nothing of
//! `kerf-comfy`. The workflow as it comes is copied into the project, beside
//! the shot, with that value changed and the rest as it was written
//! ([`kerf_workflow::detail`]), and the step is given `workflow=<file>`, by an
//! edit where the document holds it and in its recipe's text where a file does,
//! which has the model's program run the copy for this shot
//! (`docs/shots.md`). The copy is a file of the project: it travels with it,
//! a take is held to it, and taking the word off the line goes back to the
//! workflow as it comes.
//!
//! A model run by anything else has no workflow to show, and its node says
//! nothing of one. Which program runs a model is read from the person's own
//! models file and from nothing in the project.
//!
//! The reading and the rule are `kerf-shots`', which an agent's tools go
//! through too: this is the editor's end of them, its commands and events.

use crate::Shared;
use kerf_engine::recipe::model::Models;
use kerf_model::Project;
use kerf_shots::workflow::{self, Set};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::mpsc::SyncSender;

/// A value of a workflow to set: input `input` of node `node`.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Edit {
    media: String,
    /// The `generate` line the workflow is of, from 1.
    line: usize,
    /// The workflow's text as the editor read it.
    was: String,
    node: String,
    input: String,
    /// What to set it to; or, with `link`, the node and the output of it to read.
    #[serde(default)]
    value: Value,
    #[serde(default)]
    link: Option<(String, u64)>,
}

/// The `workflow` event of the step on `line` of media `id`'s recipe.
fn event(
    project: &Project,
    shared: &Shared,
    id: &str,
    line: usize,
    models: &Models,
) -> Result<Value, String> {
    let read = workflow::read(project, &shared.directory, id, line, models)?;
    let mut event = workflow::shown(id, line, &read)?;
    event["type"] = json!("workflow");
    // What each input takes on the server, where that has been asked already.
    if let Some(taken) = workflow::taken_before(models, &read) {
        workflow::with_taken(&mut event, &taken);
    }
    Ok(event)
}

/// The project as kerfd has it.
fn project(client: &kerf_runtime::RuntimeClient) -> Result<Project, String> {
    client
        .get_document(false)
        .and_then(|(_, text)| kerf_runtime::parse_document(&text))
        .map_err(|error| error.message)
}

fn failed(tx: &SyncSender<Value>, id: &str, why: String) {
    crate::emit(
        tx,
        json!({"type": "error", "message": why, "command": "workflow", "media": id}),
    );
}

/// Answer `workflow` for the `generate` step on `line` of media `id`'s recipe.
pub(crate) fn get(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    id: &str,
    line: usize,
    tx: &SyncSender<Value>,
) {
    let said = Models::load()
        .map_err(|error| error.to_string())
        .and_then(|models| {
            let project = project(client)?;
            event(&project, shared, id, line, &models)
        });
    match said {
        Ok(event) => {
            let asked = event.get("taken").is_some();
            crate::emit(tx, event);
            if !asked {
                ask_taken(shared, client, id, line, tx);
            }
        }
        Err(why) => failed(tx, id, why),
    }
}

/// Ask the model's program what the server takes at each input of the workflow, off this
/// worker, and say the workflow again with it: the choices of a sampler's name, the least
/// and most of a number. A program that cannot say leaves the workflow as it was shown,
/// with why (`takenError`), since it is still seen and set without.
fn ask_taken(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    id: &str,
    line: usize,
    tx: &SyncSender<Value>,
) {
    let Ok(project) = project(client) else {
        return;
    };
    let (directory, id, tx) = (shared.directory.clone(), id.to_owned(), tx.clone());
    std::thread::spawn(move || {
        let Ok(models) = Models::load() else {
            return;
        };
        let Ok(read) = workflow::read(&project, &directory, &id, line, &models) else {
            return;
        };
        let asked = workflow::taken(&models, &read);
        let Ok(mut event) = workflow::shown(&id, line, &read) else {
            return;
        };
        event["type"] = json!("workflow");
        match asked {
            Ok(taken) => workflow::with_taken(&mut event, &taken),
            Err(why) => event["takenError"] = json!(why),
        }
        crate::emit(&tx, event);
    });
}

/// Make `edit` and answer `workflow` as it now is, then `recipe`, whose
/// line may have gained its `workflow=`. A change that is refused is an
/// `error` with why, followed by the workflow as it is.
pub(crate) fn edit(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    edit: &Edit,
    tx: &SyncSender<Value>,
) {
    let models = match Models::load() {
        Ok(models) => models,
        Err(error) => return failed(tx, &edit.media, error.to_string()),
    };
    let project = match project(client) {
        Ok(project) => project,
        Err(why) => return failed(tx, &edit.media, why),
    };
    let set = match &edit.link {
        Some((from, output)) => Set::link(&edit.node, &edit.input, from, *output),
        None => Set::value(&edit.node, &edit.input, &edit.value),
    };
    // A copy just made is named by its step: for steps the document holds, by an edit
    // kerfd applies, and the workflow shown after is of the project as that left it.
    let named = workflow::set(
        &project,
        &shared.directory,
        &edit.media,
        edit.line,
        &edit.was,
        set,
        &models,
    )
    .and_then(|named| {
        crate::recipes::apply(client, &named.script).inspect_err(|_| {
            if let Some(copy) = &named.copy {
                let _ = std::fs::remove_file(copy);
            }
        })?;
        Ok(!named.script.is_empty())
    });
    let project = match named {
        Ok(true) => match self::project(client) {
            Ok(project) => project,
            Err(why) => return failed(tx, &edit.media, why),
        },
        Ok(false) => project,
        Err(why) => {
            failed(tx, &edit.media, why);
            project
        }
    };
    match event(&project, shared, &edit.media, edit.line, &models) {
        Ok(mut event) => {
            event["edited"] = json!(true);
            crate::emit(tx, event);
        }
        Err(why) => failed(tx, &edit.media, why),
    }
    crate::recipes::get(shared, client, &edit.media, tx);
}
