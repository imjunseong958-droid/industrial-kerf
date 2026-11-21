//! The paperwork that travels with a master: `export_captions {format:
//! srt|vtt, path?, sequence?, track?}`, the sequence's captions
//! (`kerf_lang::to_srt` / `to_vtt`).
//!
//! Files go where `path` says (a file, or a folder for the default name;
//! relative to the project folder), else beside the newest finished
//! master, else in the project's `deliveries/` folder. They are written to
//! a temporary file beside the target and renamed over it, so a reader
//! never sees half a file; paperwork is made again for each master, so an
//! earlier file of that name is replaced.

use crate::emit;
use kerf_model::{Project, Sequence};
use kerf_runtime::{RenderJob, RuntimeClient, RuntimeError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;

/// The folder paperwork goes in when no master was rendered yet.
pub(crate) const DELIVERIES: &str = "deliveries";

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(crate) struct CaptionsRequest {
    /// `srt` or `vtt`.
    pub format: String,
    #[serde(default)]
    pub path: Option<String>,
    /// The program sequence when absent.
    #[serde(default)]
    pub sequence: Option<String>,
    /// Every enabled subtitle track when absent.
    #[serde(default)]
    pub track: Option<String>,
}

/// A paperwork export that failed: `export {status: failed, kind, format,
/// message}`, and the error naming the command.
fn failed(tx: &SyncSender<Value>, command: &str, kind: &str, format: &str, message: &str) {
    emit(
        tx,
        json!({"type": "export", "status": "failed", "kind": kind, "format": format,
            "message": message}),
    );
    emit(
        tx,
        json!({"type": "error", "message": message, "command": command, "format": format}),
    );
}

fn completed(kind: &str, format: &str, path: &Path) -> Value {
    json!({"type": "export", "status": "completed", "kind": kind, "format": format,
        "path": path.to_string_lossy(), "progress": 1.0})
}

/// The captions of `sequence` (of `track` alone when given) as `format`.
pub(crate) fn captions(
    sequence: &Sequence,
    format: &str,
    track: Option<&str>,
) -> Result<String, String> {
    let track = track
        .map(|track| kerf_model::Id::new(track).map_err(|error| error.to_string()))
        .transpose()?;
    if kerf_lang::to_srt(sequence, track.as_ref()).is_empty() {
        return Err(match &track {
            Some(track) => format!("track `{track}` has no captions to export"),
            None => "the sequence has no captions to export".into(),
        });
    }
    match format {
        "srt" => Ok(kerf_lang::to_srt(sequence, track.as_ref())),
        "vtt" => Ok(kerf_lang::to_vtt(sequence, track.as_ref())),
        other => Err(format!("captions are srt or vtt, not `{other}`")),
    }
}

/// The file name paperwork of `format` takes after `base` (the master's
/// file stem, or the project's name).
fn file_name(base: &str, format: &str) -> String {
    format!("{base}.{format}")
}

/// The newest finished master whose file is still there.
pub(crate) fn latest_master(jobs: &[RenderJob]) -> Option<PathBuf> {
    jobs.iter()
        .rev()
        .filter(|job| job.status == "completed" && job.quality.as_deref() == Some("master"))
        .filter_map(|job| job.output_path.as_deref().map(PathBuf::from))
        .find(|path| path.is_file())
}

/// Where paperwork of `format` goes: `path` (relative to the project; a
/// folder takes the default name), else beside `master`, else in
/// `deliveries/` named after the project.
pub(crate) fn target(
    directory: &Path,
    path: Option<&str>,
    master: Option<&Path>,
    project_name: &str,
    format: &str,
) -> Result<PathBuf, String> {
    let base = match master.and_then(Path::file_stem) {
        Some(stem) => stem.to_string_lossy().into_owned(),
        None => kerf_collab::scaffold::folder_name(project_name)
            .unwrap_or_else(|_| "Project".to_owned()),
    };
    let default = file_name(&base, format);
    let path = path.map(str::trim).filter(|path| !path.is_empty());
    Ok(match path {
        Some(path) => {
            let named = Path::new(path);
            let full = if named.is_absolute() {
                named.to_path_buf()
            } else {
                directory.join(named)
            };
            if path.ends_with('/') || path.ends_with('\\') || full.is_dir() {
                full.join(default)
            } else {
                full
            }
        }
        None => match master.and_then(Path::parent) {
            Some(folder) => folder.join(default),
            None => directory.join(DELIVERIES).join(default),
        },
    })
}

/// Write `text` to `path` through a temporary file beside it. Missing
/// folders are made inside the project; an absolute folder elsewhere must
/// exist.
pub(crate) fn write_atomic(directory: &Path, path: &Path, text: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| format!("{} has no folder", path.display()))?;
    if parent.starts_with(directory) {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make {}: {error}", parent.display()))?;
    } else if !parent.is_dir() {
        return Err(format!("{} is not a folder", parent.display()));
    }
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} names no file", path.display()))?;
    let temporary = parent.join(format!(
        ".{}.{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        crate::key()
    ));
    let written = fs::File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|_| fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot write {}: {error}", path.display()));
    }
    Ok(())
}

/// The project kerfd has, and its program sequence (or `sequence`).
fn project(client: &RuntimeClient) -> Result<Project, RuntimeError> {
    let (_, document) = client.get_document(false)?;
    kerf_runtime::parse_document(&document)
}

fn sequence<'a>(project: &'a Project, id: Option<&str>) -> Result<&'a Sequence, String> {
    match id {
        Some(id) => project
            .sequences
            .iter()
            .find(|sequence| sequence.id == id)
            .ok_or_else(|| format!("no sequence `{id}`")),
        None => project
            .sequences
            .first()
            .ok_or_else(|| "the project has no sequence".to_owned()),
    }
}

/// `export_captions` as an srt or vtt file.
pub(crate) fn export_captions(
    client: &RuntimeClient,
    directory: &Path,
    request: &CaptionsRequest,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let format = request.format.as_str();
    let project = project(client)?;
    let master = latest_master(&client.list_renders()?);
    let made = sequence(&project, request.sequence.as_deref()).and_then(|chosen| {
        let text = captions(chosen, format, request.track.as_deref())?;
        let path = target(
            directory,
            request.path.as_deref(),
            master.as_deref(),
            &project.name,
            format,
        )?;
        write_atomic(directory, &path, &text)?;
        Ok(path)
    });
    match made {
        Ok(path) => emit(tx, completed("captions", format, &path)),
        Err(message) => failed(tx, "export_captions", "captions", format, &message),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program at 24 fps starting at 01:00:00:00, with a marker, two
    /// cuts and a caption.
    const PROGRAM: &str = "kerf 1\nproject name=\"Atlas\"\n\
        media m1 path=\"media/A001_C003.mov\" rate=24 dur=00:00:04:00 video=320x180 audio=2ch@48000\n\
        sequence s1 rate=24 size=320x180 audio=48000 start=01:00:00:00\n\
        \x20 track V1 video\n\
        \x20   clip c1 media=m1 src=00:00:00:00 at=01:00:00:00 dur=00:00:02:00\n\
        \x20   clip c2 media=m1 src=00:00:02:00 at=01:00:02:00 dur=00:00:01:00\n\
        \x20 track S1 subtitle\n\
        \x20   clip t1 caption=\"Hello <there> & you\" at=01:00:00:00 dur=00:00:01:00\n\
        \x20   clip t2 caption=\"Second line\" at=01:00:01:12 dur=00:00:00:12\n\
        \x20 marker mk1 at=01:00:00:12 name=\"Act 2\" color=green\n";

    fn program() -> Project {
        kerf_runtime::parse_document(PROGRAM).unwrap()
    }

    #[test]
    fn captions_match_their_golden_files() {
        let project = program();
        let sequence = &project.sequences[0];
        assert_eq!(
            captions(sequence, "srt", None).unwrap(),
            "1\n00:00:00,000 --> 00:00:01,000\nHello <there> & you\n\n\
             2\n00:00:01,500 --> 00:00:02,000\nSecond line\n\n"
        );
        assert_eq!(
            captions(sequence, "vtt", Some("S1")).unwrap(),
            "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello &lt;there&gt; &amp; you\n\n\
             00:00:01.500 --> 00:00:02.000\nSecond line\n\n"
        );
        assert!(captions(sequence, "srt", Some("V1"))
            .unwrap_err()
            .contains("no captions"));
        assert!(captions(sequence, "ass", None)
            .unwrap_err()
            .contains("srt or vtt"));
    }

    #[test]
    fn files_are_written_whole() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let path = root.join("deliveries/a/Atlas.srt");
        write_atomic(root, &path, "one").unwrap();
        write_atomic(root, &path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let left: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file stays");
        let elsewhere = std::env::temp_dir().join("kerf-no-such-folder-9f1/Atlas.srt");
        assert!(write_atomic(root, &elsewhere, "x")
            .unwrap_err()
            .contains("not a folder"));
    }
}
