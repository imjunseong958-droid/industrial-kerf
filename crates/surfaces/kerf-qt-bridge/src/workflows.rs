//! The workflow behind a model, for the editor's graph: a `generate` step
//! whose model a person's entry runs with `kerf-comfy` can be opened as the
//! ComfyUI graph it runs, every node and input of it, and a value in it set.
//!
//! Setting one changes nothing on a ComfyUI server and nothing of
//! `kerf-comfy`. The workflow as it comes is copied into the project, beside
//! the recipe, with that value changed and the rest as it was written
//! ([`kerf_workflow::detail`]), and the step's line is given `workflow=<file>`,
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
    value: Value,
}

/// The `workflow` event of the step on `line` of media `id`'s recipe.
fn event(project: &Project, shared: &Shared, id: &str, line: usize, models: &Models) -> Result<Value, String> {
    let read = workflow::read(project, &shared.directory, id, line, models)?;
    let mut event = workflow::shown(id, line, &read)?;
    event["type"] = json!("workflow");
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
        Ok(event) => crate::emit(tx, event),
        Err(why) => failed(tx, id, why),
    }
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
    let set = Set {
        node: &edit.node,
        input: &edit.input,
        value: &edit.value,
    };
    if let Err(why) = workflow::set(&project, &shared.directory, &edit.media, edit.line, &edit.was, set, &models) {
        failed(tx, &edit.media, why);
    }
    match event(&project, shared, &edit.media, edit.line, &models) {
        Ok(mut event) => {
            event["edited"] = json!(true);
            crate::emit(tx, event);
        }
        Err(why) => failed(tx, &edit.media, why),
    }
    crate::recipes::get(shared, client, &edit.media, tx);
}
