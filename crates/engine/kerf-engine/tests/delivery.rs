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
