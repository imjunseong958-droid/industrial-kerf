//! How long the compositor takes to draw a frame of what it draws from numbers alone.
//!
//! ```text
//! cargo run --release -p kerf-compose --example motion_bench [word]
//! ```

use kerf_compose::{Accel, ComposeOptions, Compositor};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::time::Instant;

const SCENES: &[(&str, &str)] = &[
    ("rect", "gen=shape@1 at=0f dur=96f\n      set width=1 height=0.5"),
    (
        "ellipse stroke",
        "gen=shape@1 at=0f dur=96f\n      set kind=ellipse width=0.9 height=0.9 fill=#ffffff00 stroke_width=0.02 trim_end={0f:0 48f:1}",
    ),
    (
        "star",
        "gen=shape@1 at=0f dur=96f\n      set kind=star sides=7 width=0.9 height=0.9 stroke_width=0.01 stroke=#ff0000",
    ),
    (
        "rect minus circle",
        "gen=shape@1 at=0f dur=96f\n      set width=1 height=0.8\n      fx shape.outline@1 mode=subtract kind=ellipse width=0.5 height=0.5 x={0f:-0.3 96f:0.3}",
    ),
    (
        "three outlines",
        "gen=shape@1 at=0f dur=96f\n      set kind=star sides=9 width=0.9 height=0.9 stroke_width=0.005\n      fx shape.outline@1 kind=ellipse width=0.5 height=0.5 x=0.3\n      fx shape.outline@1 mode=subtract kind=ellipse width=0.3 height=0.3 x={0f:-0.3 96f:0.3}",
    ),
    (
        "offset ellipse",
        "gen=shape@1 at=0f dur=96f\n      set kind=ellipse width=0.5 height=0.5 offset={0f:0 96f:0.2}",
    ),
    (
        "offset star",
        "gen=shape@1 at=0f dur=96f\n      set kind=star sides=7 width=0.6 height=0.6 offset={0f:-0.02 96f:0.05} stroke_width=0.005",
    ),
    (
        "tapered stroke",
        "gen=shape@1 at=0f dur=96f\n      set kind=ellipse width=0.9 height=0.9 fill=#ffffff00 stroke_width=0.02 trim_end={0f:0 48f:1} taper_start=0.5 taper_end=0.5",
    ),
    (
        "5000 particles",
        "gen=particles@1 at=0f dur=96f\n      set rate=2500 life=2 velocity=0.5 spread=360 size=0.006",
    ),
    (
        "40000 particles",
        "gen=particles@1 at=0f dur=96f\n      set rate=20000 life=2 velocity=0.5 spread=360 size=0.004",
    ),
    (
        "title",
        r#"gen=title@1 at=0f dur=96f
      set text="A line of words" font="Skia" size=0.08"#,
    ),
    (
        "weight moving",
        r#"gen=title@1 at=0f dur=96f
      set text="A line of words" font="Skia" size=0.08 wght={0f:0.5 96f:2.5}"#,
    ),
    (
        "title reveal",
        r#"gen=title@1 at=0f dur=96f
      set text="A line of words that comes in" size=0.08 reveal={0f:0 48f:1} from_y=0.05 from_rotation=20"#,
    ),
    (
        "3d apart",
        "gen=solid@1 at=0f dur=96f\n      set color=#ff0000 three_d=true scale=0.6 z=0.2 rotate_y=20\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.4 z=-0.3 rotate_y=-20",
    ),
    (
        "3d crossing",
        "gen=solid@1 at=0f dur=96f\n      set color=#ff0000 three_d=true scale=0.6 z=0.1 rotate_y=45\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.6 rotate_y=-30",
    ),
    (
        "3d lit",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.3 z=-0.3\n  track V3 video\n    clip c3 gen=light@1 at=0f dur=96f\n      set kind=directional x=0.2 z=1",
    ),
    (
        "3d shadow",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.3 z=-0.3\n  track V3 video\n    clip c3 gen=light@1 at=0f dur=96f\n      set kind=directional x=0.2 z=1 shadow=1",
    ),
    (
        "3d point shadow",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.3 z=-0.3\n  track V3 video\n    clip c3 gen=light@1 at=0f dur=96f\n      set x=0.2 y=-0.1 z=-0.6 shadow=1",
    ),
    (
        "3d soft",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.3 z=-0.3\n  track V3 video\n    clip c3 gen=light@1 at=0f dur=96f\n      set kind=directional x=0.2 z=1 shadow=1 size=5",
    ),
    (
        "3d point soft",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.3 z=-0.3\n  track V3 video\n    clip c3 gen=light@1 at=0f dur=96f\n      set x=0.2 y=-0.1 z=-0.6 shadow=1 size=0.1",
    ),
    (
        "3d soft all over",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true z=-0.3\n  track V3 video\n    clip c3 gen=light@1 at=0f dur=96f\n      set kind=directional z=1 shadow=1 size=30",
    ),
    (
        "3d soft title",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip c2 gen=title@1 at=0f dur=96f\n      set text=\"Soft\" size=0.4 three_d=true z=-0.2\n  track V3 video\n    clip c3 gen=light@1 at=0f dur=96f\n      set kind=directional x=0.2 z=1 shadow=1 size=10",
    ),
    (
        "3d deep square",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true scale=0.3 x=-0.2 depth=0.3\n  track V2 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.3 z=1",
    ),
    (
        "3d deep title",
        "gen=title@1 at=0f dur=96f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1 rotate_y={0f:-20 96f:20}\n  track V2 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.4 y=0.3 z=1",
    ),
    (
        "3d title flat",
        "gen=title@1 at=0f dur=96f\n      set text=\"Depth\" size=0.3 three_d=true rotate_y={0f:-20 96f:20}\n  track V2 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.4 y=0.3 z=1",
    ),
    (
        "3d deep title shadow",
        "gen=solid@1 at=0f dur=96f\n      set color=#808080 three_d=true z=0.3\n  track V2 video\n    clip t gen=title@1 at=0f dur=96f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1\n  track V3 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4",
    ),
    (
        "3d focus flat",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true z=0.5 scale=0.6\n  track V2 video\n    clip c2 gen=camera@1 at=0f dur=96f\n      set aperture=0.03",
    ),
    (
        "3d focus wide",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true z=2 scale=0.6\n  track V2 video\n    clip c2 gen=camera@1 at=0f dur=96f\n      set aperture=0.1",
    ),
    (
        "3d focus turned",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true rotate_y=60 scale=0.6\n  track V2 video\n    clip c2 gen=camera@1 at=0f dur=96f\n      set aperture=0.05",
    ),
    (
        "3d focus two",
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff three_d=true z=0.6\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=96f\n      set color=#0000ff three_d=true scale=0.3 z=-0.2\n  track V3 video\n    clip c3 gen=camera@1 at=0f dur=96f\n      set aperture=0.05",
    ),
];

/// A mask with as many corners as following an object gives, around the middle of the frame, with
/// what `corner` says at each of them.
fn masked(corner: impl Fn(f64) -> String, rest: &str) -> String {
    let corners: Vec<String> = (0..40)
        .map(|at| {
            let turn = f64::from(at) / 40.0 * std::f64::consts::TAU;
            let (x, y) = (0.5 + 0.3 * turn.cos(), 0.5 + 0.4 * turn.sin());
            format!("{x:.4},{y:.4}{}", corner(turn))
        })
        .collect();
    format!(
        "gen=solid@1 at=0f dur=96f\n      set color=#ffffff\n      mask m points=\"{}\"{rest}",
        corners.join(" ")
    )
}

fn main() {
    // A word on the command line picks the scenes with it in their names.
    let wanted = std::env::args().nth(1).unwrap_or_default();
    println!("{:<18}{:>12}{:>12}", "scene", "1080p ms", "4K ms");
    let masks = [
        ("mask 40 hard", masked(|_| String::new(), "")),
        ("mask 40 feather", masked(|_| String::new(), " feather=24")),
        (
            "mask 40 soft",
            masked(|turn| format!(",{:.1}", 24.0 + 24.0 * turn.cos()), ""),
        ),
    ];
    let scenes = SCENES
        .iter()
        .map(|(name, clip)| (*name, *clip))
        .chain(masks.iter().map(|(name, clip)| (*name, clip.as_str())));
    for (name, clip) in scenes.filter(|(name, _)| name.contains(&wanted)) {
        let mut line = format!("{name:<18}");
        for (w, h) in [(1920, 1080), (3840, 2160)] {
            let text = format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size={w}x{h} audio=48000\n  track V1 video\n    clip c1 {clip}\n"
            );
            let project = parse_document(&text, Catalog::builtin())
                .unwrap_or_else(|errors| panic!("{errors:?}"))
                .project;
            let options = ComposeOptions {
                accel: Accel::Never,
                frame_folder: None,
                ..ComposeOptions::default()
            };
            let compositor =
                Compositor::new(&project, Catalog::builtin(), &options).expect("composes");
            let mut times: Vec<f64> = (0..9)
                .map(|step| {
                    let begun = Instant::now();
                    compositor.frame(Rate::FPS_24.time_of(40 + step), 1.0);
                    begun.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            times.sort_by(f64::total_cmp);
            line += &format!("{:>12.1}", times[4]);
        }
        println!("{line}");
    }
}
