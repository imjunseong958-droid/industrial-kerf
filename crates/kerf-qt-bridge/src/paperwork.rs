//! The paperwork that travels with a master: `export_comments {format:
//! csv|edl|markers, includeResolved, path?}` and `export_captions {format:
//! srt|vtt, path?, sequence?, track?}`.
//!
//! - csv: kerfd's review log as a sheet (`kerf_runtime::review::to_csv`),
//!   timecodes on the program sequence.
//! - edl: a CMX3600 EDL of the program sequence
//!   (`kerf_interchange::edl::to_edl`), with a `* LOC:` locator for each
//!   sequence marker and each kept comment that has a time.
//! - markers: one edit that adds a sequence marker for each kept comment
//!   with a time (or brings the one an earlier export made up to date), so
//!   exporting twice never doubles them.
//! - srt / vtt: the sequence's captions (`kerf_lang::to_srt` / `to_vtt`).
//!
//! Comments are kept when they are open, and also when resolved with
//! `includeResolved`. Files go where `path` says (a file, or a folder for
//! the default name; relative to the project folder), else beside the
//! newest finished master, else in the project's `deliveries/` folder.
//! They are written to a temporary file beside the target and renamed over
//! it, so a reader never sees half a file; paperwork is made again for each
//! master, so an earlier file of that name is replaced.

use crate::emit;
use kerf_interchange::edl::{self, Locator};
use kerf_model::{Project, Sequence, Time};
use kerf_runtime::{Comment, EditOp, RenderJob, RuntimeClient, RuntimeError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;

/// The most characters of a comment a marker's name or a locator carries.
const MAX_LABEL_CHARS: usize = 120;
/// The folder paperwork goes in when no master was rendered yet.
pub(crate) const DELIVERIES: &str = "deliveries";

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(crate) struct CommentsRequest {
    /// `csv`, `edl` or `markers`.
    pub format: String,
    #[serde(rename = "includeResolved", default)]
    pub include_resolved: bool,
    #[serde(default)]
    pub path: Option<String>,
}

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

/// The comments paperwork carries: open ones, and resolved ones when asked.
fn kept(comments: &[Comment], include_resolved: bool) -> impl Iterator<Item = &Comment> {
    comments
        .iter()
        .filter(move |comment| include_resolved || !comment.resolved)
}

/// The comments that sit at a time on `sequence`, with that time on its
/// frame grid.
fn timed<'a>(
    comments: &'a [Comment],
    include_resolved: bool,
    sequence: &Sequence,
) -> Vec<(&'a Comment, Time)> {
    kept(comments, include_resolved)
        .filter_map(|comment| {
            let at = comment.at.filter(|at| at.is_finite() && *at >= 0.0)?;
            Some((comment, sequence.rate.round(Time::from_seconds(at)?)))
        })
        .collect()
}

/// A comment on one line: `Author: body`, at most [`MAX_LABEL_CHARS`].
fn label(comment: &Comment) -> String {
    let body = comment
        .body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!("{}: {body}", comment.author.name.trim());
    match text.char_indices().nth(MAX_LABEL_CHARS) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text,
    }
}

/// The marker colour of a comment: blocking notes red, QC orange, Kerf
/// blue, MCP agents purple, people yellow.
pub(crate) fn color(comment: &Comment) -> &'static str {
    if comment.blocking {
        return "red";
    }
    match comment.author.kind.as_str() {
        "qc" => "orange",
        "kerf" => "blue",
        "mcp" => "purple",
        _ => "yellow",
    }
}

/// The id of the marker a comment becomes, so a second export updates it.
pub(crate) fn marker_id(comment: &Comment) -> String {
    let mut id: String = std::iter::once('n')
        .chain("ote-".chars())
        .chain(comment.id.chars().map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        }))
        .collect();
    id.truncate(kerf_model::Id::MAX_LEN);
    id
}

/// The comments sheet.
pub(crate) fn csv(comments: &[Comment], include_resolved: bool, sequence: &Sequence) -> String {
    kerf_runtime::review::to_csv(comments, include_resolved, sequence)
}

/// The EDL of `sequence` with its markers and the kept comments as
/// locators.
pub(crate) fn edl(
    project: &Project,
    sequence: &Sequence,
    comments: &[Comment],
    include_resolved: bool,
) -> Result<String, String> {
    let mut locators = edl::marker_locators(sequence);
    for (comment, at) in timed(comments, include_resolved, sequence) {
        let color = kerf_model::MarkerColor::parse(color(comment));
        locators.push(Locator::new(at, edl::locator_color(color), label(comment)));
    }
    locators.sort_by_key(|locator| locator.at);
    // An unnamed program takes the project's name as the EDL's title.
    let mut titled = sequence.clone();
    if titled.name.trim().is_empty() {
        titled.name = project.name.clone();
    }
    edl::to_edl(project, &titled, &locators).map_err(|error| error.to_string())
}

/// The edit that makes the kept comments sequence markers on `sequence`:
/// named `Author: body`, noted with the body, coloured by who wrote them,
/// as long as their range. A marker an earlier export made is set again.
pub(crate) fn marker_ops(
    sequence: &Sequence,
    comments: &[Comment],
    include_resolved: bool,
) -> Vec<EditOp> {
    timed(comments, include_resolved, sequence)
        .into_iter()
        .map(|(comment, at)| {
            let id = marker_id(comment);
            let at = sequence.rate.frames_round(at);
            let frames = comment
                .duration
                .filter(|duration| duration.is_finite() && *duration > 0.0)
                .and_then(Time::from_seconds)
                .map_or(0, |duration| sequence.rate.frames_round(duration));
            let (name, note, color) = (
                Some(label(comment)),
                Some(comment.body.clone()),
                Some(color(comment).to_owned()),
            );
            if sequence
                .markers
                .iter()
                .any(|marker| marker.id == id.as_str())
            {
                EditOp::SetMarker {
                    marker: id,
                    at: Some(at),
                    frames: Some(frames),
                    name,
                    note,
                    color,
                }
            } else {
                EditOp::AddMarker {
                    at,
                    frames,
                    id: Some(id),
                    name,
                    color,
                    note,
                    sequence: Some(sequence.id.to_string()),
                }
            }
        })
        .collect()
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
    match format {
        "csv" => format!("{base} comments.csv"),
        other => format!("{base}.{other}"),
    }
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

/// `export_comments` as a csv or edl file. Errors kerfd gives back are
/// returned (a lost connection must be seen); everything else is answered
/// by an event.
pub(crate) fn export_comments(
    client: &RuntimeClient,
    directory: &Path,
    request: &CommentsRequest,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let format = request.format.as_str();
    let fail = |message: &str| failed(tx, "export_comments", "paperwork", format, message);
    if !matches!(format, "csv" | "edl") {
        fail(&format!(
            "comments export as csv, edl or markers, not `{format}`"
        ));
        return Ok(());
    }
    let comments = client.comments()?.items;
    let project = project(client)?;
    let master = latest_master(&client.list_renders()?);
    let made = sequence(&project, None).and_then(|program| {
        let text = if format == "csv" {
            csv(&comments, request.include_resolved, program)
        } else {
            edl(&project, program, &comments, request.include_resolved)?
        };
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
        Ok(path) => emit(tx, completed("paperwork", format, &path)),
        Err(message) => fail(&message),
    }
    Ok(())
}

/// `export_comments {format: markers}`: the edit to apply, at the revision
/// it was made against, and how many markers it sets; `None` when it was
/// answered already (nothing to mark, or no sequence).
pub(crate) fn markers(
    client: &RuntimeClient,
    request: &CommentsRequest,
    tx: &SyncSender<Value>,
) -> Result<Option<(Vec<EditOp>, u64, usize)>, RuntimeError> {
    let comments = client.comments()?.items;
    let (revision, document) = client.get_document(false)?;
    let project = kerf_runtime::parse_document(&document)?;
    let program = match sequence(&project, None) {
        Ok(program) => program,
        Err(message) => {
            failed(tx, "export_comments", "paperwork", "markers", &message);
            return Ok(None);
        }
    };
    let ops = marker_ops(program, &comments, request.include_resolved);
    if ops.is_empty() {
        emit(tx, markers_done(0));
        return Ok(None);
    }
    let count = ops.len();
    Ok(Some((ops, revision, count)))
}

/// What a marker export answers once its edit is in.
pub(crate) fn markers_done(count: usize) -> Value {
    json!({"type": "export", "status": "completed", "kind": "paperwork", "format": "markers",
        "count": count, "progress": 1.0})
}

/// A marker export's edit was refused.
pub(crate) fn markers_failed(tx: &SyncSender<Value>, message: &str) {
    failed(tx, "export_comments", "paperwork", "markers", message);
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
    use kerf_runtime::Author;

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

    fn comment(id: &str, kind: &str, name: &str, at: Option<f64>, body: &str) -> Comment {
        Comment {
            id: id.into(),
            parent: None,
            author: Author::new(kind, name),
            at,
            duration: None,
            frame: None,
            body: body.into(),
            tags: Vec::new(),
            blocking: false,
            drawing: Vec::new(),
            proposal: None,
            steps: Vec::new(),
            error: false,
            resolved: false,
            resolved_by: None,
            resolved_revision: None,
            qc_key: None,
            created: format!("2026-11-04T10:12:0{}Z", id.len() % 10),
            revision: 3,
            edited: None,
        }
    }

    /// The fixture review log: a person's note over a range, a blocking QC
    /// finding, an untimed agent note, and a resolved note.
    fn comments() -> Vec<Comment> {
        let mut range = comment(
            "c-1",
            "person",
            "Mina Park",
            Some(0.5),
            "Open on the road,\nnot the sky",
        );
        range.duration = Some(1.0);
        range.tags = vec!["picture".into()];
        let mut qc = comment("c-2", "qc", "QC", Some(2.25), "Caption reads too fast");
        qc.blocking = true;
        qc.tags = vec!["captions".into()];
        let untimed = comment("c-3", "mcp", "Codex", None, "Checked the whole cut");
        let mut done = comment("c-4", "kerf", "Kerf", Some(1.0), "Trimmed the head");
        done.resolved = true;
        vec![range, qc, untimed, done]
    }

    #[test]
    fn paperwork_commands_read_as_the_contract_says() {
        let Ok(crate::Command::ExportComments(request)) = serde_json::from_value(json!({
            "type": "export_comments", "format": "edl", "includeResolved": true, "path": "deliveries/"
        })) else {
            panic!("expected export_comments");
        };
        assert_eq!(
            request,
            CommentsRequest {
                format: "edl".into(),
                include_resolved: true,
                path: Some("deliveries/".into())
            }
        );
        let Ok(crate::Command::ExportComments(plain)) =
            serde_json::from_value(json!({"type": "export_comments", "format": "csv"}))
        else {
            panic!("expected export_comments");
        };
        assert!(!plain.include_resolved && plain.path.is_none());
        let Ok(crate::Command::ExportCaptions(captions)) = serde_json::from_value(json!({
            "type": "export_captions", "format": "vtt", "sequence": "s1", "track": "S1"
        })) else {
            panic!("expected export_captions");
        };
        assert_eq!(captions.format, "vtt");
        assert_eq!(captions.sequence.as_deref(), Some("s1"));
        assert_eq!(captions.track.as_deref(), Some("S1"));
    }

    #[test]
    fn the_comments_sheet_matches_its_golden_file() {
        let project = program();
        let sheet = csv(&comments(), false, &project.sequences[0]);
        assert_eq!(
            sheet,
            "TC in,TC out,Author,Kind,Body,Status,Tags\r\n\
             01:00:00:12,01:00:01:12,Mina Park,person,\"Open on the road,\nnot the sky\",open,picture\r\n\
             01:00:02:06,01:00:02:06,QC,qc,Caption reads too fast,open,captions; blocking\r\n\
             ,,Codex,mcp,Checked the whole cut,open,\r\n"
        );
        let all = csv(&comments(), true, &project.sequences[0]);
        assert!(all.contains("Kerf,kerf,Trimmed the head,resolved"), "{all}");
    }

    #[test]
    fn the_edl_matches_its_golden_file() {
        let project = program();
        let text = edl(&project, &project.sequences[0], &comments(), false).unwrap();
        assert_eq!(
            text,
            "TITLE: Atlas\n\
             FCM: NON-DROP FRAME\n\
             \n\
             001  A001_C00 V     C        00:00:00:00 00:00:02:00 01:00:00:00 01:00:02:00\n\
             * FROM CLIP NAME: A001_C003.mov\n\
             * SOURCE FILE: A001_C003.mov\n\
             * LOC: 01:00:00:12 GREEN   Act 2\n\
             * LOC: 01:00:00:12 YELLOW  Mina Park: Open on the road, not the sky\n\
             \n\
             002  A001_C00 V     C        00:00:02:00 00:00:03:00 01:00:02:00 01:00:03:00\n\
             * FROM CLIP NAME: A001_C003.mov\n\
             * SOURCE FILE: A001_C003.mov\n\
             * LOC: 01:00:02:06 RED     QC: Caption reads too fast\n"
        );
        // A resolved note joins when asked.
        let all = edl(&project, &project.sequences[0], &comments(), true).unwrap();
        assert!(all.contains("BLUE    Kerf: Trimmed the head"), "{all}");
    }

    #[test]
    fn comments_become_markers_once() {
        let mut project = program();
        let ops = marker_ops(&project.sequences[0], &comments(), false);
        let ops = serde_json::to_value(&ops).unwrap();
        assert_eq!(
            ops,
            json!([
                {"op": "add-marker", "at": 12, "frames": 24, "id": "note-c-1",
                 "name": "Mina Park: Open on the road, not the sky",
                 "note": "Open on the road,\nnot the sky", "color": "yellow", "sequence": "s1"},
                {"op": "add-marker", "at": 54, "frames": 0, "id": "note-c-2",
                 "name": "QC: Caption reads too fast", "note": "Caption reads too fast",
                 "color": "red", "sequence": "s1"},
            ])
        );
        // Exported again after the first edit: the same markers are set.
        project.sequences[0].markers.push(kerf_model::Marker {
            id: kerf_model::Id::new("note-c-1").unwrap(),
            at: Time::ZERO,
            duration: Time::ZERO,
            name: None,
            color: None,
            note: None,
        });
        let again = marker_ops(&project.sequences[0], &comments(), true);
        assert!(
            matches!(&again[0], EditOp::SetMarker { marker, at: Some(12), .. } if marker == "note-c-1")
        );
        assert_eq!(again.len(), 3, "the resolved note joins: {again:?}");
        assert!(
            matches!(&again[2], EditOp::AddMarker { color: Some(color), .. } if color == "blue")
        );
    }

    #[test]
    fn marker_ids_are_valid_ids() {
        let odd = comment("c 1/ü", "person", "A", Some(0.0), "x");
        assert_eq!(marker_id(&odd), "note-c-1--");
        let long = comment(&"x".repeat(100), "person", "A", Some(0.0), "x");
        assert_eq!(marker_id(&long).len(), kerf_model::Id::MAX_LEN);
        assert!(kerf_model::Id::is_valid(&marker_id(&long)));
    }

    #[test]
    fn labels_stay_on_one_short_line() {
        let long = comment("c-9", "person", "Mina", Some(0.0), &"word ".repeat(60));
        let text = label(&long);
        assert_eq!(text.chars().count(), MAX_LABEL_CHARS + 1);
        assert!(text.ends_with('…') && !text.contains('\n'));
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
    fn paperwork_goes_beside_the_master_else_in_deliveries() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let master = root.join("deliveries/atlas_v7/Atlas master.mov");
        assert_eq!(
            target(root, None, Some(&master), "Atlas", "csv").unwrap(),
            root.join("deliveries/atlas_v7/Atlas master comments.csv")
        );
        assert_eq!(
            target(root, None, Some(&master), "Atlas", "edl").unwrap(),
            root.join("deliveries/atlas_v7/Atlas master.edl")
        );
        assert_eq!(
            target(root, None, None, "Atlas: cut", "srt").unwrap(),
            root.join("deliveries/Atlas- cut.srt")
        );
        assert_eq!(
            target(root, Some("out/"), None, "Atlas", "vtt").unwrap(),
            root.join("out/Atlas.vtt")
        );
        assert_eq!(
            target(root, Some("notes.csv"), Some(&master), "Atlas", "csv").unwrap(),
            root.join("notes.csv")
        );
        let job = |status: &str, quality: &str, path: &Path| RenderJob {
            job_id: "render-1".into(),
            status: status.into(),
            output_url: None,
            output_path: Some(path.to_string_lossy().into_owned()),
            error: None,
            range: None,
            sequence: None,
            progress: 1.0,
            name: None,
            quality: Some(quality.into()),
            codec: None,
        };
        fs::create_dir_all(master.parent().unwrap()).unwrap();
        fs::write(&master, b"mov").unwrap();
        let older = root.join("Old.mov");
        fs::write(&older, b"mov").unwrap();
        let jobs = [
            job("completed", "master", &older),
            job("completed", "master", &master),
            job("completed", "review", &root.join("Draft.mp4")),
            job("completed", "master", &root.join("Gone.mov")),
            job("running", "master", &root.join("Next.mov")),
        ];
        assert_eq!(latest_master(&jobs).as_deref(), Some(master.as_path()));
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
