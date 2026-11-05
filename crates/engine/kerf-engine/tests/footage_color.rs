//! What importing finds out about real files: depth and color, from what FFmpeg wrote in them.

use kerf_model::ColorSpace;
use std::path::Path;
use std::process::Command;

fn encode(path: &Path, args: &[&str]) {
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc2=s=64x36:r=24:d=1"])
        .args(args)
        .arg(path)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn importing_finds_depth_and_color_in_real_files() {
    let dir = tempfile::tempdir().expect("a folder");
    // `setparams` tags the frames, which is what reaches the file, as a camera's would.
    for (name, tags, space) in [
        ("plain.mov", None, ColorSpace::Rec709),
        (
            "bt709.mov",
            Some("color_trc=bt709:color_primaries=bt709:colorspace=bt709"),
            ColorSpace::Rec709,
        ),
        (
            "hlg.mov",
            Some("color_trc=arib-std-b67:color_primaries=bt2020:colorspace=bt2020nc"),
            ColorSpace::Rec2100Hlg,
        ),
        (
            "pq.mov",
            Some("color_trc=smpte2084:color_primaries=bt2020:colorspace=bt2020nc"),
            ColorSpace::Rec2100Pq,
        ),
        (
            "wide.mov",
            Some("color_trc=bt709:color_primaries=bt2020:colorspace=bt2020nc"),
            ColorSpace::Rec2020,
        ),
    ] {
        let path = dir.path().join(name);
        let filter = match tags {
            Some(tags) => format!("setparams={tags},format=yuv422p10le"),
            None => "format=yuv422p10le".to_owned(),
        };
        encode(&path, &["-vf", &filter, "-c:v", "prores_ks"]);
        let info = kerf_engine::info(path.to_str().expect("a path")).expect("probes");
        let video = info.video_info(64, 36);
        assert_eq!((video.color, video.bits), (space, 10), "{name}");
    }
    let eight = dir.path().join("eight.mp4");
    encode(&eight, &["-pix_fmt", "yuv420p", "-c:v", "libx264"]);
    let video = kerf_engine::info(eight.to_str().expect("a path"))
        .expect("probes")
        .video_info(64, 36);
    assert_eq!((video.color, video.bits), (ColorSpace::Rec709, 8));
    assert!(!video.is_deep());
}

/// The widest step between luma levels along the middle row of a video's first frame, read at 10
/// bits: how wide its bands are.
fn widest_step(path: &Path) -> u16 {
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray10le",
            "-",
        ])
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success());
    let row = 320 * 90 * 2;
    let levels: std::collections::BTreeSet<u16> = output.stdout[row..row + 320 * 2]
        .chunks_exact(2)
        .map(|sample| u16::from_le_bytes([sample[0], sample[1]]))
        .collect();
    let levels: Vec<u16> = levels.into_iter().collect();
    levels
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .max()
        .unwrap_or(0)
}

#[test]
fn a_master_keeps_the_depth_of_its_footage() {
    let dir = tempfile::tempdir().expect("a folder");
    let ramp = dir.path().join("ramp.mov");
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    // Worked out at 16 bits, dim, in 10-bit ProRes.
    let status = Command::new(program)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("nullsrc=s=320x180:r=24:d=1,format=gray16le,geq=lum='X*12288/320'")
        .args(["-pix_fmt", "yuv422p10le", "-c:v", "prores_ks"])
        .arg(&ramp)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    let render = |fields: &str, preset: kerf_engine::render::RenderPreset, name: &str| {
        let document = format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180{fields}\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n      fx color.levels in_white=48\n",
            ramp.display()
        );
        let project = kerf_lang::parse_document(&document, kerf_model::Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let output = dir.path().join(name);
        kerf_engine::render::render_program(
            &project,
            None,
            None,
            &kerf_engine::render::RenderOptions {
                output: output.display().to_string(),
                preset,
                force_software: true,
                ..Default::default()
            },
            None::<fn(f64)>,
        )
        .expect("renders");
        widest_step(&output)
    };
    use kerf_engine::render::RenderPreset;
    let deep = render(" bits=10", RenderPreset::ProRes422Hq, "deep.mov");
    let thin = render("", RenderPreset::ProRes422Hq, "thin.mov");
    assert!(deep * 2 <= thin, "bands of {deep} against {thin} levels");
}
