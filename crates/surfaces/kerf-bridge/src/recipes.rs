//! How a shot is made, for the editor's graph: a media's steps laid out with
//! what they read ([`kerf_engine::recipe::graph`]), and a person's change to
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

use crate::Shared;
use kerf_engine::recipe::model::Models;
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
    /// Go back to the steps this take was made from ([`kerf_shots::takes::restored`]).
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
) -> Result<Changed, String> {
    let there = kerf_shots::graph::written(project, &edit.media)?;
    // What the editor read is what the document has: steps changed since are left alone.
    if there != edit.was {
        return Err(format!(
            "the steps of `{}` were changed since they were read; they are left as that left them",
            edit.media
        ));
    }
    if let Some(take) = edit.restore {
        let restored = kerf_shots::takes::restored(project, directory, &edit.media, take)?;
        if restored.text == edit.was && restored.files.is_empty() {
            return Err(format!(
                "the steps are as take {take} was made from them already"
            ));
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
    Ok(Changed {
        added: edit
            .add
            .as_ref()
            .and_then(|_| kerf_shots::graph::added_at(&edit.was, &now)),
        script: kerf_shots::graph::step_lines(&edit.media, &edit.was, &now)?,
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

/// The `media_carry_plan` event of media `id`, whose steps are in the recipe file it names:
/// what carrying them into the document would write. Nothing is changed.
fn carry_plan(project: &Project, directory: &Path, id: &str) -> Result<Value, String> {
    let carry =
        kerf_engine::recipe::carry(project, directory, id).map_err(|error| error.to_string())?;
    Ok(json!({
        "type": "media_carry_plan",
        "media": id,
        "recipe": carry.recipe,
        "script": carry.script,
        "notes": carry.notes,
    }))
}

/// Have the takes of media `id`, carried between `before` and `after`, held to the steps it
/// now has: the `media_carried` event, with how many are.
fn carried(
    before: &Project,
    after: &Project,
    directory: &Path,
    id: &str,
    recipe: &str,
) -> Result<Value, String> {
    let held = kerf_engine::recipe::takes::mark_carried(before, after, directory, id)
        .map_err(|error| error.to_string())?;
    Ok(json!({"type": "media_carried", "media": id, "recipe": recipe, "held": held}))
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
    tx: &SyncSender<Value>,
) {
    let directory = shared.directory.as_path();
    let answered = project(client).and_then(|before| {
        if !confirmed {
            return carry_plan(&before, directory, id);
        }
        let carry = kerf_engine::recipe::carry(&before, directory, id)
            .map_err(|error| error.to_string())?;
        apply(client, &carry.script)?;
        let after = project(client)?;
        carried(&before, &after, directory, id, &carry.recipe)
    });
    match answered {
        Ok(event) => crate::emit(tx, event),
        Err(why) => crate::emit(
            tx,
            json!({"type": "error", "message": format!("cannot carry {id}: {why}"),
                "command": "media_carry", "media": id}),
        ),
    }
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

    /// The opening shot's steps, as the document writes them under its line.
    const STEP: &str = "generate model=dawn prompt=\"a boat leaves\" seed=7 strength=0.6";

    fn opening() -> Project {
        kerf_runtime::parse_document(&format!(
            "kerf 1\nproject name=\"Shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=96f video=320x180\n  {STEP}\nsequence s1 rate=24 size=320x180\n",
        ))
        .unwrap()
    }

    /// `project` with `script` applied, as kerfd would apply it.
    fn applied(project: &Project, script: &[String]) -> Project {
        kerf_lang::apply_script(project, kerf_model::Catalog::builtin(), &script.join("\n"))
            .unwrap()
            .project
    }

    fn set(key: &str, value: Option<&str>) -> Edit {
        Edit {
            media: "m1".into(),
            was: format!("kerf-recipe 1\n{STEP}\n"),
            text: None,
            line: 2,
            key: key.into(),
            nth: 0,
            value: value.map(str::to_owned),
            add: None,
            remove: false,
            restore: None,
        }
    }

    /// A step is taken out, its words taken off, the whole of the steps set, and the steps
    /// set back to a take, each as the lines of an edit.
    #[cfg(unix)]
    #[test]
    fn steps_are_changed_whole_and_set_back_to_a_take_by_an_edit() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let project = opening();
        let models = Models::default();
        let shown = event(&project, dir, "m1", &models, None).unwrap();
        assert_eq!(shown["type"], "recipe");
        let kinds: Vec<&str> = shown["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["model", "takes"]);
        // No model is set up, which is why it cannot be made, and is said.
        assert!(shown["cannot"]
            .as_str()
            .unwrap()
            .contains("no model `dawn` is set up"));

        // A word taken off the step: the step set without it.
        assert_eq!(
            change(&project, dir, &set("strength", None), &models)
                .unwrap()
                .script,
            ["step m1 set 1 generate model=dawn prompt=\"a boat leaves\" seed=7"]
        );
        // The whole of the steps set.
        let whole = Edit {
            text: Some(
                "kerf-recipe 1\ngenerate model=dawn prompt=\"a boat leaves\" seed=9\n".into(),
            ),
            ..set("", None)
        };
        assert_eq!(
            change(&project, dir, &whole, &models).unwrap(),
            Changed {
                added: None,
                script: vec![
                    "step m1 set 1 generate model=dawn prompt=\"a boat leaves\" seed=9".into()
                ],
            }
        );
        // A step added and taken out again, the step on its line by its number.
        let guide = Edit {
            add: Some("guide".into()),
            ..set("", None)
        };
        let two = applied(
            &project,
            &change(&project, dir, &guide, &models).unwrap().script,
        );
        let was = kerf_shots::graph::written(&two, "m1").unwrap();
        assert_eq!(
            was,
            format!("kerf-recipe 1\nrender s1 {{guide.mp4}} pass=depth\n{STEP}\n")
        );
        let out = Edit {
            remove: true,
            line: 2,
            was: was.clone(),
            ..set("", None)
        };
        let script = change(&two, dir, &out, &models).unwrap().script;
        assert_eq!(
            script,
            [
                "step m1 remove 2",
                "step m1 remove 1",
                &format!("step m1 add {STEP}")
            ]
        );
        assert_eq!(applied(&two, &script).media("m1"), project.media("m1"));
        // The last step makes the media's file, and is not taken out from under it.
        let last = Edit {
            remove: true,
            line: 3,
            was,
            ..set("", None)
        };
        let last = change(&two, dir, &last, &models).unwrap_err();
        assert!(last.contains("its steps do not read"), "{last}");

        // A take made, and the steps changed since: going back is the step as the take had it.
        let file = dir.join("models.json");
        fs::write(
            &file,
            r#"{"dawn": {"run": ["sh", "-c", "printf take > \"$1\"", "dawn", "{out}"]}}"#,
        )
        .unwrap();
        let making = Models::read(&file).unwrap();
        let plan = kerf_engine::recipe::plan(&project, dir, "m1", &making).unwrap();
        kerf_engine::recipe::make(
            &project,
            dir,
            &plan,
            &making,
            &kerf_engine::CancellationToken::new(),
            |_| {},
        )
        .unwrap();
        let back = Edit {
            restore: Some(1),
            ..set("", None)
        };
        let already = change(&project, dir, &back, &models).unwrap_err();
        assert!(
            already.contains("as take 1 was made from them already"),
            "{already}"
        );
        let seeded = applied(
            &project,
            &change(&project, dir, &set("seed", Some("8")), &models)
                .unwrap()
                .script,
        );
        let back = Edit {
            was: kerf_shots::graph::written(&seeded, "m1").unwrap(),
            ..back
        };
        assert_eq!(
            change(&seeded, dir, &back, &models).unwrap(),
            Changed {
                added: None,
                script: vec![format!("step m1 set 1 {STEP}")]
            }
        );

        // A models file that does not read is said with the graph, which is still shown.
        let shown = event(&project, dir, "m1", &models, Some("it is not JSON".into())).unwrap();
        assert_eq!(shown["modelsError"], "it is not JSON");
        assert!(event(&project, dir, "m9", &models, None)
            .unwrap_err()
            .contains("no media `m9`"));
    }

    /// A media whose steps are still in a recipe's file is not changed here: how its steps
    /// are carried into the document is said.
    #[test]
    fn a_media_whose_steps_are_in_a_recipe_file_is_refused_with_how_to_carry_it() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        fs::create_dir_all(dir.join("shots")).unwrap();
        fs::write(
            dir.join("shots/open.kerfrecipe"),
            format!("kerf-recipe 1\n{STEP}\n"),
        )
        .unwrap();
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=96f video=320x180 recipe=\"shots/open.kerfrecipe\"\nsequence s1 rate=24 size=320x180\n",
        )
        .unwrap();
        let refused =
            change(&project, dir, &set("seed", Some("8")), &Models::default()).unwrap_err();
        assert!(
            refused.contains("in the recipe file `shots/open.kerfrecipe`"),
            "{refused}"
        );
        assert!(refused.contains("`kerf carry m1`"), "{refused}");
        assert_eq!(
            fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(),
            format!("kerf-recipe 1\n{STEP}\n")
        );
    }

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

    /// The SHA-256 of `text`, as a take's record holds it.
    #[cfg(unix)]
    fn sha256(text: &str) -> String {
        use std::io::Write;
        let mut summing = std::process::Command::new("sha256sum")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("sha256sum");
        summing
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        let out = summing.wait_with_output().unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned()
    }

    /// Carrying says first what it would write, changing nothing; once the lines are in the
    /// document, the take the recipe made before is held to the steps it became.
    #[cfg(unix)]
    #[test]
    fn a_recipe_is_carried_into_the_document_and_its_take_is_held() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let recipe = format!("kerf-recipe 1\n# The boat.\n{STEP}\n");
        let before = uncarried(dir, &recipe);
        // The take a making of the recipe left, before a document held steps.
        let takes = dir.join("shots/open.takes");
        fs::create_dir_all(takes.join("001.work")).unwrap();
        fs::write(takes.join("001.mp4"), "take").unwrap();
        fs::write(takes.join("001.work/made-from.kerfrecipe"), &recipe).unwrap();
        let record = json!({
            "kerf-take": 1,
            "made": 1_700_000_000u64,
            "from": {"recipe": sha256(&kerf_engine::recipe::Recipe::essence(&recipe))},
        });
        fs::write(takes.join("001.take.json"), record.to_string()).unwrap();

        let plan = carry_plan(&before, dir, "m1").unwrap();
        assert_eq!(plan["type"], "media_carry_plan");
        assert_eq!(
            (plan["media"].as_str(), plan["recipe"].as_str()),
            (Some("m1"), Some("shots/open.kerfrecipe"))
        );
        let script: Vec<String> = serde_json::from_value(plan["script"].clone()).unwrap();
        assert_eq!(script.last().unwrap(), &format!("step m1 add {STEP}"));
        assert!(!script[0].contains("recipe="), "{script:?}");
        assert_eq!(plan["notes"], json!([]));

        let after = applied(&before, &script);
        assert_eq!(after.media("m1").unwrap().steps.len(), 1);
        let done = carried(&before, &after, dir, "m1", "shots/open.kerfrecipe").unwrap();
        assert_eq!(
            done,
            json!({"type": "media_carried", "media": "m1", "recipe": "shots/open.kerfrecipe", "held": 1})
        );
        let held =
            kerf_engine::recipe::takes::takes(&after, dir, after.media("m1").unwrap()).unwrap();
        assert_eq!(held.len(), 1);
        assert!(held[0].current, "{held:?}");
        // The recipe's file is left where it is, and once carried there is nothing to carry.
        assert_eq!(
            fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(),
            recipe
        );
        assert!(carry_plan(&after, dir, "m1")
            .unwrap_err()
            .contains("has its steps under its line"));
    }

    /// A recipe that runs a program is not carried: a document names none.
    #[test]
    fn a_recipe_that_runs_a_program_is_not_carried() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let project = uncarried(dir, "kerf-recipe 1\nrun vork render --out {guide.mp4}\ngenerate model=dawn guide={guide.mp4}\n");
        let refused = carry_plan(&project, dir, "m1").unwrap_err();
        let engine = kerf_engine::recipe::carry(&project, dir, "m1")
            .unwrap_err()
            .to_string();
        assert_eq!(refused, engine);
        assert!(
            refused.contains("runs a program, and a document names none"),
            "{refused}"
        );
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
        assert_eq!(
            (shown["recipe"].as_str(), shown["name"].as_str()),
            (Some(""), Some("m1"))
        );
        let was = shown["text"].as_str().unwrap().to_owned();
        assert_eq!(
            was,
            "kerf-recipe 1\ngenerate model=dawn prompt=\"a boat leaves\" seed=7 strength=0.6\n"
        );
        assert_eq!(shown["nodes"][0]["line"], 2);
        let on = |line: usize| Edit {
            was: was.clone(),
            line,
            ..set("", None)
        };

        // One name on one step is that step set, by its number.
        let seeded = Edit {
            key: "seed".into(),
            value: Some("8".into()),
            ..on(2)
        };
        assert_eq!(
            change(&project, dir, &seeded, &models).unwrap(),
            Changed {
                added: None,
                script: vec!["step m1 set 1 generate model=dawn prompt=\"a boat leaves\" seed=8 strength=0.6".into()],
            }
        );
        // A step added, by its kind: every step written again, and the line it is on said.
        let guide = Edit {
            add: Some("guide".into()),
            ..on(0)
        };
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
        let edited = kerf_lang::apply_script(
            &project,
            kerf_model::Catalog::builtin(),
            &added.script.join("\n"),
        )
        .unwrap()
        .project;
        assert_eq!(edited.media("m1").unwrap().steps.len(), 2);
        // What a step cannot take is refused as a making would refuse it, and a program to
        // run has no place in the document.
        let wrong = Edit {
            key: "seed".into(),
            value: Some("soon".into()),
            ..on(2)
        };
        assert!(change(&project, dir, &wrong, &models)
            .unwrap_err()
            .contains("`seed=soon` is not a whole number"));
        let ran = Edit {
            text: Some("kerf-recipe 1\nrun vork render --out {out}\n".into()),
            ..on(0)
        };
        let ran = change(&project, dir, &ran, &models).unwrap_err();
        assert!(
            ran.contains("runs a program, and a document names none"),
            "{ran}"
        );
        // Steps changed since the editor read them are left as that left them.
        let stale = Edit {
            was: "kerf-recipe 1\ngenerate model=dawn\n".into(),
            ..seeded
        };
        assert!(change(&project, dir, &stale, &models)
            .unwrap_err()
            .contains("were changed since they were read"));
        // Nothing was written: the document is where the steps are.
        assert!(!dir.join("shots").exists());
    }
}
