//! A recipe: the text file that says how another program makes a media
//! file, and the making of it. A media line names its recipe
//! (`recipe="shots/open.kerfrecipe"`); `kerf make` runs it.
//!
//! ```text
//! kerf-recipe 1
//! # The opening shot, drawn from its scene.
//! run vork render open.scene.json --frame-end 95 --out {out}
//! ```
//!
//! `run` is a program and its arguments, quoted where they hold spaces.
//! `{out}` stands for the file to write. The program runs in the recipe's
//! folder, as a supervised sidecar and never through a shell, so nothing in
//! an argument is expanded or run. It writes to a file beside the media's,
//! which takes the media's place only when the program has ended well and
//! left something there: a failed run leaves the media as it was.
//!
//! A recipe is a command someone wrote. Kerf runs one only when a person
//! asks for that media to be made, never on opening or drawing a project.

use crate::error::{EngineError, Result};
use crate::sidecar::{self, CancellationToken};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The first line of a recipe.
const HEADER: &str = "kerf-recipe 1";

/// What stands for the file to write, in an argument.
const OUT: &str = "{out}";

/// A program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub program: String,
    pub arguments: Vec<String>,
}

impl Recipe {
    /// Read a recipe's text. What is wrong is said with its line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));
        match lines.next() {
            Some((_, HEADER)) => {}
            _ => return Err(EngineError::msg(format!("a recipe starts with `{HEADER}`"))),
        }
        let Some((number, line)) = lines.next() else {
            return Err(EngineError::msg("the recipe has no `run` line"));
        };
        let mut words = words(line)
            .map_err(|what| EngineError::msg(format!("line {number} of the recipe: {what}")))?;
        if words.first().map(String::as_str) != Some("run") || words.len() < 2 {
            return Err(EngineError::msg(format!(
                "line {number} of the recipe: expected `run <program> <arguments>`"
            )));
        }
        if let Some((number, _)) = lines.next() {
            return Err(EngineError::msg(format!(
                "line {number} of the recipe: a recipe runs one program"
            )));
        }
        let arguments = words.split_off(2);
        if !arguments.iter().any(|argument| argument.contains(OUT)) {
            return Err(EngineError::msg(format!(
                "the recipe's `run` line names no `{OUT}`, so nothing says where its file goes"
            )));
        }
        Ok(Self {
            program: words.swap_remove(1),
            arguments,
        })
    }

    /// The arguments with `out` where `{out}` stands.
    pub fn arguments_for(&self, out: &Path) -> Vec<String> {
        let out = out.to_string_lossy();
        self.arguments
            .iter()
            .map(|argument| argument.replace(OUT, &out))
            .collect()
    }

    /// The command as a person reads it, with `out` where `{out}` stands.
    pub fn shown(&self, out: &Path) -> String {
        std::iter::once(self.program.clone())
            .chain(self.arguments_for(out))
            .map(|word| {
                if word.is_empty() || word.contains(char::is_whitespace) {
                    format!("\"{word}\"")
                } else {
                    word
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A line's words: split at spaces, with `"…"` keeping what it holds as
/// one word, `\"` and `\\` inside it standing for themselves.
fn words(line: &str) -> std::result::Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => {
                started = true;
                loop {
                    match characters.next() {
                        Some('"') => break,
                        Some('\\') => match characters.next() {
                            Some(escaped @ ('"' | '\\')) => word.push(escaped),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => return Err("a quote is not closed".to_owned()),
                        },
                        Some(other) => word.push(other),
                        None => return Err("a quote is not closed".to_owned()),
                    }
                }
            }
            space if space.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            other => {
                started = true;
                word.push(other);
            }
        }
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

/// Where a recipe's program writes before its file takes the media's
/// place: beside it, with its extension, so the program can tell the format
/// from the name.
pub fn making_path(media: &Path) -> PathBuf {
    let name = media
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    media.with_file_name(format!(".making-{name}"))
}

/// Run the recipe at `recipe` to make the file at `media`. Each line the
/// program writes to stderr goes to `on_line`. The media's file is replaced
/// only when the program ends well and has written something.
pub fn make(
    recipe: &Path,
    media: &Path,
    cancellation: &CancellationToken,
    on_line: impl FnMut(&str),
) -> Result<()> {
    let text = std::fs::read_to_string(recipe).map_err(|error| {
        EngineError::msg(format!(
            "cannot read the recipe {}: {error}",
            recipe.display()
        ))
    })?;
    let parsed = Recipe::parse(&text)?;
    let making = making_path(media);
    if let Some(folder) = media
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder)?;
    }
    // What an earlier run left must not pass for this one's.
    let _ = std::fs::remove_file(&making);
    let mut command = Command::new(&parsed.program);
    command.args(parsed.arguments_for(&making));
    if let Some(folder) = recipe
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        command.current_dir(folder);
    }
    let finished = sidecar::run(command, cancellation, on_line);
    let made = match finished {
        Ok(finished) if finished.status.success() => match std::fs::metadata(&making) {
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Ok(()),
            _ => Err(EngineError::msg(format!(
                "{} ended well and wrote nothing to {}",
                parsed.program,
                making.display()
            ))),
        },
        Ok(finished) => Err(EngineError::msg(format!(
            "{} failed ({}): {}",
            parsed.program,
            finished.status,
            finished.stderr.trim()
        ))),
        Err(error) => Err(error),
    };
    if let Err(error) = made {
        let _ = std::fs::remove_file(&making);
        return Err(error);
    }
    std::fs::rename(&making, media)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_is_a_program_and_its_arguments() {
        let recipe = Recipe::parse(
            "kerf-recipe 1\n\n# The opening shot.\nrun vork render \"open scene.json\" --out {out} --note \"a \\\"quoted\\\" word\"\n",
        )
        .unwrap();
        assert_eq!(recipe.program, "vork");
        assert_eq!(
            recipe.arguments_for(Path::new("shots/open.mp4")),
            [
                "render",
                "open scene.json",
                "--out",
                "shots/open.mp4",
                "--note",
                "a \"quoted\" word"
            ]
        );
        assert_eq!(
            recipe.shown(Path::new("o.mp4")),
            "vork render \"open scene.json\" --out o.mp4 --note \"a \"quoted\" word\""
        );
        // Nothing is expanded: a shell's words are words.
        let plain = Recipe::parse("kerf-recipe 1\nrun echo $HOME;ls {out}").unwrap();
        assert_eq!(plain.arguments, ["$HOME;ls", "{out}"]);
    }

    #[test]
    fn what_is_wrong_with_a_recipe_is_said() {
        let wrong = |text: &str| Recipe::parse(text).unwrap_err().to_string();
        assert!(wrong("run vork {out}").contains("starts with `kerf-recipe 1`"));
        assert!(wrong("kerf-recipe 1\n# nothing\n").contains("no `run` line"));
        assert!(wrong("kerf-recipe 1\nvork {out}").contains("line 2"));
        assert!(wrong("kerf-recipe 1\nrun vork \"open {out}").contains("quote is not closed"));
        assert!(wrong("kerf-recipe 1\nrun vork render").contains("names no `{out}`"));
        assert!(wrong("kerf-recipe 1\nrun a {out}\nrun b {out}").contains("line 3"));
    }

    #[cfg(unix)]
    #[test]
    fn making_replaces_the_file_only_when_the_program_ends_well() {
        let folder = tempfile::tempdir().unwrap();
        let path = |name: &str| folder.path().join(name);
        std::fs::write(path("scene.txt"), "new").unwrap();
        std::fs::create_dir(path("shots")).unwrap();
        std::fs::write(path("shots/open.txt"), "old").unwrap();
        let recipe = |name: &str, run: &str| {
            std::fs::write(path(name), format!("kerf-recipe 1\nrun {run}\n")).unwrap();
            path(name)
        };
        let token = CancellationToken::new();
        let media = path("shots/open.txt");
        // It runs in the recipe's folder, so `scene.txt` is found beside it.
        make(
            &recipe("good.kerfrecipe", "cp scene.txt {out}"),
            &media,
            &token,
            |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&media).unwrap(), "new");
        assert!(!making_path(&media).exists());

        std::fs::write(&media, "kept").unwrap();
        let failed = make(
            &recipe("bad.kerfrecipe", "cp missing.txt {out}"),
            &media,
            &token,
            |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(failed.contains("cp failed"), "{failed}");
        let nothing = make(
            &recipe("idle.kerfrecipe", "true {out}"),
            &media,
            &token,
            |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(nothing.contains("wrote nothing"), "{nothing}");
        let absent = make(
            &recipe("absent.kerfrecipe", "no-such-program-of-kerf {out}"),
            &media,
            &token,
            |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(absent.contains("could not start"), "{absent}");
        assert_eq!(std::fs::read_to_string(&media).unwrap(), "kept");
        assert!(!making_path(&media).exists());
    }
}
