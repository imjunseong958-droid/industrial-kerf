//! The picture's tools: the masks a tool draws and the edit that adds them,
//! what the chip over the picture and the preview's card say, the check's
//! caption-overlap box, and the viewer's zoom.

use gpui::{point, px, size, Bounds};
use kerf_editor::chat::ClipChange;
use kerf_editor::stage::overlay::{
    dragged, drawn, mask_add, mask_names, masks_of, new_mask_name, overlap_at, Affine, Outline,
    Overlap, DRAW_LEAST,
};
use kerf_editor::stage::{chip, hud, zoom_words, zoomed, ZOOMS};
use proptest::prelude::*;
use serde_json::{json, Value};

/// A 1920 × 1080 picture drawn 960 points wide: half a point a pixel.
fn half() -> Affine {
    Affine::scale(960.0, 540.0)
}

#[test]
fn a_drag_draws_a_rectangle_or_an_ellipse_corner_to_corner() {
    let rect = drawn(false, (0.6, 0.5), (0.2, 0.1), half()).expect("a rectangle");
    assert_eq!(rect, Outline::Rect([0.2, 0.1, 0.6, 0.5]));
    let ellipse = drawn(true, (0.2, 0.1), (0.6, 0.5), half()).expect("an ellipse");
    match ellipse {
        Outline::Ellipse([x, y, rx, ry]) => {
            assert!((x - 0.4).abs() < 1e-12 && (y - 0.3).abs() < 1e-12);
            assert!((rx - 0.2).abs() < 1e-12 && (ry - 0.2).abs() < 1e-12);
        }
        other => panic!("not an ellipse: {other:?}"),
    }
}

#[test]
fn a_click_or_a_line_draws_nothing() {
    // Under four points both across and down: a click, as Qt had it.
    let step = (DRAW_LEAST - 0.5) / 960.0;
    assert_eq!(
        drawn(false, (0.5, 0.5), (0.5 + step, 0.5 + step), half()),
        None
    );
    // Far across but not down at all: no area to hold, which kerfd refuses.
    assert_eq!(drawn(false, (0.1, 0.5), (0.8, 0.5), half()), None);
    assert_eq!(drawn(true, (0.1, 0.5), (0.8, 0.5), half()), None);
    // Far across and a little down: a thin rectangle is still one.
    assert!(drawn(false, (0.1, 0.5), (0.8, 0.502), half()).is_some());
}

#[test]
fn a_new_mask_takes_the_first_name_the_clip_does_not_use() {
    let taken = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(new_mask_name(&[]), "k1");
    assert_eq!(new_mask_name(&taken(&["k1", "k2"])), "k3");
    assert_eq!(new_mask_name(&taken(&["k2", "ring"])), "k1");
    assert_eq!(new_mask_name(&taken(&["k1", "k3"])), "k2");
    // Names come from every mask, whatever its outline.
    let clip = json!({"masks": [
        {"name": "k1", "rect": [0, 0, 1, 1]},
        {"name": "moving", "keys": []},
        {"rect": [0, 0, 1, 1]},
    ]});
    assert_eq!(mask_names(&clip), ["k1", "moving"]);
}

#[test]
fn the_pen_adds_a_closed_or_an_open_outline() {
    let corners = vec![(0.1, 0.2), (0.4, 0.25), (0.3, 0.6)];
    let closed = mask_add(
        "one",
        "k3",
        &Outline::Points {
            points: corners.clone(),
            closed: true,
        },
    );
    assert_eq!(
        closed,
        json!({"op": "mask-add", "clip": "one", "name": "k3",
            "points": [[0.1, 0.2], [0.4, 0.25], [0.3, 0.6]], "closed": true})
    );
    let open = mask_add(
        "one",
        "k4",
        &Outline::Points {
            points: corners[..2].to_vec(),
            closed: false,
        },
    );
    assert_eq!(open["closed"], json!(false));
    assert_eq!(open["points"].as_array().map(Vec::len), Some(2));
}

fn outline() -> impl Strategy<Value = Outline> {
    let unit = -0.5f64..1.5;
    prop_oneof![
        (unit.clone(), unit.clone(), 0.001f64..1.0, 0.001f64..1.0)
            .prop_map(|(l, t, w, h)| Outline::Rect([l, t, l + w, t + h])),
        (unit.clone(), unit.clone(), 0.001f64..1.0, 0.001f64..1.0)
            .prop_map(|(x, y, rx, ry)| Outline::Ellipse([x, y, rx, ry])),
        (
            prop::collection::vec((unit.clone(), unit), 2..8),
            any::<bool>()
        )
            .prop_map(|(points, closed)| Outline::Points { points, closed }),
    ]
}

fn rounded(value: f64) -> f64 {
    (value * 10000.0).round() / 10000.0
}

proptest! {
    /// What is dragged out does not depend on which corner the drag began
    /// at, and lies within the two.
    #[test]
    fn a_drag_is_the_same_from_either_corner(
        a in (-0.5f64..1.5, -0.5f64..1.5),
        b in (-0.5f64..1.5, -0.5f64..1.5),
        ellipse in any::<bool>(),
    ) {
        prop_assert_eq!(dragged(ellipse, a, b), dragged(ellipse, b, a));
        prop_assert_eq!(drawn(ellipse, a, b, half()), drawn(ellipse, b, a, half()));
        let (low_x, high_x) = (a.0.min(b.0), a.0.max(b.0));
        let (low_y, high_y) = (a.1.min(b.1), a.1.max(b.1));
        match dragged(ellipse, a, b) {
            Outline::Rect([l, t, r, b]) => {
                prop_assert!(l == low_x && r == high_x && t == low_y && b == high_y);
            }
            Outline::Ellipse([x, y, rx, ry]) => {
                prop_assert!((x - rx - low_x).abs() < 1e-9 && (x + rx - high_x).abs() < 1e-9);
                prop_assert!((y - ry - low_y).abs() < 1e-9 && (y + ry - high_y).abs() < 1e-9);
            }
            other => prop_assert!(false, "{other:?}"),
        }
    }

    /// A drawn rectangle or ellipse holds an area as the document keeps it
    /// (four places), which is what kerfd asks of a mask.
    #[test]
    fn what_is_drawn_holds_an_area(
        a in (-0.5f64..1.5, -0.5f64..1.5),
        b in (-0.5f64..1.5, -0.5f64..1.5),
        ellipse in any::<bool>(),
    ) {
        if let Some(outline) = drawn(ellipse, a, b, half()) {
            let op = mask_add("one", "k1", &outline);
            match outline {
                Outline::Rect(_) => {
                    let rect: Vec<f64> = op["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                    prop_assert!(rect[2] > rect[0] && rect[3] > rect[1], "{rect:?}");
                }
                Outline::Ellipse(_) => {
                    let ellipse: Vec<f64> = op["ellipse"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                    prop_assert!(ellipse[2] > 0.0 && ellipse[3] > 0.0, "{ellipse:?}");
                }
                Outline::Points { .. } => prop_assert!(false),
            }
        }
    }

    /// The edit that adds a mask reads back, as the snapshot carries it, as
    /// the outline drawn to four places, under the name given.
    #[test]
    fn an_added_mask_reads_back_as_drawn(outline in outline(), number in 1u32..50) {
        let name = format!("k{number}");
        let op = mask_add("one", &name, &outline);
        prop_assert_eq!(&op["op"], &json!("mask-add"));
        prop_assert_eq!(&op["clip"], &json!("one"));
        let shown = masks_of(&json!({"masks": [op]}));
        prop_assert_eq!(shown.len(), 1);
        prop_assert_eq!(&shown[0].name, &name);
        let expected = match outline {
            Outline::Rect(rect) => Outline::Rect(rect.map(rounded)),
            Outline::Ellipse(ellipse) => Outline::Ellipse(ellipse.map(rounded)),
            Outline::Points { points, closed } => Outline::Points {
                points: points.iter().map(|(x, y)| (rounded(*x), rounded(*y))).collect(),
                closed,
            },
        };
        prop_assert_eq!(&shown[0].outline, &expected);
    }

    /// A new mask's name is never one the clip has, and is the first free.
    #[test]
    fn a_new_name_is_free_and_the_first(taken in prop::collection::vec(1u32..12, 0..10)) {
        let taken: Vec<String> = taken.iter().map(|n| format!("k{n}")).collect();
        let name = new_mask_name(&taken);
        prop_assert!(!taken.contains(&name));
        let number: u32 = name[1..].parse().unwrap();
        for lower in 1..number {
            let lower_name = format!("k{lower}");
            prop_assert!(taken.contains(&lower_name));
        }
    }

    /// The making's line reads the same whatever its case or the words after
    /// its first.
    #[test]
    fn a_making_line_reads_by_its_first_word(ahead in 0u32..20, seconds in 0u64..100_000) {
        prop_assert_eq!(
            chip::phase(&format!("WAITING for a slot, {ahead} ahead, {seconds}s")),
            chip::Phase::Waiting { ahead: Some(ahead) }
        );
        prop_assert_eq!(chip::phase(&format!("running {seconds}s")), chip::Phase::Running);
        let clock = chip::elapsed(seconds);
        let parts: Vec<u64> = clock.split(':').map(|part| part.parse().unwrap()).collect();
        let back = parts.iter().fold(0, |sum, part| sum * 60 + part);
        prop_assert_eq!(back, seconds);
    }

    /// The making's line is whatever its program printed, in any script:
    /// the chip reads it without cutting a character, and the words after
    /// its first do not change how it stands.
    #[test]
    fn a_making_line_in_any_words_is_read_whole(line in any::<String>(), seconds in 0u64..100_000) {
        let phase = chip::phase(&line);
        let shown = chip::makings(&[("a".to_owned(), line.clone(), seconds)]);
        let shown = shown.unwrap_or_default();
        prop_assert!(shown.starts_with("Making a · "), "{shown}");
        if !phase.queued() {
            prop_assert_eq!(shown, format!("Making a · {}", chip::elapsed(seconds)));
        }
        prop_assert_eq!(chip::phase(&format!("running {line}")), chip::Phase::Running);
        prop_assert_eq!(chip::phase(&format!("Asked {line}")), chip::Phase::Asked);
    }
}

#[test]
fn a_making_line_in_another_script_says_its_clock() {
    // Each would cut a character at the end of "asked" or "waiting".
    for line in [
        "Génération de la vidéo 40%",
        "Генерация видео",
        "動画を生成中 40%",
        "aské",
        "é",
        "madeé",
    ] {
        assert_eq!(chip::phase(line), chip::Phase::Other, "{line}");
        assert_eq!(
            chip::makings(&[("a".to_owned(), line.to_owned(), 4)]).as_deref(),
            Some("Making a · 0:04"),
            "{line}"
        );
    }
    assert_eq!(chip::phase("asked…"), chip::Phase::Asked);
    assert_eq!(chip::phase("rendering 40% — vidéo"), chip::Phase::Rendering);
}

#[test]
fn the_chip_names_the_making_and_says_how_it_stands() {
    let making = |name: &str, line: &str, seconds: u64| (name.to_owned(), line.to_owned(), seconds);
    assert_eq!(chip::makings(&[]), None);
    assert_eq!(
        chip::makings(&[making("Harbour", "running 12s", 65)]).as_deref(),
        Some("Making Harbour · 1:05")
    );
    // The one making, ahead of one waiting its turn; the other counted.
    assert_eq!(
        chip::makings(&[
            making("Boat", "waiting, 2 ahead", 3),
            making("Harbour", "rendering 40%", 3700),
        ])
        .as_deref(),
        Some("Making Harbour · 1:01:40 +1")
    );
    // All waiting: the first, queued, with how many are ahead.
    assert_eq!(
        chip::makings(&[
            making("Boat", "waiting, 2 ahead", 3),
            making("Dock", "asked", 1)
        ])
        .as_deref(),
        Some("Making Boat · queued, 2 ahead +1")
    );
    assert_eq!(
        chip::makings(&[making("Dock", "asked", 1)]).as_deref(),
        Some("Making Dock · queued")
    );
    assert_eq!(chip::phase("asked to make it"), chip::Phase::Asked);
    assert_eq!(chip::phase("made 31s"), chip::Phase::Made);
    assert_eq!(chip::phase("askedx"), chip::Phase::Other);
    assert_eq!(chip::stem("harbour.take2.mp4"), "harbour.take2");
    assert_eq!(chip::stem(".hidden"), ".hidden");
}

#[test]
fn the_preview_card_says_what_renders() {
    assert_eq!(hud::title(2), "Rendering preview · v2");
    assert_eq!(hud::title(0), "Rendering preview");
    let moved = ClipChange {
        clip: "c1".into(),
        name: "Logo reveal".into(),
        moved: true,
        old_start: 20.0,
        new_start: 26.5,
        ..ClipChange::default()
    };
    let removed = ClipChange {
        clip: "c2".into(),
        removed: true,
        ..ClipChange::default()
    };
    assert_eq!(
        hud::detail(&[moved.clone(), removed.clone()], "", "", 24.0),
        "Logo reveal → 00:00:26:12 · 2 changes"
    );
    assert_eq!(
        hud::detail(&[removed], "", "", 24.0),
        "c2 removed · 1 change"
    );
    assert_eq!(
        hud::detail(&[], "\n  Tighten the open\nmore", "Rendering", 24.0),
        "Tighten the open"
    );
    assert_eq!(
        hud::detail(&[], "", "  Rendering   frames ", 24.0),
        "Rendering frames"
    );
    assert_eq!(hud::percent(0.416), "42%");
    assert_eq!(hud::percent(1.7), "100%");
}

fn check(findings: Value) -> Value {
    json!({"type": "check", "status": "done", "findings": findings})
}

#[test]
fn the_overlap_box_is_the_finding_the_playhead_is_on() {
    let overlap = json!({"code": "text.overlap", "severity": "warning", "message": "",
        "at": 1.0, "duration": 2.0, "rect": {"x": 0.3, "y": 0.8, "w": 0.4, "h": 0.1},
        "value": 59.0, "unit": "px"});
    let other = json!({"code": "text.title_safe", "at": 0.0, "duration": 10.0,
        "rect": {"x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0}});
    let flat = json!({"code": "text.overlap", "at": 0.0, "duration": 10.0,
        "rect": {"x": 0.1, "y": 0.1, "w": 0.0, "h": 0.2}});
    let event = check(json!([other, flat, overlap]));
    let found = Overlap {
        rect: [0.3, 0.8, 0.4, 0.1],
        tag: "59 px".into(),
    };
    assert_eq!(overlap_at(&event, 1.5, 24.0), Some(found.clone()));
    // Half a frame either side is still on it; a frame past is not.
    assert_eq!(
        overlap_at(&event, 1.0 - 0.4 / 24.0, 24.0),
        Some(found.clone())
    );
    assert_eq!(overlap_at(&event, 3.0 + 0.4 / 24.0, 24.0), Some(found));
    assert_eq!(overlap_at(&event, 3.0 + 1.0 / 24.0, 24.0), None);
    assert_eq!(overlap_at(&event, 0.5, 24.0), None);
    // A finding with no time is about every frame; one with no value has no tag.
    let timeless = check(json!([{"code": "text.overlap",
        "rect": {"x": 0.1, "y": 0.2, "w": 0.3, "h": 0.4}, "value": 12.5}]));
    assert_eq!(
        overlap_at(&timeless, 99.0, 25.0),
        Some(Overlap {
            rect: [0.1, 0.2, 0.3, 0.4],
            tag: "12.5".into()
        })
    );
    let bare =
        check(json!([{"code": "text.overlap", "rect": {"x": 0.1, "y": 0.2, "w": 0.3, "h": 0.4}}]));
    assert_eq!(
        overlap_at(&bare, 0.0, 25.0).map(|found| found.tag),
        Some(String::new())
    );
    assert_eq!(overlap_at(&check(json!([])), 0.0, 25.0), None);
}

#[test]
fn the_zoom_draws_the_frame_at_its_own_pixels_centred() {
    assert_eq!(ZOOMS.map(zoom_words), ["Fit", "50%", "100%", "200%"]);
    let card = Bounds::new(point(px(100.0), px(50.0)), size(px(800.0), px(450.0)));
    // Fit, or no frame size: the picture itself.
    assert_eq!(zoomed(card, (1920, 1080), 0.0, 2.0), card);
    assert_eq!(zoomed(card, (0, 0), 1.0, 2.0), card);
    // 100% on a display of two device pixels a point: 960 × 540 points.
    let shown = zoomed(card, (1920, 1080), 1.0, 2.0);
    assert_eq!(shown.size, size(px(960.0), px(540.0)));
    assert_eq!(shown.center(), card.center());
    // 50% on one device pixel a point.
    let shown = zoomed(card, (1920, 1080), 0.5, 1.0);
    assert_eq!(shown.size, size(px(960.0), px(540.0)));
    assert_eq!(shown.center(), card.center());
}
