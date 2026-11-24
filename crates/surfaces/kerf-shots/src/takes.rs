//! A shot's takes held to its guide, to choose among them by.
//!
//! A line that says where a clip goes (`moves=<clip>`) says what the shot's
//! subject is and where it should be on each frame. A take is read against
//! that with no model ([`kerf_engine::analysis::placed`]): the subject is looked for
//! in the take's own picture where the guide's sequence has the clip, and
//! followed, and `overlap` says how much of where it is and where it was
//! asked for is the same, from 0 to 1.
//!
//! The editor's graph shows this beside each take and an agent's `look`
//! reads it (`takes`), from the one reading here. It is the place alone: how
//! a take's depth holds wants the depth read back by a model first, which
//! is an agent's `look` with `held`.

use crate::ShotError;
use kerf_engine::recipe::takes::Take;
use kerf_engine::recipe::{Recipe, Step};
use kerf_model::Project;
use serde::{Deserialize, Deserializer, Serialize};
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
pub fn guide(project: &Project, id: &str) -> Result<Guide, ShotError> {
    let text = crate::graph::written(project, id)?;
    let recipe = Recipe::parse(&text).map_err(ShotError::from)?;
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
        "the shot's model is not told where anything goes (`moves=<clip>`), so nothing says which clip is the shot's subject: wire a clip's way into the model, and its takes are held to it".to_owned()
    })?;
    let at = project
        .locate(first)
        .ok_or_else(|| ShotError::NoClip(project.no_clip(first)))?;
    Ok(Guide {
        sequence: project.sequences[at.sequence].id.to_string(),
        clips,
    })
}

/// A number that was written as `null` because it was not finite, read back as NaN so
/// that it writes as `null` again.
fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(f64::NAN))
}

/// One subject of a guide as a take has it: how much of where the take has it and where
/// the guide asked for it is the same (`overlap`, 0 to 1), how far off its middle is in
/// frame heights and how large it is beside the guide's, and on how many of the frames
/// the guide shows it (`shown`) it was followed (`frames`). A subject the take has
/// nothing of is 0 with why (`note`), and says none of the rest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipReading {
    pub clip: String,
    #[serde(deserialize_with = "number")]
    pub overlap: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One take held to its guide: its number, file and whether the media is it; each
/// subject's reading; and `overlap`, the mean of theirs, to four places.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reading {
    pub take: u32,
    pub path: String,
    pub chosen: bool,
    #[serde(deserialize_with = "number")]
    pub overlap: f64,
    pub clips: Vec<ClipReading>,
}

/// One take of media `id` held to `guide`: for each subject where the take
/// has it beside where it was asked for, and `overlap` at the top, the mean
/// of theirs. A subject the take has nothing of is 0, and says so (`note`).
/// `project` has its media paths resolved.
pub fn read(project: &Project, directory: &Path, id: &str, take: &Take, guide: &Guide) -> Reading {
    let rate = project
        .media(id)
        .map_or(24, |media| media.rate.fps().round().max(1.0) as u32);
    let file = directory.join(&take.stored);
    let mut sum = 0.0;
    let clips: Vec<ClipReading> = guide
        .clips
        .iter()
        .map(|clip| {
            match kerf_engine::analysis::placed::placed(
                project,
                Some(&guide.sequence),
                clip,
                &file,
                rate,
            ) {
                Ok(placed) => {
                    sum += placed.overlap;
                    ClipReading {
                        clip: clip.clone(),
                        overlap: placed.overlap,
                        off: Some(placed.off),
                        size: Some(placed.size),
                        frames: Some(placed.frames),
                        shown: Some(placed.shown),
                        note: None,
                    }
                }
                Err(error) => ClipReading {
                    clip: clip.clone(),
                    note: Some(error.to_string()),
                    ..ClipReading::default()
                },
            }
        })
        .collect();
    let overlap = ((sum / guide.clips.len().max(1) as f64) * 1e4).round() / 1e4;
    Reading {
        take: take.number,
        path: take.stored.clone(),
        chosen: take.chosen,
        overlap,
        clips,
    }
}

/// The takes of media `id`, in the order they were made.
pub fn takes(project: &Project, directory: &Path, id: &str) -> Result<Vec<Take>, ShotError> {
    let media = project
        .media(id)
        .ok_or_else(|| ShotError::NoMedia(id.to_owned()))?;
    kerf_engine::recipe::takes(project, directory, media).map_err(ShotError::from)
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
pub fn restored(
    project: &Project,
    directory: &Path,
    id: &str,
    take: u32,
) -> Result<Restored, ShotError> {
    use kerf_engine::recipe::text::{self, Change};
    let media = project
        .media(id)
        .ok_or_else(|| ShotError::NoMedia(id.to_owned()))?;
    crate::graph::written(project, id)?;
    let kept = kerf_engine::recipe::takes::kept(directory, media, take).ok_or_else(|| {
        format!("take {take} keeps nothing of the steps it was made from: a take made before takes kept theirs, or one whose working folder is gone, has only what it is")
    })?;
    let mut now = kept.recipe.clone();
    let mut files = Vec::new();
    for (named, held) in &kept.files {
        let (file, stored) =
            kerf_engine::recipe::beside(project, directory, id, named).map_err(ShotError::from)?;
        match std::fs::read_to_string(&file) {
            Ok(there) if there == *held => {}
            Err(_) => files.push((file, stored, held.clone())),
            Ok(_) => {
                // The file there now is another's: the take's own goes beside it.
                let (stem, extension) = named
                    .split_once('.')
                    .map_or((named.as_str(), String::new()), |(stem, rest)| {
                        (stem, format!(".{rest}"))
                    });
                let (own, file, stored) = (1..)
                    .map(|number| match number {
                        1 => format!("{stem}.take{take}{extension}"),
                        more => format!("{stem}.take{take}-{more}{extension}"),
                    })
                    .find_map(|own| {
                        let (file, stored) =
                            kerf_engine::recipe::beside(project, directory, id, &own).ok()?;
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
                let steps = Recipe::parse(&now).map_err(ShotError::from)?.steps;
                for (line, step) in text::step_lines(&now.clone()).into_iter().zip(steps) {
                    let names_it = matches!(
                        &step,
                        Step::Generate(ask) if ask.workflow == Some(kerf_engine::recipe::model::Source::File(named.clone()))
                    );
                    if names_it {
                        now = text::changed(
                            &now,
                            line,
                            Change::Set {
                                key: kerf_engine::recipe::model::WORKFLOW,
                                nth: 0,
                                value: &own,
                            },
                        )
                        .map_err(ShotError::from)?;
                    }
                }
            }
        }
    }
    Ok(Restored { text: now, files })
}

/// What a shot's readings say together: the sequence and the clips they are of, the take
/// that holds its guide best (`null` where fewer than two were read or none holds it at
/// all), and what `overlap` means.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub sequence: String,
    pub clips: Vec<String>,
    pub best: Option<u32>,
    pub says: String,
}

/// What the readings say together: the sequence and the clips they are of,
/// and which take holds its guide best, where more than one was read.
pub fn summary(guide: &Guide, readings: &[Reading]) -> Summary {
    let best = readings
        .iter()
        // A mean that is not a number holds nothing.
        .filter(|reading| reading.overlap.is_finite() && reading.overlap > 0.0)
        .max_by(|a, b| a.overlap.total_cmp(&b.overlap))
        .filter(|_| readings.len() > 1)
        .map(|reading| reading.take);
    Summary {
        sequence: guide.sequence.clone(),
        clips: guide.clips.clone(),
        best,
        says: "`overlap` is how much of where a take has the subject and where its guide asked for it is the same, as a mean over every frame the guide shows it on: 1 where it is where and as large as asked on every frame, 0 where the take has none of it. Where it was followed on few of the frames shown, it says little.".to_owned(),
    }
}
