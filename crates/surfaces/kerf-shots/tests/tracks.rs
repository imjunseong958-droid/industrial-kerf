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
