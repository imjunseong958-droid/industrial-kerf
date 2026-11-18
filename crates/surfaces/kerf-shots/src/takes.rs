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

/// What the takes of media `id` are held to, by its recipe: the clips its
/// last `generate` line names with `moves=`, in the sequence that holds
/// them. A recipe that names none has nothing to hold a take to, and is
/// refused with that said.
pub fn guide(project: &Project, directory: &Path, id: &str) -> Result<Guide, String> {
    let (_, text) = crate::graph::recipe(project, directory, id)?;
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
        std::fs::create_dir_all(dir.join("shots")).unwrap();
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Court\"\nmedia m1 path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480 recipe=\"shots/bounce.kerfrecipe\"\nsequence main rate=24 size=320x180\nsequence court rate=24 size=320x180\n  track V1 video\n    clip ball gen=solid@1 at=0f dur=96f\n",
        )
        .unwrap();
        let recipe = |line: &str| {
            std::fs::write(
                dir.join("shots/bounce.kerfrecipe"),
                format!("kerf-recipe 1\nrender court {{depth.mp4}} pass=depth\n{line}\n"),
            )
            .unwrap();
        };
        recipe("generate model=video prompt=\"a ball\" guide={depth.mp4} moves=ball");
        assert_eq!(
            guide(&project, dir, "m1").unwrap(),
            Guide { sequence: "court".into(), clips: vec!["ball".into()] }
        );
        // No clip named: nothing says what the subject is, and how to say it is said.
        recipe("generate model=video prompt=\"a ball\" guide={depth.mp4}");
        assert!(guide(&project, dir, "m1").unwrap_err().contains("moves=<clip>"));
        recipe("generate model=video prompt=\"a ball\" moves=crate");
        assert!(guide(&project, dir, "m1").unwrap_err().contains("crate"));
        // The best of several is said; of one, or of none that has the subject, it is not.
        let held = Guide { sequence: "court".into(), clips: vec!["ball".into()] };
        let reading = |take: u32, overlap: f64| json!({"take": take, "overlap": overlap});
        assert_eq!(summary(&held, &[reading(1, 0.2), reading(2, 0.6), reading(3, 0.4)])["best"], 2);
        assert_eq!(summary(&held, &[reading(1, 0.2)])["best"], Value::Null);
        assert_eq!(summary(&held, &[reading(1, 0.0), reading(2, 0.0)])["best"], Value::Null);
        assert!(takes(&project, dir, "m1").unwrap().is_empty());
    }
}
