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
