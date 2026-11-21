//! The picture in depth and the making followed in words, through the
//! stage's public functions: where a clip in depth lands as the compositor
//! draws it (the Qt editor's table, which kerf-compose's draw.rs holds the
//! compositor to), a path no one map shows sampled and a point of it solved
//! for, and what the chip over the picture says while a shot is made.

use kerf_editor::stage::chip;
use kerf_editor::stage::overlay::{
    parse_path, path_text, placement, sampled, sampled_point_for, MotionPath, Projective, Pt,
    SAMPLED_STEPS,
};
use kerf_editor::store::Making;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

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
