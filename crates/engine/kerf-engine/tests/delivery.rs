//! Delivery measurements on generated media: programme R128 loudness, the
//! picture QC pass, and the ProRes 422 HQ master probed back.

use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::loudness::measure_program;
use kerf_engine::qc::picture_qc;
use kerf_engine::render::{encoder_available, render_program, RenderOptions, RenderPreset};
use kerf_engine::{media_info, CancellationToken};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;

fn ffmpeg_present() -> bool {
    let present = Command::new(resolve_ffmpeg_bin())
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !present {
        eprintln!("skipping: ffmpeg is not installed");
    }
    present
}

fn generate(args: &[&str], output: &Path) {
    let result = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(output)
        .output()
        .expect("ffmpeg runs");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn project(text: &str) -> Project {
    parse_document(text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

/// Six seconds of a 1 kHz tone on both channels at the amplitude that
/// BS.1770 reads as −23 LUFS (a full-scale stereo sine reads 0 LUFS).
fn tone(dir: &Path) -> String {
    let path = dir.join("tone.wav");
    let amplitude = 10_f64.powf(-23.0 / 20.0);
    let channel = format!("{amplitude}*sin(2*PI*1000*t)");
    generate(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("aevalsrc={channel}|{channel}:s=48000:d=6"),
            "-c:a",
            "pcm_s16le",
        ],
        &path,
    );
    path.to_string_lossy().into_owned()
}

#[test]
fn a_minus_23_lufs_tone_measures_minus_23() {
    if !ffmpeg_present() {
        return;
    }
    let dir = tempfile::tempdir().expect("temp directory");
    let tone = tone(dir.path());
    let project = project(&format!(
        "kerf 1\nproject name=\"loudness\"\n\
         media t path=\"{tone}\" rate=24 dur=6s audio=2ch@48000\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track A1 audio\n\
         \x20   clip a media=t src=0f at=0f dur=6s\n"
    ));
    let mut progress = Vec::new();
    let loudness = kerf_engine::loudness::measure_program_with_progress(
        &project,
        None,
        dir.path(),
        &CancellationToken::new(),
        |share| progress.push(share),
    )
    .expect("measures");
    assert!(
        (loudness.integrated - -23.0).abs() <= 0.5,
        "integrated {}",
        loudness.integrated
    );
    assert!(
        loudness.lra < 1.0,
        "a steady tone has no range: {}",
        loudness.lra
    );
    for peak in loudness.true_peak {
        assert!((peak - -23.0).abs() <= 0.5, "true peak {peak}");
    }
    assert!((loudness.momentary_max - -23.0).abs() <= 0.5);
    assert!((loudness.short_term_max - -23.0).abs() <= 0.5);
    // A sample every 100 ms, ending with the programme.
    assert!(
        (59..=61).contains(&loudness.series.len()),
        "{} samples",
        loudness.series.len()
    );
    assert!((loudness.series[0].0 - 0.1).abs() < 1e-6);
    assert!((loudness.series.last().expect("samples").0 - 6.0).abs() < 1e-6);
    assert!(loudness.short_term_max_at().is_some());
    assert_eq!(progress.last(), Some(&1.0));
    assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        measure_program(&project, None, dir.path(), &cancelled),
        Err(kerf_engine::EngineError::Cancelled)
    ));
}

#[test]
fn strobing_is_flashing_and_a_black_segment_is_found() {
    if !ffmpeg_present() {
        return;
    }
    let dir = tempfile::tempdir().expect("temp directory");
    // Two seconds strobing at six flashes a second, one second of black,
    // then a second of mid grey.
    let path = dir.path().join("strobe.mp4");
    generate(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=160x90:r=24:d=4,format=yuv420p,\
             geq=lum='if(lt(T,2),if(lt(mod(N,4),2),235,16),if(lt(T,3),16,128))':cb=128:cr=128",
            "-c:v",
            "libx264",
            "-crf",
            "1",
            "-pix_fmt",
            "yuv420p",
        ],
        &path,
    );
    let project = project(&format!(
        "kerf 1\nproject name=\"qc\"\n\
         media s path=\"{}\" rate=24 dur=96f video=160x90\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip v media=s src=0f at=0f dur=96f\n",
        path.display()
    ));
    let qc = picture_qc(&project, None, dir.path(), &CancellationToken::new()).expect("measures");
    assert_eq!(qc.frames, 96);
    assert!(!qc.flashing.passed, "{:#?}", qc.flashing);
    assert_eq!(qc.flashing.ranges.len(), 1, "{:?}", qc.flashing.ranges);
    let flash = qc.flashing.ranges[0];
    assert!(
        flash.start < 0.2 && (1.9..=2.2).contains(&flash.end),
        "{flash:?}"
    );
    assert!(qc.flash_transitions_max > 3);
    assert!(!qc.black.passed, "{:#?}", qc.black);
    assert_eq!(qc.black.ranges.len(), 1, "{:?}", qc.black.ranges);
    let black = qc.black.ranges[0];
    assert!((1.8..=2.05).contains(&black.start), "{black:?}");
    assert!((2.95..=3.05).contains(&black.end), "{black:?}");
    // The drawn picture is legal: the master conversion keeps Y′ in range.
    assert!(qc.luma_levels.passed, "{:#?}", qc.luma_levels);
    assert!(qc.out_of_range.passed, "{:#?}", qc.out_of_range);
    let (low, high) = (qc.luma_min.expect("min"), qc.luma_max.expect("max"));
    assert!((64.0..80.0).contains(&low), "black is near 64: {low}");
    assert!(high > 900.0 && high <= 940.0, "white is near 940: {high}");
}

#[test]
fn a_prores_master_probes_back_as_prores_hq_with_24_bit_pcm() {
    if !ffmpeg_present() {
        return;
    }
    if !encoder_available("prores_ks") && !encoder_available("prores_videotoolbox") {
        eprintln!("skipping: this ffmpeg has no ProRes encoder");
        return;
    }
    let dir = tempfile::tempdir().expect("temp directory");
    let source = dir.path().join("source.mp4");
    generate(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:sample_rate=48000:d=2",
            "-ac",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
        &source,
    );
    let project = project(&format!(
        "kerf 1\nproject name=\"master\"\n\
         media s path=\"{}\" rate=25 dur=50f video=320x180 audio=2ch@48000\n\
         sequence s1 rate=25 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip v media=s src=0f at=0f dur=50f\n\
         \x20 track A1 audio\n\
         \x20   clip a media=s src=0f at=0f dur=50f\n",
        source.display()
    ));
    let master = dir
        .path()
        .join(format!("master.{}", RenderPreset::ProRes422Hq.extension()));
    render_program::<fn(f64)>(
        &project,
        None,
        None,
        &RenderOptions {
            output: master.to_string_lossy().into_owned(),
            preset: RenderPreset::ProRes422Hq,
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders the master");
    let info = media_info(&master.to_string_lossy()).expect("probes");
    assert_eq!(info.codec.as_deref(), Some("prores"));
    assert_eq!(info.profile.as_deref(), Some("HQ"), "{info:?}");
    assert_eq!(info.bit_depth, Some(10), "{info:?}");
    assert!(info
        .pix_fmt
        .as_deref()
        .is_some_and(|format| format.contains("422")));
    assert_eq!(info.color_space.as_deref(), Some("bt709"));
    assert_eq!(info.primaries.as_deref(), Some("bt709"));
    assert_eq!(info.transfer.as_deref(), Some("bt709"));
    assert_eq!(info.audio_codec.as_deref(), Some("pcm_s24le"));
    assert_eq!(info.audio_bit_depth, Some(24));
    assert_eq!(info.audio_channels, Some(2));
    assert_eq!(info.audio_sample_rate, Some(48_000));
    let tag = Command::new(kerf_engine::ffmpeg::resolve_ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0"])
        .args([
            "-show_entries",
            "stream=codec_tag_string:format=format_name",
        ])
        .args(["-of", "default=nw=1"])
        .arg(&master)
        .output()
        .expect("ffprobe runs");
    let text = String::from_utf8_lossy(&tag.stdout);
    assert!(
        text.contains("codec_tag_string=apch"),
        "ProRes 422 HQ: {text}"
    );
    assert!(text.contains("format_name=mov"), "QuickTime: {text}");

    // The graph draws the picture in 4:2:0; its master is the same codec,
    // tagged the same way.
    let graphed = dir.path().join("graph.mov");
    render_program::<fn(f64)>(
        &project,
        None,
        None,
        &RenderOptions {
            output: graphed.to_string_lossy().into_owned(),
            preset: RenderPreset::ProRes422Hq,
            graph: true,
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders the graph master");
    let info = media_info(&graphed.to_string_lossy()).expect("probes");
    assert_eq!(
        (
            info.codec.as_deref(),
            info.profile.as_deref(),
            info.audio_codec.as_deref()
        ),
        (Some("prores"), Some("HQ"), Some("pcm_s24le"))
    );
    assert_eq!(
        (
            info.color_space.as_deref(),
            info.primaries.as_deref(),
            info.transfer.as_deref()
        ),
        (Some("bt709"), Some("bt709"), Some("bt709"))
    );
}

#[test]
fn an_hdr_sequence_masters_as_hdr_and_comes_back_as_it() {
    if !ffmpeg_present() {
        return;
    }
    if !encoder_available("prores_ks") && !encoder_available("prores_videotoolbox") {
        eprintln!("skipping: this ffmpeg has no ProRes encoder");
        return;
    }
    let dir = tempfile::tempdir().expect("temp directory");
    for (id, transfer, space) in [
        ("rec2100-pq", "smpte2084", kerf_model::ColorSpace::Rec2100Pq),
        (
            "rec2100-hlg",
            "arib-std-b67",
            kerf_model::ColorSpace::Rec2100Hlg,
        ),
    ] {
        let project = project(&format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000 color={id}\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=12f\n      set color=#ffffff\n"
        ));
        let master = dir.path().join(format!("{id}.mov"));
        render_program::<fn(f64)>(
            &project,
            None,
            None,
            &RenderOptions {
                output: master.to_string_lossy().into_owned(),
                preset: RenderPreset::ProRes422Hq,
                ..RenderOptions::default()
            },
            None,
        )
        .expect("renders the master");
        let path = master.to_string_lossy();
        let info = media_info(&path).expect("probes");
        assert_eq!(info.color_space.as_deref(), Some("bt2020nc"), "{info:?}");
        assert_eq!(info.primaries.as_deref(), Some("bt2020"), "{info:?}");
        assert_eq!(info.transfer.as_deref(), Some(transfer), "{info:?}");
        assert_eq!(info.bit_depth, Some(10), "{info:?}");
        let found = kerf_engine::info(&path)
            .expect("probes")
            .video_info(320, 180);
        assert_eq!((found.color, found.bits), (space, 10), "{id}");

        // SDR copies of it are ordinary Rec.709.
        let copy = dir.path().join(format!("{id}.mp4"));
        render_program::<fn(f64)>(
            &project,
            None,
            None,
            &RenderOptions {
                output: copy.to_string_lossy().into_owned(),
                preset: RenderPreset::Review,
                force_software: true,
                ..RenderOptions::default()
            },
            None,
        )
        .expect("renders a review copy");
        let found = kerf_engine::info(&copy.to_string_lossy())
            .expect("probes")
            .video_info(320, 180);
        assert_eq!(found.color, kerf_model::ColorSpace::Rec709);
    }
}

/// What ffprobe finds of a video's first frame and stream, as `key=value` lines.
fn probed(path: &Path) -> String {
    let output = Command::new(kerf_engine::ffmpeg::resolve_ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-read_intervals", "%+#1", "-show_frames"])
        .args([
            "-show_entries",
            "stream=codec_name,profile,pix_fmt,color_space,color_primaries,color_transfer,color_range,codec_tag_string:frame=side_data_list",
        ])
        .args(["-of", "default=nw=1"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `name=number/denominator` or `name=number` as a number.
fn number_after(text: &str, name: &str) -> f64 {
    let line = text
        .lines()
        .find(|line| line.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} in {text}"));
    let value = line.split_once('=').expect("a value").1;
    match value.split_once('/') {
        Some((top, bottom)) => {
            top.parse::<f64>().expect("a number") / bottom.parse::<f64>().expect("a number")
        }
        None => value.parse().expect("a number"),
    }
}

/// A sequence finished in `color` holding one flat clip of 10-bit `transfer` footage and a white
/// solid under a mask over part of it.
fn hdr_project(dir: &Path, color: &str, transfer: &str, gray: u8) -> kerf_model::Project {
    let source = dir.join(format!("{transfer}.mov"));
    let made = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=0x{gray:02x}{gray:02x}{gray:02x}:s=320x180:r=24:d=1"))
        .args([
            "-vf",
            &format!("scale=in_range=pc:out_range=tv,format=yuv422p10le,setparams=range=tv:colorspace=bt2020nc:color_primaries=bt2020:color_trc={transfer}"),
            "-c:v",
            "prores_ks",
        ])
        .arg(&source)
        .output()
        .expect("ffmpeg runs");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    kerf_lang::parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180 bits=10 color={color}\nsequence s1 rate=24 size=320x180 audio=48000 color={color}\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n",
            source.display()
        ),
        kerf_model::Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project
}

#[test]
fn hdr10_delivery_says_what_it_was_mastered_on_and_how_bright_it_is() {
    if !encoder_available("libx265") {
        eprintln!("skipping: this FFmpeg has no libx265");
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    // A signal of 0xD0 / 255 in PQ is about 2100 nits.
    let project = hdr_project(dir.path(), "rec2100-pq", "smpte2084", 0xD0);
    let out = dir.path().join("hdr10.mp4");
    render_program::<fn(f64)>(
        &project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            preset: RenderPreset::HevcHdr,
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
    let text = probed(&out);
    for wanted in [
        "codec_name=hevc",
        "profile=Main 10",
        "pix_fmt=yuv420p10le",
        "color_space=bt2020nc",
        "color_transfer=smpte2084",
        "color_primaries=bt2020",
        "codec_tag_string=hvc1",
        "side_data_type=Mastering display metadata",
        "side_data_type=Content light level metadata",
    ] {
        assert!(text.contains(wanted), "no {wanted} in {text}");
    }
    // The display: P3 D65 at 1000 nits.
    assert!((number_after(&text, "red_x") - 0.68).abs() < 1e-3, "{text}");
    assert!(
        (number_after(&text, "green_y") - 0.69).abs() < 1e-3,
        "{text}"
    );
    assert!(
        (number_after(&text, "white_point_x") - 0.3127).abs() < 1e-3,
        "{text}"
    );
    assert!(
        (number_after(&text, "max_luminance") - 1000.0).abs() < 1e-3,
        "{text}"
    );
    // The picture holds the same signal, within what 4:2:0 and a compression allow.
    let signal = |path: &Path| -> f64 {
        let raw = Command::new(resolve_ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "yuv420p10le",
                "-",
            ])
            .output()
            .expect("ffmpeg runs")
            .stdout;
        let code = u16::from_le_bytes([raw[2 * (90 * 320 + 160)], raw[2 * (90 * 320 + 160) + 1]]);
        f64::from(code)
    };
    // A flat gray picture's brightest pixel is its average, and its code says how bright it is,
    // whatever the meter thought.
    let code = signal(&out);
    let expected = kerf_color::pq_eotf(((code - 64.0) / 876.0) as f32) * 10_000.0;
    let (cll, fall) = (
        number_after(&text, "max_content"),
        number_after(&text, "max_average"),
    );
    eprintln!("flat PQ footage of {expected:.0} nits measured: MaxCLL {cll}, MaxFALL {fall}");
    assert!(
        (cll - f64::from(expected)).abs() < f64::from(expected) * 0.03,
        "{cll} of {expected}"
    );
    assert!((cll - fall).abs() < 5.0, "{cll} {fall}");
    let master = dir.path().join("master.mov");
    render_program::<fn(f64)>(
        &project,
        None,
        None,
        &RenderOptions {
            output: master.to_string_lossy().into_owned(),
            preset: RenderPreset::ProRes422Hq,
            force_software: true,
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders the master");
    assert!(
        (signal(&out) - signal(&master)).abs() < 8.0,
        "{} {}",
        signal(&out),
        signal(&master)
    );
    // Told the levels, it writes those without measuring.
    let told = dir.path().join("told.mp4");
    render_program::<fn(f64)>(
        &project,
        None,
        None,
        &RenderOptions {
            output: told.to_string_lossy().into_owned(),
            preset: RenderPreset::HevcHdr,
            mastering: kerf_engine::render::MasteringDisplay {
                peak: 4000.0,
                ..Default::default()
            },
            content_light: Some(kerf_engine::render::ContentLight {
                max_cll: 3210,
                max_fall: 321,
            }),
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
    let text = probed(&told);
    assert_eq!(number_after(&text, "max_content"), 3210.0, "{text}");
    assert_eq!(number_after(&text, "max_average"), 321.0, "{text}");
    assert!(
        (number_after(&text, "max_luminance") - 4000.0).abs() < 1e-3,
        "{text}"
    );
}

#[test]
fn hlg_delivery_is_tagged_and_carries_no_hdr10_metadata() {
    if !encoder_available("libx265") {
        eprintln!("skipping: this FFmpeg has no libx265");
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let project = hdr_project(dir.path(), "rec2100-hlg", "arib-std-b67", 0xC0);
    let out = dir.path().join("hlg.mp4");
    render_program::<fn(f64)>(
        &project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            preset: RenderPreset::HevcHdr,
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
    let text = probed(&out);
    assert!(text.contains("color_transfer=arib-std-b67"), "{text}");
    assert!(text.contains("profile=Main 10"), "{text}");
    assert!(!text.contains("Mastering display"), "{text}");
}

#[test]
fn hdr_delivery_refuses_an_sdr_sequence() {
    if !encoder_available("libx265") {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let project = kerf_lang::parse_document(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n  track V1 video\n    clip g gen=solid@1 at=0f dur=4f\n",
        kerf_model::Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let refused = render_program::<fn(f64)>(
        &project,
        None,
        None,
        &RenderOptions {
            output: dir.path().join("out.mp4").to_string_lossy().into_owned(),
            preset: RenderPreset::HevcHdr,
            ..RenderOptions::default()
        },
        None,
    )
    .expect_err("nothing to deliver in HDR");
    assert!(refused.to_string().contains("rec2100"), "{refused}");
}
