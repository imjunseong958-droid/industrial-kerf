//! Footage deeper than 8 bits, and footage whose code values mean something else: real files,
//! encoded by FFmpeg, read back through the compositor.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn ffmpeg(args: &[&str], out: &Path) {
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A dim horizontal ramp, `(pix_fmt, codec)`, 320x180: the kind of gradient that bands.
fn ramp(dir: &Path, name: &str, pix_fmt: &str, codec: &str) -> String {
    let path = dir.join(name);
    // Worked out at 16 bits, since FFmpeg's own gradients are 8-bit before anything sees them.
    let source = "nullsrc=s=320x180:r=24:d=1,format=gray16le,geq=lum='X*12288/320'";
    let mut args = vec![
        "-f", "lavfi", "-i", source, "-pix_fmt", pix_fmt, "-c:v", codec,
    ];
    if codec == "libx264" {
        args.extend(["-qp", "0"]);
    }
    ffmpeg(&args, &path);
    path.display().to_string()
}

/// A flat gray of code value `gray` (0..255 scale), tagged with `transfer`.
fn tagged_gray(dir: &Path, name: &str, gray: u8, transfer: &str) -> String {
    let path = dir.join(name);
    let source = format!("color=c=0x{gray:02x}{gray:02x}{gray:02x}:s=320x180:r=24:d=1");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &source,
            "-pix_fmt",
            "yuv422p10le",
            "-c:v",
            "prores_ks",
            "-color_trc",
            transfer,
            "-color_primaries",
            "bt2020",
            "-colorspace",
            "bt2020nc",
        ],
        &path,
    );
    path.display().to_string()
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

fn document(path: &str, fields: &str, body: &str) -> String {
    format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{path}\" rate=24 dur=24f video=320x180{fields}\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n{body}"
    )
}

fn at(frame: i64) -> kerf_model::Time {
    Rate::FPS_24.time_of(frame)
}

/// How many different red values a row holds.
fn levels(frame: &Frame) -> usize {
    let row = (frame.height / 2 * frame.width * 4) as usize;
    let mut seen = [false; 256];
    for x in 0..frame.width as usize {
        seen[usize::from(frame.rgba[row + x * 4])] = true;
    }
    seen.iter().filter(|seen| **seen).count()
}

fn center(frame: &Frame) -> [u8; 3] {
    let at = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

/// Lifts a dim ramp to the whole range, which shows what levels it really holds.
const LIFT: &str = "      fx color.levels in_white=48\n";

#[test]
fn ten_bits_keep_the_levels_a_grade_stretches() {
    let dir = tempfile::tempdir().expect("a folder");
    let eight = ramp(dir.path(), "eight.mp4", "yuv444p", "libx264");
    let ten = ramp(dir.path(), "ten.mov", "yuv422p10le", "prores_ks");
    let of = |path: &str, fields: &str, accel| {
        let drawn = compositor(&document(path, fields, LIFT), accel).frame(at(6), 1.0);
        levels(&drawn)
    };
    let eight_levels = of(&eight, "", Accel::Never);
    let ten_levels = of(&ten, " bits=10", Accel::Never);
    // Eight bits of a dim ramp hold about 48 levels; ten bits, about 190.
    assert!(eight_levels < 60, "{eight_levels} levels from 8 bits");
    assert!(
        ten_levels > eight_levels * 2,
        "{ten_levels} levels from 10 bits, {eight_levels} from 8"
    );
    // The file alone does not say: without `bits=` it is read as before.
    let unmarked = of(&ten, "", Accel::Never);
    assert!(unmarked < 60, "{unmarked} levels");
}

#[test]
fn ten_bit_sdr_looks_as_its_eight_bit_self_does() {
    let dir = tempfile::tempdir().expect("a folder");
    let ten = ramp(dir.path(), "ten.mov", "yuv422p10le", "prores_ks");
    let plain = compositor(&document(&ten, "", "      set scale=0.999\n"), Accel::Never);
    let deep = compositor(
        &document(&ten, " bits=10", "      set scale=0.999\n"),
        Accel::Never,
    );
    let (a, b) = (plain.frame(at(6), 1.0), deep.frame(at(6), 1.0));
    let worst = a
        .rgba
        .iter()
        .zip(&b.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(worst <= 2, "differs by {worst}");
}

#[test]
fn hdr_reference_white_lands_near_white_and_sdr_reading_it_would_not() {
    let dir = tempfile::tempdir().expect("a folder");
    // PQ's 58% is 203 nits, and HLG's 75% is too: both are HDR reference white.
    for (name, gray, transfer, space) in [
        ("pq.mov", 148u8, "smpte2084", "rec2100-pq"),
        ("hlg.mov", 191, "arib-std-b67", "rec2100-hlg"),
    ] {
        let path = tagged_gray(dir.path(), name, gray, transfer);
        let fields = format!(" bits=10 color={space}");
        let frame = compositor(&document(&path, &fields, ""), Accel::Never).frame(at(3), 1.0);
        let [r, g, b] = center(&frame);
        // Tone mapped for SDR: bright, and neutral.
        assert!(r > 215, "{space} reads {r}");
        assert!(
            r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3,
            "{space}: {r} {g} {b}"
        );
        // Read as SDR, the same file is a mid gray.
        let wrong = compositor(&document(&path, "", ""), Accel::Never).frame(at(3), 1.0);
        assert!(
            center(&wrong)[0].abs_diff(gray) <= 3,
            "{:?} against {gray}",
            center(&wrong)
        );
    }
}

#[test]
fn log_footage_opens_up_when_the_document_says_what_it_is() {
    let dir = tempfile::tempdir().expect("a folder");
    // S-Log3's middle gray is 41% in code values, on full-swing gray bars.
    let path = dir.path().join("log.mov");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=0x696969:s=320x180:r=24:d=1",
            "-pix_fmt",
            "yuv422p10le",
            "-c:v",
            "prores_ks",
        ],
        &path,
    );
    let path = path.display().to_string();
    let frame = compositor(
        &document(&path, " bits=10 color=slog3-sgamut3cine", ""),
        Accel::Never,
    )
    .frame(at(3), 1.0);
    let [r, g, b] = center(&frame);
    // Scene-linear 18% gray, shown: not the flat 41% the code value is.
    assert!((90..=135).contains(&r), "reads {r}");
    assert!(r.abs_diff(g) <= 4 && g.abs_diff(b) <= 4, "{r} {g} {b}");
}

#[test]
fn the_gpu_draws_deep_footage_as_the_cpu_does() {
    let dir = tempfile::tempdir().expect("a folder");
    let ten = ramp(dir.path(), "ten.mov", "yuv422p10le", "prores_ks");
    let hlg = tagged_gray(dir.path(), "hlg.mov", 191, "arib-std-b67");
    let log = ramp(dir.path(), "log.mov", "yuv422p10le", "prores_ks");
    for (path, fields, body) in [
        (&ten, " bits=10", LIFT),
        (
            &ten,
            " bits=10",
            "      set scale=0.8\n      fx color.lumetri exposure=0.5\n",
        ),
        (&hlg, " bits=10 color=rec2100-hlg", "      set scale=0.9\n"),
        (
            &log,
            " bits=10 color=slog3-sgamut3cine",
            "      set rotation=8 scale=0.8\n",
        ),
    ] {
        let document = document(path, fields, body);
        let gpu = compositor(&document, Accel::Always);
        if !gpu.draws_on_gpu(at(6), 1.0) {
            eprintln!("no GPU: nothing to compare");
            return;
        }
        let cpu = compositor(&document, Accel::Never);
        let (a, b) = (gpu.frame(at(6), 1.0), cpu.frame(at(6), 1.0));
        let (mut sum, mut off) = (0u64, 0u64);
        for (a, b) in a.rgba.as_chunks::<4>().0.iter().zip(b.rgba.as_chunks::<4>().0.iter()) {
            let worst = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
            sum += u64::from(worst);
            off += u64::from(worst > 2);
        }
        let pixels = u64::from(a.width) * u64::from(a.height);
        assert!(
            sum as f64 / pixels as f64 <= 0.6 && (off as f64) / (pixels as f64) <= 0.002,
            "{fields}{body}: mean {}, {} off",
            sum as f64 / pixels as f64,
            off
        );
    }
}

#[test]
fn a_document_round_trips_what_it_says_about_footage() {
    let text = "kerf 1\nproject name=\"t\"\nmedia m path=\"a.mov\" rate=24 dur=24f video=320x180 bits=10 color=rec2100-pq\n";
    let parsed = parse_document(text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let video = parsed.media[0].video.expect("a picture");
    assert_eq!(video.bits, 10);
    assert!(video.needs_converting());
    let printed = kerf_lang::print_document(
        &parsed,
        Catalog::builtin(),
        kerf_lang::PrintOptions::default(),
    );
    assert!(printed.contains(" bits=10 color=rec2100-pq"), "{printed}");
    for bad in ["bits=9", "color=rec1234"] {
        let text = text.replace("bits=10 color=rec2100-pq", bad);
        assert!(parse_document(&text, Catalog::builtin()).is_err(), "{bad}");
    }
    let without_picture =
        "kerf 1\nproject name=\"t\"\nmedia m path=\"a.mov\" rate=24 dur=24f bits=10\n";
    assert!(parse_document(without_picture, Catalog::builtin()).is_err());
}

#[test]
fn a_file_encoded_with_the_bt2020_matrix_decodes_with_it() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("matrix.mov");
    // Pure primaries through the BT.2020 Y'CbCr matrix: another matrix turns them other colors.
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=0xff0000:s=320x180:r=24:d=1",
            "-vf",
            "scale=out_color_matrix=bt2020nc:out_range=tv,format=yuv422p10le",
            "-c:v",
            "prores_ks",
            "-colorspace",
            "bt2020nc",
            "-color_range",
            "tv",
        ],
        &path,
    );
    let path = path.display().to_string();
    for (fields, body) in [
        (" bits=10", "      set scale=0.99\n"),
        ("", "      set scale=0.99\n"),
    ] {
        let frame = compositor(&document(&path, fields, body), Accel::Never).frame(at(3), 1.0);
        let [r, g, b] = center(&frame);
        assert!(r > 245 && g < 12 && b < 12, "[{fields}] {r} {g} {b}");
    }
}

#[test]
fn a_sixteen_bit_frame_holds_what_an_eight_bit_one_rounds_off() {
    let dir = tempfile::tempdir().expect("a folder");
    let ten = ramp(dir.path(), "ten.mov", "yuv422p10le", "prores_ks");
    let document = document(&ten, " bits=10", "      fx color.levels in_white=48\n");
    for accel in [Accel::Never, Accel::Always] {
        let compositor = compositor(&document, accel);
        let (eight, sixteen) = (compositor.frame(at(6), 1.0), compositor.frame16(at(6), 1.0));
        assert_eq!(sixteen.rgba.len(), eight.rgba.len() * 2);
        let row = (sixteen.height / 2 * sixteen.width) as usize;
        let codes: Vec<u16> = (row..row + sixteen.width as usize)
            .map(|pixel| u16::from_le_bytes([sixteen.rgba[pixel * 8], sixteen.rgba[pixel * 8 + 1]]))
            .collect();
        // It says the same thing, more finely.
        for (x, code) in codes.iter().enumerate() {
            let byte = eight.rgba[(row + x) * 4];
            assert!(u16::from(byte).abs_diff(code / 257) <= 1, "{byte} {code}");
        }
        // An 8-bit frame stretched to 16 bits would be multiples of 257.
        let finer = codes.iter().filter(|code| **code % 257 != 0).count();
        assert!(finer > codes.len() / 2, "{accel:?}: {finer}");
        // An untouched alpha, and the whole range.
        assert_eq!(&sixteen.rgba[6..8], &[255, 255]);
    }
}
