//! What `look` says of the project as a whole and of its folder: with no arguments its
//! document and proposals, the footage of its folder not yet in it, what its media's steps
//! have yet to make, who and what its shots show, its brand, and the models set up; and what a
//! file or a folder of it holds, and the takes a media's steps have made.

use super::{cannot_said, parse, proposals_status};
use crate::{KerfMcpRouter, McpError};
use kerf_model::{Project, Time};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The most files of a folder one `look` reads.
const MAX_FOLDER_FILES: usize = 100;

impl KerfMcpRouter {
    /// `look` with no arguments: the annotated document at the current revision, the
    /// proposals, the footage of the project's folder not yet in the document, what its
    /// media's steps have yet to make, who and what its shots show, its brand, and the models
    /// set up.
    pub(super) fn overview(&self) -> Result<Value, McpError> {
        let (revision, document) = self.runtime.get_document(true)?;
        let state = self.runtime.get_state()?;
        let project = parse(&document).ok();
        let unplaced = project
            .as_ref()
            .map_or_else(Vec::new, |project| self.unplaced(project));
        let mut result = json!({
            "revision": revision,
            "document": document,
            "proposal": proposals_status(&state),
            "unplaced": unplaced,
        });
        if let Some(project) = project {
            let to_make = self.to_make(&project, false);
            if !to_make.is_empty() {
                result["toMake"] = json!(to_make);
            }
        }
        // Who and what the project's shots show and how they look, which a `generate` line
        // names by `cast=` and `style=`, and the project's brand, for titles and captions.
        self.cast_listed(&mut result);
        match crate::brand::Brand::read(&self.dir) {
            Ok(Some(brand)) => result["brand"] = brand.json(),
            Ok(None) => {}
            Err(error) => result["brandError"] = json!(error),
        }
        // The models a recipe's `generate` can name: those the person has set up, each
        // with what their file says it takes. Nothing is run to learn it.
        match kerf_engine::recipe::model::Models::load() {
            Ok(models) => {
                let listed = models_listed(&models);
                if !listed.is_empty() {
                    result["models"] = json!(listed);
                }
            }
            // A file that cannot be read would refuse a making too: the person mends it.
            Err(error) => result["modelsError"] = json!(error.to_string()),
        }
        Ok(result)
    }

    /// Who and what the project's shots show and how they look, as its cast file has them, by
    /// the name a `generate` line gives as `cast=` or `style=`; or why the file does not read.
    fn cast_listed(&self, result: &mut Value) {
        match kerf_engine::recipe::cast::Cast::read(&self.dir) {
            Ok(cast) => {
                use kerf_engine::recipe::cast::Kind;
                for (key, kind) in [("cast", Kind::Cast), ("styles", Kind::Style)] {
                    let listed: Vec<Value> = cast
                        .of(kind)
                        .map(|(name, entry)| {
                            let mut listed = json!({"name": name});
                            if !entry.words.is_empty() {
                                listed["words"] = json!(entry.words);
                            }
                            if !entry.references.is_empty() {
                                listed["refs"] = json!(entry.references);
                            }
                            listed
                        })
                        .collect();
                    if !listed.is_empty() {
                        result[key] = json!(listed);
                    }
                }
            }
            Err(error) => result["castError"] = json!(error.to_string()),
        }
    }

    /// `look` with no arguments adds this: footage in the project's folder not yet in the document.
    fn unplaced(&self, project: &Project) -> Vec<String> {
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let known: Vec<std::path::PathBuf> = resolved
            .media
            .iter()
            .map(|media| std::path::PathBuf::from(&media.path))
            .collect();
        // The project's own folder, and `media` in it, where imports and presets put footage.
        let mut files: Vec<String> = Vec::new();
        for folder in ["", "media"] {
            let mut found: Vec<String> = std::fs::read_dir(self.dir.join(folder))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().is_file() && is_footage(&entry.path()))
                .filter(|entry| !known.contains(&entry.path()))
                .filter_map(|entry| entry.file_name().into_string().ok())
                .map(|name| match folder {
                    "" => name,
                    folder => format!("{folder}/{name}"),
                })
                .collect();
            found.sort();
            files.extend(found);
        }
        files
    }

    /// `look` with no arguments adds this: the media whose steps, under their line or in a recipe
    /// they name, want making, each with why. Looking runs none of them. What would refuse the
    /// making is said here (`cannot`), read as a person's plan is and with nothing run, and so is
    /// what a model has said it does not take (`notes`): steps are mended before a making is
    /// asked for. With those notes is what floats, is sunk, or goes through another in the
    /// sequence a `render` step of the media draws (`crate::stands::note`), which a guide
    /// shows a model and no frame shows an agent.
    /// `waiting` says that `project` is what a proposal not yet accepted would make: a making
    /// is asked of the project, so of such a media only once its proposal is accepted.
    pub(super) fn to_make(&self, project: &Project, waiting: bool) -> Vec<Value> {
        let first = if waiting {
            "once this proposal is accepted, "
        } else {
            ""
        };
        let models = kerf_engine::recipe::model::Models::load();
        // The project with its files where they are, for a mesh's, made when first wanted,
        // and what was found of each part of a sequence, which several media may draw.
        let resolved = std::cell::OnceCell::new();
        let mut read: Vec<(String, Option<String>)> = Vec::new();
        let mut stands = |media: &kerf_model::Media| -> Vec<String> {
            let mut notes = Vec::new();
            for step in &media.steps {
                let kerf_model::making::Step::Render {
                    sequence,
                    from,
                    seconds,
                    ..
                } = step
                else {
                    continue;
                };
                let part = format!("{sequence} {from:?} {seconds:?}");
                let note = match read.iter().find(|(told, _)| *told == part) {
                    Some((_, note)) => note.clone(),
                    None => {
                        let resolved = resolved.get_or_init(|| {
                            let mut resolved = project.clone();
                            kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
                            resolved
                        });
                        let note = crate::stands::note(resolved, sequence, (*from, *seconds));
                        read.push((part, note.clone()));
                        note
                    }
                };
                notes.extend(note.filter(|note| !notes.contains(note)));
            }
            notes
        };
        project
            .media
            .iter()
            .filter_map(|media| {
                let id = media.id.as_str();
                // Steps still in a recipe's file are carried into the document before
                // anything is made of them: nothing is made from a recipe.
                if let Some(recipe) = kerf_engine::recipe::uncarried(media) {
                    return Some(json!({
                        "media": id,
                        "recipe": recipe,
                        "why": "its steps are in a recipe file, and a media is made from the steps under its line",
                        "carry": format!(
                            "`edit` with `carry: [\"{id}\"]` writes them under the media, in the same edit as the script lines that then change them; its takes stay"
                        ),
                    }));
                }
                let why = kerf_engine::recipe::unmade(project, &self.dir, media)?;
                let mut entry = json!({
                    "media": id,
                    "why": why.reason(),
                });
                let plan = models
                    .as_ref()
                    .map_err(ToString::to_string)
                    .and_then(|models| {
                        kerf_engine::recipe::plan(project, &self.dir, id, models)
                            .map_err(|error| error.to_string())
                    });
                let found = stands(media);
                if !found.is_empty() {
                    entry["notes"] = json!(found);
                }
                match plan {
                    Ok(plan) => {
                        if !plan.notes.is_empty() {
                            let notes: Vec<&String> = plan.notes.iter().chain(&found).collect();
                            entry["notes"] = json!(notes);
                        }
                        entry["make"] = json!(format!(
                            "{first}make it with `make`: it runs where the person lets an agent make on its own, and else they are asked and run `kerf make {id}`"
                        ));
                    }
                    Err(cannot) => {
                        entry["cannot"] = json!(cannot_said(cannot, models.as_ref().ok()));
                        entry["make"] = json!(format!(
                            "it cannot be made as it is: mend what `cannot` says first; then, {first}make it with `make`"
                        ));
                    }
                }
                Some(entry)
            })
            .collect()
    }

    /// `look` with `media` adds this for a media its steps make: the takes they have made,
    /// which one the media is, and what has changed since each was made.
    pub(super) fn takes(&self, project: &Project, media: &kerf_model::Media) -> Option<Value> {
        // One whose steps are still in a recipe's file has the takes that file made.
        let recipe = kerf_engine::recipe::uncarried(media);
        if media.steps.is_empty() && recipe.is_none() {
            return None;
        }
        let takes = kerf_engine::recipe::takes(project, &self.dir, media).ok()?;
        // The steps as the document has them, to tell a take made from others.
        let now = kerf_engine::recipe::steps_text(&media.steps);
        Some(json!({
            "made": takes
                .iter()
                .map(|take| {
                    let mut entry = json!({
                        "take": take.number,
                        "path": take.stored,
                        "chosen": take.chosen,
                    });
                    if take.kept {
                        entry["kept"] = json!(true);
                    }
                    if !take.changed.is_empty() {
                        entry["changedSince"] = json!(take.changed);
                    }
                    // The whole line, since a take need not be as long as the one before it.
                    // Which file the media is changes; its steps stay under it.
                    if !take.chosen
                        && let Ok(fields) = self.media_fields(&take.stored) {
                            entry["use"] = json!(match recipe {
                                Some(recipe) => format!("media replace {} {fields} recipe={recipe:?}", media.id),
                                None => format!("media replace {} {fields}", media.id),
                            });
                        }
                    // The steps a take was made from, where they are not the ones the
                    // document has now: each seed as that take had it.
                    if take.kept && recipe.is_none() {
                        let kept = kerf_engine::recipe::takes::kept(&self.dir, media, take.number)
                            .map(|kept| kept.recipe)
                            .filter(|kept| *kept != now)
                            .and_then(|kept| kerf_engine::recipe::document_steps(&kept).ok());
                        if let Some(kept) = kept {
                            entry["steps"] =
                                json!(kept.iter().map(kerf_lang::print_step).collect::<Vec<_>>());
                        }
                    }
                    entry
                })
                .collect::<Vec<_>>(),
            "choose": format!(
                "a take's `use` is the line that, in an edit, has the media be that take; its `steps` are what it was made from where the document has others now, to go back to with `edit` `restoreTake` or to copy a word from; make a new one with `make` (`kerf make {}` where the person is asked)",
                media.id
            ),
        }))
    }

    /// What a media line says of the file `file` of the project after its id: its path, rate,
    /// length in whole frames, and streams, as an import reads a file.
    fn media_fields(&self, file: &str) -> Result<String, McpError> {
        let path = self.dir.join(file);
        let info = kerf_engine::info(&path.to_string_lossy())
            .map_err(|error| McpError::new("UNREADABLE_FILE", format!("{file}: {error}")))?;
        let fps = info.fps.unwrap_or(30.0);
        let seconds = info.duration.unwrap_or(0.0);
        // Whole frames and stereo at 48 kHz, as an import reads a file.
        let mut fields = format!(
            "path={file:?} rate={} dur={}f",
            fps_text(fps),
            frames_of(seconds, fps)
        );
        if let (Some(width), Some(height)) = (info.width, info.height) {
            fields.push_str(&format!(" video={width}x{height}"));
        }
        if info.has_audio == Some(true) {
            fields.push_str(" audio=2ch@48000");
        }
        Ok(fields)
    }

    /// What each footage file of a folder holds, as `look_at_file` says it of one, and in
    /// `script` the lines that add those not yet in the document: a folder of twenty files is
    /// one look and one edit, not twenty of each. A file the document has names its media.
    fn look_at_folder(&self, folder: &str) -> Result<Value, McpError> {
        let project = self.project_at(None).ok().map(|(_, project)| project);
        let mut placed: Vec<(PathBuf, String)> = Vec::new();
        let mut taken: Vec<String> = Vec::new();
        if let Some(project) = &project {
            let mut resolved = project.clone();
            kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
            placed = resolved
                .media
                .iter()
                .map(|media| (PathBuf::from(&media.path), media.id.to_string()))
                .collect();
            taken = project.ids().map(ToString::to_string).collect();
        }
        let inside = folder.trim_end_matches('/');
        let mut names: Vec<String> = std::fs::read_dir(self.dir.join(inside))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_file() && is_footage(&entry.path()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        let more = names.len().saturating_sub(MAX_FOLDER_FILES);
        names.truncate(MAX_FOLDER_FILES);
        let mut files = Vec::new();
        let mut lines = Vec::new();
        for name in names {
            let file = match inside {
                "" | "." => name,
                inside => format!("{inside}/{name}"),
            };
            let path = self.dir.join(&file);
            if let Some((_, id)) = placed.iter().find(|(known, _)| *known == path) {
                files.push(json!({"file": file, "media": id}));
                continue;
            }
            match self.file_facts(&file, &taken) {
                Ok((id, facts)) => {
                    lines.push(facts["add"].as_str().unwrap_or_default().to_owned());
                    taken.push(id);
                    files.push(facts);
                }
                Err(error) => files.push(json!({"file": file, "unreadable": error.message})),
            }
        }
        let mut result = json!({"folder": folder, "files": files});
        if !lines.is_empty() {
            result["script"] = json!(lines.join("\n"));
        }
        if more > 0 {
            result["more"] = json!(more);
        }
        Ok(result)
    }

    /// What a file holds, and the `media add` line that puts it in the document.
    pub(super) fn look_at_file(&self, file: &str) -> Result<Value, McpError> {
        let path = self.dir.join(file);
        if path.is_dir() {
            return self.look_at_folder(file);
        }
        if !path.is_file() {
            return Err(McpError::new(
                "UNKNOWN_FILE",
                format!("no file `{file}` in the project's folder"),
            ));
        }
        let taken: Vec<String> = self
            .project_at(None)
            .map(|(_, project)| project.ids().map(ToString::to_string).collect())
            .unwrap_or_default();
        self.file_facts(file, &taken).map(|(_, facts)| facts)
    }

    /// What the file `file` of the project holds, with the line that adds it under an id
    /// made of its name that none of `taken` has, and that id.
    fn file_facts(&self, file: &str, taken: &[String]) -> Result<(String, Value), McpError> {
        let path = self.dir.join(file);
        let fields = self.media_fields(file)?;
        let info = kerf_engine::info(&path.to_string_lossy())
            .map_err(|error| McpError::new("UNREADABLE_FILE", format!("{file}: {error}")))?;
        let fps = info.fps.unwrap_or(30.0);
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("media");
        let named: String = stem
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        // `shot`, or `shot_2` where the document or the folder has a `shot` already.
        let id = std::iter::once(named.clone())
            .chain((2..).map(|count| format!("{named}_{count}")))
            .find(|id| !taken.contains(id))
            .unwrap_or(named);
        let seconds = info.duration.unwrap_or(0.0);
        let line = format!("media add {id} {fields}");
        let facts = json!({
            "file": file,
            "seconds": seconds,
            "fps": fps,
            "width": info.width,
            "height": info.height,
            "add": line,
        });
        Ok((id, facts))
    }
}

/// The models set up, as `look` lists them: each by the name a `generate` line gives it,
/// with what the person's models file says it takes. `size` is written as a line writes it,
/// and `guide` is the `render` line that draws a guide for it, with no more than it reads.
/// The models a recipe's `generate` can name, as `look` lists them: each
/// with what its owner's file says it takes, and the `render` line that draws
/// a guide for it where it follows one.
pub fn models_listed(models: &kerf_engine::recipe::model::Models) -> Vec<Value> {
    models
        .names()
        .map(|name| {
            let mut listed = json!({"name": name});
            let Some(takes) = models.takes(name) else {
                return listed;
            };
            if let Some(about) = &takes.about {
                listed["about"] = json!(about);
            }
            let size = takes
                .size
                .map(|(width, height)| format!("{width}x{height}"));
            let rate = takes.rate.map(fps_text);
            if let Some(size) = &size {
                listed["size"] = json!(size);
            }
            if let Some(rate) = takes.rate {
                listed["rate"] = json!(rate);
            }
            if let Some(seconds) = takes.seconds {
                listed["seconds"] = json!(seconds);
            }
            if let Some(reads) = &takes.reads {
                listed["reads"] = json!(reads);
            }
            if !takes.more.is_empty() {
                listed["more"] = json!(takes.more);
            }
            for (name, value) in &takes.other {
                // What the file says under a name of its own never replaces one of these.
                if listed.get(name).is_none() && name != "guide" {
                    listed[name] = value.clone();
                }
            }
            if takes.follows_a_guide() == Some(true) && (size.is_some() || rate.is_some()) {
                let mut line = "render <sequence> {guide.mp4}".to_owned();
                if let Some(size) = size {
                    line.push_str(&format!(" size={size}"));
                }
                if let Some(rate) = rate {
                    line.push_str(&format!(" rate={rate}"));
                }
                listed["guide"] = json!(line);
            }
            listed
        })
        .collect()
}

/// Whether a file's extension says it is footage: what an import takes.
fn is_footage(path: &std::path::Path) -> bool {
    const FOOTAGE: [&str; 16] = [
        "mp4", "mov", "m4v", "mkv", "webm", "avi", "mxf", "mts", "wav", "mp3", "m4a", "aac",
        "flac", "png", "jpg", "jpeg",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| FOOTAGE.contains(&extension.to_lowercase().as_str()))
}

/// How many frames a file `seconds` long has at `fps`, as a making and an import count them
/// (`kerf-runtime`'s `media_from_probe`): its length to the nearest frame of its rate, and
/// never none. A container says a length a hair under its frames' as often as not, 4.062012 s
/// for 65 frames at 16 a second, and counted down that is a frame short of the file.
fn frames_of(seconds: f64, fps: f64) -> i64 {
    let counted = kerf_model::Rate::from_fps(fps)
        .ok()
        .zip(Time::from_seconds(seconds))
        .map(|(rate, length)| rate.frames_round(length));
    counted
        .unwrap_or_else(|| (seconds * fps).round() as i64)
        .max(1)
}

/// A frame rate as a document writes it.
fn fps_text(fps: f64) -> String {
    let whole = fps.round();
    if (fps - whole).abs() < 1e-3 {
        format!("{whole}")
    } else {
        format!("{:.3}", fps)
    }
}
