//! How a shot is made, for the editor's graph: a media's steps laid out with
//! what they read ([`kerf_engine::recipe::graph()`]), and a person's change to
//! them.
//!
//! The steps are the document's own, under the media, and are changed by an
//! edit as any of the document is: the change becomes `step` lines that kerfd
//! applies, one revision that undo takes back. The editor sends the steps'
//! text as it read them ([`kerf_engine::recipe::steps_text`]) and the line to
//! change, and steps changed since it read them are left as that left them.
//!
//! Nothing here runs a step. A making is `media_make`, which shows what
//! would run and asks first.

use crate::error::BridgeError;
use crate::event::{ErrorEvent, Event, Recipe};
use crate::wake::Outbox;
use crate::CommandName;
use crate::Shared;
use kerf_engine::recipe::model::Models;
use kerf_engine::EngineError;
use kerf_model::Project;
use kerf_runtime::RuntimeError;
use kerf_shots::ShotError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A change to a media's recipe: one name on one line, or with `text` the
/// whole of it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Edit {
    pub media: String,
    /// The recipe as the editor read it. A file that holds anything else
    /// was changed since, and is left alone.
    pub was: String,
    /// The whole recipe, in place of `was`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The line, from 1, and the name on it to set: the `nth` word of that
    /// name, to `value`, or taken off the line where there is no value.
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub nth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// A step to add, by its kind: `guide`, `model`, or `join`
    /// ([`kerf_shots::graph::step`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add: Option<String>,
    /// Take the step on `line` out.
    #[serde(default)]
    pub remove: bool,
    /// Go back to the steps this take was made from ([`kerf_shots::takes::restored`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<u32>,
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
) -> Result<Recipe, ShotError> {
    let graph = kerf_shots::graph::shown(project, directory, id, models)?;
    Ok(Recipe {
        graph,
        models_error,
        edited: false,
        added: None,
    })
}

/// What a change to a media's steps comes to.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Changed {
    /// The line a step was added on, where one was.
    added: Option<usize>,
    /// The lines of the edit that makes it.
    script: Vec<String>,
}

/// What `edit` makes of its media's steps. Nothing is changed here: what would change
/// them is given back as the lines of an edit, for kerfd to apply.
fn change(
    project: &Project,
    directory: &Path,
    edit: &Edit,
    models: &Models,
) -> Result<Changed, ShotError> {
    let there = kerf_shots::graph::written(project, &edit.media)?;
    // What the editor read is what the document has: steps changed since are left alone.
    if there != edit.was {
        return Err(format!(
            "the steps of `{}` were changed since they were read; they are left as that left them",
            edit.media
        )
        .into());
    }
    if let Some(take) = edit.restore {
        let restored = kerf_shots::takes::restored(project, directory, &edit.media, take)?;
        if restored == edit.was {
            return Err(format!("the steps are as take {take} was made from them already").into());
        }
        return Ok(Changed {
            added: None,
            script: kerf_shots::graph::step_lines(&edit.media, &edit.was, &restored)?,
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
    Ok(Changed {
        added: edit
            .add
            .as_ref()
            .and_then(|_| kerf_shots::graph::added_at(&edit.was, &now)),
        script: kerf_shots::graph::step_lines(&edit.media, &edit.was, &now)?,
    })
}

/// Have kerfd run `lines`, an edit of the person's own: one revision, which undo takes back.
pub(crate) fn apply(
    client: &kerf_runtime::RuntimeClient,
    lines: &[String],
) -> Result<(), RuntimeError> {
    if lines.is_empty() {
        return Ok(());
    }
    let revision = client.get_state()?.revision;
    client
        .apply_script_request("ui", &crate::key(), &lines.join("\n"), revision)
        .map(|_| ())
}

/// The `media_carry_plan` event of media `id`, whose steps are in the recipe file it names:
/// what carrying them into the document would write. Nothing is changed.
fn carry_plan(project: &Project, directory: &Path, id: &str) -> Result<Event, EngineError> {
    let carry = kerf_engine::recipe::carry(project, directory, id)?;
    Ok(Event::MediaCarryPlan {
        media: id.to_owned(),
        recipe: carry.recipe,
        script: carry.script,
        notes: carry.notes,
    })
}

/// Have the takes of media `id`, carried between `before` and `after`, held to the steps it
/// now has: the `media_carried` event, with how many are.
fn carried(
    before: &Project,
    after: &Project,
    directory: &Path,
    id: &str,
    recipe: &str,
) -> Result<Event, EngineError> {
    let held = kerf_engine::recipe::takes::mark_carried(before, after, directory, id)?;
    Ok(Event::MediaCarried {
        media: id.to_owned(),
        recipe: recipe.to_owned(),
        held,
    })
}

/// Answer `media_carry` for media `id`, whose steps are in the recipe file it names.
/// Unconfirmed, say what would be written (`media_carry_plan`) and change nothing.
/// Confirmed, which only a person's yes to that plan sends, have kerfd apply the lines as
/// one edit of the person's own, which undo takes back, then hold the takes made from the
/// recipe to the steps (`media_carried`). The recipe's file is left where it is. What
/// cannot be carried is an `error` that names the media, with the document as it was.
pub(crate) fn carry(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
    id: &str,
    confirmed: bool,
    tx: &Outbox,
) {
    let directory = shared.directory.as_path();
    let answered = project(client)
        .map_err(BridgeError::from)
        .and_then(|before| {
            if !confirmed {
                return Ok(carry_plan(&before, directory, id)?);
            }
            let carry = kerf_engine::recipe::carry(&before, directory, id)?;
            apply(client, &carry.script)?;
            let after = project(client)?;
            Ok(carried(&before, &after, directory, id, &carry.recipe)?)
        });
    match answered {
        Ok(event) => crate::emit(tx, event),
        Err(why) => crate::emit(
            tx,
            Event::Error(ErrorEvent {
                message: format!("cannot carry {id}: {why}"),
                why: Some(why.to_string()),
                code: why.code(),
                command: Some(CommandName::MediaCarry),
                media: Some(id.to_owned()),
                ..ErrorEvent::default()
            }),
        ),
    }
}

/// The project as kerfd has it.
fn project(client: &kerf_runtime::RuntimeClient) -> Result<Project, RuntimeError> {
    client
        .get_document(false)
        .and_then(|(_, text)| kerf_runtime::parse_document(&text))
}

/// What cannot be shown or changed of media `id`'s recipe: an `error` that
/// names the media, so that what shows its graph can say so there.
fn failed(tx: &Outbox, id: &str, why: impl Into<BridgeError>) {
    let why = why.into();
    crate::emit(
        tx,
        Event::Error(ErrorEvent {
            message: why.to_string(),
            code: why.code(),
            command: Some(CommandName::Recipe),
            media: Some(id.to_owned()),
            ..ErrorEvent::default()
        }),
    );
}

/// Answer `recipe` for media `id`: how it is made, as a graph.
pub(crate) fn get(shared: &Shared, client: &kerf_runtime::RuntimeClient, id: &str, tx: &Outbox) {
    let (models, models_error) = models();
    match project(client)
        .map_err(BridgeError::from)
        .and_then(|project| {
            Ok(event(
                &project,
                &shared.directory,
                id,
                &models,
                models_error,
            )?)
        }) {
        Ok(event) => crate::emit(tx, Event::Recipe(event)),
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
    tx: &Outbox,
) {
    let project = match project(client) {
        Ok(project) => project,
        Err(why) => return failed(tx, &edit.media, why),
    };
    let (models, models_error) = models();
    // Steps the document holds change by an edit, which kerfd applies: the graph shown
    // after is of the project as that left it.
    let changed = change(&project, &shared.directory, edit, &models)
        .map_err(BridgeError::from)
        .and_then(|changed| Ok(apply(client, &changed.script).map(|()| changed)?));
    let project = match &changed {
        Ok(changed) if !changed.script.is_empty() => match self::project(client) {
            Ok(project) => project,
            Err(why) => return failed(tx, &edit.media, why),
        },
        _ => project,
    };
    let shown = event(
        &project,
        &shared.directory,
        &edit.media,
        &models,
        models_error,
    );
    let added = match changed {
        Ok(changed) => changed.added,
        Err(why) => {
            failed(tx, &edit.media, why);
            None
        }
    };
    match shown {
        Ok(mut event) => {
            event.edited = true;
            // The line of the step that was added, for what shows the graph to choose it.
            event.added = added;
            crate::emit(tx, Event::Recipe(event));
        }
        Err(why) => failed(tx, &edit.media, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A project whose media `m1` names the recipe file `shots/open.kerfrecipe`, with
    /// `recipe` written there.
    fn uncarried(dir: &Path, recipe: &str) -> Project {
        fs::create_dir_all(dir.join("shots")).unwrap();
        fs::write(dir.join("shots/open.kerfrecipe"), recipe).unwrap();
        kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=96f video=320x180 recipe=\"shots/open.kerfrecipe\"\nsequence s1 rate=24 size=320x180\n",
        )
        .unwrap()
    }

    /// A recipe that runs a program is not carried: a document names none.
    #[test]
    fn a_recipe_that_runs_a_program_is_not_carried() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let project = uncarried(dir, "kerf-recipe 1\nrun vork render --out {guide.mp4}\ngenerate model=dawn guide={guide.mp4}\n");
        let refused = carry_plan(&project, dir, "m1").unwrap_err().to_string();
        let engine = kerf_engine::recipe::carry(&project, dir, "m1")
            .unwrap_err()
            .to_string();
        assert_eq!(refused, engine);
        assert!(
            refused.contains("runs a program, and a document names none"),
            "{refused}"
        );
    }
}
