//! A clip of a sequence showing one of its tracks, as a cut of a multicam shows one camera.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project, Rate};

const DOCUMENT: &str = "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip cam seq=mc src=0f at=0f dur=12f angle=V1\n    clip cam2 seq=mc src=12f at=12f dur=12f angle=V2\n    clip cam3 seq=mc src=0f at=24f dur=12f angle=V3\n    clip all seq=mc src=0f at=36f dur=12f\nsequence mc rate=24 size=64x36 audio=48000\n  track V1 video\n    clip a gen=solid@1 at=0f dur=48f\n      set color=#ff0000\n  track V2 video\n    clip b gen=solid@1 at=0f dur=48f\n      set color=#00ff00\n  track V3 video enabled=false\n    clip c gen=solid@1 at=0f dur=48f\n      set color=#0000ff\n";

fn project() -> Project {
    parse_document(DOCUMENT, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

fn compositor(project: &Project, accel: Accel) -> Compositor {
    Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            accel,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

fn pixel(frame: &Frame) -> [u8; 3] {
    let at = ((18 * frame.width + 32) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

#[test]
fn each_cut_shows_its_own_camera() {
    let at = |frame: i64| Rate::FPS_24.time_of(frame);
    let project = project();
    for accel in [Accel::Never, Accel::Always] {
        let drawn = compositor(&project, accel);
        let color = |frame: i64| pixel(&drawn.frame(at(frame), 1.0));
        assert_eq!(color(3), [255, 0, 0], "{accel:?}");
        assert_eq!(color(15), [0, 255, 0], "{accel:?}");
        // A camera on a hidden track shows when it is asked for.
        assert_eq!(color(27), [0, 0, 255], "{accel:?}");
        // Without an angle the sequence shows as ever: the top track that shows.
        assert_eq!(color(38), [0, 255, 0], "{accel:?}");
    }
}

#[test]
fn switching_an_angle_is_an_edit_and_splitting_cuts_between_them() {
    let project = apply_script(
        &project(),
        Catalog::builtin(),
        "split cam at=6f as=late\nset late angle=V2",
    )
    .expect("edits")
    .project;
    let drawn = compositor(&project, Accel::Never);
    let color = |frame: i64| pixel(&drawn.frame(Rate::FPS_24.time_of(frame), 1.0));
    assert_eq!(color(3), [255, 0, 0]);
    assert_eq!(color(8), [0, 255, 0]);
    let cleared = apply_script(&project, Catalog::builtin(), "set late angle=\"\"")
        .expect("edits")
        .project;
    assert_eq!(cleared.clip("late").expect("the clip").angle, None);
}

#[test]
fn a_frame_depends_on_the_camera_it_shows_and_no_other() {
    let original = project();
    let at = Rate::FPS_24.time_of(3);
    let key = |project: &Project| compositor(project, Accel::Never).frame_key(at, 1.0);
    let other = apply_script(&original, Catalog::builtin(), "set b color=#123456")
        .expect("edits")
        .project;
    assert_eq!(
        key(&original),
        key(&other),
        "another camera changed nothing"
    );
    let same = apply_script(&original, Catalog::builtin(), "set a color=#123456")
        .expect("edits")
        .project;
    assert_ne!(key(&original), key(&same), "this camera changed");
    let switched = apply_script(&original, Catalog::builtin(), "set cam angle=V2")
        .expect("edits")
        .project;
    assert_ne!(
        key(&original),
        key(&switched),
        "another camera is another frame"
    );
}
