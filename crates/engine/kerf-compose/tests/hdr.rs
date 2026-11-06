//! A sequence finished in HDR: what its frames hold for a master, and what a monitor gets.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

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

fn white(color: &str) -> String {
    format!(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000{color}\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff\n"
    )
}

fn code16(frame: &Frame, x: u32, y: u32) -> u16 {
    let at = ((y * frame.width + x) * 8) as usize;
    u16::from_le_bytes([frame.rgba[at], frame.rgba[at + 1]])
}

fn signal(document: &str, accel: Accel) -> f64 {
    let frame = compositor(document, accel).frame16(Rate::FPS_24.time_of(1), 1.0);
    f64::from(code16(&frame, 32, 18)) / 65535.0
}

#[test]
fn sdr_white_sits_at_reference_white_in_an_hdr_master() {
    for accel in [Accel::Never, Accel::Always] {
        assert_eq!(signal(&white(""), accel), 1.0);
        let pq = signal(&white(" color=rec2100-pq"), accel);
        assert!((pq - 0.58).abs() < 0.005, "{pq}");
        let hlg = signal(&white(" color=rec2100-hlg"), accel);
        assert!((hlg - 0.75).abs() < 0.01, "{hlg}");
    }
}

/// A flat 10-bit PQ signal, as a camera or a grade would leave it.
fn pq_footage(dir: &Path, gray: u8) -> String {
    let path = dir.join(format!("pq{gray}.mov"));
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let source = format!("color=c=0x{gray:02x}{gray:02x}{gray:02x}:s=320x180:r=24:d=1");
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(source)
        .args([
            "-vf",
            "scale=in_range=pc:out_range=tv,format=yuv422p10le,setparams=range=tv:colorspace=bt2020nc:color_primaries=bt2020:color_trc=smpte2084",
            "-c:v",
            "prores_ks",
        ])
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

/// The first channel of the file's first pixel, as FFmpeg itself decodes it.
fn decoded_signal(path: &str) -> f64 {
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-i", path])
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb48le",
            "-",
        ])
        .output()
        .expect("ffmpeg runs");
    f64::from(u16::from_le_bytes([output.stdout[0], output.stdout[1]])) / 65535.0
}

fn over_footage(path: &str, color: &str) -> String {
    format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{path}\" rate=24 dur=24f video=320x180 bits=10 color=rec2100-pq\nsequence s1 rate=24 size=320x180 audio=48000{color}\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n"
    )
}

#[test]
fn hdr_footage_keeps_its_highlights_in_an_hdr_sequence_and_comes_down_for_a_monitor() {
    let dir = tempfile::tempdir().expect("a folder");
    // About 1100 cd/m² on the PQ curve, far above SDR white.
    let path = pq_footage(dir.path(), 0xD0);
    let shown = decoded_signal(&path);
    assert!(shown > 0.75, "{shown}");
    for accel in [Accel::Never, Accel::Always] {
        let hdr = over_footage(&path, " color=rec2100-pq");
        let held = signal(&hdr, accel);
        assert!((held - shown).abs() < 0.01, "{held} against {shown}");
        let sdr = signal(&over_footage(&path, ""), accel);
        assert!((sdr - 1.0).abs() < 0.02, "{sdr}");
        // What a monitor gets: no brighter than white, however bright the master.
        let monitor = compositor(&hdr, accel).frame(Rate::FPS_24.time_of(1), 1.0);
        let at = ((90 * monitor.width + 160) * 4) as usize;
        assert!(monitor.rgba[at] > 240, "{}", monitor.rgba[at]);
    }
}

#[test]
fn the_gpu_and_the_cpu_finish_an_hdr_frame_alike() {
    for color in [" color=rec2100-pq", " color=rec2100-hlg"] {
        // Cross-fading colors, some of them mixed, with sizes and opacity on top.
        let document = format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000{color}\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#3070c0\n  track V2 video\n    clip g2 gen=solid@1 at=0f dur=24f\n      set color=#e0a040 opacity=0.6 scale=0.5\n"
        );
        let gpu = compositor(&document, Accel::Always);
        let cpu = compositor(&document, Accel::Never);
        let at = Rate::FPS_24.time_of(1);
        let (g, c) = (gpu.frame16(at, 1.0), cpu.frame16(at, 1.0));
        assert_eq!(g.rgba.len(), c.rgba.len());
        for (g, c) in g.rgba.chunks_exact(2).zip(c.rgba.chunks_exact(2)) {
            let g = u16::from_le_bytes([g[0], g[1]]);
            let c = u16::from_le_bytes([c[0], c[1]]);
            assert!(g.abs_diff(c) <= 64, "{g} against {c} in{color}");
        }
        let (g, c) = (gpu.frame(at, 1.0), cpu.frame(at, 1.0));
        for (g, c) in g.rgba.iter().zip(&c.rgba) {
            assert!(g.abs_diff(*c) <= 2, "{g} against {c} in{color}");
        }
    }
}

#[test]
fn a_frame_kept_for_one_color_is_not_used_for_another() {
    let at = Rate::FPS_24.time_of(1);
    let key = |color: &str| {
        compositor(&white(color), Accel::Never)
            .frame_key(at, 1.0)
            .expect("a key")
    };
    assert_ne!(key(""), key(" color=rec2100-pq"));
    assert_ne!(key(" color=rec2100-hlg"), key(" color=rec2100-pq"));
}
