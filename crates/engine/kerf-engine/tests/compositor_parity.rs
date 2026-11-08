//! The compositor draws what the graph renders: the same frames drawn both
//! ways and compared pixel by pixel.
//!
//! The two differ in how they resample and blend, so each case has a
//! tolerance: the mean difference in luma and in color, and the share of
//! pixels whose luma is off by more than a step a viewer would see. A
//! wrong frame, place, or size is far outside them. Luma carries the
//! comparison because the graph works in 4:2:0, where color edges are
//! half as sharp and filters such as `gblur` spread color twice as far.
//! FFmpeg's x86_64 code blends and fades a little differently from its
//! Arm code, about one more luma step in a dip or under a raster effect,
//! and the tolerances cover both.
//!
//! The compositor mixes light linearly where the graph mixes encoded
//! values, so wherever pictures mix (opacity, a dissolve or dip midway, a
//! transparent solid, a blur, an adjustment layer's opacity) it is
//! brighter, by up to a sixth of the range midway through a dip; those
//! cases carry a tolerance of their own for it (`kerf`), and
//! `kerf-compose`'s tests check what it draws there.

use kerf_compose::{ComposeOptions, Compositor};
use kerf_engine::derivatives::DerivativeCache;
use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::program::{compile_program, program_sequence, ProgramOptions};
use kerf_engine::CancellationToken;
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;

fn run(command: &mut Command) -> Vec<u8> {
    let output = command.output().expect("runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn ffmpeg() -> Command {
    let mut command = Command::new(resolve_ffmpeg_bin());
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    command
}

/// Media the cases use: a moving test pattern with a frame counter, color
/// bars in 4:3, and the pattern at 30 fps.
fn media(dir: &Path) -> String {
    let make = |name: &str, source: &str| {
        let path = dir.join(name);
        run(ffmpeg()
            .args(["-f", "lavfi", "-i", source])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "24"])
            .arg(&path));
        path.display().to_string()
    };
    let test = make("test.mp4", "testsrc2=s=320x180:r=24:d=4");
    let bars = make("bars.mp4", "smptebars=s=320x240:r=24:d=4");
    let thirty = make("thirty.mp4", "testsrc2=s=320x180:r=30:d=4");
    format!(
        "kerf 1\nproject name=\"parity\"\n\
         media t path=\"{test}\" rate=24 dur=96f video=320x180\n\
         media b path=\"{bars}\" rate=24 dur=96f video=320x240\n\
         media h path=\"{thirty}\" rate=30 dur=120f video=320x180\n"
    )
}

fn project(media: &str, body: &str) -> Project {
    parse_document(&format!("{media}{body}"), Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

/// Frame `frame` of the graph's render, as RGB. The render starts at the
/// top, as an export does: a render starting mid-frame drops the frame
/// showing there. A `window` renders the frame alone instead, which
/// starts each clip exactly on the source frame it shows: FFmpeg's overlay
/// picks unevenly among frames that arrive faster than the sequence's.
fn graph(project: &Project, dir: &Path, frame: i64, window: bool) -> Vec<u8> {
    let cache = DerivativeCache::new(dir);
    let baked =
        kerf_engine::raster::bake(project, None, &cache, &CancellationToken::new()).expect("bakes");
    let rate = program_sequence(&baked, None).expect("a sequence").rate;
    let compiled = compile_program(
        &baked,
        Catalog::builtin(),
        &ProgramOptions {
            range: Some((
                rate.time_of(if window { frame } else { 0 }),
                rate.time_of(frame + 1),
            )),
            ..ProgramOptions::default()
        },
    )
    .expect("compiles");
    if let Some(folder) = std::env::var_os("KERF_PARITY_DUMP") {
        std::fs::write(
            Path::new(&folder).join(format!("graph-{frame}.txt")),
            &compiled.filter_complex,
        )
        .unwrap();
    }
    let rendered = dir.join(format!("graph-{frame}.nut"));
    run(ffmpeg()
        .args(compiled.argv.iter().skip(2))
        .args(["-c:v", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-c:a", "pcm_s16le", "-f", "nut"])
        .arg(&rendered));
    let frames = run(ffmpeg()
        .arg("-i")
        .arg(&rendered)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]));
    let size = (compiled.width * compiled.height * 3) as usize;
    let rendered = if window { 1 } else { frame as usize + 1 };
    assert_eq!(frames.len(), size * rendered, "the render's frames");
    frames[frames.len() - size..].to_vec()
}

/// The same frame from the compositor, as RGB.
fn composed(project: &Project, frame: i64) -> Vec<u8> {
    let compositor = Compositor::new(project, Catalog::builtin(), &ComposeOptions::default())
        .unwrap_or_else(|error| panic!("{error}"));
    let picture = compositor.frame(compositor.rate().time_of(frame), 1.0);
    assert!(
        compositor.problems().is_empty(),
        "{:?}",
        compositor.problems()
    );
    picture
        .rgba
        .as_chunks::<4>().0.iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

/// How far apart two pictures are, out of 255.
#[derive(Debug)]
struct Difference {
    /// Mean absolute difference in luma.
    luma: f64,
    /// The share of pixels whose luma is off by more than 32.
    off: f64,
    /// Mean absolute difference per color channel.
    color: f64,
}

fn difference(a: &[u8], b: &[u8]) -> Difference {
    assert_eq!(a.len(), b.len(), "the pictures differ in size");
    let luma =
        |p: &[u8]| 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]);
    let lumas: Vec<f64> = a
        .as_chunks::<3>().0.iter()
        .zip(b.as_chunks::<3>().0.iter())
        .map(|(x, y)| (luma(x) - luma(y)).abs())
        .collect();
    let color: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    Difference {
        luma: lumas.iter().sum::<f64>() / lumas.len() as f64,
        off: lumas.iter().filter(|d| **d > 32.0).count() as f64 / lumas.len() as f64,
        color: color as f64 / a.len() as f64,
    }
}

/// A sequence body, the frames compared, and how far they may differ: in
/// mean luma, in the share of pixels off, and in mean color.
struct Case {
    name: &'static str,
    body: &'static str,
    frames: &'static [i64],
    /// The graph renders each frame alone (see `graph`).
    window: bool,
    luma: f64,
    off: f64,
    color: f64,
    /// Kerf's renderer's tolerance, when it differs: luma, off, color.
    kerf: Option<(f64, f64, f64)>,
}

/// With `KERF_PARITY_DUMP` set to a folder, both pictures of every frame
/// compared are written there to look at.
fn dump(project: &Project, name: &str, frame: i64, graph: &[u8], composed: &[u8], by: &str) {
    let Some(folder) = std::env::var_os("KERF_PARITY_DUMP") else {
        return;
    };
    let sequence = program_sequence(project, None).expect("a sequence");
    for (which, rgb) in [("graph", graph), (by, composed)] {
        let path =
            Path::new(&folder).join(format!("{}-{frame}-{which}.png", name.replace(' ', "-")));
        let mut child = ffmpeg()
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
            .arg(format!("{}x{}", sequence.width, sequence.height))
            .args(["-i", "-", "-frames:v", "1", "-update", "1"])
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("ffmpeg runs");
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), rgb).unwrap();
        child.wait().unwrap();
    }
}

fn check(cases: &[Case]) {
    let dir = tempfile::tempdir().unwrap();
    let media = media(dir.path());
    let mut failures = Vec::new();
    for case in cases {
        let project = project(&media, case.body);
        for &frame in case.frames {
            let graph = graph(&project, dir.path(), frame, case.window);
            let composed = composed(&project, frame);
            dump(&project, case.name, frame, &graph, &composed, "kerf");
            let difference = difference(&graph, &composed);
            eprintln!("{} frame {frame}: {difference:?}", case.name);
            let (luma, off, color) = case.kerf.unwrap_or((case.luma, case.off, case.color));
            if difference.luma > luma || difference.off > off || difference.color > color {
                failures.push(format!("{} frame {frame}: {difference:?}", case.name));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn cuts_fit_speed_and_nesting_match_the_graph() {
    check(&[
        Case {
            name: "cuts",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=12f at=0f dur=24f\n\
                   \x20   clip c2 media=t src=60f at=24f dur=24f\n",
            window: false,
            frames: &[0, 11, 23, 24, 40],
            luma: 0.5,
            off: 0.001,
            color: 1.0,
            kerf: None,
        },
        Case {
            name: "fit",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=b src=0f at=0f dur=24f\n",
            window: false,
            frames: &[5],
            luma: 1.0,
            off: 0.001,
            color: 2.5,
            kerf: None,
        },
        Case {
            name: "speed",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f speed=2\n\
                   \x20   clip c2 media=t src=0f at=24f dur=24f speed=1/2\n\
                   \x20   clip c3 media=t src=10f at=48f dur=24f speed=-1\n",
            window: true,
            frames: &[7, 30, 48, 60],
            luma: 0.5,
            off: 0.001,
            color: 1.0,
            kerf: None,
        },
        Case {
            name: "other rate",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=h src=0f at=0f dur=48f\n",
            window: false,
            frames: &[1, 5, 30],
            luma: 0.5,
            off: 0.001,
            color: 1.0,
            kerf: None,
        },
        Case {
            name: "nested",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip n1 seq=inner src=6f at=0f dur=24f\n\
                   \x20   clip n2 seq=inner src=0f at=24f dur=24f speed=-1\n\
                   sequence inner rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip i1 media=t src=30f at=0f dur=48f\n",
            window: false,
            frames: &[3, 30],
            luma: 1.0,
            off: 0.001,
            color: 2.0,
            kerf: None,
        },
    ]);
}

#[test]
fn transforms_and_opacity_match_the_graph() {
    check(&[
        Case {
            name: "scale and move",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=b src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip c2 media=t src=0f at=0f dur=24f\n\
                   \x20     set scale=0.5 x=0.2 y=-0.1\n",
            window: false,
            frames: &[4],
            luma: 2.5,
            off: 0.02,
            color: 6.0,
            kerf: None,
        },
        Case {
            name: "flip and rotate",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f\n\
                   \x20     set scale_x=-1 rotation=8\n",
            window: false,
            frames: &[4],
            luma: 2.0,
            off: 0.015,
            color: 6.0,
            kerf: Some((3.0, 0.02, 5.0)),
        },
        // The graph's overlay keeps the size of the first picture it gets,
        // so it cannot animate scale, and its rotation cuts off a turned
        // picture's corners; the compositor does neither.
        Case {
            name: "animated move",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=b src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip c2 media=t src=0f at=0f dur=24f\n\
                   \x20     set x={0f:-0.2 23f:0.2} y={0f:0.1 23f:-0.1} opacity={0f:0.4 23f:1} scale=0.6\n",
            window: false,
            frames: &[6, 17],
            luma: 2.5,
            off: 0.02,
            color: 6.0,
            kerf: Some((11.0, 0.15, 12.0)),
        },
        Case {
            name: "crop",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f\n\
                   \x20     set crop_l=0.25 crop_b=0.2\n",
            window: false,
            frames: &[4],
            luma: 2.0,
            off: 0.005,
            color: 4.5,
            kerf: Some((2.0, 0.012, 4.0)),
        },
        Case {
            name: "opacity",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=b src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip c2 media=t src=0f at=0f dur=24f\n\
                   \x20     set opacity={0f:0 23f:1}\n",
            window: false,
            frames: &[6, 18],
            luma: 1.5,
            off: 0.002,
            color: 3.0,
            kerf: Some((25.0, 0.25, 28.0)),
        },
        Case {
            name: "solid",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
                   \x20     set color=#3060c080 scale=0.5\n",
            window: false,
            frames: &[4],
            luma: 1.0,
            off: 0.002,
            color: 2.0,
            kerf: Some((7.0, 0.002, 8.0)),
        },
    ]);
}

#[test]
fn transitions_match_the_graph() {
    check(&[
        Case {
            name: "dissolve",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=12f at=0f dur=24f\n\
                   \x20   clip c2 media=b src=12f at=24f dur=24f\n\
                   \x20   transition x1 from=c1 to=c2 fx=dissolve@1 dur=12f\n",
            window: false,
            frames: &[19, 24, 28],
            luma: 2.0,
            off: 0.002,
            color: 3.0,
            kerf: Some((25.0, 0.3, 28.0)),
        },
        Case {
            name: "wipe",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=12f at=0f dur=24f\n\
                   \x20   clip c2 media=b src=12f at=24f dur=24f\n\
                   \x20   transition x1 from=c1 to=c2 fx=wipe@1 dur=12f angle=30 softness=0.2\n",
            window: false,
            frames: &[20, 24, 27],
            luma: 2.0,
            off: 0.005,
            color: 4.0,
            kerf: Some((6.0, 0.06, 8.0)),
        },
        Case {
            name: "dip",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=12f at=0f dur=24f\n\
                   \x20   clip c2 media=b src=12f at=24f dur=24f\n\
                   \x20   transition x1 from=c1 to=c2 fx=dip@1 dur=12f color=#ffffff\n",
            window: false,
            frames: &[20, 24, 27],
            luma: 2.0,
            off: 0.002,
            color: 3.0,
            kerf: Some((42.0, 0.7, 42.0)),
        },
    ]);
}

#[test]
fn effects_match_the_graph() {
    check(&[
        Case {
            name: "color",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f\n\
                   \x20     fx color.basic exposure=0.3 contrast=1.2 saturation=1.4 temperature=300 vibrance=0.3\n",
            window: false,
            frames: &[4],
            luma: 2.0,
            off: 0.015,
            color: 3.0,
            kerf: None,
        },
        Case {
            name: "blur",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f\n\
                   \x20     fx blur.gaussian radius=6\n",
            window: false,
            frames: &[4],
            luma: 2.5,
            off: 0.002,
            // The graph blurs 4:2:0 color at half resolution, so twice as
            // far; luma is blurred alike.
            color: 10.0,
            kerf: Some((10.5, 0.1, 12.5)),
        },
        // Media shown smaller is decoded at the size it is shown, and its
        // effects run there.
        Case {
            name: "color scaled",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=b src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip c2 media=t src=0f at=0f dur=24f\n\
                   \x20     set scale=0.4 x=0.25 y=0.25\n\
                   \x20     fx color.basic exposure=0.3 contrast=1.2 saturation=1.4\n",
            window: false,
            frames: &[4],
            luma: 2.5,
            off: 0.02,
            color: 6.0,
            kerf: None,
        },
        Case {
            name: "blur scaled",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=b src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip c2 media=t src=0f at=0f dur=24f\n\
                   \x20     set scale=0.5 x=-0.2\n\
                   \x20     fx blur.gaussian radius=3\n",
            window: false,
            frames: &[4],
            luma: 2.5,
            off: 0.02,
            color: 10.0,
            kerf: Some((3.5, 0.03, 10.0)),
        },
        Case {
            name: "raster",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f\n\
                   \x20     fx color.huesaturation saturation={0f:0 23f:-100}\n",
            window: false,
            frames: &[0, 12],
            luma: 2.0,
            off: 0.002,
            color: 3.0,
            kerf: None,
        },
        Case {
            name: "raster moved",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=b src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip c2 media=t src=0f at=0f dur=24f\n\
                   \x20     set scale=0.5 x=0.2\n\
                   \x20     fx color.huesaturation saturation=-50\n",
            window: false,
            frames: &[9],
            luma: 2.5,
            off: 0.02,
            color: 6.0,
            kerf: None,
        },
        Case {
            name: "adjustment",
            body: "sequence s1 rate=24 size=320x180 audio=48000\n\
                   \x20 track V1 video\n\
                   \x20   clip c1 media=t src=0f at=0f dur=24f\n\
                   \x20 track V2 video\n\
                   \x20   clip a1 gen=adjustment@1 at=0f dur=24f\n\
                   \x20     set opacity=0.7\n\
                   \x20     fx color.basic saturation=0 brightness=0.1\n",
            window: false,
            frames: &[4],
            luma: 2.0,
            off: 0.002,
            color: 2.0,
            kerf: Some((12.5, 0.002, 15.5)),
        },
    ]);
}
