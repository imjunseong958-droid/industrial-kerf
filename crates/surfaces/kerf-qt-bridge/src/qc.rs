//! `check {sequence?}`: Kerf's full QC pass over the project
//! (`kerf_mcp::check_full_with`: the document checks, plus the programme
//! measured for R128 loudness and picture levels), with each failing
//! finding mirrored in the review log as a `qc` comment.
//!
//! A finding is a comment once, however often the check runs: its
//! `qcKey` names the check and what it is about. When a later check no
//! longer finds it, the comment is resolved; when it comes back, the
//! comment is opened again. Only errors block delivery, so only they mark
//! their comment `blocking`.

use crate::review::{self, Reviews};
use crate::{emit, Shared};
use kerf_engine::{CancellationToken, EngineError};
use kerf_mcp::qc::RowDef;
use kerf_mcp::{CheckReport, Finding, QcLimits, Severity, FULL_ROWS};
use kerf_runtime::{Author, CommentPatch, NewComment, RuntimeClient, RuntimeError};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

/// The most new comments one check writes; the findings past it are still
/// reported, and become comments once the first ones are dealt with.
pub(crate) const MAX_NEW_COMMENTS: usize = 100;

/// Run the check on its own thread, one at a time: `check {status:
/// "running", progress}` as it goes, then `done` or `failed`. Closing the
/// bridge cancels it.
pub(crate) fn spawn(shared: Arc<Shared>, tx: SyncSender<Value>, sequence: Option<String>) {
    if shared.check_busy.swap(true, Ordering::SeqCst) {
        emit(
            &tx,
            json!({"type": "error", "message": "A check is already running", "command": "check"}),
        );
        return;
    }
    let cancel = CancellationToken::new();
    if let Ok(mut running) = shared.check_cancel.lock() {
        *running = Some(cancel.clone());
    }
    std::thread::spawn(move || {
        emit(&tx, running(sequence.as_deref(), 0.0));
        let event = match RuntimeClient::new(&shared.directory)
            .and_then(|client| run(&client, &shared, &tx, sequence.as_deref(), &cancel))
        {
            Ok(event) => event,
            Err(error) => failed(sequence.as_deref(), &error.message),
        };
        if let Ok(mut running) = shared.check_cancel.lock() {
            *running = None;
        }
        emit(&tx, event);
        shared.check_busy.store(false, Ordering::SeqCst);
    });
}

/// Stop the check running, if any; it ends `failed`.
pub(crate) fn cancel(shared: &Shared) {
    if let Ok(running) = shared.check_cancel.lock() {
        if let Some(cancel) = &*running {
            cancel.cancel();
        }
    }
}

fn running(sequence: Option<&str>, progress: f64) -> Value {
    let mut event = json!({"type": "check", "status": "running", "progress": progress});
    if let Some(sequence) = sequence {
        event["sequence"] = json!(sequence);
    }
    event
}

fn failed(sequence: Option<&str>, message: &str) -> Value {
    let mut event = json!({"type": "check", "status": "failed", "message": message});
    if let Some(sequence) = sequence {
        event["sequence"] = json!(sequence);
    }
    event
}

/// Check the project kerfd has now, mirror what it found, and return the
/// `done` event (or `failed` for a sequence the project does not have, or
/// a check that was cancelled). The loudness and picture are measured on
/// `sequence`, else on the first sequence.
fn run(
    client: &RuntimeClient,
    shared: &Shared,
    tx: &SyncSender<Value>,
    sequence: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Value, RuntimeError> {
    let (_, document) = client.get_document(false)?;
    let mut project = kerf_runtime::parse_document(&document)?;
    if let Some(id) = sequence {
        if !project.sequences.iter().any(|candidate| candidate.id == id) {
            return Ok(failed(sequence, &format!("no sequence `{id}`")));
        }
    }
    kerf_runtime::resolve_media_paths(&mut project, &shared.directory);
    // Progress goes out in whole percents, so a long check sends at most
    // a hundred of them.
    let mut sent = 0;
    let report = kerf_mcp::check_full_with(
        &project,
        sequence,
        &shared.directory,
        &QcLimits::default(),
        cancel,
        |done| {
            let percent = (done.clamp(0.0, 1.0) * 100.0).floor() as u32;
            if percent > sent && percent < 100 {
                sent = percent;
                emit(tx, running(sequence, f64::from(percent) / 100.0));
            }
        },
    );
    let report = match report {
        Ok(report) => report,
        Err(EngineError::Cancelled) => return Ok(failed(sequence, "The check was cancelled")),
        Err(error) => return Ok(failed(sequence, &error.to_string())),
    };
    let program = project.sequences.first().map(|first| first.id.to_string());
    let comments = mirror(client, &report, program.as_deref(), shared, tx)?;
    Ok(done(&report, sequence, &comments))
}

/// Measured checks that report one finding per stretch of the programme,
/// so the same check of the same sequence can be found several times.
const PER_STRETCH: [&str; 2] = ["picture.flashing", "picture.black"];

/// The comment key of a finding: its check and what it is about, and for a
/// check found once per stretch, where the stretch starts.
pub(crate) fn key(finding: &Finding) -> String {
    match &finding.entity {
        Some(entity) if PER_STRETCH.contains(&finding.code.as_str()) => format!(
            "qc:{}:{entity}@{}",
            finding.code,
            finding
                .at
                .map_or_else(|| "-".to_owned(), |at| format!("{at:.3}")),
        ),
        Some(entity) => format!("qc:{}:{entity}", finding.code),
        None => format!(
            "qc:{}:{}@{}",
            finding.code,
            finding.sequence.as_deref().unwrap_or("project"),
            finding
                .at
                .map_or_else(|| "-".to_owned(), |at| format!("{at:.3}")),
        ),
    }
}

/// What a check is about, shown as its comment's role, and the tag the
/// comment carries.
pub(crate) fn area(code: &str) -> (&'static str, &'static str) {
    let family = code.split('.').next().unwrap_or(code);
    match family {
        "text" | "caption" => ("captions", "captions"),
        "loudness" | "audio" | "true_peak" | "truepeak" | "lra" => ("loudness", "audio"),
        "picture" | "clip" | "gamut" | "luma" | "flash" => ("picture", "picture"),
        "cut" => ("edit", "edit"),
        "missing_media" | "media" => ("media", "media"),
        "unrenderable" | "render" => ("render", "render"),
        _ => ("document", "document"),
    }
}

fn qc_author(code: &str) -> Author {
    let mut author = Author::new("qc", "QC");
    author.role = Some(area(code).0.to_owned());
    author
}

/// The failing findings, one per key, in report order.
fn failing(report: &CheckReport) -> Vec<(String, &Finding)> {
    let mut seen = HashSet::new();
    report
        .findings
        .iter()
        .filter(|finding| finding.severity.fails())
        .map(|finding| (key(finding), finding))
        .filter(|(key, _)| seen.insert(key.clone()))
        .collect()
}

/// Make the review log say what `report` found: a `qc` comment for each
/// failing finding (opened again when it was resolved, its text and
/// blocking kept current), and the `qc` comments of findings that are gone
/// resolved. `program` is the sequence comment times are on; findings on
/// other sequences are left untimed. Sends the comments once when anything
/// changed. Returns the comment id of each finding's key.
pub(crate) fn mirror(
    reviews: &impl Reviews,
    report: &CheckReport,
    program: Option<&str>,
    shared: &Shared,
    tx: &SyncSender<Value>,
) -> Result<HashMap<String, String>, RuntimeError> {
    let log = reviews.list()?;
    let existing: HashMap<String, kerf_runtime::Comment> = log
        .items
        .iter()
        .filter(|comment| comment.author.kind == "qc")
        .filter_map(|comment| {
            let key = comment.qc_key.as_ref()?;
            key.starts_with("qc:")
                .then(|| (key.clone(), comment.clone()))
        })
        .collect();
    let found = failing(report);
    let keys: HashSet<&str> = found.iter().map(|(key, _)| key.as_str()).collect();
    let mut ids = HashMap::new();
    let mut last = None;
    let mut added = 0;
    for (key, finding) in &found {
        let blocking = finding.severity == Severity::Error;
        if let Some(comment) = existing.get(key) {
            ids.insert(key.clone(), comment.id.clone());
            let patch = CommentPatch {
                resolved: comment.resolved.then_some(false),
                body: (comment.body != finding.message).then(|| finding.message.clone()),
                blocking: (comment.blocking != blocking).then_some(blocking),
                by: Some(qc_author(&finding.code)),
                ..CommentPatch::default()
            };
            if patch.resolved.is_some() || patch.body.is_some() || patch.blocking.is_some() {
                last = Some(reviews.set("system", &comment.id, patch)?.1);
            }
            continue;
        }
        if added >= MAX_NEW_COMMENTS {
            continue;
        }
        let mut comment = NewComment::new(qc_author(&finding.code), finding.message.clone());
        let on_program = match (&finding.sequence, program) {
            (None, _) => true,
            (Some(sequence), Some(program)) => sequence == program,
            (Some(_), None) => false,
        };
        if on_program {
            comment.at = finding.at;
            comment.duration = finding.duration;
        }
        comment.tags = vec![area(&finding.code).1.to_owned()];
        comment.blocking = blocking;
        comment.qc_key = Some(key.clone());
        let (comment, log) = reviews.add("system", comment)?;
        ids.insert(key.clone(), comment.id);
        last = Some(log);
        added += 1;
    }
    for (key, comment) in &existing {
        if comment.resolved || keys.contains(key.as_str()) {
            continue;
        }
        let code = key.split(':').nth(1).unwrap_or_default();
        let patch = CommentPatch {
            resolved: Some(true),
            by: Some(qc_author(code)),
            ..CommentPatch::default()
        };
        last = Some(reviews.set("system", &comment.id, patch)?.1);
    }
    if let Some(log) = last {
        review::emit_log(shared, tx, log, None, false);
    }
    Ok(ids)
}

/// The `done` event: the report for `sequence` (findings on other
/// sequences left out, and the summary counted again), each failing
/// finding and summary row with the comment that holds it.
pub(crate) fn done(
    report: &CheckReport,
    sequence: Option<&str>,
    comments: &HashMap<String, String>,
) -> Value {
    let shown = match sequence {
        Some(id) => {
            let findings = report
                .findings
                .iter()
                .filter(|finding| finding.sequence.as_deref().is_none_or(|on| on == id))
                .cloned()
                .collect();
            let mut shown = CheckReport::new(findings, FULL_ROWS);
            // Rows another check added keep their place, unrecounted.
            for row in &report.summary.rows {
                if !shown.summary.rows.iter().any(|kept| kept.code == row.code) {
                    shown.summary.rows.push(row.clone());
                }
            }
            shown.summary.recount();
            shown
        }
        None => report.clone(),
    };
    let comment_of = |finding: &Finding| {
        finding
            .severity
            .fails()
            .then(|| comments.get(&key(finding)))
            .flatten()
    };
    let mut findings = Vec::new();
    for finding in &shown.findings {
        let mut value = json!(finding);
        if let Some(id) = comment_of(finding) {
            value["commentId"] = json!(id);
        }
        findings.push(value);
    }
    let mut rows = Vec::new();
    for row in &shown.summary.rows {
        let mut value = json!(row);
        let first = shown
            .findings
            .iter()
            .filter(|finding| finding.severity.fails())
            .find(|finding| takes(row.code.as_str(), &finding.code));
        if let Some(id) = first.and_then(comment_of) {
            value["commentId"] = json!(id);
        }
        rows.push(value);
    }
    let mut event = json!({
        "type": "check",
        "status": "done",
        "valid": shown.valid,
        "findings": findings,
        "summary": {
            "passed": shown.summary.passed,
            "total": shown.summary.total,
            "blocking": shown.summary.blocking,
            "rows": rows,
        },
    });
    if let Some(sequence) = sequence {
        event["sequence"] = json!(sequence);
    }
    event
}

/// Whether the summary row `row` counts findings coded `code`, as
/// kerf-mcp's full check assigns them: a row lists its codes, and the row
/// that lists none takes every code no other row lists. A row the full
/// table does not have takes its own code.
fn takes(row: &str, code: &str) -> bool {
    let listed = |def: &RowDef| def.codes.contains(&code);
    match FULL_ROWS.iter().find(|def| def.code == row) {
        Some(def) if def.codes.is_empty() => !FULL_ROWS.iter().any(listed),
        Some(def) => listed(def),
        None => row == code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::tests::local;
    use kerf_mcp::Rect;
    use std::sync::mpsc;

    fn overlap() -> Finding {
        let mut finding = Finding::warning(
            "text.overlap",
            "t4",
            "Caption overlaps the lower-third by 8 px",
        );
        finding.sequence = Some("s1".into());
        finding.at = Some(17.32);
        finding.duration = Some(2.48);
        finding.timecode = Some("00:00:17:08".into());
        finding.rect = Some(Rect {
            x: 0.3,
            y: 0.8,
            w: 0.4,
            h: 0.02,
        });
        finding.value = Some(8.0);
        finding.unit = Some("px".into());
        finding
    }

    fn missing() -> Finding {
        Finding::new(
            Severity::Error,
            "missing_media",
            Some("m1"),
            "media `m1` is missing: /media/a.mov",
        )
    }

    fn report(findings: Vec<Finding>) -> CheckReport {
        CheckReport::new(findings, FULL_ROWS)
    }

    fn qc_comments(reviews: &impl Reviews) -> Vec<kerf_runtime::Comment> {
        reviews
            .list()
            .unwrap()
            .items
            .into_iter()
            .filter(|comment| comment.author.kind == "qc")
            .collect()
    }

    #[test]
    fn check_commands_read_with_and_without_a_sequence() {
        assert!(matches!(
            serde_json::from_value(json!({"type": "check"})).unwrap(),
            crate::Command::Check { sequence: None }
        ));
        let crate::Command::Check { sequence } =
            serde_json::from_value(json!({"type": "check", "sequence": "vertical"})).unwrap()
        else {
            panic!("expected check");
        };
        assert_eq!(sequence.as_deref(), Some("vertical"));
    }

    #[test]
    fn findings_that_go_resolve_their_comment_and_ones_that_return_reopen_it() {
        let directory = tempfile::tempdir().unwrap();
        let reviews = local(directory.path());
        let shared = crate::tests_support::shared(directory.path());
        let (tx, _rx) = mpsc::sync_channel(64);
        // A person's note and an agent's are never touched.
        reviews
            .add("ui", NewComment::new(Author::new("person", "Mina"), "Hi"))
            .unwrap();

        mirror(
            &reviews,
            &report(vec![overlap(), missing()]),
            Some("s1"),
            &shared,
            &tx,
        )
        .unwrap();
        mirror(&reviews, &report(vec![missing()]), Some("s1"), &shared, &tx).unwrap();
        let comments = qc_comments(&reviews);
        let state = |key: &str| {
            comments
                .iter()
                .find(|comment| comment.qc_key.as_deref() == Some(key))
                .map(|comment| (comment.resolved, comment.resolved_by.clone()))
                .unwrap()
        };
        let (resolved, by) = state("qc:text.overlap:t4");
        assert!(resolved);
        assert_eq!(by.unwrap().kind, "qc");
        assert!(!state("qc:missing_media:m1").0);

        // It comes back, measured differently: the same comment, open.
        let mut back = overlap();
        back.message = "Caption overlaps the lower-third by 6 px".into();
        let ids = mirror(&reviews, &report(vec![back]), Some("s1"), &shared, &tx).unwrap();
        let comments = qc_comments(&reviews);
        assert_eq!(comments.len(), 2);
        let overlap = comments
            .iter()
            .find(|comment| Some(&comment.id) == ids.get("qc:text.overlap:t4"))
            .unwrap();
        assert!(!overlap.resolved);
        assert_eq!(overlap.body, "Caption overlaps the lower-third by 6 px");
        assert!(
            comments
                .iter()
                .find(|comment| comment.qc_key.as_deref() == Some("qc:missing_media:m1"))
                .unwrap()
                .resolved
        );
        let people = reviews.list().unwrap().items;
        assert!(people
            .iter()
            .any(|comment| comment.author.kind == "person" && !comment.resolved));
    }

    #[test]
    fn the_done_event_names_the_comment_behind_each_failing_row() {
        let found = report(vec![overlap(), missing()]);
        let comments = HashMap::from([
            ("qc:text.overlap:t4".to_owned(), "c-1708".to_owned()),
            ("qc:missing_media:m1".to_owned(), "c-media".to_owned()),
        ]);
        let event = done(&found, None, &comments);
        assert_eq!(event["type"], "check");
        assert_eq!(event["status"], "done");
        assert_eq!(event["valid"], false);
        let row = |code: &str| {
            event["summary"]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["code"] == code)
                .cloned()
                .unwrap()
        };
        assert_eq!(row("text.overlap")["commentId"], "c-1708");
        assert_eq!(row("text.overlap")["value"], "1 at 00:00:17:08");
        assert_eq!(row("text.overlap")["passed"], false);
        assert_eq!(row("media")["commentId"], "c-media");
        assert_eq!(row("text.safe_area")["passed"], true);
        assert!(row("text.safe_area").get("commentId").is_none());
        assert_eq!(event["summary"]["blocking"], 1);
        assert_eq!(event["summary"]["total"], json!(found.summary.rows.len()));
        let finding = &event["findings"][0];
        assert_eq!(finding["commentId"], "c-1708");
        assert_eq!(finding["rect"]["w"], 0.4);
        assert_eq!(finding["severity"], "warning");

        // For one sequence, findings on others are left out and the rows
        // counted again; project-wide findings stay.
        let vertical = done(&found, Some("vertical"), &comments);
        assert_eq!(vertical["sequence"], "vertical");
        assert_eq!(vertical["findings"].as_array().unwrap().len(), 1);
        assert_eq!(vertical["findings"][0]["code"], "missing_media");
        let overlap_row = vertical["summary"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["code"] == "text.overlap")
            .cloned()
            .unwrap();
        assert_eq!(overlap_row["passed"], true);
    }

    #[test]
    fn keys_name_the_check_and_what_it_is_about() {
        assert_eq!(key(&overlap()), "qc:text.overlap:t4");
        let mut untargeted = Finding::new(Severity::Warning, "loudness.true_peak", None, "x");
        untargeted.sequence = Some("s1".into());
        untargeted.at = Some(27.72);
        assert_eq!(key(&untargeted), "qc:loudness.true_peak:s1@27.720");
        assert_eq!(area("loudness.true_peak"), ("loudness", "audio"));
        assert_eq!(area("caption.reading_speed"), ("captions", "captions"));
        assert_eq!(area("missing_media"), ("media", "media"));
        assert_eq!(area("kerf.validate.something"), ("document", "document"));
        assert!(takes("document", "overlapping_clips"));
        assert!(!takes("document", "text.overlap"));
        assert!(takes("picture.flash_black", "cut.near_scene_cut"));
        assert!(takes("picture.flash_black", "picture.flashing"));
        assert!(takes("loudness.true_peak", "loudness.unavailable"));
        assert!(!takes("document", "picture.flashing"));
        // A check found once per stretch is a comment per stretch.
        let mut flash = Finding::new(Severity::Error, "picture.flashing", Some("s1"), "flashes");
        flash.sequence = Some("s1".into());
        flash.at = Some(4.5);
        assert_eq!(key(&flash), "qc:picture.flashing:s1@4.500");
        flash.at = Some(9.0);
        assert_eq!(key(&flash), "qc:picture.flashing:s1@9.000");
        let mut loud = Finding::new(
            Severity::Error,
            "loudness.integrated",
            Some("s1"),
            "too loud",
        );
        loud.at = Some(3.0);
        assert_eq!(
            key(&loud),
            "qc:loudness.integrated:s1",
            "found once a sequence"
        );
    }

    #[test]
    fn a_check_runs_on_a_project_and_reports_every_row() {
        // The check itself is kerf-mcp's; the bridge only needs it to
        // report each row of a clean project as passing.
        let project = kerf_runtime::parse_document(crate::tests_support::PROGRAM).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let found = kerf_mcp::check(&project, directory.path());
        let event = done(&found, None, &HashMap::new());
        assert_eq!(
            event["summary"]["rows"].as_array().unwrap().len(),
            found.summary.rows.len()
        );
        // The media is not in this folder: that blocks, and is a row.
        assert_eq!(event["valid"], false);
    }
}
