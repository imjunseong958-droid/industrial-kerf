//! Shapes, particles, and titles that come in, as a frame draws them.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

const WIDE: u32 = 320;
const TALL: u32 = 180;

fn frame_with(body: &str, at: i64, accel: Accel) -> Frame {
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size={WIDE}x{TALL} audio=48000\n  track V1 video\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .frame(Rate::FPS_24.time_of(at), 1.0)
}

fn frame(body: &str, at: i64) -> Frame {
    frame_with(body, at, Accel::Never)
}

fn clip(generator: &str, set: &str) -> String {
    format!("    clip c1 gen={generator}@1 at=0f dur=96f\n      set {set}\n")
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * frame.width + x) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

fn lit(frame: &Frame) -> usize {
    frame
        .rgba
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 128)
        .count()
}

/// How many lit columns the frame has to the left of `x`, and from there on.
fn lit_either_side(frame: &Frame, x: u32) -> (usize, usize) {
    let (mut left, mut right) = (0, 0);
    for row in 0..frame.height {
        for column in 0..frame.width {
            if pixel(frame, column, row)[0] > 128 {
                if column < x {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }
    }
    (left, right)
}

/// The most any channel of any pixel differs by.
fn difference(a: &Frame, b: &Frame) -> u8 {
    a.rgba
        .iter()
        .zip(&b.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

#[test]
fn a_shape_is_the_size_its_numbers_say_and_the_clip_places_it() {
    let rect = frame(&clip("shape", "width=0.5 height=0.2"), 3);
    // Half the frame's height across, and a fifth of it down.
    let row: Vec<u32> = (0..WIDE)
        .filter(|x| pixel(&rect, *x, TALL / 2)[0] > 128)
        .collect();
    assert_eq!(row.len(), 90);
    assert_eq!(row[0], 115);
    let column = (0..TALL)
        .filter(|y| pixel(&rect, WIDE / 2, *y)[0] > 128)
        .count();
    assert_eq!(column, 36);
    // A clip's own position moves it, as it does any picture.
    let moved = frame(&clip("shape", "width=0.5 height=0.25 x=0.25"), 3);
    let first = (0..WIDE)
        .find(|x| pixel(&moved, *x, TALL / 2)[0] > 128)
        .expect("drawn");
    assert_eq!(first, 115 + 80);
    let ellipse = frame(&clip("shape", "kind=ellipse width=0.5 height=0.5"), 3);
    let area = std::f64::consts::PI * 45.0 * 45.0;
    assert!((lit(&ellipse) as f64 - area).abs() < area * 0.02);
    assert_eq!(pixel(&ellipse, WIDE / 2 - 44, TALL / 2 - 44), [0, 0, 0]);
}

#[test]
fn a_shape_has_the_color_a_solid_of_that_color_has() {
    for color in ["#808080", "#c04020", "#102030"] {
        let solid = frame(&clip("solid", &format!("color={color} scale=0.5")), 3);
        let shape = frame(
            &clip("shape", &format!("width=0.5 height=0.5 fill={color}")),
            3,
        );
        assert_eq!(
            pixel(&solid, WIDE / 2, TALL / 2),
            pixel(&shape, WIDE / 2, TALL / 2),
            "{color}"
        );
    }
}

#[test]
fn a_path_comes_from_the_document_and_is_measured_from_the_center() {
    let triangle = frame(
        &clip(
            "shape",
            r#"kind=path points="-0.25,0.25 0.25,0.25 0,-0.25""#,
        ),
        3,
    );
    assert_eq!(pixel(&triangle, WIDE / 2, TALL / 2)[0], 255);
    assert_eq!(pixel(&triangle, WIDE / 2 - 40, TALL / 2 - 40)[0], 0);
    // Half base by height, in pixels.
    let area = 0.5 * 90.0 * 90.0;
    assert!((lit(&triangle) as f64 - area).abs() < area * 0.03);
}

#[test]
fn a_stroke_draws_itself_on() {
    let body = clip(
        "shape",
        r##"kind=path points="-0.5,0 0.5,0" closed=false stroke_width=0.05 trim_end={0f:0 24f:1}"##,
    );
    assert_eq!(lit(&frame(&body, 0)), 0, "nothing yet");
    let half = frame(&body, 12);
    let whole = frame(&body, 24);
    let middle = WIDE / 2;
    // Its end is round, so it reaches half a stroke past where the length stops.
    let (left, right) = lit_either_side(&half, middle);
    assert!(left > 800 && right < 50, "{left} {right}");
    let (left, right) = lit_either_side(&whole, middle);
    assert!(
        left > 0 && (left as i64 - right as i64).abs() < 16,
        "{left} {right}"
    );
}

#[test]
fn what_animates_counts_from_where_the_clip_starts() {
    let early =
        "    clip c1 gen=shape@1 at=0f dur=48f\n      set width={0f:0 24f:0.5} height=0.5\n";
    let late =
        "    clip c1 gen=shape@1 at=24f dur=48f\n      set width={0f:0 24f:0.5} height=0.5\n";
    assert_eq!(difference(&frame(early, 12), &frame(late, 36)), 0);
    assert_ne!(lit(&frame(early, 12)), lit(&frame(early, 24)));
}

#[test]
fn particles_can_start_on_any_frame_and_draw_the_same_every_time() {
    let body = clip(
        "particles",
        "rate=200 life=2 velocity=0.5 size=0.02 size_end=0.02 color=#ffffff color_end=#ffffff soft=0 spread=360",
    );
    let direct = frame(&body, 48);
    assert!(lit(&direct) > 100);
    // A new compositor, starting elsewhere and stepping past it.
    assert_eq!(difference(&direct, &frame(&body, 48)), 0);
    assert_ne!(difference(&direct, &frame(&body, 49)), 0);
    let other = clip(
        "particles",
        "rate=200 life=2 velocity=0.5 size=0.02 size_end=0.02 color=#ffffff color_end=#ffffff soft=0 spread=360 seed=9",
    );
    assert_ne!(difference(&direct, &frame(&other, 48)), 0);
}

#[test]
fn particles_leave_the_way_they_are_told_and_gravity_pulls_them() {
    let up = clip(
        "particles",
        "rate=100 life=0.5 velocity=0.4 spread=0 vary=0 size=0.02 size_end=0.02 color=#ffffff color_end=#ffffff soft=0",
    );
    let found = frame(&up, 24);
    // 0.4 frame heights a second for half a second, straight up from the middle.
    assert!(pixel(&found, WIDE / 2, TALL / 2 - 30)[0] > 128);
    assert_eq!(pixel(&found, WIDE / 2, TALL / 2 + 30)[0], 0);
    let falling = clip(
        "particles",
        "rate=100 life=1 velocity=0.4 direction=0 spread=0 vary=0 gravity=1 size=0.02 size_end=0.02 color=#ffffff color_end=#ffffff soft=0",
    );
    let arc = frame(&falling, 48);
    // Half a second out it has gone 0.2 across and 0.125 down, in frame heights.
    assert!(pixel(&arc, WIDE / 2 + 36, TALL / 2 + 22)[0] > 128);
    assert_eq!(pixel(&arc, WIDE / 2 + 36, TALL / 2)[0], 0);
}

const WORDS: &str = "AAAA BBBB";

#[test]
fn a_title_at_rest_draws_as_it_did() {
    let plain = frame(&clip("title", &format!(r#"text="{WORDS}" size=0.2"#)), 3);
    let revealed = frame(
        &clip("title", &format!(r#"text="{WORDS}" size=0.2 reveal=1"#)),
        3,
    );
    assert!(lit(&plain) > 500);
    assert!(
        difference(&plain, &revealed) < 40,
        "{}",
        difference(&plain, &revealed)
    );
    let near: usize = plain
        .rgba
        .iter()
        .zip(&revealed.rgba)
        .filter(|(a, b)| a.abs_diff(**b) > 24)
        .count();
    assert!(near < 40, "{near}");
}

#[test]
fn a_title_comes_in_from_the_start_of_the_text() {
    let body = clip(
        "title",
        &format!(r#"text="{WORDS}" size=0.2 reveal={{0f:0 24f:1}}"#),
    );
    assert_eq!(lit(&frame(&body, 0)), 0, "nothing yet");
    let middle = WIDE / 2;
    let part = frame(&body, 10);
    let (left, right) = lit_either_side(&part, middle);
    assert!(left > 100 && right < left / 4, "{left} {right}");
    let (left, right) = lit_either_side(&frame(&body, 24), middle);
    assert!(left > 100 && right > 100, "{left} {right}");
}

#[test]
fn a_title_can_come_in_by_word() {
    // Each word waits until the one before has come in.
    let by_word = clip(
        "title",
        &format!(
            r#"text="{WORDS}" size=0.2 reveal=0.4 reveal_by=word reveal_soft=0.01 from_opacity=0"#
        ),
    );
    let (left, right) = lit_either_side(&frame(&by_word, 3), WIDE / 2);
    assert!(left > 100 && right == 0, "{left} {right}");
    let by_char = clip(
        "title",
        &format!(r#"text="{WORDS}" size=0.2 reveal=0.3 reveal_soft=0.01 from_opacity=0"#),
    );
    let (left, _) = lit_either_side(&frame(&by_char, 3), WIDE / 2);
    assert!(left > 100 && left < lit_either_side(&frame(&by_word, 3), WIDE / 2).0);
}

#[test]
fn a_title_comes_in_from_where_it_is_told() {
    let lifted = clip(
        "title",
        &format!(r#"text="{WORDS}" size=0.2 reveal=0 from_y=-0.3 from_opacity=1"#),
    );
    let found = frame(&lifted, 3);
    let rows: Vec<u32> = (0..TALL)
        .filter(|y| (0..WIDE).any(|x| pixel(&found, x, *y)[0] > 128))
        .collect();
    let rest = frame(&clip("title", &format!(r#"text="{WORDS}" size=0.2"#)), 3);
    let rest_rows: Vec<u32> = (0..TALL)
        .filter(|y| (0..WIDE).any(|x| pixel(&rest, x, *y)[0] > 128))
        .collect();
    assert_eq!(rows[0] + 54, rest_rows[0]);
}

#[test]
fn the_gpu_draws_them_as_the_cpu_does() {
    let scenes = [
        clip(
            "shape",
            "kind=star sides=5 width=0.6 height=0.6 stroke_width=0.02 stroke=#ff0000",
        ),
        clip("particles", "rate=100 life=1 spread=360 size=0.03"),
        clip("title", &format!(r#"text="{WORDS}" size=0.2 reveal=0.5"#)),
        format!(
            "{}      fx blur.gaussian@1 radius=4\n",
            clip("shape", "width=0.4 height=0.4 x=0.1 rotation=20")
        ),
    ];
    for scene in scenes {
        let cpu = frame_with(&scene, 12, Accel::Never);
        let auto = frame_with(&scene, 12, Accel::Always);
        let off = difference(&cpu, &auto);
        assert!(off <= 4, "{off} in {scene}");
    }
}

#[test]
fn the_compositor_keeps_moving_them_after_the_first_frame() {
    // One compositor, as playback has, drawing the same clip at new times.
    let body = clip("shape", "width={0f:0.1 24f:0.5} height=0.5");
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size={WIDE}x{TALL} audio=48000\n  track V1 video\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let compositor = Compositor::new(&project, Catalog::builtin(), &ComposeOptions::default())
        .expect("composes");
    let first = compositor.frame(Rate::FPS_24.time_of(0), 1.0);
    let later = compositor.frame(Rate::FPS_24.time_of(24), 1.0);
    assert!(
        lit(&later) > lit(&first) * 4,
        "{} {}",
        lit(&first),
        lit(&later)
    );
}
