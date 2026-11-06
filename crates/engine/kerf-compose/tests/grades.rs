//! The grades: what each does to a picture, and that an amount of 0 leaves it alone.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

const WHITE: &str = "#ffffff";
const LIGHT: &str = "#c8c8c8";
const MIDDLE: &str = "#808080";
const DARK: &str = "#282828";
const BLACK: &str = "#000000";
/// A saturated color and a muted one.
const RED: &str = "#c03020";
const MUTED: &str = "#907868";

fn grades() -> Vec<String> {
    let found: Vec<String> = Catalog::builtin()
        .looks()
        .filter(|look| look.id.as_str().starts_with("look.grade."))
        .map(|look| look.id.to_string())
        .collect();
    assert_eq!(found.len(), 18, "{found:?}");
    found
}

fn frame(color: &str, fx: &str, accel: Accel) -> Frame {
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color={color}\n      {fx}\n"
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
    .frame(Rate::FPS_24.time_of(1), 1.0)
}

fn middle(frame: &Frame) -> [u8; 3] {
    let at = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

/// `color` graded by `grade`, at its full amount.
fn graded(grade: &str, color: &str) -> [i32; 3] {
    let found = frame(color, &format!("fx look.grade.{grade}@1"), Accel::Never);
    middle(&found).map(i32::from)
}

fn plain(color: &str) -> [i32; 3] {
    middle(&frame(color, "fx color.basic@1", Accel::Never)).map(i32::from)
}

fn spread(pixel: [i32; 3]) -> i32 {
    pixel.iter().max().unwrap_or(&0) - pixel.iter().min().unwrap_or(&0)
}

fn sum(pixel: [i32; 3]) -> i32 {
    pixel.iter().sum()
}

#[test]
fn at_no_amount_each_leaves_the_picture_alone_and_at_full_each_changes_it() {
    for grade in grades() {
        let mut changed = false;
        for color in [WHITE, LIGHT, MIDDLE, DARK, BLACK, RED, MUTED] {
            let before = plain(color);
            let none = middle(&frame(
                color,
                &format!("fx {grade}@1 amount=0"),
                Accel::Never,
            ))
            .map(i32::from);
            for k in 0..3 {
                assert!(
                    (none[k] - before[k]).abs() <= 1,
                    "{grade} at 0 on {color}: {none:?} from {before:?}"
                );
            }
            let full = middle(&frame(color, &format!("fx {grade}@1"), Accel::Never)).map(i32::from);
            changed |= (0..3).any(|k| (full[k] - before[k]).abs() > 3);
        }
        assert!(changed, "{grade} did nothing");
    }
}

#[test]
fn half_an_amount_goes_part_of_the_way() {
    for grade in grades() {
        for color in [LIGHT, DARK, RED] {
            let (before, full) = (plain(color), {
                let found = frame(color, &format!("fx {grade}@1"), Accel::Never);
                middle(&found).map(i32::from)
            });
            let half = middle(&frame(
                color,
                &format!("fx {grade}@1 amount=0.5"),
                Accel::Never,
            ))
            .map(i32::from);
            for k in 0..3 {
                let (low, high) = (before[k].min(full[k]), before[k].max(full[k]));
                assert!(
                    half[k] >= low - 6 && half[k] <= high + 6,
                    "{grade} on {color}: {before:?}, {half:?}, {full:?}"
                );
            }
        }
    }
}

#[test]
fn cinematic_grades_do_what_they_say() {
    let [r, _, b] = graded("teal-warm", DARK);
    assert!(b > r, "teal shadows: {r} {b}");
    let [r, _, b] = graded("teal-warm", LIGHT);
    assert!(r > b, "warm highlights: {r} {b}");
    let night = graded("night-exterior", MIDDLE);
    assert!(sum(night) < sum(plain(MIDDLE)) * 3 / 4, "darker: {night:?}");
    assert!(night[2] > night[0], "cooler: {night:?}");
    let [r, _, b] = graded("golden-dusk", MIDDLE);
    assert!(r > b + 8, "warm: {r} {b}");
    let [r, _, b] = graded("steel-blue", MIDDLE);
    assert!(b > r + 4, "cool: {r} {b}");
    let (dark, light) = (graded("hard-sun", DARK), graded("hard-sun", LIGHT));
    assert!(
        light[0] - dark[0] > plain(LIGHT)[0] - plain(DARK)[0],
        "more contrast: {dark:?} {light:?}"
    );
}

#[test]
fn film_grades_do_what_they_say() {
    let faded = graded("faded-print", BLACK);
    assert!(faded[0] > 10, "lifted blacks: {faded:?}");
    let silver = graded("silver-retention", RED);
    assert!(
        spread(silver) < spread(plain(RED)),
        "less color: {silver:?}"
    );
    let (shadow, highlight) = (
        graded("cross-processed", DARK),
        graded("cross-processed", LIGHT),
    );
    assert!(shadow[2] > shadow[0], "blue shadows: {shadow:?}");
    assert!(
        highlight[1] > highlight[2],
        "yellow-green highlights: {highlight:?}"
    );
    let [r, _, b] = graded("warm-negative", MIDDLE);
    assert!(r > b, "warm: {r} {b}");
}

#[test]
fn mono_grades_do_what_they_say() {
    for grade in ["neutral-mono", "hard-mono"] {
        let found = graded(grade, RED);
        assert!(spread(found) <= 1, "{grade}: {found:?}");
    }
    let [r, _, b] = graded("sepia", RED);
    assert!(r > b + 8, "warm brown: {r} {b}");
    let [r, _, b] = graded("cold-mono", RED);
    assert!(b > r + 8, "cold: {r} {b}");
    let (dark, light) = (graded("hard-mono", DARK), graded("hard-mono", LIGHT));
    assert!(light[0] - dark[0] > plain(LIGHT)[0] - plain(DARK)[0]);
}

#[test]
fn technical_grades_do_what_they_say() {
    assert!(sum(graded("lift-shadows", DARK)) > sum(plain(DARK)) + 12);
    let protected = graded("protect-highlights", WHITE);
    assert!(sum(protected) < sum(plain(WHITE)) - 12, "{protected:?}");
    let half = spread(graded("half-saturation", RED));
    let full = spread(plain(RED));
    assert!(
        (half as f32 / full as f32 - 0.5).abs() < 0.2,
        "{half} of {full}"
    );
    assert!(spread(graded("boost-vibrance", MUTED)) > spread(plain(MUTED)));
    let (low, high) = (graded("legal-range", BLACK), graded("legal-range", WHITE));
    assert!((low[0] - 16).abs() <= 1, "{low:?}");
    assert!((high[0] - 235).abs() <= 1, "{high:?}");
}

#[test]
fn each_draws_the_same_on_the_gpu_as_on_the_cpu() {
    for grade in grades() {
        for color in [LIGHT, MIDDLE, DARK, RED] {
            let fx = format!("fx {grade}@1");
            let cpu = middle(&frame(color, &fx, Accel::Never));
            let gpu = middle(&frame(color, &fx, Accel::Always));
            for k in 0..3 {
                assert!(
                    cpu[k].abs_diff(gpu[k]) <= 3,
                    "{grade} on {color}: {cpu:?} and {gpu:?}"
                );
            }
        }
    }
}

/// The grades with color wheels or a named look, which Lumetri's GPU kernel does not draw, so a frame
/// holding one is drawn on the CPU.
const ON_THE_CPU: [&str; 8] = [
    "cold-mono",
    "cross-processed",
    "golden-dusk",
    "night-exterior",
    "sepia",
    "silver-retention",
    "teal-warm",
    "warm-negative",
];

#[test]
fn the_rest_stay_on_the_gpu_where_there_is_one() {
    let draws_on_gpu = |fx: &str| {
        let project = parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      {fx}\n"
            ),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                accel: Accel::Always,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
        .draws_on_gpu(Rate::FPS_24.time_of(1), 1.0)
    };
    if !draws_on_gpu("fx color.basic@1") {
        return;
    }
    for grade in grades() {
        let name = grade.trim_start_matches("look.grade.");
        assert_eq!(
            draws_on_gpu(&format!("fx {grade}@1")),
            !ON_THE_CPU.contains(&name),
            "{grade}"
        );
    }
}
