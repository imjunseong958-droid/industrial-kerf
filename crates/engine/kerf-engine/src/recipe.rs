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

/// What making a media runs and writes, for a person to see before it is
/// run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The recipe's file.
    pub recipe: PathBuf,
    /// The media's file, which the run replaces.
    pub file: PathBuf,
    /// The media's path as the document has it.
    pub stored: String,
    /// The command, as [`Recipe::shown`] gives it.
    pub command: String,
}

/// `path`, which a project names, as a file inside `directory`: a recipe is
/// a command to run and a media's file is written over, so neither may be
/// somewhere else.
fn inside_project(directory: &Path, path: &str, what: &str) -> Result<PathBuf> {
    let given = Path::new(path);
    let escapes = given
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir));
    if given.is_absolute() || escapes {
        return Err(EngineError::msg(format!(
            "the {what} `{path}` is not inside the project, so it is left alone"
        )));
    }
    Ok(directory.join(given))
}

/// What making media `id` of `project`, kept in `directory`, would run:
/// its recipe read, and nothing run.
pub fn plan(project: &kerf_model::Project, directory: &Path, id: &str) -> Result<Plan> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let recipe = media.recipe.as_deref().ok_or_else(|| {
        EngineError::msg(format!(
            "media `{id}` names no recipe; add `recipe=\"<file>\"` to its line with `media replace`"
        ))
    })?;
    let recipe = inside_project(directory, recipe, "recipe")?;
    let file = inside_project(directory, &media.path, "media file")?;
    let text = std::fs::read_to_string(&recipe).map_err(|error| {
        EngineError::msg(format!(
            "cannot read the recipe {}: {error}",
            recipe.display()
        ))
    })?;
    let command = Recipe::parse(&text)?.shown(&making_path(&file));
    Ok(Plan {
        recipe,
        file,
        stored: media.path.clone(),
        command,
    })
}

/// Why a media that names a recipe wants making.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unmade {
    /// Its file is not there.
    Missing,
    /// Its recipe was changed after its file was written.
    Stale,
}

impl Unmade {
    /// The reason, as a person or an agent is told it.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Missing => "its file is not there yet",
            Self::Stale => "its recipe changed after its file was made",
        }
    }
}

/// Whether the file at `file` wants making from the recipe at `recipe`:
/// it is missing, or older than the recipe. `None` when it is made, and
/// when the recipe cannot be read, since then nothing could make it.
pub fn unmade(recipe: &Path, file: &Path) -> Option<Unmade> {
    let written = std::fs::metadata(recipe).ok()?.modified().ok()?;
    match std::fs::metadata(file).and_then(|made| made.modified()) {
        Err(_) => Some(Unmade::Missing),
        Ok(made) if made < written => Some(Unmade::Stale),
        Ok(_) => None,
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

    #[test]
    fn a_file_wants_making_when_it_is_missing_or_older_than_its_recipe() {
        let folder = tempfile::tempdir().unwrap();
        let (recipe, file) = (
            folder.path().join("r.kerfrecipe"),
            folder.path().join("f.mp4"),
        );
        assert_eq!(
            unmade(&recipe, &file),
            None,
            "no recipe, nothing to make it"
        );
        std::fs::write(&recipe, "kerf-recipe 1\nrun a {out}\n").unwrap();
        assert_eq!(unmade(&recipe, &file), Some(Unmade::Missing));
        std::fs::write(&file, "frames").unwrap();
        let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let set = |path: &Path, time| {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap()
        };
        set(&recipe, earlier);
        assert_eq!(unmade(&recipe, &file), None);
        set(&file, earlier - std::time::Duration::from_secs(3600));
        assert_eq!(unmade(&recipe, &file), Some(Unmade::Stale));
    }

    #[test]
    fn a_plan_is_only_of_a_recipe_and_a_file_inside_the_project() {
        let folder = tempfile::tempdir().unwrap();
        let directory = folder.path();
        std::fs::create_dir(directory.join("shots")).unwrap();
        std::fs::write(
            directory.join("shots/open.kerfrecipe"),
            "kerf-recipe 1\nrun vork render open.json --out {out}\n",
        )
        .unwrap();
        let media = |id: &str, path: &str, recipe: Option<&str>| kerf_model::Media {
            id: kerf_model::Id::new(id).unwrap(),
            path: path.to_owned(),
            rate: kerf_model::Rate::FPS_24,
            drop_frame: false,
            start: kerf_model::Time::ZERO,
            duration: kerf_model::Time::from_seconds(1.0).unwrap(),
            video: None,
            audio: None,
            reel: None,
            recipe: recipe.map(str::to_owned),
        };
        let project = kerf_model::Project {
            media: vec![
                media("m1", "shots/open.mp4", Some("shots/open.kerfrecipe")),
                media("m2", "a.mov", None),
                media("m3", "shots/b.mp4", Some("../elsewhere/run.kerfrecipe")),
                media("m4", "/tmp/c.mp4", Some("shots/open.kerfrecipe")),
                media("m5", "shots/d.mp4", Some("shots/gone.kerfrecipe")),
            ],
            ..kerf_model::Project::default()
        };
        let made = plan(&project, directory, "m1").unwrap();
        assert_eq!(made.recipe, directory.join("shots/open.kerfrecipe"));
        assert_eq!(made.file, directory.join("shots/open.mp4"));
        assert_eq!(made.stored, "shots/open.mp4");
        assert_eq!(
            made.command,
            format!(
                "vork render open.json --out {}",
                directory.join("shots/.making-open.mp4").display()
            )
        );
        let wrong = |id: &str| plan(&project, directory, id).unwrap_err().to_string();
        assert!(wrong("m9").contains("no media `m9`"));
        assert!(wrong("m2").contains("names no recipe"));
        assert!(wrong("m3").contains("recipe `../elsewhere/run.kerfrecipe` is not inside"));
        assert!(wrong("m4").contains("media file `/tmp/c.mp4` is not inside"));
        assert!(wrong("m5").contains("cannot read the recipe"));
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
