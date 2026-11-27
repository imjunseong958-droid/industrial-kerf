//! What `look` reads of a shot, a media its steps make: the graph the editor shows of it, its
//! takes held to their guide, and a signal's keys from a sound; and what an edit's
//! `restoreTake` writes of its steps.

use super::tools::{EditInput, HeldInput, PlacedInput, SignalInput};
use super::{no_model_said, program, resolve_project_source_path};
use crate::{KerfMcpRouter, McpCode, McpError};
use kerf_model::Project;
use kerf_shots::ShotError;
use serde_json::{json, Value};

impl KerfMcpRouter {
    /// What cannot be read of a shot, with where its media is when a proposal holds it.
    fn no_shot(&self, media: &str, why: ShotError) -> McpError {
        let hint = match &why {
            ShotError::NoMedia(_) => self.in_pending(media, "proposal"),
            _ => String::new(),
        };
        McpError::new(McpCode::InvalidArguments, format!("{why}{hint}"))
    }

    /// `look` with `graph`: a media's recipe as the graph the editor shows of it, the same
    /// reading (`kerf_shots::graph`), with how each thing a person does there is done here.
    pub(super) fn shot_graph(&self, project: &Project, media: &str) -> Result<Value, McpError> {
        let (models, models_error) = match kerf_engine::recipe::model::Models::load() {
            Ok(models) => (models, None),
            Err(error) => (Default::default(), Some(error.to_string())),
        };
        let graph = kerf_shots::graph::shown(project, &self.dir, media, &models)
            .map_err(|why| self.no_shot(media, why))?;
        // An agent reads it as JSON, with what is said here for an agent alone.
        let mut shown = json!(graph);
        if let Some(model) = graph.model_not_set_up.as_deref()
            && models_error.is_none()
        {
            shown["cannot"] = json!(no_model_said(model, &models));
        }
        if let Some(error) = models_error {
            shown["modelsError"] = json!(error);
        }
        // A person chooses among the takes in the graph's last node: here, by each one's line.
        if let Some(takes) = project
            .media(media)
            .and_then(|media| self.takes(project, media))
        {
            shown["takes"] = takes;
        }
        // Steps the document holds are counted as the document has them: a node's `line` is
        // its step's number, and `text` the steps alone, a line each.
        for node in shown["nodes"].as_array_mut().into_iter().flatten() {
            if let Some(line) = node["line"].as_u64() {
                node["line"] = json!(line.saturating_sub(1));
            }
        }
        if let Some(steps) = shown["text"]
            .as_str()
            .and_then(|text| text.split_once('\n'))
        {
            shown["text"] = json!(steps.1);
        }
        if let Some(shown) = shown.as_object_mut() {
            shown.remove("recipe");
            shown.remove("document");
        }
        shown["change"] = json!(format!(
            "a node's `line` is its step's number, as the document has the steps under `{media}` and as `text` lists them. \
             Change one in an `edit` script: `step {media} set <n> <name>=<value>` sets a prompt, a seed, a knob, and an edge too, \
             since wiring a node in is the name its reader gives it (`signal=`, `ref=`, `guide=`, `moves=`; a name given more than \
             once takes every value, and `<name>=\"\"` lets it go); `step {media} add [<n>] <step>` adds one and \
             `step {media} remove <n>` takes one out. A signal's curve is its clip's `value`. Make it with `make`."
        ));
        Ok(shown)
    }

    /// `look` with `takes`: each take of a shot held to its guide by where its subjects are,
    /// the reading the editor's graph shows beside each take (`kerf_shots::takes`).
    pub(super) fn shot_takes(&self, project: &Project, media: &str) -> Result<Value, McpError> {
        let invalid = |why: ShotError| self.no_shot(media, why);
        let guide = kerf_shots::takes::guide(project, media).map_err(invalid)?;
        let takes = kerf_shots::takes::takes(project, &self.dir, media).map_err(invalid)?;
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let readings: Vec<kerf_shots::takes::Reading> = takes
            .iter()
            .map(|take| kerf_shots::takes::read(&resolved, &self.dir, media, take, &guide))
            .collect();
        let mut result = json!(kerf_shots::takes::summary(&guide, &readings));
        result["media"] = json!(media);
        result["takes"] = json!(readings);
        result["choose"] = json!(
            "have the media be a take with its `use` line, which `look` with `media` or `graph` gives; \
             how a take's depth holds is `look` with `held`, once a model has read the depth back"
        );
        Ok(result)
    }

    /// What an edit's `restoreTake` makes of the steps of the media it names: lines of the
    /// edit's own script, which set them as the take had them. Nothing is written here.
    pub(super) fn shot_changes(
        &self,
        project: &Project,
        input: &EditInput,
    ) -> Result<Vec<String>, McpError> {
        let invalid = |why: String| McpError::new(McpCode::InvalidSteps, why);
        let Some(restore) = &input.restore_take else {
            return Ok(Vec::new());
        };
        let was = kerf_shots::graph::written(project, &restore.media).map_err(|why| {
            invalid(format!(
                "{why}{}",
                self.in_pending(&restore.media, "extend")
            ))
        })?;
        let now = kerf_shots::takes::restored(project, &self.dir, &restore.media, restore.take)
            .map_err(|why| {
                invalid(format!(
                    "{why}{}",
                    self.in_pending(&restore.media, "extend")
                ))
            })?;
        kerf_shots::graph::step_lines(&restore.media, &was, &now)
            .map_err(|why| invalid(format!("{}'s steps: {why}", restore.media)))
    }

    /// The keys of a signal clip from the onsets of the sound a clip plays.
    pub(super) fn signal_from_sound(
        &self,
        project: &Project,
        revision: u64,
        input: &SignalInput,
    ) -> Result<Value, McpError> {
        // The reading a person asks for in the signal's card of a shot's graph.
        let read =
            kerf_shots::signals::from_sound(project, &self.dir, &input.of, &input.clip, input.hold)
                .map_err(|why| {
                    let hint = match &why {
                        ShotError::NoClip(_) => self.any_pending("proposal"),
                        _ => String::new(),
                    };
                    McpError::new(McpCode::InvalidArguments, format!("{why}{hint}"))
                })?;
        Ok(json!({
            "revision": revision,
            "clip": input.clip,
            "of": input.of,
            "hold": read.hold,
            "onsets": read.placed,
            "outside": read.outside,
            "script": read.script(&input.clip),
        }))
    }

    /// How a take's depth, read back from it, holds beside the guide a sequence draws.
    pub(super) fn take_follows(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        take: &str,
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let media = project.media(take).ok_or_else(|| {
            McpError::new(
                McpCode::InvalidArguments,
                format!(
                    "no media `{take}`: `follows` is a media whose picture is a take's depth, as a model read it back{}",
                    self.any_pending("proposal")
                ),
            )
        })?;
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let followed = kerf_engine::analysis::follows::follows(
            &resolved,
            Some(sequence.id.as_str()),
            &resolve_project_source_path(&self.dir, &media.path),
            media.rate.fps().round().max(1.0) as u32,
        )
        .map_err(|error| McpError::new(McpCode::MediaFailed, error.to_string()))?;
        Ok(json!({
            "revision": revision,
            "sequence": sequence.id.to_string(),
            "follows": take,
            "frames": followed.frames,
            "order": followed.order,
            "off": followed.off,
            "clips": followed.clips.iter().map(|clip| json!({
                "clip": clip.clip,
                "frames": clip.frames,
                "shows": clip.shows,
                "off": clip.off,
                "farther": clip.farther,
                "worst": {"frame": clip.worst.0, "off": clip.worst.1},
            })).collect::<Vec<_>>(),
        }))
    }

    /// A take held to its guide by its depth and by where its subjects are, in one reading.
    pub(super) fn take_held(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        input: &HeldInput,
    ) -> Result<Value, McpError> {
        let followed = self.take_follows(project, sequence, revision, &input.depth)?;
        // The subjects are named: a set or a ground is where it is whatever the take did,
        // and which clips are which is the scene's to say and not their size's.
        if input.clips.is_empty() {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`held` names in `clips` the guide's clips that are its subjects, as [\"crate\"]",
            ));
        }
        let mut clips = Vec::new();
        let mut sum = 0.0;
        for name in &input.clips {
            let Some(clip) = followed["clips"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|clip| clip["clip"] == name.as_str())
            else {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    format!(
                        "the guide has no clip `{name}` in depth: `clips` are clips in depth that `follows` reads"
                    ),
                ));
            };
            let name = name.clone();
            let placed = self.take_placed(
                project,
                sequence,
                revision,
                &PlacedInput {
                    take: input.take.clone(),
                    clip: name.clone(),
                },
            );
            // A subject the take has nothing of is not an error here: it is held not at all.
            let (place, overlap) = match placed {
                Ok(placed) => {
                    let overlap = placed["overlap"].as_f64().unwrap_or(0.0);
                    (
                        json!({
                            "overlap": overlap,
                            "off": placed["off"],
                            "size": placed["size"],
                            "frames": placed["frames"],
                            "shown": placed["shown"],
                        }),
                        overlap,
                    )
                }
                Err(error) if error.code == McpCode::MediaFailed => (json!({"overlap": 0.0}), 0.0),
                Err(error) => return Err(error),
            };
            let off = clip["off"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0);
            let held = ((overlap * (1.0 - off)) * 1e4).round() / 1e4;
            sum += held;
            clips.push(json!({
                "clip": name,
                "held": held,
                "shows": clip["shows"],
                "depth": {"off": clip["off"], "farther": clip["farther"]},
                "place": place,
            }));
        }
        let mut result = json!({
            "revision": revision,
            "sequence": followed["sequence"],
            "take": input.take,
            "depth": input.depth,
            "frames": followed["frames"],
            "order": followed["order"],
            "off": followed["off"],
            "clips": clips,
        });
        result["held"] = json!(((sum / input.clips.len() as f64) * 1e4).round() / 1e4);
        Ok(result)
    }

    /// Where a take has the subject of one of its guide's clips.
    pub(super) fn take_placed(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        input: &PlacedInput,
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let media = project.media(input.take.as_str()).ok_or_else(|| {
            McpError::new(
                McpCode::InvalidArguments,
                format!(
                    "no media `{}`: `take` is the media whose picture is the take{}",
                    input.take,
                    self.any_pending("proposal")
                ),
            )
        })?;
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let placed = kerf_engine::analysis::placed::placed(
            &resolved,
            Some(sequence.id.as_str()),
            &input.clip,
            &resolve_project_source_path(&self.dir, &media.path),
            media.rate.fps().round().max(1.0) as u32,
        )
        .map_err(|error| McpError::new(McpCode::MediaFailed, error.to_string()))?;
        let mut result = json!({
            "revision": revision,
            "sequence": sequence.id.to_string(),
            "take": input.take,
            "clip": placed.clip,
            "shown": placed.shown,
            "frames": placed.frames,
            "from": placed.from,
            "read": [placed.read.0, placed.read.1],
            "shows": placed.shows,
            "off": placed.off,
            "across": placed.across,
            "down": placed.down,
            "size": placed.size,
            "worst": {"frame": placed.worst.0, "off": placed.worst.1},
            "overlap": placed.overlap,
        });
        if placed.frames < placed.shown {
            result["note"] = json!(format!(
                "the subject was followed on {} of the {} frames the guide shows `{}` on, {} to {}: what is said is of those, and where it could not be followed the take may not have it.",
                placed.frames, placed.shown, placed.clip, placed.read.0, placed.read.1
            ));
        }
        Ok(result)
    }
}
