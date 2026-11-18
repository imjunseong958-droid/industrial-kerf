//! A recipe's text changed in one place, and nowhere else.
//!
//! A recipe is a file a person writes, with comments that say why a step is
//! there and steps in the order they thought of them. Something that sets one
//! thing in it, an editor's seed field or a knob, must not write the file
//! again from what it read: the comments would go, and so would how each line
//! was spelled. So a change is made to the text itself. The word that gives
//! the value is replaced where it stands, a new one goes to the end of its
//! line, and every other byte of the file, its line endings among them, is
//! what it was.
//!
//! ```text
//! generate model=dawn prompt="a boat leaves" seed=7     # the one we kept
//! ```
//!
//! with `seed` set to 8 is the same line with `seed=8`, comment and spacing
//! as they were. What a change makes of the recipe is read as a recipe
//! before it is given back, so a value that a line cannot take is refused
//! with the reason a making would give, and the text is left alone.

use super::{EngineError, Recipe, Result, HEADER};

/// One change to a line of a recipe. A name that a line gives more than
/// once, `ref=` or `signal=`, is told apart by `nth`: 0 is the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change<'a> {
    /// Have the line give `key` as `value`: the `nth` word that gives it is
    /// replaced where it stands, and where the line has no such word one is
    /// added at its end.
    Set {
        key: &'a str,
        nth: usize,
        value: &'a str,
    },
    /// Take the `nth` word that gives `key` off the line.
    Remove { key: &'a str, nth: usize },
}

/// A word of a line: where it starts and ends in the line, in bytes, and
/// what it reads as once its quotes are taken off.
struct Word {
    start: usize,
    end: usize,
    read: String,
    quoted: bool,
}

/// The words of `line` with where each is, read as [`super::words`] reads
/// them: split at spaces, with `"…"` keeping what it holds as one word.
fn spans(line: &str) -> std::result::Result<Vec<Word>, String> {
    let mut found = Vec::new();
    let mut word: Option<Word> = None;
    let mut characters = line.char_indices();
    while let Some((at, character)) = characters.next() {
        if character.is_whitespace() {
            if let Some(mut done) = word.take() {
                done.end = at;
                found.push(done);
            }
            continue;
        }
        let current = word.get_or_insert(Word {
            start: at,
            end: at,
            read: String::new(),
            quoted: false,
        });
        if character != '"' {
            current.read.push(character);
            continue;
        }
        current.quoted = true;
        loop {
            match characters.next() {
                Some((_, '"')) => break,
                Some((_, '\\')) => match characters.next() {
                    Some((_, escaped @ ('"' | '\\'))) => current.read.push(escaped),
                    Some((_, other)) => {
                        current.read.push('\\');
                        current.read.push(other);
                    }
                    None => return Err("a quote is not closed".to_owned()),
                },
                Some((_, other)) => current.read.push(other),
                None => return Err("a quote is not closed".to_owned()),
            }
        }
    }
    if let Some(mut done) = word {
        done.end = line.len();
        found.push(done);
    }
    Ok(found)
}

/// `key=value` as a line writes it: the value in quotes where it holds a
/// space or a quote or is empty, or where `quote` asks for them.
fn written(key: &str, value: &str, quote: bool) -> String {
    let needs = quote
        || value.is_empty()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character == '"');
    if !needs {
        return format!("{key}={value}");
    }
    let mut out = format!("{key}=\"");
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            out.push('\\');
        }
        out.push(character);
    }
    out.push('"');
    out
}

/// The lines of `text`, each with where it starts and where its words end:
/// its line ending, `\n` or `\r\n`, is left out of it.
fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = 0;
    text.split_inclusive('\n').map(move |whole| {
        let at = start;
        start += whole.len();
        let line = whole.strip_suffix('\n').unwrap_or(whole);
        (at, line.strip_suffix('\r').unwrap_or(line))
    })
}

/// The line numbers of `text`'s steps, from 1, in the order the steps are
/// in: `run`, `render`, and `generate` lines, which [`Recipe::parse`] reads
/// one step each from. A `needs` line is no step.
pub fn step_lines(text: &str) -> Vec<usize> {
    lines(text)
        .enumerate()
        .map(|(index, (_, line))| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .skip_while(|(_, line)| *line != HEADER)
        .skip(1)
        .filter(|(_, line)| line.split_whitespace().next() != Some("needs"))
        .map(|(number, _)| number)
        .collect()
}

/// `text` with `change` made to its line `number`, counted from 1 as a
/// recipe's errors count them. Nothing else of the text changes. The line
/// has to be a `generate`, a `render` or a `join` line, whose words after
/// what it reads and makes are `name=value`; of a `render` line, `sequence` names the sequence it
/// draws. What the change makes is read as a recipe, and one that does not
/// read is refused with why.
pub fn changed(text: &str, number: usize, change: Change) -> Result<String> {
    let wrong = |what: String| EngineError::msg(format!("line {number} of the recipe: {what}"));
    let (at, line) = number
        .checked_sub(1)
        .and_then(|index| lines(text).nth(index))
        .ok_or_else(|| wrong("the recipe has no such line".to_owned()))?;
    let words = spans(line).map_err(wrong)?;
    let verb = words.first().map(|word| word.read.as_str());
    // The words that are `name=value`: all after `generate`, and after the
    // sequence and the file of a `render`.
    let named = match verb {
        Some("generate") => 1,
        Some("render") => 3,
        // After the parts and the file they make, each in braces.
        Some("join") => {
            1 + words
                .iter()
                .skip(1)
                .take_while(|word| super::whole(&word.read).is_some())
                .count()
        }
        _ => {
            return Err(wrong(
                "only a `generate`, a `render` or a `join` line has names to set".to_owned(),
            ))
        }
    };
    let (key, nth) = match change {
        Change::Set { key, nth, .. } | Change::Remove { key, nth } => (key, nth),
    };
    if key.is_empty() || key.contains(|character: char| character.is_whitespace() || character == '=') {
        return Err(wrong(format!("`{key}` is not a name a line gives a value by")));
    }
    if let Change::Set { value, .. } = change {
        if value.contains(['\n', '\r']) {
            return Err(wrong(format!(
                "`{key}=` takes one line, and what it is given holds a line break"
            )));
        }
    }
    // A `render` line's sequence is its second word, with no name before it.
    let positional = verb == Some("render") && key == "sequence";
    let found = if positional {
        words.get(1)
    } else {
        words
            .iter()
            .skip(named)
            .filter(|word| word.read.split_once('=').is_some_and(|(name, _)| name == key))
            .nth(nth)
    };
    let mut line_now = line.to_owned();
    match (change, found) {
        (Change::Set { value, .. }, Some(word)) => {
            let new = if positional {
                written_alone(value)
            } else {
                written(key, value, word.quoted)
            };
            line_now.replace_range(word.start..word.end, &new);
        }
        (Change::Set { value, .. }, None) if !positional => {
            // After the last word, before whatever space ends the line.
            let end = words.last().map_or(line.len(), |word| word.end);
            line_now.insert_str(end, &format!(" {}", written(key, value, key == "prompt")));
        }
        (Change::Remove { .. }, Some(word)) if !positional => {
            // With the space before it, so that the words either side keep one between them.
            let before = line[..word.start]
                .char_indices()
                .rev()
                .take_while(|(_, character)| character.is_whitespace())
                .last()
                .map_or(word.start, |(index, _)| index);
            line_now.replace_range(before..word.end, "");
        }
        (Change::Remove { .. }, None) if !positional => return Ok(text.to_owned()),
        _ => {
            return Err(wrong(
                "a `render` line draws one sequence, which can be another and cannot be none"
                    .to_owned(),
            ))
        }
    }
    let mut now = String::with_capacity(text.len() + line_now.len());
    now.push_str(&text[..at]);
    now.push_str(&line_now);
    now.push_str(&text[at + line.len()..]);
    Recipe::parse(&now)?;
    Ok(now)
}

/// `text` with `line`, a step, added as a line of its own after line `after`, counted from
/// 1, or as the first step where `after` is 0 or the recipe's first line. Nothing else of
/// the text changes. What it makes is read as a recipe, so a step put after the one that
/// writes the shot's file, or one that reads a file no step before it makes, is refused
/// with why.
pub fn added(text: &str, after: usize, line: &str) -> Result<String> {
    let line = line.trim();
    if line.is_empty() || line.contains(['\n', '\r']) {
        return Err(EngineError::msg("a step is one line of the recipe"));
    }
    // Not before the line that says it is a recipe.
    let header = lines(text)
        .position(|(_, line)| line.trim() == HEADER)
        .map_or(0, |index| index + 1);
    let after = after.max(header);
    let (at, ends) = match after.checked_sub(1).and_then(|index| lines(text).nth(index)) {
        Some((at, line)) => (at + line.len(), true),
        None if after == 0 => (0, false),
        None => return Err(EngineError::msg(format!("the recipe has no line {after} to add a step after"))),
    };
    // Past the line break that ends the line it follows, which a last line may not have.
    let rest = &text[at..];
    let broken = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'));
    let mut now = String::with_capacity(text.len() + line.len() + 2);
    match (ends, broken) {
        (true, Some(rest)) => {
            now.push_str(&text[..text.len() - rest.len()]);
            now.push_str(line);
            now.push('\n');
            now.push_str(rest);
        }
        (true, None) => {
            now.push_str(text);
            now.push('\n');
            now.push_str(line);
            now.push('\n');
        }
        (false, _) => {
            now.push_str(line);
            now.push('\n');
            now.push_str(text);
        }
    }
    Recipe::parse(&now)?;
    Ok(now)
}

/// `text` without its line `number`, a step, counted from 1. Nothing else of the text
/// changes. What is left is read as a recipe, so a step whose file a later one reads, or
/// the one that writes the shot's file, is refused with why.
pub fn removed(text: &str, number: usize) -> Result<String> {
    if !step_lines(text).contains(&number) {
        return Err(EngineError::msg(format!(
            "line {number} of the recipe is not a step to take out"
        )));
    }
    let (at, line) = lines(text).nth(number - 1).expect("a step's line is there");
    let rest = &text[at + line.len()..];
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
        .unwrap_or(rest);
    let now = format!("{}{rest}", &text[..at]);
    Recipe::parse(&now).map_err(|error| {
        EngineError::msg(format!("without line {number} the recipe does not read: {error}"))
    })?;
    Ok(now)
}

/// Have the recipe media `id` of `project`, kept in `directory`, names hold
/// `now` in place of `was`: the file is written only where it still holds
/// `was`, so that what another program wrote since is not written over
/// unseen, and only with a text that reads as a recipe. It is written beside
/// itself and moved into place, so a making that reads it reads one or the
/// other whole.
///
/// This is a person's own change to their recipe, as a text editor makes
/// it: the file is written in place, its takes then say the recipe changed
/// since they were made, and the next making is of the recipe as it now
/// reads. An agent's recipe is kept as a version instead ([`super::version`]).
pub fn write(
    project: &kerf_model::Project,
    directory: &std::path::Path,
    id: &str,
    was: &str,
    now: &str,
) -> Result<()> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let name = match super::Home::of(media) {
        Some(super::Home::File(name)) => name,
        Some(super::Home::Document) => {
            return Err(EngineError::msg(format!(
                "media `{id}` is made by the steps under its line in the document, which an edit changes: `step {id} set <n> <name>=<value>`"
            )))
        }
        None => return Err(EngineError::msg(format!("media `{id}` names no recipe"))),
    };
    let name = name.as_str();
    let file = super::inside_project(directory, name, "recipe")?;
    let there = std::fs::read_to_string(&file).map_err(|error| {
        EngineError::msg(format!("cannot read the recipe {}: {error}", file.display()))
    })?;
    if there != was {
        return Err(EngineError::msg(format!(
            "the recipe {name} was changed by another program since it was read; it is left as that left it"
        )));
    }
    Recipe::parse(now)?;
    if there == now {
        return Ok(());
    }
    let beside = file.with_extension(format!("{}.writing", std::process::id()));
    let written = std::fs::write(&beside, now).and_then(|()| std::fs::rename(&beside, &file));
    written.map_err(|error| {
        let _ = std::fs::remove_file(&beside);
        EngineError::msg(format!("cannot write the recipe {}: {error}", file.display()))
    })
}

/// A word that is no `name=value`, as a line writes it.
fn written_alone(value: &str) -> String {
    written("", value, false)
        .strip_prefix('=')
        .expect("written with an empty name")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE: &str = "kerf-recipe 1\r\n\
        # A ball on a court.\r\n\
        needs court.json\r\n\
        \r\n\
        render court {depth.mp4}   pass=depth size=832x480 rate=16\r\n\
        \x20 # The ball, made once and handed on.\r\n\
        generate model=still prompt=\"an orange \\\"game\\\" ball\" seed=3 out={ball.png}\r\n\
        generate model=video   prompt=\"it bounces\" guide={depth.mp4} ref={ball.png} ref=../refs/court.png signal=bounce seed=7 strength=0.8  \r\n";

    fn set(text: &str, line: usize, key: &str, value: &str) -> String {
        changed(text, line, Change::Set { key, nth: 0, value }).unwrap()
    }

    #[test]
    fn a_step_is_added_and_taken_out_as_a_line_of_its_own() {
        let text = "kerf-recipe 1\n# The court.\nrender s1 {a.mp4}\n\n# The shot.\ngenerate model=m prompt=\"p\" guide={a.mp4}\n";
        // After the step it follows, the comments and the blank line where they were.
        let more = added(text, 3, "  render s1 {b.mp4} pass=depth ").unwrap();
        assert_eq!(more, text.replace("{a.mp4}\n\n", "{a.mp4}\nrender s1 {b.mp4} pass=depth\n\n"));
        // As the first step: after the line that says it is a recipe, wherever it was asked.
        let first = added(text, 0, "render s1 {b.mp4}").unwrap();
        assert_eq!(first, text.replace("kerf-recipe 1\n", "kerf-recipe 1\nrender s1 {b.mp4}\n"));
        // After a last line that has no line break.
        let open = "kerf-recipe 1\nrender s1 {a.mp4}";
        assert_eq!(
            added(open, 2, "join {a.mp4} {a.mp4} {out}").unwrap(),
            "kerf-recipe 1\nrender s1 {a.mp4}\njoin {a.mp4} {a.mp4} {out}\n"
        );
        // What it would make of the recipe is read as one.
        let late = added(text, 6, "render s1 {b.mp4}").unwrap_err().to_string();
        assert!(late.contains("so it is the last"), "{late}");
        let unmade = added(text, 1, "generate model=m prompt=\"p\" guide={c.mp4} out={d.mp4}").unwrap_err().to_string();
        assert!(unmade.contains("no step before this one makes `{c.mp4}`"), "{unmade}");
        assert!(added(text, 9, "render s1 {b.mp4}").unwrap_err().to_string().contains("no line 9"));
        assert!(added(text, 3, "render s1 {b.mp4}\nrun x").is_err());

        // Taken out again, the text is as it was.
        assert_eq!(removed(&more, 4).unwrap(), text);
        // A step a later one reads, the last, and a line that is no step stay.
        let read = removed(text, 3).unwrap_err().to_string();
        assert!(read.contains("without line 3 the recipe does not read") && read.contains("{a.mp4}"), "{read}");
        assert!(removed(text, 6).unwrap_err().to_string().contains("does not read"));
        assert!(removed(text, 2).unwrap_err().to_string().contains("is not a step"));
        assert!(removed(text, 1).unwrap_err().to_string().contains("is not a step"));
    }

    #[test]
    fn a_joins_lap_is_set_on_its_line() {
        let text = "kerf-recipe 1\nrender s1 {a.mp4}\nrender s1 {b.mp4}\njoin {a.mp4} {b.mp4} {out}  lap=1\n";
        let set = |value| Change::Set { key: "lap", nth: 0, value };
        assert_eq!(changed(text, 4, set("2")).unwrap(), text.replace("lap=1", "lap=2"));
        let none = changed(text, 4, Change::Remove { key: "lap", nth: 0 }).unwrap();
        assert_eq!(none, text.replace("  lap=1", ""));
        // Where the line has none it is added after the file the join makes.
        assert_eq!(changed(&none, 4, set("1")).unwrap(), text.replace("  lap=1", " lap=1"));
        let refused = changed(text, 4, set("some")).unwrap_err().to_string();
        assert!(refused.contains("`lap=some` is not a whole number"), "{refused}");
    }

    #[test]
    fn a_value_is_replaced_where_it_stands_and_nothing_else_moves() {
        let now = set(RECIPE, 8, "seed", "8");
        assert_eq!(now, RECIPE.replace("seed=7", "seed=8"));
        // The comments, the empty line, the spaces, and the line endings are as they were.
        assert!(now.contains("# A ball on a court.\r\n") && now.contains("strength=0.8  \r\n"));
        assert!(now.contains("render court {depth.mp4}   pass=depth"));
        // Setting what it has already is no change.
        assert_eq!(set(RECIPE, 8, "seed", "7"), RECIPE);
    }

    #[test]
    fn a_prompt_keeps_its_quotes_and_what_it_holds_is_escaped() {
        let now = set(RECIPE, 7, "prompt", "a \"red\" ball\\court");
        assert!(now.contains("prompt=\"a \\\"red\\\" ball\\\\court\" seed=3"));
        let Step::Generate(ask) = &Recipe::parse(&now).unwrap().steps[1] else {
            panic!("a generate step")
        };
        assert_eq!(ask.prompt, "a \"red\" ball\\court");
        // A word that had no quotes gets them once it needs them, and only then.
        assert!(set(RECIPE, 8, "strength", "0.6").contains(" strength=0.6  \r\n"));
        assert!(set(RECIPE, 8, "negative", "blur, text").contains("strength=0.8 negative=\"blur, text\"  \r\n"));
    }

    use super::super::Step;

    #[test]
    fn a_name_the_line_has_not_goes_to_its_end() {
        let now = set(RECIPE, 8, "steps", "30");
        assert!(now.ends_with("seed=7 strength=0.8 steps=30  \r\n"));
        // A prompt is always in quotes.
        let bare = "kerf-recipe 1\ngenerate model=dawn";
        assert_eq!(
            set(bare, 2, "prompt", "boat"),
            "kerf-recipe 1\ngenerate model=dawn prompt=\"boat\""
        );
    }

    #[test]
    fn one_of_several_is_told_by_its_place() {
        let second = changed(
            RECIPE,
            8,
            Change::Set {
                key: "ref",
                nth: 1,
                value: "../refs/hall.png",
            },
        )
        .unwrap();
        assert!(second.contains("ref={ball.png} ref=../refs/hall.png signal=bounce"));
        let removed = changed(RECIPE, 8, Change::Remove { key: "ref", nth: 0 }).unwrap();
        assert!(removed.contains("guide={depth.mp4} ref=../refs/court.png signal=bounce"));
        // One more than it has is one added.
        let third = changed(
            RECIPE,
            8,
            Change::Set {
                key: "ref",
                nth: 2,
                value: "../refs/net.png",
            },
        )
        .unwrap();
        assert!(third.contains("strength=0.8 ref=../refs/net.png  \r\n"));
        // Taking off what is not there changes nothing.
        assert_eq!(
            changed(RECIPE, 8, Change::Remove { key: "last", nth: 0 }).unwrap(),
            RECIPE
        );
    }

    #[test]
    fn a_render_line_sets_what_follows_its_file_and_its_sequence() {
        let now = set(RECIPE, 5, "pass", "ids");
        assert!(now.contains("render court {depth.mp4}   pass=ids size=832x480 rate=16\r\n"));
        let now = set(RECIPE, 5, "sequence", "hall");
        assert!(now.contains("render hall {depth.mp4}   pass=depth"));
        let now = changed(RECIPE, 5, Change::Remove { key: "pass", nth: 0 }).unwrap();
        assert!(now.contains("render court {depth.mp4} size=832x480 rate=16\r\n"));
        assert!(changed(RECIPE, 5, Change::Remove { key: "sequence", nth: 0 }).is_err());
    }

    #[test]
    fn what_does_not_read_as_a_recipe_is_refused_with_why() {
        let refused = |line, key, value| {
            changed(RECIPE, line, Change::Set { key, nth: 0, value })
                .unwrap_err()
                .to_string()
        };
        assert!(refused(8, "seed", "seven").contains("`seed=seven` is not a whole number"));
        assert!(refused(5, "size", "833x480").contains("has an odd side"));
        assert!(refused(8, "guide", "{nothing.mp4}").contains("no step before this one makes"));
        assert!(refused(8, "prompt", "two\nlines").contains("holds a line break"));
        assert!(refused(3, "seed", "1").contains("only a `generate`, a `render` or a `join` line"));
        assert!(refused(2, "seed", "1").contains("only a `generate`, a `render` or a `join` line"));
        assert!(refused(40, "seed", "1").contains("no such line"));
        // The last step writes the media's file, so its `out=` cannot be another.
        assert!(refused(8, "out", "{other.mp4}").contains("names no `{out}`"));
    }

    #[test]
    fn a_recipe_is_written_in_place_only_over_what_was_read() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("shots")).unwrap();
        let file = folder.path().join("shots/open.kerfrecipe");
        let was = "kerf-recipe 1\n# kept\ngenerate model=dawn prompt=\"a boat\" seed=7\n";
        std::fs::write(&file, was).unwrap();
        let project = kerf_lang::parse_document(
            "kerf 1\nproject name=\"shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/open.kerfrecipe\"\nmedia m2 path=\"a.mp4\" rate=24 dur=1s video=160x120\n",
            kerf_model::Catalog::builtin(),
        )
        .unwrap()
        .project;
        let now = set(was, 3, "seed", "8");
        write(&project, folder.path(), "m1", was, &now).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), now);
        // Nothing is left beside it.
        assert_eq!(std::fs::read_dir(folder.path().join("shots")).unwrap().count(), 1);
        // What was read is no longer what is there: the file stays as it is.
        let stale = write(&project, folder.path(), "m1", was, was).unwrap_err().to_string();
        assert!(stale.contains("changed by another program"), "{stale}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), now);
        // A text that is no recipe is never written.
        assert!(write(&project, folder.path(), "m1", &now, "kerf-recipe 1\n").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), now);
        assert!(write(&project, folder.path(), "m2", "", "").unwrap_err().to_string().contains("names no recipe"));
    }

    #[test]
    fn steps_are_counted_by_their_lines() {
        assert_eq!(step_lines(RECIPE), [5, 7, 8]);
        assert_eq!(step_lines("# first\n\nkerf-recipe 1\nrun a {out}\n"), [4]);
        assert!(step_lines("").is_empty());
    }
}
