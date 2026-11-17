//! The workflow behind a model: a `generate` step whose model a person's
//! entry runs with `kerf-comfy` can be read as the ComfyUI graph it runs,
//! every node and input of it, and a value in it set.
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
//! A person's change ([`set`]) makes that copy once and then changes it in
//! place. An agent's ([`copied`]) writes over nothing: each is a new copy,
//! named by a recipe that is itself kept as the next version, so that undo
//! goes back to the workflow and the recipe before.
//!
//! A model run by anything else has no workflow to show, and its node says
//! nothing of one. Which program runs a model is read from the person's own
//! models file and from nothing in the project.

use kerf_engine::recipe::model::{Ask, Models, Source};
use kerf_engine::recipe::text::{self, Change};
use kerf_engine::recipe::{Recipe, Step};
use kerf_model::Project;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The program a workflow is run by, by its file's name.
const PROGRAM: &str = "kerf-comfy";

/// What says which workflow that program runs, in a model's entry.
const FLAG: &str = "--workflow";

/// The workflow a model's `command` runs, where the command is `kerf-comfy`'s:
/// the word after `--workflow`, a name of one that comes with the program or
/// a file. None for a model any other program runs.
pub fn runs(command: &[String]) -> Option<&str> {
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

/// Where a step's workflow comes from.
pub enum Origin {
    /// It comes with `kerf-comfy`, by its name there.
    Bundled,
    /// A file the person's models entry names, outside the project.
    File,
    /// The shot's own, a file of the project its line names: its path from
    /// the project's folder, and where it is.
    Own(String, PathBuf),
}

/// The workflow of one `generate` step, read.
pub struct Read {
    pub ask: Ask,
    /// What the model's entry runs, as it names it.
    pub name: String,
    pub origin: Origin,
    pub text: String,
    /// The recipe's text, which a first change adds a word to.
    pub recipe: String,
}

/// The `generate` step on line `line` of the recipe media `id` names, and
/// the workflow it is made with.
pub fn read(
    project: &Project,
    directory: &Path,
    id: &str,
    line: usize,
    models: &Models,
) -> Result<Read, String> {
    let (_, recipe) = crate::graph::recipe(project, directory, id)?;
    read_of(project, directory, id, line, models, recipe)
}

/// As [`read`], of `recipe` as the text of media `id`'s recipe: what it
/// would be once a change not yet written is.
pub fn read_of(
    project: &Project,
    directory: &Path,
    id: &str,
    line: usize,
    models: &Models,
    recipe: String,
) -> Result<Read, String> {
    let wrong = |error: kerf_engine::EngineError| error.to_string();
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

/// The workflow of one step as an editor shows it and an agent reads it:
/// what it makes, every node with every input, and of each blank what fills
/// it and what the line gives for it now (`given`).
pub fn shown(id: &str, line: usize, read: &Read) -> Result<Value, String> {
    let mut event = kerf_workflow::detail::described(&read.text)?;
    // Beside what fills each blank, what the line gives for it now.
    if let Some(blanks) = event["blanks"].as_object_mut() {
        for (name, blank) in blanks {
            blank["given"] = json!(given(&read.ask, name));
        }
    }
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
fn copy_name(folder: &Path, shot: &str, model: &str, taken: &[PathBuf]) -> String {
    let clean: String = model
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() || "-_".contains(character) { character } else { '-' })
        .collect();
    (1..)
        .map(|number| match number {
            1 => format!("{shot}.{clean}.comfy.json"),
            more => format!("{shot}.{clean}-{more}.comfy.json"),
        })
        .find(|name| {
            let file = folder.join(name);
            !file.exists() && !taken.contains(&file)
        })
        .expect("a number no file has")
}

/// Write `text` to `file` whole: beside it first, then moved into place.
pub fn write(file: &Path, text: &str) -> Result<(), String> {
    let beside = file.with_extension(format!("{}.writing", std::process::id()));
    std::fs::write(&beside, text)
        .and_then(|()| std::fs::rename(&beside, file))
        .map_err(|error| {
            let _ = std::fs::remove_file(&beside);
            format!("cannot write the workflow {}: {error}", file.display())
        })
}

/// One value of a workflow to set: input `input` of node `node`.
#[derive(Clone, Copy, Debug)]
pub struct Set<'a> {
    pub node: &'a str,
    pub input: &'a str,
    pub value: &'a Value,
}

/// A copy of a workflow to keep for a shot, and the recipe that names it.
/// Nothing of it is written yet.
#[derive(Clone, Debug)]
pub struct Copy {
    /// Where the copy goes, and its path from the project's folder.
    pub file: PathBuf,
    pub stored: String,
    /// The workflow's text.
    pub text: String,
    /// The recipe's text with the step's line naming the copy.
    pub recipe: String,
}

/// `read`'s workflow with each of `sets` made, in turn.
pub fn with(read: &Read, sets: &[Set]) -> Result<String, String> {
    sets.iter().try_fold(read.text.clone(), |text, set| {
        kerf_workflow::detail::set_input(&text, set.node, set.input, set.value)
    })
}

/// A new copy of the workflow of the step on `line`, holding `now`, beside
/// the recipe of media `id` under a name no file there has and none of
/// `taken` is, and the recipe with the line told to run it.
pub fn copied(
    project: &Project,
    directory: &Path,
    id: &str,
    line: usize,
    read: &Read,
    now: &str,
    taken: &[PathBuf],
) -> Result<Copy, String> {
    let media = project
        .media(id)
        .ok_or_else(|| format!("no media `{id}` in this project"))?;
    let recipe = media
        .recipe
        .as_deref()
        .ok_or_else(|| format!("media `{id}` names no recipe"))?;
    let shot = kerf_engine::recipe::shot_name(recipe);
    // Beside the recipe: in its folder, by a name no file there has.
    let beside = |name: &str| {
        kerf_engine::recipe::beside(project, directory, id, name).map_err(|error| error.to_string())
    };
    let (folder, _) = beside(".")?;
    let name = copy_name(&folder, &shot, &read.ask.model, taken);
    let (file, stored) = beside(&name)?;
    let recipe = text::changed(
        &read.recipe,
        line,
        Change::Set {
            key: kerf_engine::recipe::model::WORKFLOW,
            nth: 0,
            value: &name,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(Copy {
        file,
        stored,
        text: now.to_owned(),
        recipe,
    })
}

/// A person's change: the shot's own copy of the workflow of the step on
/// `line` with `set` made, made beside the recipe where the shot had none,
/// and the line told to run it. `was` is the workflow as they read it: one
/// changed since is left alone.
pub fn set(
    project: &Project,
    directory: &Path,
    id: &str,
    line: usize,
    was: &str,
    set: Set,
    models: &Models,
) -> Result<(), String> {
    let read = read(project, directory, id, line, models)?;
    if read.text != was {
        return Err("the workflow was changed by another program since it was read; it is left as that left it".to_owned());
    }
    let now = with(&read, &[set])?;
    if let Origin::Own(_, file) = &read.origin {
        return write(file, &now);
    }
    // The first change: the workflow as it comes is copied into the project with it.
    let copy = copied(project, directory, id, line, &read, &now, &[])?;
    write(&copy.file, &copy.text)?;
    text::write(project, directory, id, &read.recipe, &copy.recipe).map_err(|error| {
        // The line does not name it, so nothing would run it: it is not left lying there.
        let _ = std::fs::remove_file(&copy.file);
        error.to_string()
    })
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
        let shown = super::shown("m1", 4, &read(&project, dir, "m1", 4, &models).unwrap()).unwrap();
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
        let change = |was: &str, input: &str, value: Value| {
            set(&project, dir, "m1", 4, was, Set { node: &node, input, value: &value }, &models)
        };
        let bundled = shown["text"].as_str().unwrap().to_owned();
        change(&bundled, &input, json!("fp8_e4m3fn")).unwrap();
        let copy = dir.join("shots/bounce.video.comfy.json");
        let own = fs::read_to_string(&copy).unwrap();
        assert_eq!(own, bundled.replacen("\"weight_dtype\": \"default\"", "\"weight_dtype\": \"fp8_e4m3fn\"", 1));
        let recipe = fs::read_to_string(dir.join("shots/bounce.kerfrecipe")).unwrap();
        assert_eq!(recipe, RECIPE.replace("steps=24\n", "steps=24 workflow=bounce.video.comfy.json\n"));
        // It is now the shot's own, and the next change is to that file.
        let read_again = read(&project, dir, "m1", 4, &models).unwrap();
        let shown = super::shown("m1", 4, &read_again).unwrap();
        assert_eq!((shown["origin"].as_str(), shown["file"].as_str()), (Some("own"), Some("shots/bounce.video.comfy.json")));
        change(&own, &input, json!("fp16")).unwrap();
        assert!(fs::read_to_string(&copy).unwrap().contains("\"weight_dtype\": \"fp16\""));
        assert_eq!(fs::read_dir(dir.join("shots")).unwrap().count(), 2, "no second copy, and nothing left beside it");
        // The editor's copy is old: refused, and the file kept.
        let stale = change(&own, &input, json!("default")).unwrap_err();
        assert!(stale.contains("changed by another program"), "{stale}");
        // A blank is the recipe's, and is refused here with where it is set.
        let now = fs::read_to_string(&copy).unwrap();
        let refused = change(&now, "unet_name", json!("other")).unwrap_err();
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
        let shown = crate::graph::shown(&project, dir, "m1", &models).unwrap();
        assert_eq!(model(&shown)["runner"], json!({"kind": "comfy", "workflow": "shot"}));
        assert_eq!(model(&shown)["workflow"], Value::Null);
        // A model another program runs, or none, has none, and its node says nothing of one.
        for other in ["other", "none"] {
            fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE.replace("model=video", &format!("model={other}"))).unwrap();
            let shown = crate::graph::shown(&project, dir, "m1", &models).unwrap();
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
        // An agent's change is a new copy and the recipe that names it, with nothing written.
        fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE).unwrap();
        let read = read(&project, dir, "m1", 4, &models).unwrap();
        let described = shown("m1", 4, &read).unwrap();
        let loader = described["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["inputs"].as_array().unwrap().iter().any(|input| input["name"] == "weight_dtype"))
            .and_then(|node| node["id"].as_str())
            .expect("a loader");
        let value = json!("fp16");
        let now = with(&read, &[Set { node: loader, input: "weight_dtype", value: &value }]).unwrap();
        assert!(now.contains("\"weight_dtype\": \"fp16\""));
        let copy = copied(&project, dir, "m1", 4, &read, &now, &[dir.join("shots/bounce.video.comfy.json")]).unwrap();
        assert_eq!(copy.stored, "shots/bounce.video-2.comfy.json");
        assert!(copy.recipe.ends_with("steps=24 workflow=bounce.video-2.comfy.json\n"), "{}", copy.recipe);
        assert!(!copy.file.exists());
        assert_eq!(fs::read_to_string(dir.join("shots/bounce.kerfrecipe")).unwrap(), RECIPE);
        // A copy that is already there under the name is not written over: the next name is taken.
        fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE).unwrap();
        fs::write(dir.join("shots/bounce.video.comfy.json"), "someone's").unwrap();
        assert_eq!(copy_name(&dir.join("shots"), "bounce", "video", &[]), "bounce.video-2.comfy.json");
        assert_eq!(copy_name(&dir.join("shots"), "bounce", "wan 2.1/vace", &[]), "bounce.wan-2-1-vace.comfy.json");
    }
}
