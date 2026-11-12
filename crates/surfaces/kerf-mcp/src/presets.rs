//! File adapters for portable Kerf presets. Applying one still goes through the
//! harness's normal edit proposal, checks, and before/after preview; the looks
//! it carries join the project's.

use crate::{KerfMcpRouter, McpError};
use kerf_engine::derivatives::DerivativeCache;
use kerf_lang::preset::{self, Preset};
use kerf_lang::Clock;
use kerf_model::look::Look;
use kerf_model::{Catalog, Content, Id, Project, Time};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_PRESET_BYTES: u64 = 1024 * 1024;
/// Where a project's presets live, the shared library first: `presets/`
/// (which travels with the repository), then `.kerf/presets` and the
/// project folder itself.
pub const PRESET_FOLDERS: [&str; 3] = ["presets", ".kerf/presets", ""];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingInput {
    media: String,
    #[serde(default = "zero")]
    from: String,
    /// Place the pattern's markers on this music's onsets.
    #[serde(default)]
    beats: bool,
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

    /// The preset's text, read and validated.
    fn read_preset(&self, file: &str) -> Result<(String, Preset), McpError> {
        let mut document = String::new();
        File::open(self.preset_path(file)?)
            .map_err(io_error)?
            .take(MAX_PRESET_BYTES + 1)
            .read_to_string(&mut document)
            .map_err(io_error)?;
        if document.len() as u64 > MAX_PRESET_BYTES {
            return Err(invalid("preset exceeds 1 MiB"));
        }
        let preset = preset::parse(&document, Catalog::builtin()).map_err(invalid)?;
        Ok((document, preset))
    }

    /// Copy the looks a preset carries that the project lacks into its
    /// looks folder, so what the preset made keeps drawing without it.
    fn keep_looks(&self, looks: &[Look]) -> Result<(), McpError> {
        write_looks(&self.dir, looks)
    }

    /// The project's preset files, the shared `presets/` library first,
    /// each folder in name order.
    pub(super) fn list_presets(&self) -> Result<Value, McpError> {
        let mut files = Vec::new();
        for directory in PRESET_FOLDERS.map(|folder| self.dir.join(folder)) {
            let entries = match fs::read_dir(directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(error)),
            };
            let mut found = Vec::new();
            for entry in entries {
                let path = entry.map_err(io_error)?.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("kerfpreset")
                {
                    found.push(
                        path.strip_prefix(&self.dir)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
            found.sort();
            files.extend(found);
        }
        let more = files.len() > 100;
        files.truncate(100);
        Ok(json!({"presets": files, "more": more}))
    }

    /// The pattern, its text slots, and each media slot with the clips that
    /// use it and the project's imported media that can fill it.
    pub(super) fn inspect_preset(&self, project: &Project, file: &str) -> Result<Value, McpError> {
        let (document, preset) = self.read_preset(file)?;
        let pattern = &preset.pattern;
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
                    "seconds": slot.duration.seconds(), "clips": slot_clips(pattern, &slot.id),
                    "fits": fits})
            })
            .collect();
        let looks: Vec<_> = preset
            .looks
            .iter()
            .map(|look| json!({"id": look.id.as_str(), "version": look.version, "name": look.name}))
            .collect();
        Ok(
            json!({"preset": file, "name": pattern.name, "document": document,
            "sequence": pattern.sequences[0].id.as_str(),
            "slots": slots, "textSlots": preset::text_slots(pattern), "looks": looks}),
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
        let preset = preset::parse(&document, Catalog::builtin()).map_err(invalid)?;
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
        let looks: Vec<_> = preset
            .looks
            .iter()
            .map(|look| format!("{}@{}", look.id, look.version))
            .collect();
        Ok(json!({"status": "saved", "preset": file, "name": name,
            "slots": preset.pattern.media.len(), "looks": looks}))
    }

    /// The script that applies a preset, and the id each thing of its
    /// pattern is given in the project, by the id the pattern has for it.
    pub(super) fn preset_script(
        &self,
        project: &Project,
        file: &str,
        sequence: Option<&str>,
        inputs: &BTreeMap<String, BindingInput>,
        texts: &BTreeMap<String, String>,
    ) -> Result<(String, BTreeMap<String, String>), McpError> {
        let (document, preset) = self.read_preset(file)?;
        let pattern = &preset.pattern;
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
            let beats = if input.beats {
                let index = kerf_engine::media_index(
                    &path,
                    &DerivativeCache::new(&self.dir),
                    &kerf_engine::CancellationToken::new(),
                )
                .map_err(|e| invalid(e.to_string()))?;
                if index.onsets.is_empty() {
                    return Err(invalid(format!(
                        "`{}` has no sound onsets to place markers on",
                        media.id
                    )));
                }
                index
                    .onsets
                    .iter()
                    .filter_map(|seconds| Time::from_seconds(*seconds))
                    .collect()
            } else {
                Vec::new()
            };
            bindings.insert(
                slot.clone(),
                preset::Binding {
                    media: Id::new(&input.media).map_err(|e| invalid(e.to_string()))?,
                    from,
                    beats,
                },
            );
        }
        let target = match sequence {
            Some(id) => project.sequence(id),
            None => project.sequences.first(),
        }
        .ok_or_else(|| invalid("no target sequence"))?;
        let (script, ids) = preset::instantiate_with_ids(
            project,
            Catalog::builtin(),
            &document,
            &target.id,
            &bindings,
            texts,
        )
        .map_err(invalid)?;
        self.keep_looks(&preset.looks)?;
        Ok((script, ids))
    }
}

/// Write the looks a preset carries that Kerf lacks into the looks folder
/// of the project in `directory` (never over a file already there), so what
/// the preset made keeps drawing without it.
pub fn keep_looks(directory: &Path, looks: &[Look]) -> Result<(), String> {
    write_looks(directory, looks).map_err(|error| error.message)
}

fn write_looks(directory: &Path, looks: &[Look]) -> Result<(), McpError> {
    let folder = kerf_model::look::project_looks(directory);
    for look in looks {
        if Catalog::builtin()
            .look(look.id.as_str(), look.version)
            .is_some()
        {
            continue;
        }
        fs::create_dir_all(&folder).map_err(io_error)?;
        let path = folder.join(format!("{}@{}.kerflook", look.id, look.version));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                invalid(format!(
                    "cannot keep `{}@{}` as {}: {error}",
                    look.id,
                    look.version,
                    path.display()
                ))
            })?;
        if let Err(error) = output
            .write_all(look.text.as_bytes())
            .and_then(|_| output.sync_all())
        {
            let _ = fs::remove_file(&path);
            return Err(io_error(error));
        }
    }
    kerf_model::look::refresh_project_looks(directory).map_err(invalid)?;
    Ok(())
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
