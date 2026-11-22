//! Steps changed in one place, as their text.
//!
//! What shows a shot's steps, an editor's graph, holds them as text
//! ([`super::steps_text`]): a line that says what the text is, then a line a
//! step. A change it makes names a line of that text: a name set on it, a
//! step added after it, the step on it taken out. Each is made here and gives
//! back the text as it then is, which whoever asked turns into the edit that
//! makes it of the document ([`super::step_lines`]).
//!
//! ```text
//! generate model=dawn prompt="a boat leaves" seed=7
//! ```
//!
//! with `seed` set to 8 is the same step with `seed=8`. What a change makes
//! is read as steps before it is given back, so a value a step cannot take is
//! refused with the reason a making would give.

use super::{EngineError, Recipe, Result, Step, HEADER};

/// One change to a step. A name that a step gives more than once, `ref=` or
/// `signal=`, is told apart by `nth`: 0 is the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change<'a> {
    /// Have the step give `key` as `value`: the `nth` word that gives it is
    /// replaced, and where the step has no such word one is added.
    Set {
        key: &'a str,
        nth: usize,
        value: &'a str,
    },
    /// Take the `nth` word that gives `key` off the step.
    Remove { key: &'a str, nth: usize },
}

/// The line numbers of `text`'s steps, from 1, in the order the steps are in.
pub fn step_lines(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .skip_while(|(_, line)| *line != HEADER)
        .skip(1)
        .map(|(number, _)| number)
        .collect()
}

/// The steps of `text`, and which of them is on line `number`.
fn on_line(text: &str, number: usize) -> Result<(Vec<Step>, usize)> {
    let steps = super::document_steps(text)?;
    let index = step_lines(text)
        .iter()
        .position(|line| *line == number)
        .ok_or_else(|| EngineError::msg(format!("line {number} is not a step")))?;
    Ok((steps, index))
}

/// `steps` as text, once they read as steps that end in the media's file.
fn read(steps: &[Step]) -> Result<String> {
    kerf_model::making::check(steps).map_err(EngineError::msg)?;
    let text = super::steps_text(steps);
    Recipe::parse(&text)?;
    Ok(text)
}

/// The files the steps before the one at `index` make, which that one may read.
fn made_before(steps: &[Step], index: usize) -> Vec<String> {
    steps
        .iter()
        .take(index)
        .map(|step| step.makes().to_owned())
        .collect()
}

/// `text` with `change` made to the step on its line `number`, counted from 1.
/// The names of a step are the words after what it is and what it draws or
/// joins; of a `render` step, `sequence` names the sequence it draws. What the
/// change makes is read as steps, and one that does not read is refused with
/// why.
pub fn changed(text: &str, number: usize, change: Change) -> Result<String> {
    let (mut steps, index) = on_line(text, number)?;
    let wrong = |what: String| EngineError::msg(format!("step {}: {what}", index + 1));
    let (key, nth) = match change {
        Change::Set { key, nth, .. } | Change::Remove { key, nth } => (key, nth),
    };
    if key.is_empty()
        || key.contains(|character: char| character.is_whitespace() || character == '=')
    {
        return Err(wrong(format!(
            "`{key}` is not a name a step gives a value by"
        )));
    }
    if let Change::Set { value, .. } = change
        && value.contains(['\n', '\r'])
    {
        return Err(wrong(format!(
            "`{key}=` takes one line, and what it is given holds a line break"
        )));
    }
    let mut words = steps[index].words();
    // The words that are `name=value`: all after `generate`, and after the sequence and
    // the file of a `render`, and the parts and the file of a `join`.
    let named = match &steps[index] {
        Step::Generate(_) => 1,
        Step::Render { .. } => 3,
        Step::Join { parts, .. } => 2 + parts.len(),
    };
    let positional = matches!(steps[index], Step::Render { .. }) && key == "sequence";
    let found = if positional {
        Some(1)
    } else {
        words
            .iter()
            .enumerate()
            .skip(named)
            .filter(|(_, word)| word.split_once('=').is_some_and(|(name, _)| name == key))
            .map(|(at, _)| at)
            .nth(nth)
    };
    match (change, found) {
        (Change::Set { value, .. }, Some(at)) if positional => words[at] = value.to_owned(),
        (Change::Set { value, .. }, Some(at)) => words[at] = format!("{key}={value}"),
        (Change::Set { value, .. }, None) => words.push(format!("{key}={value}")),
        (Change::Remove { .. }, Some(at)) if !positional => {
            words.remove(at);
        }
        (Change::Remove { .. }, None) => return Ok(text.to_owned()),
        (Change::Remove { .. }, Some(_)) => {
            return Err(wrong(
                "a `render` step draws one sequence, which can be another and cannot be none"
                    .to_owned(),
            ))
        }
    }
    steps[index] = Step::parse(&words, &made_before(&steps, index)).map_err(wrong)?;
    read(&steps)
}

/// `text` with `line`, a step, added after its line `after`, counted from 1,
/// or as the first step where `after` is 0 or the text's first line. What it
/// makes is read as steps, so a step put after the one that writes the shot's
/// file, or one that reads a file no step before it makes, is refused with
/// why.
pub fn added(text: &str, after: usize, line: &str) -> Result<String> {
    let line = line.trim();
    if line.is_empty() || line.contains(['\n', '\r']) {
        return Err(EngineError::msg("a step is one line"));
    }
    let mut steps = super::document_steps(text)?;
    let lines = step_lines(text);
    if after > lines.last().copied().unwrap_or(1).max(1) {
        return Err(EngineError::msg(format!(
            "there is no line {after} to add a step after"
        )));
    }
    let index = lines.iter().filter(|line| **line <= after).count();
    let words = super::words(line).map_err(EngineError::msg)?;
    let step = Step::parse(&words, &made_before(&steps, index)).map_err(EngineError::msg)?;
    steps.insert(index, step);
    read(&steps)
}

/// `text` without the step on its line `number`, counted from 1. What is left
/// is read as steps, so a step whose file a later one reads, or the one that
/// writes the shot's file, is refused with why.
pub fn removed(text: &str, number: usize) -> Result<String> {
    let (mut steps, index) = on_line(text, number)
        .map_err(|_| EngineError::msg(format!("line {number} is not a step to take out")))?;
    steps.remove(index);
    read(&steps).map_err(|error| {
        EngineError::msg(format!(
            "without step {} its steps do not read: {error}",
            index + 1
        ))
    })
}
