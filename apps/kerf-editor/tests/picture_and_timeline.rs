//! The picture's overlays: where a clip, its masks and its motion path land,
//! held to the compositor's and kerf_model's own tables.

use kerf_editor::stage::overlay::{
    canvas_of, mask_set, masks_of, parent_motion, parse_path, path_holds, path_text, placement,
    property_at, start_path, with_point_on, Affine, MotionPath, Outline,
};
use proptest::prelude::*;
use serde_json::json;

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

    // What draws no picture, and what is in depth, has nothing to place.
    for id in ["s", "c", "deep", "nowhere"] {
        assert!(placement(&timeline, id, 1.0).is_none(), "{id}");
    }
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
    for id in ["b", "s", "c", "deep"] {
        assert_eq!(start_path(&timeline, id, 1.0), None, "{id}");
    }
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
