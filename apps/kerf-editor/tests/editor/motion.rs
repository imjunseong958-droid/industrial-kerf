//! Motion: how things move, and Reduce Motion.

use crate::support::rect;
use gpui::{point, px, size, Bounds};
use kerf_editor::chat::{in_sight, over_composer, OVER_COMPOSER, OVER_COMPOSER_RISE};
use kerf_editor::model::{Timeline, TimelineShow};
use kerf_editor::motion::{ease, shimmer_at, Tween};
use kerf_editor::stage::{compare_at, pulse_level};
use kerf_editor::theme::motion as scene;
use kerf_editor::workspace::{
    back_flight_at, clip_flight_at, export_flight_at, grow_flight_at, grow_shows, home_flight_at,
    landing_in, lerp_bounds, let_go_flight_at, picture_within, placed_since, FlightKind, PaneGoes,
    STAND_IN_RELEASE, TIMELINE_LEAD,
};
use kerf_editor::{motion, sidebar, theme};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::{Duration, Instant};

// ------------------------------------------------------------ Reduce Motion

#[test]
fn reduce_motion_follows_the_variable_then_either_switch() {
    assert!(motion::resolve(None, false, true));
    assert!(motion::resolve(None, true, false));
    assert!(!motion::resolve(None, false, false));
    // KERF_REDUCE_MOTION=0 keeps motion whatever the system says.
    assert!(!motion::resolve(Some(false), true, true));
    assert!(motion::resolve(Some(true), false, false));
    assert_eq!(motion::env_override(None), None);
    assert_eq!(motion::env_override(Some("")), None);
    assert_eq!(motion::env_override(Some("0")), Some(false));
    assert_eq!(motion::env_override(Some("1")), Some(true));
}

#[test]
fn the_systems_setting_is_read_again_only_when_asked_and_old() {
    let start = Instant::now();
    // Never read: due.
    assert!(motion::system_read_due(None, start, false));
    // Just read: not again until SYSTEM_POLL has gone.
    assert!(!motion::system_read_due(Some(start), start, false));
    let soon = start + theme::motion::SYSTEM_POLL / 2;
    assert!(!motion::system_read_due(Some(start), soon, false));
    let later = start + theme::motion::SYSTEM_POLL;
    assert!(motion::system_read_due(Some(start), later, false));
    // One read at a time.
    assert!(!motion::system_read_due(Some(start), later, true));
    assert!(!motion::system_read_due(None, start, true));
}

#[test]
fn the_systems_setting_reads_as_it_is_printed() {
    assert!(motion::parse_macos("1\n"));
    assert!(!motion::parse_macos("0\n"));
    assert!(!motion::parse_macos(""));
    assert_eq!(motion::parse_gnome("false\n"), Some(true));
    assert_eq!(motion::parse_gnome("true\n"), Some(false));
    assert_eq!(motion::parse_gnome("No such schema"), None);
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

// ------------------------------------------------------------ flights

#[test]
fn a_clip_flies_whole_then_fades_where_it_landed() {
    assert_eq!(clip_flight_at(Duration::ZERO), Some((0.0, 1.0)));
    let (along, clear) = clip_flight_at(scene::FLIGHT / 2).unwrap();
    assert!(along > 0.5 && along < 1.0, "eased past half way: {along}");
    assert_eq!(clear, 1.0);
    // Landed, it is where it lands, and fades over what it became.
    let (along, clear) = clip_flight_at(scene::FLIGHT + scene::FLIGHT_LAND / 2).unwrap();
    assert_eq!(along, 1.0);
    assert!((clear - 0.5).abs() < 0.01);
    assert_eq!(clip_flight_at(scene::FLIGHT + scene::FLIGHT_LAND), None);
}

#[test]
fn the_pictures_stand_in_holds_where_it_landed_until_it_is_let_go() {
    // Not let go, it grows over the stage's time and stays.
    assert_eq!(grow_flight_at(scene::STAGE, None), Some((1.0, 1.0)));
    assert_eq!(grow_flight_at(scene::STAGE * 20, None), Some((1.0, 1.0)));
    // Let go before it has landed, it lands first, then fades.
    let early = Some(scene::STAGE / 2);
    assert_eq!(grow_flight_at(scene::STAGE, early), Some((1.0, 1.0)));
    let (_, clear) = grow_flight_at(scene::STAGE + STAND_IN_RELEASE / 2, early).unwrap();
    assert!((clear - 0.5).abs() < 0.01);
    assert_eq!(grow_flight_at(scene::STAGE + STAND_IN_RELEASE, early), None);
    // Let go later, it fades from then.
    let late = Some(scene::STAGE * 3);
    assert_eq!(grow_flight_at(scene::STAGE * 3, late), Some((1.0, 1.0)));
    assert_eq!(
        grow_flight_at(scene::STAGE * 3 + STAND_IN_RELEASE, late),
        None
    );
}

#[test]
fn the_picture_goes_back_into_its_card_as_the_panel_goes() {
    assert_eq!(back_flight_at(Duration::ZERO), Some((0.0, 1.0)));
    assert_eq!(back_flight_at(scene::PANEL), Some((1.0, 1.0)));
    assert_eq!(back_flight_at(scene::PANEL + scene::FLIGHT_LAND), None);
}

#[test]
fn the_stage_shows_once_the_picture_has_grown_and_the_timeline_comes_after() {
    assert_eq!(grow_shows(Duration::ZERO), (0.0, 0.0));
    // The timeline starts in a little before the picture has grown.
    let (stage, lanes) = grow_shows(scene::STAGE - TIMELINE_LEAD / 2);
    assert_eq!(stage, 0.0);
    assert!(lanes > 0.0 && lanes < 1.0);
    let (stage, _) = grow_shows(scene::STAGE + scene::FLIGHT_LAND / 2);
    assert!(stage > 0.0 && stage < 1.0);
    let whole = scene::STAGE + scene::FLIGHT_LAND.max(scene::SETTLE);
    assert_eq!(grow_shows(whole), (1.0, 1.0));
}

#[test]
fn what_flies_into_a_pane_is_let_go_as_the_pane_goes() {
    let clip = FlightKind::Clip {
        clip: "p1".into(),
        name: "a".into(),
    };
    let stand_in = FlightKind::Grow {
        frame: None,
        released: None,
    };
    // A clip lands in the timeline, the stand-in in the picture.
    assert!(!clip.flies_on(PaneGoes::Panel));
    assert!(!clip.flies_on(PaneGoes::Timeline));
    assert!(clip.flies_on(PaneGoes::Picture));
    assert!(!stand_in.flies_on(PaneGoes::Panel));
    assert!(!stand_in.flies_on(PaneGoes::Picture));
    assert!(stand_in.flies_on(PaneGoes::Timeline));
    // What lands outside the panel, or was let go already, flies on.
    let back = FlightKind::Back {
        proposal: "pr1".into(),
    };
    let let_go = FlightKind::LetGo {
        was: Box::new(stand_in.clone()),
        clear: 1.0,
    };
    for kind in [FlightKind::Export, FlightKind::Home, back, let_go] {
        for goes in [PaneGoes::Panel, PaneGoes::Picture, PaneGoes::Timeline] {
            assert!(kind.flies_on(goes), "{kind:?} as {goes:?} goes");
        }
    }
}

#[test]
fn what_is_let_go_fades_where_it_is_from_how_clear_it_was() {
    let let_go = FlightKind::LetGo {
        was: Box::new(FlightKind::Grow {
            frame: None,
            released: None,
        }),
        clear: 0.8,
    };
    assert_eq!(let_go.at(Duration::ZERO), Some((1.0, 0.8)));
    let (along, clear) = let_go.at(scene::FLIGHT_LAND / 2).unwrap();
    assert_eq!(along, 1.0);
    assert!((clear - 0.4).abs() < 0.01, "half faded: {clear}");
    assert_eq!(let_go.at(scene::FLIGHT_LAND), None);
    // Gone well before a stand-in would have held.
    assert_eq!(let_go_flight_at(scene::STAGE, 1.0), None);
}

proptest! {
    #[test]
    fn what_is_let_go_only_fades(a in 0u64..1000, b in 0u64..1000, clear in 0.0f32..=1.0) {
        let (early, late) = (ms(a.min(b)), ms(a.max(b)));
        match (let_go_flight_at(early, clear), let_go_flight_at(late, clear)) {
            (Some((_, clear_early)), Some((_, clear_late))) => {
                prop_assert!(clear_early <= clear && clear_late <= clear_early);
            }
            (None, later) => prop_assert!(later.is_none()),
            (Some(_), None) => prop_assert!(late >= scene::FLIGHT_LAND),
        }
    }

    #[test]
    fn flights_only_go_on_and_only_fade(a in 0u64..4000, b in 0u64..4000) {
        let (early, late) = (ms(a.min(b)), ms(a.max(b)));
        for at in [clip_flight_at, back_flight_at] {
            match (at(early), at(late)) {
                (Some((along_early, clear_early)), Some((along_late, clear_late))) => {
                    prop_assert!((0.0..=1.0).contains(&along_early));
                    prop_assert!(along_late >= along_early);
                    prop_assert!(clear_late <= clear_early);
                }
                // Once gone, it stays gone.
                (None, later) => prop_assert!(later.is_none()),
                (Some(_), None) => {}
            }
        }
        let (stage_early, lanes_early) = grow_shows(early);
        let (stage_late, lanes_late) = grow_shows(late);
        prop_assert!(stage_late >= stage_early && lanes_late >= lanes_early);
        // The timeline has begun to come in before the stage shows.
        if stage_early > 0.0 {
            prop_assert!(lanes_early > 0.0);
        }
    }
}

// ------------------------------------------------------------ a cut being built

fn clip(id: &str, source: &str, start: f64, duration: f64) -> Value {
    json!({"id": id, "name": source, "sourceId": source, "timelineStart": start,
           "duration": duration, "startFrame": (start * 24.0) as i64,
           "durationFrames": (duration * 24.0) as i64, "enabled": true})
}

fn cut(picture: &[Value], sound: &[Value]) -> Timeline {
    Timeline::from_value(
        &json!({"name": "Cut", "fps": 24.0, "duration": 30.0, "tracks": [
            {"id": "V1", "type": "video", "name": "V1", "enabled": true, "clips": picture},
            {"id": "A1", "type": "audio", "name": "A1", "enabled": true, "clips": sound},
        ]}),
    )
}

#[test]
fn each_picture_clip_a_cut_places_flies_once_in_the_order_they_play() {
    let program = cut(&[], &[]);
    let first = cut(&[clip("p1", "a", 0.0, 3.0)], &[clip("s1", "a", 0.0, 3.0)]);
    let (placed, seen) = placed_since(&program, &first, &HashSet::new());
    assert_eq!(
        placed
            .iter()
            .map(|placed| (placed.clip.as_str(), placed.source.as_deref()))
            .collect::<Vec<_>>(),
        [("p1", Some("a"))],
        "its sound comes with it, and does not fly"
    );
    // Two more at once: in the order they play, the one already there not again.
    let more = cut(
        &[
            clip("p1", "a", 0.0, 3.0),
            clip("p3", "a", 5.5, 2.0),
            clip("p2", "b", 3.0, 2.5),
        ],
        &[clip("s1", "a", 0.0, 3.0)],
    );
    let (placed, seen) = placed_since(&program, &more, &seen);
    assert_eq!(
        placed
            .iter()
            .map(|placed| placed.clip.as_str())
            .collect::<Vec<_>>(),
        ["p2", "p3"]
    );
    assert_eq!(placed[0].name, "b");
    // Nothing new, nothing flies; one trimmed is placed again where it ends.
    assert!(placed_since(&program, &more, &seen).0.is_empty());
    let trimmed = cut(
        &[
            clip("p1", "a", 0.0, 2.0),
            clip("p3", "a", 5.5, 2.0),
            clip("p2", "b", 3.0, 2.5),
        ],
        &[],
    );
    let (placed, _) = placed_since(&program, &trimmed, &seen);
    assert_eq!(
        placed
            .iter()
            .map(|placed| placed.clip.as_str())
            .collect::<Vec<_>>(),
        ["p1"]
    );
}

// ------------------------------------------------------------ where it leaves from

#[test]
fn what_has_no_tile_leaves_from_over_the_composers_middle() {
    let composer = rect(580.0, 896.0, 720.0, 82.0);
    let from = over_composer(composer);
    assert_eq!(f32::from(from.size.width), OVER_COMPOSER.0);
    assert_eq!(f32::from(from.size.height), OVER_COMPOSER.1);
    assert_eq!(from.center().x, composer.center().x);
    assert_eq!(f32::from(composer.top() - from.top()), OVER_COMPOSER_RISE);
}

#[test]
fn a_card_goes_in_and_out_only_while_it_is_whole_in_sight() {
    let view = rect(300.0, 60.0, 700.0, 800.0);
    assert!(in_sight(rect(320.0, 100.0, 440.0, 248.0), view));
    // Cut at the top or the foot, or scrolled off, it is not.
    assert!(!in_sight(rect(320.0, 20.0, 440.0, 248.0), view));
    assert!(!in_sight(rect(320.0, 700.0, 440.0, 248.0), view));
    assert!(!in_sight(rect(320.0, -400.0, 440.0, 248.0), view));
}

proptest! {
    #[test]
    fn a_flight_lands_inside_the_projects_row(
        x in 0f32..400.0,
        y in 0f32..800.0,
        width in 60f32..600.0,
        height in 20f32..60.0,
    ) {
        let row = Bounds::new(point(px(x), px(y)), size(px(width), px(height)));
        let landing = landing_in(row);
        let (left, top) = kerf_editor::theme::motion::LANDING_INSET;
        prop_assert_eq!(landing.origin, point(px(x + left), px(y + top)));
        prop_assert_eq!(f32::from(landing.size.width), kerf_editor::theme::motion::LANDING_WIDTH);
        prop_assert!((f32::from(landing.size.height) - (height - top * 2.0)).abs() < 1e-3);
        // Inside the row, at its start.
        prop_assert!(landing.origin.x + landing.size.width <= row.origin.x + row.size.width);
        prop_assert!(landing.origin.y + landing.size.height <= row.origin.y + row.size.height + px(1e-3));
    }
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

#[test]
fn the_working_dot_pulses_down_and_back() {
    assert!((pulse_level(0.0) - 1.0).abs() < 1e-6);
    assert!((pulse_level(1.0) - 1.0).abs() < 1e-6);
    assert!((pulse_level(0.5) - 0.35).abs() < 1e-6);
    for step in 0..=24 {
        let level = pulse_level(step as f32 / 24.0);
        assert!((0.35 - 1e-6..=1.0 + 1e-6).contains(&level));
    }
}

#[test]
fn going_home_travels_whole_then_fades_where_it_landed() {
    let (start, clear) = home_flight_at(Duration::ZERO).unwrap();
    assert_eq!((start, clear), (0.0, 1.0));
    let stage = kerf_editor::theme::motion::STAGE;
    let land = kerf_editor::theme::motion::FLIGHT_LAND;
    let (there, clear) = home_flight_at(stage).unwrap();
    assert!((there - 1.0).abs() < 1e-6 && (clear - 1.0).abs() < 1e-6);
    let (_, fading) = home_flight_at(stage + land / 2).unwrap();
    assert!(fading > 0.0 && fading < 1.0);
    assert_eq!(home_flight_at(stage + land), None);
}

#[test]
fn the_compare_line_sweeps_there_and_back_and_holds_still_without_motion() {
    let from = compare_at(Duration::ZERO, false);
    let half = compare_at(Duration::from_millis(3400), false);
    let round = compare_at(Duration::from_millis(6800), false);
    assert!((from - 0.28).abs() < 1e-4);
    assert!((half - 0.78).abs() < 1e-4);
    assert!((round - 0.28).abs() < 1e-4);
    for ms in (0..13_600).step_by(170) {
        let at = compare_at(Duration::from_millis(ms), false);
        assert!((0.28 - 1e-4..=0.78 + 1e-4).contains(&at));
    }
    assert_eq!(compare_at(Duration::from_millis(1234), true), 0.28);
}

#[test]
fn the_light_over_a_working_title_moves_in_steps() {
    let steps = sidebar::TITLE_SHIMMER_STEPS as usize;
    assert_eq!(sidebar::shimmer_step(0.0), 0.0);
    assert_eq!(
        sidebar::shimmer_step(0.001),
        sidebar::shimmer_step(1.0 / steps as f32 - 0.001)
    );
    assert!(sidebar::shimmer_step(1.0) < 1.0);
    let mut seen: Vec<f32> = (0..10_000)
        .map(|at| sidebar::shimmer_step(at as f32 / 10_000.0))
        .collect();
    seen.dedup();
    assert_eq!(seen.len(), steps);
}

#[test]
fn a_finished_exports_frame_crosses_fading_then_fades_where_it_lands() {
    let (along, opacity) = export_flight_at(Duration::ZERO).unwrap();
    assert_eq!(along, 0.0);
    assert!((opacity - 0.9).abs() < 1e-4);
    let (along, opacity) = export_flight_at(kerf_editor::theme::motion::FLIGHT).unwrap();
    assert!((along - 1.0).abs() < 1e-4);
    assert!((opacity - 0.2).abs() < 1e-4);
    let (_, opacity) = export_flight_at(
        kerf_editor::theme::motion::FLIGHT + kerf_editor::theme::motion::FLIGHT_LAND / 2,
    )
    .unwrap();
    assert!((opacity - 0.1).abs() < 1e-3);
    assert!(export_flight_at(
        kerf_editor::theme::motion::FLIGHT + kerf_editor::theme::motion::FLIGHT_LAND
    )
    .is_none());
}

proptest! {
    #[test]
    fn the_frame_never_brightens_and_ends_where_it_lands(ms in 0u64..1200) {
        let now = Duration::from_millis(ms);
        let later = now + Duration::from_millis(16);
        if let (Some((_, now)), Some((_, later))) = (export_flight_at(now), export_flight_at(later)) {
            prop_assert!(later <= now + 1e-6);
        }
        let from = Bounds::new(point(px(600.0), px(200.0)), size(px(640.0), px(360.0)));
        let to = Bounds::new(point(px(12.0), px(118.0)), size(px(44.0), px(26.0)));
        prop_assert_eq!(lerp_bounds(from, to, 0.0), from);
        prop_assert_eq!(lerp_bounds(from, to, 1.0), to);
    }

    #[test]
    fn the_picture_flies_from_inside_the_stage_at_its_own_shape(
        width in 200f32..2000.0,
        height in 200f32..1400.0,
        frame_w in 16u32..4000,
        frame_h in 16u32..4000,
    ) {
        let slot = Bounds::new(point(px(400.0), px(80.0)), size(px(width), px(height)));
        let picture = picture_within(slot, frame_w, frame_h);
        let (w, h) = (f32::from(picture.size.width), f32::from(picture.size.height));
        prop_assert!(f32::from(picture.origin.x) >= 400.0 - 0.01);
        prop_assert!(f32::from(picture.origin.y) >= 80.0 - 0.01);
        prop_assert!(f32::from(picture.origin.x) + w <= 400.0 + width + 0.01);
        prop_assert!(f32::from(picture.origin.y) + h <= 80.0 + height + 0.01);
        let aspect = frame_w as f32 / frame_h as f32;
        prop_assert!(((w / h) - aspect).abs() / aspect < 1e-3);
    }
}
