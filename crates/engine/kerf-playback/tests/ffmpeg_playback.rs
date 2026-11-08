use kerf_playback::{PlaybackConfig, PlaybackEvent, PlaybackSession};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn ffmpeg_binary() -> String {
    std::env::var("KERF_FFMPEG").unwrap_or_else(|_| {
        if cfg!(windows) {
            "ffmpeg.exe".into()
        } else {
            "ffmpeg".into()
        }
    })
}

fn ffmpeg_available(binary: &str) -> bool {
    Command::new(binary)
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// One clip of `frames` frames at 10 fps.
fn project(source: &std::path::Path, name: &str, frames: u32) -> kerf_model::Project {
    let text = format!(
        "kerf 1\nproject name=\"{name}\"\n\
         media source-a path=\"{}\" rate=10 dur={frames}f video=64x36\n\
         sequence s1 rate=10 size=64x36 audio=48000\n\
         \x20 track video-1 video\n\
         \x20   clip clip-a media=source-a src=0f at=0f dur={frames}f\n",
        source.display()
    );
    kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

#[test]
fn real_ffmpeg_surface_delivers_rgba_frames_off_the_calling_thread() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("source.mp4");
    let status = Command::new(&binary)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=64x36:r=10",
            "-t",
            "0.4",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            source.to_str().expect("utf8 source path"),
        ])
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");

    let project = project(&source, "Playback integration", 4);
    let session = PlaybackSession::open(
        project,
        PlaybackConfig::default()
            .with_audio_enabled(false)
            .with_frame_size(64, 36),
    )
    .expect("open playback session");
    session.play().expect("play");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut frame = None;
    while Instant::now() < deadline {
        if let Some(PlaybackEvent::Frame(value)) =
            session.next_event_timeout(Duration::from_millis(100))
        {
            frame = Some(value);
            break;
        }
    }
    let frame = frame.expect("a decoded frame");
    assert_eq!((frame.width, frame.height), (64, 36));
    assert_eq!(frame.pixels.len(), 64 * 36 * 4);
    assert!(frame.has_valid_pixels());
    assert!(frame.pixels[0] > 240);
    assert!(frame.pixels[1] < 10);
    assert!(frame.pixels[2] < 10);
    assert_eq!(frame.pixels[3], 255);
}

#[test]
fn small_forward_seek_still_delivers_a_later_frame() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("source.mp4");
    let status = Command::new(&binary)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x36:rate=10",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            source.to_str().expect("utf8 source path"),
        ])
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");

    let project = project(&source, "Small seek", 10);
    let session = PlaybackSession::open(
        project,
        PlaybackConfig::default()
            .with_audio_enabled(false)
            .with_frame_size(64, 36)
            .with_decode_queue_capacity(1),
    )
    .expect("open playback session");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut first = None;
    while Instant::now() < deadline {
        if let Some(PlaybackEvent::Frame(value)) =
            session.next_event_timeout(Duration::from_millis(100))
        {
            first = Some(value);
            break;
        }
    }
    let first = first.expect("opening frame");
    session.seek(first.timeline_time + 0.2).expect("small seek");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut second = None;
    while Instant::now() < deadline {
        if let Some(PlaybackEvent::Frame(value)) =
            session.next_event_timeout(Duration::from_millis(100))
        {
            second = Some(value);
            break;
        }
    }
    let second = second.expect("seek frame");
    assert!(
        second.timeline_time + 1e-3 >= first.timeline_time + 0.15,
        "small seek stayed at {} after {}",
        second.timeline_time,
        first.timeline_time
    );
    assert!(second.has_valid_pixels());
}

/// Leaving a skim sends the playhead's seek, and the editor sends it again
/// when the pointer settles: however they land, its frame must come back.
#[test]
fn seeking_back_after_a_skim_shows_the_playheads_frame() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("source.mp4");
    let status = Command::new(&binary)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x36:rate=10",
            "-t",
            "30",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            source.to_str().expect("utf8 source path"),
        ])
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");

    let session = PlaybackSession::open(
        project(&source, "Skim back", 300),
        PlaybackConfig::default()
            .with_audio_enabled(false)
            .with_frame_size(64, 36),
    )
    .expect("open playback session");
    let frame_near = |target: f64, what: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(PlaybackEvent::Frame(frame)) =
                session.next_event_timeout(Duration::from_millis(100))
            {
                if (frame.timeline_time - target).abs() < 0.15 {
                    return;
                }
            }
        }
        panic!("no frame near {target} after {what}");
    };

    session.seek(2.0).expect("seek to the playhead");
    frame_near(2.0, "the first seek");
    // The pointer gone before its skim drew anything.
    session.scrub(18.0).expect("skim");
    session.seek(2.0).expect("seek back");
    frame_near(2.0, "leaving the skim at once");
    // The pointer settled, then gone: the seek back, and the same seek again.
    for apart in [0, 2, 4, 8, 16, 32] {
        session.scrub(25.0).expect("skim");
        std::thread::sleep(Duration::from_millis(50));
        session.seek(25.0).expect("settle the skim");
        frame_near(25.0, "settling the skim");
        session.seek(2.0).expect("seek back");
        std::thread::sleep(Duration::from_millis(apart));
        session.seek(2.0).expect("the same seek again");
        frame_near(2.0, &format!("two seeks back {apart} ms apart"));
        while session
            .next_event_timeout(Duration::from_millis(100))
            .is_some()
        {}
    }
}

/// The end of a sequence has no frame of its own: the playhead stays there and
/// the picture is the last frame, not whatever was showing before.
#[test]
fn seeking_to_the_end_shows_the_last_frame() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("source.mp4");
    let status = Command::new(&binary)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x36:rate=10",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            source.to_str().expect("utf8 source path"),
        ])
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");

    let session = PlaybackSession::open(
        project(&source, "The end", 20),
        PlaybackConfig::default()
            .with_audio_enabled(false)
            .with_frame_size(64, 36),
    )
    .expect("open playback session");
    // The frame near `target`, and the last clock position before it.
    let frame_near = |target: f64, what: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut clock = None;
        while Instant::now() < deadline {
            match session.next_event_timeout(Duration::from_millis(100)) {
                Some(PlaybackEvent::Frame(frame))
                    if (frame.timeline_time - target).abs() < 0.05 =>
                {
                    return clock;
                }
                Some(PlaybackEvent::Clock(now)) => clock = Some(now.position),
                _ => {}
            }
        }
        panic!("no frame near {target} after {what}");
    };

    session.seek(1.0).expect("seek inside");
    frame_near(1.0, "the seek inside");
    session.seek(2.0).expect("seek to the end");
    let clock = frame_near(1.9, "the seek to the end");
    assert_eq!(clock, Some(2.0), "the playhead stays at the end");
    session.seek(0.5).expect("seek back");
    frame_near(0.5, "the seek back from the end");
    // A skim to the end settles on the same frame.
    session.scrub(2.0).expect("skim to the end");
    session.seek(2.0).expect("settle the skim");
    frame_near(1.9, "the skim to the end");
}

#[test]
fn raster_effects_play_live_inside_the_letterbox() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("source.mp4");
    let status = Command::new(&binary)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("color=c=red:s=64x36:r=10")
        .args(["-t", "0.4", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&source)
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");
    // A wide picture in a square sequence, lightened to white.
    let text = format!(
        "kerf 1\nproject name=\"Live effects\"\n\
         media source-a path=\"{}\" rate=10 dur=4f video=64x36\n\
         sequence s1 rate=10 size=64x64 audio=48000\n\
         \x20 track video-1 video\n\
         \x20   clip clip-a media=source-a src=0f at=0f dur=4f\n\
         \x20     fx color.huesaturation lightness=100\n",
        source.display()
    );
    let project = kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    assert_eq!(
        kerf_playback::preview_plan(&project).mode,
        kerf_core::preview::PreviewMode::NativeExact
    );
    let session = PlaybackSession::open(
        project,
        PlaybackConfig::default()
            .with_audio_enabled(false)
            .with_frame_size(64, 64),
    )
    .expect("open playback session");
    session.play().expect("play");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut frame = None;
    while Instant::now() < deadline {
        if let Some(PlaybackEvent::Frame(value)) =
            session.next_event_timeout(Duration::from_millis(100))
        {
            frame = Some(value);
            break;
        }
    }
    let frame = frame.expect("a decoded frame");
    assert_eq!((frame.width, frame.height), (64, 64));
    let at = |x: usize, y: usize| {
        let i = (y * 64 + x) * 4;
        [
            frame.pixels[i],
            frame.pixels[i + 1],
            frame.pixels[i + 2],
            frame.pixels[i + 3],
        ]
    };
    // Red at full lightness is white; the bars stay black, untouched by
    // the effect.
    let center = at(32, 32);
    assert!(
        center[0] > 240 && center[1] > 240 && center[2] > 240,
        "{center:?}"
    );
    assert_eq!(at(32, 2), [0, 0, 0, 255]);
}

/// A source whose frames grow brighter, 64x36 at 10 fps: frame `n` is
/// gray `16 + 10n`.
fn counting_source(binary: &str, directory: &std::path::Path) -> std::path::PathBuf {
    let source = directory.join("counting.mp4");
    let status = Command::new(binary)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=64x36:r=10:d=2,format=gray,geq=lum='16+N*10'")
        .args(["-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv420p", "-y"])
        .arg(&source)
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");
    source
}

fn next_frame(
    session: &PlaybackSession,
    wanted: impl Fn(f64) -> bool,
) -> kerf_playback::PlaybackFrame {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(PlaybackEvent::Frame(frame)) =
            session.next_event_timeout(Duration::from_millis(100))
        {
            if wanted(frame.timeline_time) {
                return frame;
            }
        }
    }
    panic!("no frame arrived");
}

fn gray(frame: &kerf_playback::PlaybackFrame, x: usize, y: usize) -> u8 {
    let at = (y * frame.width as usize + x) * 4;
    frame.pixels[at + 1]
}

#[test]
fn composited_programs_play_and_seek_frame_exactly() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = counting_source(&binary, directory.path());
    let text = format!(
        "kerf 1\nproject name=\"Composited\"\n\
         media source-a path=\"{}\" rate=10 dur=20f video=64x36\n\
         sequence s1 rate=10 size=64x36 audio=48000\n\
         \x20 track video-1 video\n\
         \x20   clip clip-a media=source-a src=0f at=0f dur=20f\n\
         \x20     set scale=0.5\n",
        source.display()
    );
    let project = kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    assert_eq!(
        kerf_playback::preview_plan(&project).mode,
        kerf_core::preview::PreviewMode::Composited
    );
    let session = PlaybackSession::open(
        project,
        PlaybackConfig::default()
            .with_audio_enabled(false)
            .with_frame_size(64, 36)
            .with_composited(true),
    )
    .expect("open playback session");
    let expected = |time: f64| 16.0 + (time * 10.0).round() * 10.0;

    // A seek shows the frame there, half the size of the canvas.
    session.seek(1.2).expect("seek");
    let frame = next_frame(&session, |time| (time - 1.2).abs() < 1e-6);
    assert_eq!((frame.width, frame.height), (64, 36));
    let center = f64::from(gray(&frame, 32, 18));
    assert!((center - expected(1.2)).abs() < 4.0, "{center}");
    assert_eq!(gray(&frame, 2, 2), 0);

    // Playing shows later frames in order, each the frame of its time.
    session.seek(0.0).expect("seek");
    session.play().expect("play");
    let mut times = Vec::new();
    while times.len() < 4 {
        let frame = next_frame(&session, |_| true);
        let center = f64::from(gray(&frame, 32, 18));
        assert!(
            (center - expected(frame.timeline_time)).abs() < 4.0,
            "{} at {}",
            center,
            frame.timeline_time
        );
        times.push(frame.timeline_time);
    }
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "{times:?}");
}

#[test]
fn masked_clips_play_live_as_the_compositor_draws_them() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("pattern.mp4");
    let status = Command::new(&binary)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=96x54:rate=10:d=1")
        .args(["-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv444p", "-y"])
        .arg(&source)
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");
    let project = |masks: &str, effects: &str| {
        let text = format!(
            "kerf 1\nproject name=\"Live masks\"\n\
             media source-a path=\"{}\" rate=10 dur=10f video=96x54\n\
             sequence s1 rate=10 size=96x54 audio=48000\n\
             \x20 track video-1 video\n\
             \x20   clip clip-a media=source-a src=0f at=0f dur=10f\n{masks}{effects}",
            source.display()
        );
        kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project
    };
    let at = kerf_model::Time::from_seconds(0.5).expect("time");
    let drawn = |project: &kerf_model::Project, scale: f64| {
        kerf_compose::Compositor::new(
            project,
            kerf_model::Catalog::builtin(),
            &kerf_compose::ComposeOptions::default(),
        )
        .expect("compositor")
        .frame(at, scale)
    };
    // The largest and the mean difference between two frames' colors.
    let difference = |left: &[u8], right: &[u8]| {
        let (mut most, mut total) = (0u8, 0u64);
        for (left, right) in left.as_chunks::<4>().0.iter().zip(right.as_chunks::<4>().0.iter()) {
            for channel in 0..3 {
                let difference = left[channel].abs_diff(right[channel]);
                most = most.max(difference);
                total += u64::from(difference);
            }
        }
        (most, total as f64 / (left.len() / 4 * 3) as f64)
    };
    // Every shape, mode, and setting, an animated feather, and an effect
    // after the masks.
    for (masks, effects) in [
        (
            "\x20     mask k1 ellipse=0.45,0.5,0.35,0.42 feather={0f:2 10f:12} expansion=3\n\
             \x20     mask k2 points=\"0.1,0.1 0.9,0.2 0.6,0.9\" mode=subtract opacity=0.7\n\
             \x20     mask k3 rect=0.2,0.1,0.95,0.8 mode=intersect inverted=true feather=4\n",
            "",
        ),
        (
            "\x20     mask k1 rect=0.15,0.2,0.8,0.85 mode=subtract expansion=-2 feather=6\n\
             \x20     mask k2 ellipse=0.5,0.5,0.2,0.3 opacity=0.6\n\
             \x20     mask k3 points=\"0.5,0 1,1 0,1\" mode=none\n",
            "\x20     fx color.curves rgb=\"0,0 0.3,1 1,1\"\n",
        ),
    ] {
        let masked = project(masks, effects);
        let plan = kerf_playback::preview_plan(&masked);
        assert_eq!(
            plan.mode,
            kerf_core::preview::PreviewMode::NativeExact,
            "{:?}",
            plan.reasons
        );
        // The masks clear a good part of the picture.
        let whole = drawn(&project("", effects), 1.0);
        let cut = whole
            .rgba
            .as_chunks::<4>().0.iter()
            .zip(drawn(&masked, 1.0).rgba.as_chunks::<4>().0.iter())
            .filter(|(whole, masked)| (0..3).any(|c| whole[c].abs_diff(masked[c]) > 32))
            .count();
        assert!(cut > 96 * 54 / 5, "{effects:?}: {cut}");
        // Live playback clears the same part, edges and feathers included,
        // at full size and at the half a smaller viewer plays.
        for (width, height, scale) in [(96, 54, 1.0), (48, 27, 0.5)] {
            let session = PlaybackSession::open(
                masked.clone(),
                PlaybackConfig::default()
                    .with_audio_enabled(false)
                    .with_frame_size(width, height),
            )
            .expect("open playback session");
            session.seek(0.5).expect("seek");
            let live = next_frame(&session, |time| (time - 0.5).abs() < 1e-6);
            let drawn = drawn(&masked, scale);
            assert_eq!((live.width, live.height), (drawn.width, drawn.height));
            let (most, mean) = difference(&live.pixels, &drawn.rgba);
            assert!(
                most <= 4 && mean < 0.25,
                "{effects:?} at {width}x{height}: most {most}, mean {mean}"
            );
        }
    }
}
