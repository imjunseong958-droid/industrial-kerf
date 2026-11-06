//! Effects presets: a clip's picture effects kept as a look
//! (`.kerflook`, `kerf_model::look`) and put on another clip.
//!
//! `look_save {clip, name, replace?}` writes the clip's enabled picture
//! effects, with their settings and keyframes, as a look in the project's
//! `presets/looks/` folder, which travels with the repository:
//! `look_saved {path, id, version, effects, skipped}`. Effects a look cannot
//! carry (looks, sound effects, parameters naming a mask or a track) are
//! listed in `skipped`.
//!
//! `look_apply {clip, path}` reads a look file, keeps it in the project's
//! looks (`.kerf/looks/<id>@<version>.kerflook`, where Kerf finds the looks
//! documents use) and adds it to the clip as one effect, which draws as the
//! effects it holds: a `state`, then `look_applied {path, clip, fx,
//! version}`. A look that differs from one of the same id already in the
//! project is kept as the next version of it.

use crate::emit;
use kerf_model::look::{self, Look, On};
use kerf_model::{Catalog, Clip, Domain, Engine, FxKind, Interp, Param, ParamType, Project, Value};
use kerf_runtime::EditOp;
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;

/// Where saved effects presets go, relative to the project.
pub(crate) const LOOKS_DIR: &str = "presets/looks";
const EXTENSION: &str = "kerflook";

/// A refused look command: `error {command, message}`.
pub(crate) fn refused(tx: &SyncSender<Json>, command: &str, message: &str) {
    emit(
        tx,
        json!({"type": "error", "message": message, "command": command}),
    );
}

/// The clip `id` of `project`.
fn find_clip<'a>(project: &'a Project, id: &str) -> Option<&'a Clip> {
    project
        .sequences
        .iter()
        .flat_map(|sequence| &sequence.tracks)
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == id)
}

/// A look id from a name: `look.` and the name in lower case, with `_` for
/// anything else.
pub(crate) fn look_id(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('_') && !slug.is_empty() {
            slug.push('_');
        }
    }
    let slug = slug.trim_end_matches('_');
    let mut id = format!("look.{}", if slug.is_empty() { "effects" } else { slug });
    id.truncate(kerf_model::Id::MAX_LEN);
    id.trim_end_matches('_').to_owned()
}

/// Text in a look file's quotes: on one line, with `"` and `\` escaped.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn number(value: f64, time: bool) -> String {
    if time {
        format!("{value}s")
    } else {
        format!("{value}")
    }
}

/// A static value as a look writes it; `None` for one it cannot carry.
fn word(value: &Value, ty: &ParamType) -> Option<String> {
    Some(match (value, ty) {
        (Value::Number(n), ParamType::Time) => number(*n, true),
        (Value::Number(n), _) => number(*n, false),
        (Value::Bool(b), _) => b.to_string(),
        (Value::Color(color), _) => color.to_string(),
        (Value::Enum(choice), _) => quoted(choice),
        (Value::Text(text), ParamType::Text | ParamType::File(_)) => quoted(text),
        (Value::Text(_), _) => return None,
    })
}

/// A parameter as a look writes it: a static value, or keyframes on
/// numbers (times from the clip's first frame, in seconds).
fn param(param: &Param, ty: &ParamType) -> Option<String> {
    match param {
        Param::Static(value) => word(value, ty),
        Param::Animated(keys) => {
            let time = *ty == ParamType::Time;
            let keys: Option<Vec<String>> = keys
                .iter()
                .map(|key| {
                    let Value::Number(value) = key.value else {
                        return None;
                    };
                    let mut text = format!("{}s:{}", key.at.seconds(), number(value, time));
                    if key.interp != Interp::Linear {
                        text.push(':');
                        text.push_str(&key.interp.name());
                    }
                    Some(text)
                })
                .collect();
            Some(format!("{{{}}}", keys?.join(" ")))
        }
    }
}

/// A saved effects preset: the look file's text, its id, how many effects
/// it holds, and what it left out.
#[derive(Debug, PartialEq)]
pub(crate) struct Saved {
    pub text: String,
    pub id: String,
    pub effects: usize,
    pub skipped: Vec<String>,
}

/// The look `name` of `clip`'s enabled picture effects, checked to read
/// back against `catalog`.
pub(crate) fn from_clip(clip: &Clip, name: &str, catalog: &Catalog) -> Result<Saved, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("the effects preset needs a name".into());
    }
    let id = look_id(name);
    let mut text = String::from("kerf-look 1\n");
    writeln!(
        text,
        "look {id} version=1 name={} category=\"Effects presets\" on=clip",
        quoted(name)
    )
    .ok();
    writeln!(
        text,
        "  doc {}",
        quoted(&format!("Saved from clip {}.", clip.id))
    )
    .ok();
    let (mut effects, mut skipped) = (0, Vec::new());
    for effect in clip.effects.iter().filter(|effect| effect.enabled) {
        let Some(def) = catalog.fx(effect.fx.as_str()).filter(|def| {
            def.engine != Engine::Look && def.kind == FxKind::Effect && def.domain == Domain::Video
        }) else {
            skipped.push(effect.fx.to_string());
            continue;
        };
        let mut line = format!("  fx {}@{}", effect.fx, effect.version);
        for (key, value) in &effect.params {
            let written = def
                .find(key)
                .filter(|found| {
                    !matches!(
                        found.ty,
                        ParamType::Mask | ParamType::Track | ParamType::Sound
                    )
                })
                .and_then(|found| param(value, &found.ty));
            match written {
                Some(written) => write!(line, " {key}={written}").unwrap_or(()),
                None => skipped.push(format!("{}.{key}", effect.fx)),
            }
        }
        text.push_str(&line);
        text.push('\n');
        effects += 1;
    }
    if effects == 0 {
        return Err(format!("clip {} has no picture effects to save", clip.id));
    }
    if text.len() > look::MAX_BYTES {
        return Err("the effects preset is too large".into());
    }
    let read = look::read(&text, "effects preset").map_err(|error| error.to_string())?;
    read.check(catalog).map_err(|error| error.to_string())?;
    Ok(Saved {
        text,
        id,
        effects,
        skipped,
    })
}

/// The file an effects preset named `name` is saved as, relative to the
/// project.
pub(crate) fn library_path(name: &str) -> Result<String, String> {
    let stem = kerf_collab::scaffold::folder_name(name)
        .map_err(|_| "the effects preset needs a name".to_owned())?;
    Ok(format!("{LOOKS_DIR}/{stem}.{EXTENSION}"))
}

/// Write `text` as `relative` in `directory`, refusing an existing file
/// unless `replace`.
fn write(directory: &Path, relative: &str, text: &str, replace: bool) -> Result<(), String> {
    let path = directory.join(relative);
    let failed = |error: io::Error| format!("cannot save {relative}: {error}");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    if replace {
        return crate::paperwork::write_atomic(directory, &path, text);
    }
    let mut output = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(format!(
                "{relative} already exists; choose another name or replace it"
            ))
        }
        Err(error) => return Err(failed(error)),
    };
    if let Err(error) = output
        .write_all(text.as_bytes())
        .and_then(|_| output.sync_all())
    {
        let _ = fs::remove_file(&path);
        return Err(failed(error));
    }
    Ok(())
}

/// `look_save`: the `look_saved` event, or why it was refused.
pub(crate) fn save(
    directory: &Path,
    project: &Project,
    clip: &str,
    name: &str,
    replace: bool,
) -> Result<Json, String> {
    let found = find_clip(project, clip).ok_or_else(|| format!("no clip `{clip}`"))?;
    let saved = from_clip(found, name, Catalog::builtin())?;
    let relative = library_path(name)?;
    write(directory, &relative, &saved.text, replace)?;
    Ok(
        json!({"type": "look_saved", "path": relative, "id": saved.id, "version": 1,
        "effects": saved.effects, "skipped": saved.skipped}),
    )
}

/// `look_save` against the project kerfd has: `look_saved`, or `error
/// {command: look_save}`. Only a lost connection is returned.
pub(crate) fn save_from(
    client: &kerf_runtime::RuntimeClient,
    directory: &Path,
    clip: &str,
    name: &str,
    replace: bool,
    tx: &SyncSender<Json>,
) -> Result<(), kerf_runtime::RuntimeError> {
    let (_, document) = client.get_document(false)?;
    let saved = kerf_runtime::parse_document(&document)
        .map_err(|error| error.message)
        .and_then(|project| save(directory, &project, clip, name, replace));
    match saved {
        Ok(event) => emit(tx, event),
        Err(message) => refused(tx, "look_save", &message),
    }
    Ok(())
}

/// The look at `path` (relative to the project), as Kerf will keep it: the
/// same look the project already has, or a new version of it when one of
/// that id and version differs.
fn read_file(directory: &Path, path: &str) -> Result<Look, String> {
    let file = {
        let named = Path::new(path);
        if named.is_absolute() {
            named.to_path_buf()
        } else {
            directory.join(named)
        }
    };
    if file.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
        return Err("effects presets are .kerflook files".into());
    }
    let meta = fs::metadata(&file).map_err(|error| format!("cannot read {path}: {error}"))?;
    if meta.len() > look::MAX_BYTES as u64 {
        return Err(format!("a look is at most {} KiB", look::MAX_BYTES >> 10));
    }
    let text = fs::read_to_string(&file).map_err(|error| format!("cannot read {path}: {error}"))?;
    look::read(&text, path).map_err(|error| error.to_string())
}

/// `look` with its header's version set to `version`.
fn with_version(look: &Look, version: u32) -> Result<Look, String> {
    let mut done = false;
    let text: Vec<String> = look
        .text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if !done && trimmed.starts_with("look ") {
                done = true;
                line.split(' ')
                    .map(|part| {
                        if part.starts_with("version=") {
                            format!("version={version}")
                        } else {
                            part.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                line.to_owned()
            }
        })
        .collect();
    look::read(&(text.join("\n") + "\n"), &look.source).map_err(|error| error.to_string())
}

/// Keep `look` among the project's looks; the id and version the clip's
/// effect will name. A look the catalog already has as it is is not
/// written again.
pub(crate) fn keep(
    directory: &Path,
    look: Look,
    catalog: &Catalog,
) -> Result<(String, u32), String> {
    let id = look.id.to_string();
    let look = match catalog.look(&id, look.version) {
        Some(known) if known.same(&look) => return Ok((id, look.version)),
        Some(_) => {
            let newest = catalog
                .looks()
                .filter(|other| other.id == look.id)
                .map(|other| other.version)
                .max()
                .unwrap_or(look.version);
            with_version(&look, newest + 1)?
        }
        None => look,
    };
    if !matches!(look.on, On::Clip | On::Title) {
        return Err(format!("`{id}` is a look for transitions, not clips"));
    }
    let mut checked = catalog.clone();
    checked.define_look(look.clone())?;
    let folder = look::project_looks(directory);
    fs::create_dir_all(&folder).map_err(|error| format!("cannot keep {id}: {error}"))?;
    let path: PathBuf = folder.join(format!("{id}@{}.{EXTENSION}", look.version));
    write_new(&path, &look.text)?;
    if let Err(error) = look::refresh_project_looks(directory) {
        let _ = fs::remove_file(&path);
        let _ = look::refresh_project_looks(directory);
        return Err(error);
    }
    Ok((id, look.version))
}

fn write_new(path: &Path, text: &str) -> Result<(), String> {
    let failed = |error: io::Error| format!("cannot keep {}: {error}", path.display());
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(failed)?;
    if let Err(error) = output
        .write_all(text.as_bytes())
        .and_then(|_| output.sync_all())
    {
        let _ = fs::remove_file(path);
        return Err(failed(error));
    }
    Ok(())
}

/// `look_apply`: the edit that adds the look at `path` to `clip`, and the
/// `look_applied` event that follows its state.
pub(crate) fn apply(
    directory: &Path,
    project: &Project,
    clip: &str,
    path: &str,
) -> Result<(Vec<EditOp>, Json), String> {
    find_clip(project, clip).ok_or_else(|| format!("no clip `{clip}`"))?;
    let read = read_file(directory, path)?;
    let (fx, version) = keep(directory, read, Catalog::builtin())?;
    let op = EditOp::FxAdd {
        clip: clip.to_owned(),
        fx: fx.clone(),
        params: BTreeMap::new(),
        index: None,
    };
    Ok((
        vec![op],
        json!({"type": "look_applied", "path": path, "clip": clip, "fx": fx, "version": version}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = "kerf 1\nproject name=\"Looks\"\n\
        media m1 path=\"media/shot.mov\" rate=24 dur=96f video=320x180 audio=2ch@48000\n\
        sequence s1 rate=24 size=320x180 audio=48000\n\
        \x20 track V1 video\n\
        \x20   clip c1 media=m1 src=0f at=0f dur=48f\n\
        \x20     fx color.basic@1 exposure=0.3 saturation={0f:1 24f:1.5:ease}\n\
        \x20     fx blur.gaussian@1 radius=4\n\
        \x20   clip c2 media=m1 src=48f at=48f dur=24f\n";

    fn program() -> Project {
        kerf_runtime::parse_document(PROGRAM).unwrap()
    }

    #[test]
    fn look_commands_read_as_the_contract_says() {
        assert!(matches!(
            serde_json::from_value(json!({"type": "look_save", "clip": "c1", "name": "Harbor warm"})).unwrap(),
            crate::Command::LookSave { clip, name, replace: false } if clip == "c1" && name == "Harbor warm"
        ));
        assert!(matches!(
            serde_json::from_value(
                json!({"type": "look_save", "clip": "c1", "name": "x", "replace": true})
            )
            .unwrap(),
            crate::Command::LookSave { replace: true, .. }
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "look_apply", "clip": "c2", "path": "presets/looks/Harbor warm.kerflook"})).unwrap(),
            crate::Command::LookApply { clip, path } if clip == "c2" && path.ends_with(".kerflook")
        ));
    }

    #[test]
    fn ids_come_from_names() {
        assert_eq!(look_id("Harbor warm"), "look.harbor_warm");
        assert_eq!(look_id("  Coast grade, warm!  "), "look.coast_grade_warm");
        assert_eq!(look_id("☀"), "look.effects");
        assert!(look_id(&"long name ".repeat(20)).len() <= kerf_model::Id::MAX_LEN);
        assert!(kerf_model::Id::is_valid(&look_id(&"long name ".repeat(20))));
    }

    #[test]
    fn a_clips_effects_become_a_look_that_reads_back() {
        let project = program();
        let clip = find_clip(&project, "c1").unwrap();
        let saved = from_clip(clip, "Harbor \"warm\"", Catalog::builtin()).unwrap();
        assert_eq!(saved.id, "look.harbor_warm");
        assert_eq!(saved.effects, 2);
        assert!(saved.skipped.is_empty(), "{:?}", saved.skipped);
        assert!(saved.text.starts_with(
            "kerf-look 1\nlook look.harbor_warm version=1 name=\"Harbor \\\"warm\\\"\" category=\"Effects presets\" on=clip\n"
        ), "{}", saved.text);
        assert!(
            saved
                .text
                .contains("  fx color.basic@1 exposure=0.3 saturation={0s:1 1s:1.5:ease}\n"),
            "{}",
            saved.text
        );
        assert!(
            saved.text.contains("  fx blur.gaussian@1 radius=4\n"),
            "{}",
            saved.text
        );
        let read = look::read(&saved.text, "test").unwrap();
        assert_eq!(read.name, "Harbor \"warm\"");
        assert_eq!(read.on, On::Clip);
        // Nothing to save on a clip without effects.
        let bare = find_clip(&project, "c2").unwrap();
        assert!(from_clip(bare, "Bare", Catalog::builtin())
            .unwrap_err()
            .contains("no picture effects"));
        assert!(from_clip(clip, "  ", Catalog::builtin()).is_err());
    }

    #[test]
    fn saving_writes_the_library_file_and_never_overwrites() {
        let directory = tempfile::tempdir().unwrap();
        let project = program();
        let event = save(directory.path(), &project, "c1", "Harbor warm", false).unwrap();
        assert_eq!(event["type"], "look_saved");
        assert_eq!(event["path"], "presets/looks/Harbor warm.kerflook");
        assert_eq!(event["effects"], 2);
        let file = directory.path().join("presets/looks/Harbor warm.kerflook");
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("look.harbor_warm"));
        assert!(save(directory.path(), &project, "c1", "Harbor warm", false)
            .unwrap_err()
            .contains("already exists"));
        save(directory.path(), &project, "c1", "Harbor warm", true).unwrap();
        assert!(save(directory.path(), &project, "nope", "X", false)
            .unwrap_err()
            .contains("no clip"));
    }

    #[test]
    fn applying_keeps_the_look_and_adds_it_to_the_clip() {
        let directory = tempfile::tempdir().unwrap();
        let project = program();
        save(directory.path(), &project, "c1", "Harbor warm", false).unwrap();
        let path = "presets/looks/Harbor warm.kerflook";
        let read = read_file(directory.path(), path).unwrap();
        // A catalog without the look keeps it as it is.
        let catalog = Catalog::bundled().clone();
        let (fx, version) = keep(directory.path(), read.clone(), &catalog).unwrap();
        assert_eq!((fx.as_str(), version), ("look.harbor_warm", 1));
        let kept = look::project_looks(directory.path()).join("look.harbor_warm@1.kerflook");
        assert_eq!(fs::read_to_string(&kept).unwrap(), read.text);
        // One the catalog has as it is is not written again.
        let mut known = catalog.clone();
        known.define_look(read.clone()).unwrap();
        assert_eq!(
            keep(directory.path(), read.clone(), &known).unwrap(),
            (fx.clone(), 1)
        );
        // A different look of that id and version becomes the next version.
        let changed = look::read(&read.text.replace("radius=4", "radius=9"), "changed").unwrap();
        let (_, next) = keep(directory.path(), changed, &known).unwrap();
        assert_eq!(next, 2);
        let second = look::project_looks(directory.path()).join("look.harbor_warm@2.kerflook");
        assert!(fs::read_to_string(second).unwrap().contains("version=2"));
        // The edit names the look.
        let (ops, event) = apply(directory.path(), &project, "c2", path).unwrap();
        assert_eq!(
            serde_json::to_value(&ops).unwrap(),
            json!([{"op": "fx-add", "clip": "c2", "fx": "look.harbor_warm"}])
        );
        assert_eq!(event["type"], "look_applied");
        assert_eq!(event["clip"], "c2");
        assert!(apply(directory.path(), &project, "c2", "presets/none.kerflook").is_err());
        assert!(
            apply(directory.path(), &project, "c2", "presets/looks/x.txt")
                .unwrap_err()
                .contains(".kerflook")
        );
        let _ = look::install_project_looks(&std::env::temp_dir().join("kerf-no-looks-here"));
    }
}
