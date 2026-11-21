//! The editor's state as the bridge's events leave it: each event folded in
//! through the public `EditorState::apply`, as the window folds them.

use kerf_editor::model::{timecode, Snapshot, Speaker};
use kerf_editor::store::{Change, Connection, EditorState, StepState};
use proptest::prelude::*;
use serde_json::{json, Value};

fn apply_all(state: &mut EditorState, events: &[Value]) -> Vec<Change> {
    events.iter().map(|event| state.apply(event)).collect()
}

/// The events a turn that fails at once sends, in the order the bridge
/// sends them (taken from a real run without an API key).
fn failed_turn() -> Vec<Value> {
    vec![
        json!({"type": "agent_busy", "busy": true}),
        json!({"type": "agent_state", "state": "running"}),
        json!({"type": "conversation", "id": "c-1", "title": "Trim the first clip", "archived": false,
               "items": [{"id": "i-1", "kind": "you", "text": "Trim the first clip", "at": "2026-11-20T03:49:08Z", "timecode": 0.0}]}),
        json!({"type": "conversations", "items": [{"id": "c-1", "title": "Trim the first clip", "archived": false, "count": 1}]}),
        json!({"type": "agent_state", "state": "failed", "message": "Add an API key in Settings"}),
        json!({"type": "error", "command": "agent", "message": "Add an API key in Settings"}),
        json!({"type": "conversation", "id": "c-1", "title": "Trim the first clip", "archived": false,
               "items": [{"id": "i-1", "kind": "you", "text": "Trim the first clip", "at": "x", "timecode": 0.0},
                         {"id": "i-2", "kind": "kerf", "text": "Add an API key in Settings", "at": "y", "error": true}]}),
        json!({"type": "agent_busy", "busy": false}),
    ]
}

#[test]
fn a_turn_shows_in_its_conversation_and_its_error_is_said_once() {
    let mut state = EditorState::default();
    let changes = apply_all(&mut state, &failed_turn());
    let conversation = state
        .open_conversation
        .as_ref()
        .expect("the conversation the turn went on in shows");
    assert_eq!(conversation.id, "c-1");
    assert_eq!(conversation.items.len(), 2);
    assert_eq!(conversation.items[0].kind, Speaker::You);
    assert_eq!(conversation.items[0].timecode, Some(0.0));
    assert!(conversation.items[1].error);
    assert_eq!(conversation.items[1].text, "Add an API key in Settings");
    assert_eq!(state.conversations.len(), 1);
    assert!(!state.work.busy());
    assert_eq!(changes.last(), Some(&Change::TurnEnded));
    // The failed state and the error say the same thing: one notice.
    assert_eq!(state.notices.len(), 1);
    assert!(state.notices[0].error);
}

#[test]
fn a_working_turn_keeps_its_steps_and_streams_its_answer() {
    let mut state = EditorState::default();
    apply_all(
        &mut state,
        &[
            json!({"type": "agent_busy", "busy": true}),
            json!({"type": "agent", "kind": "status", "text": "Planning next step…"}),
            json!({"type": "agent", "kind": "tool", "id": "t1", "state": "running", "text": "Reading the cut"}),
            json!({"type": "agent", "kind": "delta", "text": "I trimmed "}),
            json!({"type": "agent", "kind": "tool", "id": "t1", "state": "done", "text": "Read the cut"}),
            json!({"type": "agent", "kind": "tool", "id": "t2", "state": "failed", "text": "Render failed"}),
            json!({"type": "agent", "kind": "delta", "text": "the first clip."}),
            json!({"type": "agent_focus", "times": [1.5], "clips": ["one"]}),
        ],
    );
    assert!(state.work.busy());
    // Beside "Working", the step it is on: the last tool call's words.
    assert_eq!(state.work.status, "Render failed");
    assert_eq!(state.work.answer, "I trimmed the first clip.");
    let steps: Vec<_> = state
        .work
        .steps
        .iter()
        .map(|step| (step.text.as_str(), step.state))
        .collect();
    assert_eq!(
        steps,
        [
            ("Read the cut", StepState::Done),
            ("Render failed", StepState::Failed)
        ]
    );
    assert_eq!(state.agent_clips, ["one"]);
    state.apply(&json!({"type": "agent_busy", "busy": false}));
    assert!(!state.work.busy());
    assert!(
        state.agent_clips.is_empty(),
        "Kerf's cursor goes when the turn ends"
    );
}

#[test]
fn a_turn_that_ends_in_a_conversation_not_looked_at_leaves_a_dot() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "conversation", "id": "seen", "items": []}));
    state.apply(&json!({"type": "agent_busy", "busy": true}));
    state.work.conversation = Some("elsewhere".into());
    state.apply(&json!({"type": "agent_busy", "busy": false}));
    assert_eq!(state.settled, ["elsewhere"]);
    state.looked_at("elsewhere");
    assert!(state.settled.is_empty());
}

#[test]
fn the_connection_follows_what_kerfd_says() {
    let mut state = EditorState::default();
    assert_eq!(state.connection, Connection::Connecting);
    state.apply(&json!({"type": "connection", "status": "ready"}));
    assert_eq!(state.connection, Connection::Connected);
    state
        .apply(&json!({"type": "connection", "status": "disconnected", "message": "kerfd exited"}));
    assert_eq!(state.connection, Connection::Failed("kerfd exited".into()));
}

#[test]
fn a_render_job_is_followed_from_queued_to_done() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "export", "status": "queued", "jobId": "j1", "name": "Master", "path": "deliveries/Master.mov", "progress": 0.0, "quality": "master"}));
    state.apply(&json!({"type": "export", "status": "running", "jobId": "j1", "progress": 0.5}));
    assert_eq!(state.rendering().as_deref(), Some("Rendering Master 50%"));
    state.apply(&json!({"type": "export", "status": "completed", "jobId": "j1", "path": "deliveries/Master.mov", "progress": 1.0}));
    assert_eq!(state.renders.len(), 1);
    assert_eq!(state.renders[0].status, "completed");
    assert!(state.rendering().is_none());
    assert_eq!(
        state.notices.last().map(|notice| notice.text.as_str()),
        Some("Exported Master.mov")
    );
    // The queue as kerfd lists it replaces what was followed.
    state.apply(&json!({"type": "render_queue", "jobs": [{"jobId": "j2", "name": "Review copy", "status": "queued", "progress": 0.0}]}));
    assert_eq!(state.renders.len(), 1);
    assert!(state.renders[0].is_active());
}

#[test]
fn a_making_is_shown_until_it_is_made_or_stopped() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "media_making", "media": "m4", "take": 2, "line": ""}));
    state.apply(&json!({"type": "media_making", "media": "m4", "take": 2, "line": "half way"}));
    assert_eq!(state.makings["m4"].line, "half way");
    assert!(state.making_label().unwrap().starts_with("Making m4 · "));
    state.apply(&json!({"type": "media_made", "media": "m4", "take": 2}));
    assert!(state.makings.is_empty());
}

#[test]
fn a_snapshot_reads_its_timeline_and_proposals() {
    let snapshot = Snapshot::from_value(&json!({
        "document": "kerf 1",
        "revision": 7,
        "canUndo": true,
        "canRedo": false,
        "duration": 7.0,
        "timeline": {
            "name": "Harbour", "fps": 24.0, "width": 1920, "height": 1080, "duration": 7.0,
            "tracks": [{"id": "V1", "type": "video", "clips": [
                {"id": "one", "timelineStart": 0.0, "duration": 4.0, "startFrame": 0, "durationFrames": 96, "kind": "media", "sourceId": "a", "in": 0.0, "out": 4.0}
            ]}],
            "sources": [{"id": "a", "path": "media/a.mp4", "duration": 6.0, "width": 1280, "height": 720, "hasAudio": true}]
        },
        "proposals": [{"proposalId": "p2", "status": "pending", "freshness": "stale", "documentDiff": "--- a\n+++ b\n-old\n+new\n+more\n", "version": 2}],
        "proposalTimeline": {"fps": 24.0, "tracks": []}
    }));
    assert_eq!(snapshot.revision, 7);
    assert!(snapshot.can_undo && !snapshot.can_redo);
    let (track, clip) = snapshot.timeline.clip("one").unwrap();
    assert_eq!(track.id, "V1");
    assert_eq!(clip.end_frame(), 96);
    assert_eq!(snapshot.timeline.source("a").unwrap().file_name(), "a.mp4");
    assert!((snapshot.timeline.aspect() - 16.0 / 9.0).abs() < 1e-6);
    let pending = snapshot.pending().unwrap();
    assert!(pending.is_stale());
    assert_eq!(pending.diff_counts(), (2, 1));
    assert!(snapshot.proposal_timeline.is_some());
}

#[test]
fn a_newer_kerfd_saying_more_breaks_nothing() {
    let mut state = EditorState::default();
    state.choose(Some("c".into()));
    for event in [
        json!({"type": "something_new", "with": [1, 2, 3]}),
        json!({"type": "state", "snapshot": {"timeline": {"tracks": "not a list"}}}),
        json!({"type": "conversation", "id": "c", "items": [{"kind": "someone else"}, {"id": "ok", "kind": "kerf", "text": "Hi"}]}),
        json!({"type": "clock"}),
        json!({}),
    ] {
        state.apply(&event);
    }
    assert!(state.has_snapshot);
    let conversation = state.open_conversation.unwrap();
    assert_eq!(
        conversation.items.len(),
        1,
        "a turn that does not read is left out, the rest kept"
    );
}

#[test]
fn timecodes_count_whole_frames() {
    assert_eq!(timecode(0.0, 24.0), "00:00:00:00");
    assert_eq!(timecode(1.5, 24.0), "00:00:01:12");
    assert_eq!(timecode(3661.0, 25.0), "01:01:01:00");
    assert_eq!(timecode(-3.0, 24.0), "00:00:00:00");
}

proptest! {
    /// Whatever the bridge sends, in whatever order, folding it in never
    /// panics and never leaves a turn half-ended.
    #[test]
    fn any_events_fold_in(kinds in proptest::collection::vec(0usize..12, 0..40), numbers in proptest::collection::vec(-10.0f64..1e4, 40)) {
        let mut state = EditorState::default();
        for (index, kind) in kinds.iter().enumerate() {
            let number = numbers[index % numbers.len()];
            let event = match kind {
                0 => json!({"type": "agent_busy", "busy": number > 0.0}),
                1 => json!({"type": "agent", "kind": "delta", "text": "x"}),
                2 => json!({"type": "agent", "kind": "tool", "id": format!("t{}", number as i64 % 3), "state": "done", "text": "t"}),
                3 => json!({"type": "clock", "position": number, "duration": number.abs()}),
                4 => json!({"type": "export", "status": "running", "jobId": "j", "progress": number}),
                5 => json!({"type": "error", "message": format!("{number}")}),
                6 => json!({"type": "conversation", "id": format!("c{}", number as i64 % 2), "items": []}),
                7 => json!({"type": "media_making", "media": "m", "take": number as i64}),
                8 => json!({"type": "media_made", "media": "m"}),
                9 => json!({"type": "preview", "status": "running", "progress": number}),
                10 => json!({"type": "playback", "playing": number > 5.0}),
                _ => json!({"type": "agent_state", "state": "failed"}),
            };
            state.apply(&event);
            prop_assert!(state.notices.len() <= 4);
        }
        if !state.work.busy() {
            prop_assert!(state.work.request.is_none());
            prop_assert!(state.agent_clips.is_empty());
        }
    }
}

fn timeline(clips: &[(&str, &str, i64, i64)]) -> kerf_editor::model::Timeline {
    let mut tracks: Vec<Value> = Vec::new();
    for (track, id, start, frames) in clips {
        let clip = json!({"id": id, "timelineStart": *start as f64 / 24.0, "duration": *frames as f64 / 24.0,
                          "startFrame": start, "durationFrames": frames, "enabled": true, "in": 0.0});
        match tracks.iter_mut().find(|known| known["id"] == *track) {
            Some(known) => known["clips"].as_array_mut().unwrap().push(clip),
            None => tracks.push(json!({"id": track, "type": "video", "clips": [clip]})),
        }
    }
    kerf_editor::model::Timeline::from_value(
        &json!({"fps": 24.0, "duration": 30.0, "tracks": tracks}),
    )
}

#[test]
fn watching_a_change_plays_only_what_it_changes() {
    use kerf_editor::model::changed_span;
    let current = timeline(&[
        ("V1", "a", 0, 96),
        ("V1", "b", 96, 96),
        ("V1", "c", 192, 96),
    ]);
    // b trimmed by a second at its end: only b, a second either side.
    let trimmed = timeline(&[
        ("V1", "a", 0, 96),
        ("V1", "b", 96, 72),
        ("V1", "c", 192, 96),
    ]);
    assert_eq!(
        changed_span(&current, &trimmed, 1.0, 120.0),
        Some((3.0, 9.0))
    );
    // A clip taken out, and one put in later: from the first to the last.
    let replaced = timeline(&[
        ("V1", "a", 0, 96),
        ("V1", "c", 192, 96),
        ("V2", "d", 480, 48),
    ]);
    assert_eq!(
        changed_span(&current, &replaced, 0.0, 120.0),
        Some((4.0, 22.0))
    );
    // Held to the most a preview renders.
    assert_eq!(
        changed_span(&current, &replaced, 0.0, 5.0),
        Some((4.0, 9.0))
    );
    // Nothing changed: nothing to watch.
    assert_eq!(changed_span(&current, &current, 1.0, 120.0), None);
}

#[test]
fn a_making_that_fails_stops_showing_and_is_said_where_it_was_asked() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "media_making", "media": "m4", "take": 2, "line": ""}));
    let change = state.apply(&json!({"type": "error", "command": "media_make", "media": "m4", "message": "no model `shot` is set up"}));
    assert!(state.makings.is_empty(), "its stripes and its Stop go");
    assert_eq!(change, Change::Making);
    assert!(
        state.notices.is_empty(),
        "the plan or the card says it, not a notice over the conversation"
    );
    for command in [
        "recipe_edit",
        "shot_follow",
        "workflow_get",
        "takes_read",
        "media_carry",
    ] {
        state.apply(&json!({"type": "error", "command": command, "message": "refused"}));
    }
    assert!(state.notices.is_empty());
    // Anything else is said over the conversation.
    state.apply(
        &json!({"type": "error", "command": "export", "message": "The output folder must exist"}),
    );
    assert_eq!(state.notices.len(), 1);
}

#[test]
fn only_the_chosen_conversation_shows() {
    let mut state = EditorState::default();
    state.choose(Some("mine".into()));
    state.apply(&json!({"type": "conversation", "id": "mine", "items": [{"id": "1", "kind": "you", "text": "a"}]}));
    // Another window's turn in another conversation changes nothing here.
    state.apply(&json!({"type": "conversation", "id": "theirs", "items": [{"id": "2", "kind": "you", "text": "b"}]}));
    assert_eq!(
        state.open_conversation.as_ref().map(|c| c.id.as_str()),
        Some("mine")
    );
    // A new conversation shows nothing until the request sent in it starts one.
    state.choose(None);
    assert!(state.open_conversation.is_none());
    state.apply(&json!({"type": "conversation", "id": "theirs", "items": []}));
    assert!(state.open_conversation.is_none());
    state.apply(&json!({"type": "agent_busy", "busy": true}));
    state.apply(&json!({"type": "conversation", "id": "started", "items": [{"id": "3", "kind": "you", "text": "c"}]}));
    assert_eq!(state.chosen.as_deref(), Some("started"));
    assert_eq!(
        state.open_conversation.as_ref().map(|c| c.id.as_str()),
        Some("started")
    );
    state.apply(&json!({"type": "agent_busy", "busy": false}));
    assert!(
        state.work.conversation.is_none(),
        "a turn that ended is no longer under way anywhere"
    );
    assert!(state.settled.is_empty(), "it ended where you were looking");
}

#[test]
fn a_chosen_conversation_deleted_elsewhere_leaves_a_new_one() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "conversations", "items": [{"id": "a", "title": "A"}, {"id": "b", "title": "B"}]}));
    state.choose(Some("b".into()));
    state.apply(&json!({"type": "conversation", "id": "b", "items": []}));
    state.apply(&json!({"type": "conversations", "items": [{"id": "a", "title": "A"}]}));
    assert!(state.chosen.is_none());
    assert!(state.open_conversation.is_none());
}
