//! A stale proposal brought forward onto the project as it is.

use super::*;
use serde_json::Value;

struct Project(PathBuf);

impl Project {
    /// Four clips of four seconds, end to end.
    fn new(label: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("kerf-runtime-{label}-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(PROJECT_FILE),
            "kerf 1\nproject name=\"Forward\"\n\n\
             media m1 path=\"a.mov\" rate=24 dur=00:10:00:00 video=1920x1080\n\n\
             sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00\n\
             \x20   clip c2 media=m1 src=00:01:00:00 at=00:00:04:00 dur=00:00:04:00\n\
             \x20   clip c3 media=m1 src=00:02:00:00 at=00:00:08:00 dur=00:00:04:00\n\
             \x20   clip c4 media=m1 src=00:03:00:00 at=00:00:12:00 dur=00:00:04:00\n",
        )
        .unwrap();
        Self(directory)
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

/// Stage `script` as Cursor's proposal on the project as it is.
fn propose(runtime: &mut Runtime, key: &str, script: &str) -> String {
    let revision = runtime.state.revision;
    let staged = runtime
        .stage_script_proposal_by(key, revision, script, None, &cursor(), None)
        .unwrap();
    staged.proposals[0].proposal_id.clone()
}

/// Apply `script` as the editor does, which leaves open proposals stale.
fn edit(runtime: &mut Runtime, key: &str, script: &str) -> StateSnapshot {
    let revision = runtime.state.revision;
    runtime
        .apply_script_request("ui", key, script, revision)
        .unwrap()
}

/// The proposal trims c3 and closes up; the edit accepted first trimmed c1
/// and closed up.
fn two_ripples(runtime: &mut Runtime) -> String {
    let id = propose(runtime, "trim-c3", "trim c3 out -12f ripple");
    edit(runtime, "trim-c1", "trim c1 out -24f ripple");
    id
}

/// The line of `document` that holds clip `id`.
fn clip_line<'a>(document: &'a str, id: &str) -> &'a str {
    document
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("clip {id} ")))
        .unwrap_or_else(|| panic!("no clip {id} in {document}"))
        .trim()
}

#[test]
fn a_stale_proposal_is_brought_forward_as_its_next_version() {
    let project = Project::new("forward");
    let mut runtime = project.open();
    let v1 = two_ripples(&mut runtime);
    let stale = runtime.snapshot();
    assert_eq!(stale.proposals[0].freshness, ProposalFreshness::Stale);
    // Accepting it is refused, and says what moves it. Nothing moves it
    // unasked.
    let refused = runtime
        .accept_proposal(&v1, "ui", "accept-stale", 1)
        .unwrap_err();
    assert_eq!(refused.code, "PROPOSAL_BASE_MISMATCH");
    assert!(
        refused.message.contains("bring it forward onto revision 1"),
        "{}",
        refused.message
    );
    assert_eq!(runtime.snapshot().proposals, stale.proposals);

    let records = runtime.journal_sequence;
    let forwarded = runtime.forward_proposal("forward-1", 1, &v1).unwrap();
    assert_eq!(
        runtime.journal_sequence,
        records + 1,
        "one journal record keeps it"
    );
    assert_eq!(forwarded.revision, 1, "a proposal is not a revision");
    assert_eq!(forwarded.document, stale.document);
    assert_eq!(forwarded.proposals.len(), 1);
    let entry = &forwarded.proposals[0];
    assert_eq!(entry.status, "pending");
    assert_eq!(entry.freshness, ProposalFreshness::Current);
    assert_eq!(entry.base_revision, 1);
    assert_eq!(entry.version, 2);
    assert_eq!(entry.supersedes.as_deref(), Some(v1.as_str()));
    assert_eq!(entry.author, cursor(), "it is still its author's edit");
    assert_eq!(entry.script, "trim c3 out -12f ripple\n");
    assert_eq!(entry.versions.len(), 2);
    assert_eq!(entry.versions[0].proposal_id, v1);
    assert_eq!(entry.versions[0].base_revision, 0);
    assert!(
        entry
            .document_diff
            .contains("-    clip c3 media=m1 src=00:02:00:00 at=00:00:07:00 dur=00:00:04:00\n"),
        "the diff is against the project as it is: {}",
        entry.document_diff
    );
    // A retry changes nothing more, and the key is that request's alone.
    let again = runtime.forward_proposal("forward-1", 1, &v1).unwrap();
    assert_eq!(again.proposals, forwarded.proposals);
    assert_eq!(runtime.journal_sequence, records + 1);
    assert_eq!(
        runtime
            .forward_proposal("forward-1", 1, "another")
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    assert!(
        runtime.state.bases.is_empty(),
        "no open proposal is stale, so no project is kept for one"
    );

    // Reviewed again and accepted, c4 starts where c3 ends. Merged by ids,
    // it was left half a second after it.
    let v2 = entry.proposal_id.clone();
    let accepted = runtime
        .accept_proposal(&v2, "ui", "accept-forwarded", 1)
        .unwrap();
    assert_eq!(accepted.revision, 2);
    assert_eq!(
        clip_line(&accepted.document, "c3"),
        "clip c3 media=m1 src=00:02:00:00 at=00:00:07:00 dur=00:00:03:12"
    );
    assert_eq!(
        clip_line(&accepted.document, "c4"),
        "clip c4 media=m1 src=00:03:00:00 at=00:00:10:12 dur=00:00:04:00"
    );
}

#[test]
fn a_proposal_whose_clip_changed_is_refused_by_name_and_stays() {
    let project = Project::new("forward-refused");
    let mut runtime = project.open();
    let id = propose(&mut runtime, "trim-c3", "trim c3 out -12f ripple");
    edit(&mut runtime, "cut-c3", "trim c3 in +6f");
    let before = runtime.snapshot();
    let records = runtime.journal_sequence;
    let error = runtime.forward_proposal("forward-1", 1, &id).unwrap_err();
    assert_eq!(error.code, "PROPOSAL_NOT_FORWARDED");
    assert!(
        error
            .message
            .contains("1:1: clip `c3` was cut since this was proposed"),
        "{}",
        error.message
    );
    let after = runtime.snapshot();
    assert_eq!(after.proposals, before.proposals);
    assert_eq!(after.proposals[0].proposal_id, id);
    assert_eq!(after.proposals[0].freshness, ProposalFreshness::Stale);
    assert_eq!(after.document, before.document);
    assert_eq!(runtime.journal_sequence, records, "nothing was recorded");
    // It can still be revised, or brought forward once the clip is as it was.
    runtime.undo_request("ui", "undo-cut", 1).unwrap();
    let forwarded = runtime.forward_proposal("forward-2", 2, &id).unwrap();
    assert_eq!(forwarded.proposals[0].version, 2);
    assert_eq!(forwarded.proposals[0].script, "trim c3 out -12f ripple\n");
}

#[test]
fn a_place_a_proposal_names_moves_with_its_clips() {
    let project = Project::new("forward-place");
    let mut runtime = project.open();
    // At the cut between c2 and c3, and inside c3.
    let id = propose(
        &mut runtime,
        "places",
        "marker add as=cut at=00:00:08:00\nsplit c3 at=00:00:10:00",
    );
    edit(&mut runtime, "trim-c1", "trim c1 out -24f ripple");
    let forwarded = runtime.forward_proposal("forward-1", 1, &id).unwrap();
    assert_eq!(
        forwarded.proposals[0].script,
        "marker add as=cut at=00:00:07:00\nsplit c3 at=00:00:09:00\n"
    );
    assert_eq!(forwarded.proposals[0].source, forwarded.proposals[0].script);
    // The version before it keeps the places it was reviewed with.
    assert_eq!(
        forwarded.proposals[0].versions[0].script,
        "marker add as=cut at=00:00:08:00\nsplit c3 at=00:00:10:00\n"
    );
}

#[test]
fn a_forwarded_proposal_is_the_same_after_a_restart() {
    let project = Project::new("forward-restart");
    let mut runtime = project.open();
    let v1 = two_ripples(&mut runtime);
    // Stale across a restart: the journal says what the project was.
    drop(runtime);
    let mut runtime = project.open();
    assert!(runtime.state.bases.contains_key(&0));
    let forwarded = runtime.forward_proposal("forward-1", 1, &v1).unwrap();
    drop(runtime);
    let mut runtime = project.open();
    let reopened = runtime.snapshot();
    assert_eq!(reopened.proposals, forwarded.proposals);
    assert_eq!(reopened.document, forwarded.document);
    assert_eq!(reopened.revision, forwarded.revision);
    assert!(runtime.state.bases.is_empty());
    // And it is accepted there as it would have been.
    let v2 = reopened.proposals[0].proposal_id.clone();
    let accepted = runtime
        .accept_proposal(&v2, "ui", "accept-forwarded", 1)
        .unwrap();
    assert!(clip_line(&accepted.document, "c4").contains("at=00:00:10:12"));
    drop(runtime);
    assert_eq!(project.open().snapshot().document, accepted.document);
}

#[test]
fn the_project_a_proposal_was_made_on_survives_a_checkpoint_and_undo() {
    let project = Project::new("forward-checkpoint");
    let mut runtime = project.open();
    edit(&mut runtime, "first", "marker add as=first at=0f");
    let id = propose(&mut runtime, "trim-c3", "trim c3 out -12f ripple");
    // Undone and edited again: history no longer holds revision 1.
    runtime.undo_request("ui", "undo-first", 1).unwrap();
    edit(&mut runtime, "trim-c1", "trim c1 out -24f ripple");
    assert!(runtime.state.future.is_empty());
    assert_eq!(runtime.state.bases.keys().copied().collect::<Vec<_>>(), [1]);
    runtime.persist_checkpoint().unwrap();
    truncate_journal(&runtime.journal_path).unwrap();
    drop(runtime);
    let mut runtime = project.open();
    let forwarded = runtime.forward_proposal("forward-1", 3, &id).unwrap();
    assert_eq!(forwarded.proposals[0].base_revision, 3);
    assert_eq!(forwarded.proposals[0].version, 2);
}

#[test]
fn only_a_stale_open_proposal_whose_project_is_kept_is_brought_forward() {
    let project = Project::new("forward-errors");
    let mut runtime = project.open();
    let v1 = propose(&mut runtime, "trim-c3", "trim c3 out -12f ripple");
    let code = |runtime: &mut Runtime, revision: u64, id: &str| {
        runtime
            .forward_proposal(&format!("forward-{}", unique_id()), revision, id)
            .unwrap_err()
            .code
    };
    assert_eq!(code(&mut runtime, 0, &v1), "PROPOSAL_NOT_STALE");
    assert_eq!(code(&mut runtime, 0, "nope"), "PROPOSAL_NOT_FOUND");
    assert_eq!(code(&mut runtime, 7, &v1), "REVISION_CONFLICT");
    assert_eq!(
        runtime.forward_proposal(" ", 0, &v1).unwrap_err().code,
        "INVALID_REQUEST"
    );
    edit(&mut runtime, "trim-c1", "trim c1 out -24f ripple");
    let forwarded = runtime.forward_proposal("forward-1", 1, &v1).unwrap();
    let v2 = forwarded.proposals[0].proposal_id.clone();
    edit(&mut runtime, "mark", "marker add as=mark at=0f");
    // The version before the newest is not what waits for review.
    assert_eq!(code(&mut runtime, 2, &v1), "PROPOSAL_SUPERSEDED");

    // A checkpoint from before these projects were kept has none: the
    // proposal is revised, as it was then.
    runtime.persist_checkpoint().unwrap();
    truncate_journal(&runtime.journal_path).unwrap();
    drop(runtime);
    let checkpoint_path = project.0.join(CHECKPOINT_FILE);
    let mut checkpoint: Value =
        serde_json::from_str(&fs::read_to_string(&checkpoint_path).unwrap()).unwrap();
    assert!(checkpoint
        .as_object_mut()
        .unwrap()
        .remove("bases")
        .is_some());
    fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();
    let mut runtime = project.open();
    let gone = runtime.forward_proposal("forward-2", 2, &v2).unwrap_err();
    assert_eq!(gone.code, "PROPOSAL_BASE_GONE");
    assert!(gone.message.contains("revise the proposal instead"));

    // Settled, it is not brought anywhere.
    runtime.reject_proposal(&v2).unwrap();
    assert_eq!(code(&mut runtime, 2, &v2), "PROPOSAL_NOT_PENDING");
}
