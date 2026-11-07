//! How long the compositor takes to draw a frame, on footage it decodes.
//!
//! ```text
//! ffmpeg -f lavfi -i testsrc2=s=3840x2160:r=24:d=4 -pix_fmt yuv420p clip.mp4
//! cargo run --release -p kerf-compose --example frame_bench -- clip.mp4 [scene] [second.mp4]
//! ```
//!
//! `transition` as the scene draws each transition in the catalog between the two clips, and
//! `transition push,iris-round` just those.

use kerf_compose::{Accel, ComposeOptions, Compositor};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::time::Instant;

/// Frames drawn of each scene; the first is left out of the median.
const FRAMES: i64 = 9;

const SCENES: &[(&str, &str)] = &[
    // Decoding alone: what no drawing can go below.
    ("plain", ""),
    ("placed", "      set scale=0.9\n"),
    ("lumetri", "      fx color.lumetri@2 exposure=0.5\n"),
    ("color basic", "      fx color.basic exposure=0.3\n"),
    ("unsharp", "      fx blur.unsharp amount=100 radius=3\n"),
    ("blur 40", "      fx blur.gaussian@1 radius=40\n"),
    ("glow", "      fx light.wonderglow@1\n"),
    ("ultra key", "      fx key.ultra@1 key_color=#30c040\n"),
    (
        "radial zoom",
        "      fx blur.radialzoom@1 amount=30 quality=30\n",
    ),
    ("grade steel-blue", "      fx look.grade.steel-blue@1\n"),
    ("grade sepia", "      fx look.grade.sepia@1\n"),
    (
        "grade stack of 3",
        "      fx look.grade.teal-warm@1 amount=0.6\n      fx look.grade.golden-dusk@1 amount={0f:0.2 48f:1}\n      fx look.grade.sepia@1 amount=0.4\n",
    ),
    (
        "grade stack of 3 at 0",
        "      fx look.grade.teal-warm@1 amount=0\n      fx look.grade.golden-dusk@1 amount=0\n      fx look.grade.sepia@1 amount=0\n",
    ),
    (
        "heal found",
        "      fx image.heal@1 points=\"0.4,0.5 0.5,0.52\" radius=0.03\n",
    ),
    (
        "clone",
        "      fx image.clone@1 points=\"0.4,0.5 0.5,0.52\" radius=0.03 from_x=0.2\n",
    ),
    (
        "warp",
        "      fx image.warp@1 moves=\"0.5,0.5>0.55,0.45\" radius=0.2\n",
    ),
    (
        "warp small",
        "      fx image.warp@1 moves=\"0.5,0.5>0.51,0.49\" radius=0.02\n",
    ),
    (
        "fill small",
        "      mask k1 rect=0.45,0.45,0.55,0.55 mode=none\n      fx image.fill@1 path=k1\n",
    ),
    (
        "fill big",
        "      mask k1 rect=0.2,0.2,0.8,0.8 mode=none\n      fx image.fill@1 path=k1\n",
    ),
];

fn time(document: &str, frames: i64) -> (f64, f64) {
    time_from(document, 0, frames)
}

/// The middle of the cut in [`transition`], its first frame, and how many frames it lasts.
const CUT: (i64, i64) = (36, 24);

/// How long a frame takes in the middle half of `fx` between two clips, as named in the catalog, each
/// with footage of its own, since one file read at two places seeks for every frame.
fn transition(head: &str, fx: &str) -> f64 {
    let document = format!(
        "{head}  track V1 video\n    clip c1 media=m1 src=0f at=0f dur=48f\n    clip c2 media=m2 src=24f at=48f dur=48f\n    transition t1 from=c1 to=c2 fx={fx}@1 dur={}f\n",
        CUT.1
    );
    let (_, median) = time_from(&document, CUT.0 + CUT.1 / 4, CUT.1 / 2);
    median
}

fn time_from(document: &str, from: i64, frames: i64) -> (f64, f64) {
    let project = parse_document(document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    // Wait for the kernels rather than timing the CPU while they build. `KERF_GPU=0` measures the
    // CPU, and so does a frame the kernels cannot draw.
    let options = ComposeOptions {
        accel: Accel::Always,
        keep_frames: std::env::var_os("KERF_BENCH_KEEP").is_some(),
        ..ComposeOptions::default()
    };
    let compositor = Compositor::new(&project, Catalog::builtin(), &options).expect("draws");
    // With frames kept, what is timed is drawing them again.
    if std::env::var_os("KERF_BENCH_KEEP").is_some() {
        for index in from..from + frames {
            compositor.frame(Rate::FPS_24.time_of(index), 1.0);
        }
    }
    let mut times: Vec<f64> = (from..from + frames)
        .map(|index| {
            let began = Instant::now();
            let frame = compositor.frame(Rate::FPS_24.time_of(index), 1.0);
            std::hint::black_box(frame.rgba.len());
            began.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let first = times.remove(0);
    times.sort_by(f64::total_cmp);
    (first, times[times.len() / 2])
}

fn main() {
    let clip = std::env::args().nth(1).expect("a clip to draw");
    // Only the scenes whose names hold this.
    let only = std::env::args().nth(2).unwrap_or_default();
    // What the document says about the footage beyond its size, such as ` bits=10`.
    let fields = std::env::var("KERF_BENCH_MEDIA").unwrap_or_default();
    // The incoming clip of a transition: the third argument, or the same file again.
    let second = std::env::args().nth(3).unwrap_or_else(|| clip.clone());
    let media = format!(
        "media m1 path=\"{clip}\" rate=24 dur=00:00:04:00 video=3840x2160{fields}\nmedia m2 path=\"{second}\" rate=24 dur=00:00:04:00 video=3840x2160{fields}\n"
    );
    // What the sequence says of itself, such as ` color=rec2100-pq`.
    let sequence = std::env::var("KERF_BENCH_SEQUENCE").unwrap_or_default();
    println!(
        "{:<10} {:>9} {:>11} {:>8}",
        "scene", "size", "median ms", "fps"
    );
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        let head = format!(
            "kerf 1\nproject name=\"bench\"\n{media}sequence s1 rate=24 size={width}x{height} audio=48000{sequence}\n"
        );
        // `transition iris-round,push`, or `transition` for every one the catalog has.
        if let Some(wanted) = only.strip_prefix("transition") {
            let wanted = wanted.trim();
            let named: Vec<&str> = if wanted.is_empty() {
                Catalog::builtin()
                    .all()
                    .filter(|def| kerf_fx::transitions::filmcraft_id(def.id.as_str()).is_some())
                    .map(|def| def.id.as_str())
                    .collect()
            } else {
                wanted.split(',').collect()
            };
            for fx in named {
                let median = transition(&head, fx);
                println!(
                    "{fx:<18} {:>9} {median:>11.1} {:>8.1}",
                    format!("{width}x{height}"),
                    1000.0 / median
                );
            }
            continue;
        }
        let clip_line = "    clip c1 media=m1 src=0f at=0f dur=48f\n";
        for (name, body) in SCENES.iter().filter(|(name, _)| name.contains(&only)) {
            let document = format!("{head}  track V1 video\n{clip_line}{body}");
            let (_, median) = time(&document, FRAMES);
            println!(
                "{name:<10} {:>9} {median:>11.1} {:>8.1}",
                format!("{width}x{height}"),
                1000.0 / median
            );
        }
        if !"4 layers".contains(&only) {
            continue;
        }
        let layers: String = (1..=4)
            .rev()
            .map(|n| {
                format!(
                    "  track V{n} video\n    clip c{n} media=m1 src=0f at=0f dur=48f\n      set scale=0.8 opacity=0.8\n"
                )
            })
            .collect();
        let (_, median) = time(&format!("{head}{layers}"), FRAMES);
        println!(
            "{:<10} {:>9} {median:>11.1} {:>8.1}",
            "4 layers",
            format!("{width}x{height}"),
            1000.0 / median
        );
    }
}
