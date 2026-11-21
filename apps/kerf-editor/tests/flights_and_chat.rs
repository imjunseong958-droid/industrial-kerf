//! What travels over the window as a cut is built and its picture comes
//! and goes, and choosing the words of the conversation.

use gpui::{point, px, size, Bounds};
use kerf_editor::chat::{in_sight, over_composer, OVER_COMPOSER, OVER_COMPOSER_RISE};
use kerf_editor::input::Unit;
use kerf_editor::model::Timeline;
use kerf_editor::selectable::{copied, stretch, within, Chosen, Spot};
use kerf_editor::theme::motion as scene;
use kerf_editor::workspace::{
    back_flight_at, clip_flight_at, grow_flight_at, grow_shows, let_go_flight_at, panel_heights,
    picture_in, picture_within, placed_since, FlightKind, PaneGoes, STAND_IN_RELEASE,
    TIMELINE_LEAD,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<gpui::Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
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

    #[test]
    fn the_panel_is_shared_out_whole(
        height in 0.0f32..2000.0,
        width in 1.0f32..3000.0,
        aspect in 0.3f32..3.0,
        picture in 0.0f32..=1.0,
        timeline in 0.0f32..=1.0,
        media in 0.0f32..=1.0,
    ) {
        let (picture_tall, band, lanes) =
            panel_heights(height, width, aspect, (picture, timeline, media));
        prop_assert!(picture_tall >= 0.0 && band >= 0.0 && lanes >= 0.0);
        prop_assert!(picture_tall + band + lanes <= height.max(band) + 0.5);
        if timeline == 0.0 {
            prop_assert_eq!(lanes, 0.0);
        }
    }

    #[test]
    fn a_picture_placed_in_its_slot_keeps_its_shape(
        width in 200.0f32..2000.0, height in 200.0f32..1500.0, aspect in 0.4f32..2.5,
    ) {
        let slot = rect(100.0, 50.0, width, height);
        let placed = picture_in(slot, aspect);
        let shape = f32::from(placed.size.width) / f32::from(placed.size.height);
        prop_assert!((shape - aspect).abs() / aspect < 0.01);
        prop_assert!(placed.left() >= slot.left() && placed.right() <= slot.right() + px(0.5));
        prop_assert!(placed.top() >= slot.top() && placed.bottom() <= slot.bottom());
    }
}

#[test]
fn a_frame_without_a_size_is_placed_as_a_wide_picture() {
    let slot = rect(0.0, 0.0, 1000.0, 700.0);
    assert_eq!(picture_within(slot, 0, 0), picture_in(slot, 16.0 / 9.0));
    assert_eq!(
        picture_within(slot, 1080, 1920),
        picture_in(slot, 1080.0 / 1920.0)
    );
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

#[test]
fn a_clip_taken_out_does_not_fly() {
    let program = cut(&[clip("p1", "a", 0.0, 3.0)], &[]);
    let emptied = cut(&[], &[]);
    assert!(placed_since(&program, &emptied, &HashSet::new())
        .0
        .is_empty());
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

// ------------------------------------------------------------ choosing words

#[test]
fn a_drag_chooses_from_where_it_went_down_across_pieces() {
    let mut chosen = Chosen::pressed("answer", 0, 2..2, Unit::Letter);
    assert!(chosen.is_empty());
    chosen.drag_to(Spot::new(2, 3), (Spot::new(2, 3), Spot::new(2, 3)));
    assert_eq!(chosen.span(), (Spot::new(0, 2), Spot::new(2, 3)));
    assert_eq!(chosen.within(0, 10), Some(2..10));
    assert_eq!(chosen.within(1, 7), Some(0..7));
    assert_eq!(chosen.within(2, 9), Some(0..3));
    assert_eq!(chosen.within(3, 9), None);
    // Dragged back before where it went down, it reaches back from there.
    chosen.drag_to(Spot::new(0, 0), (Spot::new(0, 0), Spot::new(0, 0)));
    assert_eq!(chosen.span(), (Spot::new(0, 0), Spot::new(0, 2)));
}

#[test]
fn a_drag_after_a_double_click_goes_by_whole_words() {
    // "Cut these together": "these" chosen, then the pointer on "Cut".
    let mut chosen = Chosen::pressed("request", 0, 4..9, Unit::Word);
    chosen.drag_to(Spot::new(0, 1), (Spot::new(0, 0), Spot::new(0, 3)));
    assert_eq!(chosen.span(), (Spot::new(0, 0), Spot::new(0, 9)));
    // On to "together": the first word stays whole.
    chosen.drag_to(Spot::new(0, 12), (Spot::new(0, 10), Spot::new(0, 18)));
    assert_eq!(chosen.span(), (Spot::new(0, 4), Spot::new(0, 18)));
}

#[test]
fn what_is_copied_keeps_the_breaks_between_pieces() {
    let pieces = [
        (0, "I cut 3 clips together.", "\n\n"),
        (1, "one", "\n\n"),
        (2, "two", "\n"),
    ];
    assert_eq!(
        copied(&pieces, (Spot::new(0, 6), Spot::new(2, 3))),
        "3 clips together.\n\none\ntwo"
    );
    assert_eq!(copied(&pieces, (Spot::new(1, 1), Spot::new(1, 3))), "ne");
    assert_eq!(copied(&pieces, (Spot::new(0, 3), Spot::new(0, 3))), "");
}

proptest! {
    #[test]
    fn a_span_covers_whole_pieces_between_its_ends(
        lens in prop::collection::vec(0usize..40, 1..6),
        a in (0usize..6, 0usize..50),
        b in (0usize..6, 0usize..50),
    ) {
        let (start, end) = {
            let (a, b) = (Spot::new(a.0, a.1), Spot::new(b.0, b.1));
            if a <= b { (a, b) } else { (b, a) }
        };
        for (piece, len) in lens.iter().enumerate() {
            match within((start, end), piece, *len) {
                Some(range) => {
                    prop_assert!(range.start < range.end && range.end <= *len);
                    if piece > start.piece && piece < end.piece {
                        prop_assert_eq!(range, 0..*len);
                    }
                }
                // A piece between the ends is covered whole, if it has words.
                None if piece > start.piece && piece < end.piece => prop_assert_eq!(*len, 0),
                None => {}
            }
        }
    }

    #[test]
    fn copying_everything_joins_every_piece(words in prop::collection::vec("[a-z ]{0,12}", 1..6)) {
        let pieces: Vec<(usize, &str, &str)> =
            words.iter().enumerate().map(|(at, words)| (at, words.as_str(), "\n")).collect();
        let last = pieces.len() - 1;
        let all = copied(&pieces, (Spot::new(0, 0), Spot::new(last, words[last].len())));
        let expected: Vec<&str> = words.iter().map(String::as_str).filter(|words| !words.is_empty()).collect();
        prop_assert_eq!(all, expected.join("\n"));
    }

    #[test]
    fn a_word_drag_always_keeps_the_first_word(
        first in (0usize..4, 0usize..20, 0usize..10),
        here in (0usize..4, 0usize..20, 0usize..10),
    ) {
        let first = (Spot::new(first.0, first.1), Spot::new(first.0, first.1 + first.2));
        let here = (Spot::new(here.0, here.1), Spot::new(here.0, here.1 + here.2));
        let (anchor, head) = stretch(first, here);
        let (start, end) = if anchor <= head { (anchor, head) } else { (head, anchor) };
        prop_assert!(start <= first.0 && end >= first.1);
        prop_assert!(start <= here.0 || end >= here.1);
    }
}
