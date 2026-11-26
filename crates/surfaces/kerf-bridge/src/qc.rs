//! `check {sequence?}`: Kerf's full QC pass over the project
//! (`kerf_mcp::check_full_with`: the document checks, plus the programme
//! measured for R128 loudness and picture levels), reported to the
//! editor's Preflight as rows and findings.

use crate::event::{Check, ErrorEvent, Event};
use crate::wake::Outbox;
use crate::CommandName;
use crate::{emit, Shared};
use kerf_engine::{CancellationToken, EngineError};
use kerf_mcp::{CheckReport, QcLimits, FULL_ROWS};
use kerf_runtime::{RuntimeClient, RuntimeError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The QC check, one at a time: whether one is running, and what cancels it, so closing
/// the bridge stops it.
#[derive(Default)]
pub(crate) struct Running {
    busy: AtomicBool,
    cancel: Mutex<Option<CancellationToken>>,
}

/// Run the check on its own thread, one at a time: `check {status:
/// "running", progress}` as it goes, then `done` or `failed`. Closing the
/// bridge cancels it.
pub(crate) fn spawn(shared: Arc<Shared>, tx: Outbox, sequence: Option<String>) {
    if shared.check.busy.swap(true, Ordering::SeqCst) {
        // Said from a thread of its own: the caller's empties the event queue.
        shared.workers.spawn("bridge:refused", move || {
            emit(
                &tx,
                Event::Error(ErrorEvent {
                    message: "A check is already running".to_owned(),
                    command: Some(CommandName::Check),
                    ..ErrorEvent::default()
                }),
            );
        });
        return;
    }
    let cancel = CancellationToken::new();
    if let Ok(mut running) = shared.check.cancel.lock() {
        *running = Some(cancel.clone());
    }
    shared.workers.clone().spawn("bridge:check", move || {
        emit(&tx, running(sequence.as_deref(), 0.0));
        let event = match RuntimeClient::new(&shared.directory)
            .and_then(|client| run(&client, &shared, &tx, sequence.as_deref(), &cancel))
        {
            Ok(event) => event,
            Err(error) => failed(sequence.as_deref(), &error.message),
        };
        if let Ok(mut running) = shared.check.cancel.lock() {
            *running = None;
        }
        emit(&tx, event);
        shared.check.busy.store(false, Ordering::SeqCst);
    });
}

/// What a check that was stopped says of itself.
const CANCELLED: &str = "The check was cancelled";

/// Stop the check running, if any; it ends `failed`.
pub(crate) fn cancel(shared: &Shared) {
    if let Ok(running) = shared.check.cancel.lock()
        && let Some(cancel) = &*running
    {
        cancel.cancel();
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
    tx: &Outbox,
    sequence: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Event, RuntimeError> {
    let (_, document) = client.get_document(false)?;
    let mut project = kerf_runtime::parse_document(&document)?;
    if let Some(id) = sequence
        && !project.sequences.iter().any(|candidate| candidate.id == id)
    {
        return Ok(failed(sequence, &format!("no sequence `{id}`")));
    }
    kerf_runtime::resolve_media_paths(&mut project, &shared.directory);
    Ok(measure(&project, shared, tx, sequence, cancel))
}

/// Check `project`, its media paths resolved, and return how it went: `done`, or `failed`
/// for a check that was cancelled or could not be made.
///
/// The picture it measures is drawn in this process, so it begins on the bridge's drawers
/// (`Shared::drawers`) and holds them until it is over: a bridge let go cancels it, and what
/// is about to exit waits for it to stop. One that gets here once the bridge is let go
/// measures nothing.
fn measure(
    project: &kerf_model::Project,
    shared: &Shared,
    tx: &Outbox,
    sequence: Option<&str>,
    cancel: &CancellationToken,
) -> Event {
    let Some(_drawing) = shared.drawers.begin() else {
        return failed(sequence, CANCELLED);
    };
    // Progress goes out in whole percents, so a long check sends at most
    // a hundred of them.
    let mut sent = 0;
    let report = kerf_mcp::check_full_with(
        project,
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
    match report {
        Ok(report) => done(&report, sequence),
        Err(EngineError::Cancelled) => failed(sequence, CANCELLED),
        Err(error) => failed(sequence, &error.to_string()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_bridge_measures_nothing() {
        // A solid, which needs no media to draw or to hear.
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Check\"\n\
             sequence s1 rate=24 size=64x36 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 gen=solid@1 at=0f dur=24f\n",
        )
        .expect("the document");
        let shared = crate::tests_support::shared(&std::env::temp_dir());
        // Room for all a check says of how far it is: nothing takes them here.
        let (tx, _rx) = crate::wake::outbox(256);
        let ended = || {
            let event = measure(
                &project,
                &shared,
                &tx,
                Some("s1"),
                &CancellationToken::new(),
            );
            let event = serde_json::to_value(event).expect("an event writes as JSON");
            (event["status"].clone(), event["message"].clone())
        };
        // Open, the programme is measured to its end.
        assert_eq!(ended(), ("done".into(), serde_json::Value::Null));
        // Its drawers closed, as the bridge closes them when it is let go, none of it is.
        shared.drawers.close();
        assert_eq!(ended(), ("failed".into(), "The check was cancelled".into()));
    }
}
