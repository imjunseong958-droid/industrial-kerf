//! How the editor moves: the one curve, and a value that goes on from
//! where it got to when a new motion takes over.

use kerf_editor::motion::{cubic_bezier, ease, shimmer_at, Tween};
use proptest::prelude::*;
use std::time::{Duration, Instant};

#[test]
fn the_curve_starts_and_ends_where_it_should() {
    assert_eq!(ease(0.0), 0.0);
    assert_eq!(ease(1.0), 1.0);
    // An ease-out: most of the way there early.
    assert!(ease(0.25) > 0.6);
    // A straight line is its own curve.
    assert!((cubic_bezier(0.0, 0.0, 1.0, 1.0, 0.37) - 0.37).abs() < 1e-3);
}

#[test]
fn a_motion_cut_short_goes_on_from_where_it_got_to() {
    let start = Instant::now();
    let mut tween = Tween::at(0.0);
    tween.go_at(100.0, Duration::from_millis(400), start);
    let halfway = start + Duration::from_millis(200);
    let there = tween.value_at(halfway);
    assert!(there > 0.0 && there < 100.0);
    tween.go_at(0.0, Duration::from_millis(400), halfway);
    assert!(
        (tween.value_at(halfway) - there).abs() < 1e-3,
        "no jump when a new motion takes over"
    );
    assert_eq!(tween.value_at(halfway + Duration::from_secs(1)), 0.0);
    assert!(
        !tween.running_at(halfway + Duration::from_secs(1)),
        "every start has its end"
    );
}

#[test]
fn the_light_over_a_title_rests_after_crossing() {
    assert_eq!(shimmer_at(0.0, 0.7), Some(-0.3));
    assert!(shimmer_at(0.69, 0.7).unwrap() > 1.2);
    assert_eq!(shimmer_at(0.8, 0.7), None);
}

proptest! {
    #[test]
    fn the_curve_only_goes_forward(a in 0.0f32..1.0, b in 0.0f32..1.0) {
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        prop_assert!(ease(low) <= ease(high) + 1e-4);
        prop_assert!((0.0..=1.0001).contains(&ease(a)));
    }

    #[test]
    fn a_tween_stays_between_where_it_was_and_where_it_goes(from in -500.0f32..500.0, to in -500.0f32..500.0, ms in 0u64..2000) {
        let start = Instant::now();
        let mut tween = Tween::at(from);
        tween.go_at(to, Duration::from_millis(850), start);
        let value = tween.value_at(start + Duration::from_millis(ms));
        let (low, high) = if from < to { (from, to) } else { (to, from) };
        prop_assert!(value >= low - 0.5 && value <= high + 0.5);
    }
}
