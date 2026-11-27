use kerf_playback::{PlaybackConfig, PlaybackEvent, PlaybackSession, PlaybackState};
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

#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
                && (frame.timeline_time - target).abs() < 0.15
            {
                return;
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
            && wanted(frame.timeline_time)
        {
            return frame;
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
    let session = crate::Composited::open(
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

    // Played on, it ends where the program does, and says so.
    let deadline = Instant::now() + Duration::from_secs(20);
    let ended = std::iter::from_fn(|| {
        (Instant::now() < deadline).then(|| session.next_event_timeout(Duration::from_millis(100)))
    })
    .any(|event| matches!(event, Some(PlaybackEvent::Ended)));
    assert!(ended, "the program never ended");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
        for (left, right) in left
            .as_chunks::<4>()
            .0
            .iter()
            .zip(right.as_chunks::<4>().0.iter())
        {
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
            .as_chunks::<4>()
            .0
            .iter()
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

/// Footage of `frames` frames at `rate` a second whose grey says which frame it is:
/// frame `n` is `16 + 8 n` light.
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn numbered_source(
    binary: &str,
    directory: &std::path::Path,
    rate: u32,
    frames: u32,
) -> std::path::PathBuf {
    let source = directory.join(format!("numbered-{rate}.mp4"));
    let status = Command::new(binary)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "color=c=black:s=64x36:r={rate},format=gray,geq=lum='16+N*8',trim=end_frame={frames}"
        ))
        .args(["-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv420p", "-y"])
        .arg(&source)
        .status()
        .expect("run ffmpeg fixture");
    assert!(status.success(), "fixture ffmpeg process failed");
    source
}

/// The frame of [`numbered_source`] a picture is.
fn numbered(frame: &kerf_playback::PlaybackFrame) -> i64 {
    ((f64::from(gray(frame, 32, 18)) - 16.0) / 8.0).round() as i64
}

/// A program the native decoders play shows, at each of its frames, the frame of the media
/// the document says: the one whose display has started, as a render draws it, not the next
/// one. Footage at another rate, at another speed, and in a nested sequence at a rate of
/// its own all put the source position between two frames.
#[test]
fn native_playback_shows_the_frame_of_the_media_a_render_shows() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let film = numbered_source(&binary, directory.path(), 24, 28);
    let video = numbered_source(&binary, directory.path(), 30, 28);
    let head = |source: &std::path::Path, rate: u32| {
        format!(
            "kerf 1\nproject name=\"Frames\"\n\
             media m path=\"{}\" rate={rate} dur=28f video=64x36\n\
             sequence s1 rate=30 size=64x36 audio=48000\n\
             \x20 track V1 video\n",
            source.display()
        )
    };
    type Shows = fn(i64) -> i64;
    let cases: [(&str, String, Shows); 3] = [
        (
            "24 frames a second in a sequence of 30",
            format!(
                "{}    clip c1 media=m src=0f at=0f dur=30f\n",
                head(&film, 24)
            ),
            |frame| frame * 24 / 30,
        ),
        (
            "one and a half times the speed",
            format!(
                "{}    clip c1 media=m src=2f at=0f dur=16f speed=1.5\n",
                head(&video, 30)
            ),
            |frame| 2 + frame * 3 / 2,
        ),
        (
            "a nested sequence of 24 frames a second holding footage of 30",
            format!(
                "{}    clip n1 seq=s2 src=0f at=0f dur=27f\n\
                 sequence s2 rate=24 size=64x36 audio=48000\n\
                 \x20 track V1 video\n\
                 \x20   clip c1 media=m src=0f at=0f dur=22f\n",
                head(&video, 30)
            ),
            // The nested sequence's frame, then the footage's at that frame's start.
            |frame| (frame * 24 / 30) * 30 / 24,
        ),
    ];
    for (what, text, shows) in cases {
        let project = kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        assert_eq!(
            kerf_playback::preview_plan(&project).mode,
            kerf_core::preview::PreviewMode::NativeExact,
            "{what}"
        );
        let session = PlaybackSession::open(
            project,
            PlaybackConfig::default()
                .with_audio_enabled(false)
                .with_frame_size(64, 36),
        )
        .expect("open playback session");
        // Paused on a frame, far from the last one and just after it.
        for frame in [1, 7, 11, 13, 3] {
            let at = frame as f64 / 30.0;
            session.seek(at).expect("seek");
            let shown = next_frame(&session, |time| (time - at).abs() < 1e-6);
            assert_eq!(
                numbered(&shown),
                shows(frame),
                "{what}, paused on frame {frame}"
            );
        }
        // And playing through.
        session.seek(0.0).expect("seek");
        session.play().expect("play");
        let mut played = 0;
        while played < 12 {
            let shown = next_frame(&session, |_| true);
            let frame = (shown.timeline_time * 30.0).round() as i64;
            assert_eq!(
                numbered(&shown),
                shows(frame),
                "{what}, playing frame {frame}"
            );
            played += 1;
        }
    }
}

/// A paused seek shows the frame of the program at the playhead when it keeps the decoder it
/// has as well as when it opens one: asked again for the frame it has just shown, which that
/// decoder has read past; a little way on from part way through a frame; and onto a frame
/// decoded ahead while playing from part way through one.
#[test]
fn a_paused_seek_shows_the_frame_at_the_playhead() {
    let binary = ffmpeg_binary();
    if !ffmpeg_available(&binary) {
        eprintln!("skipping playback integration test: ffmpeg is unavailable");
        return;
    }
    let directory = tempdir().expect("temporary directory");
    let source = numbered_source(&binary, directory.path(), 10, 28);
    // The clip starts on the sequence's frame 3, after a gap, with the media's first frame.
    let text = format!(
        "kerf 1\nproject name=\"Seeks\"\n\
         media m path=\"{}\" rate=10 dur=28f video=64x36\n\
         sequence s1 rate=10 size=64x36 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m src=0f at=3f dur=28f\n",
        source.display()
    );
    let project = kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let session = PlaybackSession::open(
        project,
        PlaybackConfig::default()
            .with_audio_enabled(false)
            .with_frame_size(64, 36)
            .with_decode_queue_capacity(24),
    )
    .expect("open playback session");
    // Seek `part` of the way through the program's `frame`: the picture shown is of that
    // frame, the media's frame 3 before it.
    let seek = |frame: i64, part: f64, what: &str| {
        session.seek((frame as f64 + part) / 10.0).expect("seek");
        let shown = next_frame(&session, |_| true);
        let at = (shown.timeline_time * 10.0 + 1e-6).floor() as i64;
        assert_eq!((at, numbered(&shown)), (frame, frame - 3), "{what}");
    };

    seek(3, 0.0, "the clip's first frame");
    seek(3, 0.0, "the clip's first frame again");
    seek(10, 0.0, "a frame a little way on");
    seek(10, 0.0, "that frame again");
    seek(5, 0.6, "part way through a frame before");
    seek(8, 0.0, "a frame a little way on from part way through one");

    seek(5, 0.6, "part way through a frame before, to play from");
    session.play().expect("play");
    next_frame(&session, |_| true);
    session.pause().expect("pause");
    // Where the pause left the playhead: the clock it says after it says it paused.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut paused, mut position) = (false, None);
    while position.is_none() && Instant::now() < deadline {
        match session.next_event_timeout(Duration::from_millis(100)) {
            Some(PlaybackEvent::StateChanged(PlaybackState::Paused)) => paused = true,
            Some(PlaybackEvent::Clock(clock)) if paused => position = Some(clock.position),
            _ => {}
        }
    }
    let paused = position.expect("playback paused");
    // Three frames on from the playhead, past what playing has shown, so the frame before it
    // is still queued.
    let ahead = (paused * 10.0 + 1e-6).floor() as i64 + 3;
    seek(ahead, 0.0, "a frame decoded ahead while playing");
}
