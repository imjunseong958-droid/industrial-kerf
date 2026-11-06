//! Clips that move with another: rigs under a null, chains, and what editing them does.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::{apply_script, parse_document, print_document, PrintOptions};
use kerf_model::{Catalog, Project, Rate};

/// A 640x360 sequence: `body` on top of a black ground, each bit of it a small square to find.
fn project(body: &str) -> Project {
    let document =
        format!("kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=640x360 audio=48000\n{body}");
    parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

fn frame(project: &Project, at: i64, accel: Accel) -> Frame {
    Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            accel,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .frame(Rate::FPS_24.time_of(at), 1.0)
}

/// The center and size of what is bright in a frame.
fn bright(frame: &Frame) -> (f64, f64, f64, f64) {
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..frame.height {
        for x in 0..frame.width {
            if frame.rgba[((y * frame.width + x) * 4) as usize] > 128 {
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    assert!(left != u32::MAX, "nothing bright");
    (
        f64::from(left + right) / 2.0,
        f64::from(top + bottom) / 2.0,
        f64::from(right - left + 1),
        f64::from(bottom - top + 1),
    )
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() <= 2.5, "{a} against {b}");
}

/// A white square 1/10 of the frame, a quarter of the way across from the center.
const SQUARE: &str =
    "    clip c1 gen=solid@1 at=0f dur=48f parent=n1\n      set color=#ffffff scale=0.1 x=0.25\n";

fn rig(null: &str) -> String {
    format!(
        "  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n{null}  track V2 video\n{SQUARE}"
    )
}

#[test]
fn a_null_draws_nothing_and_a_still_parent_changes_nothing() {
    let drawn = frame(&project(&rig("")), 0, Accel::Never);
    let (x, y, w, h) = bright(&drawn);
    close(x, 320.0 + 160.0);
    close(y, 180.0);
    close(w, 64.0);
    close(h, 36.0);
}

#[test]
fn a_parents_move_carries_the_child() {
    let drawn = frame(&project(&rig("      set x=-0.25 y=0.1\n")), 0, Accel::Never);
    let (x, y, _, _) = bright(&drawn);
    close(x, 320.0);
    close(y, 180.0 + 36.0);
}

/// A rig whose square sits a tenth of the way across, so that it stays in the frame.
fn near(null: &str) -> Project {
    project(&format!(
        "  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n{null}  track V2 video\n    clip c1 gen=solid@1 at=0f dur=48f parent=n1\n      set color=#ffffff scale=0.1 x=0.1\n"
    ))
}

#[test]
fn a_parents_scale_scales_the_child_and_its_distance() {
    let (x, y, w, h) = bright(&frame(&near("      set scale=2\n"), 0, Accel::Never));
    close(x, 320.0 + 128.0);
    close(y, 180.0);
    close(w, 128.0);
    close(h, 72.0);
}

#[test]
fn a_parents_rotation_swings_the_child_around_it() {
    let (x, y, w, h) = bright(&frame(&near("      set rotation=90\n"), 0, Accel::Never));
    // A quarter turn: from right of the center to below it, and turned with it.
    close(x, 320.0);
    close(y, 180.0 + 64.0);
    close(w, 36.0);
    close(h, 64.0);
}

#[test]
fn animation_on_the_parent_is_followed_on_its_own_clock() {
    // The null starts earlier: at frame 24 on the sequence it is 24 frames into its move.
    let body = "  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n      set x={0f:0 24f:-0.25}\n  track V2 video\n    clip c1 gen=solid@1 at=12f dur=36f parent=n1\n      set color=#ffffff scale=0.1 x=0.25\n";
    let project = project(body);
    close(bright(&frame(&project, 12, Accel::Never)).0, 480.0 - 80.0);
    close(bright(&frame(&project, 24, Accel::Never)).0, 320.0);
}

#[test]
fn parents_chain() {
    let body = "  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n      set x=0.1\n    \n  track V2 video\n    clip n2 gen=null@1 at=0f dur=48f parent=n1\n      set y=0.1\n  track V3 video\n    clip c1 gen=solid@1 at=0f dur=48f parent=n2\n      set color=#ffffff scale=0.1\n";
    let (x, y, _, _) = bright(&frame(&project(body), 0, Accel::Never));
    close(x, 320.0 + 64.0);
    close(y, 180.0 + 36.0);
}

#[test]
fn scripts_set_and_let_go_of_a_parent() {
    let base = project("  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n      set x=-0.25\n  track V2 video\n    clip c1 gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale=0.1 x=0.25\n");
    let catalog = Catalog::builtin();
    close(bright(&frame(&base, 0, Accel::Never)).0, 480.0);
    let on = apply_script(&base, catalog, "set c1 parent=n1")
        .expect("applies")
        .project;
    close(bright(&frame(&on, 0, Accel::Never)).0, 320.0);
    let printed = print_document(&on, catalog, PrintOptions::default());
    assert!(printed.contains("parent=n1"), "{printed}");
    let again = parse_document(&printed, catalog)
        .expect("reads back")
        .project;
    assert_eq!(again, on);
    let off = apply_script(&on, catalog, "set c1 parent=\"\"")
        .expect("applies")
        .project;
    close(bright(&frame(&off, 0, Accel::Never)).0, 480.0);
    for refused in [
        "set c1 parent=c9",
        "set c1 parent=c1",
        "set n1 parent=c1\nset c1 parent=n1",
    ] {
        assert!(apply_script(&base, catalog, refused).is_err(), "{refused}");
    }
    let gone = apply_script(&on, catalog, "delete n1")
        .expect("applies")
        .project;
    let clip = gone.sequences[0].tracks[1].clip("c1").expect("still there");
    assert_eq!(clip.parent, None);
}

#[test]
fn a_document_cannot_hold_a_loop_or_a_stranger() {
    for body in [
        "  track V1 video\n    clip a gen=null@1 at=0f dur=4f parent=b\n    clip b gen=null@1 at=4f dur=4f parent=a\n",
        "  track V1 video\n    clip a gen=null@1 at=0f dur=4f parent=a\n",
        "  track V1 video\n    clip a gen=null@1 at=0f dur=4f parent=nobody\n",
    ] {
        let document = format!("kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n{body}");
        assert!(parse_document(&document, Catalog::builtin()).is_err(), "{body}");
    }
}

#[test]
fn moving_a_parent_changes_its_childrens_frames_and_no_others() {
    let keys = |null: &str, at: i64| {
        Compositor::new(
            &project(&rig(null)),
            Catalog::builtin(),
            &ComposeOptions::default(),
        )
        .expect("composes")
        .frame_key(Rate::FPS_24.time_of(at), 1.0)
    };
    assert_ne!(keys("", 6), keys("      set x=0.1\n", 6));
    let lone = "  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#336699\n";
    let with =
        |x: &str| format!("{lone}  track V2 video\n    clip n1 gen=null@1 at=24f dur=24f\n{x}");
    let key = |body: String| {
        Compositor::new(
            &project(&body),
            Catalog::builtin(),
            &ComposeOptions::default(),
        )
        .expect("composes")
        .frame_key(Rate::FPS_24.time_of(6), 1.0)
    };
    assert_eq!(key(with("")), key(with("      set x=0.1\n")));
}

#[test]
fn the_gpu_draws_a_rig_as_the_cpu_does() {
    for null in [
        "      set x=-0.2 y=0.1 rotation=25 scale=1.5\n",
        "      set rotation={0f:0 24f:60}\n",
    ] {
        let project = project(&rig(null));
        let compositor = Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                accel: Accel::Always,
                ..ComposeOptions::default()
            },
        )
        .expect("composes");
        if !compositor.draws_on_gpu(Rate::FPS_24.time_of(6), 1.0) {
            eprintln!("no GPU: nothing to compare");
            return;
        }
        let (a, b) = (
            frame(&project, 6, Accel::Always),
            frame(&project, 6, Accel::Never),
        );
        let worst = a
            .rgba
            .iter()
            .zip(&b.rgba)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(worst <= 3, "{null}: differs by {worst}");
    }
}

/// A small square moving along `path`.
fn on_path(path: &str, along: &str) -> Project {
    project(&format!(
        "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale=0.1 path=\"{path}\" along={along}\n"
    ))
}

#[test]
fn a_path_moves_a_clip_along_it_at_an_even_speed() {
    let project = on_path("-0.25,0 0,0.25 0.25,0", "{0f:0 24f:1}");
    let at = |frame_at| bright(&frame(&project, frame_at, Accel::Never));
    let (start, middle, end) = (at(0), at(12), at(24));
    close(start.0, 320.0 - 160.0);
    close(start.1, 180.0);
    close(middle.0, 320.0);
    close(middle.1, 180.0 + 90.0);
    close(end.0, 320.0 + 160.0);
    close(end.1, 180.0);
    // It held its last place after that.
    close(at(40).0, 320.0 + 160.0);
}

#[test]
fn x_and_y_add_to_a_path() {
    let body = "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale=0.1 path=\"0,0 0.2,0\" along=1 x=0.1 y=0.1\n";
    let (x, y, _, _) = bright(&frame(&project(body), 0, Accel::Never));
    close(x, 320.0 + 192.0);
    close(y, 180.0 + 36.0);
}

#[test]
fn orient_turns_a_clip_to_face_its_way() {
    let body = |orient: &str| {
        format!(
            "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale_x=0.2 scale_y=0.02 path=\"0,-0.3 0,0.3\" along=0.5{orient}\n"
        )
    };
    // Heading straight down: the long thin bar stands up.
    let (_, _, flat_w, flat_h) = bright(&frame(&project(&body("")), 0, Accel::Never));
    let (_, _, up_w, up_h) = bright(&frame(&project(&body(" orient=true")), 0, Accel::Never));
    assert!(flat_w > flat_h * 2.0, "{flat_w} by {flat_h}");
    assert!(up_h > up_w * 2.0, "{up_w} by {up_h}");
}

#[test]
fn a_bad_path_is_refused_and_a_parent_on_a_path_carries_its_children() {
    let document = "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=4f\n      set path=\"0,0\"\n";
    assert!(parse_document(document, Catalog::builtin()).is_err());
    let rigged = "  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n      set path=\"0,0 0.25,0\" along=1\n  track V2 video\n    clip c1 gen=solid@1 at=0f dur=48f parent=n1\n      set color=#ffffff scale=0.1\n";
    close(
        bright(&frame(&project(rigged), 0, Accel::Never)).0,
        320.0 + 160.0,
    );
}

#[test]
fn the_gpu_draws_a_path_as_the_cpu_does() {
    let project = on_path("-0.3,-0.1 0,0.2 0.3,-0.1", "{0f:0 24f:1:ease}");
    let compositor = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Always,
            ..ComposeOptions::default()
        },
    )
    .expect("composes");
    if !compositor.draws_on_gpu(Rate::FPS_24.time_of(9), 1.0) {
        eprintln!("no GPU: nothing to compare");
        return;
    }
    let (a, b) = (
        frame(&project, 9, Accel::Always),
        frame(&project, 9, Accel::Never),
    );
    let worst = a
        .rgba
        .iter()
        .zip(&b.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(worst <= 3, "differs by {worst}");
}
