//! What the picture keeps while the pointer skims it, and which path point
//! stays chosen when a snapshot changes the path.

use kerf_editor::stage::overlay::{follow_point, parse_path, path_of, path_text, Pt};
use kerf_editor::stage::{Heard, Kept};
use proptest::prelude::*;
use serde_json::json;

#[test]
fn clocks_that_answer_the_skim_leave_the_playhead() {
    let mut kept = Kept::new(0.0);
    kept.show(4.75);
    kept.show(5.25);
    assert_eq!(kept.hear(4.75), Heard::Shown);
    assert_eq!(kept.hear(5.25), Heard::Shown);
    assert_eq!(kept.hear(0.0), Heard::Playhead);
    assert_eq!(kept.playhead(), 0.0);
}

#[test]
fn kerfd_answers_in_order_so_older_times_are_let_go() {
    let mut kept = Kept::new(0.0);
    for time in [1.0, 2.0, 3.0] {
        kept.show(time);
    }
    assert_eq!(kept.hear(2.0), Heard::Shown);
    // Sent before the one answered: never answered after it.
    assert_eq!(kept.hear(1.0), Heard::Elsewhere);
    assert_eq!(kept.hear(3.0), Heard::Shown);
}

#[test]
fn a_key_that_moves_the_playhead_is_kept_from_there() {
    let mut kept = Kept::new(0.0);
    kept.show(4.75);
    let frame = 1.0 / 24.0;
    assert!(kept.holds(0.0));
    assert!(!kept.holds(frame));
    kept.moved_to(frame);
    assert!(kept.holds(frame));
    // A late answer to the skim still does not move it.
    assert_eq!(kept.hear(4.75), Heard::Shown);
    assert_eq!(kept.hear(frame), Heard::Playhead);
}

proptest! {
    #[test]
    fn every_time_shown_is_heard_as_shown_in_order(
        playhead in 0.0f64..60.0,
        times in prop::collection::vec(0.0f64..60.0, 1..40),
    ) {
        let times: Vec<f64> = times
            .into_iter()
            .filter(|time| (time - playhead).abs() > 0.01)
            .collect();
        let mut kept = Kept::new(playhead);
        for time in &times {
            kept.show(*time);
        }
        // kerfd answers the newest of a run of seeks at least.
        if let Some(last) = times.last() {
            prop_assert_eq!(kept.hear(*last), Heard::Shown);
        }
        prop_assert_eq!(kept.hear(playhead), Heard::Playhead);
        prop_assert!(kept.holds(playhead));
        prop_assert_eq!(kept.playhead(), playhead);
    }
}

#[test]
fn the_chosen_point_goes_with_its_place() {
    let before: Vec<Pt> = vec![(0.1, 0.1), (0.5, 0.2), (0.9, 0.8)];
    // A point added ahead of it.
    let after: Vec<Pt> = vec![(0.1, 0.1), (0.3, 0.4), (0.5, 0.2), (0.9, 0.8)];
    assert_eq!(follow_point(&before, 1, &after), Some(2));
    // The point itself gone: another one at its index is not it.
    let fewer: Vec<Pt> = vec![(0.1, 0.1), (0.9, 0.8)];
    assert_eq!(follow_point(&before, 1, &fewer), None);
    assert_eq!(follow_point(&before, 2, &fewer), Some(1));
    // An index the path never had.
    assert_eq!(follow_point(&before, 7, &after), None);
}

#[test]
fn a_clips_path_is_read_from_the_program() {
    let timeline = json!({"tracks": [{"type": "video", "clips": [
        {"id": "one", "props": {"path": "0.1,0.2 0.3,0.4"}},
        {"id": "two", "props": {}},
    ]}]});
    assert_eq!(
        path_of(&timeline, "one"),
        Some(vec![(0.1, 0.2), (0.3, 0.4)])
    );
    assert_eq!(path_of(&timeline, "two"), None);
    assert_eq!(path_of(&timeline, "three"), None);
}

proptest! {
    /// kerfd says a path back as `path=` writes it: each point is still
    /// found where it was.
    #[test]
    fn a_point_is_found_again_in_the_path_as_written(
        points in prop::collection::vec((-2.0f64..2.0, -2.0f64..2.0), 2..12),
        pick in 0usize..12,
    ) {
        let index = pick % points.len();
        let distinct = points.iter().enumerate().all(|(i, a)| {
            points
                .iter()
                .skip(i + 1)
                .all(|b| (a.0 - b.0).hypot(a.1 - b.1) > 0.001)
        });
        prop_assume!(distinct);
        let written = parse_path(&path_text(&points)).expect("a path reads back");
        prop_assert_eq!(follow_point(&points, index, &written), Some(index));
    }
}
