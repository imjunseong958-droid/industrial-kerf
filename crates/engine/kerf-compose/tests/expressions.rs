//! Parameters written as expressions of the clip's time: `rotation=(t*90)`.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::{apply_script, parse_document, print_document, PrintOptions};
use kerf_model::{Catalog, Project, Rate};

/// A 320x180 sequence with a white bar over black, `set` on the bar.
fn project(set: &str) -> Project {
    let document = format!(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=48f\n      set color=#000000\n  track V2 video\n    clip g2 gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale=0.4 crop_t=0.4 crop_b=0.4 {set}\n"
    );
    parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

fn frame(project: &Project, at: i64) -> Frame {
    Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Never,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .frame(Rate::FPS_24.time_of(at), 1.0)
}

#[test]
fn an_expression_draws_what_its_number_would_at_each_frame() {
    // Two seconds of `t*90` and `t*0.5` are what keyframes from 0 to 180 and from 0 to 1 are.
    let moving = project("rotation=(t*90) x=(t*0.5)");
    let keyed = project("rotation={0f:0 48f:180} x={0f:0 48f:1}");
    for at in [0, 7, 12, 30] {
        assert_eq!(
            frame(&moving, at).rgba,
            frame(&keyed, at).rgba,
            "frame {at}"
        );
    }
    assert_ne!(frame(&moving, 12).rgba, frame(&moving, 13).rgba);
}

#[test]
fn a_clip_cut_in_two_goes_on_as_it_was() {
    let whole = project("rotation=(t*90) opacity=(0.5+0.5*sin(t*pi))");
    let cut = apply_script(&whole, Catalog::builtin(), "split g2 at=20f as=g3")
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    for at in [0, 19, 20, 21, 40] {
        assert_eq!(frame(&whole, at).rgba, frame(&cut, at).rgba, "frame {at}");
    }
    // The second half says where its time starts, and reads back the same.
    let printed = print_document(&cut, Catalog::builtin(), PrintOptions::default());
    assert!(
        printed.contains("rotation=((t+0.8333333333333334)*90)"),
        "{printed}"
    );
    let read = parse_document(&printed, Catalog::builtin())
        .unwrap()
        .project;
    assert_eq!(frame(&read, 30).rgba, frame(&whole, 30).rgba);
}

#[test]
fn an_expression_stays_inside_what_its_parameter_allows() {
    // Opacity goes from 0 to 1, whatever the expression says.
    let over = project("opacity=(t*100)");
    let full = project("opacity=1");
    assert_eq!(frame(&over, 24).rgba, frame(&full, 24).rgba);
}
