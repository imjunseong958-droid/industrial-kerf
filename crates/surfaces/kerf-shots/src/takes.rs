//! A shot's takes held to its guide, to choose among them by.
//!
//! A line that says where a clip goes (`moves=<clip>`) says what the shot's
//! subject is and where it should be on each frame. A take is read against
//! that with no model ([`kerf_engine::placed`]): the subject is looked for
//! in the take's own picture where the guide's sequence has the clip, and
//! followed, and `overlap` says how much of where it is and where it was
//! asked for is the same, from 0 to 1.
//!
//! The editor's graph shows this beside each take and an agent's `look`
//! reads it (`takes`), from the one reading here. It is the place alone: how
//! a take's depth holds wants the depth read back by a model first, which
//! is an agent's `look` with `held`.

use kerf_engine::recipe::takes::Take;
use kerf_engine::recipe::{Recipe, Step};
use kerf_model::Project;
use serde_json::{json, Value};
use std::path::Path;

/// What a shot's takes are held to: the sequence its guide is drawn from,
/// and the clips of it that are its subjects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guide {
    pub sequence: String,
    pub clips: Vec<String>,
}

/// What the takes of media `id` are held to, by its steps: the clips its
/// last `generate` step names with `moves=`, in the sequence that holds
/// them. Steps that name none have nothing to hold a take to, and are
/// refused with that said.
pub fn guide(project: &Project, id: &str) -> Result<Guide, String> {
    let text = crate::graph::written(project, id)?;
    let recipe = Recipe::parse(&text).map_err(|error| error.to_string())?;
    let clips = recipe
        .steps
        .iter()
        .rev()
        .find_map(|step| match step {
            Step::Generate(ask) => Some(ask.moves.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let first = clips.first().ok_or_else(|| {
        "the recipe's model is not told where anything goes (`moves=<clip>`), so nothing says which clip is the shot's subject: wire a clip's way into the model, and its takes are held to it".to_owned()
    })?;
    let at = project
        .locate(first)
        .ok_or_else(|| project.no_clip(first))?;
    Ok(Guide {
        sequence: project.sequences[at.sequence].id.to_string(),
        clips,
    })
}

/// One take of media `id` held to `guide`: for each subject where the take
/// has it beside where it was asked for, and `overlap` at the top, the mean
/// of theirs. A subject the take has nothing of is 0, and says so (`note`).
/// `project` has its media paths resolved.
pub fn read(project: &Project, directory: &Path, id: &str, take: &Take, guide: &Guide) -> Value {
    let rate = project
        .media(id)
        .map_or(24, |media| media.rate.fps().round().max(1.0) as u32);
    let file = directory.join(&take.stored);
    let mut sum = 0.0;
    let clips: Vec<Value> = guide
        .clips
        .iter()
        .map(|clip| {
            match kerf_engine::placed::placed(project, Some(&guide.sequence), clip, &file, rate) {
                Ok(placed) => {
                    sum += placed.overlap;
                    json!({
                        "clip": clip,
                        "overlap": placed.overlap,
                        "off": placed.off,
                        "size": placed.size,
                        "frames": placed.frames,
                        "shown": placed.shown,
                    })
                }
                Err(error) => json!({"clip": clip, "overlap": 0.0, "note": error.to_string()}),
            }
        })
        .collect();
    let overlap = ((sum / guide.clips.len().max(1) as f64) * 1e4).round() / 1e4;
    json!({
        "take": take.number,
        "path": take.stored,
        "chosen": take.chosen,
        "overlap": overlap,
        "clips": clips,
    })
}

/// The takes of media `id`, in the order they were made.
pub fn takes(project: &Project, directory: &Path, id: &str) -> Result<Vec<Take>, String> {
    let media = project
        .media(id)
        .ok_or_else(|| format!("no media `{id}` in this project"))?;
    kerf_engine::recipe::takes(project, directory, media).map_err(|error| error.to_string())
}

/// What going back to a take would write: the recipe as that take was made from it, and
/// each workflow file it named that is no longer there as it was. Nothing is written.
#[derive(Clone, Debug)]
pub struct Restored {
    /// The media's steps as the take had them, as text.
    pub text: String,
    /// Files to write beside the recipe: where each goes, its path from the project's
    /// folder, and what it holds.
    pub files: Vec<(std::path::PathBuf, String, String)>,
}

/// The recipe of media `id` as its take `take` was made from it: its text as that making
/// read it, each seed as that take had it, so a making of it is that take's again where
/// the model gives the same file for the same seed.
///
/// A workflow file the take's lines named comes back with it. Where the project still has
/// it as it was, nothing is written; where it is gone it is written under its name; where
/// the file there now holds something else it is left alone, the take's own is written
/// beside it (`bounce.take3.video.comfy.json`), and the line names that.
///
/// What a take was made from beside its recipe, the sequences its guides were drawn from
/// and its signals, is the project's, and comes back by undoing in the project, not here.
pub fn restored(project: &Project, directory: &Path, id: &str, take: u32) -> Result<Restored, String> {
    use kerf_engine::recipe::text::{self, Change};
    let media = project
        .media(id)
        .ok_or_else(|| format!("no media `{id}` in this project"))?;
    crate::graph::written(project, id)?;
    let kept = kerf_engine::recipe::takes::kept(directory, media, take).ok_or_else(|| {
        format!("take {take} keeps nothing of the recipe it was made from: a take made before takes kept theirs, or one whose working folder is gone, has only what it is")
    })?;
    let mut now = kept.recipe.clone();
    let mut files = Vec::new();
    for (named, held) in &kept.files {
        let (file, stored) =
            kerf_engine::recipe::beside(project, directory, id, named).map_err(|error| error.to_string())?;
        match std::fs::read_to_string(&file) {
            Ok(there) if there == *held => {}
            Err(_) => files.push((file, stored, held.clone())),
            Ok(_) => {
                // The file there now is another's: the take's own goes beside it.
                let (stem, extension) = named
                    .split_once('.')
                    .map_or((named.as_str(), String::new()), |(stem, rest)| (stem, format!(".{rest}")));
                let (own, file, stored) = (1..)
                    .map(|number| match number {
                        1 => format!("{stem}.take{take}{extension}"),
                        more => format!("{stem}.take{take}-{more}{extension}"),
                    })
                    .find_map(|own| {
                        let (file, stored) = kerf_engine::recipe::beside(project, directory, id, &own).ok()?;
                        let free = match std::fs::read_to_string(&file) {
                            Ok(there) => there == *held,
                            Err(_) => true,
                        };
                        free.then_some((own, file, stored))
                    })
                    .ok_or("no name is free for the take's workflow")?;
                if !file.is_file() {
                    files.push((file, stored, held.clone()));
                }
                // Each line that named it names the take's own.
                let steps = Recipe::parse(&now).map_err(|error| error.to_string())?.steps;
                for (line, step) in text::step_lines(&now.clone()).into_iter().zip(steps) {
                    let names_it = matches!(
                        &step,
                        Step::Generate(ask) if ask.workflow == Some(kerf_engine::recipe::model::Source::File(named.clone()))
                    );
                    if names_it {
                        now = text::changed(
                            &now,
                            line,
                            Change::Set { key: kerf_engine::recipe::model::WORKFLOW, nth: 0, value: &own },
                        )
                        .map_err(|error| error.to_string())?;
                    }
                }
            }
        }
    }
    Ok(Restored { text: now, files })
}

/// What the readings say together: the sequence and the clips they are of,
/// and which take holds its guide best, where more than one was read.
pub fn summary(guide: &Guide, readings: &[Value]) -> Value {
    let best = readings
        .iter()
        .filter(|reading| reading["overlap"].as_f64().is_some_and(|overlap| overlap > 0.0))
        .max_by(|a, b| {
            let of = |reading: &Value| reading["overlap"].as_f64().unwrap_or(0.0);
            of(a).total_cmp(&of(b))
        })
        .filter(|_| readings.len() > 1)
        .map(|reading| reading["take"].clone());
    json!({
        "sequence": guide.sequence,
        "clips": guide.clips,
        "best": best,
        "says": "`overlap` is how much of where a take has the subject and where its guide asked for it is the same, as a mean over every frame the guide shows it on: 1 where it is where and as large as asked on every frame, 0 where the take has none of it. Where it was followed on few of the frames shown, it says little.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shot_is_held_to_the_clips_its_line_says_go_somewhere() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        let project = |step: &str| {
            kerf_runtime::parse_document(&format!(
                "kerf 1\nproject name=\"Court\"\nmedia m1 path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480\n  render court {{depth.mp4}} pass=depth\n  {step}\nsequence main rate=24 size=320x180\nsequence court rate=24 size=320x180\n  track V1 video\n    clip ball gen=solid@1 at=0f dur=96f\n",
            ))
            .unwrap()
        };
        assert_eq!(
            guide(&project("generate model=video prompt=\"a ball\" guide={depth.mp4} moves=ball"), "m1").unwrap(),
            Guide { sequence: "court".into(), clips: vec!["ball".into()] }
        );
        // No clip named: nothing says what the subject is, and how to say it is said.
        let none = project("generate model=video prompt=\"a ball\" guide={depth.mp4}");
        assert!(guide(&none, "m1").unwrap_err().contains("moves=<clip>"));
        let elsewhere = project("generate model=video prompt=\"a ball\" moves=crate");
        assert!(guide(&elsewhere, "m1").unwrap_err().contains("crate"));
        // The best of several is said; of one, or of none that has the subject, it is not.
        let held = Guide { sequence: "court".into(), clips: vec!["ball".into()] };
        let reading = |take: u32, overlap: f64| json!({"take": take, "overlap": overlap});
        assert_eq!(summary(&held, &[reading(1, 0.2), reading(2, 0.6), reading(3, 0.4)])["best"], 2);
        assert_eq!(summary(&held, &[reading(1, 0.2)])["best"], Value::Null);
        assert_eq!(summary(&held, &[reading(1, 0.0), reading(2, 0.0)])["best"], Value::Null);
        assert!(takes(&none, dir, "m1").unwrap().is_empty());
    }

    /// A media whose steps are still in a recipe's file holds no take to a guide, and says
    /// how its steps are carried.
    #[test]
    fn an_uncarried_shot_is_held_to_nothing_and_says_how_to_carry_it() {
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Court\"\nmedia m1 path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480 recipe=\"shots/bounce.kerfrecipe\"\nsequence main rate=24 size=320x180\n",
        )
        .unwrap();
        let refused = guide(&project, "m1").unwrap_err();
        assert!(refused.contains("kerf carry m1"), "{refused}");
    }

    /// A person goes back to a take: the steps as it was made from them, and its workflow
    /// beside the one that is there now.
    #[cfg(unix)]
    #[test]
    fn a_takes_steps_come_back_with_the_workflow_they_named() {
        use kerf_engine::recipe::model::Models;
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path();
        std::fs::create_dir_all(dir.join("shots")).unwrap();
        std::fs::write(dir.join("shots/own.comfy.json"), "{\"steps\": 20}").unwrap();
        let file = dir.join("models.json");
        std::fs::write(
            &file,
            r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"]}}"#,
        )
        .unwrap();
        let models = Models::read(&file).unwrap();
        let project = |seed: u32| {
            kerf_runtime::parse_document(&format!(
                "kerf 1\nproject name=\"Shots\"\nmedia m1 path=\"shots/open.json\" rate=24 dur=24f video=320x180\n  generate model=dawn prompt=\"a boat leaves\" seed={seed} workflow=shots/own.comfy.json\nsequence s1 rate=24 size=320x180\n",
            ))
            .unwrap()
        };
        let made = project(7);
        let was = crate::graph::written(&made, "m1").unwrap();
        let planned = kerf_engine::recipe::plan(&made, dir, "m1", &models).unwrap();
        kerf_engine::recipe::make(&made, dir, &planned, &models, &kerf_engine::CancellationToken::new(), |_| {}).unwrap();

        // Made from the steps as they are: going back sets nothing.
        let same = restored(&made, dir, "m1", 1).unwrap();
        assert_eq!(same.text, was);
        assert!(same.files.is_empty());
        assert!(crate::graph::step_lines("m1", &was, &same.text).unwrap().is_empty());
        // The person changes the seed and the workflow.
        let changed = project(9);
        let now = crate::graph::written(&changed, "m1").unwrap();
        std::fs::write(dir.join("shots/own.comfy.json"), "{\"steps\": 30}").unwrap();
        let back = restored(&changed, dir, "m1", 1).unwrap();
        assert_eq!(back.text, was.replace("workflow=shots/own.comfy.json", "workflow=shots/own.take1.comfy.json"));
        assert_eq!(back.files.len(), 1);
        assert_eq!(back.files[0].0, dir.join("shots/own.take1.comfy.json"));
        assert_eq!((back.files[0].1.as_str(), back.files[0].2.as_str()), ("shots/own.take1.comfy.json", "{\"steps\": 20}"));
        // The step lines that set the steps back: the one step, as the take had it.
        assert_eq!(
            crate::graph::step_lines("m1", &now, &back.text).unwrap(),
            vec!["step m1 set 1 generate model=dawn prompt=\"a boat leaves\" seed=7 workflow=shots/own.take1.comfy.json".to_owned()]
        );
        // Nothing is written until it is asked for.
        assert!(!dir.join("shots/own.take1.comfy.json").exists());
        assert_eq!(std::fs::read_to_string(dir.join("shots/own.comfy.json")).unwrap(), "{\"steps\": 30}");
        // A take that kept nothing says so.
        assert!(restored(&changed, dir, "m1", 4).unwrap_err().contains("keeps nothing of the recipe"));
    }
}
