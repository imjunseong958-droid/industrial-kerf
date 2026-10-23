//! File adapters for portable Kerf presets. Applying one still goes through the
//! harness's normal edit proposal, checks, and before/after preview.

use crate::{KerfMcpRouter, McpError};
use kerf_lang::{preset, Clock};
use kerf_model::{Catalog, Content, Id, Project};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_PRESET_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingInput {
    media: String,
    #[serde(default = "zero")]
    from: String,
}

fn zero() -> String {
    "0f".into()
}
fn invalid(message: impl Into<String>) -> McpError {
    McpError::new("INVALID_PRESET", message)
}
fn io_error(error: std::io::Error) -> McpError {
    McpError::new("PRESET_IO", error.to_string())
}

impl KerfMcpRouter {
    fn preset_path(&self, file: &str) -> Result<PathBuf, McpError> {
        let path = Path::new(file);
        if path.extension().and_then(|s| s.to_str()) != Some("kerfpreset") {
            return Err(invalid("preset files must use the .kerfpreset extension"));
        }
        Ok(if path.is_absolute() {
            path.to_owned()
        } else {
            self.dir.join(path)
        })
    }

    /// The preset's text and its validated pattern.
    fn read_preset(&self, file: &str) -> Result<(String, Project), McpError> {
        let mut document = String::new();
        File::open(self.preset_path(file)?)
            .map_err(io_error)?
            .take(MAX_PRESET_BYTES + 1)
            .read_to_string(&mut document)
            .map_err(io_error)?;
        if document.len() as u64 > MAX_PRESET_BYTES {
            return Err(invalid("preset exceeds 1 MiB"));
        }
        let pattern = preset::parse(&document, Catalog::builtin()).map_err(invalid)?;
        Ok((document, pattern))
    }

    pub(super) fn list_presets(&self) -> Result<Value, McpError> {
        let mut files = Vec::new();
        for directory in [self.dir.clone(), self.dir.join(".kerf/presets")] {
            let entries = match fs::read_dir(directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(error)),
            };
            for entry in entries {
                let path = entry.map_err(io_error)?.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("kerfpreset")
                {
                    files.push(
                        path.strip_prefix(&self.dir)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        files.sort();
        let more = files.len() > 100;
        files.truncate(100);
        Ok(json!({"presets": files, "more": more}))
    }

    /// The pattern, its text slots, and each media slot with the clips that
    /// use it and the project's imported media that can fill it.
    pub(super) fn inspect_preset(&self, project: &Project, file: &str) -> Result<Value, McpError> {
        let (document, pattern) = self.read_preset(file)?;
        let slots: Vec<_> = pattern
            .media
            .iter()
            .map(|slot| {
                // Judged from imported metadata; applying re-probes the files.
                let fits: Vec<_> = project
                    .media
                    .iter()
                    .filter_map(|media| {
                        let latest = preset::latest_start(slot, media)?;
                        Some(json!({"media": media.id.as_str(),
                            "latestFrame": media.rate.frames_floor(latest)}))
                    })
                    .collect();
                json!({"id": slot.id.as_str(), "video": slot.video.is_some(), "audio": slot.audio.is_some(),
                    "minimumDuration": Clock::of_media(slot).print_duration(slot.duration),
                    "seconds": slot.duration.seconds(), "clips": slot_clips(&pattern, &slot.id),
                    "fits": fits})
            })
            .collect();
        Ok(
            json!({"preset": file, "name": pattern.name, "document": document,
            "sequence": pattern.sequences[0].id.as_str(),
            "slots": slots, "textSlots": preset::text_slots(&pattern)}),
        )
    }

    pub(super) fn save_preset(
        &self,
        project: &Project,
        file: &str,
        sequence: Option<&str>,
        name: Option<&str>,
    ) -> Result<Value, McpError> {
        let sequence = match sequence {
            Some(id) => project.sequence(id),
            None => project.sequences.first(),
        }
        .ok_or_else(|| invalid("no sequence to save"))?;
        let name = name.unwrap_or(&sequence.name);
        let document =
            preset::export(project, Catalog::builtin(), &sequence.id, name).map_err(invalid)?;
        // Verify the exported document before writing; an empty sequence is not a preset.
        let pattern = preset::parse(&document, Catalog::builtin()).map_err(invalid)?;
        if document.len() as u64 > MAX_PRESET_BYTES {
            return Err(invalid("preset exceeds 1 MiB"));
        }
        let path = self.preset_path(file)?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("preset has no parent directory"))?
            .canonicalize()
            .map_err(io_error)?;
        if !parent.starts_with(self.dir.canonicalize().map_err(io_error)?) {
            return Err(invalid(
                "save presets inside the project; copy the file to share it",
            ));
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io_error)?;
        if let Err(error) = output
            .write_all(document.as_bytes())
            .and_then(|_| output.sync_all())
        {
            let _ = fs::remove_file(&path);
            return Err(io_error(error));
        }
        Ok(json!({"status": "saved", "preset": file, "name": name, "slots": pattern.media.len()}))
    }

    pub(super) fn preset_script(
        &self,
        project: &Project,
        file: &str,
        sequence: Option<&str>,
        inputs: &BTreeMap<String, BindingInput>,
        texts: &BTreeMap<String, String>,
    ) -> Result<String, McpError> {
        let (document, pattern) = self.read_preset(file)?;
        let mut bindings = BTreeMap::new();
        for slot in &pattern.media {
            if !inputs.contains_key(slot.id.as_str()) {
                return Err(invalid(format!("missing binding for `{}`", slot.id)));
            }
        }
        let mut probes = BTreeMap::new();
        for (slot, input) in inputs {
            let required = pattern
                .media(slot)
                .ok_or_else(|| invalid(format!("unknown slot `{slot}`")))?;
            let media = project
                .media(&input.media)
                .ok_or_else(|| invalid(format!("no imported media `{}`", input.media)))?;
            let from = Clock::of_media(media)
                .position(&input.from)
                .map_err(invalid)?;
            let path = if Path::new(&media.path).is_absolute() {
                PathBuf::from(&media.path)
            } else {
                self.dir.join(&media.path)
            };
            if !path.is_file() {
                return Err(invalid(format!(
                    "replacement media `{}` is missing",
                    media.id
                )));
            }
            if !probes.contains_key(&input.media) {
                probes.insert(
                    input.media.clone(),
                    kerf_engine::info(&path.to_string_lossy())
                        .map_err(|e| invalid(e.to_string()))?,
                );
            }
            let info = &probes[&input.media];
            // Relink currently changes only a path. Do not trust stale source metadata
            // when deciding that a replacement has enough material or the right streams.
            let actual_duration = info
                .duration
                .ok_or_else(|| invalid(format!("cannot determine duration of `{}`", media.id)))?;
            if from.seconds() + required.duration.seconds() > actual_duration + 0.000001
                || (required.video.is_some() && info.width.is_none())
                || (required.audio.is_some() && info.has_audio != Some(true))
            {
                return Err(invalid(format!("replacement `{}` cannot fill slot `{slot}`; check its actual duration and streams", media.id)));
            }
            bindings.insert(
                slot.clone(),
                preset::Binding {
                    media: Id::new(&input.media).map_err(|e| invalid(e.to_string()))?,
                    from,
                },
            );
        }
        let target = match sequence {
            Some(id) => project.sequence(id),
            None => project.sequences.first(),
        }
        .ok_or_else(|| invalid("no target sequence"))?;
        preset::instantiate(
            project,
            Catalog::builtin(),
            &document,
            &target.id,
            &bindings,
            texts,
        )
        .map_err(invalid)
    }
}

/// Where a slot appears in the pattern, so a person can tell slots apart.
fn slot_clips(pattern: &Project, slot: &Id) -> Vec<Value> {
    let mut clips = Vec::new();
    for sequence in &pattern.sequences {
        let clock = Clock::of_sequence(sequence);
        for track in &sequence.tracks {
            for clip in &track.clips {
                if matches!(&clip.content, Content::Media { media, .. } if media == slot) {
                    clips.push(json!({"id": clip.id.as_str(), "name": clip.name,
                        "sequence": sequence.id.as_str(), "track": track.id.as_str(),
                        "at": clock.print_position(clip.start)}));
                }
            }
        }
    }
    clips
}
