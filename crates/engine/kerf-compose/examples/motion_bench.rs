//! How long the compositor takes to draw a frame of what it draws from numbers alone.
//!
//! ```text
//! cargo run --release -p kerf-compose --example motion_bench [word]
//! KERF_BENCH_GPU=1 cargo run --release -p kerf-compose --example motion_bench [word]
//! ```
//!
//! On the CPU unless told otherwise, whatever a machine has.

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
        "3d bevel title",
        "gen=title@1 at=0f dur=96f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1 bevel=0.02 rotate_y={0f:-20 96f:20}\n  track V2 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.4 y=0.3 z=1",
    ),
    (
        "3d bevel title shadow",
        "gen=solid@1 at=0f dur=96f\n      set color=#808080 three_d=true z=0.3\n  track V2 video\n    clip t gen=title@1 at=0f dur=96f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1 bevel=0.02\n  track V3 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4",
    ),
    (
        "3d round bevel title",
        "gen=title@1 at=0f dur=96f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1 bevel=0.02 bevel_profile=round rotate_y={0f:-20 96f:20}\n  track V2 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.4 y=0.3 z=1",
    ),
    (
        "3d round bevel title shadow",
        "gen=solid@1 at=0f dur=96f\n      set color=#808080 three_d=true z=0.3\n  track V2 video\n    clip t gen=title@1 at=0f dur=96f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1 bevel=0.02 bevel_profile=round\n  track V3 video\n    clip c2 gen=light@1 at=0f dur=96f\n      set kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4",
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

/// A solid `kind`, as large as `size` across, down, and back and with its middle at `place`,
/// both in frame heights, of a frame of 16 by 9.
fn solid(kind: &str, size: [f64; 3], place: [f64; 3], color: &str, more: &str) -> String {
    let [w, h, l] = size;
    let [x, y, z] = place;
    format!(
        "gen=mesh@1 at=0f dur=96f\n      set kind={kind} width={w:.3} height={h:.3} length={l:.3} color={color} x={:.4} y={y:.3} z={z:.3}{more}",
        x * 9.0 / 16.0
    )
}

/// Clips one over another, each on a track of its own, as a scene's first clip.
fn stacked(clips: &[String]) -> String {
    let mut scene = clips[0].clone();
    for (above, clip) in clips[1..].iter().enumerate() {
        let track = above + 2;
        scene += &format!("\n  track V{track} video\n    clip c{track} {clip}");
    }
    scene
}

/// The ground `solids` stand on, half a frame under its middle, with a light on them from
/// over the viewer's left shoulder and a little light all around, seen by a camera that moves.
fn lit(mut solids: Vec<String>, light: &str) -> String {
    let ground = "gen=solid@1 at=0f dur=96f\n      set color=#a09880 three_d=true rotate_x=-90 y=0.5 z=4 scale=16";
    solids.insert(0, ground.to_owned());
    solids.push(format!(
        "gen=light@1 at=0f dur=96f\n      set kind=directional x=0.35 y=1 z=0.45{light}"
    ));
    solids.push("gen=light@1 at=0f dur=96f\n      set kind=ambient intensity=0.35".to_owned());
    solids.push(
        "gen=camera@1 at=0f dur=96f\n      set y=-0.3 tilt=-9 z={0f:-0.3 96f:0.5} pan={0f:-5 96f:5}"
            .to_owned(),
    );
    stacked(&solids)
}

/// A box, a ball, and a cylinder standing beside each other.
fn few() -> Vec<String> {
    vec![
        solid(
            "box",
            [0.5, 0.5, 0.5],
            [-0.75, 0.25, 1.0],
            "#c04030",
            " rotate_y=30",
        ),
        solid("ball", [0.5, 0.5, 0.5], [0.0, 0.25, 0.8], "#3060c0", ""),
        solid(
            "cylinder",
            [0.35, 0.6, 0.35],
            [0.7, 0.2, 1.1],
            "#40a050",
            "",
        ),
    ]
}

/// A street of 53 solids, as a guide blocks one: seven houses, each a box with a roof and a
/// door, two carts of thirteen solids on cylinder wheels, and three people, each a cylinder
/// under a ball.
fn street() -> Vec<String> {
    let mut solids = Vec::new();
    let houses = [
        (-1.15, 0.4, "#c8b090"),
        (1.15, 1.0, "#b0b8c0"),
        (-1.15, 1.6, "#d0c0a0"),
        (1.15, 2.2, "#c0a890"),
        (-1.15, 2.8, "#a8b0a0"),
        (1.15, 3.4, "#d8c8b0"),
        (-1.15, 4.0, "#b8a8a0"),
    ];
    for (x, z, color) in houses {
        let toward_street = if x < 0.0 { 1.0 } else { -1.0 };
        solids.push(solid("box", [0.9, 0.8, 0.9], [x, 0.1, z], color, ""));
        solids.push(solid(
            "box",
            [0.64, 0.64, 0.9],
            [x, -0.3, z],
            "#804030",
            " rotation=45",
        ));
        let door = [x + toward_street * 0.47, 0.3, z];
        solids.push(solid("box", [0.04, 0.4, 0.2], door, "#503020", ""));
    }
    for (x, z) in [(-0.3, 1.1), (0.35, 2.5)] {
        let wood = "#906838";
        solids.push(solid("box", [0.5, 0.04, 0.3], [x, 0.34, z], wood, ""));
        for side in [-0.14, 0.14] {
            solids.push(solid(
                "box",
                [0.5, 0.1, 0.02],
                [x, 0.27, z + side],
                wood,
                "",
            ));
            solids.push(solid(
                "box",
                [0.3, 0.02, 0.02],
                [x + 0.4, 0.34, z + side * 0.7],
                wood,
                "",
            ));
        }
        for end in [-0.24, 0.24] {
            solids.push(solid("box", [0.02, 0.1, 0.3], [x + end, 0.27, z], wood, ""));
        }
        for axle in [-0.17, 0.17] {
            let turned = " rotate_x=90";
            solids.push(solid(
                "cylinder",
                [0.03, 0.36, 0.03],
                [x + axle, 0.38, z],
                "#403020",
                turned,
            ));
            for wheel in [-0.17, 0.17] {
                let place = [x + axle, 0.38, z + wheel];
                solids.push(solid(
                    "cylinder",
                    [0.24, 0.03, 0.24],
                    place,
                    "#604020",
                    turned,
                ));
            }
        }
    }
    for (x, z, color) in [
        (0.25, 0.7, "#3050a0"),
        (-0.2, 1.9, "#a03040"),
        (0.1, 3.2, "#308050"),
    ] {
        solids.push(solid(
            "cylinder",
            [0.12, 0.3, 0.12],
            [x, 0.35, z],
            color,
            "",
        ));
        solids.push(solid(
            "ball",
            [0.11, 0.11, 0.11],
            [x, 0.145, z],
            "#e0b090",
            "",
        ));
    }
    solids
}

fn main() {
    // A word on the command line picks the scenes with it in their names.
    let wanted = std::env::args().nth(1).unwrap_or_default();
    let accel = if std::env::var_os("KERF_BENCH_GPU").is_some() {
        Accel::Always
    } else {
        Accel::Never
    };
    println!("{:<18}{:>12}{:>12}", "scene", "1080p ms", "4K ms");
    let masks = [
        ("mask 40 hard", masked(|_| String::new(), "")),
        ("mask 40 feather", masked(|_| String::new(), " feather=24")),
        (
            "mask 40 soft",
            masked(|turn| format!(",{:.1}", 24.0 + 24.0 * turn.cos()), ""),
        ),
    ];
    // Solids, which are meshes: a few and a street of them, lit, and in the way of a light
    // that casts a hard shadow and of one that casts a soft one.
    let shadows = [
        ("lit", ""),
        ("shadow", " shadow=0.55"),
        ("soft", " shadow=0.55 size=2.5"),
    ];
    let meshes: Vec<(String, String)> = [("mesh", few()), ("street", street())]
        .into_iter()
        .flat_map(|(name, solids)| {
            shadows
                .map(|(shadow, light)| (format!("3d {name} {shadow}"), lit(solids.clone(), light)))
        })
        .collect();
    let scenes = SCENES
        .iter()
        .map(|(name, clip)| (*name, *clip))
        .chain(
            meshes
                .iter()
                .map(|(name, clip)| (name.as_str(), clip.as_str())),
        )
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
                accel,
                frame_folder: None,
                keep_frames: false,
                ..ComposeOptions::default()
            };
            let compositor =
                Compositor::new(&project, Catalog::builtin(), &options).expect("composes");
            let on_gpu = compositor.draws_on_gpu(Rate::FPS_24.time_of(40), 1.0);
            if accel == Accel::Always && !on_gpu {
                line += "         cpu";
                continue;
            }
            // Its first frame on the GPU builds what later ones keep.
            compositor.frame(Rate::FPS_24.time_of(39), 1.0);
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
