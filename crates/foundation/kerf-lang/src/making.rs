//! The steps that say how a media's file is made, as text: the lines under a
//! media line of a document, and the `step` lines of a script.
//!
//! ```text
//! media depth path="shots/depth.mp4" rate=16 dur=65f video=832x480
//!   render s2 {guide.mp4} pass=depth size=832x480 rate=16
//!   generate model=shot prompt="a red car on a wet street" guide={guide.mp4} seconds=4 seed=7
//! ```
//!
//! What a step is, and what it may say, is the model's
//! ([`kerf_model::making`]). Here its words are read from a lexed line and
//! written back, one way for every step, so that a document's diff is the
//! change that was made and a take is held to the steps and not to how they
//! were typed.

use crate::lex::{Arg, Atom, Line};
use crate::text::quote;
use kerf_model::making::{Step, VERBS};
use kerf_model::Diagnostic;

/// The words of a step as [`Step::parse`] reads them, from the arguments of a lexed line:
/// each a word alone or a `name=value`, with no quotes around it.
fn words(args: &[Arg]) -> Result<Vec<String>, (usize, String)> {
    args.iter()
        .map(|arg| {
            let value = match &arg.value {
                Atom::Word(word) | Atom::Str(word) => word.clone(),
                Atom::Group(inside) => format!("{{{}}}", inside.join(" ")),
            };
            // A step is one line, and is handed on as one: what breaks a line is not kept.
            if value.chars().any(char::is_control) {
                return Err((
                    arg.column,
                    "a step is one line of words: take the line break or the tab out of this one"
                        .to_owned(),
                ));
            }
            Ok(match &arg.key {
                Some(key) => format!("{key}={value}"),
                None => value,
            })
        })
        .collect()
}

/// The step the arguments of `line` from `from` on say, the first of them being what it
/// is. `made` holds the files the steps before it make.
pub(crate) fn read(line: &Line, from: usize, made: &[String]) -> Result<Step, Diagnostic> {
    let args = line.args.get(from..).unwrap_or_default();
    let column = args.first().map_or(1, |arg| arg.column);
    let words = words(args).map_err(|(column, why)| line.error(column, why))?;
    Step::parse(&words, made).map_err(|why| line.error(column, why))
}

/// Whether `word` starts a step.
pub(crate) fn starts(word: &str) -> bool {
    VERBS.contains(&word)
}

/// Whether `value` is written as it is: a word the lexer reads whole, or a `{name}`.
fn plain(value: &str, alone: bool) -> bool {
    // Any space, not the one the lexer parts words at alone: whatever reads the line
    // parts them at each.
    let word = |text: &str| {
        !text.is_empty()
            && !text
                .chars()
                .any(|c| matches!(c, '"' | '{' | '}' | '=') || c.is_whitespace() || c.is_control())
    };
    let braced = value
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .is_some_and(word);
    // A word alone that starts with `#` would read as a comment.
    braced || (word(value) && !(alone && value.starts_with('#')))
}

/// A step as a document writes it: its words in the order the model gives them, each
/// value quoted where it holds a space or anything else a word cannot, and a prompt always.
pub fn print_step(step: &Step) -> String {
    step.words()
        .iter()
        .enumerate()
        .map(|(index, word)| match word.split_once('=') {
            // What a step is and what it draws or joins are words alone; the rest are named.
            Some((key, value)) if index > 0 && !key.is_empty() && plain(key, true) => {
                // A prompt is text, and is written as text is, whatever it holds.
                if plain(value, false) && key != "prompt" {
                    word.clone()
                } else {
                    format!("{key}={}", quote(value))
                }
            }
            _ if plain(word, true) => word.clone(),
            _ => quote(word),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The words of a step line of a script that set names on a step that is there, each
/// `name=value`: `None` where the first of them is what a step is, and the line writes
/// the step whole.
pub(crate) fn names(line: &Line, from: usize) -> Result<Option<Vec<(String, String)>>, Diagnostic> {
    let args = line.args.get(from..).unwrap_or_default();
    match args.first() {
        Some(Arg {
            key: None,
            value: Atom::Word(word),
            ..
        }) if starts(word) => return Ok(None),
        _ => {}
    }
    let words = words(args).map_err(|(column, why)| line.error(column, why))?;
    args.iter()
        .zip(words)
        .map(|(arg, word)| match (&arg.key, word.split_once('=')) {
            (Some(_), Some((key, value))) => Ok((key.to_owned(), value.to_owned())),
            _ => Err(line.error(
                arg.column,
                format!(
                    "`{word}` is not `<name>=<value>`: set names on the step, as `seed=8`, or write it whole, starting with `render`, `generate`, or `join`"
                ),
            )),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// `step` with each of `names` set: a name the step has takes the value, or every value
/// where it is given more than once (`ref=`, `signal=`), in place of those it had; a name
/// it has not is added; and a name given nothing (`seed=""`) is taken off. What that
/// makes is read as a step, so a value the step cannot take is refused with why. `made`
/// holds the files the steps before it make.
pub(crate) fn with_names(
    step: &Step,
    names: &[(String, String)],
    made: &[String],
) -> Result<Step, String> {
    let mut words = step.words();
    // The words that are named: all after `generate`, and after what a `render` draws
    // and a `join` joins, which are words alone.
    let named = |word: &str| {
        kerf_model::making::whole(word).is_none() && word.contains('=')
    };
    let first = match step {
        Step::Render { .. } => 3,
        Step::Generate(_) => 1,
        Step::Join { .. } => words
            .iter()
            .position(|word| named(word))
            .unwrap_or(words.len()),
    };
    let mut set: Vec<&str> = Vec::new();
    for (key, _) in names {
        if set.contains(&key.as_str()) {
            continue;
        }
        set.push(key);
        let mut kept = 0;
        words.retain(|word| {
            kept += 1;
            kept <= first || word.split_once('=').is_none_or(|(name, _)| name != key)
        });
    }
    words.extend(
        names
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, value)| format!("{key}={value}")),
    );
    Step::parse(&words, made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::lex;

    fn step(text: &str, made: &[&str]) -> Result<Step, Diagnostic> {
        let lines = lex(text).unwrap();
        let made: Vec<String> = made.iter().map(|name| (*name).to_owned()).collect();
        read(&lines[0], 0, &made)
    }

    #[test]
    fn a_step_prints_as_it_reads() {
        for (written, made) in [
            ("render s2 {guide.mp4} pass=depth size=832x480 rate=16", vec![]),
            ("render court {d2.mp4} matte=ball from=5 seconds=5.0625", vec![]),
            (
                "generate model=shot prompt=\"a red car, \\\"wet\\\" street\" guide={guide.mp4} seconds=4 seed=7",
                vec!["guide.mp4"],
            ),
            (
                "generate model=matte guide=footage/walk.mp4 inside=\"0.55,0.80 0.45,0.82\" which=0,2 out={man.mp4}",
                vec![],
            ),
            ("generate model=still prompt=\"#1\" tint=#ff0000 empty=\"\" out={ball.png}", vec![]),
            ("join {a.mp4} {b.mp4} {out} lap=1", vec!["a.mp4", "b.mp4"]),
            // A space that is not the plain one is kept inside quotes too.
            ("generate model=dawn prompt=\"a\u{a0}boat\" ref=refs/a\u{a0}b.png", vec![]),
        ] {
            let read = step(written, &made).unwrap();
            let printed = print_step(&read);
            assert_eq!(step(&printed, &made).unwrap(), read, "{printed}");
            assert!(!printed.contains("=refs/a\u{a0}b.png"), "{printed}");
        }
        // One way for every step, however it was typed.
        let typed = step("generate seed=7 prompt=boat model=dawn guide=\"refs/a b.mp4\"", &[]).unwrap();
        assert_eq!(
            print_step(&typed),
            "generate model=dawn prompt=\"boat\" guide=\"refs/a b.mp4\" seed=7"
        );
    }

    #[test]
    fn what_a_step_cannot_say_is_refused_where_it_is() {
        let error = step("generate model=dawn prompt=\"a boat\\nleaves\"", &[]).unwrap_err();
        assert!(error.message.contains("a step is one line"), "{error}");
        assert_eq!(error.span.unwrap().column, 21);
        let error = step("generate model=dawn guide={guide.mp4}", &[]).unwrap_err();
        assert!(error.message.contains("no step before this one makes `{guide.mp4}`"), "{error}");
        let error = step("run vork render", &[]).unwrap_err();
        assert!(error.message.contains("`run` is not a step"), "{error}");
    }

    #[test]
    fn names_are_set_on_a_step_and_taken_off() {
        let made = vec!["guide.mp4".to_owned(), "ball.png".to_owned()];
        let read = step(
            "generate model=video prompt=\"it bounces\" guide={guide.mp4} ref=refs/court.png signal=bounce seed=7",
            &["guide.mp4"],
        )
        .unwrap();
        let set = |names: &[(&str, &str)]| {
            let names: Vec<(String, String)> = names
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect();
            with_names(&read, &names, &made).map(|step| print_step(&step))
        };
        assert_eq!(
            set(&[("seed", "8")]).unwrap(),
            "generate model=video prompt=\"it bounces\" guide={guide.mp4} ref=refs/court.png signal=bounce seed=8"
        );
        // A name that repeats takes every value given, in place of those it had.
        assert_eq!(
            set(&[("ref", "{ball.png}"), ("ref", "refs/net.png"), ("strength", "0.6")]).unwrap(),
            "generate model=video prompt=\"it bounces\" guide={guide.mp4} ref={ball.png} ref=refs/net.png signal=bounce seed=7 strength=0.6"
        );
        // Given nothing, it is taken off.
        assert_eq!(
            set(&[("signal", ""), ("seed", "")]).unwrap(),
            "generate model=video prompt=\"it bounces\" guide={guide.mp4} ref=refs/court.png"
        );
        assert!(set(&[("seed", "seven")]).unwrap_err().contains("not a whole number"));
        assert!(set(&[("model", "")]).unwrap_err().contains("names its model"));
        // What a `render` draws is a word alone, and its names are set the same way.
        let guide = step("render s2 {guide.mp4} pass=depth size=832x480", &[]).unwrap();
        let named = [("pass".to_owned(), "ids".to_owned()), ("rate".to_owned(), "16".to_owned())];
        assert_eq!(
            print_step(&with_names(&guide, &named, &[]).unwrap()),
            "render s2 {guide.mp4} pass=ids size=832x480 rate=16"
        );
    }
}
