//! The picture: the viewer and what is drawn over it, the transport, the picture tools, and where the compositor puts things.

use gpui::{point, px, size, Bounds};
use kerf_editor::chat::ClipChange;
use kerf_editor::focus::{
    crop_box, crop_props, ffmpeg_bin, first_cut, focus_ops, readable_frame, shown_time,
    track_region, typed_time, Region, NO_FRAME, TRACK_SIZE,
};
use kerf_editor::model::{Clip, Snapshot, Timeline};
use kerf_editor::stage::overlay::{
    canvas_of, dragged, drawn, follow_point, mask_add, mask_names, mask_set, masks_of,
    new_mask_name, overlap_at, parent_motion, parse_path, path_holds, path_of, path_text,
    placement, property_at, sampled, sampled_point_for, start_path, with_point_on, Affine,
    MotionPath, Outline, Overlap, Projective, Pt, DRAW_LEAST, SAMPLED_STEPS,
};
use kerf_editor::stage::{
    base64_decode, chip, hud, kerf_cutting, placed_clips, poster_source, preview_chip_words,
    previewed_version, releases_cover, shows_program, transport_reasons, transport_tip, wide,
    zoom_words, zoomed, Heard, Kept, TRANSPORT, WIDE_FROM, ZOOMS,
};
use kerf_editor::store::{EditorState, Making, Preview};
use kerf_editor::workspace::render_stop_holds;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[test]
fn the_render_preview_button_says_why_a_render_is_needed() {
    let preview = |status: &str, message: Option<&str>| Preview {
        status: status.to_owned(),
        message: message.map(str::to_owned),
        ..Preview::default()
    };
    assert_eq!(
        preview("required", Some("Effects need a render"))
            .reason()
            .as_deref(),
        Some("Effects need a render")
    );
    assert_eq!(
        preview("required", Some("  ")).reason().as_deref(),
        Some("Rendered preview required")
    );
    assert_eq!(
        preview("unavailable", None).reason().as_deref(),
        Some("Rendered preview required")
    );
    for status in ["ready", "live", "running", "stopped", ""] {
        assert_eq!(preview(status, Some("x")).reason(), None, "{status}");
    }
    // As the store keeps it from the bridge's event.
    let mut state = EditorState::default();
    state.apply(
        &json!({"type": "preview", "status": "required", "message": "Effects need a render"}),
    );
    assert_eq!(
        state.preview.reason().as_deref(),
        Some("Effects need a render")
    );
}

proptest! {
    /// A render stopped here keeps its card done with while that render
    /// runs on, and no longer: any other status ends it, another
    /// proposal's render takes over, and other events leave it be.
    #[test]
    fn a_stopped_render_stays_stopped_until_its_run_ends(
        stopped in "[a-c]{0,2}",
        proposal in "[a-c]{0,2}",
        status in prop::sample::select(vec!["running", "done", "stopped", "failed", "required", ""]),
        kind in prop::sample::select(vec!["preview", "state", "agent", "progress"]),
    ) {
        let event = json!({"type": kind, "status": status, "proposalId": proposal});
        let holds = render_stop_holds(Some(&stopped), &event);
        if kind != "preview" {
            prop_assert!(holds);
        } else {
            prop_assert_eq!(holds, status == "running" && proposal == stopped);
        }
        // Nothing stopped, nothing held.
        prop_assert!(!render_stop_holds(None, &event));
    }
}

fn close(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual} against {expected}"
    );
}

/// kerf-compose's `a_parents_motion_is_where_the_table_says`: each row is a
/// parent's numbers, its picture's size, whether it is fitted, the canvas, and
/// a point that its motion takes somewhere.
#[test]
fn a_parents_motion_is_where_the_compositor_puts_it() {
    let hd = (1920.0, 1080.0);
    let table = [
        (
            json!({"x": -0.25, "y": 0.1}),
            hd,
            false,
            hd,
            (1200.0, 540.0),
            (720.0, 648.0),
        ),
        (
            json!({"rotation": 90}),
            hd,
            false,
            hd,
            (1200.0, 540.0),
            (960.0, 780.0),
        ),
        (
            json!({"scale": 0.5}),
            hd,
            false,
            hd,
            (1200.0, 300.0),
            (1080.0, 420.0),
        ),
        (
            json!({"scale_x": 2, "rotation": 30, "anchor_x": 0.25, "anchor_y": -0.5, "x": 0.1}),
            hd,
            false,
            hd,
            (100.0, 900.0),
            (-1138.948082142, -560.577136594),
        ),
        (
            json!({"crop_l": 0.2, "crop_t": 0.1, "scale": 1.5, "y": -0.2}),
            (1280.0, 720.0),
            true,
            hd,
            (640.0, 200.0),
            (480.0, -186.0),
        ),
        (
            json!({"rotation": 45, "anchor_x": 0.5, "scale": 0.75}),
            (1000.0, 500.0),
            true,
            (1080.0, 1920.0),
            (540.0, 960.0),
            (793.621753619, 673.621753619),
        ),
        (
            json!({"path": "0,0 0.2,0.3 0.5,-0.1", "along": 0.5, "orient": true, "scale": 0.8}),
            hd,
            false,
            hd,
            (1000.0, 500.0),
            (1462.428694397, 793.07977502),
        ),
    ];
    for (props, picture, fit, canvas, from, to) in table {
        let at = parent_motion(&props, picture, fit, canvas, 0.0).apply(from);
        close(at.0, to.0, 1e-6, &props.to_string());
        close(at.1, to.1, 1e-6, &props.to_string());
    }
}

/// kerf_model's `paths_read_the_table_the_picture_is_held_to`.
#[test]
fn a_motion_path_is_where_kerf_model_says() {
    const WIDE: f64 = 16.0 / 9.0;
    const TALL: f64 = 9.0 / 16.0;
    const CURVED: &str = "0,0 0.2,0.3 0.5,-0.1 0.8,0.2";
    const LONG: &str = "-0.4,0.1 0.1,-0.3 0.2,0.25 0.45,0.05 0.3,-0.2";
    let table: &[(&str, f64, f64, f64, f64, f64)] = &[
        ("0,0 1,0", 1.0, 0.25, 0.25, 0.0, 0.0),
        (CURVED, WIDE, 0.0, 0.0, 0.0, 40.186380),
        (CURVED, WIDE, 0.2, 0.138852744, 0.255491011, 37.182011),
        (CURVED, WIDE, 0.5, 0.377158462, 0.044861162, -46.192624),
        (CURVED, WIDE, 0.8, 0.637278976, -0.006712763, 34.546793),
        (CURVED, WIDE, 1.0, 0.8, 0.2, 29.384498),
        ("0,0 0.01,0 0.02,0 1,0", 1.0, 0.5, 0.499950046, 0.0, 0.0),
        (
            "0,0 0.5,0 0.5,0.5",
            2.0,
            0.5,
            0.381055017,
            -0.061333311,
            0.422966,
        ),
        (
            "0,0 0.5,0 0.5,0.5",
            2.0,
            0.9,
            0.508284207,
            0.348099501,
            99.522964,
        ),
        (LONG, TALL, 0.1, -0.251897404, -0.034195522, -59.751285),
        (LONG, TALL, 0.37, 0.146873937, -0.219570365, 84.473801),
        (LONG, TALL, 0.66, 0.182237509, 0.237851333, 68.262319),
        (LONG, TALL, 0.95, 0.354996038, -0.127356395, -115.521511),
        ("0.3,0.3 0.3,0.3", 1.0, 0.5, 0.3, 0.3, 0.0),
    ];
    for (text, aspect, along, x, y, heading) in table {
        let points = parse_path(text).expect(text);
        let on = MotionPath::new(points, *aspect).at(*along);
        close(on.x, *x, 1e-6, text);
        close(on.y, *y, 1e-6, text);
        close(on.heading, *heading, 1e-3, text);
    }
    for refused in ["", "0,0", "0 0", "0,0 x,1", "0,0 11,0", "0,0 NaN,0"] {
        assert!(parse_path(refused).is_none(), "{refused}");
    }
}

#[test]
fn a_point_added_on_the_curve_lands_between_its_neighbours() {
    let points = parse_path("0,0 0.5,0 0.5,0.5").unwrap();
    let more = with_point_on(&points, 16.0 / 9.0, 1.5);
    assert_eq!(more.len(), 4);
    assert_eq!(
        (more[0], more[1], more[3]),
        (points[0], points[1], points[2])
    );
    let on = MotionPath::new(points, 16.0 / 9.0).curve(1.5);
    assert_eq!(more[2], on);
}

#[test]
fn keyframes_ease_between_their_values() {
    let props = json!({
        "scale": 2,
        "x": [
            {"at": 1.0, "value": 0.0, "interp": "linear"},
            {"at": 3.0, "value": 1.0, "interp": "hold"},
            {"at": 4.0, "value": 0.5}
        ]
    });
    assert_eq!(property_at(&props, "scale", 5.0, 1.0), 2.0);
    assert_eq!(property_at(&props, "missing", 5.0, 0.25), 0.25);
    assert_eq!(property_at(&props, "x", 0.0, 9.0), 0.0);
    assert_eq!(property_at(&props, "x", 2.0, 9.0), 0.5);
    // Held until the next key, then that key's value after it.
    assert_eq!(property_at(&props, "x", 3.5, 9.0), 1.0);
    assert_eq!(property_at(&props, "x", 9.0, 9.0), 0.5);
}

fn timeline() -> serde_json::Value {
    json!({
        "width": 1920, "height": 1080,
        "sources": [{"id": "m", "width": 1280, "height": 720}],
        "tracks": [
            {"id": "V1", "type": "video", "clips": [
                {"id": "a", "kind": "media", "sourceId": "m", "timelineStart": 0.0, "duration": 4.0,
                 "durationFrames": 96,
                 "masks": [
                    {"name": "k1", "mode": "add", "rect": [0.1, 0.2, 0.6, 0.8]},
                    {"name": "ring", "mode": "none", "ellipse": [0.5, 0.5, 0.25, 0.3]},
                    {"name": "cut", "mode": "subtract", "points": [[0, 0], [0.5, 1], [1, 0]], "closed": false}
                 ]},
                {"id": "b", "kind": "media", "sourceId": "m", "timelineStart": 4.0, "duration": 2.0,
                 "props": {"scale": 0.5, "x": 0.25, "path": "0,0 0.1,0.1", "along": 1.0}},
                {"id": "deep", "kind": "media", "sourceId": "m", "timelineStart": 6.0, "duration": 2.0,
                 "props": {"three_d": true}},
                {"id": "right", "kind": "media", "sourceId": "m", "timelineStart": 8.0, "duration": 2.0,
                 "durationFrames": 48, "props": {"x": 0.25, "along": 0.5}}
            ]},
            {"id": "A1", "type": "audio", "clips": [
                {"id": "s", "kind": "media", "sourceId": "m", "timelineStart": 0.0, "duration": 4.0}
            ]},
            {"id": "S1", "type": "subtitle", "clips": [
                {"id": "c", "kind": "caption", "timelineStart": 0.0, "duration": 4.0}
            ]}
        ]
    })
}

#[test]
fn a_fitted_picture_fills_the_canvas_and_a_moved_one_goes_where_it_is_moved() {
    let timeline = timeline();
    assert_eq!(canvas_of(&timeline), (1920.0, 1080.0));
    let fitted = placement(&timeline, "a", 1.0).expect("a picture");
    assert_eq!(fitted.picture.apply((0.0, 0.0)), (0.0, 0.0));
    let corner = fitted.picture.apply((1.0, 1.0));
    close(corner.0, 1920.0, 1e-9, "right");
    close(corner.1, 1080.0, 1e-9, "bottom");
    // Its path's 0, 0 is the middle of the canvas.
    assert_eq!(fitted.path.apply((0.0, 0.0)), (0.5, 0.5));

    // Half the size, a quarter of the width to the right, and at the end of
    // its path a tenth on again either way.
    let moved = placement(&timeline, "b", 5.0).expect("a picture");
    let middle = moved.picture.apply((0.5, 0.5));
    close(middle.0, 1920.0 * (0.5 + 0.25 + 0.1), 1e-6, "across");
    close(middle.1, 1080.0 * (0.5 + 0.1), 1e-6, "down");
    let left = moved.picture.apply((0.0, 0.5));
    close(middle.0 - left.0, 1920.0 / 4.0, 1e-6, "half the width");
    // The path is drawn from where the clip would be without it.
    let start = moved.path.apply((0.0, 0.0));
    close(start.0, 0.75, 1e-9, "path across");
    close(start.1, 0.5, 1e-9, "path down");
    assert_eq!(moved.clip_time, 1.0);

    // What draws no picture has nothing to place.
    for id in ["s", "c", "nowhere"] {
        assert!(placement(&timeline, id, 1.0).is_none(), "{id}");
    }
    // In depth and at rest, the camera sees it as it is flat.
    let deep = placement(&timeline, "deep", 1.0).expect("in depth, at rest");
    let corner = deep.picture.apply((1.0, 1.0));
    close(corner.0, 1920.0, 1e-6, "in depth, right");
    close(corner.1, 1080.0, 1e-6, "in depth, bottom");
}

#[test]
fn a_motion_path_starts_from_the_clip_towards_the_middle_of_the_canvas() {
    let timeline = timeline();
    // In the middle, it goes right, along it over the clip's frames.
    assert_eq!(
        start_path(&timeline, "a", 1.0),
        Some(json!({"op": "set", "clip": "a", "props": {
            "path": "0,0 0.25,0",
            "along": {"keys": [
                {"at": 0, "value": 0.0, "interp": "linear"},
                {"at": 95, "value": 1.0, "interp": "linear"}
            ]}
        }}))
    );
    // Right of the middle it goes left, and keeps how far along it says it is.
    assert_eq!(
        start_path(&timeline, "right", 9.0),
        Some(json!({"op": "set", "clip": "right", "props": {"path": "0,0 -0.25,0"}}))
    );
    // One with a path has its path; what draws no picture has none to start.
    for id in ["b", "s", "c"] {
        assert_eq!(start_path(&timeline, id, 1.0), None, "{id}");
    }
    // One in depth starts its path on the plane it is on.
    let deep = start_path(&timeline, "deep", 7.0).expect("a path in depth");
    assert_eq!(deep["props"]["path"], "0,0 0.25,0");
}

#[test]
fn masks_read_with_their_outlines_and_edit_by_their_handles() {
    let timeline = timeline();
    let clip = &timeline["tracks"][0]["clips"][0];
    let masks = masks_of(clip);
    let names: Vec<_> = masks.iter().map(|mask| mask.name.as_str()).collect();
    assert_eq!(names, ["k1", "ring", "cut"]);
    assert!(!masks[0].path_only() && masks[1].path_only() && !masks[2].path_only());
    assert_eq!(
        masks[2].outline,
        Outline::Points {
            points: vec![(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)],
            closed: false
        }
    );

    // A rectangle's corner dragged past the opposite one keeps that corner.
    let rect = &masks[0].outline;
    assert_eq!(
        rect.with_handle(0, (0.7, 0.9)),
        Outline::Rect([0.6, 0.8, 0.7, 0.9])
    );
    // An ellipse's side changes its radius that way only.
    assert_eq!(
        masks[1].outline.with_handle(1, (0.9, 0.0)),
        Outline::Ellipse([0.5, 0.5, 0.4, 0.3])
    );
    let op = mask_set("a", "k1", &rect.moved_by((0.123456, -0.1)));
    assert_eq!(
        op,
        json!({"op": "mask-set", "clip": "a", "name": "k1", "rect": [0.2235, 0.1, 0.7235, 0.7]})
    );
    let op = mask_set("a", "cut", &masks[2].outline);
    assert_eq!(op["points"], json!([[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]]));
    assert_eq!(op["closed"], json!(false));
}

proptest! {
    #[test]
    fn a_map_and_its_inverse_take_a_point_back(
        rotation in -360.0f64..360.0,
        sx in 0.05f64..8.0,
        sy in 0.05f64..8.0,
        tx in -4000.0f64..4000.0,
        ty in -4000.0f64..4000.0,
        x in -2.0f64..2.0,
        y in -2.0f64..2.0,
    ) {
        let map = Affine::scale(sx, sy)
            .then(Affine::rotate(rotation))
            .then(Affine::translate(tx, ty));
        let back = map.inverse().expect("a map that folds nothing");
        let (bx, by) = back.apply(map.apply((x, y)));
        prop_assert!((bx - x).abs() < 1e-6 && (by - y).abs() < 1e-6);
    }

    #[test]
    fn a_path_written_reads_back_to_four_places(
        points in prop::collection::vec((-12.0f64..12.0, -12.0f64..12.0), 2..8)
    ) {
        let text = path_text(&points);
        let read = parse_path(&text).expect("what path_text writes reads");
        prop_assert_eq!(read.len(), points.len());
        for ((x, y), (rx, ry)) in points.iter().zip(&read) {
            prop_assert!((x.clamp(-10.0, 10.0) - rx).abs() <= 5e-5);
            prop_assert!((y.clamp(-10.0, 10.0) - ry).abs() <= 5e-5);
            prop_assert!(path_holds((*rx, *ry)));
        }
        // Written again, it is the same text.
        prop_assert_eq!(path_text(&read), text);
    }

    #[test]
    fn a_path_goes_from_its_first_point_to_its_last(
        points in prop::collection::vec((-1.0f64..1.0, -1.0f64..1.0), 2..6),
        aspect in 0.4f64..2.5,
    ) {
        let path = MotionPath::new(points.clone(), aspect);
        let first = path.at(0.0);
        let last = path.at(1.0);
        let end = points[points.len() - 1];
        prop_assert!((first.x - points[0].0).abs() < 1e-9 && (first.y - points[0].1).abs() < 1e-9);
        prop_assert!((last.x - end.0).abs() < 1e-6 && (last.y - end.1).abs() < 1e-6);
        prop_assert_eq!(path.samples().len(), (points.len() - 1) * 32 + 1);
        prop_assert_eq!(path.along_of(0), 0.0);
    }
}

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

#[test]
fn a_square_or_near_square_picture_is_framed_and_captioned_as_a_phone() {
    assert!(!wide(9.0 / 16.0));
    assert!(!wide(1.0));
    assert!(!wide(1.19));
    assert!(wide(WIDE_FROM));
    assert!(wide(16.0 / 9.0));
    assert_eq!(WIDE_FROM, 1.2);
}

fn apart(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// A timeline of `canvas`, its clips `c1` (with `props`, `size` its
/// picture: a media fitted to the canvas when `fit`, else a generator the
/// canvas's size), a camera with `camera` and a null `n1` with `rig`, which
/// `c1` names as its parent when it has numbers.
fn scene(camera: Value, rig: Value, props: Value, size: Pt, fit: bool, canvas: Pt) -> Value {
    let mut clip = if fit {
        json!({"id": "c1", "kind": "media", "sourceId": "m"})
    } else {
        assert_eq!(size, canvas, "a generator is the canvas's size");
        json!({"id": "c1", "kind": "generator", "fx": "solid"})
    };
    clip["timelineStart"] = json!(0.0);
    clip["duration"] = json!(2.0);
    clip["props"] = props;
    if rig.as_object().is_some_and(|rig| !rig.is_empty()) {
        clip["parent"] = json!("n1");
    }
    json!({
        "width": canvas.0, "height": canvas.1,
        "sources": [{"id": "m", "width": size.0, "height": size.1}],
        "tracks": [{"id": "V1", "type": "video", "clips": [
            {"id": "cam", "kind": "generator", "fx": "camera", "timelineStart": 0.0, "duration": 2.0, "props": camera},
            {"id": "n1", "kind": "generator", "fx": "null", "timelineStart": 0.0, "duration": 2.0, "props": rig},
            clip
        ]}]
    })
}

/// One row of [`table`]: a camera, a parent, a clip, its picture's size and
/// whether it is fitted, the canvas; a point of the picture and where on the
/// canvas it lands, or none.
type TableRow = (Value, Value, Value, Pt, bool, Pt, Pt, Option<Pt>);

/// The Qt editor's table, which the compositor is held to.
fn table() -> Vec<TableRow> {
    let wide = (1920.0, 1080.0);
    let path = "0,0 0.2,0.3 0.5,-0.1";
    vec![
        (
            json!({}),
            json!({}),
            json!({"three_d": true}),
            wide,
            false,
            wide,
            (0.25, 0.75),
            Some((480.0, 810.0)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "z": 1.0}),
            wide,
            false,
            wide,
            (0.0, 0.0),
            Some((404.425304296, 227.489233666)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "z": -0.5, "x": 0.25, "y": -0.25, "scale": 0.5}),
            wide,
            false,
            wide,
            (1.0, 1.0),
            Some((2469.363321845, 540.0)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "rotate_y": 60.0}),
            wide,
            false,
            wide,
            (0.0, 0.0),
            Some((652.380423425, 193.927976353)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "rotate_y": 60.0}),
            wide,
            false,
            wide,
            (1.0, 1.0),
            Some((2051.824322445, 1768.30236275)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "z": 0.4, "rotate_x": -35.0, "rotate_y": 20.0, "scale": 0.6, "rotation": 30.0, "x": 0.15}),
            wide,
            false,
            wide,
            (0.1, 0.9),
            Some((813.293244888, 536.56976567)),
        ),
        (
            json!({"x": 0.1, "y": -0.1, "z": -0.2}),
            json!({}),
            json!({"three_d": true, "z": 0.5}),
            wide,
            false,
            wide,
            (0.75, 0.25),
            Some((1150.784280848, 432.683842023)),
        ),
        (
            json!({"pan": 15.0, "tilt": -10.0, "roll": 20.0, "fov": 60.0}),
            json!({}),
            json!({"three_d": true, "rotate_y": -25.0}),
            wide,
            false,
            wide,
            (0.9, 0.1),
            Some((925.135566674, 137.609390156)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "crop_l": 0.2, "crop_t": 0.1, "anchor_x": 0.25, "rotate_y": 30.0}),
            wide,
            false,
            wide,
            (0.6, 0.6),
            Some((932.497292288, 597.735845172)),
        ),
        (
            json!({"tilt": 5.0}),
            json!({}),
            json!({"three_d": true, "rotate_x": 25.0, "scale": 0.75}),
            (1280.0, 720.0),
            true,
            (1080.0, 1920.0),
            (0.2, 0.8),
            Some((300.339390717, 1313.504377609)),
        ),
        (
            json!({}),
            json!({"x": -0.1, "rotation": 20.0}),
            json!({"three_d": true, "z": 0.5, "rotate_y": 30.0, "x": 0.2}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            Some((1083.787134162, 636.289297037)),
        ),
        (
            json!({"pan": 10.0}),
            json!({}),
            json!({"three_d": true, "z": 1.0, "rotate_y": 40.0, "path": path, "along": 0.0}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            Some((698.394625114, 540.0)),
        ),
        (
            json!({"pan": 10.0}),
            json!({}),
            json!({"three_d": true, "z": 1.0, "rotate_y": 40.0, "path": path, "along": 0.5}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            Some((980.39775465, 708.363349572)),
        ),
        (
            json!({"pan": 10.0}),
            json!({}),
            json!({"three_d": true, "z": 1.0, "rotate_y": 40.0, "path": path, "along": 1.0}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            Some((1235.761146107, 480.464694441)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "z": 0.5, "rotate_y": 40.0, "anchor_x": 0.25, "path": path, "along": 0.25}),
            wide,
            false,
            wide,
            (0.75, 0.5),
            Some((1461.484615779, 736.839478992)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "z": 0.5, "rotate_y": 40.0, "anchor_x": 0.25, "path": path, "along": 1.0}),
            wide,
            false,
            wide,
            (0.75, 0.5),
            Some((2108.520888367, 446.575181565)),
        ),
        (
            json!({}),
            json!({"x": -0.1, "scale": 0.8}),
            json!({"three_d": true, "z": 0.5, "path": "0,0 0.2,0.3", "along": 1.0}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            Some((1044.45932111, 730.033472497)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "rotate_y": 30.0, "orient": true, "path": path, "along": 0.5}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            Some((1448.468778868, 836.127664149)),
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "z": -5.0}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            None,
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "rotate_y": 90.0}),
            wide,
            false,
            wide,
            (0.75, 0.5),
            None,
        ),
        (
            json!({}),
            json!({}),
            json!({"three_d": true, "x": 0.1, "rotate_y": 90.0}),
            wide,
            false,
            wide,
            (0.75, 0.5),
            Some((1243.825954944, 540.0)),
        ),
        (
            json!({"pan": 80.0}),
            json!({}),
            json!({"three_d": true}),
            wide,
            false,
            wide,
            (0.0, 0.5),
            None,
        ),
        (
            json!({"pan": 80.0}),
            json!({}),
            json!({"three_d": true}),
            wide,
            false,
            wide,
            (1.0, 0.5),
            Some((-636.292895473, 540.0)),
        ),
        (
            json!({}),
            json!({"z": 1.0}),
            json!({"three_d": true}),
            wide,
            false,
            wide,
            (0.0, 0.0),
            Some((404.425304296, 227.489233666)),
        ),
        (
            json!({}),
            json!({"rotate_y": 60.0}),
            json!({"three_d": true}),
            wide,
            false,
            wide,
            (1.0, 1.0),
            Some((2051.824322445, 1768.30236275)),
        ),
        (
            json!({}),
            json!({"x": 0.1, "y": -0.05, "z": 0.3, "rotate_x": -20.0, "rotate_y": 35.0}),
            json!({"three_d": true, "z": 0.2, "rotate_y": 15.0, "x": 0.2, "scale": 0.5}),
            wide,
            false,
            wide,
            (0.25, 0.75),
            Some((1304.251220744, 671.403611257)),
        ),
        (
            json!({"pan": 10.0, "tilt": -5.0}),
            json!({"x": -0.1, "rotation": 20.0, "scale": 0.8, "z": 0.5, "rotate_y": -40.0}),
            json!({"three_d": true, "z": -0.1, "rotate_x": 25.0, "y": 0.15, "scale": 0.6}),
            wide,
            false,
            wide,
            (0.9, 0.2),
            Some((806.73633729, 481.710745385)),
        ),
        (
            json!({}),
            json!({"anchor_x": 0.25, "rotation": 15.0, "rotate_y": 50.0, "z": 0.2}),
            json!({"three_d": true, "x": 0.3}),
            wide,
            false,
            wide,
            (0.5, 0.5),
            Some((1395.719634219, 568.948088124)),
        ),
    ]
}

#[test]
fn clips_in_depth_land_as_the_compositor_draws_them() {
    for (camera, rig, props, size, fit, canvas, from, to) in table() {
        let name = props.to_string();
        let timeline = scene(
            camera.clone(),
            rig.clone(),
            props.clone(),
            size,
            fit,
            canvas,
        );
        let placed = placement(&timeline, "c1", 0.0);
        let picture = placed.map_or(Projective::UNSEEN, |placed| placed.picture);
        let Some(to) = to else {
            assert!(!picture.shows() || !picture.in_sight(from), "{name}");
            continue;
        };
        assert!(picture.in_sight(from), "{name}");
        let at = picture.apply(from);
        assert!(apart(at, to) < 1e-6, "{name}: {at:?} against {to:?}");
        // And back, as a drag reads it.
        let back = picture.point_in_sight(to).expect("seen back");
        assert!(apart(back, from) < 1e-6, "{name}: back at {back:?}");
        let Some(path) = props["path"].as_str().and_then(parse_path) else {
            continue;
        };
        // The clip's anchor is where its path says, wherever along it the
        // clip is now: the path's map takes the point there to the same place.
        let along = props["along"].as_f64().unwrap_or(0.0);
        let on = MotionPath::new(path, canvas.0 / canvas.1).at(along);
        for elsewhere in [0.0, 0.3, 1.0] {
            if props["orient"] == true && elsewhere != along {
                continue;
            }
            let mut moved = props.clone();
            moved["along"] = json!(elsewhere);
            let timeline = scene(camera.clone(), rig.clone(), moved, size, fit, canvas);
            let map = placement(&timeline, "c1", 0.0).expect("placed").path;
            assert!(map.in_sight((on.x, on.y)), "{name} along {elsewhere}");
            let shares = map.apply((on.x, on.y));
            let landed = (shares.0 * canvas.0, shares.1 * canvas.1);
            assert!(
                apart(landed, to) < 1e-6,
                "{name} along {elsewhere}: {landed:?}"
            );
        }
    }
}

#[test]
fn a_plane_that_cannot_be_seen_places_nothing_and_a_flat_clip_stays_flat() {
    let wide = (1920.0, 1080.0);
    let picture_of = |props: Value| {
        placement(
            &scene(json!({}), json!({}), props, wide, false, wide),
            "c1",
            0.0,
        )
        .map_or(Projective::UNSEEN, |placed| placed.picture)
    };
    // Edge on through the camera, and wholly behind it.
    assert!(!picture_of(json!({"three_d": true, "rotate_y": 90.0})).shows());
    assert!(!picture_of(json!({"three_d": true, "rotate_x": 90.0})).shows());
    assert!(!picture_of(json!({"three_d": true, "z": -5.0})).shows());
    // Facing the other way, a clip in depth is mirrored, not gone.
    assert!(picture_of(json!({"three_d": true, "rotate_y": 180.0})).shows());
    // A flat clip keeps its flat map: everything is in sight of it.
    let flat = picture_of(json!({"x": 0.1, "three_d": false}));
    assert_eq!(flat.m[0][2], 0.0);
    assert_eq!(flat.m[1][2], 0.0);
    assert!(flat.in_sight((-100.0, 100.0)));
    // Turned edge on, its path is still on the plane at its depth.
    let turned = scene(
        json!({}),
        json!({}),
        json!({"three_d": true, "rotate_y": 90.0, "path": "0,0 0.5,0"}),
        wide,
        false,
        wide,
    );
    let placed = placement(&turned, "c1", 0.0).expect("its path shows");
    assert!(!placed.picture.shows());
    assert!(placed.path.shows());
    // A path's plane behind the camera has no point in sight.
    let behind = scene(
        json!({}),
        json!({}),
        json!({"three_d": true, "z": -5.0, "path": "0,0 0.5,0"}),
        wide,
        false,
        wide,
    );
    let path = placement(&behind, "c1", 0.0).map_or(Projective::UNSEEN, |placed| placed.path);
    assert!(!path.in_sight((0.0, 0.0)));
    assert!(path.point_in_sight((0.5, 0.5)).is_none());
    // An anchor off the middle leaves the plane by a different amount at
    // each point of a path it faces along: no one map shows that.
    let path_of = |props: Value| {
        placement(
            &scene(json!({}), json!({}), props, wide, false, wide),
            "c1",
            0.0,
        )
        .expect("placed")
        .path
    };
    assert!(!path_of(json!({"three_d": true, "orient": true, "anchor_x": 0.25, "rotate_y": 30.0, "path": "0,0 0.5,0"})).shows());
    assert!(path_of(
        json!({"three_d": true, "orient": true, "anchor_x": 0.25, "path": "0,0 0.5,0"})
    )
    .shows());
    assert!(path_of(
        json!({"three_d": true, "orient": true, "rotate_y": 30.0, "path": "0,0 0.5,0"})
    )
    .shows());
    assert!(path_of(
        json!({"three_d": true, "anchor_x": 0.25, "rotate_y": 30.0, "path": "0,0 0.5,0"})
    )
    .shows());
}

#[test]
fn a_camera_is_the_topmost_showing() {
    let wide = (1920.0, 1080.0);
    let camera = |id: &str, start: f64, fov: f64, enabled: bool| json!({"id": id, "fx": "camera", "timelineStart": start, "duration": 1.0, "enabled": enabled, "props": {"fov": fov}});
    let clip = json!({"id": "c1", "kind": "generator", "fx": "solid", "timelineStart": 0.0, "duration": 4.0, "props": {"three_d": true}});
    let width = |cameras: Value, at: f64| {
        let mut tracks = cameras.as_array().cloned().unwrap_or_default();
        tracks.push(json!({"id": "V9", "type": "video", "clips": [clip.clone()]}));
        let timeline = json!({"width": 1920, "height": 1080, "tracks": tracks});
        let map = placement(&timeline, "c1", at).expect("placed").picture;
        map.apply((1.0, 0.5)).0 - map.apply((0.0, 0.5)).0
    };
    let tracks = json!([
        {"clips": [camera("a", 0.0, 20.0, true), camera("b", 2.0, 80.0, false)]},
        {"clips": [camera("c", 0.0, 40.0, true)]},
        {"enabled": false, "clips": [camera("d", 0.0, 80.0, true)]}
    ]);
    // The upper of the two, not the one on a track turned off: at the
    // resting 40 degrees, the picture is its own size.
    assert!((width(tracks.clone(), 0.5) - wide.0).abs() < 1e-6);
    // Past the end of both, the resting one; one turned off does not show.
    assert!((width(tracks, 2.5) - wide.0).abs() < 1e-6);
    // Alone, a narrower one sees it wider, by the tangents of half of each.
    let narrow = json!([{"clips": [camera("a", 0.0, 20.0, true)]}]);
    let wider = wide.0 * 20f64.to_radians().tan() / 10f64.to_radians().tan();
    assert!((width(narrow.clone(), 0.5) - wider).abs() < 1e-6);
    assert!((width(narrow, 1.5) - wide.0).abs() < 1e-6);
}

#[test]
fn a_camera_goes_with_its_parent_as_a_clip_does() {
    let at = |rig: Value, props: Value, follows: bool| {
        let mut camera = json!({"id": "cam", "kind": "generator", "fx": "camera", "timelineStart": 0.0, "duration": 2.0,
            "props": {"x": 0.15, "y": -0.2, "z": -0.3, "pan": -8.0, "tilt": -10.0, "roll": 4.0, "fov": 50.0}});
        if follows {
            camera["parent"] = json!("rig");
        }
        let timeline = json!({"width": 1920, "height": 1080, "tracks": [{"id": "V1", "type": "video", "clips": [
            camera,
            // The null is itself on a null that does nothing, so the chain is walked.
            {"id": "rig", "kind": "generator", "fx": "null", "timelineStart": 0.0, "duration": 2.0, "parent": "still", "props": rig},
            {"id": "still", "kind": "generator", "fx": "null", "timelineStart": 0.0, "duration": 2.0, "props": {}},
            {"id": "c1", "kind": "generator", "fx": "solid", "timelineStart": 0.0, "duration": 2.0, "parent": "rig", "props": props}
        ]}]});
        placement(&timeline, "c1", 0.0).map_or(Projective::UNSEEN, |placed| placed.picture)
    };
    let rows = [
        (
            json!({"x": 0.3, "y": -0.2, "z": 1.5}),
            json!({"three_d": true, "z": 0.8, "rotate_x": -75.0, "rotate_y": 35.0, "scale": 0.5, "x": 0.1}),
        ),
        (
            json!({"rotate_y": 70.0, "z": 0.4}),
            json!({"three_d": true, "z": 0.8, "rotate_y": 35.0, "scale": 0.5, "x": -0.2}),
        ),
        (
            json!({"x": -0.1, "rotate_x": -40.0, "rotate_y": 25.0, "z": -0.3}),
            json!({"three_d": true, "z": 0.5, "rotate_x": 20.0, "scale": 0.5}),
        ),
        (
            json!({"x": 0.2, "rotation": 30.0, "z": 0.5, "rotate_y": -50.0}),
            json!({"three_d": true, "z": 0.8, "scale": 0.5, "y": 0.1}),
        ),
    ];
    for (rig, props) in rows {
        let name = rig.to_string();
        let still = at(json!({}), props.clone(), true);
        let moved = at(rig.clone(), props.clone(), true);
        assert!(still.shows() && moved.shows(), "{name}");
        for corner in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.25, 0.75)] {
            assert!(still.in_sight(corner) && moved.in_sight(corner), "{name}");
            assert!(
                apart(still.apply(corner), moved.apply(corner)) < 1e-6,
                "{name} at {corner:?}"
            );
        }
        // Let go, the camera stays where it was, and the clip has gone.
        let alone = at(rig, props, false);
        assert!(
            !alone.shows() || apart(alone.apply((0.25, 0.75)), still.apply((0.25, 0.75))) > 1.0,
            "{name}"
        );
    }
}

const FACING: &str = "0,0 0.2,0.3 0.5,-0.1";

fn facing(props: Value) -> Value {
    scene(
        json!({}),
        json!({}),
        props,
        (1920.0, 1080.0),
        false,
        (1920.0, 1080.0),
    )
}

#[test]
fn a_path_facing_along_itself_is_drawn_place_by_place() {
    let props = json!({"three_d": true, "orient": true, "anchor_x": 0.25, "rotate_y": 30.0, "z": 0.2, "path": FACING});
    let timeline = facing(props.clone());
    let placed = placement(&timeline, "c1", 0.0).expect("placed");
    assert!(!placed.path.shows(), "no one map shows it");
    let curve = sampled(&timeline, "c1", 0.0, None).expect("sampled");
    assert_eq!(curve.places.len(), SAMPLED_STEPS + 1);
    assert!(curve.places.iter().all(Option::is_some));
    let landed = |step: usize| {
        let (x, y) = curve.places[step].expect("in sight");
        (x * 1920.0, y * 1080.0)
    };
    // Where the compositor puts the anchor half way along, and at the end.
    assert!(apart(landed(SAMPLED_STEPS / 2), (1869.824956407, 835.685078047)) < 1e-6);
    assert!(apart(landed(SAMPLED_STEPS), (2326.408450295, 433.723742822)) < 1e-6);
    // Its points are where the anchor is at each: the ends are the curve's.
    assert_eq!(curve.handles.len(), 3);
    assert!(apart(curve.handles[0].unwrap(), curve.places[0].unwrap()) < 1e-9);
    assert!(
        apart(
            curve.handles[2].unwrap(),
            curve.places[SAMPLED_STEPS].unwrap()
        ) < 1e-9
    );
    // Where the clip is now: its `along`, 0 when it says none.
    assert_eq!(curve.now, curve.places[0]);
    // A flat path, or one a map shows, is not sampled.
    let shown = facing(json!({"three_d": true, "orient": true, "rotate_y": 30.0, "path": FACING}));
    assert!(sampled(&shown, "c1", 0.0, None).is_none());
    // All of it behind the camera shows none.
    let mut behind = props;
    behind["z"] = json!(-5.0);
    let behind = facing(behind);
    assert!(sampled(&behind, "c1", 0.0, None).is_none());
    assert!(placement(&behind, "c1", 0.0).is_none());
}

#[test]
fn a_path_facing_along_itself_is_dragged_by_solving_for_its_point() {
    let points: Vec<Pt> = vec![(0.0, 0.0), (0.2, 0.3), (0.5, -0.1)];
    for props in [
        json!({"three_d": true, "orient": true, "anchor_x": 0.25, "rotate_y": 30.0, "z": 0.2, "path": FACING}),
        json!({"three_d": true, "orient": true, "anchor_x": -0.3, "anchor_y": 0.2, "rotate_x": -25.0, "rotate_y": 40.0, "scale": 0.6, "path": FACING}),
    ] {
        let timeline = facing(props.clone());
        let from = sampled(&timeline, "c1", 0.0, Some(&points))
            .expect("sampled")
            .handles;
        for (index, start) in from.iter().enumerate() {
            let start = start.expect("in sight");
            for by in [(0.03, 0.05), (-0.1, 0.02), (0.2, -0.15)] {
                let wanted = (start.0 + by.0, start.1 + by.1);
                let point = sampled_point_for(&timeline, "c1", 0.0, &points, index, wanted)
                    .unwrap_or_else(|| panic!("{props} point {index} to {wanted:?}"));
                let mut moved = points.clone();
                moved[index] = point;
                // Exactly, as far as a double says it.
                let exact = sampled(&timeline, "c1", 0.0, Some(&moved)).expect("sampled");
                assert!(apart(exact.handles[index].unwrap(), wanted) < 1e-6);
                // Through what the document says of it: under a third of a pixel off.
                let written = parse_path(&path_text(&moved)).expect("written");
                let landed = sampled(&timeline, "c1", 0.0, Some(&written))
                    .expect("sampled")
                    .handles[index]
                    .unwrap();
                let off = ((landed.0 - wanted.0) * 1920.0).hypot((landed.1 - wanted.1) * 1080.0);
                assert!(off < 0.3, "point {index} off by {off} pixels");
                // What it lands on is the curve the clip goes along.
                let nearest = exact
                    .places
                    .iter()
                    .flatten()
                    .map(|place| apart(*place, wanted))
                    .fold(f64::MAX, f64::min);
                assert!(nearest < 0.02);
            }
        }
    }
    // Behind the camera there is no such point to find.
    let held = facing(
        json!({"three_d": true, "orient": true, "anchor_x": 0.25, "rotate_y": 30.0, "z": -5.0, "path": FACING}),
    );
    assert!(sampled_point_for(&held, "c1", 0.0, &points, 1, (0.5, 0.5)).is_none());
}

proptest! {
    /// Whatever a clip in depth turns or how far back it is, a point of it
    /// in sight is seen back where it is (as a drag reads the picture).
    #[test]
    fn a_point_in_depth_is_read_back_where_it_is(
        pitch in -70.0f64..70.0,
        yaw in -70.0f64..70.0,
        z in -0.4f64..2.0,
        scale in 0.2f64..1.5,
        x in -0.4f64..0.4,
        point in (0.0f64..1.0, 0.0f64..1.0),
    ) {
        let props = json!({"three_d": true, "rotate_x": pitch, "rotate_y": yaw, "z": z, "scale": scale, "x": x});
        let timeline = facing(props);
        let Some(placed) = placement(&timeline, "c1", 0.0) else {
            return Ok(());
        };
        let map = placed.picture;
        prop_assume!(map.shows() && map.in_sight(point));
        let seen = map.apply(point);
        let back = map.point_in_sight(seen).expect("seen back");
        prop_assert!(apart(back, point) < 1e-6, "{point:?} came back at {back:?}");
    }

    /// A flat clip's map is the flat one, whatever it is: nothing is out of
    /// sight of it and a point comes back exactly where it was.
    #[test]
    fn a_flat_clip_is_seen_flat(
        scale in 0.2f64..2.0,
        rotation in -180.0f64..180.0,
        x in -0.5f64..0.5,
        point in (-1.0f64..2.0, -1.0f64..2.0),
    ) {
        let timeline = facing(json!({"scale": scale, "rotation": rotation, "x": x}));
        let map = placement(&timeline, "c1", 0.0).expect("placed").picture;
        prop_assert!(map.in_sight(point));
        let back = map.point_in_sight(map.apply(point)).expect("back");
        prop_assert!(apart(back, point) < 1e-6);
    }
}

// ------------------------------------------- what the picture's clicks take

/// The path of `text`, which the tests write well.
fn path(text: &str) -> Vec<Pt> {
    parse_path(text).expect("a path")
}

#[test]
fn a_picture_out_of_sight_takes_no_mask_and_a_path_out_of_sight_is_none() {
    let placed = |props: Value| placement(&facing(props), "c1", 0.0);
    let two = path("0,0 0.5,0");
    // Behind the camera: its path's map still folds nothing, so the clip is
    // placed, but neither its masks nor any of its path can be seen, and a
    // click on the picture plays (Qt's editsMasks, and setMotionPath
    // dropping a path all on or behind the camera).
    let behind = placed(json!({"three_d": true, "z": -2.0, "path": "0,0 0.5,0"}))
        .expect("placed by its path's map");
    assert!(behind.path.shows());
    assert!(!behind.masks_seen());
    assert!(!behind.path_seen(&two));
    // Edge on: its masks cannot be seen, its path on the plane can.
    let edge_on =
        placed(json!({"three_d": true, "rotate_y": 90.0, "path": "0,0 0.5,0"})).expect("placed");
    assert!(!edge_on.masks_seen());
    assert!(edge_on.path_seen(&two));
    // Facing the camera, flat or in depth, both are seen.
    for props in [
        json!({}),
        json!({"three_d": true, "z": 0.5, "rotate_y": 30.0}),
    ] {
        let seen = placed(props).expect("placed");
        assert!(seen.masks_seen());
        assert!(seen.path_seen(&two));
    }
    // A path no one map shows is not seen through it: it is sampled.
    let facing_along = placed(
        json!({"three_d": true, "orient": true, "anchor_x": 0.25, "rotate_y": 30.0, "path": FACING}),
    )
    .expect("placed");
    assert!(!facing_along.path_seen(&path(FACING)));
}

proptest! {
    /// However far behind the camera a clip in depth is, and however a
    /// path on it runs, nothing of it takes a click; turned a little, its
    /// picture is still all behind.
    #[test]
    fn nothing_behind_the_camera_is_seen(
        z in -50.0f64..-3.0,
        yaw in -60.0f64..60.0,
        points in proptest::collection::vec((-1.0f64..2.0, -1.0f64..2.0), 2..6),
    ) {
        let timeline = facing(json!({"three_d": true, "z": z, "rotate_y": yaw}));
        if let Some(placed) = placement(&timeline, "c1", 0.0) {
            prop_assert!(!placed.masks_seen());
            prop_assert!(!placed.path_seen(&points));
        }
    }

    /// A flat clip's masks and any path on it are always seen.
    #[test]
    fn a_flat_clip_is_always_seen(
        scale in 0.2f64..2.0,
        rotation in -180.0f64..180.0,
        x in -0.5f64..0.5,
        points in proptest::collection::vec((-1.0f64..2.0, -1.0f64..2.0), 2..6),
    ) {
        let timeline = facing(json!({"scale": scale, "rotation": rotation, "x": x}));
        let placed = placement(&timeline, "c1", 0.0).expect("placed");
        prop_assert!(placed.masks_seen());
        prop_assert!(placed.path_seen(&points));
    }
}

// ------------------------------------------------------------ the chip

/// A making begun at `begun()` with `line`. The chip is read some seconds
/// later (`at`), counted forward from when it began, so that no clock has
/// to go back before the machine's own clock began (Windows counts from boot).
fn making(take: u32, line: &str) -> Making {
    Making {
        take,
        line: line.to_owned(),
        since: begun(),
    }
}

fn begun() -> Instant {
    static BEGUN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *BEGUN.get_or_init(Instant::now)
}

fn at(seconds: u64) -> Instant {
    begun() + Duration::from_secs(seconds)
}

#[test]
fn the_chip_follows_a_making_in_its_own_words() {
    assert_eq!(chip::following(&[]), None);
    // Before its steps have said anything.
    let begun =
        chip::following_at(&[("Harbour".to_owned(), making(2, ""))], at(3)).expect("a chip");
    assert_eq!(begun.said, "Making take 2 of Harbour from its steps…");
    assert_eq!(begun.stands, "0:03");
    assert_eq!(begun.tip, "Making take 2 of Harbour from its steps… · 0:03");
    // Then what the step last said, and its clock.
    let going = chip::following_at(
        &[("Harbour".to_owned(), making(2, "  rendering 40%  "))],
        at(65),
    )
    .expect("a chip");
    assert_eq!(going.said, "Making Harbour: rendering 40%");
    assert_eq!(going.stands, "1:05");
    // The one making goes ahead of one waiting its turn, whose words are in
    // the tip under it; the others are counted after the clock.
    let both = chip::following_at(
        &[
            ("Boat".to_owned(), making(1, "waiting, 2 ahead")),
            ("Harbour".to_owned(), making(3, "running 12s")),
        ],
        at(3700),
    )
    .expect("a chip");
    assert_eq!(both.said, "Making Harbour: running 12s");
    assert_eq!(both.stands, "1:01:40 +1");
    assert_eq!(
        both.tip,
        "Making Harbour: running 12s · 1:01:40\nMaking Boat: waiting, 2 ahead · queued, 2 ahead"
    );
    // All waiting: the first, queued.
    let waiting =
        chip::following_at(&[("Dock".to_owned(), making(1, "asked"))], at(1)).expect("a chip");
    assert_eq!(waiting.said, "Making Dock: asked");
    assert_eq!(waiting.stands, "queued");
}

#[test]
fn a_making_read_before_it_began_says_no_time() {
    // As the window can read a clock a hair before the making's own.
    let later = Making {
        take: 1,
        line: "running".to_owned(),
        since: at(10),
    };
    let chip = chip::following_at(&[("Dock".to_owned(), later)], at(0)).expect("a chip");
    assert_eq!(chip.stands, "0:00");
}

proptest! {
    /// Whatever a making's program prints, the chip says it whole (its
    /// tip too), and its clock or how it waits is said after it, never cut.
    #[test]
    fn the_chip_says_any_line_whole(line in any::<String>(), take in 1u32..20, seconds in 0u64..100_000) {
        let said = chip::following_at(&[("m4".to_owned(), making(take, &line))], at(seconds)).expect("a chip");
        let status = kerf_editor::store::making_status("m4", &making(take, &line));
        prop_assert_eq!(&said.said, &status);
        prop_assert!(said.tip.starts_with(&status));
        prop_assert!(said.tip.ends_with(&said.stands));
    }
}

fn clip_value(id: &str, start: f64) -> Value {
    json!({"id": id, "mediaId": "m", "timelineStart": start, "duration": 2.0,
        "sourceIn": 0.0, "sourceOut": 2.0})
}

/// A state whose program has clips (or none), with a proposal waiting
/// (or not).
fn state(program_has_clips: bool, proposal_waits: bool) -> EditorState {
    let program = if program_has_clips {
        json!({"fps": 24.0, "tracks": [{"id": "V1", "type": "video", "clips": [clip_value("a", 0.0)]}]})
    } else {
        json!({"fps": 24.0, "tracks": [{"id": "V1", "type": "video", "clips": []}]})
    };
    let proposed = json!({"fps": 24.0, "tracks": [{"id": "V1", "type": "video", "clips": [clip_value("b", 0.0)]}]});
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

// ------------------------------------------------------------ Focus Region

fn clip(json: Value) -> Clip {
    serde_json::from_value(json).unwrap()
}

proptest! {
    #[test]
    fn a_box_dragged_is_inside_the_frame_either_way_round(
        a in (-0.5f64..1.5, -0.5f64..1.5),
        b in (-0.5f64..1.5, -0.5f64..1.5),
    ) {
        let region = Region::between(a, b);
        prop_assert_eq!(region, Region::between(b, a));
        prop_assert!(region.x >= 0.0 && region.y >= 0.0);
        prop_assert!(region.x + region.width <= 1.0 + 1e-12);
        prop_assert!(region.y + region.height <= 1.0 + 1e-12);
    }

    #[test]
    fn a_crop_reads_back_as_its_box(
        x in 0.0f64..0.9, y in 0.0f64..0.9, w in 0.05f64..1.0, h in 0.05f64..1.0,
    ) {
        let region = Region { x, y, width: w.min(1.0 - x), height: h.min(1.0 - y) };
        let back = crop_box(&crop_props(Some(region))).unwrap();
        prop_assert!((back.x - region.x).abs() < 1e-9 && (back.y - region.y).abs() < 1e-9);
        prop_assert!((back.width - region.width).abs() < 1e-9);
        prop_assert!((back.height - region.height).abs() < 1e-9);
    }
}

#[test]
fn focusing_splits_the_clip_round_the_range_and_crops_the_middle() {
    let region = Region {
        x: 0.25,
        y: 0.25,
        width: 0.5,
        height: 0.5,
    };
    let old = Some(Region {
        x: 0.1,
        y: 0.0,
        width: 0.8,
        height: 1.0,
    });
    let (ops, focused) = focus_ops("one", (0, 96), (24, 72), region, &[], old, "7");
    assert_eq!(focused, "one-focus-7");
    assert_eq!(
        ops[0],
        json!({"op": "split", "clip": "one", "at": 24, "tail": "one-focus-7"})
    );
    assert_eq!(ops[1]["op"], "set");
    assert_eq!(ops[1]["clip"], "one-focus-7");
    assert_eq!(ops[1]["props"]["crop_l"], 0.25);
    assert_eq!(
        ops[2],
        json!({"op": "split", "clip": "one-focus-7", "at": 72, "tail": "one-after-7"})
    );
    // After the range, the crop it had.
    assert_eq!(ops[3]["clip"], "one-after-7");
    assert_eq!(ops[3]["props"]["crop_l"], 0.1);
    // The whole clip: no split, the clip itself cropped.
    let (ops, focused) = focus_ops("one", (0, 96), (0, 96), region, &[], None, "7");
    assert_eq!(focused, "one");
    assert_eq!(ops.len(), 1);
}

#[test]
fn a_followed_subject_is_a_part_a_step() {
    let region = Region {
        x: 0.2,
        y: 0.2,
        width: 0.3,
        height: 0.3,
    };
    let moved = Region { x: 0.3, ..region };
    let tracked = [(0, region), (24, moved), (200, moved)];
    let (ops, focused) = focus_ops("one", (0, 96), (0, 96), region, &tracked, None, "s");
    // The step past the range is not split off.
    assert_eq!(focused, "one-track-1-s");
    assert_eq!(ops.len(), 3);
    assert_eq!(
        ops[1],
        json!({"op": "split", "clip": "one", "at": 24, "tail": "one-track-1-s"})
    );
    assert_eq!(ops[2]["props"]["crop_l"], 0.3);
}

#[test]
fn a_scene_cut_inside_the_range_is_where_the_box_ends() {
    // Media seconds 0..4 play at 2..6 of the program.
    let clip =
        clip(json!({"id": "one", "timelineStart": 2.0, "duration": 4.0, "in": 0.0, "out": 4.0}));
    assert_eq!(first_cut(&[1.5, 3.0], &clip, 2.0, 6.0, 24.0), Some(3.5));
    // Outside the range, or on its ends, none.
    assert_eq!(first_cut(&[0.0, 3.99], &clip, 2.0, 6.0, 24.0), None);
    assert_eq!(first_cut(&[], &clip, 2.0, 6.0, 24.0), None);
    // A clip played twice as fast reaches its cuts sooner.
    let fast = clip_twice();
    assert_eq!(first_cut(&[4.0], &fast, 0.0, 4.0, 24.0), Some(2.0));
}

fn clip_twice() -> Clip {
    clip(
        json!({"id": "fast", "timelineStart": 0.0, "duration": 4.0, "in": 0.0, "out": 8.0, "speed": 2.0}),
    )
}

#[test]
fn times_are_typed_as_seconds_frames_or_timecode() {
    assert_eq!(typed_time("1.500 s", 24.0), Some(1.5));
    assert_eq!(typed_time("2", 24.0), Some(2.0));
    assert_eq!(typed_time("48f", 24.0), Some(2.0));
    assert_eq!(typed_time("00:00:01:12", 24.0), Some(1.5));
    assert_eq!(typed_time("soon", 24.0), None);
    assert_eq!(typed_time(&shown_time(3.25), 24.0), Some(3.25));
}

/// Grey frames of a bright square on a dark ground, at `(x, y)` in each.
fn frames_of(places: &[(usize, usize)]) -> Vec<u8> {
    let (width, height) = TRACK_SIZE;
    let mut frames = Vec::new();
    for (x, y) in places {
        let mut frame = vec![20u8; width * height];
        for row in 0..20 {
            for column in 0..20 {
                // A pattern inside, so the square is not flat.
                frame[(y + row) * width + x + column] = if (row / 4 + column / 4) % 2 == 0 {
                    230
                } else {
                    120
                };
            }
        }
        frames.extend(frame);
    }
    frames
}

#[test]
fn a_subject_that_moves_is_followed_and_a_flat_box_is_not() {
    let (width, height) = TRACK_SIZE;
    let seed = Region {
        x: 40.0 / width as f64,
        y: 30.0 / height as f64,
        width: 20.0 / width as f64,
        height: 20.0 / height as f64,
    };
    let frames = frames_of(&[(40, 30), (44, 31), (49, 33), (55, 34)]);
    let tracked = track_region(&frames, TRACK_SIZE, seed, (0.0, 10.0), 24.0).unwrap();
    assert_eq!(tracked.reliable_end, 10.0);
    // The start, then each frame it moved in (two a second: 12 frames apart).
    let frames_at: Vec<i64> = tracked.boxes.iter().map(|(frame, _)| *frame).collect();
    assert_eq!(frames_at, [0, 12, 24, 36]);
    let last = tracked.boxes.last().unwrap().1;
    assert!((last.x * width as f64 - 55.0).abs() < 1.0, "{last:?}");
    assert!((last.y * height as f64 - 34.0).abs() < 1.0, "{last:?}");
    // A box with nothing in it cannot be followed; nor one frame.
    let flat = Region {
        x: 0.7,
        y: 0.7,
        width: 0.2,
        height: 0.2,
    };
    assert_eq!(
        track_region(&frames, TRACK_SIZE, flat, (0.0, 10.0), 24.0),
        None
    );
    assert_eq!(
        track_region(&frames_of(&[(40, 30)]), TRACK_SIZE, seed, (0.0, 10.0), 24.0),
        None
    );
    // Gone, it is lost where it went.
    let mut gone = frames_of(&[(40, 30), (42, 30)]);
    gone.extend(vec![20u8; width * height]);
    let lost = track_region(&gone, TRACK_SIZE, seed, (0.0, 10.0), 24.0).unwrap();
    assert_eq!(lost.reliable_end, 1.0);
}

// ------------------------------------------------------------ the picture

#[test]
fn the_transport_runs_from_start_to_end_and_says_why_it_cannot() {
    let ids: Vec<&str> = TRANSPORT.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(
        ids,
        [
            "playback.goToStart",
            "playback.previousFrame",
            "playback.toggle",
            "playback.nextFrame",
            "playback.goToEnd"
        ]
    );
    assert_eq!(transport_tip("Go to start", None), "Go to start");
    assert_eq!(
        transport_tip("Go to end", Some("Not connected")),
        "Go to end · Not connected"
    );
}

#[test]
fn the_chip_says_cutting_only_while_kerf_builds_a_first_cut() {
    assert!(kerf_cutting(true, false, true));
    // Looking at clips there already, or with nothing proposed: looking.
    assert!(!kerf_cutting(true, true, true));
    assert!(!kerf_cutting(true, false, false));
    assert!(!kerf_cutting(false, false, true));
}

fn raw_timeline(value: Value) -> Timeline {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_poster_stands_in_for_the_topmost_footage_at_the_playhead() {
    let cut = raw_timeline(json!({"tracks": [
        {"id": "V1", "type": "video", "clips": [
            {"id": "one", "timelineStart": 0.0, "duration": 4.0, "sourceId": "a"},
            {"id": "two", "timelineStart": 4.0, "duration": 3.0, "sourceId": "b"}]},
        {"id": "V2", "type": "video", "clips": [
            {"id": "title", "timelineStart": 1.0, "duration": 2.0},
            {"id": "over", "timelineStart": 5.0, "duration": 1.0, "sourceId": "c"}]},
        {"id": "A1", "type": "audio", "clips": [
            {"id": "sound", "timelineStart": 0.0, "duration": 7.0, "sourceId": "s"}]}
    ]}));
    // A title has no media of its own: the footage under it.
    assert_eq!(poster_source(&cut, 1.5).as_deref(), Some("a"));
    assert_eq!(poster_source(&cut, 5.5).as_deref(), Some("c"));
    assert_eq!(poster_source(&cut, 4.5).as_deref(), Some("b"));
    // Past the end, the first footage.
    assert_eq!(poster_source(&cut, 30.0).as_deref(), Some("a"));
    assert_eq!(
        poster_source(&raw_timeline(json!({"tracks": []})), 0.0),
        None
    );
}

#[test]
fn a_cut_built_shows_each_clip_as_it_is_placed() {
    let cut = raw_timeline(json!({"tracks": [
        {"id": "V1", "type": "video", "clips": [
            {"id": "late", "timelineStart": 3.0, "duration": 1.0, "sourceId": "b"},
            {"id": "early", "timelineStart": 0.0, "duration": 3.0, "sourceId": "a"}]}
    ]}));
    let mut seen = HashSet::new();
    assert_eq!(
        placed_clips(&seen, &cut),
        [
            ("early".to_owned(), "a".to_owned()),
            ("late".to_owned(), "b".to_owned())
        ]
    );
    seen.insert("early".to_owned());
    assert_eq!(
        placed_clips(&seen, &cut),
        [("late".to_owned(), "b".to_owned())]
    );
}

proptest! {
    #[test]
    fn base64_reads_back_what_was_written(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut text = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (at, byte)| n | u32::from(*byte) << (16 - 8 * at));
            for at in 0..4 {
                if at <= chunk.len() {
                    text.push(TABLE[(n >> (18 - 6 * at) & 63) as usize] as char);
                } else {
                    text.push('=');
                }
            }
        }
        prop_assert_eq!(base64_decode(&text), Some(bytes));
    }
}

// ------------------------------------------------------------ Focus Region's frame

#[test]
fn focus_region_needs_a_frame_of_the_clips_media() {
    let footage =
        clip(json!({"id": "a", "timelineStart": 10.0, "duration": 4.0, "in": 2.0, "out": 6.0}));
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("a.mov");
    std::fs::write(&file, b"not much").unwrap();
    // The frame at program second 11 is the media's second 3.
    let (read, seconds) = readable_frame(Some(file.clone()), &footage, 11.0).unwrap();
    assert_eq!(read, file);
    assert!((seconds - 3.0).abs() < 1e-9);
    // A title or a solid has no media; a file gone cannot be read.
    assert_eq!(readable_frame(None, &footage, 11.0), Err(NO_FRAME));
    assert_eq!(
        readable_frame(Some(folder.path().join("gone.mov")), &footage, 11.0),
        Err(NO_FRAME)
    );
    let title = clip(json!({"id": "t", "timelineStart": 0.0, "duration": 2.0}));
    assert_eq!(readable_frame(Some(file), &title, 1.0), Err(NO_FRAME));
}

#[test]
fn ffmpeg_is_looked_for_where_kerfd_looks() {
    // The order it is looked for in is kerf_engine's (its own tests hold
    // it); the editor runs the very one kerfd finds.
    assert_eq!(
        ffmpeg_bin(),
        std::path::Path::new(kerf_bridge::ffmpeg_bin())
    );
}

// ------------------------------------------------------------ the picture standing in, the transport

#[test]
fn the_cuts_render_coming_releases_the_poster_standing_in() {
    let preview = |status: &str, proposal: &str| json!({"type": "preview", "status": status, "proposalId": proposal});
    assert!(!releases_cover(&preview("running", "p2"), Some("p2")));
    assert!(releases_cover(&preview("ready", "p2"), Some("p2")));
    assert!(releases_cover(&preview("failed", "p2"), Some("p2")));
    // A render of a version before it, or with no cut being built: not yet.
    assert!(!releases_cover(&preview("ready", "p1"), Some("p2")));
    assert!(!releases_cover(&preview("ready", "p2"), None));
    assert!(!releases_cover(
        &json!({"type": "render", "status": "complete", "proposalId": "p2"}),
        Some("p2")
    ));
}

#[test]
fn the_transport_says_play_waits_for_a_preview() {
    let gone = transport_reasons(&json!({"connected": false}));
    assert_eq!(gone.len(), TRANSPORT.len());
    let connected = transport_reasons(&json!({"connected": true}));
    assert!(connected.is_empty(), "{connected:?}");
    let required = transport_reasons(&json!({"connected": true, "previewRequired": true}));
    assert_eq!(required.len(), 1);
    assert_eq!(
        required.get("playback.toggle").map(String::as_str),
        Some("Render a preview to play this timeline")
    );
}

// --------------------------------------------------------------- the preview

fn proposals() -> Snapshot {
    Snapshot::from_value(&json!({
        "timeline": {"fps": 24.0, "tracks": []},
        "proposals": [
            {"proposalId": "p3", "status": "pending", "version": 3,
             "versions": [{"version": 1, "proposalId": "p1"}, {"version": 2, "proposalId": "p2"},
                          {"version": 3, "proposalId": "p3"}]},
            {"proposalId": "q", "status": "pending", "version": 1}
        ]
    }))
}

#[test]
fn the_preview_chip_names_the_version_that_plays() {
    let snapshot = proposals();
    let proposals = &snapshot.proposals;
    assert_eq!(previewed_version(proposals, "p2"), Some(2));
    assert_eq!(previewed_version(proposals, "p3"), Some(3));
    // A proposal with no versions listed: its own.
    assert_eq!(previewed_version(proposals, "q"), Some(1));
    assert_eq!(previewed_version(proposals, "gone"), None);
    assert_eq!(preview_chip_words(Some(2)), "Previewing proposal v2");
    assert_eq!(preview_chip_words(None), "Previewing proposal");
}

#[test]
fn a_preview_to_render_says_why() {
    let required = |message: Option<&str>| Preview {
        status: "required".into(),
        message: message.map(str::to_owned),
        ..Preview::default()
    };
    assert_eq!(
        required(Some("Blur needs a render")).reason().as_deref(),
        Some("Blur needs a render")
    );
    // With no reason given, Qt's words.
    assert_eq!(
        required(None).reason().as_deref(),
        Some("Rendered preview required")
    );
    assert_eq!(
        required(Some("  ")).reason().as_deref(),
        Some("Rendered preview required")
    );
    let ready = Preview {
        status: "ready".into(),
        ..Preview::default()
    };
    assert_eq!(ready.reason(), None);
}
