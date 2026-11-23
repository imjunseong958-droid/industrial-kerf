//! A stale proposal brought forward onto the project as it is.

use super::*;

struct Project(PathBuf);

impl Project {
    /// Four clips of four seconds, end to end.
    fn new(label: &str) -> Self {
        let directory = std::env::temp_dir().join(format!("kerf-runtime-{label}-{}", unique_id()));
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
fn a_proposal_whose_clip_changed_is_refused_by_name_and_stays() {
    let project = Project::new("forward-refused");
    let mut runtime = project.open();
    let id = propose(&mut runtime, "trim-c3", "trim c3 out -12f ripple");
    edit(&mut runtime, "cut-c3", "trim c3 in +6f");
    let before = runtime.snapshot();
    let records = runtime.journal_sequence;
    let error = runtime.forward_proposal("forward-1", 1, &id).unwrap_err();
    assert_eq!(error.code.as_str(), "PROPOSAL_NOT_FORWARDED");
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
