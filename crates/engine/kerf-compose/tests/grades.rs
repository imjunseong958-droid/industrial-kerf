//! The grades: what each does to a picture, that an amount of 0 leaves it alone, and how they stack.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_fx::filmcraft::instance::leaves_alone;
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

/// The grades whose frames go to the CPU: none, since Lumetri's kernels draw color wheels and named
/// looks. A new grade with a LUT, curves, sharpening, or an HSL secondary would be named here.
const ON_THE_CPU: [&str; 0] = [];

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

/// Decoded footage, whose picture has many levels and colors where a solid has one, so that
/// rounding has somewhere to show.
struct Footage {
    _folder: tempfile::TempDir,
    media: String,
}

/// Grades that do not commute, in pairs.
const PAIRS: [(&str, &str); 5] = [
    ("teal-warm", "sepia"),
    ("legal-range", "hard-sun"),
    ("night-exterior", "faded-print"),
    ("silver-retention", "golden-dusk"),
    ("half-saturation", "cross-processed"),
];

const ACCELS: [Accel; 2] = [Accel::Never, Accel::Always];

impl Footage {
    fn new() -> Self {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("footage.mp4");
        let output = std::process::Command::new(
            std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned()),
        )
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc2=s=320x180:r=24:d=1"])
        .args(["-c:v", "libx264", "-crf", "12", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let media = format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\n",
            path.display()
        );
        Self {
            _folder: folder,
            media,
        }
    }

    /// The document of a clip with `grades`, in the order written.
    fn stack(&self, grades: &[String]) -> String {
        let lines: String = grades.iter().map(|g| format!("      fx {g}\n")).collect();
        format!(
            "{}sequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n{lines}",
            self.media
        )
    }

    /// One grade applied, and then the other on what it drew: the first on a clip in a sequence of
    /// its own, the second on a clip of that sequence.
    fn by_hand(&self, first: &str, second: &str) -> String {
        format!(
            "{}sequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 seq=inner at=0f dur=24f\n      fx {second}\nsequence inner rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c2 media=m src=0f at=0f dur=24f\n      fx {first}\n",
            self.media
        )
    }
}

fn compositor(document: &str, accel: Accel) -> Compositor {
    let project = parse_document(document, Catalog::builtin())
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
}

fn picture(document: &str, accel: Accel, at: i64) -> Vec<u8> {
    compositor(document, accel)
        .frame(Rate::FPS_24.time_of(at), 1.0)
        .rgba
}

/// The most any channel of any pixel differs.
fn worst(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

fn full(grade: &str) -> String {
    format!("look.grade.{grade}@1")
}

#[test]
fn a_stack_is_its_grades_one_after_the_other_in_the_order_written() {
    let footage = Footage::new();
    for (first, second) in PAIRS {
        let (a, b) = (full(first), full(second));
        for accel in ACCELS {
            let one_way = picture(&footage.stack(&[a.clone(), b.clone()]), accel, 1);
            let other_way = picture(&footage.stack(&[b.clone(), a.clone()]), accel, 1);
            assert!(
                worst(&one_way, &other_way) > 8,
                "{first} and {second} on {accel:?} come out the same in either order"
            );
            assert!(
                one_way == picture(&footage.by_hand(&a, &b), accel, 1),
                "{first} then {second} on {accel:?}"
            );
            assert!(
                other_way == picture(&footage.by_hand(&b, &a), accel, 1),
                "{second} then {first} on {accel:?}"
            );
        }
    }
}

#[test]
fn the_last_grade_has_the_last_word() {
    let stack = |a: &str, b: &str, color: &str| {
        let fx = format!("fx look.grade.{a}@1\n      fx look.grade.{b}@1");
        middle(&frame(color, &fx, Accel::Never)).map(i32::from)
    };
    // In legal range when it comes last; contrast takes it out when it comes first.
    let (black, white) = (
        stack("hard-mono", "legal-range", BLACK),
        stack("hard-mono", "legal-range", WHITE),
    );
    assert!(black[0] >= 15 && white[0] <= 236, "{black:?} {white:?}");
    let (black, white) = (
        stack("legal-range", "hard-mono", BLACK),
        stack("legal-range", "hard-mono", WHITE),
    );
    assert!(black[0] < 15 && white[0] > 236, "{black:?} {white:?}");
    // Grey when mono comes last; toned when the tone does.
    assert!(spread(stack("sepia", "neutral-mono", RED)) <= 1);
    assert!(spread(stack("neutral-mono", "sepia", RED)) > 8);
}

#[test]
fn at_no_amount_a_grade_is_not_there_down_to_the_byte() {
    let footage = Footage::new();
    // Faded Print before a grade at no amount is where a run of it moved a byte on the CPU.
    let others = ["faded-print", "teal-warm", "sepia", "legal-range"].map(full);
    for accel in ACCELS {
        let plain = picture(&footage.stack(&[]), accel, 1);
        let alone: Vec<(&String, Vec<u8>)> = others
            .iter()
            .map(|other| {
                (
                    other,
                    picture(&footage.stack(std::slice::from_ref(other)), accel, 1),
                )
            })
            .collect();
        for grade in grades() {
            let zero = format!("{grade}@1 amount=0");
            assert!(
                plain == picture(&footage.stack(std::slice::from_ref(&zero)), accel, 1),
                "{grade} alone on {accel:?}"
            );
            for (other, only) in &alone {
                let (before, after) = (
                    [zero.clone(), (*other).clone()],
                    [(*other).clone(), zero.clone()],
                );
                assert!(
                    *only == picture(&footage.stack(&before), accel, 1),
                    "{grade} before {other} on {accel:?}"
                );
                assert!(
                    *only == picture(&footage.stack(&after), accel, 1),
                    "{grade} after {other} on {accel:?}"
                );
            }
        }
    }
}

#[test]
fn at_no_amount_a_grade_is_not_there_in_hdr_either() {
    let footage = Footage::new();
    let hdr = |grades: &[String]| {
        footage
            .stack(grades)
            .replace("audio=48000\n", "audio=48000 color=rec2100-pq\n")
    };
    let at = Rate::FPS_24.time_of(1);
    for accel in ACCELS {
        let plain = compositor(&hdr(&[]), accel);
        let on_gpu = plain.draws_on_gpu(at, 1.0);
        let plain = plain.frame16(at, 1.0).rgba;
        for grade in grades() {
            let drawn = compositor(&hdr(&[format!("{grade}@1 amount=0")]), accel);
            assert_eq!(drawn.draws_on_gpu(at, 1.0), on_gpu, "{grade} on {accel:?}");
            assert!(plain == drawn.frame16(at, 1.0).rgba, "{grade} on {accel:?}");
        }
    }
}

#[test]
fn a_grade_says_it_leaves_the_picture_alone_only_at_no_amount() {
    let catalog = Catalog::builtin();
    let expanded = |fx: &str| {
        let project = parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      fx {fx}\n"
            ),
            catalog,
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        kerf_model::look::expand(&project, catalog)
            .expect("expands")
            .clip("c1")
            .expect("a clip")
            .effects
            .clone()
    };
    let working = kerf_color::WorkingSpace::Rec709;
    let frame = |at: i64| Rate::FPS_24.time_of(at);
    for grade in grades() {
        for effect in expanded(&format!("{grade}@1 amount=0")) {
            assert!(
                leaves_alone(&effect, frame(0), working),
                "{grade}: {effect:?}"
            );
        }
        for effect in expanded(&format!("{grade}@1 amount=0.001")) {
            assert!(!leaves_alone(&effect, frame(0), working), "{grade}");
        }
        for effect in expanded(&format!("{grade}@1 amount={{0f:0 12f:1 24f:0}}")) {
            assert!(leaves_alone(&effect, frame(0), working), "{grade}");
            assert!(!leaves_alone(&effect, frame(1), working), "{grade}");
            assert!(leaves_alone(&effect, frame(24), working), "{grade}");
        }
    }
    // Other things Lumetri can do are not at rest just because its numbers are.
    for fx in [
        "color.lumetri@2 sharpen=20",
        "color.lumetri@2 hsl_on=true",
        "color.lumetri@2 curve_luma=\"0,0.1 1,1\"",
    ] {
        let effects = expanded(fx);
        assert!(!leaves_alone(&effects[0], frame(0), working), "{fx}");
    }
    assert!(leaves_alone(
        &expanded("color.lumetri@2")[0],
        frame(0),
        working
    ));
}

#[test]
fn an_amount_that_animates_draws_the_amount_it_has_then() {
    let footage = Footage::new();
    let teal = full("teal-warm");
    let keys = "{0f:0:ease 12f:1 24f:0}";
    // Where the keys put the amount: eased halfway up, at the top, and straight back down.
    for (frame, amount) in [(3, 0.15625), (6, 0.5), (12, 1.0), (18, 0.5)] {
        for grade in ["sepia", "golden-dusk", "legal-range", "silver-retention"].map(full) {
            for accel in ACCELS {
                let animated = picture(
                    &footage.stack(&[teal.clone(), format!("{grade} amount={keys}")]),
                    accel,
                    frame,
                );
                let held = picture(
                    &footage.stack(&[teal.clone(), format!("{grade} amount={amount}")]),
                    accel,
                    frame,
                );
                assert!(
                    worst(&animated, &held) <= 1,
                    "{grade} at frame {frame} on {accel:?}"
                );
            }
        }
    }
    // Where it comes to 0, the grade is not there.
    for accel in ACCELS {
        let sepia = format!("look.grade.sepia@1 amount={keys}");
        let (only, stacked) = (
            footage.stack(std::slice::from_ref(&teal)),
            footage.stack(&[sepia, teal.clone()]),
        );
        assert!(
            picture(&only, accel, 0) == picture(&stacked, accel, 0),
            "{accel:?}"
        );
        assert!(worst(&picture(&only, accel, 12), &picture(&stacked, accel, 12)) > 8);
    }
}

#[test]
fn stacks_draw_the_same_on_the_gpu_as_on_the_cpu() {
    let footage = Footage::new();
    let all = grades();
    let at = Rate::FPS_24.time_of(1);
    let on_gpu = compositor(&footage.stack(&[]), Accel::Always).draws_on_gpu(at, 1.0);
    let compare = |document: String, name: String| {
        let gpu = compositor(&document, Accel::Always);
        assert_eq!(gpu.draws_on_gpu(at, 1.0), on_gpu, "{name}");
        let (cpu, gpu) = (
            compositor(&document, Accel::Never).frame(at, 1.0).rgba,
            gpu.frame(at, 1.0).rgba,
        );
        assert!(worst(&cpu, &gpu) <= 3, "{name}: {}", worst(&cpu, &gpu));
    };
    for first in &all {
        for second in &all {
            let stack = [format!("{first}@1"), format!("{second}@1")];
            compare(footage.stack(&stack), format!("{first} then {second}"));
        }
    }
    let three = [
        "look.grade.teal-warm@1 amount=0.6".to_owned(),
        "look.grade.golden-dusk@1 amount={0f:0 24f:2}".to_owned(),
        "look.grade.sepia@1 amount={0f:0 12f:1:ease}".to_owned(),
    ];
    compare(footage.stack(&three), "three".to_owned());
}

#[test]
fn flattening_a_grade_in_a_stack_draws_the_same() {
    let footage = Footage::new();
    let stack = footage.stack(&[
        "look.grade.teal-warm@1 amount=0.6".to_owned(),
        "look.grade.sepia@1 amount={0f:0 12f:1:ease 24f:0.5}".to_owned(),
        "look.grade.legal-range@1".to_owned(),
    ]);
    let project = parse_document(&stack, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let scripts = [
        "fx c1 flatten 1",
        "fx c1 flatten 2",
        "fx c1 flatten 3",
        "fx c1 flatten 3\nfx c1 flatten 2\nfx c1 flatten 1",
    ];
    for script in scripts {
        let flat = kerf_lang::apply_script(&project, Catalog::builtin(), script)
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        for accel in ACCELS {
            for at in [0, 5, 12, 23] {
                let time = Rate::FPS_24.time_of(at);
                let draw = |project: &kerf_model::Project| {
                    Compositor::new(
                        project,
                        Catalog::builtin(),
                        &ComposeOptions {
                            accel,
                            ..ComposeOptions::default()
                        },
                    )
                    .expect("composes")
                    .frame(time, 1.0)
                    .rgba
                };
                assert!(
                    draw(&project) == draw(&flat),
                    "`{script}` at frame {at} on {accel:?}"
                );
            }
        }
    }
}

#[test]
fn a_clip_split_through_an_amount_that_animates_draws_as_it_did() {
    let footage = Footage::new();
    let whole = parse_document(
        &footage.stack(&["look.grade.sepia@1 amount={0f:0 24f:1:ease}".to_owned()]),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let split = kerf_lang::apply_script(&whole, Catalog::builtin(), "split c1 at=7f")
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let draw = |project: &kerf_model::Project, at: i64| {
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
        .rgba
    };
    for at in [3, 7, 15] {
        assert!(
            worst(&draw(&whole, at), &draw(&split, at)) <= 1,
            "frame {at}"
        );
    }
}
