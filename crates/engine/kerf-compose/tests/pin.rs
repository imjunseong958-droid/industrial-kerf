//! A corner pin from tracked keyframes lands a clip on the surface at each moment.

use kerf_compose::{Accel, ComposeOptions, Compositor};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

#[test]
fn keyframed_corners_put_the_clip_where_they_say_at_each_moment() {
    let document = "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n  track V1 video\n    clip g gen=solid@1 at=0f dur=24f\n      set color=#000000\n  track V2 video\n    clip sq gen=solid@1 at=0f dur=24f\n      set color=#ffffff\n      fx distort.cornerpin@2 upper_left_x={0f:0.1 12f:0.5} upper_left_y=0.2 upper_right_x={0f:0.3 12f:0.7} upper_right_y=0.2 lower_right_x={0f:0.3 12f:0.7} lower_right_y=0.8 lower_left_x={0f:0.1 12f:0.5} lower_left_y=0.8\n";
    let project = parse_document(document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let compositor = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Never,
            ..ComposeOptions::default()
        },
    )
    .expect("composes");
    let bright = |frame: i64, x: f64, y: f64| {
        let drawn = compositor.frame(Rate::FPS_24.time_of(frame), 1.0);
        let at = (y * 90.0) as usize * 160 + (x * 160.0) as usize;
        drawn.rgba[at * 4]
    };
    assert!(bright(0, 0.2, 0.5) > 240);
    assert!(bright(0, 0.6, 0.5) < 15);
    assert!(bright(12, 0.2, 0.5) < 15);
    assert!(bright(12, 0.6, 0.5) > 240);
    // Half way, it is half way.
    assert!(bright(6, 0.4, 0.5) > 240);
    assert!(bright(6, 0.2, 0.5) < 15);
}
