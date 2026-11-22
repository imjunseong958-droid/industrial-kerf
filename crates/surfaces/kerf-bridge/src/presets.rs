//! The project's preset library: `presets/`, which travels with the
//! repository, then `.kerf/presets` and the project folder. Presets are read
//! here without kerfd, and the program is saved into the library.

use crate::collab::git_text;
use crate::event::{Event, LookEntry, PresetEntry, PresetLibrary, PresetMade, PresetSlot};
use kerf_collab::scaffold::PRESETS_DIR;
use kerf_lang::preset::{self, Preset};
use kerf_mcp::PRESET_FOLDERS;
use kerf_model::{Catalog, Content, Id, Project};
use std::collections::BTreeMap;
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

/// The `preset_library` event for the project in `directory`: the
/// presets, and the effects presets saved in `presets/looks/`.
pub(crate) fn library(directory: &Path) -> Event {
    let presets = library_files(directory)
        .iter()
        .map(|relative| entry(directory, relative))
        .collect();
    Event::PresetLibrary(PresetLibrary {
        directory: directory.join(PRESETS_DIR).to_string_lossy().into_owned(),
        presets,
        looks: looks(directory),
    })
}

/// The effects presets in `presets/looks/`, in name order: `{path, name,
/// id, version, effects, author?, updated?}`, or `{path, name, error}` for
/// one that does not read.
fn looks(directory: &Path) -> Vec<LookEntry> {
    let folder = directory.join(crate::looks::LOOKS_DIR);
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("kerflook")
        })
        .collect();
    files.sort();
    files.truncate(MAX_LISTED);
    files
        .iter()
        .map(|file| {
            let name = file
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            let path = format!(
                "{}/{}",
                crate::looks::LOOKS_DIR,
                file.file_name().unwrap_or_default().to_string_lossy()
            );
            let read = fs::read_to_string(file)
                .map_err(|error| error.to_string())
                .and_then(|text| kerf_model::look::read(&text, &path).map_err(|e| e.to_string()));
            let mut entry = match read {
                Ok(look) => {
                    let effects = look
                        .lines
                        .iter()
                        .filter(|line| matches!(line, kerf_model::look::Line::Fx { .. }))
                        .count();
                    LookEntry {
                        path: path.clone(),
                        name: look.name,
                        id: Some(look.id.as_str().to_owned()),
                        version: Some(look.version),
                        effects: Some(effects),
                        ..LookEntry::default()
                    }
                }
                Err(message) => LookEntry {
                    path: path.clone(),
                    name,
                    error: Some(message),
                    ..LookEntry::default()
                },
            };
            if let Some(line) = git_text(
                directory,
                &["log", "-1", "--format=%an%x1f%cI", "--", &path],
            ) {
                if let Some((author, updated)) = line.split_once('\u{1f}') {
                    entry.author = Some(author.to_owned());
                    entry.updated = Some(updated.to_owned());
                }
            }
            entry
        })
        .collect()
}

/// One library entry: what the preset needs, and who last changed it when
/// the project is in a repository.
fn entry(directory: &Path, relative: &Path) -> PresetEntry {
    let path = relative.to_string_lossy().replace('\\', "/");
    let mut entry = match read(&directory.join(relative)) {
        Ok((_, preset)) => {
            let (slots, texts) = summary(&preset);
            PresetEntry {
                path: path.clone(),
                name: preset.pattern.name.clone(),
                slots,
                texts,
                made: Some(made(&preset)),
                ..PresetEntry::default()
            }
        }
        Err(message) => {
            let name = relative
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            PresetEntry {
                path: path.clone(),
                name,
                error: Some(message),
                ..PresetEntry::default()
            }
        }
    };
    if let Some(line) = git_text(
        directory,
        &["log", "-1", "--format=%an%x1f%cI", "--", &path],
    ) {
        if let Some((author, updated)) = line.split_once('\u{1f}') {
            entry.author = Some(author.to_owned());
            entry.updated = Some(updated.to_owned());
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
/// `clips` counts the pattern's clips that use it) and its text slots. The
/// media it makes from steps are not slots: nothing binds them.
pub(crate) fn summary(preset: &Preset) -> (Vec<PresetSlot>, BTreeMap<String, String>) {
    let pattern = &preset.pattern;
    let slots = preset::slots(pattern)
        .map(|slot| PresetSlot {
            id: slot.id.as_str().to_owned(),
            seconds: slot.duration.seconds(),
            video: slot.video.is_some(),
            audio: slot.audio.is_some(),
            clips: clips_using(pattern, &slot.id),
        })
        .collect();
    (slots, preset::text_slots(pattern))
}

/// The media a preset makes from the steps under their lines, which applying
/// it adds to the project: `{id, of, steps, clips}`, `of` the slot it is of
/// frame for frame where it is one's, and `steps` as the document writes them.
pub(crate) fn made(preset: &Preset) -> Vec<PresetMade> {
    let pattern = &preset.pattern;
    preset::made(pattern)
        .map(|media| {
            let steps: Vec<String> = media.steps.iter().map(kerf_lang::print_step).collect();
            PresetMade {
                id: media.id.as_str().to_owned(),
                of: preset::footage_of(media).map(|slot| slot.to_string()),
                steps,
                clips: clips_using(pattern, &media.id),
            }
        })
        .collect()
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
            // The write's error is the one to report; this only tidies the temporary file.
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
        // The write's error is the one to report; this only tidies the half-written file.
        let _ = fs::remove_file(&path);
        return Err(failed(error));
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::PROGRAM;

    fn program() -> Project {
        kerf_runtime::parse_document(PROGRAM).unwrap()
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
}
