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
