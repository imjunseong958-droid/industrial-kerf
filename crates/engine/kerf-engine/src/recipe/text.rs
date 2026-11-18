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
    if key.is_empty() || key.contains(|character: char| character.is_whitespace() || character == '=') {
        return Err(wrong(format!("`{key}` is not a name a step gives a value by")));
    }
    if let Change::Set { value, .. } = change {
        if value.contains(['\n', '\r']) {
            return Err(wrong(format!(
                "`{key}=` takes one line, and what it is given holds a line break"
            )));
        }
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
        return Err(EngineError::msg(format!("there is no line {after} to add a step after")));
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

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: &str = "kerf-recipe 1\n\
        render court {depth.mp4} pass=depth size=832x480 rate=16\n\
        generate model=still prompt=\"an orange \\\"game\\\" ball\" seed=3 out={ball.png}\n\
        generate model=video prompt=\"it bounces\" guide={depth.mp4} ref={ball.png} ref=refs/court.png signal=bounce seed=7 strength=0.8\n";

    fn set(text: &str, line: usize, key: &str, value: &str) -> String {
        changed(text, line, Change::Set { key, nth: 0, value }).unwrap()
    }

    #[test]
    fn a_name_is_set_on_one_step_and_no_other() {
        assert_eq!(step_lines(STEPS), [2, 3, 4]);
        // The step's own word, and the others as they were.
        assert_eq!(set(STEPS, 4, "seed", "8"), STEPS.replace("seed=7", "seed=8"));
        assert_eq!(set(STEPS, 3, "seed", "8"), STEPS.replace("seed=3", "seed=8"));
        // A value that holds spaces or quotes is written as the document writes it.
        let said = set(STEPS, 4, "prompt", "a \"red\" ball bounces");
        assert!(said.contains("prompt=\"a \\\"red\\\" ball bounces\" guide={depth.mp4}"), "{said}");
        // A name the step has not is added, and one of several is told apart by `nth`.
        assert!(set(STEPS, 4, "seconds", "4").contains("seconds=4 seed=7"));
        let second = changed(STEPS, 4, Change::Set { key: "ref", nth: 1, value: "refs/net.png" }).unwrap();
        assert!(second.contains("ref={ball.png} ref=refs/net.png signal=bounce"), "{second}");
        let third = changed(STEPS, 4, Change::Set { key: "ref", nth: 2, value: "refs/net.png" }).unwrap();
        assert!(third.contains("ref={ball.png} ref=refs/court.png ref=refs/net.png"), "{third}");
        // Taken off, and nothing for one that is not there.
        let off = changed(STEPS, 4, Change::Remove { key: "signal", nth: 0 }).unwrap();
        assert_eq!(off, STEPS.replace(" signal=bounce", ""));
        assert_eq!(changed(STEPS, 4, Change::Remove { key: "moves", nth: 0 }).unwrap(), STEPS);
        // What a guide draws is a word alone, and its names are set the same way.
        assert_eq!(set(STEPS, 2, "sequence", "main"), STEPS.replace("render court", "render main"));
        assert_eq!(set(STEPS, 2, "pass", "ids"), STEPS.replace("pass=depth", "pass=ids"));
    }

    #[test]
    fn what_a_step_cannot_take_is_refused_as_a_making_would_refuse_it() {
        let wrong = |line: usize, key: &str, value: &str| {
            changed(STEPS, line, Change::Set { key, nth: 0, value }).unwrap_err().to_string()
        };
        assert_eq!(wrong(4, "seed", "soon"), "step 3: `seed=soon` is not a whole number");
        assert!(wrong(4, "guide", "{nothing.mp4}").contains("no step before this one makes `{nothing.mp4}`"));
        assert!(wrong(2, "size", "831x480").contains("odd side"));
        assert!(wrong(1, "seed", "8").contains("line 1 is not a step"));
        assert!(wrong(4, "a b", "c").contains("is not a name a step gives a value by"));
        // A file a later step reads cannot be named away.
        assert!(wrong(3, "out", "{other.png}").contains("step 3 reads `{ball.png}`, which no step before it makes"));
        let none = changed(STEPS, 2, Change::Remove { key: "sequence", nth: 0 }).unwrap_err().to_string();
        assert!(none.contains("cannot be none"), "{none}");
    }

    #[test]
    fn a_step_is_added_after_a_line_and_taken_out() {
        // After a line, as the first where that is the text's first, and the text as it was
        // once it is taken out again.
        let guide = "render court {ids.mp4} pass=ids";
        let more = added(STEPS, 2, guide).unwrap();
        assert_eq!(step_lines(&more), [2, 3, 4, 5]);
        assert!(more.starts_with("kerf-recipe 1\nrender court {depth.mp4} pass=depth size=832x480 rate=16\nrender court {ids.mp4} pass=ids\n"), "{more}");
        assert_eq!(removed(&more, 3).unwrap(), STEPS);
        let first = added(STEPS, 0, guide).unwrap();
        assert!(first.starts_with("kerf-recipe 1\nrender court {ids.mp4} pass=ids\nrender court {depth.mp4}"), "{first}");
        assert_eq!(added(STEPS, 1, guide).unwrap(), first);
        // After the step that writes the shot's file, and one that reads what is not made yet.
        let last = added(STEPS, 4, guide).unwrap_err().to_string();
        assert!(last.contains("so it is the last"), "{last}");
        let early = added(STEPS, 1, "generate model=video guide={depth.mp4} out={a.mp4}").unwrap_err().to_string();
        assert!(early.contains("no step before this one makes `{depth.mp4}`"), "{early}");
        assert!(added(STEPS, 9, guide).unwrap_err().to_string().contains("no line 9"));
        assert!(added(STEPS, 2, "run vork --out {x.mp4}").unwrap_err().to_string().contains("`run` is not a step"));
        // A step a later one reads, and the last, stay.
        let read = removed(STEPS, 2).unwrap_err().to_string();
        assert!(read.contains("without step 1 its steps do not read: step 2 reads `{depth.mp4}`"), "{read}");
        let out = removed(STEPS, 4).unwrap_err().to_string();
        assert!(out.contains("names no `{out}`"), "{out}");
        assert!(removed(STEPS, 1).unwrap_err().to_string().contains("line 1 is not a step to take out"));
    }
}
