//! What the editor's review and delivery screens ask of kerfd, over the
//! real socket (or named pipe) and from more than one client: proposals
//! waiting side by side and a revised version, the render jobs kept for
//! the queue, and titles placed by the generator insert op on a sequence
//! whose frame rate was set while it was empty.

#![cfg(any(unix, windows))]

use kerf_runtime::{Author, EditOp, RuntimeClient, StartRenderOptions, StateSnapshot};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A project folder with a running kerfd, both gone when the test ends.
struct Project {
    directory: PathBuf,
    daemon: Child,
}

impl Project {
    fn new(label: &str, document: Option<&str>) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("kerfd-review-{label}-{stamp}"));
        fs::create_dir_all(&directory).unwrap();
        if let Some(document) = document {
            fs::write(directory.join("project.kerf"), document).unwrap();
        }
        let daemon = Command::new(env!("CARGO_BIN_EXE_kerfd"))
            .arg("--dir")
            .arg(&directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn kerfd");
        let project = Self { directory, daemon };
        let client = project.client();
        // A busy Windows runner takes more than five seconds to start several at once.
        for _ in 0..1500 {
            if client.get_state().is_ok() {
                return project;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("kerfd did not become ready for {:?}", project.directory);
    }

    fn client(&self) -> RuntimeClient {
        RuntimeClient::new(&self.directory).unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Two seconds of a generated picture, so renders need no media.
const SOLID: &str = "kerf 1\nproject name=\"Queue\"\n\
    sequence s1 rate=24 size=64x36 audio=48000\n\
    \x20 track V1 video\n\
    \x20   clip c1 gen=solid@1 at=0f dur=48f\n\
    \x20     set color=#336699\n";

fn ops(value: serde_json::Value) -> Vec<EditOp> {
    serde_json::from_value(value).unwrap()
}

fn pending(state: &StateSnapshot) -> Vec<&kerf_runtime::ProposalEntry> {
    state
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
        .collect()
}

fn agent() -> Author {
    Author {
        id: Some("codex".into()),
        ..Author::new("mcp", "Codex")
    }
}

#[test]
fn proposals_from_two_clients_wait_side_by_side_and_a_revise_makes_version_two() {
    let project = Project::new("proposals", Some(SOLID));
    let (editor, mcp) = (project.client(), project.client());
    let base = editor.get_state().unwrap().revision;
    let kerf = editor
        .stage_script_proposal_by(
            "kerf",
            base,
            "marker add at=1s name=\"one\"",
            None,
            &Author::kerf(),
            None,
        )
        .unwrap()
        .proposals[0]
        .proposal_id
        .clone();
    let theirs = mcp
        .stage_script_proposal_by(
            "agent",
            base,
            "marker add at=1s name=\"two\"",
            None,
            &agent(),
            None,
        )
        .unwrap()
        .proposals[0]
        .proposal_id
        .clone();
    let both = editor.get_state().unwrap();
    assert_eq!(pending(&both).len(), 2, "{:#?}", both.proposals);
    // Newest first, each with its author.
    assert_eq!(both.proposals[0].proposal_id, theirs);
    assert_eq!(both.proposals[0].author, agent());
    assert_eq!(both.proposals[1].author, Author::kerf());

    // Accepting one makes the other stale for every client.
    let accepted = editor
        .accept_proposal(&kerf, "ui", "accept-kerf", both.revision)
        .unwrap();
    assert_eq!(accepted.revision, both.revision + 1);
    let seen = mcp.get_state().unwrap();
    let stale = seen
        .proposals
        .iter()
        .find(|entry| entry.proposal_id == theirs)
        .unwrap();
    assert_eq!(stale.status, "pending");
    assert_eq!(stale.freshness, kerf_runtime::ProposalFreshness::Stale);
    // A stale proposal cannot be accepted until it is revised.
    assert!(editor
        .accept_proposal(&theirs, "ui", "accept-stale", seen.revision)
        .is_err());

    // The revise is version 2 on the new revision, and keeps version 1.
    let revised = mcp
        .stage_script_proposal_by(
            "agent-2",
            seen.revision,
            "marker add at=1s name=\"two, again\"",
            None,
            &agent(),
            Some(&theirs),
        )
        .unwrap();
    let v2 = revised
        .proposals
        .iter()
        .find(|entry| entry.supersedes.as_deref() == Some(theirs.as_str()))
        .expect("version 2");
    assert_eq!(v2.version, 2);
    assert_eq!(v2.freshness, kerf_runtime::ProposalFreshness::Current);
    let versions: Vec<(u32, &str)> = v2
        .versions
        .iter()
        .map(|version| (version.version, version.proposal_id.as_str()))
        .collect();
    assert_eq!(
        versions,
        [(1, theirs.as_str()), (2, v2.proposal_id.as_str())]
    );
    assert!(!pending(&revised)
        .iter()
        .any(|entry| entry.proposal_id == theirs));
    let after = editor
        .accept_proposal(&v2.proposal_id, "ui", "accept-v2", revised.revision)
        .unwrap();
    assert!(after.document.contains("two, again"), "{}", after.document);
}

#[test]
fn the_render_queue_keeps_the_newest_finished_jobs_for_every_client() {
    let project = Project::new("queue", Some(SOLID));
    let (editor, other) = (project.client(), project.client());
    let start = |number: usize| {
        let options = StartRenderOptions {
            name: Some(format!("History {number}")),
            ..StartRenderOptions::new("full")
        };
        editor.start_render_with(&options).unwrap().0.job_id
    };
    for number in 0..52 {
        let job = start(number);
        assert_eq!(editor.cancel_render(&job).unwrap().status, "cancelled");
    }
    // kerfd trims the finished jobs as it queues one: the newest 50 stay,
    // oldest first, then the queued one, for any client that asks.
    let waiting = start(52);
    let jobs = other.list_renders().unwrap();
    let finished: Vec<&str> = jobs
        .iter()
        .filter(|job| job.status != "queued" && job.status != "running")
        .map(|job| job.name.as_deref().unwrap_or(""))
        .collect();
    let expected: Vec<String> = (2..52).map(|number| format!("History {number}")).collect();
    assert_eq!(finished, expected);
    let last = jobs.last().unwrap();
    assert_eq!(
        (last.job_id.as_str(), last.status.as_str()),
        (waiting.as_str(), "queued")
    );
    assert_eq!(last.quality.as_deref(), Some("full"));
    assert_eq!(last.codec.as_deref(), Some("h264"));
    other.cancel_render(&waiting).unwrap();
}

fn fps(state: &StateSnapshot) -> f64 {
    state.timeline["fps"].as_f64().unwrap()
}

#[test]
fn titles_go_in_as_generators_once_the_empty_sequences_rate_is_set() {
    let project = Project::new("titles", None);
    let client = project.client();
    let empty = client.get_state().unwrap();
    let spot = client
        .apply_ops_request(
            "ui",
            "rate",
            ops(json!([{"op": "set-sequence", "rate": {"num": 25, "den": 1}, "width": 640, "height": 360}])),
            empty.revision,
        )
        .unwrap();
    assert_eq!(fps(&spot), 25.0);
    assert!(
        spot.document.contains("rate=25 size=640x360"),
        "{}",
        spot.document
    );
    let titled = client
        .apply_ops_request(
            "ui",
            "title",
            ops(json!([
                {"op": "add-track", "kind": "video", "id": "V2"},
                {"op": "insert", "generator": {"fx": "title", "params": {"text": "Atlas"}},
                 "at": 0, "frames": 50, "tracks": ["V2"], "mode": "overwrite"},
                {"op": "insert", "generator": {"fx": "solid", "params": {"color": "#ff0000"}},
                 "at": 0, "frames": 25, "tracks": ["V2"]}
            ])),
            spot.revision,
        )
        .unwrap();
    assert!(
        titled.document.contains("gen=title@1") && titled.document.contains("gen=solid@1"),
        "{}",
        titled.document
    );
    assert!((titled.duration - 3.0).abs() < 1e-9, "{}", titled.duration);
    // With clips in it, the rate stays, and nothing changes.
    let refused = client
        .apply_ops_request(
            "ui",
            "rate-again",
            ops(json!([{"op": "set-sequence", "rate": "29.97"}])),
            titled.revision,
        )
        .unwrap_err();
    assert!(
        refused.message.ends_with(kerf_runtime::RATE_NEEDS_EMPTY),
        "{refused:?}"
    );
    let kept = client.get_state().unwrap();
    assert_eq!((kept.revision, fps(&kept)), (titled.revision, 25.0));
    assert_eq!(kept.document, titled.document);
    // A project file that says the same opens the same in a new kerfd.
    let reopened = Project::new("titles-again", Some(&kept.document));
    let state = reopened.client().get_state().unwrap();
    assert_eq!(state.document, kept.document);
    assert_eq!(fps(&state), 25.0);
}
