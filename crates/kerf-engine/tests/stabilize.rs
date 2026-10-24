//! Warp Stabilizer analyses: made once from a shaky clip, kept in the
//! cache, and drawn from.

use kerf_compose::{ComposeOptions, Compositor, Device};
use kerf_engine::derivatives::DerivativeCache;
use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::stabilize::ANALYSIS;
use kerf_engine::{analyze_stabilizations, attach_stabilizations, CancellationToken};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Param, Project, Rate, Value};
use std::process::Command;

/// A test pattern that jumps a few pixels every frame, as a handheld
/// camera shakes.
fn shaky(dir: &std::path::Path) -> String {
    let path = dir.join("shaky.mp4");
    let output = Command::new(resolve_ffmpeg_bin())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("testsrc2=s=400x240:r=24:d=1")
        .args(["-vf", "crop=320:180:40+12*sin(n*1.9):30+9*cos(n*2.7)"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "6"])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path.display().to_string()
}

fn project(path: &str, framing: &str) -> Project {
    let text = format!(
        "kerf 1\nproject name=\"shake\"\n\
         media m path=\"{path}\" rate=24 dur=24f video=320x180\n\
         sequence s1 rate=24 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m src=0f at=0f dur=24f\n\
         \x20     fx distort.warpstabilizer {framing}\n"
    );
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

/// Every file under `dir` with when it was last written.
fn files(dir: &std::path::Path) -> Vec<(std::path::PathBuf, std::time::SystemTime)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files(&path));
        } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            out.push((path, modified));
        }
    }
    out.sort();
    out
}

fn analysis(project: &Project) -> Option<String> {
    match project.clip("c1")?.effects[0].params.get(ANALYSIS)? {
        Param::Static(Value::Text(json)) => Some(json.clone()),
        _ => None,
    }
}

#[test]
fn a_shaky_clip_is_analysed_once_and_drawn_steady() {
    let dir = tempfile::tempdir().unwrap();
    let path = shaky(dir.path());
    let cache = DerivativeCache::new(dir.path());
    let settings = "stabilization/result=no-motion stabilization/method=position borders/framing=stabilize-only";
    let mut shaky = project(&path, settings);
    // Nothing cached yet: the clip is named, and nothing is attached.
    assert_eq!(attach_stabilizations(&mut shaky.clone(), &cache), ["c1"]);
    analyze_stabilizations(&shaky, &cache, &CancellationToken::new()).unwrap();
    assert!(attach_stabilizations(&mut shaky, &cache).is_empty());
    let json = analysis(&shaky).expect("attached");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["frames"].as_array().unwrap().len(), 24);
    assert_eq!(parsed["size"], serde_json::json!([320.0, 180.0]));
    // Analysed once: a second pass finds it cached and writes nothing.
    let before = files(dir.path());
    analyze_stabilizations(&shaky, &cache, &CancellationToken::new()).unwrap();
    assert_eq!(files(dir.path()), before);
    // The stabilized frames hold still where the source jumps: neighbouring
    // frames differ less than the shaky ones do.
    let frames = |project: &Project| {
        let compositor = Compositor::new(
            project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: None,
                captions: false,
                device: Device::Cpu,
            },
        )
        .unwrap();
        (8..12)
            .map(|at| compositor.frame(Rate::FPS_24.time_of(at), 1.0).rgba)
            .collect::<Vec<_>>()
    };
    let jitter = |frames: &[Vec<u8>]| -> f64 {
        // The middle of the picture, where the stabilized one keeps its pixels.
        let centre = |frame: &[u8]| -> Vec<f64> {
            (60..120)
                .flat_map(|y| (100..220).map(move |x| (y * 320 + x) * 4))
                .map(|i| f64::from(frame[i]))
                .collect()
        };
        frames
            .windows(2)
            .map(|pair| {
                let (a, b) = (centre(&pair[0]), centre(&pair[1]));
                a.iter().zip(&b).map(|(a, b)| (a - b).abs()).sum::<f64>() / a.len() as f64
            })
            .sum()
    };
    let unstabilized = project(&path, settings);
    let (steady, moving) = (jitter(&frames(&shaky)), jitter(&frames(&unstabilized)));
    assert!(steady < moving * 0.7, "{steady} vs {moving}");
}
