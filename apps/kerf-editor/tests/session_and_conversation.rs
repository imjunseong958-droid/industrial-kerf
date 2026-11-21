//! What the session keeps and says, through the public modules: the frame
//! that stands in after playback is opened again, a making's takes, a
//! preset kept in Activity, the preview's hold on Render Preview and Play
//! (and what a render that ends unshown gives back; a pause always goes),
//! a still placed from the bin, the mode said in place of the last, and the
//! words the conversation's controls give on a rest.

use kerf_editor::chat::{say_permissions, MODE_HOLD};
use kerf_editor::model::{Source, Timeline, STILL_SECONDS};
use kerf_editor::prefs::Permissions;
use kerf_editor::session::{source_range, stands_in};
use kerf_editor::store::{EditorState, StatusLine};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::time::Duration;

fn texts(state: &EditorState) -> Vec<String> {
    state
        .notices
        .iter()
        .map(|notice| notice.text.clone())
        .collect()
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
    state.apply(&making(1, ""));
    let first = state.makings["m4"].since;
    std::thread::sleep(Duration::from_millis(5));
    state.apply(&making(1, "rendering the guide, 40%"));
    assert_eq!(
        state.makings["m4"].since, first,
        "the same take keeps its clock"
    );
    assert_eq!(state.makings["m4"].line, "rendering the guide, 40%");
    // The second of several takes begins: a clock of its own, no line yet.
    state.apply(&making(2, ""));
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
    state.apply(&making(2, "asked the server"));
    state.apply(&making(2, ""));
    assert_eq!(state.makings["m4"].line, "");
    assert_eq!(state.makings["m4"].since, since);
}

// ------------------------------------------------------------ Activity

#[test]
fn a_preset_exported_or_saved_is_kept_in_activity_by_its_path() {
    let mut state = EditorState::default();
    assert!(state.take_logged().is_empty());
    state.apply(&json!({"type": "preset_saved", "path": "presets/Intro.kerfpreset"}));
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
    state.apply(&json!({"type": "preset_export", "path": path, "document": "kerf-preset 1\n"}));
    assert_eq!(state.take_logged(), vec![("preset", path.clone())]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "kerf-preset 1\n");

    // One that could not be written is said, and not kept as done.
    let nowhere = folder.path().join("missing/Intro.kerfpreset");
    state.apply(
        &json!({"type": "preset_export", "path": nowhere.to_string_lossy(), "document": "x"}),
    );
    assert!(state.take_logged().is_empty());
    let last = state.notices.last().unwrap();
    assert!(last.error && last.text.starts_with("Could not export preset"));
}

// ------------------------------------------------------------ the preview's hold

fn preview(status: &str) -> Value {
    json!({"type": "preview", "status": status})
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
    state.apply(&json!({"type": "connection", "status": "ready"}));
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
    state.apply(&json!({"type": "connection", "status": "ready"}));
    state.apply(&json!({"type": "clock", "position": 0.0, "duration": 7.0}));
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
    state.apply(&json!({"type": "clock", "position": 0.0, "duration": 0.0}));
    state.snapshot.duration = 0.0;
    assert!(!state.playable());
    state.apply(&json!({"type": "clock", "position": 0.0, "duration": 7.0}));
    state.apply(&json!({"type": "connection", "status": "disconnected"}));
    assert!(!state.playable());
}

#[test]
fn a_render_that_ends_unshown_leaves_play_and_render_preview_as_they_were() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "connection", "status": "ready"}));
    state.apply(&json!({"type": "clock", "position": 0.0, "duration": 7.0}));
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
    state.apply(&json!({"type": "viewer_reset"}));
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
    state.apply(&json!({"type": "connection", "status": "ready"}));
    state.apply(&json!({"type": "clock", "position": 0.0, "duration": 7.0}));
    state.apply(&preview("live"));
    assert_eq!(state.playback_toggle(), Some("play"));
    state.apply(&json!({"type": "playback", "playing": true}));
    assert_eq!(state.playback_toggle(), Some("pause"));
    // Render Preview while it plays: kerfd plays on, and Pause still goes.
    state.apply(&preview("running"));
    assert!(state.preview_required);
    assert!(!state.play_waits());
    assert_eq!(state.playback_toggle(), Some("pause"));
    assert!(reasons(&state).1.is_null());
    // Paused, it does not play again until the render is over.
    state.apply(&json!({"type": "playback", "playing": false}));
    assert!(state.play_waits());
    assert_eq!(state.playback_toggle(), None);
    assert_eq!(
        reasons(&state).1,
        json!("Render a preview to play this timeline")
    );
    state.apply(&preview("ready"));
    assert_eq!(state.playback_toggle(), Some("play"));
    // Nothing plays and kerfd is gone: nothing to send.
    state.apply(&json!({"type": "connection", "status": "disconnected"}));
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
        state.apply(&json!({"type": "connection", "status": "ready"}));
        state.apply(&json!({"type": "clock", "position": 0.0, "duration": 3.0}));
        let mut picture = "";
        let mut waits = false;
        for (index, playing) in statuses {
            let status = ALL[index];
            state.apply(&preview(status));
            state.apply(&json!({"type": "playback", "playing": playing}));
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
                Some("pause")
            } else if waits {
                None
            } else {
                Some("play")
            };
            prop_assert_eq!(state.playback_toggle(), toggle);
            prop_assert_eq!(reasons(&state).1.is_null(), toggle.is_some());
        }
    }
}

// ------------------------------------------------------------ a still from the bin

fn source(value: Value) -> Source {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_still_chosen_in_the_bin_is_placed_for_five_seconds() {
    let still = source(
        json!({"id": "logo", "path": "media/logo.png", "duration": 0.0, "width": 1920, "height": 1080}),
    );
    let video = source(
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

// ------------------------------------------------------------ the mode said

#[test]
fn each_mode_chosen_takes_the_place_of_the_mode_said_last() {
    let mut state = EditorState::default();
    state.notify("Exported cut.mov", false);
    for permissions in [
        Permissions::AcceptEdits,
        Permissions::Auto,
        Permissions::Ask,
        Permissions::AcceptEdits,
    ] {
        say_permissions(&mut state, permissions);
    }
    assert_eq!(
        texts(&state),
        [
            "Exported cut.mov",
            "Accept edits · Edits apply as Kerf makes them"
        ]
    );
    let said = state.line_shown(StatusLine::Permissions).unwrap();
    assert_eq!(said.hold, Some(MODE_HOLD), "Qt's 3 s");
    assert_eq!(MODE_HOLD, Duration::from_millis(3000));
    // What the preview says is a line of its own: neither takes the other's place.
    state.apply(&preview("required"));
    say_permissions(&mut state, Permissions::Auto);
    assert_eq!(
        texts(&state),
        [
            "Exported cut.mov",
            "Rendered preview required",
            "Auto · Edits apply, and Kerf makes its shots"
        ]
    );
    assert_eq!(
        state.preview_line().map(|line| line.text.as_str()),
        Some("Rendered preview required")
    );
    // It keeps its own 3 s past the window's hold; gone, the next is said anew.
    state.expire_notices(Duration::ZERO, Duration::ZERO);
    assert_eq!(
        texts(&state),
        ["Auto · Edits apply, and Kerf makes its shots"]
    );
    state.notices.clear();
    assert!(state.line_shown(StatusLine::Permissions).is_none());
    say_permissions(&mut state, Permissions::Ask);
    assert_eq!(
        texts(&state),
        ["Ask before edits · Each edit waits for you"]
    );
}

// ------------------------------------------------------------ words on a rest
