//! The reducer's tests: edits and their receipts, proposals, renders, and
//! what survives a restart from the journal and checkpoints.
#![expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]

use super::*;
use crate::ipc::expect_state;
use crate::render::render_cancel_path;
use crate::store::{load_journal, CHECKPOINT_TEMP_FILE, JOURNAL_FILE, PREVIOUS_STATE_PREFIX};
use crate::{run_stdio, EditOp, RenderQuality, RenderStatus, RuntimeHandle};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::thread;

fn runtime() -> Runtime {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-test-{}", unique_id()));
    Runtime::open(&directory).unwrap()
}

/// Apply `script` at `revision` under a fresh request key.
fn edit(runtime: &mut Runtime, script: &str, revision: u64) -> Result<StateSnapshot, RuntimeError> {
    runtime.apply_script_request("ui", &unique_id(), script, revision)
}

fn undo(runtime: &mut Runtime, revision: u64) -> Result<StateSnapshot, RuntimeError> {
    runtime.undo_request("ui", &unique_id(), revision)
}

/// Resize the first sequence with a typed operation.
fn resize(runtime: &mut Runtime, width: u32) -> StateSnapshot {
    let ops = vec![EditOp::SetSequence {
        width: Some(width),
        height: Some(1080),
        name: None,
        rate: None,
        start: None,
        sequence: None,
    }];
    let revision = runtime.state.revision;
    runtime
        .apply_ops_request("ui", &unique_id(), ops, revision)
        .unwrap()
}

fn has_marker(state: &StateSnapshot, id: &str) -> bool {
    state.document.contains(&format!("marker {id} "))
}

fn pending_id(state: &StateSnapshot) -> String {
    match state.pending_proposal.as_ref() {
        Some(PendingProposal::Pending { proposal_id, .. }) => proposal_id.clone(),
        other => panic!("expected a pending proposal, got {other:?}"),
    }
}

#[test]
fn edits_are_atomic_and_revisioned() {
    let mut runtime = runtime();
    let state = edit(&mut runtime, "track add T1 subtitle", 0).unwrap();
    assert_eq!(state.revision, 1);
    assert!(state.can_undo);
    let error = edit(&mut runtime, "marker add at=0f\ntrack remove missing", 1).unwrap_err();
    assert_eq!(error.code.as_str(), "EDIT_REJECTED");
    assert_eq!(runtime.snapshot().revision, 1);
    assert!(!has_marker(&runtime.snapshot(), "mk1"));
    undo(&mut runtime, 1).unwrap();
    assert!(runtime.snapshot().can_redo);
    runtime.redo_request("ui", "redo", 2).unwrap();
    assert_eq!(runtime.snapshot().revision, 3);
}

#[test]
fn one_runtime_owns_a_project_directory() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-lock-{}", unique_id()));
    let runtime = Runtime::open(&directory).unwrap();
    let error = Runtime::open(&directory).unwrap_err();
    assert_eq!(error.code.as_str(), "RUNTIME_ALREADY_RUNNING");
    drop(runtime);
    let reopened = Runtime::open(&directory).unwrap();
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn rejecting_a_proposal_keeps_canonical_state_and_converges() {
    let mut runtime = runtime();
    let before = runtime.snapshot();
    let staged = runtime
        .stage_script_proposal("agent-reject", 0, "track add T1 subtitle", None)
        .unwrap();
    assert!(matches!(
        staged.pending_proposal,
        Some(PendingProposal::Pending {
            freshness: ProposalFreshness::Current,
            ..
        })
    ));
    let proposal_id = pending_id(&staged);
    let rejected = runtime.reject_proposal(&proposal_id).unwrap();
    assert_eq!(rejected.document, before.document);
    assert_eq!(rejected.revision, before.revision);
    assert!(!rejected.can_undo);
    assert!(matches!(
        rejected.pending_proposal,
        Some(PendingProposal::Rejected { .. })
    ));
    let replay = runtime.reject_proposal(&proposal_id).unwrap();
    assert_eq!(replay.document, before.document);
    assert_eq!(replay.revision, before.revision);
}

#[test]
fn accepting_a_proposal_is_one_revision_and_one_undo_entry() {
    let mut runtime = runtime();
    let staged = runtime
        .stage_script_proposal(
            "agent-accept",
            0,
            "track add T1 subtitle\nmarker add at=0f as=intent",
            None,
        )
        .unwrap();
    let proposal_id = pending_id(&staged);
    let accepted = runtime
        .accept_proposal(&proposal_id, "ui", "accept-1", 0)
        .unwrap();
    assert_eq!(accepted.revision, 1);
    assert!(accepted.can_undo);
    assert!(has_marker(&accepted, "intent"));
    assert!(matches!(
        accepted.pending_proposal,
        Some(PendingProposal::Accepted {
            applied_revision: 1,
            ..
        })
    ));
    let replay = runtime
        .accept_proposal(&proposal_id, "ui", "accept-1", 0)
        .unwrap();
    assert_eq!(replay.revision, 1);
    undo(&mut runtime, 1).unwrap();
    assert_eq!(runtime.snapshot().revision, 2);
    assert!(!runtime.document(false).contains("track T1"));
}

#[test]
fn accepted_proposal_receipt_survives_later_mutation_and_rejects_reuse() {
    let mut runtime = runtime();
    let staged = runtime
        .stage_script_proposal("agent-receipt", 0, "marker add at=0f as=approved", None)
        .unwrap();
    let proposal_id = pending_id(&staged);
    let first = runtime
        .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
        .unwrap();
    let later = edit(&mut runtime, "marker add at=0f as=later", first.revision).unwrap();
    assert_eq!(later.revision, 2);

    let replay = runtime
        .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
        .unwrap();
    assert_eq!(replay.revision, first.revision);
    assert_eq!(replay.document, first.document);
    assert!(has_marker(&replay, "approved"));
    assert!(!has_marker(&replay, "later"));

    let error = runtime
        .accept_proposal(&proposal_id, "ui", "accept-other", 0)
        .unwrap_err();
    assert_eq!(error.code.as_str(), "IDEMPOTENCY_CONFLICT");
}

#[test]
fn accepting_a_stale_proposal_conflicts_without_clearing_it() {
    let mut runtime = runtime();
    let staged = runtime
        .stage_script_proposal("agent-stale", 0, "track add T1 subtitle", None)
        .unwrap();
    let proposal_id = pending_id(&staged);
    edit(&mut runtime, "marker add at=0f as=changed", 0).unwrap();
    let error = runtime
        .accept_proposal(&proposal_id, "ui", "accept-stale", 0)
        .unwrap_err();
    assert_eq!(error.code.as_str(), "REVISION_CONFLICT");
    assert!(matches!(
        runtime.snapshot().pending_proposal,
        Some(PendingProposal::Pending {
            freshness: ProposalFreshness::Stale,
            ..
        })
    ));
}

#[test]
fn invalid_proposal_scripts_stage_nothing() {
    let mut runtime = runtime();
    let before = runtime.snapshot();
    let error = runtime
        .stage_script_proposal(
            "agent-invalid",
            0,
            "track add T1 subtitle\ntrack remove missing",
            None,
        )
        .unwrap_err();
    assert_eq!(error.code.as_str(), "EDIT_REJECTED");
    let after = runtime.snapshot();
    assert_eq!(after.document, before.document);
    assert_eq!(after.revision, before.revision);
    assert!(after.pending_proposal.is_none());
}

#[test]
fn revision_and_history_survive_a_runtime_restart() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-restart-{}", unique_id()));
    {
        let mut runtime = Runtime::open(&directory).unwrap();
        edit(&mut runtime, "track add T1 subtitle", 0).unwrap();
        assert_eq!(runtime.snapshot().revision, 1);
    }
    let mut reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.snapshot().revision, 1);
    assert!(reopened.snapshot().can_undo);
    undo(&mut reopened, 1).unwrap();
    assert_eq!(reopened.snapshot().revision, 2);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn import_rejects_paths_that_are_not_files() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-import-boundary-{}", unique_id()));
    fs::create_dir_all(&directory).unwrap();
    let mut runtime = Runtime::open(&directory).unwrap();
    let missing = runtime
        .import_media_request("ui", "missing", "/definitely/not-a-file", 0)
        .unwrap_err();
    assert_eq!(missing.code.as_str(), "INVALID_IMPORT");
    let directory_error = runtime
        .import_media_request("ui", "directory", directory.to_str().unwrap(), 0)
        .unwrap_err();
    assert_eq!(directory_error.code.as_str(), "INVALID_IMPORT");
    assert_eq!(runtime.snapshot().revision, 0);
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

/// A fresh project folder with a `media/` folder in it.
fn media_project(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-{label}-{}", unique_id()));
    fs::create_dir_all(directory.join("media")).unwrap();
    directory
}

/// `seconds` of 64x36 picture at 24 fps, with a tone when `sound`.
fn media_clip(path: &Path, seconds: u32, sound: bool) {
    let mut command =
        std::process::Command::new(kerf_engine::process::ffmpeg::resolve_ffmpeg_bin());
    command
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!("color=c=gray:s=64x36:r=24:d={seconds}"));
    if sound {
        command
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=f=440:sample_rate=48000:d={seconds}"))
            .args(["-c:a", "aac", "-shortest"]);
    }
    let output = command
        .args(["-c:v", "mpeg4", "-pix_fmt", "yuv420p", "-y"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_replacement_replays_from_the_journal_after_a_restart() {
    let directory = media_project("replace-restart");
    media_clip(&directory.join("media/a.mp4"), 2, false);
    media_clip(&directory.join("media/b.mp4"), 3, false);
    let (imported, replaced) = {
        let mut runtime = Runtime::open(&directory).unwrap();
        let (imported, media, _) = runtime
            .import_media_request("ui", "import", "media/a.mp4", 0)
            .unwrap();
        let replaced = runtime
            .replace_media_request("ui", "replace", &media, "media/b.mp4", 1)
            .unwrap();
        (imported, replaced)
    };
    assert!(directory.join(JOURNAL_FILE).is_file());
    let mut reopened = Runtime::open(&directory).unwrap();
    let recovered = reopened.snapshot();
    assert_eq!(recovered.revision, 2);
    assert_eq!(recovered.document, replaced.document);
    assert_eq!(recovered.projection_status.as_str(), "current");
    assert!(recovered.can_undo);
    // The receipt survived too.
    let replay = reopened
        .replace_media_request("ui", "replace", "m1", "media/b.mp4", 1)
        .unwrap();
    assert_eq!(replay.revision, 2);
    assert_eq!(undo(&mut reopened, 2).unwrap().document, imported.document);
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn durable_journal_recovers_after_projection_failure() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-persist-fail-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    let before = runtime.snapshot();
    let before_document = runtime.document(false);
    // The projection is deliberately made unrenamable. The journal commit
    // must still advance the authoritative runtime state and survive a
    // restart after the old projection is restored.
    fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
    fs::create_dir(directory.join(PROJECT_FILE)).unwrap();
    let committed = edit(&mut runtime, "track add T1 subtitle", before.revision).unwrap();
    assert_eq!(committed.revision, before.revision + 1);
    assert_eq!(committed.projection_status.as_str(), "diverged");
    assert!(directory.join(JOURNAL_FILE).is_file());
    drop(runtime);
    fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
    fs::write(directory.join(PROJECT_FILE), &before_document).unwrap();
    let reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.snapshot().revision, committed.revision);
    assert!(reopened.document(false).contains("track T1 subtitle"));
    assert_eq!(reopened.snapshot().projection_status.as_str(), "current");
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn journal_tail_recovery_keeps_external_projection_edits_divergent() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-journal-tail-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    let first = edit(&mut runtime, "marker add at=0f as=first", 0).unwrap();
    let first_document = runtime.document(false);

    fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
    fs::create_dir(directory.join(PROJECT_FILE)).unwrap();
    let second = edit(&mut runtime, "marker add at=0f as=second", first.revision).unwrap();
    assert_eq!(second.projection_status.as_str(), "diverged");
    drop(runtime);

    // The projection is the last successfully published journal state,
    // while the second mutation remains durable in the journal.
    fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
    fs::write(directory.join(PROJECT_FILE), &first_document).unwrap();
    let reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.snapshot().revision, second.revision);
    assert!(has_marker(&reopened.snapshot(), "second"));
    assert_eq!(reopened.snapshot().projection_status.as_str(), "current");
    drop(reopened);

    // An edit that does not match any recognized journal state must remain
    // visible on disk and keep subsequent mutations blocked.
    let edited = fs::read_to_string(directory.join(PROJECT_FILE))
        .unwrap()
        .replace(
            "project name=\"Untitled project\"",
            "project name=\"external edit\"",
        );
    fs::write(directory.join(PROJECT_FILE), edited).unwrap();
    let mut diverged = Runtime::open(&directory).unwrap();
    assert_eq!(diverged.snapshot().projection_status.as_str(), "diverged");
    assert_eq!(
        edit(
            &mut diverged,
            "marker add at=0f as=blocked",
            second.revision
        )
        .unwrap_err()
        .code
        .as_str(),
        "PROJECTION_DIVERGED"
    );
    let on_disk = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
    assert!(on_disk.contains("project name=\"external edit\""));
    drop(diverged);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn failed_startup_projection_repair_reports_project_status() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-repair-status-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    let before_document = runtime.document(false);
    let committed = resize(&mut runtime, 1280);
    drop(runtime);

    // Put back the last checkpointed document so journal replay
    // recognizes it as an internal stale projection and repairs it.
    fs::write(directory.join(PROJECT_FILE), before_document).unwrap();
    let reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.snapshot().revision, committed.revision);
    assert_eq!(reopened.snapshot().timeline["width"], 1280);
    assert_eq!(reopened.snapshot().projection_status.as_str(), "current");
    assert!(fs::read_to_string(directory.join(PROJECT_FILE))
        .unwrap()
        .contains("size=1280x1080"));
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn operation_receipts_and_revision_survive_restart() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-receipt-{}", unique_id()));
    let first = {
        let mut runtime = Runtime::open(&directory).unwrap();
        runtime
            .apply_script_request("ui", "request-1", "track add T1 subtitle", 0)
            .unwrap()
    };
    let mut reopened = Runtime::open(&directory).unwrap();
    let replay = reopened
        .apply_script_request("ui", "request-1", "track add T1 subtitle", 0)
        .unwrap();
    assert_eq!(replay.revision, first.revision);
    assert_eq!(replay.document, first.document);
    let conflict = reopened
        .apply_script_request("ui", "request-1", "marker add at=0f", 1)
        .unwrap_err();
    assert_eq!(conflict.code.as_str(), "IDEMPOTENCY_CONFLICT");
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn accepted_proposal_receipt_survives_restart() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-accepted-restart-{}", unique_id()));
    let proposal_id = {
        let mut runtime = Runtime::open(&directory).unwrap();
        let staged = runtime
            .stage_script_proposal(
                "agent-accepted-restart",
                0,
                "marker add at=0f as=persisted",
                None,
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        runtime
            .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
            .unwrap();
        proposal_id
    };
    let mut reopened = Runtime::open(&directory).unwrap();
    edit(&mut reopened, "marker add at=0f as=later", 1).unwrap();
    let replay = reopened
        .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
        .unwrap();
    assert_eq!(replay.revision, 1);
    assert!(has_marker(&replay, "persisted"));
    assert!(!has_marker(&replay, "later"));
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn mailbox_serializes_two_clients_against_one_revision() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-mailbox-{}", unique_id()));
    let handle = RuntimeHandle::open(&directory).unwrap();
    let apply = |actor: &str, key: &str, script: &str| {
        let handle = handle.clone();
        let request = RuntimeRequest::ApplyScript {
            actor: actor.into(),
            idempotency_key: key.into(),
            base_revision: 0,
            script: script.into(),
        };
        thread::spawn(move || handle.request(request).and_then(expect_state))
    };
    let first = apply("ui", "left", "track add T1 video");
    let second = apply("agent", "right", "track add T2 video");
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    assert_eq!(first.is_ok() as u8 + second.is_ok() as u8, 1);
    let error = if let Err(error) = first {
        error
    } else {
        second.unwrap_err()
    };
    assert_eq!(error.code.as_str(), "REVISION_CONFLICT");
    let state = expect_state(handle.request(RuntimeRequest::GetState).unwrap()).unwrap();
    assert_eq!(state.revision, 1);
    drop(handle);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn jsonl_rejects_empty_idempotency_keys_at_runtime_boundary() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-jsonl-{}", unique_id()));
    let requests = [
        json!({
            "protocolVersion":1,
            "id":"script-empty",
            "operation":"apply_script",
            "actor":"ui",
            "idempotencyKey":"",
            "baseRevision":0,
            "script":"marker add at=0f"
        }),
        json!({
            "protocolVersion":1,
            "id":"undo-empty",
            "operation":"undo",
            "actor":"ui",
            "idempotencyKey":"   ",
            "baseRevision":0
        }),
        json!({
            "protocolVersion":1,
            "id":"redo-empty",
            "operation":"redo",
            "actor":"ui",
            "idempotencyKey":"",
            "baseRevision":0
        }),
        json!({
            "protocolVersion":1,
            "id":"import-empty",
            "operation":"import",
            "actor":"ui",
            "idempotencyKey":"",
            "baseRevision":0,
            "path":"/definitely/not-a-file"
        }),
        json!({
            "protocolVersion":1,
            "id":"replace-empty",
            "operation":"replace_media",
            "actor":"ui",
            "idempotencyKey":"",
            "baseRevision":0,
            "media":"m1",
            "path":"/definitely/not-a-file"
        }),
        json!({
            "protocolVersion":1,
            "id":"accept-empty",
            "operation":"accept_proposal",
            "proposalId":"proposal-missing",
            "actor":"ui",
            "idempotencyKey":"",
            "baseRevision":0
        }),
        json!({
            "protocolVersion":1,
            "id":"reject-empty",
            "operation":"reject_proposal",
            "proposalId":"proposal-missing",
            "actor":"ui",
            "idempotencyKey":"",
            "baseRevision":0
        }),
    ];
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let mut output = Vec::new();
    run_stdio(
        &directory,
        std::io::Cursor::new(format!("{input}\n")),
        &mut output,
    )
    .unwrap();
    let responses = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), requests.len());
    for (response, request) in responses.iter().zip(requests.iter()) {
        assert_eq!(response["id"], request["id"]);
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "INVALID_REQUEST");
    }
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn durable_rejection_receipt_survives_later_mutation_and_rejects_reuse() {
    let mut runtime = runtime();
    let staged = runtime
        .stage_script_proposal(
            "agent-reject-receipt",
            0,
            "marker add at=0f as=proposed",
            None,
        )
        .unwrap();
    let proposal_id = pending_id(&staged);
    let first = runtime
        .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
        .unwrap();
    edit(&mut runtime, "marker add at=0f as=later", first.revision).unwrap();
    let replay = runtime
        .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
        .unwrap();
    assert_eq!(replay.revision, first.revision);
    assert_eq!(replay.document, first.document);
    let error = runtime
        .reject_proposal_request(&proposal_id, "ui", "reject-other", 0)
        .unwrap_err();
    assert_eq!(error.code.as_str(), "IDEMPOTENCY_CONFLICT");
}

#[test]
fn a_stale_proposal_is_rejected_at_the_revision_the_client_is_at() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-reject-stale-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    let staged = runtime
        .stage_script_proposal(
            "agent-reject-stale",
            0,
            "marker add at=0f as=proposed",
            None,
        )
        .unwrap();
    let proposal_id = pending_id(&staged);
    let later = edit(&mut runtime, "marker add at=0f as=later", 0).unwrap();
    assert_eq!(later.revision, 1);
    assert!(matches!(
        later.pending_proposal,
        Some(PendingProposal::Pending {
            freshness: ProposalFreshness::Stale,
            ..
        })
    ));
    assert!(runtime.state.bases.contains_key(&0));

    // The revision asked about is the client's, which is out of date
    // at the one the proposal was made on.
    let error = runtime
        .reject_proposal_request(&proposal_id, "ui", "reject-made-on", 0)
        .unwrap_err();
    assert_eq!(error.code.as_str(), "REVISION_CONFLICT");
    assert_eq!(error.actual_revision, Some(1));

    let rejected = runtime
        .reject_proposal_request(&proposal_id, "ui", "reject-stale", 1)
        .unwrap();
    assert_eq!(rejected.revision, later.revision);
    assert_eq!(rejected.document, later.document);
    assert_eq!(rejected.can_undo, later.can_undo);
    assert_eq!(rejected.proposals[0].proposal_id, proposal_id);
    assert_eq!(rejected.proposals[0].status.as_str(), "rejected");
    // No open proposal was made on revision 0 now: its project goes.
    assert!(runtime.state.bases.is_empty());

    // A retry converges on the receipt; the key cannot be reused, and a
    // rejected proposal is not accepted.
    let replay = runtime
        .reject_proposal_request(&proposal_id, "ui", "reject-stale", 1)
        .unwrap();
    assert_eq!(replay.revision, rejected.revision);
    assert_eq!(replay.document, rejected.document);
    assert_eq!(replay.proposals[0].status.as_str(), "rejected");
    let error = runtime
        .reject_proposal_request(&proposal_id, "ui", "reject-again", 1)
        .unwrap_err();
    assert_eq!(error.code.as_str(), "IDEMPOTENCY_CONFLICT");
    let error = runtime
        .accept_proposal(&proposal_id, "ui", "accept-rejected", 1)
        .unwrap_err();
    assert_eq!(error.code.as_str(), "PROPOSAL_REJECTED");

    // The journal keeps the rejection and its receipt.
    drop(runtime);
    let mut reopened = Runtime::open(&directory).unwrap();
    let state = reopened.snapshot();
    assert_eq!(state.revision, 1);
    assert_eq!(state.proposals[0].status.as_str(), "rejected");
    let replay = reopened
        .reject_proposal_request(&proposal_id, "ui", "reject-stale", 1)
        .unwrap();
    assert_eq!(replay.document, rejected.document);
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn checkpoint_and_torn_journal_tail_recover_authoritative_state() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-journal-recovery-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    for index in 0..(CHECKPOINT_INTERVAL + 1) {
        resize(&mut runtime, 1000 + 2 * index as u32);
    }
    let checkpoint: Value =
        serde_json::from_str(&fs::read_to_string(directory.join(CHECKPOINT_FILE)).unwrap())
            .unwrap();
    assert_eq!(checkpoint["journal_sequence"], CHECKPOINT_INTERVAL);
    assert_eq!(
        checkpoint["past"].as_array().unwrap().len(),
        CHECKPOINT_INTERVAL as usize
    );
    drop(runtime);

    let mut journal = OpenOptions::new()
        .append(true)
        .open(directory.join(JOURNAL_FILE))
        .unwrap();
    journal.write_all(br#"{"#).unwrap();
    journal.sync_all().unwrap();

    let reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.snapshot().revision, CHECKPOINT_INTERVAL + 1);
    assert_eq!(
        reopened.snapshot().timeline["width"],
        1000 + 2 * CHECKPOINT_INTERVAL
    );
    drop(reopened);
    let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
    assert!(journal
        .lines()
        .all(|line| serde_json::from_str::<Value>(line).is_ok()));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn interrupted_checkpoint_write_recovers_from_complete_temp_file() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-checkpoint-recovery-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    edit(&mut runtime, "marker add at=0f as=saved", 0).unwrap();
    drop(runtime);
    let checkpoint = fs::read(directory.join(CHECKPOINT_FILE)).unwrap();
    fs::write(directory.join(CHECKPOINT_TEMP_FILE), checkpoint).unwrap();
    fs::write(directory.join(CHECKPOINT_FILE), b"{broken").unwrap();

    let runtime = Runtime::open(&directory).unwrap();
    assert_eq!(runtime.snapshot().revision, 1);
    assert!(has_marker(&runtime.snapshot(), "saved"));
    assert!(!directory.join(CHECKPOINT_TEMP_FILE).exists());
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn state_from_an_older_runtime_is_preserved_and_the_project_reopened() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-old-state-{}", unique_id()));
    fs::create_dir_all(&directory).unwrap();
    let document = kernel::print(&Project {
        name: "Earlier".into(),
        ..kernel::empty_project()
    });
    fs::write(directory.join(PROJECT_FILE), &document).unwrap();
    let old_checkpoint = br#"{"format_version":3,"project":"","revision":7}"#;
    fs::write(directory.join(CHECKPOINT_FILE), old_checkpoint).unwrap();
    fs::write(directory.join(JOURNAL_FILE), b"{\"format_version\":2}\n").unwrap();
    let runtime = Runtime::open(&directory).unwrap();
    let state = runtime.snapshot();
    assert_eq!(state.revision, 0);
    assert_eq!(state.document, document);
    assert!(state
        .import_warnings
        .iter()
        .any(|warning| warning.contains(PREVIOUS_STATE_PREFIX)));
    assert_eq!(
        fs::read(directory.join(format!("{PREVIOUS_STATE_PREFIX}.checkpoint.json"))).unwrap(),
        old_checkpoint
    );
    assert!(directory
        .join(format!("{PREVIOUS_STATE_PREFIX}.journal"))
        .is_file());
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn divergent_projection_requires_repair_or_explicit_import() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-projection-policy-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    edit(&mut runtime, "marker add at=0f as=authoritative", 0).unwrap();
    drop(runtime);

    let original = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
    fs::write(
        directory.join(PROJECT_FILE),
        original.replace("Untitled project", "human edit"),
    )
    .unwrap();

    let mut runtime = Runtime::open(&directory).unwrap();
    assert_eq!(runtime.snapshot().projection_status.as_str(), "diverged");
    let blocked = edit(&mut runtime, "marker add at=0f as=blocked", 1);
    assert_eq!(blocked.unwrap_err().code.as_str(), "PROJECTION_DIVERGED");
    let repaired = runtime.repair_projection().unwrap();
    assert_eq!(repaired.projection_status.as_str(), "current");
    assert_eq!(repaired.timeline["name"], "Untitled project");

    // A text edit of the canonical document, made outside kerfd.
    fs::write(
        directory.join(PROJECT_FILE),
        original.replace("Untitled project", "explicit import")
            + "# a comment the import does not keep\n",
    )
    .unwrap();
    let imported_state = runtime
        .import_projection("ui", "projection-import-1", repaired.revision)
        .unwrap();
    assert_eq!(imported_state.revision, repaired.revision + 1);
    assert_eq!(imported_state.timeline["name"], "explicit import");
    assert!(!imported_state.can_undo);
    assert_eq!(imported_state.projection_status.as_str(), "current");
    drop(runtime);

    let reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.snapshot().timeline["name"], "explicit import");
    assert_eq!(reopened.snapshot().projection_status.as_str(), "current");
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn render_cancellation_is_job_scoped_and_survives_restart() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-job-cancel-{}", unique_id()));
    let (cancelled_id, queued_id) = {
        let mut runtime = Runtime::open(&directory).unwrap();
        let (cancelled, _, _) = runtime
            .start_render_range(RenderQuality::Preview, None, None, None)
            .unwrap();
        let (queued, _, _) = runtime
            .start_render_range(RenderQuality::Preview, None, None, None)
            .unwrap();
        let cancel_path = render_cancel_path(&directory, &cancelled.job_id);
        let queued_path = render_cancel_path(&directory, &queued.job_id);
        let cancelled_job = runtime.cancel_render(&cancelled.job_id).unwrap();
        assert_eq!(cancelled_job.status.as_str(), "cancelled");
        assert!(cancel_path.is_file());
        assert!(!queued_path.is_file());
        assert_eq!(
            runtime.render_job(&queued.job_id).unwrap().status.as_str(),
            "queued"
        );
        (cancelled.job_id, queued.job_id)
    };

    let runtime = Runtime::open(&directory).unwrap();
    assert_eq!(
        runtime.render_job(&cancelled_id).unwrap().status.as_str(),
        "cancelled"
    );
    assert_eq!(
        runtime.render_job(&queued_id).unwrap().status.as_str(),
        "failed"
    );
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

/// A project with media, a picture clip, and its linked sound.
fn kernel_runtime(label: &str) -> (Runtime, PathBuf) {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-{label}-{}", unique_id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(PROJECT_FILE),
        "kerf 1\nproject name=\"Kernel\"\n\n\
         media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\n\
         sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1\n",
    )
    .unwrap();
    (Runtime::open(&directory).unwrap(), directory)
}

#[test]
fn typed_ops_apply_atomically_and_replay_by_key() {
    let (mut runtime, directory) = kernel_runtime("ops");
    let ops: Vec<EditOp> = serde_json::from_value(json!([
        {"op": "split", "clip": "c1", "at": 48, "tail": "c2"},
        {"op": "set", "clip": "c2", "props": {"opacity": 0.25}},
        {"op": "add-marker", "at": 12, "name": "look", "color": "green"}
    ]))
    .unwrap();
    let first = runtime
        .apply_ops_request("ui", "ops-1", ops.clone(), 0)
        .unwrap();
    assert_eq!(first.revision, 1);
    let replay = runtime.apply_ops_request("ui", "ops-1", ops, 0).unwrap();
    assert_eq!(replay.revision, 1);
    assert!(runtime.document(false).contains("set opacity=0.25"));
    assert_eq!(
        runtime.state.past[0].script,
        "split c1 at=00:00:02:00 as=c2\nset c2 opacity=0.25\nmarker add at=00:00:00:12 name=\"look\" color=green\n"
    );
    // Any failing operation leaves the project unchanged.
    let failing: Vec<EditOp> = serde_json::from_value(json!([
        {"op": "trim", "clip": "c1", "edge": "out", "frames": -6},
        {"op": "slip", "clip": "missing", "frames": 1}
    ]))
    .unwrap();
    let before = runtime.document(false);
    let error = runtime
        .apply_ops_request("ui", "ops-2", failing, 1)
        .unwrap_err();
    assert_eq!(error.code.as_str(), "INVALID_EDIT");
    assert_eq!(runtime.document(false), before);
    assert_eq!(runtime.snapshot().revision, 1);
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn history_snapshots_survive_checkpoints() {
    let (mut runtime, directory) = kernel_runtime("history-checkpoint");
    for index in 0..(CHECKPOINT_INTERVAL + 2) {
        runtime
            .apply_script_request(
                "ui",
                &format!("marker-{index}"),
                &format!("marker add at={index}f"),
                index,
            )
            .unwrap();
    }
    let expected = runtime.document(false);
    drop(runtime);
    let mut reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.document(false), expected);
    let revision = reopened.snapshot().revision;
    for step in 0..(CHECKPOINT_INTERVAL + 2) {
        undo(&mut reopened, revision + step).unwrap();
    }
    let undone = reopened.document(false);
    assert!(!undone.contains("marker mk"), "{undone}");
    // Undone markers keep their ids retired.
    assert!(undone.contains("retired=mk1,"), "{undone}");
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn replay_refuses_a_record_that_does_not_reproduce_its_project() {
    let (mut runtime, directory) = kernel_runtime("replay-digest");
    runtime
        .apply_script_request("ui", "trim", "trim c1 out -12f", 0)
        .unwrap();
    drop(runtime);
    let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
    let mut record: JournalRecord = serde_json::from_str(journal.trim()).unwrap();
    record.change.as_mut().unwrap().script = Some("trim c1 out -6f\n".into());
    let record = seal_journal_record(record).unwrap();
    fs::write(
        directory.join(JOURNAL_FILE),
        format!("{}\n", serde_json::to_string(&record).unwrap()),
    )
    .unwrap();
    let error = Runtime::open(&directory).unwrap_err();
    assert_eq!(error.code.as_str(), "JOURNAL_REPLAY_MISMATCH");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn receipts_keep_no_state_and_a_retry_answers_with_the_state_now() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-small-receipt-{}", unique_id()));
    let mut runtime = Runtime::open(&directory).unwrap();
    let first = runtime
        .apply_script_request("ui", "first", "marker add at=0f", 0)
        .unwrap();
    let later = edit(&mut runtime, "marker add at=1f", 1).unwrap();
    let retried = runtime
        .apply_script_request("ui", "first", "marker add at=0f", 0)
        .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(retried.revision, later.revision);
    assert_eq!(retried.document, later.document);
    assert_eq!(runtime.state.operations["ui:first"].revision(), 1);
    let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
    assert!(!journal.contains("\"snapshot\""), "{journal}");
    runtime.persist_checkpoint().unwrap();
    let checkpoint = fs::read_to_string(directory.join(CHECKPOINT_FILE)).unwrap();
    assert!(!checkpoint.contains("\"snapshot\""), "{checkpoint}");
    assert!(!checkpoint.contains('\n'), "the checkpoint is compact");
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

/// A receipt as kerfds wrote it while receipts kept the state they
/// left, in a journal record of that time.
#[derive(serde::Serialize)]
struct LegacyReceipt {
    fingerprint: String,
    snapshot: StateSnapshot,
}

#[derive(serde::Serialize)]
struct LegacyOperation {
    key: String,
    record: LegacyReceipt,
}

#[derive(serde::Serialize)]
struct LegacyJournalRecord {
    format_version: u64,
    sequence: u64,
    revision: u64,
    action: JournalAction,
    change: Option<Change>,
    operation: Option<LegacyOperation>,
    import: Option<JournalImport>,
    proposal: Option<ProposalRecord>,
    job: Option<RenderJob>,
    checksum: String,
}

#[test]
fn receipts_written_with_their_state_still_load() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-legacy-receipt-{}", unique_id()));
    let (before, after) = {
        let mut runtime = Runtime::open(&directory).unwrap();
        let before = runtime.snapshot();
        let after = runtime
            .apply_script_request("ui", "journaled", "marker add at=0f", 0)
            .unwrap();
        (before, after)
    };
    // The journal record as the previous format wrote it.
    let journal_path = directory.join(JOURNAL_FILE);
    let record = load_journal(&journal_path, 0).unwrap().remove(0);
    let operation = record.operation.unwrap();
    let mut legacy = LegacyJournalRecord {
        format_version: 3,
        sequence: record.sequence,
        revision: record.revision,
        action: record.action,
        change: record.change,
        operation: Some(LegacyOperation {
            key: operation.key,
            record: LegacyReceipt {
                fingerprint: operation.record.fingerprint,
                snapshot: after.clone(),
            },
        }),
        import: None,
        proposal: record.proposal,
        job: None,
        checksum: String::new(),
    };
    legacy.checksum = sha256_hex(&serde_json::to_vec(&legacy).unwrap());
    fs::write(
        &journal_path,
        format!("{}\n", serde_json::to_string(&legacy).unwrap()),
    )
    .unwrap();
    // And a checkpoint of format 4 with a receipt of its own.
    let checkpoint_path = directory.join(CHECKPOINT_FILE);
    let mut checkpoint: Value =
        serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
    checkpoint["format_version"] = json!(4);
    checkpoint["operations"]["ui:checkpointed"] =
        json!({"fingerprint": "ui:0:old", "snapshot": before});
    fs::write(
        &checkpoint_path,
        serde_json::to_vec_pretty(&checkpoint).unwrap(),
    )
    .unwrap();

    let mut reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.snapshot().revision, 1);
    assert_eq!(reopened.state.operations["ui:journaled"].revision(), 1);
    assert_eq!(reopened.state.operations["ui:checkpointed"].revision(), 0);
    let retried = reopened
        .apply_script_request("ui", "journaled", "marker add at=0f", 0)
        .unwrap();
    assert_eq!(retried.document, after.document);
    let conflict = reopened
        .apply_script_request("ui", "checkpointed", "marker add at=1f", 1)
        .unwrap_err();
    assert_eq!(conflict.code.as_str(), "IDEMPOTENCY_CONFLICT");
    reopened.persist_checkpoint().unwrap();
    let rewritten = fs::read_to_string(&checkpoint_path).unwrap();
    assert!(rewritten.contains("\"format_version\":5"), "{rewritten}");
    assert!(!rewritten.contains("\"snapshot\""), "{rewritten}");
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn receipts_beyond_the_bound_go_oldest_first() {
    let mut receipts: HashMap<String, OperationRecord> = (0..=MAX_RECEIPTS as u64 + 1)
        .map(|revision| {
            (
                format!("ui:{revision}"),
                OperationRecord::new(String::new(), revision),
            )
        })
        .collect();
    bound_receipts(&mut receipts, Some("ui:0"), OperationRecord::revision);
    assert_eq!(receipts.len(), MAX_RECEIPTS);
    assert!(receipts.contains_key("ui:0"));
    assert!(!receipts.contains_key("ui:1"));
    assert!(!receipts.contains_key("ui:2"));
    assert!(receipts.contains_key("ui:3"));
}

#[test]
fn a_journal_left_without_its_checkpoint_is_set_aside_and_the_project_opens() {
    let directory =
        std::env::temp_dir().join(format!("kerf-runtime-journal-alone-{}", unique_id()));
    let document = {
        let mut runtime = Runtime::open(&directory).unwrap();
        runtime
            .apply_script_request("ui", "one", "marker add at=0f as=first", 0)
            .unwrap();
        runtime
            .apply_script_request("ui", "two", "marker add at=0f as=second", 1)
            .unwrap()
            .document
    };
    // The checkpoint goes, as when a folder is copied without it: the
    // journal's records were run on a state that is no longer kept, and
    // the project file already holds what they made.
    let journal_path = directory.join(JOURNAL_FILE);
    assert_eq!(load_journal(&journal_path, 0).unwrap().len(), 2);
    fs::remove_file(directory.join(CHECKPOINT_FILE)).unwrap();

    let mut runtime = Runtime::open(&directory).unwrap();
    let reopened = runtime.snapshot();
    assert_eq!(reopened.document, document, "as the project file has it");
    assert_eq!(reopened.revision, 0);
    assert!(
        reopened
            .import_warnings
            .iter()
            .any(|note| note.contains("journal") && note.contains(PREVIOUS_STATE_PREFIX)),
        "{:?}",
        reopened.import_warnings
    );
    // Kept, not run a second time and not thrown away.
    let kept = directory.join(format!("{PREVIOUS_STATE_PREFIX}.journal"));
    assert_eq!(load_journal(&kept, 0).unwrap().len(), 2);
    assert!(load_journal(&journal_path, 0).unwrap().is_empty());
    // And the project can be edited again.
    let edited = runtime
        .apply_script_request("ui", "three", "marker add at=0f as=third", 0)
        .unwrap();
    assert_eq!(edited.revision, 1);
    assert!(edited.document.contains("third"), "{}", edited.document);
}

#[test]
fn render_progress_is_not_journaled_and_finished_jobs_are_bounded() {
    let directory = std::env::temp_dir().join(format!("kerf-runtime-job-bound-{}", unique_id()));
    let journal_path = directory.join(JOURNAL_FILE);
    let mut runtime = Runtime::open(&directory).unwrap();
    let (running, _, _) = runtime
        .start_render_range(RenderQuality::Preview, None, None, None)
        .unwrap();
    runtime
        .update_job(
            &running.job_id,
            RenderStatus::Running,
            None,
            None,
            Some(0.0),
        )
        .unwrap();
    let journaled = load_journal(&journal_path, 0).unwrap().len();
    let since_checkpoint = runtime.journal_records_since_checkpoint;
    for step in 1..100 {
        let progress = f64::from(step) / 100.0;
        runtime
            .update_job(
                &running.job_id,
                RenderStatus::Running,
                None,
                None,
                Some(progress),
            )
            .unwrap();
    }
    assert_eq!(load_journal(&journal_path, 0).unwrap().len(), journaled);
    assert_eq!(runtime.journal_records_since_checkpoint, since_checkpoint);
    assert_eq!(runtime.render_job(&running.job_id).unwrap().progress, 0.99);

    let mut finished = Vec::new();
    for _ in 0..MAX_FINISHED_JOBS + 5 {
        let (job, _, _) = runtime
            .start_render_range(RenderQuality::Preview, None, None, None)
            .unwrap();
        runtime
            .update_job(&job.job_id, RenderStatus::Completed, None, None, None)
            .unwrap();
        finished.push(job.job_id);
    }
    let (queued, _, _) = runtime
        .start_render_range(RenderQuality::Preview, None, None, None)
        .unwrap();
    // The running and queued jobs, and the newest finished ones.
    assert_eq!(runtime.state.jobs.len(), MAX_FINISHED_JOBS + 2);
    assert!(runtime.render_job(&running.job_id).is_ok());
    assert!(runtime.render_job(&queued.job_id).is_ok());
    assert!(runtime.render_job(&finished[4]).is_err());
    assert!(runtime.render_job(&finished[5]).is_ok());
    drop(runtime);

    // A restart fails the running and queued jobs, which then count.
    let reopened = Runtime::open(&directory).unwrap();
    assert_eq!(reopened.state.jobs.len(), MAX_FINISHED_JOBS);
    assert!(reopened.render_job(&queued.job_id).is_ok());
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

/// What `update_job` refuses, each by its own code: a status kerfd has no name for, as a
/// client of another build may send one; a progress that is no fraction; and another end
/// for a job that has ended.
#[test]
fn a_job_update_kerfd_cannot_take_is_refused_by_its_own_code() {
    let root = std::env::temp_dir().join(format!("kerf-runtime-job-update-{}", unique_id()));
    // Off the wire the name still reads: it is refused as a status, not as a request kerfd
    // cannot read, and before the job is looked for. In a folder of its own: the runtime
    // that answers lets go of its folder a moment after the answer.
    let request = json!({
        "protocolVersion": 1, "id": "u", "operation": "update_job",
        "jobId": "none", "status": "paused", "error": null, "outputUrl": null,
    });
    let mut output = Vec::new();
    run_stdio(
        &root.join("wire"),
        std::io::Cursor::new(format!("{request}\n")),
        &mut output,
    )
    .unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(response["error"]["code"], "INVALID_RENDER_STATUS");

    let mut runtime = Runtime::open(&root.join("jobs")).unwrap();
    let (job, _, _) = runtime
        .start_render_range(RenderQuality::Preview, None, None, None)
        .unwrap();
    let id = job.job_id.as_str();
    let refused = runtime
        .update_job(id, RenderStatus::Running, None, None, Some(2.0))
        .unwrap_err();
    assert_eq!(refused.code.as_str(), "INVALID_RENDER_PROGRESS");
    runtime
        .update_job(id, RenderStatus::Completed, None, None, None)
        .unwrap();
    let refused = runtime
        .update_job(id, RenderStatus::Failed, Some("late".into()), None, None)
        .unwrap_err();
    assert_eq!(refused.code.as_str(), "RENDER_STATE_CONFLICT");
    // The same end said again is no conflict, and the job keeps how it ended.
    runtime
        .update_job(id, RenderStatus::Completed, None, None, None)
        .unwrap();
    let kept = runtime.render_job(id).unwrap();
    assert_eq!((kept.status.as_str(), kept.error), ("completed", None));
    drop(runtime);
    let _ = fs::remove_dir_all(root);
}
