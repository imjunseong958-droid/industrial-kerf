//! The project's preset library: `presets/`, which travels with the
//! repository, then `.kerf/presets` and the project folder. Presets are read
//! here without kerfd, and the program is saved into the library.

use crate::collab::git_text;
use kerf_collab::scaffold::PRESETS_DIR;
use kerf_lang::preset::{self, Preset};
use kerf_mcp::PRESET_FOLDERS;
use kerf_model::{Catalog, Content, Id, Project};
use serde_json::{json, Map, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MAX_PRESET_BYTES: u64 = 1024 * 1024;
/// The most presets a library lists.
const MAX_LISTED: usize = 200;
const EXTENSION: &str = "kerfpreset";

/// The preset files of the project in `directory`, relative to it: the
/// library folders in order, each in name order.
pub(crate) fn library_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for folder in PRESET_FOLDERS {
        let Ok(entries) = fs::read_dir(directory.join(folder)) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.is_file() && path.extension().and_then(|e| e.to_str()) == Some(EXTENSION)
            })
            .filter_map(|path| path.file_name().map(|name| Path::new(folder).join(name)))
            .collect();
        found.sort();
        files.extend(found);
    }
    files.truncate(MAX_LISTED);
    files
}

/// The `preset_library` event for the project in `directory`.
pub(crate) fn library(directory: &Path) -> Value {
    let presets: Vec<Value> = library_files(directory)
        .iter()
        .map(|relative| entry(directory, relative))
        .collect();
    json!({"type": "preset_library",
        "directory": directory.join(PRESETS_DIR).to_string_lossy(),
        "presets": presets})
}

/// One library entry: what the preset needs, and who last changed it when
/// the project is in a repository.
fn entry(directory: &Path, relative: &Path) -> Value {
    let path = relative.to_string_lossy().replace('\\', "/");
    let mut entry = match read(&directory.join(relative)) {
        Ok((_, preset)) => {
            let (slots, texts) = summary(&preset);
            json!({"path": path, "name": preset.pattern.name, "slots": slots, "texts": texts})
        }
        Err(message) => {
            let name = relative
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            json!({"path": path, "name": name, "slots": [], "texts": {}, "error": message})
        }
    };
    if let Some(line) = git_text(
        directory,
        &["log", "-1", "--format=%an%x1f%cI", "--", &path],
    ) {
        if let Some((author, updated)) = line.split_once('\u{1f}') {
            entry["author"] = json!(author);
            entry["updated"] = json!(updated);
        }
    }
    entry
}

/// A preset file's text and what it reads as.
pub(crate) fn read(path: &Path) -> Result<(String, Preset), String> {
    if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
        return Err("preset files must use the .kerfpreset extension".into());
    }
    let mut document = String::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_PRESET_BYTES + 1)
                .read_to_string(&mut document)
        })
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if document.len() as u64 > MAX_PRESET_BYTES {
        return Err("preset exceeds 1 MiB".into());
    }
    let preset = preset::parse(&document, Catalog::builtin())?;
    Ok((document, preset))
}

/// A preset's media slots (`{id, seconds, video, audio, clips}`, where
/// `clips` counts the pattern's clips that use it) and its text slots.
pub(crate) fn summary(preset: &Preset) -> (Value, Value) {
    let pattern = &preset.pattern;
    let slots: Vec<Value> = pattern
        .media
        .iter()
        .map(|slot| {
            json!({"id": slot.id.as_str(), "seconds": slot.duration.seconds(),
                "video": slot.video.is_some(), "audio": slot.audio.is_some(),
                "clips": clips_using(pattern, &slot.id)})
        })
        .collect();
    let texts: Map<String, Value> = preset::text_slots(pattern)
        .into_iter()
        .map(|(clip, text)| (clip, Value::String(text)))
        .collect();
    (Value::Array(slots), Value::Object(texts))
}

fn clips_using(pattern: &Project, slot: &Id) -> usize {
    pattern
        .sequences
        .iter()
        .flat_map(|sequence| &sequence.tracks)
        .flat_map(|track| &track.clips)
        .filter(|clip| matches!(&clip.content, Content::Media { media, .. } if media == slot))
        .count()
}

/// The program (the first sequence) as a preset document named `name`,
/// else after the sequence, checked to read back.
pub(crate) fn export_program(project: &Project, name: Option<&str>) -> Result<String, String> {
    let sequence = project
        .sequences
        .first()
        .ok_or_else(|| "no sequence to export".to_owned())?;
    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&sequence.name);
    let document = preset::export(project, Catalog::builtin(), &sequence.id, name)?;
    preset::parse(&document, Catalog::builtin())?;
    if document.len() as u64 > MAX_PRESET_BYTES {
        return Err("preset exceeds 1 MiB".into());
    }
    Ok(document)
}

/// The library file a preset named `name` is saved as.
pub(crate) fn library_path(name: &str) -> Result<String, String> {
    let stem = kerf_collab::scaffold::folder_name(name)
        .map_err(|_| "the preset needs a name".to_owned())?;
    Ok(format!("{PRESETS_DIR}/{stem}.{EXTENSION}"))
}

/// Save `document` in the library of the project in `directory` as the
/// preset `name`, refusing an existing file unless `replace`. Returns the
/// path relative to the project.
pub(crate) fn save(
    directory: &Path,
    name: &str,
    document: &str,
    replace: bool,
) -> Result<String, String> {
    let relative = library_path(name)?;
    let path = directory.join(&relative);
    let failed = |error: io::Error| format!("cannot save {relative}: {error}");
    fs::create_dir_all(directory.join(PRESETS_DIR)).map_err(failed)?;
    if replace {
        // Written where Git does not look, so a sync never commits half.
        let temporary =
            kerf_collab::media::incoming_path(directory, ".kerfpreset").map_err(failed)?;
        let written = fs::write(&temporary, document).and_then(|_| fs::rename(&temporary, &path));
        if let Err(error) = written {
            let _ = fs::remove_file(&temporary);
            return Err(failed(error));
        }
        return Ok(relative);
    }
    let mut output = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(format!(
                "{relative} already exists; choose another name or replace it"
            ))
        }
        Err(error) => return Err(failed(error)),
    };
    if let Err(error) = output
        .write_all(document.as_bytes())
        .and_then(|_| output.sync_all())
    {
        let _ = fs::remove_file(&path);
        return Err(failed(error));
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::PROGRAM;
    use crate::Command;

    fn program() -> Project {
        kerf_runtime::parse_document(PROGRAM).unwrap()
    }

    #[test]
    fn preset_commands_are_read() {
        assert!(matches!(
            serde_json::from_value(json!({"type": "preset_library"})).unwrap(),
            Command::PresetLibrary
        ));
        let Command::PresetSave { name, replace } =
            serde_json::from_value(json!({"type": "preset_save", "name": "Intro"})).unwrap()
        else {
            panic!("expected preset_save");
        };
        assert_eq!((name.as_str(), replace), ("Intro", false));
        let Command::PresetSave { replace, .. } = serde_json::from_value(
            json!({"type": "preset_save", "name": "Intro", "replace": true}),
        )
        .unwrap() else {
            panic!("expected preset_save");
        };
        assert!(replace);
        assert!(serde_json::from_value::<Command>(json!({"type": "preset_save"})).is_err());
    }

    #[test]
    fn saving_names_the_file_after_the_preset_and_never_overwrites() {
        let directory = tempfile::tempdir().unwrap();
        let document = export_program(&program(), Some("Quick Cut")).unwrap();
        assert!(
            document.contains("project name=\"Quick Cut\""),
            "{document}"
        );
        let path = save(directory.path(), "Quick Cut", &document, false).unwrap();
        assert_eq!(path, "presets/Quick Cut.kerfpreset");
        assert_eq!(
            fs::read_to_string(directory.path().join(&path)).unwrap(),
            document
        );

        let other = export_program(&program(), Some("Other")).unwrap();
        let refused = save(directory.path(), "Quick Cut", &other, false).unwrap_err();
        assert!(refused.contains("already exists"), "{refused}");
        assert_eq!(
            fs::read_to_string(directory.path().join(&path)).unwrap(),
            document
        );

        save(directory.path(), "Quick Cut", &other, true).unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join(&path)).unwrap(),
            other
        );
        assert_eq!(library_path("a/b: c").unwrap(), "presets/a-b- c.kerfpreset");
        assert!(library_path("  ").is_err());
        let names: Vec<_> = fs::read_dir(directory.path().join("presets"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let incoming = directory.path().join(kerf_collab::media::INCOMING_DIR);
        assert_eq!(fs::read_dir(incoming).unwrap().count(), 0);
    }

    #[test]
    fn the_library_lists_presets_with_their_slots_and_texts() {
        let directory = tempfile::tempdir().unwrap();
        let document = export_program(&program(), Some("Two Cuts")).unwrap();
        save(directory.path(), "Two Cuts", &document, false).unwrap();
        fs::create_dir_all(directory.path().join(".kerf/presets")).unwrap();
        fs::write(
            directory.path().join(".kerf/presets/old.kerfpreset"),
            &document,
        )
        .unwrap();
        fs::write(
            directory.path().join("broken.kerfpreset"),
            "kerf 1\nnot a preset\n",
        )
        .unwrap();
        fs::write(directory.path().join("notes.txt"), "x").unwrap();

        let paths: Vec<_> = library_files(directory.path())
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            paths,
            [
                "presets/Two Cuts.kerfpreset",
                ".kerf/presets/old.kerfpreset",
                "broken.kerfpreset"
            ]
        );

        let event = library(directory.path());
        assert_eq!(event["type"], "preset_library");
        let presets = event["presets"].as_array().unwrap();
        assert_eq!(presets.len(), 3);
        let first = &presets[0];
        assert_eq!(first["path"], "presets/Two Cuts.kerfpreset");
        assert_eq!(first["name"], "Two Cuts");
        let slots = first["slots"].as_array().unwrap();
        // One slot per cut.
        assert_eq!(slots.len(), 2, "{first}");
        assert_eq!(slots[0]["clips"], 1);
        assert_eq!(slots[0]["video"], true);
        assert_eq!(slots[0]["seconds"], 2.0);
        assert_eq!(first["texts"], json!({"t1": "Hello"}));
        assert!(first.get("error").is_none());
        let broken = &presets[2];
        assert_eq!(broken["name"], "broken");
        assert!(broken["error"].is_string(), "{broken}");
    }
}
