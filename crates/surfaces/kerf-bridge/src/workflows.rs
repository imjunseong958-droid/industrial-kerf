//! The workflow behind a model, for the editor's graph: a `generate` step
//! whose model a person's entry runs with `kerf-comfy` can be opened as the
//! ComfyUI graph it runs, every node and input of it, and a value in it set.
//!
//! Setting one changes nothing on a ComfyUI server and nothing of
//! `kerf-comfy`. The workflow as it comes is copied into the project, beside
//! the shot, with that value changed and the rest as it was written
//! (`kerf_workflow::detail`), and the step is given `workflow=<file>`, by an
//! edit where the document holds it and in its recipe's text where a file does,
//! which has the model's program run the copy for this shot
//! The copy is a file of the project: it travels with it,
//! a take is held to it, and taking the word off the line goes back to the
//! workflow as it comes.
//!
//! A model run by anything else has no workflow to show, and its node says
//! nothing of one. Which program runs a model is read from the person's own
//! models file and from nothing in the project.
//!
//! The reading and the rule are `kerf-shots`', which an agent's tools go
//! through too: this is the editor's end of them, its commands and events.
//!
//! What the server takes at each input is asked off the worker, and may come
//! back after an edit has said the workflow again. It is then said of the
//! workflow as last said ([`Said::late`]), never of the text it was asked
//! of, so a late answer cannot bring back what the edit changed.

use crate::event::{ErrorEvent, Event};
use crate::wake::Outbox;
use crate::Shared;
use kerf_engine::recipe::model::Models;
use kerf_model::Project;
use kerf_shots::workflow::{self, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// The `workflow` event last said of each step, by its media and its line: what the editor
/// shows of it.
#[derive(Clone, Default)]
pub(crate) struct Said(Arc<Mutex<HashMap<(String, usize), Value>>>);

/// The step a `workflow` event is of.
fn step_of(event: &Value) -> (String, usize) {
    (
        event["media"].as_str().unwrap_or_default().to_owned(),
        event["line"].as_u64().unwrap_or_default() as usize,
    )
}

impl Said {
    /// Say `event`, a `workflow` event, and keep it as what the editor shows of its step.
    /// Kept and sent under one lock, so what is kept is what was sent last.
    pub(crate) fn say(&self, tx: &Outbox, event: Value) {
        let mut said = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        said.insert(step_of(&event), event.clone());
        crate::emit(tx, workflow_event(event));
    }

    /// Say what the server takes (`asked`), asked of the workflow as `own` says it, once it
    /// is known. An edit may have said the workflow again meanwhile: it is said of the
    /// workflow as last said, whose text is the newer, and not of the text it was asked of.
    /// Nothing is said when what was last said has it already, or is of another model.
    pub(crate) fn late(&self, tx: &Outbox, own: Value, asked: Result<Value, String>) {
        let mut said = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let step = step_of(&own);
        let mut event = match said.get(&step) {
            Some(last) if last.get("taken").is_some() => return,
            Some(last) if last["model"] != own["model"] || last["name"] != own["name"] => return,
            Some(last) => {
                let mut last = last.clone();
                if let Some(fields) = last.as_object_mut() {
                    fields.remove("edited");
                }
                last
            }
            None => own,
        };
        match asked {
            Ok(taken) => workflow::with_taken(&mut event, &taken),
            Err(why) => event["takenError"] = json!(why),
        }
        said.insert(step, event.clone());
        crate::emit(tx, workflow_event(event));
    }
}

/// The `workflow` event saying `event`, a workflow as `workflow::shown` gives it.
fn workflow_event(event: Value) -> Event {
    let mut fields = match event {
        Value::Object(fields) => fields,
        _ => Map::new(),
    };
    // Its type is the event's own.
    fields.remove("type");
    Event::Workflow(fields)
}

/// A value of a workflow to set: input `input` of node `node`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Edit {
    pub media: String,
    /// The `generate` line the workflow is of, from 1.
    pub line: usize,
    /// The workflow's text as the editor read it.
    pub was: String,
    pub node: String,
    pub input: String,
    /// What to set it to; or, with `link`, the node and the output of it to read.
    #[serde(default)]
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<(String, u64)>,
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

fn failed(tx: &Outbox, id: &str, why: String) {
    crate::emit(
        tx,
        Event::Error(ErrorEvent {
            message: why,
            command: Some("workflow".to_owned()),
            media: Some(id.to_owned()),
            ..ErrorEvent::default()
        }),
    );
}

/// Answer `workflow` for the `generate` step on `line` of media `id`'s recipe.
pub(crate) fn get(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    id: &str,
    line: usize,
    tx: &Outbox,
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
            shared.workflows.say(tx, event);
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
/// with why (`takenError`), since it is still seen and set without. What it says goes on
/// the workflow as the editor shows it by then ([`Said::late`]).
fn ask_taken(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    id: &str,
    line: usize,
    tx: &Outbox,
) {
    let Ok(project) = project(client) else {
        return;
    };
    let (directory, id, tx) = (shared.directory.clone(), id.to_owned(), tx.clone());
    let said = shared.workflows.clone();
    std::thread::spawn(move || {
        let Ok(models) = Models::load() else {
            return;
        };
        let Ok(read) = workflow::read(&project, &directory, &id, line, &models) else {
            return;
        };
        let asked = workflow::taken(&models, &read);
        let Ok(event) = workflow::shown(&id, line, &read) else {
            return;
        };
        said.late(&tx, event, asked);
    });
}

/// Make `edit` and answer `workflow` as it now is, then `recipe`, whose
/// line may have gained its `workflow=`. A change that is refused is an
/// `error` with why, followed by the workflow as it is.
pub(crate) fn edit(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    edit: &Edit,
    tx: &Outbox,
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
                // Only tidying: no step names the copy, as the edit that would have was refused.
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
            shared.workflows.say(tx, event);
        }
        Err(why) => failed(tx, &edit.media, why),
    }
    crate::recipes::get(shared, client, &edit.media, tx);
}

#[cfg(test)]
mod tests {
    use super::Said;
    use crate::event::Event;
    use serde_json::{json, Value};
    use std::sync::mpsc::Receiver;

    fn shown(text: &str) -> Value {
        json!({"type": "workflow", "media": "m1", "line": 2, "model": "flux", "name": "flux",
               "text": text, "nodes": [{"id": "3", "class": "KSampler",
               "inputs": [{"name": "steps", "value": 20}]}]})
    }

    fn taken() -> Value {
        json!({"KSampler": {"steps": {"min": 1, "max": 150, "step": 1}}})
    }

    fn all(rx: &Receiver<Event>) -> Vec<Value> {
        rx.try_iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect()
    }

    #[test]
    fn what_the_server_takes_comes_late_on_the_workflow_an_edit_said_meanwhile() {
        let (tx, rx) = crate::wake::outbox(16);
        let said = Said::default();
        // Opened, and asked what the server takes; then a value is set before it answers.
        said.say(&tx, shown("old"));
        let mut edited = shown("new");
        edited["edited"] = json!(true);
        said.say(&tx, edited);
        // The answer read before the edit comes last: on the newer text, not the old.
        said.late(&tx, shown("old"), Ok(taken()));
        let events = all(&rx);
        assert_eq!(events.len(), 3);
        let last = &events[2];
        assert_eq!(last["text"], "new");
        assert_eq!(last["taken"], true);
        assert_eq!(last["nodes"][0]["inputs"][0]["max"], 150);
        assert!(last.get("edited").is_none());
        // What was last said has it now: an answer after that says nothing again.
        said.late(&tx, shown("old"), Ok(taken()));
        assert!(all(&rx).is_empty());
    }
}
