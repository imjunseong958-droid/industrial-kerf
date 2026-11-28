//! A model that is a program on the person's own computer (`{"run": [...]}`), handed the request
//! and run until it has written the file.
//!
//! A program written for Kerf reads the request, and its command names its file
//! (`{"dawn": {"run": ["dawn-adapter", "--request", "{request}"]}}`). One that was not, a renderer
//! or a script, is handed its arguments in place of a request ([`command_for`]):
//! `{"previs": {"run": ["vork", "render", "{ref}", "--out", "{out}"]}}`, asked by
//! `generate model=previs ref=scenes/open.scene.json out={guide.mp4}`. So what runs is always a
//! program the person named, and a document only says which and with what.
//!
//! The program runs as a recipe's `run` did, supervised and never through a shell, in the
//! project's folder; it writes what it has to say to stderr, and ends well once the file at
//! `out` is written. A project cannot name a program: the file that does is the person's,
//! outside any project.

use kerf_engine::error::{EngineError, Result};
use kerf_engine::process::sidecar;
use kerf_engine::recipe::model::{GenerateRequest, Generation, ModelSpec};
use serde_json::Value;
use std::path::Path;

/// What stands for the request's file in a program's command.
pub const REQUEST: &str = "{request}";

/// What stands for where the file goes in the command of a program that is handed its
/// arguments and no request: a renderer, or any program that was not written for Kerf.
pub const OUT: &str = "{out}";

/// `words` as a person reads a command, quoted where one holds a space.
pub(crate) fn shown(words: &[String]) -> String {
    words
        .iter()
        .map(|word| {
            if word.is_empty() || word.contains(char::is_whitespace) {
                format!("\"{word}\"")
            } else {
                word.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run model `model`'s program, `run` as the person wrote it, for `generation`, to its end;
/// one that does not end well is an error with what it said.
pub(crate) fn generate(
    model: &str,
    run: &[String],
    takes: Option<&ModelSpec>,
    generation: &Generation,
    on_line: &mut dyn FnMut(&str),
) -> Result<()> {
    let words = command_for(model, run, generation.file, generation.request, takes)?;
    let mut command = std::process::Command::new(&words[0]);
    command.args(&words[1..]).current_dir(generation.directory);
    let finished = sidecar::run(command, generation.cancellation, on_line)?;
    if finished.status.success() {
        Ok(())
    } else {
        Err(EngineError::msg(format!(
            "{} failed ({}): {}",
            words[0],
            finished.status,
            finished.stderr.trim()
        )))
    }
}

/// A number as a blank is filled with it: a whole one without a point.
fn number_text(number: &serde_json::Number) -> String {
    match number.as_f64() {
        Some(whole) if whole.fract() == 0.0 && whole.abs() < 1e15 => format!("{whole:.0}"),
        _ => number.to_string(),
    }
}

/// The command that asks a model: `words`, as the person's file gives its `run`, with each
/// blank filled from `asked`, the request a step comes to, whose file is `request`.
///
/// A program written for Kerf reads the request, and its command names it (`{request}`).
/// One that was not, a renderer, a script, is handed what it needs as its arguments, by
/// the names a step gives them: `{out}`, where its file goes; `{prompt}`; the files,
/// `{guide}`, `{first}`, `{last}` and `{ref}` (`{ref2}` for the second); `{seconds}`,
/// `{seed}`, `{size}`, `{width}` and `{height}`; and any other name the step sets
/// (`profile=std/previs` fills `{profile}`), or the value the entry's `takes` gives it
/// under `more` where the step does not. A blank with nothing to fill it is refused, with
/// the word that fills it.
///
/// The program and the shape of its command are the person's own, and a project only
/// fills the blanks: an argument that is a blank alone is refused where what fills it
/// would read as an option (`-…`), so a step cannot hand the program a switch its
/// owner did not write.
pub fn command_for(
    model: &str,
    words: &[String],
    request: &Path,
    asked: &GenerateRequest,
    takes: Option<&ModelSpec>,
) -> Result<Vec<String>> {
    // What the person's file gives a name under `more`, as they wrote it.
    let text = |value: &Value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number_text(number)),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    };
    let filled = |name: &str| -> Option<String> {
        let size = |at: usize| asked.size.map(|size| size[at]);
        match name {
            "request" => Some(request.to_string_lossy().into_owned()),
            "out" => Some(asked.out.clone()),
            "prompt" => Some(asked.prompt.clone()),
            "guide" => asked.guide.clone(),
            "first" => asked.first.clone(),
            "last" => asked.last.clone(),
            "seconds" => asked
                .seconds
                .and_then(serde_json::Number::from_f64)
                .map(|seconds| number_text(&seconds)),
            "seed" => asked.seed.map(|seed| number_text(&seed.into())),
            "size" => Some(format!("{}x{}", size(0)?, size(1)?)),
            "width" => size(0).map(|side| side.to_string()),
            "height" => size(1).map(|side| side.to_string()),
            other => {
                // `ref` is the first picture, `ref2` the second.
                if let Some(count) = other.strip_prefix("ref") {
                    let at = match count {
                        "" => Some(0),
                        number => number
                            .parse::<usize>()
                            .ok()
                            .filter(|at| *at >= 2)
                            .map(|at| at - 1),
                    };
                    if let Some(at) = at {
                        return asked.references.get(at).cloned();
                    }
                }
                if let Some(guide) = other.strip_prefix("guide_") {
                    return asked.guide_of(guide).cloned();
                }
                asked
                    .more
                    .get(other)
                    .cloned()
                    .or_else(|| takes.and_then(|takes| takes.more.get(other)).and_then(text))
            }
        }
    };
    words
        .iter()
        .map(|word| {
            let mut out = String::new();
            let mut rest = word.as_str();
            while let Some(open) = rest.find('{') {
                out.push_str(&rest[..open]);
                let after = &rest[open + 1..];
                let named = after
                    .split_once('}')
                    .filter(|(name, _)| kerf_model::making::file_name(name));
                match named {
                    Some((name, tail)) => {
                        let value = filled(name).ok_or_else(|| {
                            EngineError::msg(format!(
                                "model `{model}`'s program is handed `{{{name}}}`, and the step gives none: add `{}` to its step",
                                match name {
                                    "out" => "out={<name>}".to_owned(),
                                    "width" | "height" => "size=<W>x<H>".to_owned(),
                                    numbered if numbered.starts_with("ref") => "ref=<file>".to_owned(),
                                    other => format!("{other}=<value>"),
                                }
                            ))
                        })?;
                        if word.len() == name.len() + 2 && value.starts_with('-') {
                            return Err(EngineError::msg(format!(
                                "`{name}={value}` would read as an option of model `{model}`'s program, which its owner did not write: a value does not start with `-`"
                            )));
                        }
                        out.push_str(&value);
                        rest = tail;
                    }
                    None => {
                        out.push('{');
                        rest = after;
                    }
                }
            }
            out.push_str(rest);
            Ok(out)
        })
        .collect()
}
