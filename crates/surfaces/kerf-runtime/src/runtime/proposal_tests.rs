//! Several proposals at once, their versions and authors, the review log
//! beside them, and the formats older kerfds left on disk.

use super::*;
use serde_json::{json, Value};

struct Project(PathBuf);

impl Project {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("kerf-runtime-{label}-{}", unique_id())))
    }

    fn open(&self) -> Runtime {
        Runtime::open(&self.0).unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cursor() -> Author {
    Author {
        role: Some("MCP".into()),
        ..Author::new("mcp", "Cursor")
    }
}

fn entry<'a>(state: &'a StateSnapshot, id: &str) -> &'a ProposalEntry {
    state
        .proposals
        .iter()
        .find(|entry| entry.proposal_id == id || entry.versions.iter().any(|v| v.proposal_id == id))
        .unwrap_or_else(|| panic!("no proposal {id} in {:?}", state.proposals))
}

fn pending_id(state: &StateSnapshot) -> String {
    match state.pending_proposal.as_ref() {
        Some(PendingProposal::Pending { proposal_id, .. }) => proposal_id.clone(),
        other => panic!("expected a pending proposal, got {other:?}"),
    }
}

fn staged_id(state: &StateSnapshot) -> String {
    state.proposals[0].proposal_id.clone()
}

fn request(runtime: &mut Runtime, request: Value) -> Result<RuntimeResponse, RuntimeError> {
    runtime.execute(serde_json::from_value(request).unwrap())
}

fn comments(response: RuntimeResponse) -> (crate::ReviewLog, Option<crate::Comment>) {
    match response {
        RuntimeResponse::Comments { log, comment } => (log, comment),
        _ => panic!("expected comments"),
    }
}

#[test]
fn two_proposals_wait_together_and_accepting_one_stales_the_other() {
    let project = Project::new("two-proposals");
    let mut runtime = project.open();
    let kerf = runtime
        .stage_script_proposal("kerf-1", 0, "marker add at=0f as=kerf", None)
        .unwrap();
    let kerf_id = staged_id(&kerf);
    let both = runtime
        .stage_script_proposal_by(
            "cursor-1",
            0,
            "marker add at=12f as=cursor",
            None,
            &cursor(),
            None,
        )
        .unwrap();
    let cursor_id = staged_id(&both);
    assert_ne!(kerf_id, cursor_id);
    assert_eq!(both.proposals.len(), 2);
    // Newest first; the first open one is the pending proposal of old.
    assert_eq!(both.proposals[0].proposal_id, cursor_id);
    assert_eq!(both.proposals[0].author, cursor());
    assert_eq!(both.proposals[1].author, Author::kerf());
    assert_eq!(pending_id(&both), cursor_id);
    for entry in &both.proposals {
        assert_eq!(entry.status, "pending");
        assert_eq!(entry.freshness, ProposalFreshness::Current);
        assert_eq!(entry.version, 1);
        assert_eq!(entry.versions.len(), 1);
        assert!(entry.staged.is_some());
    }
    // Both can be previewed while they wait.
    assert!(runtime.proposal_documents(&kerf_id).is_ok());
    assert!(runtime.proposal_documents(&cursor_id).is_ok());

    let accepted = runtime
        .accept_proposal(&kerf_id, "ui", "accept-kerf", 0)
        .unwrap();
    assert_eq!(accepted.revision, 1);
    assert!(accepted.document.contains("marker kerf "));
    assert!(!accepted.document.contains("marker cursor "));
    let kerf_entry = entry(&accepted, &kerf_id);
    assert_eq!(kerf_entry.status, "accepted");
    assert_eq!(kerf_entry.applied_revision, Some(1));
    assert!(kerf_entry.script.contains("as=kerf"));
    let cursor_entry = entry(&accepted, &cursor_id);
    assert_eq!(cursor_entry.status, "pending");
    assert_eq!(cursor_entry.freshness, ProposalFreshness::Stale);
    assert!(matches!(
        accepted.pending_proposal,
        Some(PendingProposal::Pending { ref proposal_id, freshness: ProposalFreshness::Stale, .. })
            if *proposal_id == cursor_id
    ));
    // The stale one cannot be accepted or previewed against a project it
    // was not made for.
    let stale = runtime
        .accept_proposal(&cursor_id, "ui", "accept-cursor", 1)
        .unwrap_err();
    assert_eq!(stale.code, "PROPOSAL_BASE_MISMATCH");
    assert_eq!(
        runtime.proposal_documents(&cursor_id).unwrap_err().code,
        "PROPOSAL_STALE"
    );
    // A retried accept still converges on its receipt.
    let retried = runtime
        .accept_proposal(&kerf_id, "ui", "accept-kerf", 0)
        .unwrap();
    assert_eq!(retried.revision, 1);
    // And everything survives a restart.
    drop(runtime);
    let reopened = project.open().snapshot();
    assert_eq!(reopened.proposals, accepted.proposals);
}

#[test]
fn a_revise_keeps_v1_and_either_version_can_be_accepted() {
    let project = Project::new("revise");
    let mut runtime = project.open();
    let v1 = runtime
        .stage_script_proposal_by(
            "cursor-v1",
            0,
            "marker add at=0f as=first",
            None,
            &cursor(),
            None,
        )
        .unwrap();
    let v1_id = staged_id(&v1);
    let v2 = runtime
        .stage_script_proposal_by(
            "cursor-v2",
            0,
            "marker add at=2f as=second",
            None,
            &cursor(),
            Some(&v1_id),
        )
        .unwrap();
    assert_eq!(
        v2.proposals.len(),
        1,
        "a revise replaces the card it revises"
    );
    let current = &v2.proposals[0];
    assert_eq!(current.version, 2);
    assert_eq!(current.supersedes.as_deref(), Some(v1_id.as_str()));
    assert_eq!(current.versions.len(), 2);
    assert_eq!(current.versions[0].proposal_id, v1_id);
    assert_eq!(current.versions[0].version, 1);
    assert!(current.versions[0].script.contains("as=first"));
    assert!(current.versions[1].script.contains("as=second"));
    assert_eq!(current.versions[1].proposal_id, current.proposal_id);
    let v2_id = current.proposal_id.clone();
    // A retry of the revise changes nothing; a revise of what is gone fails.
    let again = runtime
        .stage_script_proposal_by(
            "cursor-v2",
            0,
            "marker add at=2f as=second",
            None,
            &cursor(),
            Some(&v1_id),
        )
        .unwrap();
    assert_eq!(again.proposals, v2.proposals);
    assert_eq!(
        runtime
            .stage_script_proposal_by("x", 0, "marker add at=0f", None, &cursor(), Some("nope"))
            .unwrap_err()
            .code,
        "PROPOSAL_NOT_FOUND"
    );
    // Each version previews on its own.
    let (_, first) = runtime.proposal_documents(&v1_id).unwrap();
    assert!(first.contains("marker first "));
    let (_, second) = runtime.proposal_documents(&v2_id).unwrap();
    assert!(second.contains("marker second "));

    // The versions survive a restart.
    drop(runtime);
    let mut runtime = project.open();
    assert_eq!(runtime.snapshot().proposals, v2.proposals);

    let accepted = runtime
        .accept_proposal(&v1_id, "ui", "accept-v1", 0)
        .unwrap();
    assert!(accepted.document.contains("marker first "));
    assert!(!accepted.document.contains("marker second "));
    let settled = &accepted.proposals[0];
    assert_eq!(settled.status, "accepted");
    assert_eq!(settled.proposal_id, v1_id);
    assert_eq!(settled.version, 1);
    assert_eq!(settled.versions.len(), 2);
    assert_eq!(
        runtime
            .accept_proposal(&v2_id, "ui", "accept-v2", 1)
            .unwrap_err()
            .code,
        "PROPOSAL_ALREADY_ACCEPTED"
    );
    // A settled proposal cannot be revised.
    assert_eq!(
        runtime
            .stage_script_proposal_by("x", 1, "marker add at=0f", None, &cursor(), Some(&v2_id))
            .unwrap_err()
            .code,
        "PROPOSAL_NOT_PENDING"
    );
}

#[test]
fn rejecting_takes_every_version_and_a_ninth_proposal_waits_for_room() {
    let project = Project::new("bounded");
    let mut runtime = project.open();
    let mut ids = Vec::new();
    for index in 0..MAX_PROPOSALS {
        let state = runtime
            .stage_script_proposal(
                &format!("agent-{index}"),
                0,
                &format!("marker add at={index}f as=m{index}"),
                None,
            )
            .unwrap();
        ids.push(staged_id(&state));
    }
    let full = runtime
        .stage_script_proposal("agent-full", 0, "marker add at=9f", None)
        .unwrap_err();
    assert_eq!(full.code, "PROPOSAL_PENDING");
    // A revise needs no room.
    let revised = runtime
        .stage_script_proposal_by(
            "agent-revise",
            0,
            "marker add at=20f as=r",
            None,
            &Author::kerf(),
            Some(&ids[0]),
        )
        .unwrap();
    let revised_id = staged_id(&revised);
    // Rejecting an earlier version rejects the proposal.
    let rejected = runtime
        .reject_proposal_request(&ids[0], "ui", "reject-v1", 0)
        .unwrap();
    assert_eq!(entry(&rejected, &revised_id).status, "rejected");
    assert_eq!(
        runtime
            .accept_proposal(&revised_id, "ui", "accept-rejected", 0)
            .unwrap_err()
            .code,
        "PROPOSAL_REJECTED"
    );
    let room = runtime
        .stage_script_proposal("agent-room", 0, "marker add at=9f as=room", None)
        .unwrap();
    // Eight open proposals leave no room for a settled one.
    assert_eq!(room.proposals.len(), MAX_PROPOSALS);
    assert!(room.proposals.iter().all(|entry| entry.status == "pending"));
    assert_eq!(
        runtime
            .accept_proposal(&revised_id, "ui", "accept-gone", 0)
            .unwrap_err()
            .code,
        "PROPOSAL_NOT_FOUND"
    );
}

#[test]
fn comments_never_touch_the_project_and_accepting_resolves_its_comments() {
    let project = Project::new("review");
    let mut runtime = project.open();
    let staged = runtime
        .stage_script_proposal("kerf-review", 0, "marker add at=0f as=reviewed", None)
        .unwrap();
    let proposal_id = staged_id(&staged);
    let before = runtime.snapshot();

    let (log, added) = comments(
        request(
            &mut runtime,
            json!({
                "operation": "add_comment",
                "actor": "agent",
                "comment": {
                    "author": {"kind": "kerf", "name": "Kerf"},
                    "body": "Added a marker for review",
                    "at": 1.0,
                    "duration": 0.5,
                    "proposal": proposal_id,
                    "steps": ["Read the cut"]
                }
            }),
        )
        .unwrap(),
    );
    let added = added.unwrap();
    assert_eq!(
        added.frame,
        Some(30),
        "1 s on the new project's 30p sequence"
    );
    assert_eq!(log.items, vec![added.clone()]);
    let after = runtime.snapshot();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.document, before.document);
    assert_eq!(after.can_undo, before.can_undo);
    assert_eq!(after.proposals[0].freshness, ProposalFreshness::Current);
    assert!(after.review_revision > before.review_revision);

    let (_, reply) = comments(
        request(
            &mut runtime,
            json!({
                "operation": "add_comment",
                "actor": "ui",
                "comment": {
                    "author": {"kind": "person", "name": "Mina Park", "id": "mina@example.com"},
                    "body": "Looks right",
                    "parent": added.id,
                }
            }),
        )
        .unwrap(),
    );
    let reply = reply.unwrap();

    let accepted = runtime
        .accept_proposal(&proposal_id, "ui", "accept-review", 0)
        .unwrap();
    assert!(accepted.review_revision > after.review_revision);
    let (log, _) = comments(request(&mut runtime, json!({"operation": "list_comments"})).unwrap());
    let linked = log.items.iter().find(|item| item.id == added.id).unwrap();
    assert!(linked.resolved);
    assert_eq!(linked.resolved_revision, Some(1));
    assert!(
        !log.items
            .iter()
            .find(|item| item.id == reply.id)
            .unwrap()
            .resolved
    );

    // Reopen through a request; the review counter moves, the project
    // does not.
    let (log, reopened) = comments(
        request(
            &mut runtime,
            json!({
                "operation": "set_comment",
                "commentId": added.id,
                "patch": {"resolved": false, "tags": ["markers"]},
            }),
        )
        .unwrap(),
    );
    let reopened = reopened.unwrap();
    assert!(!reopened.resolved);
    assert_eq!(reopened.tags, ["markers"]);
    assert_eq!(log.review_revision, runtime.snapshot().review_revision);
    assert_eq!(runtime.snapshot().revision, 1);
    assert_eq!(
        request(
            &mut runtime,
            json!({"operation": "set_comment", "commentId": "c-nope", "patch": {"resolved": true}}),
        )
        .unwrap_err()
        .code,
        "COMMENT_NOT_FOUND"
    );
    // The log is a file beside the project, which Git merges by union.
    assert!(project.0.join(crate::review::COMMENTS_FILE).is_file());
}

#[test]
fn every_revision_records_when_it_was_made() {
    let project = Project::new("revision-time");
    let mut runtime = project.open();
    assert_eq!(runtime.snapshot().revision_time, None);
    let edited = runtime
        .apply_script_request("ui", "timed", "marker add at=0f", 0)
        .unwrap();
    let time = edited.revision_time.clone().expect("a time for revision 1");
    assert!(time.ends_with('Z') && time.len() == "2026-11-04T10:12:00.000000Z".len());
    // A proposal staged later is not a revision.
    let staged = runtime
        .stage_script_proposal("timed-proposal", 1, "marker add at=2f", None)
        .unwrap();
    assert_eq!(staged.revision_time.as_ref(), Some(&time));
    drop(runtime);
    // The journal carries the time across a restart.
    assert_eq!(project.open().snapshot().revision_time, Some(time));
}

#[test]
fn state_from_the_one_proposal_slot_still_loads() {
    let project = Project::new("legacy-slot");
    let proposal_id = {
        let mut runtime = project.open();
        let staged = runtime
            .stage_script_proposal("legacy", 0, "marker add at=0f as=legacy", None)
            .unwrap();
        runtime.persist_checkpoint().unwrap();
        truncate_journal(&runtime.journal_path).unwrap();
        staged_id(&staged)
    };
    // The checkpoint as a kerfd with one proposal slot wrote it.
    let checkpoint_path = project.0.join(CHECKPOINT_FILE);
    let mut checkpoint: Value =
        serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
    let mut slot = checkpoint["proposals"][0].take();
    slot.as_object_mut().unwrap().remove("meta");
    let object = checkpoint.as_object_mut().unwrap();
    object.remove("proposals");
    object.insert("proposal".into(), slot.clone());
    fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();

    let runtime = project.open();
    let state = runtime.snapshot();
    assert_eq!(pending_id(&state), proposal_id);
    let entry = &state.proposals[0];
    assert_eq!(entry.author, Author::kerf());
    assert_eq!(entry.version, 1);
    assert_eq!(entry.versions.len(), 1);
    assert_eq!(entry.versions[0].proposal_id, proposal_id);

    // And a journal record with the one slot, as that kerfd appended it:
    // the proposal rejected.
    slot["lifecycle"] = json!({
        "status": "rejected", "proposalId": proposal_id, "baseRevision": 0
    });
    let mut record = JournalRecord::from_event(
        runtime.journal_sequence + 1,
        0,
        JournalEvent::Proposal {
            proposals: Vec::new(),
        },
    );
    record.proposals = None;
    record.proposal = Some(serde_json::from_value(slot).unwrap());
    let record = seal_journal_record(record).unwrap();
    let serialized = serde_json::to_string(&record).unwrap();
    assert!(!serialized.contains("\"proposals\""), "{serialized}");
    append_journal_record(&runtime.journal_path, &record).unwrap();
    drop(runtime);
    let reopened = project.open().snapshot();
    assert!(matches!(
        reopened.pending_proposal,
        Some(PendingProposal::Rejected { .. })
    ));
    assert_eq!(reopened.proposals[0].status, "rejected");
}
