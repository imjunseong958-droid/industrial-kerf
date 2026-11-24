//! Several proposals at once, their versions and authors, and the formats
//! older kerfds left on disk.

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
        ..Author::new(crate::AuthorKind::Mcp, "Cursor")
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
        assert_eq!(entry.status.as_str(), "pending");
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
    assert_eq!(kerf_entry.status.as_str(), "accepted");
    assert_eq!(kerf_entry.applied_revision, Some(1));
    assert!(kerf_entry.script.contains("as=kerf"));
    let cursor_entry = entry(&accepted, &cursor_id);
    assert_eq!(cursor_entry.status.as_str(), "pending");
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
    assert_eq!(stale.code.as_str(), "PROPOSAL_BASE_MISMATCH");
    assert_eq!(
        runtime
            .proposal_documents(&cursor_id)
            .unwrap_err()
            .code
            .as_str(),
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
    assert_eq!(reopened.proposals[0].status.as_str(), "rejected");
}
