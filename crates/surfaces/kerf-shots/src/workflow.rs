//! The workflow behind a model: a `generate` step whose model a person's
//! entry runs with `kerf-comfy` can be read as the ComfyUI graph it runs,
//! every node and input of it, and a value in it set.
//!
//! Setting one changes nothing on a ComfyUI server and nothing of
//! `kerf-comfy`. The workflow as it comes is copied into the project, beside
//! the recipe, with that value changed and the rest as it was written
//! ([`kerf_workflow::detail`]), and the step's line is given `workflow=<file>`,
//! which has the model's program run the copy for this shot
//! The copy is a file of the project: it travels with it,
//! a take is held to it, and taking the word off the line goes back to the
//! workflow as it comes.
//!
//! A person's change ([`set`]) makes that copy once and then changes it in
//! place. An agent's ([`copied`]) writes over nothing: each is a new copy,
//! named by a `step` line of its edit, so that undo goes back to the
//! workflow and the steps before.
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
    let recipe = crate::graph::written(project, id)?;
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
        .ok_or_else(|| format!("step {} asks no model", line.saturating_sub(1)))?;
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
        .map(|character| {
            if character.is_ascii_alphanumeric() || "-_".contains(character) {
                character
            } else {
                '-'
            }
        })
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
        .expect("a folder holds finitely many files, so a number past them all is free")
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

/// What the server a workflow would be run on takes at each input of each kind of node,
/// as the program that runs it says (`kerf-comfy --workflow … --inputs`): by a node's
/// kind and then its input's name, `choices` to choose among, or a number's `min`, `max`
/// and `step`. Asked once for a workflow and kept for as long as this process runs; a
/// program that cannot say, with no server named or none answering, is said why, and
/// asked again the next time.
///
/// Kerf asks no server itself: this runs the model's own program, as a making does.
pub fn taken(models: &Models, read: &Read) -> Result<Value, String> {
    let command = models
        .command(&read.ask.model)
        .map_err(|error| error.to_string())?;
    let program = command.first().ok_or("the model's entry runs no program")?;
    let named = match &read.origin {
        Origin::Own(_, file) => file.to_string_lossy().into_owned(),
        Origin::Bundled | Origin::File => read.name.clone(),
    };
    let key = taken_key(program, read);
    if let Some(kept) = taken_kept()
        .lock()
        .ok()
        .and_then(|kept| kept.get(&key).cloned())
    {
        return Ok(kept);
    }
    // What could not be asked is asked again the next time: a server may be back by then.
    let asked = asked_inputs(program, &named)?;
    if let Ok(mut kept) = taken_kept().lock() {
        kept.insert(key, asked.clone());
    }
    Ok(asked)
}

/// What [`taken`] has learned of this workflow already, without asking.
pub fn taken_before(models: &Models, read: &Read) -> Option<Value> {
    let command = models.command(&read.ask.model).ok()?;
    taken_kept()
        .lock()
        .ok()?
        .get(&taken_key(command.first()?, read))
        .cloned()
}

/// What a workflow's answer is kept under: the program asked and the kinds of node it
/// has, so the shot's own copy of a workflow, which has the same nodes, is not asked again.
fn taken_key(program: &str, read: &Read) -> String {
    let classes = kerf_workflow::workflow::Workflow::parse(&read.text)
        .map(|workflow| workflow.classes().into_iter().collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    format!("{program}\n{classes}")
}

type Taken = std::sync::Mutex<std::collections::HashMap<String, Value>>;

fn taken_kept() -> &'static Taken {
    static KEPT: std::sync::OnceLock<Taken> = std::sync::OnceLock::new();
    KEPT.get_or_init(Default::default)
}

/// How long the program is given to ask its server before it is given up on.
const ASKING: std::time::Duration = std::time::Duration::from_secs(20);

/// Run `program --workflow <named> --inputs` and read what it prints.
fn asked_inputs(program: &str, named: &str) -> Result<Value, String> {
    use std::process::Stdio;
    let mut child = kerf_engine::recipe::model::program(program)
        .args([FLAG, named, "--inputs"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < ASKING => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{PROGRAM} did not say what the server takes within {} s",
                    ASKING.as_secs()
                ));
            }
        }
    }
    let said = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !said.status.success() {
        let why = String::from_utf8_lossy(&said.stderr);
        return Err(why.trim().trim_start_matches("kerf-comfy: ").to_owned());
    }
    serde_json::from_slice(&said.stdout).map_err(|error| {
        format!("{PROGRAM} said something that is not what its inputs take: {error}")
    })
}

/// `shown`, a workflow as [`shown`] says it, with what each of its inputs takes beside
/// it: `choices`, or `min`, `max` and `step`, from `taken`.
pub fn with_taken(shown: &mut Value, taken: &Value) {
    for node in shown["nodes"].as_array_mut().into_iter().flatten() {
        let class = node["class"].as_str().unwrap_or_default().to_owned();
        // What it gives at each output, for a link drawn from it.
        if let Some(outputs) = taken[&class].get(OUTPUTS) {
            node["outputs"] = outputs.clone();
        }
        for input in node["inputs"].as_array_mut().into_iter().flatten() {
            let name = input["name"].as_str().unwrap_or_default().to_owned();
            if let Some(takes) = taken[&class][&name].as_object() {
                for (key, value) in takes {
                    input[key] = value.clone();
                }
            }
        }
    }
    shown["taken"] = json!(true);
}

/// Whether each of `sets` is something its input takes, by `taken`: one of its choices,
/// or a number no less than its least and no more than its most. Refused with what it
/// does take. An input `taken` says nothing of is let through.
///
/// Asked by [`with`], of sets the workflow itself has taken: what a server lists for an
/// input says nothing of whether the input is the workflow's to set.
fn check(read: &Read, sets: &[Set], taken: &Value) -> Result<(), String> {
    let described = kerf_workflow::detail::described(&read.text)?;
    for set in sets {
        let class = described["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|node| node["id"] == set.node)
            .and_then(|node| node["class"].as_str())
            .unwrap_or_default();
        let takes = &taken[class][set.input];
        if let Some((from, output)) = set.link {
            // A link carries what its input takes: a model to a model, pictures to pictures.
            let from_class = described["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|node| node["id"] == from)
                .and_then(|node| node["class"].as_str())
                .unwrap_or_default();
            let gives = &taken[from_class][OUTPUTS];
            if let (Some(wanted), Some(outputs)) = (takes["type"].as_str(), gives.as_array()) {
                match outputs.get(output as usize).and_then(Value::as_str) {
                    Some(given) if given == wanted || wanted == "*" || given == "*" => {}
                    Some(given) => {
                        return Err(format!(
                            "`{}` of node {} takes a {wanted}, and output {output} of node {from}, a {from_class}, is a {given}",
                            set.input, set.node
                        ))
                    }
                    None => {
                        return Err(format!(
                            "node {from}, a {from_class}, has {} outputs, and none is its output {output}",
                            outputs.len()
                        ))
                    }
                }
            }
            continue;
        }
        if let Some(choices) = takes["choices"].as_array()
            && !choices.contains(set.value)
        {
            let listed: Vec<String> = choices
                .iter()
                .take(12)
                .map(|choice| match choice {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect();
            return Err(format!(
                "`{}` of node {} takes one of {}{}, not {}",
                set.input,
                set.node,
                listed.join(", "),
                if choices.len() > listed.len() {
                    format!(", and {} more", choices.len() - listed.len())
                } else {
                    String::new()
                },
                set.value
            ));
        }
        if let Some(value) = set.value.as_f64() {
            let (least, most) = (takes["min"].as_f64(), takes["max"].as_f64());
            if least.is_some_and(|least| value < least) || most.is_some_and(|most| value > most) {
                return Err(format!(
                    "`{}` of node {} takes a number from {} to {}, not {}",
                    set.input, set.node, takes["min"], takes["max"], set.value
                ));
            }
        }
    }
    Ok(())
}

/// One input of a workflow to set, input `input` of node `node`: to a value, or, with
/// `link`, to read output `link.1` of node `link.0` in place of the node it read.
#[derive(Clone, Copy, Debug)]
pub struct Set<'a> {
    pub node: &'a str,
    pub input: &'a str,
    pub value: &'a Value,
    pub link: Option<(&'a str, u64)>,
}

impl<'a> Set<'a> {
    /// Input `input` of node `node` set to `value`.
    pub fn value(node: &'a str, input: &'a str, value: &'a Value) -> Self {
        Self {
            node,
            input,
            value,
            link: None,
        }
    }

    /// Input `input` of node `node` linked to output `output` of node `from`.
    pub fn link(node: &'a str, input: &'a str, from: &'a str, output: u64) -> Self {
        Self {
            node,
            input,
            value: &Value::Null,
            link: Some((from, output)),
        }
    }
}

/// What a kind of node gives is said under, in what [`taken`] returns.
const OUTPUTS: &str = "$outputs";

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

/// `read`'s workflow with each of `sets` made, in turn, and each held to what its input
/// takes on the server, where that has been asked: `taken`, as [`taken_before`] has it.
///
/// What the workflow itself refuses is refused first. A blank is the step's to fill and a
/// link the graph's shape whatever a server lists for the input, and the refusal says
/// where each is set. The server's list is no answer there: it sends whoever set the
/// value to one that is on the list, which is refused all the same.
pub fn with(read: &Read, sets: &[Set], taken: Option<&Value>) -> Result<String, String> {
    let now = sets
        .iter()
        .try_fold(read.text.clone(), |text, set| match set.link {
            Some((from, output)) => {
                kerf_workflow::detail::set_link(&text, set.node, set.input, from, output)
            }
            None => kerf_workflow::detail::set_input(&text, set.node, set.input, set.value),
        })?;
    if let Some(taken) = taken {
        check(read, sets, taken)?;
    }
    Ok(now)
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
    let wrong = |error: kerf_engine::EngineError| error.to_string();
    // Beside the shot: in the folder its takes' folder is in, by a name no file there has.
    let folder = kerf_engine::recipe::shot_folder(directory, media).map_err(wrong)?;
    let name = copy_name(&directory.join(&folder), id, &read.ask.model, taken);
    let stored = folder
        .join(&name)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    let file = directory.join(&stored);
    // The step names it as it names any file: from the project's folder.
    let named = stored.as_str();
    let recipe = text::changed(
        &read.recipe,
        line,
        Change::Set {
            key: kerf_engine::recipe::model::WORKFLOW,
            nth: 0,
            value: named,
        },
    )
    .map_err(wrong)?;
    Ok(Copy {
        file,
        stored,
        text: now.to_owned(),
        recipe,
    })
}

/// What a person's change to a workflow left to do: nothing, for a copy the shot has
/// already; and for a copy just made, which its step has yet to name, the lines of the
/// edit that has it name it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Named {
    /// Lines for kerfd to apply, as an edit of the person's own.
    pub script: Vec<String>,
    /// The copy just made, which nothing names until the lines are applied: if they are
    /// refused, it is not left lying there.
    pub copy: Option<PathBuf>,
}

/// A person's change: the shot's own copy of the workflow of the step on
/// `line` with `set` made, made beside the shot where it had none,
/// and the step told to run it. `was` is the workflow as they read it: one
/// changed since is left alone.
pub fn set(
    project: &Project,
    directory: &Path,
    id: &str,
    line: usize,
    was: &str,
    set: Set,
    models: &Models,
) -> Result<Named, String> {
    let read = read(project, directory, id, line, models)?;
    if read.text != was {
        return Err("the workflow was changed by another program since it was read; it is left as that left it".to_owned());
    }
    // Held to what the server takes there, where that has been asked.
    let now = with(&read, &[set], taken_before(models, &read).as_ref())?;
    if let Origin::Own(_, file) = &read.origin {
        return write(file, &now).map(|()| Named::default());
    }
    // The first change: the workflow as it comes is copied into the project with it.
    let copy = copied(project, directory, id, line, &read, &now, &[])?;
    write(&copy.file, &copy.text)?;
    // The step does not name it yet, so nothing would run it: it is not left lying there.
    let unnamed = |why: String| {
        let _ = std::fs::remove_file(&copy.file);
        why
    };
    let script = crate::graph::step_lines(id, &read.recipe, &copy.recipe).map_err(unnamed)?;
    Ok(Named {
        script,
        copy: Some(copy.file),
    })
}
