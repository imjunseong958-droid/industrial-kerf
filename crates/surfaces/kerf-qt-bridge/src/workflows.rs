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

use crate::Shared;
use kerf_engine::recipe::model::{Ask, Models, Source};
use kerf_engine::recipe::text::{self, Change};
use kerf_engine::recipe::{Recipe, Step};
use kerf_model::Project;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;

/// The program a workflow is run by, by its file's name.
const PROGRAM: &str = "kerf-comfy";

/// What says which workflow that program runs, in a model's entry.
const FLAG: &str = "--workflow";

/// The workflow a model's `command` runs, where the command is `kerf-comfy`'s:
/// the word after `--workflow`, a name of one that comes with the program or
/// a file. None for a model any other program runs.
pub(crate) fn runs(command: &[String]) -> Option<&str> {
    // The program's own name, whichever way the path to it is written: a
    // models file written on Windows is read the same on any machine.
    let program = command.first()?.rsplit(['/', '\\']).next()?;
    let program = program
        .len()
        .checked_sub(4)
        .filter(|at| program.is_char_boundary(*at) && program[*at..].eq_ignore_ascii_case(".exe"))
        .map_or(program, |at| &program[..at]);
    if !program.eq_ignore_ascii_case(PROGRAM) {
        return None;
    }
    let flag = command.iter().position(|word| word == FLAG)?;
    command.get(flag + 1).map(String::as_str)
}

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

/// Where a step's workflow comes from.
enum Origin {
    /// It comes with `kerf-comfy`, by its name there.
    Bundled,
    /// A file the person's models entry names, outside the project.
    File,
    /// The shot's own, a file of the project its line names: its path from
    /// the project's folder, and where it is.
    Own(String, PathBuf),
}

/// The workflow of one `generate` step, read.
struct Read {
    ask: Ask,
    /// What the model's entry runs, as it names it.
    name: String,
    origin: Origin,
    text: String,
    /// The recipe's text, which a first change adds a word to.
    recipe: String,
}

/// The `generate` step on line `line` of the recipe media `id` names, and
/// the workflow it is made with.
fn read(
    project: &Project,
    directory: &Path,
    id: &str,
    line: usize,
    models: &Models,
) -> Result<Read, String> {
    let wrong = |error: kerf_engine::EngineError| error.to_string();
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
    let (file, _) = kerf_engine::recipe::beside(project, directory, id, &own).map_err(wrong)?;
    let recipe = std::fs::read_to_string(&file)
        .map_err(|error| format!("cannot read the recipe {}: {error}", file.display()))?;
    let steps = Recipe::parse(&recipe).map_err(wrong)?.steps;
    let ask = text::step_lines(&recipe)
        .into_iter()
        .zip(steps)
        .find_map(|(at, step)| match step {
            Step::Generate(ask) if at == line => Some(*ask),
            _ => None,
        })
        .ok_or_else(|| format!("line {line} of the recipe asks no model"))?;
    let command = models.command(&ask.model).map_err(wrong)?;
    let named = runs(command).ok_or_else(|| {
        format!(
            "model `{}` is not run by {PROGRAM}, so it has no workflow to show",
            ask.model
        )
    })?;
    let (origin, text) = match &ask.workflow {
        Some(Source::File(own)) => {
            let (file, stored) = kerf_engine::recipe::beside(project, directory, id, own).map_err(wrong)?;
            let text = std::fs::read_to_string(&file)
                .map_err(|error| format!("cannot read the workflow {stored}: {error}"))?;
            (Origin::Own(stored, file), text)
        }
        Some(Source::Made(made)) => {
            return Err(format!(
                "the line's workflow is `{{{made}}}`, a file a step makes: it is there only while the shot is made"
            ))
        }
        None if Path::new(named).is_file() => (
            Origin::File,
            std::fs::read_to_string(named)
                .map_err(|error| format!("cannot read the workflow {named}: {error}"))?,
        ),
        None => {
            let (_, text) = kerf_workflow::workflow::BUNDLED
                .iter()
                .find(|(bundled, _)| *bundled == named)
                .ok_or_else(|| {
                    format!(
                        "model `{}` runs the workflow `{named}`, which is neither a file nor one of those that come with {PROGRAM}",
                        ask.model
                    )
                })?;
            (Origin::Bundled, (*text).to_owned())
        }
    };
    Ok(Read {
        name: named.to_owned(),
        ask,
        origin,
        text,
        recipe,
    })
}

/// What the line gives for the blank `name`, as it writes it: none where it gives nothing.
fn given(ask: &Ask, name: &str) -> Option<String> {
    let shown = |source: &Source| match source {
        Source::File(file) => file.clone(),
        Source::Made(made) => format!("{{{made}}}"),
    };
    // Which of several a name is, as kerf-comfy counts them: `ref` the first, `ref2` the second.
    let counted = |stem: &str, name: &str| match name.strip_prefix(stem)? {
        "" => Some(0),
        number => number
            .parse::<usize>()
            .ok()
            .filter(|number| *number >= 2)
            .map(|number| number - 1),
    };
    match name {
        "prompt" => Some(ask.prompt.clone()),
        "seed" => ask.seed.map(|seed| seed.to_string()),
        "width" | "height" => ask.size.map(|(width, height)| format!("{width}x{height}")),
        "frames" => ask.seconds.map(|seconds| format!("{seconds} s")),
        "fps" | "no_picture" => None,
        "guide" => ask.guide.as_ref().map(shown),
        "first" => ask.first.as_ref().map(shown),
        "last" => ask.last.as_ref().map(shown),
        "moves" | "moves_video" | "moves_box" => {
            (!ask.moves.is_empty()).then(|| ask.moves.join(", "))
        }
        _ => {
            if let Some(of) = name.strip_prefix("guide_") {
                return ask
                    .guides
                    .iter()
                    .find(|(named, _)| named == of)
                    .map(|(_, source)| shown(source));
            }
            if let Some(at) = counted("ref", name) {
                return ask.references.get(at).map(shown);
            }
            if let Some(at) = counted("signal", name.strip_suffix("_video").unwrap_or(name)) {
                return ask.signals.get(at).cloned();
            }
            ask.more.get(name).cloned()
        }
    }
}

/// The `workflow` event of one step: its workflow as an editor shows it.
fn event(id: &str, line: usize, read: &Read) -> Result<Value, String> {
    let mut event = kerf_workflow::detail::described(&read.text)?;
    // Beside what fills each blank, what the line gives for it now.
    if let Some(blanks) = event["blanks"].as_object_mut() {
        for (name, blank) in blanks {
            blank["given"] = json!(given(&read.ask, name));
        }
    }
    event["type"] = json!("workflow");
    event["media"] = json!(id);
    event["line"] = json!(line);
    event["model"] = json!(read.ask.model);
    event["name"] = json!(read.name);
    event["text"] = json!(read.text);
    let (origin, file) = match &read.origin {
        Origin::Bundled => ("bundled", None),
        Origin::File => ("file", None),
        Origin::Own(stored, _) => ("own", Some(stored.clone())),
    };
    event["origin"] = json!(origin);
    event["file"] = json!(file);
    Ok(event)
}

/// The name of the copy a shot keeps of a workflow, beside its recipe: the
/// shot's and the model's, `bounce.video.comfy.json`, and the next free one
/// where a file of that name is there already.
fn copy_name(folder: &Path, shot: &str, model: &str) -> String {
    let clean: String = model
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() || "-_".contains(character) { character } else { '-' })
        .collect();
    (1..)
        .map(|number| match number {
            1 => format!("{shot}.{clean}.comfy.json"),
            more => format!("{shot}.{clean}-{more}.comfy.json"),
        })
        .find(|name| !folder.join(name).exists())
        .expect("a number no file has")
}

/// Write `text` to `file` whole: beside it first, then moved into place.
fn write(file: &Path, text: &str) -> Result<(), String> {
    let beside = file.with_extension(format!("{}.writing", std::process::id()));
    std::fs::write(&beside, text)
        .and_then(|()| std::fs::rename(&beside, file))
        .map_err(|error| {
            let _ = std::fs::remove_file(&beside);
            format!("cannot write the workflow {}: {error}", file.display())
        })
}

/// Make `edit`: the shot's own copy of the workflow with that value set,
/// made beside the recipe where the shot had none, and the line told to
/// run it.
fn change(project: &Project, directory: &Path, edit: &Edit, models: &Models) -> Result<(), String> {
    let read = read(project, directory, &edit.media, edit.line, models)?;
    if read.text != edit.was {
        return Err("the workflow was changed by another program since it was read; it is left as that left it".to_owned());
    }
    let now = kerf_workflow::detail::set_input(&read.text, &edit.node, &edit.input, &edit.value)?;
    if let Origin::Own(_, file) = &read.origin {
        return write(file, &now);
    }
    // The first change: the workflow as it comes is copied into the project with it.
    let media = project.media(&edit.media).expect("read found it");
    let recipe = media.recipe.as_deref().expect("read found it");
    let shot = kerf_engine::recipe::shot_name(recipe);
    // Beside the recipe: in its folder, by a name no file there has.
    let beside = |name: &str| {
        kerf_engine::recipe::beside(project, directory, &edit.media, name)
            .map(|(file, _)| file)
            .map_err(|error| error.to_string())
    };
    let folder = beside(".")?;
    let name = copy_name(&folder, &shot, &read.ask.model);
    let file = beside(&name)?;
    write(&file, &now)?;
    let line = text::changed(
        &read.recipe,
        edit.line,
        Change::Set {
            key: kerf_engine::recipe::model::WORKFLOW,
            nth: 0,
            value: &name,
        },
    )
    .and_then(|recipe| text::write(project, directory, &edit.media, &read.recipe, &recipe));
    line.map_err(|error| {
        // The line does not name it, so nothing would run it: it is not left lying there.
        let _ = std::fs::remove_file(&file);
        error.to_string()
    })
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
            let read = read(&project, &shared.directory, id, line, &models)?;
            event(id, line, &read)
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
    if let Err(why) = change(&project, &shared.directory, edit, &models) {
        failed(tx, &edit.media, why);
    }
    match read(&project, &shared.directory, &edit.media, edit.line, &models)
        .and_then(|read| event(&edit.media, edit.line, &read))
    {
        Ok(mut event) => {
            event["edited"] = json!(true);
            crate::emit(tx, event);
        }
        Err(why) => failed(tx, &edit.media, why),
    }
    crate::recipes::get(shared, client, &edit.media, tx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const RECIPE: &str = "kerf-recipe 1\n# The court, as depth.\nrender s1 {depth.mp4} pass=depth\ngenerate model=video prompt=\"a ball bounces\" guide={depth.mp4} seed=7 steps=24\n";

    fn court(directory: &Path) -> (Project, Models) {
        fs::create_dir_all(directory.join("shots")).unwrap();
        fs::write(directory.join("shots/bounce.kerfrecipe"), RECIPE).unwrap();
        let models = directory.join("models.json");
        fs::write(
            &models,
            r#"{"video": {"run": ["/opt/kerf/kerf-comfy", "--workflow", "shot", "{request}"]},
                "other": {"run": ["dawn-adapter", "--workflow", "shot", "{request}"]}}"#,
        )
        .unwrap();
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Court\"\nmedia m1 path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480 recipe=\"shots/bounce.kerfrecipe\"\nsequence s1 rate=24 size=320x180\n",
        )
        .unwrap();
        (project, Models::read(&models).unwrap())
    }

    #[test]
    fn a_models_program_says_which_workflow_it_runs() {
        let words = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
        assert_eq!(runs(&words(&["/opt/kerf/kerf-comfy", "--workflow", "shot", "{request}"])), Some("shot"));
        assert_eq!(runs(&words(&["C:\\kerf\\Kerf-Comfy.exe", "--workflow", "D:\\own.json", "{request}"])), Some("D:\\own.json"));
        // Another program, whatever it is told, is not one whose workflows are read here.
        assert_eq!(runs(&words(&["dawn-adapter", "--workflow", "shot", "{request}"])), None);
        assert_eq!(runs(&words(&["kerf-comfy", "{request}"])), None);
        assert_eq!(runs(&[]), None);
    }

    #[test]
    fn a_steps_workflow_is_shown_and_a_value_set_in_a_copy_of_the_projects_own() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let (project, models) = court(dir);
        let shown = event("m1", 4, &read(&project, dir, "m1", 4, &models).unwrap()).unwrap();
        assert_eq!(shown["type"], "workflow");
        assert_eq!((shown["name"].as_str(), shown["origin"].as_str()), (Some("shot"), Some("bundled")));
        assert_eq!(shown["file"], Value::Null);
        assert_eq!(shown["model"], "video");
        // What the line gives for a blank is said beside what fills it.
        assert_eq!(shown["blanks"]["prompt"]["given"], "a ball bounces");
        assert_eq!(shown["blanks"]["seed"]["given"], "7");
        assert_eq!(shown["blanks"]["guide"]["given"], "{depth.mp4}");
        assert_eq!(shown["blanks"]["steps"]["given"], "24");
        assert_eq!(shown["blanks"]["steps"]["default"], 20);
        assert_eq!(shown["blanks"]["ref"]["given"], Value::Null);
        // A node with a value the workflow itself sets: the loader's `weight_dtype`.
        let (node, input, was) = shown["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|node| {
                let input = node["inputs"].as_array()?.iter().find(|input| input["name"] == "weight_dtype")?;
                Some((node["id"].as_str()?.to_owned(), "weight_dtype".to_owned(), input["value"].clone()))
            })
            .expect("a loader");
        assert_eq!(was, "default");

        // Set, the workflow is copied into the project beside the recipe, with that value
        // alone changed, and the line is told to run it.
        let set = |was: &str, value: Value| Edit {
            media: "m1".into(),
            line: 4,
            was: was.to_owned(),
            node: node.clone(),
            input: input.clone(),
            value,
        };
        let bundled = shown["text"].as_str().unwrap().to_owned();
        change(&project, dir, &set(&bundled, json!("fp8_e4m3fn")), &models).unwrap();
        let copy = dir.join("shots/bounce.video.comfy.json");
        let own = fs::read_to_string(&copy).unwrap();
        assert_eq!(own, bundled.replacen("\"weight_dtype\": \"default\"", "\"weight_dtype\": \"fp8_e4m3fn\"", 1));
        let recipe = fs::read_to_string(dir.join("shots/bounce.kerfrecipe")).unwrap();
        assert_eq!(recipe, RECIPE.replace("steps=24\n", "steps=24 workflow=bounce.video.comfy.json\n"));
        // It is now the shot's own, and the next change is to that file.
        let read_again = read(&project, dir, "m1", 4, &models).unwrap();
        let shown = event("m1", 4, &read_again).unwrap();
        assert_eq!((shown["origin"].as_str(), shown["file"].as_str()), (Some("own"), Some("shots/bounce.video.comfy.json")));
        change(&project, dir, &set(&own, json!("fp16")), &models).unwrap();
        assert!(fs::read_to_string(&copy).unwrap().contains("\"weight_dtype\": \"fp16\""));
        assert_eq!(fs::read_dir(dir.join("shots")).unwrap().count(), 2, "no second copy, and nothing left beside it");
        // The editor's copy is old: refused, and the file kept.
        let stale = change(&project, dir, &set(&own, json!("default")), &models).unwrap_err();
        assert!(stale.contains("changed by another program"), "{stale}");
        // A blank is the recipe's, and is refused here with where it is set.
        let now = fs::read_to_string(&copy).unwrap();
        let refused = change(
            &project,
            dir,
            &Edit { input: "unet_name".into(), value: json!("other"), ..set(&now, json!(0)) },
            &models,
        )
        .unwrap_err();
        assert!(refused.contains("which the recipe fills"), "{refused}");
        assert_eq!(fs::read_to_string(&copy).unwrap(), now);
    }

    #[test]
    fn a_shots_graph_says_which_models_have_a_workflow_to_open() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let (project, models) = court(dir);
        let model = |recipe: &Value| {
            recipe["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["kind"] == "model")
                .cloned()
                .unwrap()
        };
        let shown = crate::recipes::event(&project, dir, "m1", &models, None).unwrap();
        assert_eq!(model(&shown)["runner"], json!({"kind": "comfy", "workflow": "shot"}));
        assert_eq!(model(&shown)["workflow"], Value::Null);
        // A model another program runs, or none, has none, and its node says nothing of one.
        for other in ["other", "none"] {
            fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE.replace("model=video", &format!("model={other}"))).unwrap();
            let shown = crate::recipes::event(&project, dir, "m1", &models, None).unwrap();
            assert!(model(&shown).get("runner").is_none(), "{other}");
        }
    }

    #[test]
    fn a_step_no_workflow_runs_says_so() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let (project, models) = court(dir);
        let none = |line: usize| read(&project, dir, "m1", line, &models).err().unwrap();
        assert!(none(3).contains("line 3 of the recipe asks no model"));
        fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE.replace("model=video", "model=other")).unwrap();
        assert!(none(4).contains("model `other` is not run by kerf-comfy"));
        fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE.replace("model=video", "model=none")).unwrap();
        assert!(none(4).contains("no model `none` is set up"));
        // A copy that is already there under the name is not written over: the next name is taken.
        fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE).unwrap();
        fs::write(dir.join("shots/bounce.video.comfy.json"), "someone's").unwrap();
        assert_eq!(copy_name(&dir.join("shots"), "bounce", "video"), "bounce.video-2.comfy.json");
        assert_eq!(copy_name(&dir.join("shots"), "bounce", "wan 2.1/vace"), "bounce.wan-2-1-vace.comfy.json");
    }
}
