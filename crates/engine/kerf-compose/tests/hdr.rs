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
    f64::from(code16(&frame, frame.width / 2, frame.height / 2)) / 65535.0
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
        for (g, c) in g.rgba.as_chunks::<2>().0.iter().zip(c.rgba.as_chunks::<2>().0.iter()) {
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

#[test]
fn effects_written_for_0_to_1_keep_what_an_hdr_picture_holds() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = pq_footage(dir.path(), 0xD0);
    let footage = over_footage(&path, " color=rec2100-pq");
    let shown = signal(&footage, Accel::Never);
    // Each way of holding values: a grade that knows HDR, each pixel on its own, the whole picture.
    for fx in [
        "color.lumetri",
        "color.levels",
        "color.tint",
        "color.gamma",
        "key.color",
        "stylize.vignette",
        "blur.radialzoom",
        "distort.upscale",
    ] {
        let document = format!("{footage}      fx {fx}\n");
        for accel in [Accel::Never, Accel::Always] {
            let held = signal(&document, accel);
            assert!(
                (held - shown).abs() < 0.01,
                "{fx} on {accel:?} made {held} of {shown}"
            );
        }
    }
}

#[test]
fn the_gpu_holds_what_is_over_1_for_effects_as_the_cpu_does() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = pq_footage(dir.path(), 200);
    let line = |effect: &str| {
        format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{path}\" rate=24 dur=24f video=320x180 bits=10 color=rec2100-pq\nsequence s1 rate=24 size=320x180 audio=48000 color=rec2100-pq\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n      fx {effect}\n"
        )
    };
    let at = Rate::FPS_24.time_of(1);
    let plain = signal(&line("color.tint amount=0"), Accel::Never);
    // A blur has a kernel at every depth, so this asks whether there is a GPU at all.
    let has_gpu = compositor(&line("blur.gaussian radius=2"), Accel::Always).draws_on_gpu(at, 1.0);
    for effect in [
        "color.brightnesscontrast contrast=10",
        "color.gamma gamma=12",
        "color.levels",
        "color.tint amount=60",
        "key.luma threshold=0 cutoff=0",
    ] {
        let document = line(effect);
        let gpu = compositor(&document, Accel::Always);
        let cpu = compositor(&document, Accel::Never);
        assert!(
            gpu.draws_on_gpu(at, 1.0) || !has_gpu,
            "{effect} goes to the CPU"
        );
        let (on_gpu, on_cpu) = (gpu.frame16(at, 1.0), cpu.frame16(at, 1.0));
        let (x, y) = (on_gpu.width / 2, on_gpu.height / 2);
        let (a, b) = (code16(&on_gpu, x, y), code16(&on_cpu, x, y));
        assert!(
            a.abs_diff(b) < 300,
            "{effect}: {a} on the GPU, {b} on the CPU"
        );
        // It kept something over SDR white, which sits near 0.58 in PQ.
        if effect != "color.tint amount=60" {
            assert!(
                f64::from(a) / 65535.0 > 0.6,
                "{effect} lost its highlights: {a}"
            );
        }
    }
    assert!(plain > 0.6, "the footage should be over SDR white: {plain}");
    // Bringing a whole picture under 1 goes by its peak, which the CPU has to read.
    assert!(!compositor(&line("blur.radialzoom amount=20"), Accel::Always).draws_on_gpu(at, 1.0));
}

#[test]
fn lumetri_grades_highlights_in_an_hdr_sequence_alike_on_gpu_and_cpu() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = pq_footage(dir.path(), 0xD0);
    let footage = over_footage(&path, " color=rec2100-pq");
    let shown = signal(&footage, Accel::Never);
    let document = format!("{footage}      fx color.lumetri exposure=-1\n");
    let cpu = signal(&document, Accel::Never);
    let gpu = signal(&document, Accel::Always);
    assert!(cpu < shown - 0.02, "{cpu} of {shown}");
    assert!(
        (cpu - gpu).abs() < 0.005,
        "{cpu} on the CPU, {gpu} on the GPU"
    );
}

/// A flat 10-bit PQ picture in the color `rgb` of BT.2020, with what its signal is as code values.
fn flat_footage(dir: &Path, name: &str, rgb: &str) -> String {
    let path = dir.join(format!("{name}.mov"));
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=0x{rgb}:s=128x72:r=24:d=1"))
        .args([
            "-vf",
            "scale=in_range=pc:out_range=tv:out_color_matrix=bt2020nc,format=yuv422p10le,setparams=range=tv:colorspace=bt2020nc:color_primaries=bt2020:color_trc=smpte2084",
            "-c:v",
            "prores_ks",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success());
    path.display().to_string()
}

/// The signal of each channel in the middle of a frame.
fn signals(document: &str, accel: Accel) -> [f64; 3] {
    let frame = compositor(document, accel).frame16(Rate::FPS_24.time_of(1), 1.0);
    let at = ((frame.height / 2 * frame.width + frame.width / 2) * 8) as usize;
    [0, 2, 4].map(|channel| {
        f64::from(u16::from_le_bytes([
            frame.rgba[at + channel],
            frame.rgba[at + channel + 1],
        ])) / 65535.0
    })
}

fn pq_document(path: &str, fx: &str) -> String {
    let line = if fx.is_empty() {
        String::new()
    } else {
        format!("      fx {fx}\n")
    };
    format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{path}\" rate=24 dur=24f video=128x72 bits=10 color=rec2100-pq\nsequence s1 rate=24 size=128x72 audio=48000 color=rec2100-pq\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n{line}"
    )
}

/// Whether frames with `fx` on it would run on the GPU at all, for the GPU's half of a test: Linux CI
/// has none.
fn has_gpu_path(document: &str) -> bool {
    compositor(document, Accel::Always).draws_on_gpu(Rate::FPS_24.time_of(1), 1.0)
}

#[test]
fn looks_built_on_a_basic_correction_keep_highlights_apart() {
    let dir = tempfile::tempdir().expect("a folder");
    let low = flat_footage(dir.path(), "low", "C0C0C0");
    let high = flat_footage(dir.path(), "high", "E0E0E0");
    let plain = (
        signals(&pq_document(&low, ""), Accel::Never)[1],
        signals(&pq_document(&high, ""), Accel::Never)[1],
    );
    // SDR white is at 0.58, and three of these clipped there, each at the same place.
    assert!(plain.0 > 0.68 && plain.1 > 0.78, "{plain:?}");
    for look in [
        "look.noir@1",
        "look.vhs@1",
        "look.flash@1",
        "color.basic@1 contrast=1.2 saturation=0.8",
        "noise.noise amount=10",
    ] {
        for accel in [Accel::Never, Accel::Always] {
            let (a, b) = (
                signals(&pq_document(&low, look), accel)[1],
                signals(&pq_document(&high, look), accel)[1],
            );
            assert!(a > 0.65 && b > 0.75, "{look} on {accel:?}: {a} {b}");
            assert!(b - a > 0.07, "{look} on {accel:?}: {a} {b}");
        }
    }
    // On the GPU where it has a path, alike.
    let document = pq_document(&high, "look.noir@1");
    if has_gpu_path(&document) {
        let (cpu, gpu) = (
            signals(&document, Accel::Never),
            signals(&document, Accel::Always),
        );
        for (cpu, gpu) in cpu.iter().zip(&gpu) {
            assert!((cpu - gpu).abs() < 0.01, "{cpu} {gpu}");
        }
    }
}

#[test]
fn effects_written_for_0_to_1_keep_colors_past_bt709() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = flat_footage(dir.path(), "wide", "00C000");
    let plain = signals(&pq_document(&path, ""), Accel::Never);
    // Green of BT.2020, as little of the other two as a flat 4:2:2 picture leaves. Clipped to BT.709,
    // they rise to 0.62 and 0.48.
    assert!(
        plain[0] < 0.1 && plain[2] < 0.1 && plain[1] > 0.7,
        "{plain:?}"
    );
    // Each way of holding values, and what on the GPU has a path.
    for fx in [
        "color.asccdl",
        "color.brightnesscontrast",
        "color.channelmixer",
        "color.gamma",
        "color.procamp",
        "stylize.vignette",
        "key.color",
        "blur.radialzoom",
        "distort.upscale",
        "noise.removegrain",
        "blur.sharpen",
        "blur.unsharp",
    ] {
        let document = pq_document(&path, fx);
        for accel in [Accel::Never, Accel::Always] {
            let held = signals(&document, accel);
            for (channel, (held, plain)) in held.iter().zip(&plain).enumerate() {
                assert!(
                    (held - plain).abs() < 0.02,
                    "{fx} on {accel:?}: channel {channel} is {held} of {plain}"
                );
            }
        }
    }
}

#[test]
fn a_grade_keeps_colors_past_bt709_as_it_changes_them() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = flat_footage(dir.path(), "wide", "00C000");
    let plain = signals(&pq_document(&path, ""), Accel::Never);
    // A grade that changes little of green's brightness, and a mono one that takes all of its color.
    let warm = signals(
        &pq_document(&path, "color.lumetri temperature=5"),
        Accel::Never,
    );
    assert!(warm[0] < 0.35 && warm[2] < 0.2, "{warm:?} of {plain:?}");
    let document = pq_document(&path, "look.grade.neutral-mono@1");
    for accel in [Accel::Never, Accel::Always] {
        let mono = signals(&document, accel);
        assert!(
            (mono[0] - mono[1]).abs() < 0.01 && (mono[1] - mono[2]).abs() < 0.01,
            "{accel:?}: {mono:?}"
        );
    }
}

#[test]
fn an_sdr_picture_in_an_hdr_sequence_is_held_by_nothing() {
    // Under 1 and inside BT.709, no effect's hold touches it, so it meets an effect as it does in SDR.
    let solid = |color: &str, fx: &str| {
        format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000{color}\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#c08040\n      fx {fx}\n"
        )
    };
    for fx in [
        "color.gamma gamma=12",
        "blur.radialzoom amount=20",
        "blur.sharpen",
    ] {
        let sdr = compositor(&solid("", fx), Accel::Never).frame(Rate::FPS_24.time_of(1), 1.0);
        let again = compositor(&solid("", fx), Accel::Never).frame(Rate::FPS_24.time_of(1), 1.0);
        assert_eq!(sdr.rgba, again.rgba, "{fx}");
    }
}

/// A PQ signal in cd/m^2.
fn nits(signal: f64) -> f64 {
    let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
    let (c1, c2, c3) = (
        3424.0 / 4096.0,
        2413.0 / 4096.0 * 32.0,
        2392.0 / 4096.0 * 32.0,
    );
    let p = signal.powf(1.0 / m2);
    10000.0 * (((p - c1).max(0.0)) / (c2 - c3 * p)).powf(1.0 / m1)
}

/// A flat HLG picture of those code values, in BT.2020.
fn flat_hlg_footage(dir: &Path, name: &str, rgb: &str) -> String {
    let path = dir.join(format!("{name}.mov"));
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=0x{rgb}:s=128x72:r=24:d=1"))
        .args([
            "-vf",
            "scale=in_range=pc:out_range=tv:out_color_matrix=bt2020nc,format=yuv422p10le,setparams=range=tv:colorspace=bt2020nc:color_primaries=bt2020:color_trc=arib-std-b67",
            "-c:v",
            "prores_ks",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success());
    path.display().to_string()
}

fn hlg_document(path: &str, fx: &str) -> String {
    let line = if fx.is_empty() {
        String::new()
    } else {
        format!("      fx {fx}\n")
    };
    format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{path}\" rate=24 dur=24f video=128x72 bits=10 color=rec2100-hlg\nsequence s1 rate=24 size=128x72 audio=48000 color=rec2100-hlg\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n{line}"
    )
}

/// What each way of holding values does to what an HLG picture holds, on what draws it.
#[test]
fn effects_written_for_0_to_1_keep_what_an_hlg_picture_holds() {
    let dir = tempfile::tempdir().expect("a folder");
    let bright = flat_hlg_footage(dir.path(), "bright", "E8E8E8");
    let wide = flat_hlg_footage(dir.path(), "wide", "00C000");
    let high = signal(&hlg_document(&bright, ""), Accel::Never);
    // SDR white sits at 0.75 of the signal: this is above it.
    assert!(high > 0.8, "{high}");
    let green = signals(&hlg_document(&wide, ""), Accel::Never);
    assert!(
        green[0] < 0.1 && green[2] < 0.1 && green[1] > 0.6,
        "{green:?}"
    );
    for fx in [
        "color.lumetri",
        "color.levels",
        "color.tint",
        "color.gamma",
        "key.color",
        "stylize.vignette",
        "blur.radialzoom",
        "distort.upscale",
    ] {
        for accel in [Accel::Never, Accel::Always] {
            let held = signal(&hlg_document(&bright, fx), accel);
            assert!(
                (held - high).abs() < 0.015,
                "{fx} on {accel:?} made {held} of {high}"
            );
        }
    }
    for fx in [
        "color.asccdl",
        "color.brightnesscontrast",
        "color.channelmixer",
        "color.gamma",
        "color.procamp",
        "stylize.vignette",
        "key.color",
        "blur.radialzoom",
        "distort.upscale",
        "noise.removegrain",
        "blur.sharpen",
        "blur.unsharp",
    ] {
        for accel in [Accel::Never, Accel::Always] {
            let held = signals(&hlg_document(&wide, fx), accel);
            for (channel, (held, plain)) in held.iter().zip(&green).enumerate() {
                assert!(
                    (held - plain).abs() < 0.025,
                    "{fx} on {accel:?}: channel {channel} is {held} of {plain}"
                );
            }
        }
    }
}

#[test]
fn the_gpu_holds_an_hlg_picture_for_effects_as_the_cpu_does() {
    let dir = tempfile::tempdir().expect("a folder");
    let bright = flat_hlg_footage(dir.path(), "bright", "E8E8E8");
    let at = Rate::FPS_24.time_of(1);
    for fx in [
        "color.basic exposure=-0.3",
        "color.brightnesscontrast contrast=10",
        "color.gamma gamma=12",
        "color.tint amount=60",
        "blur.gaussian radius=2",
        "light.wonderglow",
    ] {
        let document = hlg_document(&bright, fx);
        let (gpu, cpu) = (
            compositor(&document, Accel::Always),
            compositor(&document, Accel::Never),
        );
        if !gpu.draws_on_gpu(at, 1.0) {
            assert!(
                !has_gpu_path(&hlg_document(&bright, "blur.gaussian radius=2")),
                "{fx}"
            );
            return;
        }
        let (on_gpu, on_cpu) = (gpu.frame16(at, 1.0), cpu.frame16(at, 1.0));
        let (x, y) = (on_gpu.width / 2, on_gpu.height / 2);
        let (a, b) = (code16(&on_gpu, x, y), code16(&on_cpu, x, y));
        assert!(a.abs_diff(b) < 300, "{fx}: {a} on the GPU, {b} on the CPU");
    }
}

#[test]
fn echoes_add_up_a_color_past_bt709_and_do_not_wash_it_out() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = flat_footage(dir.path(), "wide", "00C000");
    let plain = signals(&pq_document(&path, ""), Accel::Never).map(nits);
    // Three of a flat picture at 1, 0.5, and 0.25 are 1.75 of it, each channel alike. It was 1491 cd/m²
    // of red and 73 of blue on 757 of green, from what lifted it under 1 being added up with it.
    let echoed = signals(
        &pq_document(&path, "time.echo time=0.04 count=2 decay=0.5"),
        Accel::Never,
    )
    .map(nits);
    assert!(
        (echoed[1] / plain[1] - 1.75).abs() < 0.05,
        "{echoed:?} of {plain:?}"
    );
    assert!(echoed[0] < 5.0 && echoed[2] < 5.0, "{echoed:?}");
    // The same in HLG, on a signal it stays low in.
    let wide = flat_hlg_footage(dir.path(), "hlg", "00C000");
    let plain = signals(&hlg_document(&wide, ""), Accel::Never);
    let echoed = signals(
        &hlg_document(&wide, "time.echo time=0.04 count=2 decay=0.5"),
        Accel::Never,
    );
    assert!(echoed[1] > plain[1], "{echoed:?} of {plain:?}");
    assert!(echoed[0] < 0.1 && echoed[2] < 0.1, "{echoed:?}");
}

#[test]
fn an_echo_screened_keeps_what_lies_above_sdr_white() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = flat_footage(dir.path(), "wide", "00C000");
    let plain = signals(&pq_document(&path, ""), Accel::Never).map(nits);
    assert!(plain[1] > 400.0, "{plain:?}");
    // Two of a flat picture, at 1 and at a half, screened: white, and what each has past it, which the
    // second has little of. It stopped at SDR white, 203 cd/m², however bright what went in.
    let echoed = signals(
        &pq_document(
            &path,
            "time.echo time=0.04 count=1 decay=0.5 operator=screen",
        ),
        Accel::Never,
    )
    .map(nits);
    assert!(echoed[1] > plain[1] * 1.1, "{echoed:?} of {plain:?}");
    assert!(echoed[1] < plain[1] * 1.75, "{echoed:?} of {plain:?}");
    // And its color stays about what it was, past BT.709 as it is. It is not added up, so it moves a
    // little, where what is between 0 and white is screened and the rest is not.
    assert!(
        echoed[0] < 5.0 && echoed[2] < echoed[1] * 0.06,
        "{echoed:?}"
    );
}

#[test]
fn glows_add_light_to_a_color_past_bt709_and_leave_what_it_is() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = flat_footage(dir.path(), "wide", "00C000");
    let plain = signals(&pq_document(&path, ""), Accel::Never).map(nits);
    let accels = |fx: &str| {
        let on_gpu = has_gpu_path(&pq_document(&path, fx));
        if on_gpu {
            vec![Accel::Never, Accel::Always]
        } else {
            vec![Accel::Never]
        }
    };
    // White light adds alike to every channel, so green stands as far past the other two as it did,
    // or further where some of the light is its own.
    for fx in [
        "light.glint intensity=5 colorize=100 color=#ffffff",
        "light.glint intensity=5 colorize=50 color=#ffffff",
    ] {
        for accel in accels(fx) {
            let lit = signals(&pq_document(&path, fx), accel).map(nits);
            assert!(
                lit[0] > plain[0] + 20.0,
                "{fx} on {accel:?} added nothing: {lit:?}"
            );
            let (past, was) = (lit[1] - lit[0], plain[1] - plain[0]);
            assert!(
                past >= was * 0.95,
                "{fx} on {accel:?}: {past} past where it was {was}"
            );
        }
    }
    // Light taken from the picture itself is all green.
    for accel in accels("light.glint intensity=5 colorize=0") {
        let lit = signals(
            &pq_document(&path, "light.glint intensity=5 colorize=0"),
            accel,
        )
        .map(nits);
        assert!(
            lit[1] > plain[1] + 100.0 && lit[0] < 5.0 && lit[2] < 5.0,
            "{accel:?} {lit:?}"
        );
    }
}

/// A picture that moves, in SDR or as an HLG signal in BT.2020.
fn moving_footage(dir: &Path, hlg: bool) -> String {
    let path = dir.join(if hlg { "moving-hlg.mov" } else { "moving.mov" });
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let filter = if hlg {
        "scale=in_range=pc:out_range=tv:out_color_matrix=bt2020nc,format=yuv422p10le,setparams=range=tv:colorspace=bt2020nc:color_primaries=bt2020:color_trc=arib-std-b67"
    } else {
        "format=yuv422p10le"
    };
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=s=128x72:r=24:d=1")
        .args(["-vf", filter, "-c:v", "prores_ks"])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success());
    path.display().to_string()
}

/// How far `fx` moves the codes of a frame from what they are without it, on average.
fn moved_by(document: &dyn Fn(&str) -> String, fx: &str) -> f64 {
    let frame =
        |fx: &str| compositor(&document(fx), Accel::Never).frame16(Rate::FPS_24.time_of(11), 1.0);
    let (plain, with) = (frame(""), frame(fx));
    let codes = |frame: &Frame| -> Vec<f64> {
        frame
            .rgba
            .as_chunks::<2>()
            .0
            .iter()
            .map(|code| f64::from(u16::from_le_bytes(*code)))
            .collect()
    };
    let (plain, with) = (codes(&plain), codes(&with));
    plain
        .iter()
        .zip(&with)
        .map(|(a, b)| (a - b).abs())
        .sum::<f64>()
        / plain.len() as f64
}

/// An effect brought under 1 whole reads the clip's other frames brought under 1 the same way.
/// Rolling Shutter Repair once got them as they were, several times brighter than the frame it was
/// on, found nothing of that frame in them, and so repaired nothing in an HDR sequence.
#[test]
fn an_effect_reading_other_frames_reads_them_held_as_its_own() {
    let dir = tempfile::tempdir().expect("a folder");
    let (sdr, hlg) = (
        moving_footage(dir.path(), false),
        moving_footage(dir.path(), true),
    );
    let sdr_document = |fx: &str| {
        hlg_document(&sdr, fx)
            .replace(" bits=10 color=rec2100-hlg", " bits=10")
            .replace(" color=rec2100-hlg", "")
    };
    let hlg_document = |fx: &str| hlg_document(&hlg, fx);
    let fx = "distort.rollingshutterrepair";
    let in_sdr = moved_by(&sdr_document, fx);
    let in_hdr = moved_by(&hlg_document, fx);
    assert!(in_sdr > 20.0, "it does nothing to this shot: {in_sdr}");
    assert!(in_hdr > in_sdr / 2.0, "{in_hdr} in HDR, {in_sdr} in SDR");
}

/// How far two frames of 16 bits are apart: the mean of their codes' differences, and the share of
/// them further apart than 2 of an 8-bit frame's 255.
fn codes_apart(a: &Frame, b: &Frame) -> (f64, f64) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let codes = |frame: &Frame| -> Vec<u16> {
        let pairs = frame.rgba.as_chunks::<2>().0.iter();
        pairs.map(|code| u16::from_le_bytes(*code)).collect()
    };
    let (a, b) = (codes(a), codes(b));
    let apart = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b));
    let sum: f64 = apart.clone().map(f64::from).sum();
    let off = apart.filter(|apart| *apart > 514).count();
    (sum / a.len() as f64, off as f64 / a.len() as f64)
}

/// Effects an editor needs exact and no look or grade reaches: what a document writes under a clip
/// for each.
const EXACT_EFFECTS: &[&str] = &[
    "      fx distort.cornerpin upper_left_x=0.1 upper_left_y=0.15 upper_right_x=0.85 upper_right_y=0.05 lower_right_x=0.95 lower_right_y=0.9 lower_left_x=0.05 lower_left_y=0.8\n",
    // A shadow shows past what casts it, so of a picture with an outline.
    "      mask k1 ellipse=0.5,0.5,0.4,0.4\n      fx perspective.dropshadow distance=6 softness=4 direction=120 color=#ffffff opacity=80\n",
    "      fx utility.roundedcrop left=6 right=10 radius=20 border=3 border_color=#ffffff\n",
    "      fx transform.spacer left=10 radius=8 fill=true color=#ffffff\n",
    "      fx stylize.mosaic horizontal=12 vertical=7 softness=30\n",
    "      fx distort.lens curvature=35 h_decentering=15\n",
    "      fx distort.magnify center_x=0.4 center_y=0.45 size=30 magnification=250 feather=6\n",
    "      fx generate.fourcolor blend=200 opacity=50\n",
    "      fx generate.grid size=20 border=2 opacity=80\n",
    "      fx generate.gradientramp start_x=0.2 end_x=0.8 blend=50\n",
    "      fx generate.gradient shape=radial end_x=0.9 scatter=40 blend=50\n",
    "      fx generate.checkerboard width=16 mode=add opacity=60\n",
    // Written for 0 to 1, so each pixel is brought under 1 for them and put back.
    "      fx color.curves rgb=\"0,0 0.5,0.6 1,1\"\n",
    "      fx color.huesaturation hue=30 saturation=20\n",
    "      fx color.selectivecolor details/reds/redsCyan=40 details/neutrals/neutralsYellow=30\n",
    "      fx color.photofilter density=60\n",
    "      fx key.unmult blackLevel=10 softness=60\n",
    "      fx utility.cineon gamma=2 rolloff=30\n",
];

/// An HLG picture that moves, above SDR white in places and past BT.709 in others, through each of
/// [`EXACT_EFFECTS`]: what the GPU makes of it is what the CPU does, held or not as
/// `catalog::headroom` says of each.
#[test]
fn the_gpu_draws_an_hdr_picture_through_an_effect_as_the_cpu_does() {
    let dir = tempfile::tempdir().expect("a folder");
    let hlg = moving_footage(dir.path(), true);
    let at = Rate::FPS_24.time_of(11);
    if !has_gpu_path(&hlg_document(&hlg, "blur.gaussian radius=2")) {
        eprintln!("no GPU: nothing to compare");
        return;
    }
    let plain = compositor(&hlg_document(&hlg, ""), Accel::Never).frame16(at, 1.0);
    let over_white = plain.rgba.as_chunks::<8>().0.iter();
    let over_white = over_white.filter(|pixel| u16::from_le_bytes([pixel[2], pixel[3]]) > 52000);
    // SDR white is at 0.75 of the signal, 49151 of 65535.
    assert!(over_white.count() > 200, "the footage should go over SDR white");
    // What the two are apart before any effect, from decoding and finishing alone, which an effect
    // carries along and may spread. With one it may be twice that and a little, where an effect that
    // lost what the picture holds over 1 would be many times as far.
    let alone = compositor(&hlg_document(&hlg, ""), Accel::Always).frame16(at, 1.0);
    let (alone, alone_off) = codes_apart(&alone, &plain);
    eprintln!("no effect: mean {alone:.1} of 65535, {:.3}% off", alone_off * 100.0);
    for lines in EXACT_EFFECTS {
        let fx = lines.lines().last().unwrap_or(lines).trim();
        let document = format!("{}{lines}", hlg_document(&hlg, ""));
        let gpu = compositor(&document, Accel::Always);
        let cpu = compositor(&document, Accel::Never);
        assert!(gpu.draws_on_gpu(at, 1.0), "{fx} goes to the CPU");
        let (on_gpu, on_cpu) = (gpu.frame16(at, 1.0), cpu.frame16(at, 1.0));
        let (mean, off) = codes_apart(&on_gpu, &on_cpu);
        eprintln!("{fx}: mean {mean:.1} of 65535, {:.3}% off", off * 100.0);
        assert!(
            mean < alone * 2.0 + 20.0 && off < alone_off + 0.002,
            "{fx}: mean {mean:.1} of 65535, {:.3}% off by more than 514",
            off * 100.0
        );
        // The same clip with its effect left out.
        let without: String = lines
            .lines()
            .filter(|line| !line.trim_start().starts_with("fx "))
            .map(|line| format!("{line}\n"))
            .collect();
        let without = format!("{}{without}", hlg_document(&hlg, ""));
        let (moved, _) = codes_apart(&on_cpu, &compositor(&without, Accel::Never).frame16(at, 1.0));
        assert!(moved > 300.0, "{fx} does nothing to this picture: {moved}");
        // A device that failed has the CPU draw, which would pass for a match.
        assert!(gpu.draws_on_gpu(at, 1.0), "{fx} failed the device");
    }
}
