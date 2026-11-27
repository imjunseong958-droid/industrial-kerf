//! The session: the bridge's events applied to the editor's state, the commands it sends, and what the model reads.

use crate::support::{
    apply, clip, command, event, preview, snapshot, snapshot_timeline, source, texts,
    timeline_value, EXISTS,
};
use kerf_bridge::event::PresetInspection;
use kerf_bridge::CommandName;
use kerf_editor::changes::clip_changes;
use kerf_editor::chat::try_asking;
use kerf_editor::model::{
    focus_seconds, timecode, Clip, ClipShow, RenderJobShow, Snapshot, SnapshotShow, Source,
    SourceShow, Speaker, Timeline, TimelineShow, TurnShow, STILL_SECONDS,
};
use kerf_editor::names;
use kerf_editor::names::{source_names, without_content_hash};
use kerf_editor::session::{
    activity_of, activity_of_command, cutting_status, nearer_frame, newest_change, preview_request,
    preview_span, scrubs_from, sent_unconnected, shot_of, source_range, stands_in, ActivityLog,
    LoopSpan, ACTIVITY_KEPT, LOOP_SLACK, REEL_MOST_RENDERED, SCRUB_AHEAD, SCRUB_SETTLE,
};
use kerf_editor::store::{
    making_status, next_steps, Badge, Change, Connection, EditorState, Making, MediaActivity,
    NextStep, StepState, PREVIEW_NOTICE_HOLD,
};
use kerf_editor::theme::motion;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

// ------------------------------------------------------------------ names

#[test]
fn an_imports_hash_comes_off_its_name() {
    assert_eq!(without_content_hash("Beach Day-1a2b3c4d"), "Beach Day");
    assert_eq!(without_content_hash("Beach Day -1a2b3c4d"), "Beach Day");
    // Not eight lowercase hex digits after a hyphen: as it was.
    assert_eq!(
        without_content_hash("Beach Day-1A2B3C4D"),
        "Beach Day-1A2B3C4D"
    );
    assert_eq!(
        without_content_hash("Beach Day-1a2b3c4"),
        "Beach Day-1a2b3c4"
    );
    assert_eq!(
        without_content_hash("Beach Day_1a2b3c4d"),
        "Beach Day_1a2b3c4d"
    );
    // Nothing before the hash: kept whole.
    assert_eq!(without_content_hash("-1a2b3c4d"), "-1a2b3c4d");
    assert_eq!(without_content_hash(" -1a2b3c4d"), " -1a2b3c4d");
}

proptest! {
    /// Whatever was before the hash comes back, trailing spaces trimmed.
    #[test]
    fn the_hash_comes_off_any_stem(
        stem in "[A-Za-z0-9 ._()é]{0,12}[A-Za-z0-9é]",
        hash in "[0-9a-f]{8}",
        gap in " {0,2}",
    ) {
        let named = format!("{stem}{gap}-{hash}");
        prop_assert_eq!(without_content_hash(&named), stem.trim_end());
        // And taking it off twice changes nothing more.
        let once = without_content_hash(&named).to_owned();
        prop_assert_eq!(without_content_hash(&once), once.as_str());
    }
}

#[test]
fn media_read_alike_only_where_they_must() {
    let mut made = source("m4", "shots/m4.takes/001.mp4");
    made.steps = 2;
    let mut recipe = source("m5", "shots/boat.takes/003.mp4");
    recipe.recipe = Some("shots/boat@2.kerfrecipe".into());
    let sources = vec![
        source("m1", "media/Beach Day-1a2b3c4d.mov"),
        source("m2", "media/Beach Day-9f8e7d6c.mp4"),
        source("m3", "media/Harbour-00aa11bb.mov"),
        made,
        recipe,
        source("m6", "media/Same-1a2b3c4d.mov"),
        source("m7", "other/Same-1a2b3c4d.mov"),
        source("m8", ""),
    ];
    let names = source_names(&sources);
    // Two stems alike: the extension comes back.
    assert_eq!(names["m1"], "Beach Day.mov");
    assert_eq!(names["m2"], "Beach Day.mp4");
    assert_eq!(names["m3"], "Harbour");
    // A shot made by steps goes by its id, one from a recipe by the recipe.
    assert_eq!(names["m4"], "m4");
    assert_eq!(names["m5"], "boat");
    // Even the file names alike: the whole file name, hash and all.
    assert_eq!(names["m6"], "Same-1a2b3c4d.mov");
    assert_eq!(names["m7"], "Same-1a2b3c4d.mov");
    assert_eq!(names["m8"], names::UNTITLED);
}

proptest! {
    /// Every media has a name, and two media read alike only when their
    /// whole file names are alike too.
    #[test]
    fn names_tell_media_apart_unless_their_files_are_alike(
        stems in proptest::collection::vec("[ab]{1,2}", 1..8),
        extensions in proptest::collection::vec(prop_oneof!["mov", "mp4"], 8),
        hashes in proptest::collection::vec("[0-9a-f]{8}", 8),
    ) {
        let sources: Vec<Source> = stems
            .iter()
            .enumerate()
            .map(|(at, stem)| source(&format!("m{at}"), &format!("media/{stem}-{}.{}", hashes[at], extensions[at])))
            .collect();
        let names = source_names(&sources);
        prop_assert_eq!(names.len(), sources.len());
        for one in &sources {
            for other in &sources {
                if one.id != other.id && names[&one.id] == names[&other.id] {
                    prop_assert_eq!(one.file_name(), other.file_name());
                }
            }
        }
    }
}

#[test]
fn a_clip_with_no_name_of_its_own_goes_by_its_medias() {
    let timeline = Timeline::from_value(&snapshot_timeline());
    let label = |id: &str| timeline.clip(id).unwrap().1.label().to_owned();
    assert_eq!(label("c1"), "Beach Day");
    // Its own name first; the document's name is left as it was.
    assert_eq!(label("c2"), "Wide");
    assert_eq!(timeline.clip("c1").unwrap().1.name, None);
    // Media with no file names none: the id stands.
    assert_eq!(label("c3"), "c3");
    assert_eq!(label("c4"), "Hello");
    assert_eq!(timeline.media_name("t"), "tone");
    assert_eq!(timeline.media_name("nope"), "nope");
}

#[test]
fn tracks_are_badged_and_media_say_where_they_are_used() {
    let timeline = Timeline::from_value(&snapshot_timeline());
    assert_eq!(timeline.track_badge("V1"), "V1");
    // Captions count among the picture tracks.
    assert_eq!(timeline.track_badge("S1"), "V2");
    assert_eq!(timeline.track_badge("A2"), "A2");
    let used = timeline.used_on();
    assert_eq!(used["a"], vec!["V1".to_owned(), "A2".to_owned()]);
    assert_eq!(used["t"], vec!["A1".to_owned()]);
}

#[test]
fn a_clip_shows_its_source_forwards_or_backwards() {
    let clip = Clip {
        source_in: Some(10.0),
        source_out: Some(14.0),
        duration: 2.0,
        speed: Some(2.0),
        ..Clip::default()
    };
    assert_eq!(clip.source_at(0.0), Some(10.0));
    assert_eq!(clip.source_at(1.0), Some(12.0));
    let reversed = Clip {
        reverse: true,
        ..clip.clone()
    };
    assert_eq!(reversed.source_at(0.0), Some(14.0));
    assert_eq!(reversed.source_at(2.0), Some(10.0));
    assert_eq!(Clip::default().source_at(1.0), None);
}

// ------------------------------------------------------------------ store

fn state_with_media() -> EditorState {
    let mut state = EditorState::default();
    let mut timeline = snapshot_timeline();
    timeline["sources"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "shot", "path": "shots/shot.mp4", "duration": 2.0, "width": 1920, "height": 1080, "steps": 1}));
    state.apply(&event(
        json!({"type": "state", "snapshot": {"timeline": timeline}}),
    ));
    state
}

#[test]
fn media_status_badges_with_missing_first() {
    let mut state = state_with_media();
    state.apply(&event(json!({"type": "media_status", "missing": ["a", "shot"], "pointers": ["a", "t"], "external": ["t", "gone"]})));
    let status = &state.media_status;
    assert_eq!(status.badge("a"), Some(Badge::Offline));
    assert_eq!(status.badge("t"), Some(Badge::NotDownloaded));
    assert_eq!(status.badge("gone"), Some(Badge::External));
    assert_eq!(status.badge("x"), None);
    // A missing shot made from steps is not made yet; other media is offline.
    assert_eq!(state.media_activity("shot"), MediaActivity::NotMade);
    assert_eq!(state.media_activity("a"), MediaActivity::Offline);
    assert_eq!(state.media_activity("t"), MediaActivity::Ready);
}

#[test]
fn a_making_waiting_its_turn_is_queued_until_it_runs() {
    let mut state = state_with_media();
    state.apply(&event(json!({"type": "media_making", "media": "shot", "take": 1, "line": "asked the server for 48 frames: job 7"})));
    assert_eq!(
        state.media_activity("shot"),
        MediaActivity::Queued { ahead: None }
    );
    state.apply(&event(json!({"type": "media_making", "media": "shot", "take": 1, "line": "waiting, 2 ahead, 4 s"})));
    assert_eq!(
        state.media_activity("shot"),
        MediaActivity::Queued { ahead: Some(2) }
    );
    state.apply(&event(
        json!({"type": "media_making", "media": "shot", "take": 1, "line": "running, 2 s"}),
    ));
    assert!(matches!(
        state.media_activity("shot"),
        MediaActivity::Making { take: 1, .. }
    ));
    let making = state.makings["shot"].clone();
    assert_eq!(making_status("shot", &making), "Making shot: running, 2 s");
    let starting = Making {
        line: String::new(),
        ..making
    };
    assert_eq!(
        making_status("shot", &starting),
        "Making take 1 of shot from its steps…"
    );
    state.apply(&event(
        json!({"type": "media_made", "media": "shot", "take": 1}),
    ));
    assert_eq!(
        state.notices.last().unwrap().text,
        "Made take 1 of shot from its steps"
    );
    assert!(!state.notices.last().unwrap().error);
    state.apply(&event(
        json!({"type": "media_making", "media": "a", "take": 1, "line": ""}),
    ));
    state.apply(&event(
        json!({"type": "media_make_cancelled", "media": "a"}),
    ));
    assert_eq!(
        state.notices.last().unwrap().text,
        "Stopped making a; it is as it was"
    );
    assert!(state.makings.is_empty());
}

#[test]
fn media_info_and_indexes_are_kept_and_go_with_a_relink() {
    let mut state = state_with_media();
    state.apply(&event(
        json!({"type": "media_info", "media": {"a": {"pixFmt": "yuva420p"}}}),
    ));
    state.apply(&event(
        json!({"type": "media_info", "media": {"t": {"codec": "pcm"}}}),
    ));
    assert_eq!(state.media_info["a"]["pixFmt"], "yuva420p");
    assert_eq!(state.media_info.len(), 2);
    state.index_asked.insert("a".into());
    state.apply(&event(
        json!({"type": "media_index", "media": "a", "cuts": [1.0]}),
    ));
    assert!(state.media_indexes.contains_key("a") && state.index_asked.is_empty());
    // A refusal is said on the Index page, not over the window.
    let notices = state.notices.len();
    state.apply(&event(
        json!({"type": "error", "command": "media_index", "media": "t", "message": "No sound"}),
    ));
    assert_eq!(state.media_index_errors["t"], "No sound");
    assert_eq!(state.notices.len(), notices);
    // The same file again keeps them; another file lets them go.
    let mut timeline = snapshot_timeline();
    state.apply(&event(
        json!({"type": "state", "snapshot": {"timeline": timeline.clone()}}),
    ));
    assert!(state.media_indexes.contains_key("a"));
    timeline["sources"][0]["path"] = json!("media/other.mov");
    state.apply(&event(
        json!({"type": "state", "snapshot": {"timeline": timeline}}),
    ));
    assert!(!state.media_indexes.contains_key("a"));
    assert!(!state.media_info.contains_key("a"));
    assert!(state.media_index_errors.contains_key("t"));
}

#[test]
fn a_cast_says_what_wants_making_and_saving_says_so() {
    let mut state = state_with_media();
    let cast = json!({"type": "cast", "saved": false, "entries": [],
                      "toMake": [{"media": "shot", "why": "cast Maya changed"}]});
    state.apply(&event(cast.clone()));
    assert_eq!(state.wants_making["shot"], "cast Maya changed");
    assert!(state.notices.is_empty());
    let saved = json!({"type": "cast", "saved": true, "entries": [],
                       "toMake": [{"media": "shot", "why": "x"}, {"media": "m9", "why": "y"}]});
    state.apply(&event(saved.clone()));
    assert_eq!(
        state.notices.last().unwrap().text,
        "Saved the cast; shot, m9 want making again"
    );
    state.apply(&event(
        json!({"type": "media_made", "media": "shot", "take": 2}),
    ));
    assert!(!state.wants_making.contains_key("shot"));
    let none = json!({"type": "cast", "saved": true, "toMake": []});
    assert_eq!(
        kerf_editor::store::cast_saved(&event(none.clone())),
        "Saved the cast; no shot wants making for it"
    );
    // What is wrong with a cast is said in its dialog.
    let notices = state.notices.len();
    state.apply(&event(
        json!({"type": "error", "command": "cast", "message": "line 2: no name"}),
    ));
    assert_eq!(state.notices.len(), notices);
}

#[test]
fn a_new_connection_asks_again() {
    let mut state = state_with_media();
    state.info_asked.insert("a".into());
    state.index_asked.insert("a".into());
    state.apply(&event(json!({"type": "connection", "status": "connected"})));
    assert!(state.info_asked.is_empty() && state.index_asked.is_empty());
}

// --------------------------------------------------------------- activity

#[test]
fn makings_takes_and_carries_leave_activity_lines() {
    let mut timeline = Timeline::from_value(&snapshot_timeline());
    let mut shot = source("m4", "shots/m4.takes/002.mp4");
    shot.steps = 2;
    shot.takes = vec![kerf_editor::model::Take {
        number: 1,
        path: "shots/m4.takes/001.mp4".into(),
        chosen: false,
    }];
    timeline.sources.push(shot);
    let line = |value: Value| activity_of_command(&command(value), &timeline).map(|(_, text)| text);
    assert_eq!(
        line(json!({"type": "media_make", "media": "m4", "confirmed": true})).as_deref(),
        Some("Making m4 from its steps in the document")
    );
    assert_eq!(
        line(json!({"type": "media_make", "media": "m4", "confirmed": true, "rolls": 3}))
            .as_deref(),
        Some("Making 3 takes of m4 from its steps in the document")
    );
    // Asking for the plan is not a making.
    assert_eq!(line(json!({"type": "media_make", "media": "m4"})), None);
    assert_eq!(
        line(json!({"type": "media_carry", "media": "m5", "confirmed": true})).as_deref(),
        Some("Carrying the steps of m5 into the document")
    );
    assert_eq!(
        line(json!({"type": "media_replace", "media": "m4", "path": "shots/m4.takes/001.mp4"}))
            .as_deref(),
        Some("Using take 1 of m4")
    );
    assert_eq!(
        line(json!({"type": "media_replace", "media": "a", "path": "/home/me/b.mov"})).as_deref(),
        Some("Replacing a with b.mov")
    );
    assert_eq!(line(json!({"type": "seek", "position": 1.0})), None);
}

#[test]
fn the_work_row_says_the_step_kerf_is_on_beside_working() {
    let mut state = EditorState::default();
    apply(
        &mut state,
        &[
            json!({"type": "agent_busy", "busy": true}),
            json!({"type": "agent", "kind": "status", "text": "Planning  next\nstep…"}),
        ],
    );
    assert_eq!(state.work.status, "Planning next step…");
    apply(
        &mut state,
        &[
            json!({"type": "agent", "kind": "tool", "id": "t1", "state": "running", "text": "Reading the cut"}),
        ],
    );
    assert_eq!(state.work.status, "Reading the cut");
    apply(
        &mut state,
        &[
            json!({"type": "agent", "kind": "tool", "id": "t1", "state": "done", "text": "Read the cut"}),
        ],
    );
    assert_eq!(state.work.status, "Read the cut");
    // A turn starts with nothing said yet beside "Working".
    apply(
        &mut state,
        &[
            json!({"type": "agent_busy", "busy": false}),
            json!({"type": "agent_busy", "busy": true}),
        ],
    );
    assert_eq!(state.work.status, "");
}

#[test]
fn activity_keeps_what_kerf_did() {
    let line = |event: Value| activity_of(&crate::support::event(event.clone()));
    assert_eq!(
        line(json!({"type": "agent", "kind": "tool", "state": "done", "text": "Trimmed two"})),
        Some(("tool", "Trimmed two".to_owned()))
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "tool", "state": "failed", "text": "Render failed"})),
        Some(("tool", "Render failed".to_owned()))
    );
    // A call still running is said once it ends.
    assert_eq!(
        line(json!({"type": "agent", "kind": "tool", "state": "running", "text": "Trimming"})),
        None
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "status", "text": "Planning next step…"})),
        Some(("status", "Planning next step…".to_owned()))
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "note", "text": "Asked for a key"})),
        Some(("agent", "Asked for a key".to_owned()))
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "delta", "text": "I"})),
        None
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "done", "text": "Done."})),
        None
    );
    let long = "x".repeat(300);
    assert_eq!(
        line(json!({"type": "agent", "kind": "status", "text": long}))
            .map(|(_, text)| text.chars().count()),
        Some(120)
    );
}

#[test]
fn the_context_meter_shows_only_in_the_conversation_it_is_of() {
    let mut state = EditorState::default();
    // A turn in a new conversation says how full it is before it has an id.
    apply(
        &mut state,
        &[
            json!({"type": "agent_busy", "busy": true}),
            json!({"type": "agent_context", "used": 1000, "limit": 4000, "unit": "tokens", "conversation": null}),
        ],
    );
    assert_eq!(state.context_shown().map(|c| c.percent()), Some(25));
    // The bridge names it: the reading is of it.
    apply(
        &mut state,
        &[json!({"type": "conversation", "id": "c1", "title": "Trim", "items": []})],
    );
    assert_eq!(state.chosen.as_deref(), Some("c1"));
    assert_eq!(
        state
            .context
            .as_ref()
            .and_then(|c| c.conversation.as_deref()),
        Some("c1")
    );
    apply(&mut state, &[json!({"type": "agent_busy", "busy": false})]);
    assert!(state.context_shown().is_some());
    // Another conversation, or a new one, has no reading of its own.
    state.choose(Some("c2".into()));
    assert!(state.context_shown().is_none());
    state.choose(None);
    assert!(state.context_shown().is_none());
    state.choose(Some("c1".into()));
    assert!(state.context_shown().is_some());
    // A reading of another conversation is not this one's.
    apply(
        &mut state,
        &[
            json!({"type": "agent_context", "used": 1, "limit": 4000, "unit": "kerf", "conversation": "c9"}),
        ],
    );
    assert!(state.context_shown().is_none());
}

#[test]
fn a_time_kerf_names_as_a_timecode_is_followed() {
    let mut state = EditorState::default();
    apply(
        &mut state,
        &[
            json!({"type": "state", "snapshot": {"revision": 1, "timeline":
            {"name": "Cut", "fps": 25.0, "start": 3600.0, "duration": 30.0, "tracks": []}}}),
        ],
    );
    apply(
        &mut state,
        &[
            json!({"type": "agent_focus", "times": ["01:00:04:12", 2.5, "50f", "an anchor"], "clips": []}),
        ],
    );
    assert_eq!(state.agent_focus.len(), 3);
    assert!((state.agent_focus[0] - (4.0 + 12.0 / 25.0)).abs() < 1e-9);
    assert_eq!(state.agent_focus[1], 2.5);
    assert_eq!(state.agent_focus[2], 2.0);
}

#[test]
fn kerf_at_work_goes_to_its_newest_change() {
    let program =
        Timeline::from_value(&timeline_value(&[clip("a", 0.0, 4.0), clip("b", 4.0, 4.0)]));
    let trimmed =
        Timeline::from_value(&timeline_value(&[clip("a", 0.0, 3.0), clip("b", 4.0, 4.0)]));
    let changes = clip_changes(&program, &trimmed);
    let (keys, at) = newest_change(&changes, &HashSet::new());
    // A trim: where it cuts.
    assert_eq!(at, Some(3.0));
    // Seen already: nothing new to go to.
    assert_eq!(newest_change(&changes, &keys).1, None);
    let moved = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 6.0, 4.0),
        clip("c", 12.0, 2.0),
    ]));
    // A move and a new clip: the latest start of them.
    assert_eq!(
        newest_change(&clip_changes(&program, &moved), &HashSet::new()).1,
        Some(12.0)
    );
    // Taking a clip away is nothing to look at.
    let removed = Timeline::from_value(&timeline_value(&[clip("a", 0.0, 4.0)]));
    assert_eq!(
        newest_change(&clip_changes(&program, &removed), &HashSet::new()).1,
        None
    );
    assert_eq!(cutting_status(1), "Cutting · 1 clip");
    assert_eq!(cutting_status(3), "Cutting · 3 clips");
}

#[test]
fn a_first_cut_renders_again_once_a_reel_over_it_ends() {
    use kerf_editor::session::{CutRender, CUT_SETTLE};
    use std::time::Instant;
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let settle = CUT_SETTLE.as_millis() as u64;
    let mut cut = CutRender::default();
    // Kerf stages a first cut; once its edits are quiet it renders.
    cut.follow(Some("p1"), true, false, at(0));
    assert_eq!(cut.take_due(Some("p1"), false, at(settle - 1)), None);
    assert_eq!(
        cut.take_due(Some("p1"), false, at(settle)),
        Some("p1".to_owned())
    );
    cut.set_rendering(true);
    cut.rendered(Some("p1"), false, true, at(settle + 100));
    assert!(!cut.rendering());
    assert_eq!(cut.due(), None);
    // The same cut again: it was rendered, nothing is due.
    cut.follow(Some("p1"), true, false, at(settle + 200));
    assert_eq!(cut.due(), None);
    // Watch what changed, then Exit: the program came back under the
    // reel's picture, so the cut renders again once settled.
    let exit = settle + 5_000;
    cut.reel_ended(Some("p1"), at(exit));
    assert_eq!(cut.previewed(), None);
    assert_eq!(cut.take_due(Some("p1"), false, at(exit + settle - 1)), None);
    assert_eq!(
        cut.take_due(Some("p1"), false, at(exit + settle)),
        Some("p1".to_owned())
    );
    // A reel that ends with no first cut shown leaves nothing to render.
    let mut none = CutRender::default();
    none.reel_ended(None, at(0));
    assert_eq!(none.due(), None);
}

#[derive(Clone, Debug)]
enum CutStep {
    Follow(Option<u8>, bool, bool),
    TakeDue(Option<u8>, bool),
    Asked(bool),
    Rendered(Option<u8>, bool, bool),
    ReelEnded(Option<u8>),
}

fn cut_step() -> impl Strategy<Value = CutStep> {
    let id = proptest::option::of(0u8..3);
    prop_oneof![
        (id.clone(), any::<bool>(), any::<bool>())
            .prop_map(|(id, follow, reel)| CutStep::Follow(id, follow, reel)),
        (id.clone(), any::<bool>()).prop_map(|(id, reel)| CutStep::TakeDue(id, reel)),
        any::<bool>().prop_map(CutStep::Asked),
        (id.clone(), any::<bool>(), any::<bool>())
            .prop_map(|(id, still, follow)| CutStep::Rendered(id, still, follow)),
        id.prop_map(CutStep::ReelEnded),
    ]
}

proptest! {
    /// Whatever the bridge says in whatever order: a render is given only
    /// for the cut shown, never while one runs or a reel plays, and never
    /// before it is due; what is due is never further than `CUT_SETTLE`
    /// from the last step; a cut no longer shown forgets it all; and a
    /// reel ending over a cut always leaves it to render.
    #[test]
    fn a_first_cut_renders_only_when_settled_and_free(
        steps in proptest::collection::vec((cut_step(), 0u64..800), 1..60)
    ) {
        use kerf_editor::session::{CutRender, CUT_SETTLE};
        use std::time::Instant;
        let name = |id: Option<u8>| id.map(|id| format!("p{id}"));
        let mut now = Instant::now();
        let mut cut = CutRender::default();
        for (step, wait) in steps {
            now += Duration::from_millis(wait);
            let due_before = cut.due();
            let rendering_before = cut.rendering();
            match step {
                CutStep::Follow(id, follow, reel) => {
                    let id = name(id);
                    cut.follow(id.as_deref(), follow, reel, now);
                    if id.is_none() {
                        prop_assert_eq!(cut.due(), None);
                        prop_assert_eq!(cut.previewed(), None);
                    }
                }
                CutStep::TakeDue(id, reel) => {
                    let id = name(id);
                    let given = cut.take_due(id.as_deref(), reel, now);
                    if let Some(given) = given {
                        prop_assert_eq!(Some(&given), id.as_ref());
                        prop_assert!(!reel && !rendering_before);
                        prop_assert!(due_before.is_some_and(|due| due <= now));
                        prop_assert_eq!(cut.previewed(), Some(given.as_str()));
                        prop_assert_eq!(cut.due(), None);
                    }
                }
                CutStep::Asked(taken) => cut.set_rendering(taken),
                CutStep::Rendered(id, still, follow) => {
                    let id = name(id);
                    cut.rendered(id.as_deref(), still, follow, now);
                    if still {
                        prop_assert_eq!(cut.rendering(), rendering_before);
                    }
                }
                CutStep::ReelEnded(id) => {
                    let id = name(id);
                    cut.reel_ended(id.as_deref(), now);
                    if id.is_some() {
                        prop_assert_eq!(cut.previewed(), None);
                        prop_assert_eq!(cut.due(), Some(now + CUT_SETTLE));
                    } else {
                        prop_assert_eq!(cut.due(), due_before);
                    }
                }
            }
            if let Some(due) = cut.due() {
                prop_assert!(due <= now + CUT_SETTLE);
            }
        }
    }
}

/// A step as the bridge sends it, or as plain words.
#[derive(Clone, Debug)]
enum Shape {
    Object(String, String),
    Words(String),
}

fn shape() -> impl Strategy<Value = Shape> {
    let words = "[ a-zA-Z]{0,12}";
    prop_oneof![
        (words, words).prop_map(|(label, request)| Shape::Object(label, request)),
        words.prop_map(Shape::Words),
    ]
}

fn as_value(shape: &Shape) -> Value {
    match shape {
        Shape::Object(label, request) => json!({"label": label, "request": request}),
        Shape::Words(words) => json!(words),
    }
}

/// What a step reads as: the words as label and request, or the object's
/// label and request (its label when it has none); none without a label.
fn as_read(shape: &Shape) -> Option<NextStep> {
    let (label, request) = match shape {
        Shape::Object(label, request) => (label.trim(), request.trim()),
        Shape::Words(words) => (words.trim(), words.trim()),
    };
    (!label.is_empty())
        .then(|| NextStep::new(label, if request.is_empty() { label } else { request }))
}

proptest! {
    /// Kerf's next steps read in either shape: `{label, request}` as the
    /// bridge sends them, or plain words as both; a step with no label is
    /// left out, the rest kept in their order.
    #[test]
    fn next_steps_read_in_either_shape(shapes in proptest::collection::vec(shape(), 0..6)) {
        let event = json!({"type": "agent_next", "steps": shapes.iter().map(as_value).collect::<Vec<_>>()});
        let expected: Vec<NextStep> = shapes.iter().filter_map(as_read).collect();
        prop_assert_eq!(&next_steps(&crate::support::event(event.clone())), &expected);
        for step in &expected {
            prop_assert!(!step.label.is_empty() && !step.request.is_empty());
            prop_assert_eq!(step.label.trim(), step.label.as_str());
        }
        let mut state = EditorState::default();
        state.apply(&crate::support::event(event.clone()));
        prop_assert_eq!(&state.next_steps, &expected);
        // Try asking shows at most four of them, else the first requests.
        let offered = try_asking(&state.next_steps);
        if expected.is_empty() {
            prop_assert_eq!(offered.len(), 3);
        } else {
            prop_assert_eq!(&offered[..], &expected[..expected.len().min(4)]);
        }
    }
}

#[test]
fn kerf_s_next_steps_show_their_label_and_ask_their_request() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "agent_next", "steps": [
        {"label": "Stronger glow", "request": "Make the glow on the title stronger"},
        "Tighten the pauses",
        {"label": "  ", "request": "nothing to press"},
        {"label": "Captions"},
        42
    ]})));
    assert_eq!(
        state.next_steps,
        [
            NextStep::new("Stronger glow", "Make the glow on the title stronger"),
            NextStep::new("Tighten the pauses", "Tighten the pauses"),
            NextStep::new("Captions", "Captions"),
        ]
    );
    // An empty conversation's Try asking offers them, label and request.
    assert_eq!(try_asking(&state.next_steps), state.next_steps);
    // A new turn takes them away until Kerf offers more.
    state.apply(&event(json!({"type": "agent_busy", "busy": true})));
    assert!(state.next_steps.is_empty());
    assert_eq!(
        try_asking(&state.next_steps)[0].label,
        "Make a 30-second cut from the best takes"
    );
}

#[test]
fn the_preview_s_words_take_the_last_ones_place() {
    let mut state = EditorState::default();
    state.notify("Exported cut.mov", false);
    state.apply(&event(
        json!({"type": "preview", "status": "required", "message": "A title needs a render"}),
    ));
    let texts = |state: &EditorState| -> Vec<String> {
        state
            .notices
            .iter()
            .map(|notice| notice.text.clone())
            .collect()
    };
    assert_eq!(
        texts(&state),
        ["Exported cut.mov", "A title needs a render"]
    );
    assert_eq!(state.notices[1].hold, None, "the window's 6 s");
    assert!(!state.notices[1].error);
    // Rendered: "Preview ready" in its place, for 4 s.
    state.apply(&event(
        json!({"type": "preview", "status": "running", "progress": 0.5}),
    ));
    state.apply(&event(
        json!({"type": "preview", "status": "ready", "proposalId": "p1"}),
    ));
    assert_eq!(texts(&state), ["Exported cut.mov", "Preview ready"]);
    assert_eq!(state.notices[1].hold, Some(PREVIEW_NOTICE_HOLD));
    // The same words again stay where they are, held afresh.
    let id = state.notices[1].id;
    state.apply(&event(json!({"type": "preview", "status": "ready"})));
    assert_eq!(texts(&state), ["Exported cut.mov", "Preview ready"]);
    assert_eq!(state.notices[1].id, id);
    state.apply(&event(
        json!({"type": "preview", "status": "live", "message": "masks are soft"}),
    ));
    assert_eq!(
        texts(&state),
        ["Exported cut.mov", "Live preview: masks are soft"]
    );
    // It keeps its own 4 s, past the window's hold.
    state.expire_notices(Duration::ZERO, Duration::ZERO);
    assert_eq!(texts(&state), ["Live preview: masks are soft"]);
    // Gone, what the preview says next is said anew.
    state.notices.clear();
    state.apply(&event(json!({"type": "preview", "status": "ready"})));
    assert_eq!(texts(&state), ["Preview ready"]);
}

#[test]
fn a_failed_render_is_said_once_in_the_bridge_s_words() {
    let mut state = EditorState::default();
    // As the bridge says a render that failed: failed, then why a render
    // is still needed.
    state.apply(&event(
        json!({"type": "preview", "status": "failed", "proposalId": "p1"}),
    ));
    state.apply(&event(
        json!({"type": "preview", "status": "required", "proposalId": "p1",
        "message": "Preview render failed. Retry after fixing the reported error."}),
    ));
    let lines: Vec<(String, bool, Option<Duration>)> = state
        .notices
        .iter()
        .map(|notice| (notice.text.clone(), notice.error, notice.hold))
        .collect();
    assert_eq!(
        lines,
        [(
            "Preview render failed. Retry after fixing the reported error.".to_owned(),
            true,
            Some(kerf_editor::theme::motion::NOTICE_HOLD)
        )]
    );
    assert_eq!(
        state.preview.reason().as_deref(),
        Some("Preview render failed. Retry after fixing the reported error.")
    );
    // Rendered again: ready in its place.
    state.apply(&event(
        json!({"type": "preview", "status": "running", "proposalId": "p1"}),
    ));
    state.apply(&event(
        json!({"type": "preview", "status": "ready", "proposalId": "p1"}),
    ));
    assert_eq!(state.notices.len(), 1);
    assert_eq!(state.notices[0].text, "Preview ready");
    assert!(!state.notices[0].error);
    assert!(state.preview_line().is_some());
}

#[test]
fn a_card_s_preview_renders_only_what_the_proposal_changes() {
    let snapshot = Snapshot::from_value(&snapshot());
    // p-far trims a at 3.52 s: from a's start to its old end.
    assert_eq!(
        preview_request(&snapshot, Some("p-far")),
        command(json!({"type": "preview", "proposalId": "p-far", "start": 0.0, "duration": 4.0}))
    );
    // An earlier version renders what it changes itself.
    assert_eq!(
        preview_request(&snapshot, Some("p-near-1")),
        command(
            json!({"type": "preview", "proposalId": "p-near-1", "start": 20.0, "duration": 4.5})
        )
    );
    assert_eq!(
        preview_request(&snapshot, Some("p-near-2")),
        command(
            json!({"type": "preview", "proposalId": "p-near-2", "start": 20.0, "duration": 5.0})
        )
    );
    // One with no timeline known renders whole; so does the program.
    assert_eq!(
        preview_request(&snapshot, Some("p-done")),
        command(json!({"type": "preview", "proposalId": "p-done"}))
    );
    assert_eq!(
        preview_request(&snapshot, None),
        command(json!({"type": "preview"}))
    );
    assert_eq!(
        preview_request(&snapshot, Some("unknown")),
        command(json!({"type": "preview", "proposalId": "unknown"}))
    );
}

#[test]
fn a_preview_s_stretch_is_held_to_a_frame_and_to_two_minutes() {
    let program = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 300.0, 4.0),
        clip("c", 400.0, 4.0),
    ]));
    // Moved far: from where it was to where it goes, at most 120 s.
    let moved = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 10.0, 4.0),
        clip("c", 400.0, 4.0),
    ]));
    assert_eq!(
        preview_span(&program, &moved),
        Some((10.0, REEL_MOST_RENDERED))
    );
    let program = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 300.0, 4.0),
    ]));
    // A clip added with no length still renders a frame (25 fps).
    let added = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 300.0, 4.0),
        clip("c", 7.0, 0.0),
    ]));
    assert_eq!(preview_span(&program, &added), Some((7.0, 1.0 / 25.0)));
    assert_eq!(preview_span(&program, &program), None);
}

proptest! {
    /// Whatever a proposal moves, its preview starts where the earliest
    /// change does, never before the program, lasts a frame to 120 s, and
    /// ends where the change does or, sooner, where the proposal does.
    #[test]
    fn a_preview_s_stretch_covers_the_earliest_change(
        from in 0.0f64..400.0,
        to in 0.0f64..400.0,
        length in 0.05f64..60.0,
    ) {
        let program = Timeline::from_value(&timeline_value(&[clip("a", from, length)]));
        let proposed = Timeline::from_value(&timeline_value(&[clip("a", to, length)]));
        let span = preview_span(&program, &proposed);
        if (from - to).abs() <= 1e-4 {
            prop_assert_eq!(span, None);
        } else {
            let (start, duration) = span.expect("a stretch");
            prop_assert!((start - from.min(to)).abs() < 1e-9);
            prop_assert!(start >= 0.0);
            prop_assert!((1.0 / 25.0 - 1e-12..=REEL_MOST_RENDERED).contains(&duration));
            // Moved later, it renders from where it was to its new end;
            // moved earlier, the proposal ends with it.
            let end = to + length;
            prop_assert!((duration - (end - start).min(REEL_MOST_RENDERED)).abs() < 1e-9);
            prop_assert!(start + duration <= end + 1e-9);
        }
    }
}

#[test]
fn an_archived_conversation_stays_open() {
    let mut state = EditorState::default();
    state.choose(Some("c1".into()));
    state.apply(&event(
        json!({"type": "conversation", "id": "c1", "title": "Trim", "archived": true, "items": []}),
    ));
    state.apply(&event(json!({"type": "conversations", "items": [
        {"id": "c1", "title": "Trim", "archived": true, "count": 2},
        {"id": "c2", "title": "Other", "archived": false, "count": 1}
    ]})));
    assert_eq!(state.chosen.as_deref(), Some("c1"));
    assert!(state
        .open_conversation
        .as_ref()
        .is_some_and(|open| open.archived));
    assert!(state.conversations[0].archived);
    // Deleted, it goes.
    state.apply(&event(json!({"type": "conversations", "items": [
        {"id": "c2", "title": "Other", "archived": false, "count": 1}
    ]})));
    assert_eq!(state.chosen, None);
}

// ------------------------------------------------- wave five review fixes

/// Where a timeline's last clip ends: how long kerfd lets a render of it run.
fn last_end(timeline: &Timeline) -> f64 {
    timeline
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.end())
        .fold(0.0, f64::max)
}

#[test]
fn a_preview_of_a_shorter_programme_ends_where_the_proposal_does() {
    // Tighten the pauses: b and c come earlier, the programme ends at 12.
    let program = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 5.0, 4.0),
        clip("c", 10.0, 4.0),
    ]));
    let tightened = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 4.0, 4.0),
        clip("c", 8.0, 4.0),
    ]));
    assert_eq!(preview_span(&program, &tightened), Some((4.0, 8.0)));
    // The last clip trimmed: from where it starts to its new end.
    let trimmed = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 5.0, 4.0),
        clip("c", 10.0, 1.5),
    ]));
    assert_eq!(preview_span(&program, &trimmed), Some((10.0, 1.5)));
    // The last clip taken out: nothing it changes lies within the
    // proposal, which renders whole.
    let cut = Timeline::from_value(&timeline_value(&[clip("a", 0.0, 4.0), clip("b", 5.0, 4.0)]));
    assert_eq!(preview_span(&program, &cut), None);
    // The card's request says the same.
    let mut value = snapshot();
    value["timeline"] = timeline_value(&[clip("a", 0.0, 4.0), clip("b", 4.0, 3.0)]);
    value["proposalTimelines"]["p-far"] = timeline_value(&[clip("a", 0.0, 4.0)]);
    value["proposalTimelines"]["p-near-2"] =
        timeline_value(&[clip("a", 0.0, 3.0), clip("b", 3.0, 3.0)]);
    let snapshot = Snapshot::from_value(&value);
    assert_eq!(
        preview_request(&snapshot, Some("p-far")),
        command(json!({"type": "preview", "proposalId": "p-far"}))
    );
    assert_eq!(
        preview_request(&snapshot, Some("p-near-2")),
        command(
            json!({"type": "preview", "proposalId": "p-near-2", "start": 0.0, "duration": 6.0})
        )
    );
}

/// What a proposal does to one clip of a cut.
#[derive(Clone, Debug)]
enum Edit {
    Keep,
    Remove,
    Trim(f64),
    Shift(f64),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        Just(Edit::Keep),
        Just(Edit::Remove),
        (0.04f64..1.0).prop_map(Edit::Trim),
        (-30.0f64..30.0).prop_map(Edit::Shift),
    ]
}

proptest! {
    /// Whatever a proposal takes out, trims or moves, kerfd takes the
    /// stretch its preview asks for: it starts at the program's start or
    /// later, lasts a frame to 120 s, and ends within the proposal; and it
    /// starts where the earliest change does whenever it is asked for.
    #[test]
    fn kerfd_takes_every_preview_stretch_a_proposal_asks_for(
        cut in proptest::collection::vec((0.0f64..20.0, 0.5f64..40.0, edit()), 1..6),
    ) {
        let mut at = 0.0;
        let mut before = Vec::new();
        let mut after = Vec::new();
        for (index, (gap, length, edit)) in cut.iter().enumerate() {
            let id = format!("c{index}");
            let start = at + gap;
            at = start + length;
            before.push(clip(&id, start, *length));
            match edit {
                Edit::Keep => after.push(clip(&id, start, *length)),
                Edit::Remove => {}
                Edit::Trim(part) => after.push(clip(&id, start, length * part)),
                Edit::Shift(by) => after.push(clip(&id, (start + by).max(0.0), *length)),
            }
        }
        let program = Timeline::from_value(&timeline_value(&before));
        let proposed = Timeline::from_value(&timeline_value(&after));
        let frame = 1.0 / 25.0;
        let changes = clip_changes(&program, &proposed);
        if let Some((start, duration)) = preview_span(&program, &proposed) {
            prop_assert!(start >= 0.0);
            prop_assert!((frame - 1e-12..=REEL_MOST_RENDERED).contains(&duration));
            prop_assert!(start + duration <= last_end(&proposed) + 1e-9);
            let earliest = changes
                .iter()
                .map(|change| change.span().0)
                .fold(f64::INFINITY, f64::min);
            prop_assert!((start - earliest.max(0.0)).abs() < 1e-9);
        } else {
            // Nothing changed, or nothing of it a frame inside the proposal.
            let earliest = changes
                .iter()
                .map(|change| change.span().0.max(0.0))
                .fold(f64::INFINITY, f64::min);
            prop_assert!(changes.is_empty() || last_end(&proposed) - earliest < frame);
        }
    }
}

#[test]
fn an_error_notice_stays_longer_than_the_rest() {
    assert!(
        kerf_editor::theme::motion::NOTICE_ERROR_HOLD > kerf_editor::theme::motion::NOTICE_HOLD
    );
    let mut state = EditorState::default();
    state.notify("Rendered master", false);
    state.notify("The project was not pushed", true);
    assert_eq!(state.notices.len(), 2);
    // Past a notice's hold and not an error's: the error stays.
    assert!(state.expire_notices(Duration::ZERO, Duration::from_secs(3600)));
    let left: Vec<&str> = state
        .notices
        .iter()
        .map(|notice| notice.text.as_str())
        .collect();
    assert_eq!(left, ["The project was not pushed"]);
    assert!(!state.expire_notices(Duration::ZERO, Duration::from_secs(3600)));
    assert!(state.expire_notices(Duration::ZERO, Duration::ZERO));
    assert!(state.notices.is_empty());
}

// ---------------------------------------------------------------- the store

fn notices(state: &EditorState) -> Vec<(String, bool)> {
    state
        .notices
        .iter()
        .map(|notice| (notice.text.clone(), notice.error))
        .collect()
}

#[test]
fn the_store_routes_preset_answers() {
    let mut state = EditorState::default();
    let preset = json!({"name": "Intro", "slots": []});
    assert_eq!(
        state.apply(&event(
            json!({"type": "preset_inspect", "path": "/p/a.kerfpreset", "preset": preset})
        )),
        Change::Other
    );
    assert_eq!(
        state.inspected_preset,
        Some((
            "/p/a.kerfpreset".to_owned(),
            PresetInspection {
                name: "Intro".into(),
                ..PresetInspection::default()
            }
        ))
    );
    assert_eq!(
        state.apply(&event(
            json!({"type": "preset_applied", "path": "/p/a.kerfpreset", "proposal": "p1"})
        )),
        Change::Notice
    );
    state.saving_preset = Some("Intro".into());
    assert_eq!(
        state.apply(&event(
            json!({"type": "preset_saved", "path": "presets/Intro.kerfpreset"})
        )),
        Change::Notice
    );
    assert_eq!(state.saving_preset, None);
    assert_eq!(
        notices(&state),
        [
            ("Preset proposal ready for review".to_owned(), false),
            ("Saved preset presets/Intro.kerfpreset".to_owned(), false),
        ]
    );
}

#[test]
fn saving_over_a_preset_is_asked_about_not_refused() {
    let refusal = json!({"type": "error",
        "message": format!("presets/Intro.kerfpreset{EXISTS}")});
    let mut saving = EditorState::default();
    saving.saving_preset = Some("Intro".into());
    assert_eq!(saving.apply(&event(refusal.clone())), Change::Other);
    assert!(
        saving.notices.is_empty(),
        "the question says it, not a notice"
    );
    assert_eq!(
        saving.preset_exists.as_deref(),
        Some("presets/Intro.kerfpreset")
    );
    // Not saving, the same words are an error like any other.
    let mut idle = EditorState::default();
    assert_eq!(idle.apply(&event(refusal.clone())), Change::Notice);
    assert_eq!(notices(&idle).len(), 1);
    assert_eq!(idle.preset_exists, None);
}

#[test]
fn an_exported_preset_is_written_where_the_bridge_says() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("editing.kerfpreset");
    let mut state = EditorState::default();
    let change = state.apply(&event(json!({"type": "preset_export", "path": path,
        "document": "kerf 1\nproject name=\"Harbour\"\n"})));
    assert_eq!(change, Change::Notice);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "kerf 1\nproject name=\"Harbour\"\n"
    );
    assert_eq!(
        notices(&state),
        [("Preset exported: editing.kerfpreset".to_owned(), false)]
    );
    let nowhere = root.path().join("missing/editing.kerfpreset");
    state.apply(&event(
        json!({"type": "preset_export", "path": nowhere, "document": "x"}),
    ));
    let (text, error) = notices(&state).pop().unwrap();
    assert!(text.starts_with("Could not export preset: "), "{text}");
    assert!(error);
}

#[test]
fn project_warnings_wait_for_the_project_not_the_connecting() {
    let mut state = EditorState::default();
    assert!(!state.opened_or_failed());
    // The bridge's first word is that it is connecting: not yet.
    assert_eq!(
        state.apply(&event(
            json!({"type": "connection", "status": "connecting"})
        )),
        Change::Connection
    );
    assert!(!state.opened_or_failed());
    // The project's first snapshot: up.
    assert_eq!(
        state.apply(&event(json!({"type": "state", "snapshot": {}}))),
        Change::Snapshot
    );
    assert!(state.opened_or_failed());
    state.apply(&event(json!({"type": "connection", "status": "ready"})));
    assert!(state.opened_or_failed());
    // kerfd could not be reached: said then, since nothing else will come.
    let mut failed = EditorState::default();
    failed.apply(&event(
        json!({"type": "connection", "status": "connecting"}),
    ));
    failed.apply(&event(
        json!({"type": "connection", "status": "disconnected",
        "message": "cannot start kerfd"}),
    ));
    assert!(failed.opened_or_failed());
}

#[test]
fn a_loop_runs_from_the_in_mark_to_the_out_mark_held_to_the_program() {
    let span = LoopSpan::new(Some(2.0), Some(9.0), 6.0, 24.0);
    assert_eq!(span.start, 2.0);
    // An out mark past a program that got shorter ends at its end.
    assert_eq!(span.end, 6.0);
    let span = LoopSpan::new(None, None, 8.0, 25.0);
    assert_eq!((span.start, span.end), (0.0, 8.0));
    assert!(span.loops());
    // Nothing to play over: no loop.
    assert!(!LoopSpan::new(None, None, 0.0, 24.0).loops());
}

#[test]
fn playback_that_stops_at_the_end_goes_round_and_one_that_stops_short_does_not() {
    let span = LoopSpan::new(Some(1.0), None, 8.0, 24.0);
    // kerfd stops by itself at the program's end, its last clock up to
    // 50 ms short of it.
    assert_eq!(span.stopped(8.0), Some(1.0));
    assert_eq!(span.stopped(8.0 - 0.05), Some(1.0));
    // Paused by a person well before it: no loop.
    assert_eq!(span.stopped(4.0), None);
}

#[test]
fn a_clock_past_the_out_mark_goes_back_once_until_it_plays_short_of_it_again() {
    let span = LoopSpan::new(Some(1.0), Some(4.0), 8.0, 24.0);
    let mut wrapped = false;
    assert_eq!(span.clock(2.0, &mut wrapped), None);
    assert_eq!(span.clock(4.0, &mut wrapped), Some(1.0));
    // Clocks still on their way from before that seek do not send it back.
    assert_eq!(span.clock(4.01, &mut wrapped), None);
    // Once it plays short of the end, the next pass goes round again.
    assert_eq!(span.clock(1.5, &mut wrapped), None);
    assert_eq!(span.clock(4.0, &mut wrapped), Some(1.0));
}

#[test]
fn a_stop_just_after_the_clock_went_round_plays_on_from_the_start() {
    // kerfd's last clock at the end comes just before its stop: the clock
    // has sent the playhead back already when the stop is heard.
    let span = LoopSpan::new(Some(1.0), None, 8.0, 24.0);
    assert!(span.just_wrapped(1.0));
    assert!(span.just_wrapped(1.05));
    // Well after the start, or before it: not Loop Playback's stop.
    assert!(!span.just_wrapped(3.0));
    assert!(!span.just_wrapped(0.5));
    // No loop at all: never.
    assert!(!LoopSpan::new(None, None, 0.0, 24.0).just_wrapped(0.0));
}

proptest! {
    #[test]
    fn a_stop_loops_only_within_the_slack_of_the_loops_end(
        length in 1.0f64..600.0,
        fps in prop::sample::select(vec![23.976f64, 24.0, 25.0, 30.0, 50.0, 60.0]),
        mark_in in proptest::option::of(0.0f64..0.5),
        at in 0.0f64..1.0,
    ) {
        let span = LoopSpan::new(mark_in.map(|share| share * length), None, length, fps);
        let position = at * length;
        let slack = (2.0 / fps).max(LOOP_SLACK);
        let looped = span.stopped(position);
        if position >= span.end - slack && span.loops() {
            prop_assert_eq!(looped, Some(span.start));
        } else {
            prop_assert_eq!(looped, None);
        }
        // Wherever it goes back to is inside the program.
        if let Some(start) = looped {
            prop_assert!((0.0..=length).contains(&start));
        }
    }
}

#[test]
fn the_pointers_seek_is_rough_unless_just_ahead_of_the_frame_shown() {
    // Nothing shown yet: rough.
    assert!(scrubs_from(None, 3.0, 24.0));
    // Just ahead: decoding on to it exactly costs little.
    assert!(!scrubs_from(Some(3.0), 3.0 + SCRUB_AHEAD / 2.0, 24.0));
    // Far ahead or behind: rough at once.
    assert!(scrubs_from(Some(3.0), 3.0 + SCRUB_AHEAD + 0.1, 24.0));
    assert!(scrubs_from(Some(3.0), 2.5, 24.0));
    // The settle, 50 ms, is kept in the theme.
    assert_eq!(SCRUB_SETTLE, motion::SCRUB_SETTLE);
    assert_eq!(SCRUB_SETTLE, Duration::from_millis(50));
}

proptest! {
    #[test]
    fn a_late_frame_never_takes_the_place_of_one_nearer_the_pointer(
        shown in 0.0f64..100.0,
        frame in 0.0f64..100.0,
        target in 0.0f64..100.0,
        fps in 1.0f64..120.0,
    ) {
        let takes = nearer_frame(Some(shown), frame, target, fps);
        let slack = 0.5 / fps;
        prop_assert_eq!(takes, (frame - target).abs() <= (shown - target).abs() + slack);
        // The frame at the pointer always shows; with nothing shown, any does.
        prop_assert!(nearer_frame(Some(shown), target, target, fps));
        prop_assert!(nearer_frame(None, frame, target, fps));
    }
}

#[test]
fn while_kerfd_is_away_edits_and_playback_wait_and_reconnect_goes() {
    for waits in [
        CommandName::Edit,
        CommandName::Seek,
        CommandName::Play,
        CommandName::Pause,
        CommandName::Preview,
        CommandName::Export,
        CommandName::Undo,
        CommandName::RenderQueue,
    ] {
        assert!(!sent_unconnected(&waits), "{waits} should wait");
    }
    for goes in [
        CommandName::Reconnect,
        CommandName::Abort,
        CommandName::Conversations,
        CommandName::Agent,
        CommandName::PreviewStop,
        CommandName::CollabSync,
        CommandName::CloudRefresh,
    ] {
        assert!(sent_unconnected(&goes), "{goes} should go");
    }
    // Something with no type is never sent unconnected.
    assert!(!sent_unconnected(&CommandName::from_name("")));
}

// ------------------------------------------------------------ the picture

#[test]
fn the_frame_shown_stands_in_after_an_edit_unless_its_shape_changes_or_nothing_plays() {
    // The program's frame, at the sequence's size or scaled to the window.
    assert!(stands_in((1920, 1080), (1920, 1080), 7.0));
    assert!(stands_in((960, 540), (1920, 1080), 7.0));
    // Another shape would stretch it: it goes.
    assert!(!stands_in((1920, 1080), (1080, 1080), 7.0));
    assert!(!stands_in((1280, 720), (1440, 1080), 7.0));
    // Nothing to play: no first frame comes to take its place.
    assert!(!stands_in((1920, 1080), (1920, 1080), 0.0));
    assert!(!stands_in((1920, 1080), (1920, 1080), -1.0));
    // No frame, no stand-in.
    assert!(!stands_in((0, 0), (1920, 1080), 7.0));
}

#[test]
fn the_canvas_is_the_sequence_s_size_else_its_largest_picture() {
    let timeline = |value: Value| Timeline::from_value(&value);
    assert_eq!(
        timeline(json!({"width": 1080, "height": 1920})).canvas(),
        (1080, 1920)
    );
    // No size of its own: the largest picture among its media, sound left out.
    let bare = timeline(json!({"sources": [
        {"id": "a", "width": 1280, "height": 720},
        {"id": "s", "hasAudio": true},
        {"id": "b", "width": 3840, "height": 1600},
        {"id": "c", "width": 1920, "height": 1080},
    ]}));
    assert_eq!(bare.canvas(), (3840, 1600));
    // Nothing to go by: 1920 × 1080.
    assert_eq!(timeline(json!({})).canvas(), (1920, 1080));
}

proptest! {
    /// A frame stands in only when there is something to play and it has
    /// the canvas's shape, whatever its size: any frame scaled from the
    /// canvas does.
    #[test]
    fn a_stand_in_has_the_canvas_s_shape(
        width in 2u32..8000, height in 2u32..8000,
        canvas_w in 2u32..8000, canvas_h in 2u32..8000,
        scale in 1u32..4, duration in -5.0f64..60.0,
    ) {
        let kept = stands_in((width, height), (canvas_w, canvas_h), duration);
        if kept {
            prop_assert!(duration > 0.0);
            let shown = f64::from(width) / f64::from(height);
            let canvas = f64::from(canvas_w) / f64::from(canvas_h);
            prop_assert!((shown - canvas).abs() < 0.01);
        }
        prop_assert_eq!(
            stands_in((canvas_w * scale, canvas_h * scale), (canvas_w, canvas_h), duration),
            duration > 0.0
        );
    }
}

// ------------------------------------------------------------ makings

#[test]
fn each_take_of_a_making_starts_its_clock_afresh_with_no_line_yet() {
    let mut state = EditorState::default();
    let making = |take: u32, line: &str| json!({"type": "media_making", "media": "m4", "take": take, "line": line});
    state.apply(&event(making(1, "")));
    let first = state.makings["m4"].since;
    std::thread::sleep(Duration::from_millis(5));
    state.apply(&event(making(1, "rendering the guide, 40%")));
    assert_eq!(
        state.makings["m4"].since, first,
        "the same take keeps its clock"
    );
    assert_eq!(state.makings["m4"].line, "rendering the guide, 40%");
    // The second of several takes begins: a clock of its own, no line yet.
    state.apply(&event(making(2, "")));
    let second = &state.makings["m4"];
    assert_eq!(second.take, 2);
    assert!(second.since > first, "its clock starts as its take does");
    assert_eq!(
        second.line, "",
        "the first take's last line is not left standing"
    );
    assert_eq!(
        kerf_editor::store::making_status("Sea", second),
        "Making take 2 of Sea from its steps…"
    );
    let since = second.since;
    // Each event says the line as it is now, an empty one among them.
    state.apply(&event(making(2, "asked the server")));
    state.apply(&event(making(2, "")));
    assert_eq!(state.makings["m4"].line, "");
    assert_eq!(state.makings["m4"].since, since);
}

// ------------------------------------------------------------ Activity

#[test]
fn a_preset_exported_or_saved_is_kept_in_activity_by_its_path() {
    let mut state = EditorState::default();
    assert!(state.take_logged().is_empty());
    state.apply(&event(
        json!({"type": "preset_saved", "path": "presets/Intro.kerfpreset"}),
    ));
    assert_eq!(
        state.take_logged(),
        vec![("preset", "presets/Intro.kerfpreset".to_owned())]
    );
    assert!(state.take_logged().is_empty(), "taken, it is gone");
    assert_eq!(texts(&state), ["Saved preset presets/Intro.kerfpreset"]);

    // Exported: written by the editor, then kept by its path.
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("Intro.kerfpreset");
    let path = path.to_string_lossy().into_owned();
    state.apply(&event(
        json!({"type": "preset_export", "path": path, "document": "kerf-preset 1\n"}),
    ));
    assert_eq!(state.take_logged(), vec![("preset", path.clone())]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "kerf-preset 1\n");

    // One that could not be written is said, and not kept as done.
    let nowhere = folder.path().join("missing/Intro.kerfpreset");
    state.apply(&event(
        json!({"type": "preset_export", "path": nowhere.to_string_lossy(), "document": "x"}),
    ));
    assert!(state.take_logged().is_empty());
    let last = state.notices.last().unwrap();
    assert!(last.error && last.text.starts_with("Could not export preset"));
}

#[test]
fn a_clip_s_shot_is_the_clip_of_the_first_video_track_where_it_starts() {
    let clip = |id: &str, start: i64| {
        json!({"id": id, "timelineStart": start as f64 / 24.0, "duration": 2.0,
               "startFrame": start, "durationFrames": 48, "enabled": true})
    };
    let track = |id: &str, kind: &str, clips: Vec<Value>| json!({"id": id, "type": kind, "name": id, "enabled": true, "clips": clips});
    let timeline = Timeline::from_value(&json!({"name": "Cut", "fps": 24.0, "duration": 8.0,
    "tracks": [
        track("V1", "video", vec![clip("c1", 0), clip("c2", 96)]),
        track("V2", "video", vec![clip("t1", 12)]),
        track("A1", "audio", vec![clip("m1", 60)]),
    ]}));
    // Over the first shot, and in the gap after it, where the next is its shot.
    assert_eq!(shot_of(&timeline, "V2", 12).as_deref(), Some("c1"));
    assert_eq!(shot_of(&timeline, "A1", 60).as_deref(), Some("c2"));
    // After the last shot, the last; and a shot has none of its own.
    assert_eq!(shot_of(&timeline, "V2", 400).as_deref(), Some("c2"));
    assert_eq!(shot_of(&timeline, "V1", 12), None);
    // A sequence of sound alone has no shots.
    let sound = Timeline::from_value(&json!({"name": "Cut", "fps": 24.0, "duration": 8.0,
        "tracks": [track("A1", "audio", vec![clip("m1", 0)])]}));
    assert_eq!(shot_of(&sound, "A1", 0), None);
}

/// The registry's say on Render Preview and Play with the state as it is,
/// told as the session tells it (`Session::command_context`).
fn reasons(state: &EditorState) -> (Value, Value) {
    let states = kerf_bridge::commands_states(&json!({
        "connected": state.connection == kerf_editor::store::Connection::Connected,
        "previewRequired": state.play_waits(),
        "previewRenderable": state.preview_renderable(),
    }));
    (
        states["sequence.renderPreview"].clone(),
        states["playback.toggle"].clone(),
    )
}

#[test]
fn render_preview_waits_for_a_picture_that_needs_one_or_plays_live() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "connection", "status": "ready"})));
    // Before the bridge has said: nothing to render.
    assert!(!state.preview_renderable());
    assert_eq!(
        reasons(&state).0,
        json!("The timeline plays without a rendered preview")
    );
    for (status, renderable) in [
        ("required", true),
        ("running", false),
        ("ready", false),
        ("live", true),
        ("unavailable", false),
        ("failed", false),
        ("exited", false),
        ("stopped", false),
    ] {
        state.apply(&preview(status));
        assert_eq!(state.preview_renderable(), renderable, "{status}");
        let (render, _) = reasons(&state);
        assert_eq!(render.is_null(), renderable, "{status}: {render}");
    }
    // While one renders, Kerf's words say it cannot be rendered now.
    state.apply(&preview("running"));
    assert_eq!(
        reasons(&state).0,
        json!("A preview cannot be rendered right now")
    );
}

#[test]
fn play_waits_from_a_render_needed_or_running_until_the_picture_plays_again() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "connection", "status": "ready"})));
    state.apply(&event(
        json!({"type": "clock", "position": 0.0, "duration": 7.0}),
    ));
    assert!(state.playable());
    assert!(reasons(&state).1.is_null());
    for (status, waits) in [
        ("required", true),
        // A render failing, stopped or left keeps Play waiting while the
        // picture still needs one.
        ("failed", true),
        ("running", true),
        ("stopped", true),
        ("ready", false),
        ("unavailable", true),
        ("exited", true),
        ("live", false),
        ("running", true),
        ("live", false),
    ] {
        state.apply(&preview(status));
        assert_eq!(state.preview_required, waits, "{status}");
        assert_eq!(state.playable(), !waits, "{status}");
        let (_, play) = reasons(&state);
        assert_eq!(
            play,
            if waits {
                json!("Render a preview to play this timeline")
            } else {
                Value::Null
            },
            "{status}"
        );
    }
    // Nothing to play, or kerfd gone: Play does not run either.
    state.apply(&event(
        json!({"type": "clock", "position": 0.0, "duration": 0.0}),
    ));
    state.snapshot.duration = 0.0;
    assert!(!state.playable());
    state.apply(&event(
        json!({"type": "clock", "position": 0.0, "duration": 7.0}),
    ));
    state.apply(&event(
        json!({"type": "connection", "status": "disconnected"}),
    ));
    assert!(!state.playable());
}

#[test]
fn a_render_that_ends_unshown_leaves_play_and_render_preview_as_they_were() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "connection", "status": "ready"})));
    state.apply(&event(
        json!({"type": "clock", "position": 0.0, "duration": 7.0}),
    ));
    // A program kerfd plays live, rendered and stopped (Esc, the card's
    // Stop: only `stopped` answers), or left before it showed (`exited`
    // alone, no `viewer_reset`): it plays, and renders, as before.
    for end in ["stopped", "exited"] {
        state.apply(&preview("live"));
        state.apply(&preview("running"));
        assert!(!state.playable(), "{end}");
        assert!(!state.preview_renderable(), "{end}");
        state.apply(&preview(end));
        assert!(state.playable(), "{end}");
        assert!(state.preview_renderable(), "{end}");
        assert_eq!(reasons(&state), (Value::Null, Value::Null), "{end}");
    }
    // One that needed a render still does, and can render again.
    state.apply(&preview("required"));
    state.apply(&preview("running"));
    state.apply(&preview("stopped"));
    assert!(!state.playable());
    assert!(state.preview_renderable());
    assert_eq!(
        reasons(&state),
        (Value::Null, json!("Render a preview to play this timeline"))
    );
    // A rendered preview showing, left: the program's own word follows
    // the exit, and decides.
    state.apply(&preview("running"));
    state.apply(&preview("ready"));
    assert!(state.playable());
    state.apply(&preview("exited"));
    assert!(state.playable());
    assert!(!state.preview_renderable());
    state.apply(&event(json!({"type": "viewer_reset"})));
    state.apply(&preview("required"));
    assert!(!state.playable());
    assert!(state.preview_renderable());
    // A render that fails over a live program: the bridge says a render is
    // needed after it, and Render Preview can try again.
    state.apply(&preview("live"));
    state.apply(&preview("running"));
    state.apply(&preview("failed"));
    assert!(state.playable());
    state.apply(&preview("required"));
    assert!(!state.playable());
    assert!(state.preview_renderable());
}

#[test]
fn what_plays_can_always_be_paused_even_as_a_render_runs() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "connection", "status": "ready"})));
    state.apply(&event(
        json!({"type": "clock", "position": 0.0, "duration": 7.0}),
    ));
    state.apply(&preview("live"));
    assert_eq!(state.playback_toggle(), Some(CommandName::Play));
    state.apply(&event(json!({"type": "playback", "playing": true})));
    assert_eq!(state.playback_toggle(), Some(CommandName::Pause));
    // Render Preview while it plays: kerfd plays on, and Pause still goes.
    state.apply(&preview("running"));
    assert!(state.preview_required);
    assert!(!state.play_waits());
    assert_eq!(state.playback_toggle(), Some(CommandName::Pause));
    assert!(reasons(&state).1.is_null());
    // Paused, it does not play again until the render is over.
    state.apply(&event(json!({"type": "playback", "playing": false})));
    assert!(state.play_waits());
    assert_eq!(state.playback_toggle(), None);
    assert_eq!(
        reasons(&state).1,
        json!("Render a preview to play this timeline")
    );
    state.apply(&preview("ready"));
    assert_eq!(state.playback_toggle(), Some(CommandName::Play));
    // Nothing plays and kerfd is gone: nothing to send.
    state.apply(&event(
        json!({"type": "connection", "status": "disconnected"}),
    ));
    assert_eq!(state.playback_toggle(), None);
}

proptest! {
    /// Whatever the preview says, in whatever order: the picture's own
    /// word is the last of required, unavailable, ready and live; Play
    /// waits after required, unavailable and running, not after ready and
    /// live, and goes back to the picture's word after a render stopped,
    /// left or failed. Render Preview runs after required and live, or
    /// after a render ended unshown over a picture that needs one or plays
    /// live. Pause always goes.
    #[test]
    fn play_follows_the_last_status_that_decides(
        statuses in proptest::collection::vec((0usize..9, any::<bool>()), 0..30),
    ) {
        const ALL: [&str; 9] = [
            "required", "unavailable", "running", "ready", "live", "failed", "stopped", "exited", "",
        ];
        let mut state = EditorState::default();
        state.apply(&event(json!({"type": "connection", "status": "ready"})));
        state.apply(&event(json!({"type": "clock", "position": 0.0, "duration": 3.0})));
        let mut picture = "";
        let mut waits = false;
        for (index, playing) in statuses {
            let status = ALL[index];
            state.apply(&preview(status));
            state.apply(&event(json!({"type": "playback", "playing": playing})));
            match status {
                "required" | "unavailable" => {
                    picture = status;
                    waits = true;
                }
                "ready" | "live" => {
                    picture = status;
                    waits = false;
                }
                "running" => waits = true,
                "failed" | "stopped" | "exited" => {
                    waits = matches!(picture, "required" | "unavailable");
                }
                _ => {}
            }
            let said = match status {
                "failed" | "stopped" | "exited" => picture,
                status => status,
            };
            prop_assert_eq!(state.preview_required, waits);
            prop_assert_eq!(state.playable(), !waits);
            prop_assert_eq!(state.play_waits(), waits && !playing);
            prop_assert_eq!(state.preview_renderable(), matches!(said, "required" | "live"));
            let toggle = if playing {
                Some(CommandName::Pause)
            } else if waits {
                None
            } else {
                Some(CommandName::Play)
            };
            prop_assert_eq!(state.playback_toggle(), toggle.clone());
            prop_assert_eq!(reasons(&state).1.is_null(), toggle.is_some());
        }
    }
}

// ------------------------------------------------------------ a still from the bin

fn source_from_json(value: Value) -> Source {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_still_chosen_in_the_bin_is_placed_for_five_seconds() {
    let still = source_from_json(
        json!({"id": "logo", "path": "media/logo.png", "duration": 0.0, "width": 1920, "height": 1080}),
    );
    let video = source_from_json(
        json!({"id": "a", "path": "media/a.mp4", "duration": 6.0, "width": 1280, "height": 720, "hasAudio": true}),
    );
    assert_eq!(STILL_SECONDS, 5.0);
    assert_eq!(still.placed_seconds(), 5.0);
    assert_eq!(video.placed_seconds(), 6.0);
    assert_eq!(
        source_range(&still, 24.0),
        json!({"media": "logo", "inFrame": 0, "durationFrames": 120})
    );
    assert_eq!(source_range(&video, 25.0)["durationFrames"], json!(150));

    // Insert and Overwrite (the keys, the tile's menu, a double-click and
    // Return all run them) can place it, and place it for 5 s.
    let context = json!({
        "connected": true,
        "source": source_range(&still, 24.0),
        "sourceTargets": ["V1"],
        "playheadFrame": 48,
        "videoTracks": 1,
    });
    let states = kerf_bridge::commands_states(&context);
    assert!(
        states["source.insert"].is_null(),
        "{}",
        states["source.insert"]
    );
    assert!(states["source.overwrite"].is_null());
    let run = kerf_bridge::commands_run(&json!({"command": "source.insert", "context": context}));
    let request = &run["request"];
    assert_eq!(request["type"], "edit", "{run}");
    let op = &request["ops"][0];
    assert_eq!(op["op"], "insert");
    assert_eq!(op["media"], "logo");
    assert_eq!(op["frames"], 120);
    assert_eq!(op["at"], 48);
}

fn apply_all(state: &mut EditorState, events: &[Value]) -> Vec<Change> {
    events
        .iter()
        .map(|value| state.apply(&event(value.clone())))
        .collect()
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
    assert_eq!(conversation.items[0].speaker(), Speaker::You);
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
    state.apply(&event(json!({"type": "agent_busy", "busy": false})));
    assert!(!state.work.busy());
    assert!(
        state.agent_clips.is_empty(),
        "Kerf's cursor goes when the turn ends"
    );
}

#[test]
fn a_turn_that_ends_in_a_conversation_not_looked_at_leaves_a_dot() {
    let mut state = EditorState::default();
    state.apply(&event(
        json!({"type": "conversation", "id": "seen", "items": []}),
    ));
    state.apply(&event(json!({"type": "agent_busy", "busy": true})));
    state.work.conversation = Some("elsewhere".into());
    state.apply(&event(json!({"type": "agent_busy", "busy": false})));
    assert_eq!(state.settled, ["elsewhere"]);
    state.looked_at("elsewhere");
    assert!(state.settled.is_empty());
}

#[test]
fn the_connection_follows_what_kerfd_says() {
    let mut state = EditorState::default();
    assert_eq!(state.connection, Connection::Connecting);
    state.apply(&event(json!({"type": "connection", "status": "ready"})));
    assert_eq!(state.connection, Connection::Connected);
    state.apply(&event(
        json!({"type": "connection", "status": "disconnected", "message": "kerfd exited"}),
    ));
    assert_eq!(state.connection, Connection::Failed("kerfd exited".into()));
}

#[test]
fn a_render_job_is_followed_from_queued_to_done() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "export", "status": "queued", "jobId": "j1", "name": "Master", "path": "deliveries/Master.mov", "progress": 0.0, "quality": "master"})));
    state.apply(&event(
        json!({"type": "export", "status": "running", "jobId": "j1", "progress": 0.5}),
    ));
    assert_eq!(state.rendering().as_deref(), Some("Rendering Master 50%"));
    state.apply(&event(json!({"type": "export", "status": "completed", "jobId": "j1", "path": "deliveries/Master.mov", "progress": 1.0})));
    assert_eq!(state.renders.len(), 1);
    assert_eq!(state.renders[0].status.as_str(), "completed");
    assert!(state.rendering().is_none());
    assert_eq!(
        state.notices.last().map(|notice| notice.text.as_str()),
        Some("Exported Master.mov")
    );
    // The queue as kerfd lists it replaces what was followed.
    state.apply(&event(json!({"type": "render_queue", "jobs": [{"jobId": "j2", "name": "Review copy", "status": "queued", "progress": 0.0}]})));
    assert_eq!(state.renders.len(), 1);
    assert!(state.renders[0].is_active());
}

#[test]
fn a_job_kerfd_kept_without_its_quality_is_left_out_of_the_queue() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "render_queue", "jobs": [
        {"jobId": "old", "name": "Before qualities", "quality": null, "codec": null, "status": "completed", "progress": 1.0, "outputPath": "deliveries/Old.mp4"},
        {"jobId": "new", "name": "Master", "quality": "master", "codec": "prores_hq", "status": "completed", "progress": 1.0, "outputPath": "deliveries/Master.mov"},
    ]})));
    let jobs: Vec<&str> = state
        .renders
        .iter()
        .map(|job| job.job_id.as_str())
        .collect();
    assert_eq!(jobs, ["new"]);
}

#[test]
fn a_making_is_shown_until_it_is_made_or_stopped() {
    let mut state = EditorState::default();
    state.apply(&event(
        json!({"type": "media_making", "media": "m4", "take": 2, "line": ""}),
    ));
    state.apply(&event(
        json!({"type": "media_making", "media": "m4", "take": 2, "line": "half way"}),
    ));
    assert_eq!(state.makings["m4"].line, "half way");
    assert!(state.making_label().unwrap().starts_with("Making m4 · "));
    state.apply(&event(
        json!({"type": "media_made", "media": "m4", "take": 2}),
    ));
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
        state.apply(&crate::support::event(event.clone()));
    }
    assert!(state.has_snapshot);
    let conversation = state.open_conversation.unwrap();
    assert_eq!(
        conversation.items.len(),
        1,
        "a turn that does not read is left out, the rest kept"
    );
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
            state.apply(&crate::support::event(event.clone()));
            prop_assert!(state.notices.len() <= 4);
        }
        if !state.work.busy() {
            prop_assert!(state.work.request.is_none());
            prop_assert!(state.agent_clips.is_empty());
        }
    }

    /// A clock, caption or meter reading leaves the state as it is exactly
    /// when folding it in changes nothing it sets; anything else may change it.
    #[test]
    fn a_reading_leaves_the_state_as_it_is_only_when_it_changes_nothing(
        kinds in proptest::collection::vec(0usize..5, 1..40),
        numbers in proptest::collection::vec(prop_oneof![Just(0.0), Just(2.5), -1.0f64..30.0], 40),
        drops in proptest::collection::vec(0u64..2, 40),
    ) {
        let mut state = EditorState::default();
        let readings = |state: &EditorState| {
            (
                state.position.to_bits(),
                state.duration.to_bits(),
                state.dropped,
                state.caption.clone(),
                state.meter.map(|peak| peak.map(f64::to_bits)),
            )
        };
        for (index, kind) in kinds.iter().enumerate() {
            let number = numbers[index % numbers.len()];
            let value = match kind {
                0 => json!({"type": "clock", "position": number, "duration": 20.0, "dropped": drops[index % drops.len()]}),
                1 => json!({"type": "caption", "text": if number > 10.0 { "World" } else { "Hello" }}),
                2 => json!({"type": "meter", "peak": [number, number - 6.0]}),
                3 => json!({"type": "meter", "peak": null}),
                _ => json!({"type": "connection", "status": if number > 15.0 { "disconnected" } else { "ready" }}),
            };
            let event = event(value);
            let before = readings(&state);
            let said = state.leaves_as_is(&event);
            state.apply(&event);
            if *kind == 4 {
                prop_assert!(!said);
            } else {
                prop_assert_eq!(said, before == readings(&state));
            }
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
    state.apply(&event(
        json!({"type": "media_making", "media": "m4", "take": 2, "line": ""}),
    ));
    let change = state.apply(&event(json!({"type": "error", "command": "media_make", "media": "m4", "message": "no model `shot` is set up"})));
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
        state.apply(&event(
            json!({"type": "error", "command": command, "message": "refused"}),
        ));
    }
    assert!(state.notices.is_empty());
    // Anything else is said over the conversation.
    state.apply(&event(
        json!({"type": "error", "command": "export", "message": "The output folder must exist"}),
    ));
    assert_eq!(state.notices.len(), 1);
}

#[test]
fn only_the_chosen_conversation_shows() {
    let mut state = EditorState::default();
    state.choose(Some("mine".into()));
    state.apply(&event(json!({"type": "conversation", "id": "mine", "items": [{"id": "1", "kind": "you", "text": "a"}]})));
    // Another window's turn in another conversation changes nothing here.
    state.apply(&event(json!({"type": "conversation", "id": "theirs", "items": [{"id": "2", "kind": "you", "text": "b"}]})));
    assert_eq!(
        state.open_conversation.as_ref().map(|c| c.id.as_str()),
        Some("mine")
    );
    // A new conversation shows nothing until the request sent in it starts one.
    state.choose(None);
    assert!(state.open_conversation.is_none());
    state.apply(&event(
        json!({"type": "conversation", "id": "theirs", "items": []}),
    ));
    assert!(state.open_conversation.is_none());
    state.apply(&event(json!({"type": "agent_busy", "busy": true})));
    state.apply(&event(json!({"type": "conversation", "id": "started", "items": [{"id": "3", "kind": "you", "text": "c"}]})));
    assert_eq!(state.chosen.as_deref(), Some("started"));
    assert_eq!(
        state.open_conversation.as_ref().map(|c| c.id.as_str()),
        Some("started")
    );
    state.apply(&event(json!({"type": "agent_busy", "busy": false})));
    assert!(
        state.work.conversation.is_none(),
        "a turn that ended is no longer under way anywhere"
    );
    assert!(state.settled.is_empty(), "it ended where you were looking");
}

#[test]
fn a_chosen_conversation_deleted_elsewhere_leaves_a_new_one() {
    let mut state = EditorState::default();
    state.apply(&event(json!({"type": "conversations", "items": [{"id": "a", "title": "A"}, {"id": "b", "title": "B"}]})));
    state.choose(Some("b".into()));
    state.apply(&event(
        json!({"type": "conversation", "id": "b", "items": []}),
    ));
    state.apply(&event(
        json!({"type": "conversations", "items": [{"id": "a", "title": "A"}]}),
    ));
    assert!(state.chosen.is_none());
    assert!(state.open_conversation.is_none());
}

#[test]
fn a_snapshot_keeps_every_open_proposals_timeline_and_its_versions() {
    let cut = json!({"fps": 24.0, "tracks": [{"id": "V1", "type": "video", "clips": [
        {"id": "z", "timelineStart": 0.0, "duration": 1.0, "startFrame": 0, "durationFrames": 24}
    ]}]});
    let snapshot = Snapshot::from_value(&json!({
        "timeline": {"fps": 24.0, "tracks": []},
        "proposals": [{"proposalId": "p2", "status": "pending", "version": 2,
            "versions": [{"version": 1, "proposalId": "p1"}, {"version": 2, "proposalId": "p2"}]}],
        "proposalTimelines": {"p1": cut, "p2": {"fps": 24.0, "tracks": []}}
    }));
    let proposal = &snapshot.proposals[0];
    assert_eq!(proposal.versions.len(), 2);
    // The version the pinned card shows, else its own.
    assert!(snapshot
        .timeline_of(proposal, Some("p1"))
        .unwrap()
        .has_clips());
    assert!(!snapshot.timeline_of(proposal, None).unwrap().has_clips());
    assert!(!snapshot
        .timeline_of(proposal, Some("elsewhere"))
        .unwrap()
        .has_clips());
}

#[test]
fn kerf_names_times_in_seconds_frames_or_timecode() {
    assert_eq!(focus_seconds(&json!(4.5), 24.0, 0.0), Some(4.5));
    assert_eq!(focus_seconds(&json!("4.5s"), 24.0, 0.0), Some(4.5));
    assert_eq!(focus_seconds(&json!("4.5"), 24.0, 0.0), Some(4.5));
    assert_eq!(focus_seconds(&json!("12f"), 24.0, 0.0), Some(0.5));
    assert_eq!(focus_seconds(&json!("00:01:02:12"), 24.0, 0.0), Some(62.5));
    // A timecode counts from the sequence's start.
    assert_eq!(
        focus_seconds(&json!("01:00:02:00"), 25.0, 3600.0),
        Some(2.0)
    );
    for nonsense in [json!("soon"), json!(null), json!("1:2"), json!(-3.0)] {
        assert_eq!(focus_seconds(&nonsense, 24.0, 0.0), None, "{nonsense}");
    }
}

proptest! {
    /// A timecode the editor writes reads back as the frame it names.
    #[test]
    fn a_timecode_written_reads_back(frames in 0i64..400_000, fps in prop::sample::select(vec![24.0, 25.0, 30.0, 60.0])) {
        let seconds = frames as f64 / fps;
        let read = focus_seconds(&json!(timecode(seconds, fps)), fps, 0.0).unwrap();
        prop_assert!((read - seconds).abs() < 0.5 / fps);
    }
}

// ---------------------------------------------------------------- notices

#[test]
fn a_notice_with_a_hold_of_its_own_stays_for_it() {
    // The sync's summary and its pull request's line stay 8 s, longer than a
    // plain notice: there is more in them to read.
    assert_eq!(motion::NOTICE_LONG_HOLD, Duration::from_secs(8));
    assert!(motion::NOTICE_LONG_HOLD > motion::NOTICE_HOLD);
    let mut state = EditorState::default();
    state.notify_held(
        "Synced: 2 commits in, pushed",
        false,
        Some(motion::NOTICE_LONG_HOLD),
    );
    state.notify("Rendered master", false);
    state.notify("The project was not pushed", true);
    // Past the window's holds, an error's too: the one held longer stays.
    assert!(state.expire_notices(Duration::ZERO, Duration::ZERO));
    let left: Vec<&str> = state
        .notices
        .iter()
        .map(|notice| notice.text.as_str())
        .collect();
    assert_eq!(left, ["Synced: 2 commits in, pushed"]);
    // A hold of its own shorter than the window's goes first.
    state.notify_held("Gone at once", false, Some(Duration::ZERO));
    assert!(state.expire_notices(Duration::from_secs(3600), Duration::from_secs(3600)));
    assert!(state
        .notices
        .iter()
        .all(|notice| notice.text != "Gone at once"));
}

#[test]
fn a_notice_without_a_hold_keeps_the_windows() {
    let mut state = EditorState::default();
    state.notify("Rendered master", false);
    state.notify("It failed", true);
    let hold = Duration::from_secs(6);
    let error_hold = Duration::from_secs(12);
    assert_eq!(state.notices[0].hold, None);
    assert_eq!(state.notices[0].held_for(hold, error_hold), hold);
    assert_eq!(state.notices[1].held_for(hold, error_hold), error_hold);
}

#[test]
fn a_cast_saved_is_held_long() {
    let mut state = EditorState::default();
    state.apply(&event(
        json!({"type": "cast", "saved": true, "toMake": [{"media": "shot-2"}]}),
    ));
    let notice = state.notices.last().expect("a notice");
    assert_eq!(notice.text, "Saved the cast; shot-2 wants making again");
    assert_eq!(notice.hold, Some(motion::NOTICE_LONG_HOLD));
}

proptest! {

    #[test]
    fn activity_keeps_the_newest_lines_in_order(count in 0usize..200) {
        let mut log = ActivityLog::default();
        for index in 0..count {
            log.push("export", &format!("line {index}"));
        }
        prop_assert_eq!(log.len(), count.min(ACTIVITY_KEPT));
        let shown: Vec<String> = log.latest(8).map(|(_, text)| text.to_owned()).collect();
        let expected: Vec<String> = (count.saturating_sub(8)..count)
            .map(|index| format!("line {index}"))
            .collect();
        prop_assert_eq!(shown, expected);
    }
}

#[test]
fn activity_says_what_an_export_made_and_keeps_nothing_empty() {
    let mut log = ActivityLog::default();
    log.push("import", "   ");
    assert!(log.is_empty());
    assert_eq!(
        activity_of(&event(
            json!({"type": "export", "status": "completed", "jobId": "j1",
            "path": "/films/Atlas/deliveries/Atlas_master.mov"})
        )),
        Some((
            "export",
            "/films/Atlas/deliveries/Atlas_master.mov".to_owned()
        ))
    );
    assert_eq!(
        activity_of(&event(
            json!({"type": "export", "status": "failed", "message": "disk full"})
        )),
        Some(("export", "Export failed: disk full".to_owned()))
    );
    assert_eq!(
        activity_of(&event(
            json!({"type": "export", "status": "running", "progress": 0.4})
        )),
        None
    );
    assert_eq!(
        activity_of(&event(json!({"type": "clock", "position": 2.0}))),
        None
    );
}
