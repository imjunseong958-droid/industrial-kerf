//! A clip read from footage ([`kerf_shots::tracks`]), by whoever says when its frames may be
//! drawn.

use kerf_shots::tracks;
use std::cell::Cell;

/// A clip over a block that fills the middle of the frame, for twelve frames. A point holds
/// to a corner of the block, and has nothing to hold to in its middle.
fn project() -> kerf_model::Project {
    kerf_runtime::parse_document(
        "kerf 1\nproject name=\"Tracks\"\n\
         sequence s1 rate=24 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip block gen=solid@1 at=0f dur=12f\n\
         \x20     set color=#e08030 scale=0.3\n\
         \x20 track V2 video\n\
         \x20   clip follower gen=solid@1 at=0f dur=12f\n\
         \x20     set color=#30a0e0 scale=0.05\n",
    )
    .expect("the document")
}

#[test]
fn a_clip_is_followed_only_while_its_frames_may_be_drawn() {
    let project = project();
    let (corner, middle) = ([0.35, 0.35], [0.5, 0.5]);
    // Every frame let be drawn, the clip is followed all its way: by the point where there
    // is something to hold to, and by the thing the point is on where there is not.
    for (point, as_a_thing) in [(corner, false), (middle, true)] {
        let read = tracks::moves(&project, "follower", point, || Some(()))
            .expect("followed")
            .expect("nothing kept a frame from being drawn");
        assert_eq!(
            (read.frames, read.lost_at, read.as_a_thing),
            (12, None, as_a_thing)
        );
    }
    // Three frames in, no more may be: none is asked for after, and a clip followed that
    // far is no read of it, either way.
    for point in [corner, middle] {
        let asked = Cell::new(0);
        let three = || {
            asked.set(asked.get() + 1);
            (asked.get() <= 3).then_some(())
        };
        assert_eq!(tracks::moves(&project, "follower", point, three), Ok(None));
        assert_eq!(asked.get(), 4);
    }
    // The same of what is cut out of it.
    let none = || None::<()>;
    let cut = tracks::matte(&project, "follower", &[middle], &[], none);
    assert_eq!(cut, Ok(None));
}

/// A white clip put in depth by [`tracks::in_depth`] lands on the corners it was given, as the
/// compositor draws it: just inside each corner and the middle of each side is white, and just
/// outside is the black under it.
#[test]
fn a_clip_put_in_depth_on_a_surface_lands_on_its_corners() {
    let (wide, high) = (320.0, 180.0);
    let quads = [
        // Square on, as a corner pin of the frame would be.
        [[0.3, 0.3], [0.7, 0.3], [0.7, 0.7], [0.3, 0.7]],
        // Leaning back, narrower at its top.
        [[0.35, 0.3], [0.65, 0.3], [0.75, 0.75], [0.25, 0.75]],
        // Turned every way, a wall seen from below and to the side.
        [[0.2, 0.35], [0.6, 0.2], [0.75, 0.6], [0.3, 0.8]],
    ];
    for quad in quads {
        let depth = tracks::in_depth(quad, (wide, high), (wide, high)).expect("a plane");
        let document = format!(
            "kerf 1\nproject name=\"Pinned\"\n\
             sequence s1 rate=24 size=320x180 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip under gen=solid@1 at=0f dur=12f\n\
             \x20     set color=#000000\n\
             \x20 track V2 video\n\
             \x20   clip wall gen=solid@1 at=0f dur=12f\n\
             \x20     set color=#ffffff three_d=true x={} y={} rotation={} rotate_x={} rotate_y={} skew={} scale_x={} scale_y={}\n",
            depth.x,
            depth.y,
            depth.rotation,
            depth.rotate_x,
            depth.rotate_y,
            depth.skew,
            depth.scale_x,
            depth.scale_y
        );
        let project = kerf_runtime::parse_document(&document).expect("the document");
        let frame = kerf_engine::render::still::drawer(&project, Some("s1"))
            .expect("a drawer")
            .frame(kerf_model::Rate::FPS_24.time_of(3), 1.0);
        let white = |[x, y]: [f64; 2]| {
            let at = ((y as u32 * frame.width + x as u32) * 4) as usize;
            frame.rgba[at]
        };
        let middle = [0, 1].map(|axis| quad.iter().map(|corner| corner[axis]).sum::<f64>() / 4.0);
        // Each corner, and the middle of each side.
        let edges = (0..4).flat_map(|index| {
            let (from, to) = (quad[index], quad[(index + 1) % 4]);
            [from, [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0]]
        });
        for on in edges {
            let way = [middle[0] - on[0], middle[1] - on[1]];
            let away = [way[0] * wide, way[1] * high];
            let long = away[0].hypot(away[1]);
            let step = |by: f64| {
                [
                    on[0] * wide + away[0] / long * by,
                    on[1] * high + away[1] / long * by,
                ]
            };
            assert!(white(step(4.0)) > 200, "{quad:?} {depth:?}: inside {on:?}");
            assert!(white(step(-4.0)) < 50, "{quad:?} {depth:?}: outside {on:?}");
        }
    }
}
