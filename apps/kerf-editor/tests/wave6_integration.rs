//! What the sixth wave's integration wired across the groups' files: the
//! picture's own rule for when its clip can be drawn on (the mask tools in
//! every menu follow it), the preview card's rules now the stage's, and the
//! sizes and holds the groups kept locally now named in the theme.

use kerf_editor::stage::shows_program;
use kerf_editor::store::EditorState;
use proptest::prelude::*;
use serde_json::{json, Value};

fn clip(id: &str, start: f64) -> Value {
    json!({"id": id, "mediaId": "m", "timelineStart": start, "duration": 2.0,
        "sourceIn": 0.0, "sourceOut": 2.0})
}

/// A state whose program has clips (or none), with a proposal waiting
/// (or not).
fn state(program_has_clips: bool, proposal_waits: bool) -> EditorState {
    let program = if program_has_clips {
        json!({"fps": 24.0, "tracks": [{"id": "V1", "type": "video", "clips": [clip("a", 0.0)]}]})
    } else {
        json!({"fps": 24.0, "tracks": [{"id": "V1", "type": "video", "clips": []}]})
    };
    let proposed =
        json!({"fps": 24.0, "tracks": [{"id": "V1", "type": "video", "clips": [clip("b", 0.0)]}]});
    let mut snapshot = json!({"timeline": program, "revision": 3});
    if proposal_waits {
        snapshot["proposalTimeline"] = proposed;
        snapshot["proposals"] = json!([{"proposalId": "p1", "status": "pending", "version": 1}]);
    }
    let mut state = EditorState::default();
    state.apply(&json!({"type": "state", "snapshot": snapshot}));
    state
}

#[test]
fn a_proposal_waiting_over_a_program_leaves_the_picture_the_program_s() {
    // Nothing waits: the program shows.
    assert!(shows_program(&state(true, false), true));
    // A proposal waits over a program with clips: kerfd still plays the
    // document, so its clips can be drawn on, as Qt's viewer drew on them.
    assert!(shows_program(&state(true, true), true));
    // A first cut proposed on an empty program stands in for it.
    assert!(!shows_program(&state(false, true), false));
}

#[test]
fn a_proposal_s_preview_stands_in_while_it_renders_or_shows() {
    for status in ["running", "ready", "live"] {
        let mut shown = state(true, true);
        shown.apply(&json!({"type": "preview", "status": status, "proposalId": "p1"}));
        assert!(!shows_program(&shown, true), "{status}");
        // The program's own preview is the program.
        let mut own = state(true, false);
        own.apply(&json!({"type": "preview", "status": status}));
        assert!(shows_program(&own, true), "{status}, the program's");
    }
    // A render that ended unshown leaves the program.
    for status in ["stopped", "exited", "failed", "required"] {
        let mut ended = state(true, true);
        ended.apply(&json!({"type": "preview", "status": status, "proposalId": "p1"}));
        assert!(shows_program(&ended, true), "{status}");
    }
}

proptest! {
    #[test]
    fn the_program_shows_unless_a_first_cut_or_a_proposal_s_preview_stands_in(
        has_clips in any::<bool>(),
        waits in any::<bool>(),
        status in prop::sample::select(vec![
            "", "running", "ready", "live", "required", "unavailable", "stopped", "exited", "failed",
        ]),
        of_proposal in any::<bool>(),
    ) {
        let mut state = state(has_clips, waits);
        if !status.is_empty() {
            let mut event = json!({"type": "preview", "status": status});
            if of_proposal {
                event["proposalId"] = json!("p1");
            }
            state.apply(&event);
        }
        let cut = waits && !has_clips;
        let preview = !status.is_empty()
            && of_proposal
            && matches!(status, "running" | "ready" | "live");
        prop_assert_eq!(shows_program(&state, has_clips), !cut && !preview);
    }
}
