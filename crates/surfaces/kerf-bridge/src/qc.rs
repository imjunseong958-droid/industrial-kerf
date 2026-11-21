//! `check {sequence?}`: Kerf's full QC pass over the project
//! (`kerf_mcp::check_full_with`: the document checks, plus the programme
//! measured for R128 loudness and picture levels), reported to the
//! editor's Preflight as rows and findings.

use crate::event::{Check, ErrorEvent, Event};
use crate::{emit, Shared};
use kerf_engine::{CancellationToken, EngineError};
use kerf_mcp::{CheckReport, QcLimits, FULL_ROWS};
use kerf_runtime::{RuntimeClient, RuntimeError};
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

/// Run the check on its own thread, one at a time: `check {status:
/// "running", progress}` as it goes, then `done` or `failed`. Closing the
/// bridge cancels it.
pub(crate) fn spawn(shared: Arc<Shared>, tx: SyncSender<Event>, sequence: Option<String>) {
    if shared.check_busy.swap(true, Ordering::SeqCst) {
        // Said from a thread of its own: the caller's empties the event queue.
        std::thread::spawn(move || {
            emit(
                &tx,
                Event::Error(ErrorEvent {
                    message: "A check is already running".to_owned(),
                    command: Some("check".to_owned()),
                    ..ErrorEvent::default()
                }),
            );
        });
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

fn running(sequence: Option<&str>, progress: f64) -> Event {
    Event::Check(Check::Running {
        progress,
        sequence: sequence.map(str::to_owned),
    })
}

fn failed(sequence: Option<&str>, message: &str) -> Event {
    Event::Check(Check::Failed {
        message: message.to_owned(),
        sequence: sequence.map(str::to_owned),
    })
}

/// Check the project kerfd has now, mirror what it found, and return the
/// `done` event (or `failed` for a sequence the project does not have, or
/// a check that was cancelled). The loudness and picture are measured on
/// `sequence`, else on the first sequence.
fn run(
    client: &RuntimeClient,
    shared: &Shared,
    tx: &SyncSender<Event>,
    sequence: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Event, RuntimeError> {
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
    Ok(done(&report, sequence))
}

/// The `done` event: the report for `sequence` (findings on other
/// sequences left out, and the summary counted again).
pub(crate) fn done(report: &CheckReport, sequence: Option<&str>) -> Event {
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
    Event::Check(Check::Done {
        valid: shown.valid,
        findings: shown.findings,
        summary: shown.summary,
        sequence: sequence.map(str::to_owned),
    })
}
