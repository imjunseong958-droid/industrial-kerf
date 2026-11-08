//! The review log: comment threads kept beside the project, not in it.
//!
//! Notes from Kerf, MCP agents, people and QC live in
//! `review/comments.jsonl` in the project folder, one operation per line:
//! `add`, `edit`, `tag`, `resolve`, `reopen` or `delete`. The file is only
//! ever appended to, so Git can merge it with `merge=union`; [`fold`] reads
//! it back deterministically, dropping the duplicate lines a union merge can
//! leave (the same `id`, `op` and `created`), into the comments as they
//! stand, ordered by when they were made.
//!
//! A comment never touches the document: it is not an edit, it does not
//! enter undo, and it never makes a pending proposal stale. Times are
//! seconds on the program sequence; drawings are normalised 0..1 to the
//! frame.

use crate::{io_error, unique_id, RuntimeError};
use kerf_model::time::format_timecode;
use kerf_model::{Project, Sequence, Time};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The review log, relative to the project folder.
pub const COMMENTS_FILE: &str = "review/comments.jsonl";

/// Who may write a comment.
pub const AUTHOR_KINDS: [&str; 4] = ["kerf", "mcp", "person", "qc"];

const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 256;
const MAX_TAGS: usize = 32;
const MAX_STEPS: usize = 256;
const MAX_STROKES: usize = 64;
const MAX_DRAWING_POINTS: usize = 20_000;
const DRAWING_TOOLS: [&str; 4] = ["pen", "ellipse", "rect", "arrow"];

/// Who wrote a comment or a proposal: `kind` is `kerf`, `mcp`, `person` or
/// `qc`; `id` is a person's email or an agent's client id; `role` is shown
/// beside the name (`MCP · via Kerf`, `loudness`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

impl Author {
    pub fn new(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            id: None,
            role: None,
        }
    }

    /// Kerf's own agent.
    pub fn kerf() -> Self {
        Self::new("kerf", "Kerf")
    }

    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        if !AUTHOR_KINDS.contains(&self.kind.as_str()) {
            return Err(invalid("author kind must be kerf, mcp, person, or qc"));
        }
        if self.name.trim().is_empty() || self.name.len() > MAX_NAME_BYTES {
            return Err(invalid("author name must be 1 to 256 bytes"));
        }
        for text in [&self.id, &self.role].into_iter().flatten() {
            if text.len() > MAX_NAME_BYTES {
                return Err(invalid("author id and role must be at most 256 bytes"));
            }
        }
        Ok(())
    }
}

/// One stroke drawn on the frame: points normalised 0..1 to the picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// `pen`, `ellipse`, `rect` or `arrow`.
    pub tool: String,
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// A comment as it stands after folding the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    /// The root of the thread this replies to.
    pub parent: Option<String>,
    pub author: Author,
    /// Seconds on the program sequence.
    pub at: Option<f64>,
    pub duration: Option<f64>,
    /// `at` on the sequence's frame grid, when the comment was made.
    pub frame: Option<i64>,
    pub body: String,
    pub tags: Vec<String>,
    pub blocking: bool,
    pub drawing: Vec<Stroke>,
    /// The proposal this comment is about.
    pub proposal: Option<String>,
    /// What an agent did on the way to this reply.
    pub steps: Vec<String>,
    /// An agent turn that failed.
    pub error: bool,
    pub resolved: bool,
    pub resolved_by: Option<Author>,
    /// The project revision a resolve was made at; for a comment resolved
    /// by accepting its proposal, the revision the proposal made.
    pub resolved_revision: Option<u64>,
    /// The QC finding a `qc` comment mirrors.
    pub qc_key: Option<String>,
    /// RFC 3339.
    pub created: String,
    /// The project revision the comment was made at.
    pub revision: u64,
    /// When the body was last edited, RFC 3339.
    pub edited: Option<String>,
}

/// A comment as a client writes it. The runtime fills in when it was made,
/// the revision, and an id when none is given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewComment {
    /// A client-chosen id makes a retried add harmless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub author: Author,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<i64>,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drawing: Vec<Stroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qc_key: Option<String>,
}

impl NewComment {
    /// A comment by `author` saying `body`.
    pub fn new(author: Author, body: impl Into<String>) -> Self {
        Self {
            id: None,
            parent: None,
            author,
            at: None,
            duration: None,
            frame: None,
            body: body.into(),
            tags: Vec::new(),
            blocking: false,
            drawing: Vec::new(),
            proposal: None,
            steps: Vec::new(),
            error: false,
            qc_key: None,
        }
    }
}

/// What a `SetComment` changes; absent fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
    /// Link the comment to a proposal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<String>>,
    /// Who resolved, reopened or deleted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Author>,
}

/// The comments as they stand, and the counter that changes whenever the
/// log does (a write here, or a file Git or another process changed).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLog {
    pub items: Vec<Comment>,
    pub review_revision: u64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::new("INVALID_COMMENT", message)
}

fn not_found(id: &str) -> RuntimeError {
    RuntimeError::new("COMMENT_NOT_FOUND", format!("comment not found: {id}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Op {
    Add,
    Edit,
    Tag,
    Resolve,
    Reopen,
    Delete,
}

/// One line of the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    op: Op,
    id: String,
    /// When this operation was made, RFC 3339 in UTC with microseconds, so
    /// lines sort by time as text.
    created: String,
    /// The runtime actor that wrote it: `ui`, `agent`, `mcp` or `system`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    by: Option<Author>,
    /// The project revision the operation was made at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
    /// `add`: the comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comment: Option<NewComment>,
    /// `edit`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proposal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    steps: Option<Vec<String>>,
    /// `tag`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocking: Option<bool>,
}

impl Entry {
    fn new(op: Op, id: &str, actor: Option<&str>, revision: u64) -> Self {
        Self {
            op,
            id: id.to_owned(),
            created: timestamp(),
            actor: actor.map(str::to_owned),
            by: None,
            revision: Some(revision),
            comment: None,
            body: None,
            proposal: None,
            steps: None,
            tags: None,
            blocking: None,
        }
    }
}

/// The comments a log's text describes, ordered by when they were made.
/// Lines that do not read are skipped; a line repeated by a union merge
/// counts once; an operation on a comment the log never added is ignored.
pub fn fold(text: &str) -> Vec<Comment> {
    let mut seen = HashSet::new();
    let mut entries: Vec<Entry> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
        .filter(|entry| seen.insert((entry.id.clone(), entry.op, entry.created.clone())))
        .collect();
    // Clocks on two machines need not agree, so every add is applied
    // before any change to it; the rest apply in time order, ties broken
    // by operation and then by content, which every clone sees alike.
    entries.sort_by(|a, b| {
        (a.op != Op::Add, &a.created, a.op, &a.id)
            .cmp(&(b.op != Op::Add, &b.created, b.op, &b.id))
            .then_with(|| {
                let text = |entry: &Entry| serde_json::to_string(entry).unwrap_or_default();
                text(a).cmp(&text(b))
            })
    });
    let mut comments: Vec<(Comment, bool)> = Vec::new();
    for entry in entries {
        if entry.op == Op::Add {
            if comments.iter().any(|(comment, _)| comment.id == entry.id) {
                continue;
            }
            let Some(added) = entry.comment else {
                continue;
            };
            comments.push((
                Comment {
                    id: entry.id,
                    parent: added.parent,
                    author: added.author,
                    at: added.at,
                    duration: added.duration,
                    frame: added.frame,
                    body: added.body,
                    tags: added.tags,
                    blocking: added.blocking,
                    drawing: added.drawing,
                    proposal: added.proposal,
                    steps: added.steps,
                    error: added.error,
                    resolved: false,
                    resolved_by: None,
                    resolved_revision: None,
                    qc_key: added.qc_key,
                    created: entry.created,
                    revision: entry.revision.unwrap_or_default(),
                    edited: None,
                },
                false,
            ));
            continue;
        }
        let Some((comment, deleted)) = comments
            .iter_mut()
            .find(|(comment, _)| comment.id == entry.id)
        else {
            continue;
        };
        match entry.op {
            Op::Add => {}
            Op::Edit => {
                if let Some(body) = entry.body {
                    comment.body = body;
                    comment.edited = Some(entry.created);
                }
                if let Some(proposal) = entry.proposal {
                    comment.proposal = Some(proposal);
                }
                if let Some(steps) = entry.steps {
                    comment.steps = steps;
                }
            }
            Op::Tag => {
                if let Some(tags) = entry.tags {
                    comment.tags = tags;
                }
                if let Some(blocking) = entry.blocking {
                    comment.blocking = blocking;
                }
            }
            Op::Resolve => {
                comment.resolved = true;
                comment.resolved_by = entry.by;
                comment.resolved_revision = entry.revision;
            }
            Op::Reopen => {
                comment.resolved = false;
                comment.resolved_by = None;
                comment.resolved_revision = None;
            }
            Op::Delete => *deleted = true,
        }
    }
    let mut comments: Vec<Comment> = comments
        .into_iter()
        .filter(|(_, deleted)| !deleted)
        .map(|(comment, _)| comment)
        .collect();
    comments.sort_by(|a, b| (&a.created, &a.id).cmp(&(&b.created, &b.id)));
    comments
}

/// The comments as a CSV sheet: `TC in, TC out, Author, Kind, Body, Status,
/// Tags`, in timecode order (comments without a time last), with the
/// timecode of `sequence`. Resolved comments are left out unless
/// `include_resolved`.
pub fn to_csv(comments: &[Comment], include_resolved: bool, sequence: &Sequence) -> String {
    let timecode = |seconds: Option<f64>| {
        seconds
            .and_then(Time::from_seconds)
            .map(|time| format_timecode(sequence.start + time, sequence.rate, sequence.drop_frame))
            .unwrap_or_default()
    };
    let mut rows: Vec<&Comment> = comments
        .iter()
        .filter(|comment| include_resolved || !comment.resolved)
        .collect();
    rows.sort_by(|a, b| {
        let at = |comment: &Comment| comment.at.unwrap_or(f64::INFINITY);
        at(a)
            .total_cmp(&at(b))
            .then_with(|| a.created.cmp(&b.created))
    });
    let mut text = String::from("TC in,TC out,Author,Kind,Body,Status,Tags\r\n");
    for comment in rows {
        let mut tags = comment.tags.clone();
        if comment.blocking && !tags.iter().any(|tag| tag == "blocking") {
            tags.push("blocking".into());
        }
        let out = comment
            .at
            .map(|at| at + comment.duration.unwrap_or_default());
        let cells = [
            timecode(comment.at),
            timecode(out),
            comment.author.name.clone(),
            comment.author.kind.clone(),
            comment.body.clone(),
            if comment.resolved { "resolved" } else { "open" }.to_owned(),
            tags.join("; "),
        ];
        let line: Vec<String> = cells.iter().map(|cell| csv_cell(cell)).collect();
        text.push_str(&line.join(","));
        text.push_str("\r\n");
    }
    text
}

/// The threads that hold delivery back, by their roots in the order they
/// were made: an open thread with a comment in it marked blocking (or
/// tagged `blocking`) and not resolved. The editor's Deliver tray and
/// toolbar count by the same rule.
pub fn blocking_threads(comments: &[Comment]) -> Vec<&Comment> {
    let held: HashSet<&str> = comments
        .iter()
        .filter(|comment| {
            !comment.resolved
                && (comment.blocking || comment.tags.iter().any(|tag| tag == "blocking"))
        })
        .map(|comment| comment.parent.as_deref().unwrap_or(&comment.id))
        .collect();
    comments
        .iter()
        .filter(|comment| {
            comment.parent.is_none() && !comment.resolved && held.contains(comment.id.as_str())
        })
        .collect()
}

/// The refusal of a delivery (a `full` or `master` render) while
/// `comments` hold it back: the first note, where it is on `sequence` (the
/// program), and how to clear it. `None` when nothing does.
pub fn delivery_held(comments: &[Comment], sequence: Option<&Sequence>) -> Option<RuntimeError> {
    let threads = blocking_threads(comments);
    let first = *threads.first()?;
    let at = first
        .at
        .and_then(Time::from_seconds)
        .zip(sequence)
        .map(|(time, sequence)| {
            format!(
                " at {}",
                format_timecode(sequence.start + time, sequence.rate, sequence.drop_frame)
            )
        })
        .unwrap_or_default();
    let body: String = first.body.chars().take(80).collect();
    let body = if body.len() < first.body.len() {
        format!("{}…", body.trim_end())
    } else {
        body
    };
    let more = match threads.len() {
        1 => String::new(),
        2 => " and 1 more".to_owned(),
        n => format!(" and {} more", n - 1),
    };
    Some(RuntimeError::new(
        "BLOCKING_NOTES",
        format!(
            "a blocking note holds delivery back: \"{body}\" by {}{at} (comment {}){more}; \
             resolve it in the review once it is dealt with, or render a draft or a review copy meanwhile",
            first.author.name, first.id
        ),
    ))
}

/// One CSV cell, quoted when it must be, and kept from reading as a
/// formula in a spreadsheet.
fn csv_cell(text: &str) -> String {
    let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// Now, in RFC 3339 (UTC, microseconds). Later calls in one process always
/// read later, so two operations never share a time.
pub fn timestamp() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    let mut last = LAST.load(Ordering::Relaxed);
    let micros = loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break next,
            Err(current) => last = current,
        }
    };
    rfc3339_micros(micros)
}

/// `micros` since the Unix epoch as RFC 3339 in UTC.
pub fn rfc3339_micros(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{fraction:06}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn validate_text_list(list: &[String], limit: usize, what: &str) -> Result<(), RuntimeError> {
    if list.len() > limit || list.iter().any(|item| item.len() > MAX_BODY_BYTES) {
        return Err(RuntimeError::new(
            "INVALID_COMMENT",
            format!("a comment holds at most {limit} {what}"),
        ));
    }
    Ok(())
}

fn validate_tags(tags: &[String]) -> Result<(), RuntimeError> {
    validate_text_list(tags, MAX_TAGS, "tags")?;
    if tags
        .iter()
        .any(|tag| tag.trim().is_empty() || tag.len() > MAX_NAME_BYTES)
    {
        return Err(invalid("tags must be 1 to 256 bytes"));
    }
    Ok(())
}

fn validate_time(value: Option<f64>, what: &str) -> Result<(), RuntimeError> {
    match value {
        Some(value) if !value.is_finite() || value < 0.0 => Err(RuntimeError::new(
            "INVALID_COMMENT",
            format!("{what} must be a finite number of seconds, at least 0"),
        )),
        _ => Ok(()),
    }
}

fn validate_drawing(drawing: &[Stroke]) -> Result<(), RuntimeError> {
    if drawing.len() > MAX_STROKES
        || drawing
            .iter()
            .map(|stroke| stroke.points.len())
            .sum::<usize>()
            > MAX_DRAWING_POINTS
    {
        return Err(invalid("the drawing is too large"));
    }
    for stroke in drawing {
        if !DRAWING_TOOLS.contains(&stroke.tool.as_str()) {
            return Err(invalid("drawing tools are pen, ellipse, rect, and arrow"));
        }
        if stroke
            .points
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || !(-1.0..=2.0).contains(value))
        {
            return Err(invalid(
                "drawing points are normalised to the frame (0 to 1)",
            ));
        }
        if stroke.color.as_ref().is_some_and(|color| color.len() > 32) {
            return Err(invalid(
                "a drawing colour is a short colour name or #rrggbb",
            ));
        }
    }
    Ok(())
}

fn validate_body(body: &str) -> Result<(), RuntimeError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(invalid("a comment body is at most 64 KiB"));
    }
    Ok(())
}

/// The log as the runtime keeps it: the comments folded from the file, and
/// the counter clients watch for changes.
#[derive(Debug)]
pub(crate) struct Review {
    path: PathBuf,
    /// The file's length and modification time when it was last read.
    signature: Option<(u64, Option<SystemTime>)>,
    comments: Vec<Comment>,
    revision: u64,
}

impl Review {
    pub(crate) fn open(directory: &Path) -> Self {
        let mut review = Self {
            path: directory.join(COMMENTS_FILE),
            signature: None,
            comments: Vec::new(),
            revision: 0,
        };
        review.reload();
        // Start from the number of comments so a log with comments never
        // reads as unchanged from an empty one.
        review.revision = review.comments.len() as u64;
        review
    }

    fn file_signature(&self) -> Option<(u64, Option<SystemTime>)> {
        fs::metadata(&self.path)
            .ok()
            .map(|metadata| (metadata.len(), metadata.modified().ok()))
    }

    fn reload(&mut self) {
        self.signature = self.file_signature();
        self.comments = fs::read_to_string(&self.path)
            .map(|text| fold(&text))
            .unwrap_or_default();
    }

    /// Read the file again when something else changed it: a Git merge, a
    /// branch switch, or a copy of the project.
    pub(crate) fn refresh(&mut self) {
        if self.file_signature() != self.signature {
            self.reload();
            self.revision += 1;
        }
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn log(&self) -> ReviewLog {
        ReviewLog {
            items: self.comments.clone(),
            review_revision: self.revision,
        }
    }

    pub(crate) fn comments(&self) -> &[Comment] {
        &self.comments
    }

    pub(crate) fn comment(&self, id: &str) -> Option<&Comment> {
        self.comments.iter().find(|comment| comment.id == id)
    }

    fn append(&mut self, entries: &[Entry]) -> Result<(), RuntimeError> {
        if entries.is_empty() {
            return Ok(());
        }
        // Another writer may have changed the file since it was read.
        self.refresh();
        let mut bytes = Vec::new();
        for entry in entries {
            serde_json::to_writer(&mut bytes, entry)
                .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
            bytes.push(b'\n');
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)
            .map_err(io_error)?;
        // A line torn by a crash stays one bad line, which fold skips; start
        // on a fresh line after one.
        let mut last = *b"\n";
        if file.seek(SeekFrom::End(-1)).is_ok() {
            file.read_exact(&mut last).map_err(io_error)?;
        }
        if last[0] != b'\n' {
            file.write_all(b"\n").map_err(io_error)?;
        }
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        self.reload();
        self.revision += 1;
        Ok(())
    }

    /// Add `new` at project `revision`, its frame on the first sequence of
    /// `project` when it has a time and no frame. An add whose id is already
    /// taken returns that comment, as does a `qc` comment whose `qcKey` an
    /// existing comment has.
    pub(crate) fn add(
        &mut self,
        actor: &str,
        mut new: NewComment,
        project: &Project,
        revision: u64,
    ) -> Result<Comment, RuntimeError> {
        crate::validate_actor(actor)?;
        self.refresh();
        new.author.validate()?;
        validate_body(&new.body)?;
        if new.body.trim().is_empty() && new.drawing.is_empty() {
            return Err(invalid("a comment needs a body or a drawing"));
        }
        validate_time(new.at, "at")?;
        validate_time(new.duration, "duration")?;
        validate_tags(&new.tags)?;
        validate_text_list(&new.steps, MAX_STEPS, "steps")?;
        validate_drawing(&new.drawing)?;
        if let Some(id) = &new.id {
            if id.trim().is_empty() || id.len() > MAX_NAME_BYTES {
                return Err(invalid("a comment id must be 1 to 256 bytes"));
            }
            if let Some(existing) = self.comment(id) {
                return Ok(existing.clone());
            }
        }
        if let Some(key) = &new.qc_key {
            if let Some(existing) = self
                .comments
                .iter()
                .find(|comment| comment.qc_key.as_ref() == Some(key))
            {
                return Ok(existing.clone());
            }
        }
        if let Some(parent) = &new.parent {
            if self.comment(parent).is_none() {
                return Err(not_found(parent));
            }
        }
        if new.frame.is_none() {
            if let (Some(at), Some(sequence)) = (new.at, project.sequences.first()) {
                new.frame = Some((at * sequence.rate.fps()).round() as i64);
            }
        }
        let id = new
            .id
            .take()
            .unwrap_or_else(|| format!("c-{}", unique_id()));
        let mut entry = Entry::new(Op::Add, &id, Some(actor), revision);
        entry.comment = Some(new);
        self.append(&[entry])?;
        self.comment(&id).cloned().ok_or_else(|| {
            RuntimeError::new(
                "REVIEW_LOG_ERROR",
                "the review log did not keep the comment",
            )
        })
    }

    /// Change comment `id` as `patch` says, at project `revision`. Setting
    /// what is already so writes nothing. Returns the comment, or `None`
    /// once deleted.
    pub(crate) fn set(
        &mut self,
        actor: &str,
        id: &str,
        patch: CommentPatch,
        revision: u64,
    ) -> Result<Option<Comment>, RuntimeError> {
        crate::validate_actor(actor)?;
        self.refresh();
        let current = self.comment(id).cloned().ok_or_else(|| not_found(id))?;
        if let Some(by) = &patch.by {
            by.validate()?;
        }
        let mut entries = Vec::new();
        let mut edit = Entry::new(Op::Edit, id, Some(actor), revision);
        if let Some(body) = patch.body.filter(|body| *body != current.body) {
            validate_body(&body)?;
            if body.trim().is_empty() && current.drawing.is_empty() {
                return Err(invalid("a comment needs a body or a drawing"));
            }
            edit.body = Some(body);
        }
        edit.proposal = patch
            .proposal
            .filter(|proposal| current.proposal.as_ref() != Some(proposal));
        if let Some(steps) = patch.steps.filter(|steps| *steps != current.steps) {
            validate_text_list(&steps, MAX_STEPS, "steps")?;
            edit.steps = Some(steps);
        }
        if edit.body.is_some() || edit.proposal.is_some() || edit.steps.is_some() {
            entries.push(edit);
        }
        let mut tag = Entry::new(Op::Tag, id, Some(actor), revision);
        if let Some(tags) = patch.tags.filter(|tags| *tags != current.tags) {
            validate_tags(&tags)?;
            tag.tags = Some(tags);
        }
        tag.blocking = patch
            .blocking
            .filter(|blocking| *blocking != current.blocking);
        if tag.tags.is_some() || tag.blocking.is_some() {
            entries.push(tag);
        }
        if let Some(resolved) = patch
            .resolved
            .filter(|resolved| *resolved != current.resolved)
        {
            let op = if resolved { Op::Resolve } else { Op::Reopen };
            let mut entry = Entry::new(op, id, Some(actor), revision);
            entry.by = patch.by.clone();
            entries.push(entry);
        }
        if patch.deleted == Some(true) {
            let mut entry = Entry::new(Op::Delete, id, Some(actor), revision);
            entry.by = patch.by;
            entries.push(entry);
        }
        self.append(&entries)?;
        Ok(self.comment(id).cloned())
    }

    /// Resolve the open comments about any of `proposals`, as accepting
    /// one at `revision` does. Whether any were.
    pub(crate) fn resolve_proposal(
        &mut self,
        proposals: &[String],
        revision: u64,
        by: Option<Author>,
    ) -> Result<bool, RuntimeError> {
        self.refresh();
        let entries: Vec<Entry> = self
            .comments
            .iter()
            .filter(|comment| {
                !comment.resolved
                    && comment
                        .proposal
                        .as_ref()
                        .is_some_and(|proposal| proposals.contains(proposal))
            })
            .map(|comment| {
                let mut entry = Entry::new(Op::Resolve, &comment.id, Some("system"), revision);
                entry.by = by.clone();
                entry
            })
            .collect();
        let any = !entries.is_empty();
        self.append(&entries)?;
        Ok(any)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("kerf-review-{label}-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn project() -> Project {
        crate::parse_document(
            "kerf 1\nproject name=\"Review\"\nsequence s1 rate=25 size=1920x1080 audio=48000\n",
        )
        .unwrap()
    }

    fn person(name: &str) -> Author {
        Author {
            id: Some(format!("{}@example.com", name.to_lowercase())),
            ..Author::new("person", name)
        }
    }

    #[test]
    fn timestamps_are_rfc3339_and_always_increase() {
        assert_eq!(rfc3339_micros(0), "1970-01-01T00:00:00.000000Z");
        assert_eq!(
            rfc3339_micros(1_793_787_120_123_456),
            "2026-11-04T10:12:00.123456Z"
        );
        assert_eq!(
            rfc3339_micros(951_782_400_000_000),
            "2000-02-29T00:00:00.000000Z"
        );
        let first = timestamp();
        let second = timestamp();
        assert!(second > first, "{first} then {second}");
    }

    #[test]
    fn a_thread_is_added_replied_to_resolved_and_reopened() {
        let directory = directory("thread");
        let mut review = Review::open(&directory);
        assert_eq!(review.revision(), 0);
        let mut note = NewComment::new(person("Mina"), "The flare hides the logo");
        note.at = Some(17.32);
        note.duration = Some(2.48);
        note.tags = vec!["picture".into()];
        note.drawing = vec![Stroke {
            tool: "ellipse".into(),
            points: vec![[0.61, 0.22], [0.74, 0.38]],
            color: Some("#ededed".into()),
        }];
        let root = review.add("ui", note, &project(), 22).unwrap();
        assert_eq!(root.frame, Some(433));
        assert_eq!(root.revision, 22);
        assert!(root.id.starts_with("c-"));
        assert_eq!(review.revision(), 1);

        let mut reply = NewComment::new(Author::kerf(), "Masked the flare");
        reply.parent = Some(root.id.clone());
        reply.steps = vec!["Sampled the sky".into()];
        let reply = review.add("agent", reply, &project(), 22).unwrap();
        assert_eq!(reply.parent.as_deref(), Some(root.id.as_str()));

        let resolved = review
            .set(
                "ui",
                &root.id,
                CommentPatch {
                    resolved: Some(true),
                    by: Some(person("Tom")),
                    ..CommentPatch::default()
                },
                23,
            )
            .unwrap()
            .unwrap();
        assert!(resolved.resolved);
        assert_eq!(resolved.resolved_revision, Some(23));
        assert_eq!(resolved.resolved_by.as_ref().unwrap().name, "Tom");

        // Setting what is already so writes nothing.
        let before = review.revision();
        review
            .set(
                "ui",
                &root.id,
                CommentPatch {
                    resolved: Some(true),
                    ..CommentPatch::default()
                },
                23,
            )
            .unwrap();
        assert_eq!(review.revision(), before);

        let reopened = review
            .set(
                "ui",
                &root.id,
                CommentPatch {
                    resolved: Some(false),
                    body: Some("The flare still hides the logo".into()),
                    tags: Some(vec!["picture".into(), "logo".into()]),
                    blocking: Some(true),
                    ..CommentPatch::default()
                },
                24,
            )
            .unwrap()
            .unwrap();
        assert!(!reopened.resolved);
        assert_eq!(reopened.resolved_revision, None);
        assert_eq!(reopened.body, "The flare still hides the logo");
        assert!(reopened.edited.is_some());
        assert!(reopened.blocking);
        assert_eq!(reopened.tags, ["picture", "logo"]);

        // The file folds back to what the runtime holds.
        let text = fs::read_to_string(directory.join(COMMENTS_FILE)).unwrap();
        assert_eq!(fold(&text), review.log().items);

        let deleted = review
            .set(
                "ui",
                &reply.id,
                CommentPatch {
                    deleted: Some(true),
                    ..CommentPatch::default()
                },
                24,
            )
            .unwrap();
        assert!(deleted.is_none());
        assert_eq!(review.log().items.len(), 1);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_union_merge_folds_to_the_same_comments_whatever_the_line_order() {
        let directory = directory("union");
        let mut review = Review::open(&directory);
        let root = review
            .add(
                "ui",
                NewComment::new(person("Mina"), "Trim the intro"),
                &project(),
                1,
            )
            .unwrap();
        review
            .set(
                "ui",
                &root.id,
                CommentPatch {
                    resolved: Some(true),
                    ..CommentPatch::default()
                },
                2,
            )
            .unwrap();
        let ours = fs::read_to_string(directory.join(COMMENTS_FILE)).unwrap();
        let mut theirs_review = Review::open(&directory);
        theirs_review
            .add(
                "mcp",
                NewComment::new(Author::new("mcp", "Cursor"), "Captions run long"),
                &project(),
                2,
            )
            .unwrap();
        let both = fs::read_to_string(directory.join(COMMENTS_FILE)).unwrap();
        let theirs_line = both.lines().last().unwrap();

        // A union merge keeps both sides' lines and may repeat ones they
        // share; the order differs between the two clones.
        let left = format!("{ours}{theirs_line}\n{ours}");
        let mut lines: Vec<&str> = left.lines().collect();
        lines.reverse();
        let right = lines.join("\n");
        let folded = fold(&left);
        assert_eq!(folded, fold(&right));
        assert_eq!(folded.len(), 2);
        assert!(folded[0].resolved);
        assert_eq!(folded[1].author.name, "Cursor");
        // A torn line or a stray one is skipped.
        assert_eq!(fold(&format!("{left}{{\"op\":\"add\",\"id\"")), folded);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_write_after_a_torn_line_starts_a_fresh_one() {
        let directory = directory("torn");
        let path = directory.join(COMMENTS_FILE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"op\":\"add\",\"id\":\"c-torn\"").unwrap();
        let mut review = Review::open(&directory);
        assert!(review.log().items.is_empty());
        review
            .add(
                "ui",
                NewComment::new(person("Mina"), "After"),
                &project(),
                0,
            )
            .unwrap();
        assert_eq!(review.log().items.len(), 1);
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_change_from_outside_bumps_the_review_revision() {
        let directory = directory("outside");
        let mut review = Review::open(&directory);
        review
            .add("ui", NewComment::new(person("Mina"), "One"), &project(), 0)
            .unwrap();
        let mut other = Review::open(&directory);
        assert_eq!(other.log().items.len(), 1);
        let revision = review.revision();
        other
            .add(
                "mcp",
                NewComment::new(Author::new("mcp", "Codex"), "Two"),
                &project(),
                0,
            )
            .unwrap();
        review.refresh();
        assert_eq!(review.revision(), revision + 1);
        assert_eq!(review.log().items.len(), 2);
        review.refresh();
        assert_eq!(
            review.revision(),
            revision + 1,
            "unchanged file, same revision"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn adds_are_validated_and_retries_are_harmless() {
        let directory = directory("validate");
        let mut review = Review::open(&directory);
        let refused = [
            NewComment::new(Author::new("robot", "R2"), "Hi"),
            NewComment::new(person("Mina"), "  "),
            NewComment {
                at: Some(f64::NAN),
                ..NewComment::new(person("Mina"), "x")
            },
            NewComment {
                drawing: vec![Stroke {
                    tool: "spray".into(),
                    points: vec![],
                    color: None,
                }],
                ..NewComment::new(person("Mina"), "x")
            },
            NewComment {
                parent: Some("c-missing".into()),
                ..NewComment::new(person("Mina"), "x")
            },
        ];
        for comment in refused {
            assert!(
                review.add("ui", comment.clone(), &project(), 0).is_err(),
                "{comment:?}"
            );
        }
        assert!(review
            .add(
                "hacker",
                NewComment::new(person("Mina"), "x"),
                &project(),
                0
            )
            .is_err());
        let with_id = NewComment {
            id: Some("c-client-1".into()),
            ..NewComment::new(person("Mina"), "Once")
        };
        let first = review.add("ui", with_id.clone(), &project(), 0).unwrap();
        let again = review.add("ui", with_id, &project(), 0).unwrap();
        assert_eq!(first, again);
        let qc = NewComment {
            qc_key: Some("qc:text.overlap:c12".into()),
            blocking: true,
            ..NewComment::new(Author::new("qc", "QC"), "Caption overlaps the lower third")
        };
        let first = review.add("system", qc.clone(), &project(), 0).unwrap();
        assert_eq!(
            review.add("system", qc, &project(), 0).unwrap().id,
            first.id
        );
        assert_eq!(review.log().items.len(), 2);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn accepting_a_proposal_resolves_its_comments() {
        let directory = directory("proposal");
        let mut review = Review::open(&directory);
        let linked = review
            .add(
                "agent",
                NewComment {
                    proposal: Some("proposal-1".into()),
                    ..NewComment::new(Author::kerf(), "Tightened the cut")
                },
                &project(),
                3,
            )
            .unwrap();
        let other = review
            .add(
                "ui",
                NewComment::new(person("Mina"), "Unrelated"),
                &project(),
                3,
            )
            .unwrap();
        assert!(review
            .resolve_proposal(&["proposal-1".into()], 4, Some(person("Mina")))
            .unwrap());
        let linked = review.comment(&linked.id).unwrap();
        assert!(linked.resolved);
        assert_eq!(linked.resolved_revision, Some(4));
        assert!(!review.comment(&other.id).unwrap().resolved);
        assert!(!review
            .resolve_proposal(&["proposal-1".into()], 5, None)
            .unwrap());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn comments_export_as_a_csv_sheet() {
        let sequence = project().sequences[0].clone();
        let comment = |body: &str, at: Option<f64>, resolved: bool| Comment {
            id: format!("c-{body}"),
            parent: None,
            author: person("Mina"),
            at,
            duration: at.map(|_| 2.0),
            frame: None,
            body: body.into(),
            tags: vec!["captions".into()],
            blocking: true,
            drawing: Vec::new(),
            proposal: None,
            steps: Vec::new(),
            error: false,
            resolved,
            resolved_by: None,
            resolved_revision: None,
            qc_key: None,
            created: timestamp(),
            revision: 1,
            edited: None,
        };
        let comments = [
            comment("Later, \"quoted\"", Some(20.0), false),
            comment("=SUM(A1)", None, false),
            comment("First", Some(1.5), false),
            comment("Done", Some(0.0), true),
        ];
        let csv = to_csv(&comments, false, &sequence);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "TC in,TC out,Author,Kind,Body,Status,Tags");
        assert_eq!(
            lines[1],
            "00:00:01:12,00:00:03:12,Mina,person,First,open,captions; blocking"
        );
        assert_eq!(
            lines[2],
            "00:00:20:00,00:00:22:00,Mina,person,\"Later, \"\"quoted\"\"\",open,captions; blocking"
        );
        assert_eq!(lines[3], ",,Mina,person,'=SUM(A1),open,captions; blocking");
        assert_eq!(lines.len(), 4);
        assert_eq!(to_csv(&comments, true, &sequence).lines().count(), 5);
    }
}
