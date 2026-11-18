//! A media's recipe as a graph, with what runs each of its models.

use kerf_engine::recipe::model::Models;
use kerf_engine::recipe::text::{self, Change};
use kerf_model::Project;
use serde_json::{json, Value};
use std::path::Path;

/// The recipe of media `id` as a graph: its nodes and edges, its text, what
/// would refuse its making (`cannot`) and what is worth knowing first
/// (`notes`), as [`kerf_engine::recipe::graph`] lays it out. A model its
/// entry runs with a program whose workflow can be opened says so under
/// `runner`: which program, and which workflow.
pub fn shown(project: &Project, directory: &Path, id: &str, models: &Models) -> Result<Value, String> {
    let graph =
        kerf_engine::recipe::graph(project, directory, id, models).map_err(|error| error.to_string())?;
    let mut shown = serde_json::to_value(&graph).map_err(|error| error.to_string())?;
    // Which workflow a model is run with is the person's own models file's to say.
    for node in shown["nodes"].as_array_mut().into_iter().flatten() {
        let workflow = node["model"]
            .as_str()
            .and_then(|model| models.command(model).ok())
            .and_then(crate::workflow::runs);
        if let Some(workflow) = workflow {
            node["runner"] = json!({"kind": "comfy", "workflow": workflow});
        }
    }
    Ok(shown)
}

/// One name on one line of a recipe to set, or with no `value` to take off.
#[derive(Clone, Copy, Debug)]
pub struct Word<'a> {
    /// The line, from 1.
    pub line: usize,
    pub key: &'a str,
    /// Which of the words of that name on the line, from 0.
    pub nth: usize,
    pub value: Option<&'a str>,
}

/// `recipe` with `word` set or taken off, and nothing else of it changed
/// ([`text::changed`]): refused as a making would refuse what it makes.
pub fn changed(recipe: &str, word: Word) -> Result<String, String> {
    let change = match word.value {
        Some(value) => Change::Set {
            key: word.key,
            nth: word.nth,
            value,
        },
        None => Change::Remove {
            key: word.key,
            nth: word.nth,
        },
    };
    text::changed(recipe, word.line, change).map_err(|error| error.to_string())
}

/// `recipe` with the step `line` added as a line of its own after line `after`
/// ([`text::added`]), and without its step on line `line` ([`text::removed`]):
/// each refused where what it makes does not read as a recipe.
pub fn added(recipe: &str, after: usize, line: &str) -> Result<String, String> {
    text::added(recipe, after, line).map_err(|error| error.to_string())
}

pub fn removed(recipe: &str, line: usize) -> Result<String, String> {
    text::removed(recipe, line).map_err(|error| error.to_string())
}

/// A step to add to `recipe`, of a kind a person picks in the editor, and the line to add
/// it after: a guide (`guide`), a model asked for a file (`model`), or the join of the
/// pictures in motion its steps make (`join`). It is written to read as it stands, with
/// names no step has yet, and is then set in its card as any step is:
///
/// - a guide draws the sequence the recipe already draws, or the project's first, in depth;
/// - a model is the one the recipe already asks, or the first the person has set up, with
///   the prompt of the step before it;
/// - a join sets every picture in motion the recipe's models make end to end.
///
/// A guide and a model go before the step that writes the shot's file, which stays last; a
/// join of parts goes in its place where that was one of the parts.
pub fn step(
    project: &Project,
    models: &Models,
    recipe: &str,
    kind: &str,
) -> Result<(usize, String), String> {
    use kerf_engine::recipe::{Recipe, Step};
    let read = Recipe::parse(recipe).map_err(|error| error.to_string())?;
    let lines = text::step_lines(recipe);
    let made = read.makes();
    // A name no step has: `guide.mp4`, then `guide2.mp4`.
    let fresh = |stem: &str, extension: &str| {
        (1..)
            .map(|number| match number {
                1 => format!("{stem}.{extension}"),
                more => format!("{stem}{more}.{extension}"),
            })
            .find(|name| !made.contains(name))
            .expect("a number no file has")
    };
    // Before the step that writes the shot's file.
    let before_last = lines.len().checked_sub(2).map_or(0, |index| lines[index]);
    match kind {
        "guide" => {
            let sequence = read
                .sequences()
                .next()
                .map(str::to_owned)
                .or_else(|| project.sequences.first().map(|sequence| sequence.id.to_string()))
                .ok_or("the project has no sequence to draw a guide from")?;
            Ok((before_last, format!("render {sequence} {{{}}} pass=depth", fresh("guide", "mp4"))))
        }
        "model" => {
            let asked = read.steps.iter().rev().find_map(|step| match step {
                Step::Generate(ask) => Some(ask.as_ref()),
                _ => None,
            });
            let model = asked
                .map(|ask| ask.model.clone())
                .or_else(|| models.names().next().map(str::to_owned))
                .ok_or("no model is set up to ask: set one up in your models file first")?;
            let prompt = asked.map_or("", |ask| ask.prompt.as_str());
            // Quoted as a line quotes a value that holds spaces.
            let quoted = prompt.replace('\\', "\\\\").replace('"', "\\\"");
            let line = format!(
                "generate model={model} prompt=\"{quoted}\" out={{{}}}",
                fresh("part", "mp4")
            );
            Ok((before_last, line))
        }
        "join" => {
            // What models made, in the order they made it: a guide is not a part of the shot.
            let parts: Vec<&String> = read
                .steps
                .iter()
                .filter_map(|step| match step {
                    Step::Generate(ask) => Some(&ask.out),
                    _ => None,
                })
                .filter(|name| name.ends_with(".mp4") || name.ends_with(".mov") || name.ends_with(".webm"))
                .collect();
            if parts.len() < 2 {
                return Err("a join sets two or more pictures in motion that models made end to end, and the recipe has fewer: add the parts first, each a model with `out={name.mp4}`".to_owned());
            }
            let named: Vec<String> = parts.iter().map(|part| format!("{{{part}}}")).collect();
            Ok((
                before_last,
                format!("join {} {{{}}}", named.join(" "), fresh("joined", "mp4")),
            ))
        }
        other => Err(format!("`{other}` is not a step to add: a guide, a model, or a join")),
    }
}

/// The steps that make media `id`, as text ([`kerf_engine::recipe::steps_text`]): a line
/// that says what the text is, then a line a step. A media nothing makes is refused, and
/// so is one whose steps are still in a recipe's file, with how they are carried.
pub fn written(project: &Project, id: &str) -> Result<String, String> {
    kerf_engine::recipe::written(project, id).map_err(|error| error.to_string())
}

/// The step lines a script gets to have media `id`, whose steps the document holds,
/// made by `now` in place of `was` ([`kerf_engine::recipe::step_lines`]): what a change
/// made to their text comes to as the edit that makes it.
pub fn step_lines(id: &str, was: &str, now: &str) -> Result<Vec<String>, String> {
    kerf_engine::recipe::step_lines(id, was, now).map_err(|error| error.to_string())
}

/// The line of `now`, from 1, that `was` has not got: where a step was added.
pub fn added_at(was: &str, now: &str) -> Option<usize> {
    let mut before = was.lines();
    now.lines()
        .position(|line| before.next() != Some(line))
        .map(|index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_to_add_reads_as_it_stands_and_goes_before_the_last() {
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Court\"\nsequence main rate=24 size=320x180\nsequence court rate=24 size=320x180\n",
        )
        .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("models.json");
        std::fs::write(&file, r#"{"video": {"run": ["v", "{request}"]}, "still": {"run": ["s", "{request}"]}}"#).unwrap();
        let models = Models::read(&file).unwrap();
        let recipe = "kerf-recipe 1\nrender court {guide.mp4} pass=depth\ngenerate model=video prompt=\"a \\\"ball\\\" bounces\" guide={guide.mp4}\n";
        let add = |recipe: &str, kind: &str| {
            let (after, line) = step(&project, &models, recipe, kind)?;
            let now = added(recipe, after, &line)?;
            Ok::<_, String>((added_at(recipe, &now), line, now))
        };
        // A guide of the sequence the steps draw, by a name no step has, before the last step.
        let (at, line, with_guide) = add(recipe, "guide").unwrap();
        assert_eq!((at, line.as_str()), (Some(3), "render court {guide2.mp4} pass=depth"));
        // A model as the steps ask one, with its prompt, quotes kept.
        let (at, line, with_part) = add(&with_guide, "model").unwrap();
        assert_eq!(at, Some(4));
        assert_eq!(line, "generate model=video prompt=\"a \\\"ball\\\" bounces\" out={part.mp4}");
        // A join wants two pictures in motion that models made: a guide is not a part.
        let few = step(&project, &models, &with_part, "join").unwrap_err();
        assert!(few.contains("two or more pictures in motion"), "{few}");
        let (_, _, with_parts) = add(&with_part, "model").unwrap();
        let (_, line, _) = add(&with_parts, "join").unwrap();
        assert_eq!(line, "join {part.mp4} {part2.mp4} {joined.mp4}");
        // In steps of one it is the first.
        let (at, _, now) = add("kerf-recipe 1\ngenerate model=still prompt=\"p\"\n", "guide").unwrap();
        assert_eq!(at, Some(2));
        assert!(now.starts_with("kerf-recipe 1\nrender main {guide.mp4} pass=depth\n"));
        assert!(step(&project, &models, recipe, "sound").unwrap_err().contains("is not a step to add"));
        // Taken out, the steps are as they were.
        assert_eq!(removed(&with_guide, 3).unwrap(), recipe);
    }
}
