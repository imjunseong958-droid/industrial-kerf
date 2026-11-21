//! Playback and the picture as Qt's viewer had them (Loop Playback going
//! round past the program's end, the pointer's rough seeks settling on the
//! exact frame, a caption wrapping on a square picture), what waits while
//! kerfd is away, and the words' menus and the composer's refusal.

use kerf_editor::chat::refusal;
use kerf_editor::input::{edit_rows, EditCan, EditRow};
use kerf_editor::session::{
    nearer_frame, scrubs_from, sent_unconnected, LoopSpan, LOOP_SLACK, SCRUB_AHEAD, SCRUB_SETTLE,
};
use kerf_editor::stage::{wide, WIDE_FROM};
use kerf_editor::theme::motion;
use kerf_editor::timeline::{ZOOM_REPEAT, ZOOM_REPEAT_DELAY};
use proptest::prelude::*;
use serde_json::json;
use std::time::Duration;

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
    // The settle is Qt's 50 ms, kept in the theme.
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
        "edit",
        "seek",
        "play",
        "pause",
        "preview",
        "export",
        "undo",
        "render_queue",
    ] {
        assert!(
            !sent_unconnected(&json!({"type": waits})),
            "{waits} should wait"
        );
    }
    for goes in [
        "reconnect",
        "abort",
        "conversations",
        "agent",
        "preview_stop",
        "collab_sync",
        "cloud_status",
    ] {
        assert!(sent_unconnected(&json!({"type": goes})), "{goes} should go");
    }
    // Something with no type is never sent unconnected.
    assert!(!sent_unconnected(&json!({})));
}

#[test]
fn a_square_or_near_square_picture_is_framed_and_captioned_as_a_phone() {
    assert!(!wide(9.0 / 16.0));
    assert!(!wide(1.0));
    assert!(!wide(1.19));
    assert!(wide(WIDE_FROM));
    assert!(wide(16.0 / 9.0));
    assert_eq!(WIDE_FROM, 1.2);
}

#[test]
fn holding_zoom_repeats_at_qts_pace() {
    assert_eq!(ZOOM_REPEAT_DELAY, Duration::from_millis(300));
    assert_eq!(ZOOM_REPEAT, Duration::from_millis(100));
    assert_eq!(ZOOM_REPEAT, motion::ZOOM_REPEAT);
    assert_eq!(ZOOM_REPEAT_DELAY, motion::ZOOM_REPEAT_DELAY);
}

#[test]
fn a_fields_edit_menu_has_qts_rows_in_qts_order() {
    let rows: Vec<EditRow> = edit_rows(EditCan::default())
        .into_iter()
        .map(|(row, _)| row)
        .collect();
    assert_eq!(
        rows,
        [
            EditRow::Undo,
            EditRow::Redo,
            EditRow::Cut,
            EditRow::Copy,
            EditRow::Paste,
            EditRow::Delete,
            EditRow::SelectAll,
        ]
    );
    let words: Vec<&str> = rows.iter().map(|row| row.words()).collect();
    assert_eq!(
        words,
        [
            "Undo",
            "Redo",
            "Cut",
            "Copy",
            "Paste",
            "Delete",
            "Select All"
        ]
    );
    assert_eq!(EditRow::Copy.keys(), Some("Ctrl+C"));
    assert_eq!(EditRow::Delete.keys(), None);
}

proptest! {
    #[test]
    fn each_edit_row_can_be_chosen_only_when_it_can_act(
        undo in any::<bool>(),
        redo in any::<bool>(),
        chosen in any::<bool>(),
        secret in any::<bool>(),
        paste in any::<bool>(),
        empty in any::<bool>(),
        all_chosen in any::<bool>(),
    ) {
        let can = EditCan { undo, redo, chosen, secret, paste, empty, all_chosen };
        for (row, enabled) in edit_rows(can) {
            let expected = match row {
                EditRow::Undo => undo,
                EditRow::Redo => redo,
                // A secret's words are never cut or copied.
                EditRow::Cut | EditRow::Copy => chosen && !secret,
                EditRow::Paste => paste,
                EditRow::Delete => chosen,
                EditRow::SelectAll => !empty && !all_chosen,
            };
            prop_assert_eq!(enabled, expected, "{:?}", row);
        }
    }
}

#[test]
fn the_composer_says_connect_while_kerfd_is_not_connected() {
    let refused = refusal(true, true, false, None).expect("refused");
    assert_eq!(refused.placeholder, "Connect to ask Kerf");
    assert_eq!(refused.reason, "Connect to start editing");
    // Connected, with Kerf set up: nothing in the way.
    assert!(refusal(true, false, false, None).is_none());
}
