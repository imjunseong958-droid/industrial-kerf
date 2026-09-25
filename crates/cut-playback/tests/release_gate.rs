//! Release-gate coverage for the native playback surface.
//!
//! The fixtures are encoded by the installed FFmpeg at test time. This keeps
//! the gate useful on a clean checkout without committing large media files,
//! while still exercising real 4K, VFR, audio, and long-GOP bitstreams.

use cut_playback::{ClockSource, PlaybackConfig, PlaybackEvent, PlaybackSession, PlaybackTimeline};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn ffmpeg_binary() -> PathBuf {
    std::env::var_os("CUT_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            })
        })
}

fn ffprobe_binary() -> PathBuf {
    std::env::var_os("CUT_FFPROBE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(windows) {
                "ffprobe.exe"
            } else {
                "ffprobe"
            })
        })
}

fn available(binary: &Path, version_arg: &str) -> bool {
    Command::new(binary)
        .arg(version_arg)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run(binary: &Path, args: &[&str]) -> Output {
    let output = Command::new(binary)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "{} failed:\n{}",
        binary.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn media_project(
    path: &Path,
    duration: f64,
    width: u32,
    height: u32,
    fps: f64,
) -> serde_json::Value {
    json!({
        "version": 2,
        "name": "Native playback release gate",
        "fps": fps,
        "width": width,
        "height": height,
        "sources": [{
            "id": "source-a",
            "path": path,
            "duration": duration,
            "width": width,
            "height": height,
            "hasAudio": true
        }],
        "tracks": [
            {
                "id": "video-1",
                "type": "video",
                "clips": [{
                    "id": "clip-a",
                    "sourceId": "source-a",
                    "in": 0,
                    "out": duration,
                    "timelineStart": 0
                }]
            },
            {
                "id": "audio-1",
                "type": "audio",
                "muted": false,
                "clips": [{
                    "id": "clip-audio",
                    "sourceId": "source-a",
                    "in": 0,
                    "out": duration,
                    "timelineStart": 0
                }]
            }
        ],
        "overlays": [],
        "gaps": [],
        "transitions": [],
        "effects": [],
        "markers": []
    })
}

fn wait_for_frame(session: &PlaybackSession, timeout: Duration) -> cut_playback::PlaybackFrame {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(event) = session.next_event_timeout(Duration::from_millis(100)) {
            if let PlaybackEvent::Frame(frame) = event {
                return frame;
            }
            if let PlaybackEvent::Error(error) = event {
                panic!("playback error: {error}");
            }
        }
    }
    panic!("playback did not deliver a frame before the timeout");
}

fn skip_without_ffmpeg() -> bool {
    let ffmpeg = ffmpeg_binary();
    let ffprobe = ffprobe_binary();
    if !available(&ffmpeg, "-version") || !available(&ffprobe, "-version") {
        eprintln!("skipping native playback release gate: ffmpeg/ffprobe unavailable");
        true
    } else {
        false
    }
}

#[test]
fn release_gate_decodes_real_4k_media() {
    if skip_without_ffmpeg() {
        return;
    }
    let directory = TempDir::new().expect("temporary fixture directory");
    let source = directory.path().join("4k.mp4");
    let ffmpeg = ffmpeg_binary();
    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=3840x2160:rate=24",
            "-t",
            "0.25",
            "-frames:v",
            "3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-y",
            source.to_str().expect("UTF-8 path"),
        ],
    );
    let project = media_project(&source, 0.25, 3840, 2160, 24.0);
    let session = PlaybackSession::open_json(
        &project,
        PlaybackConfig::default()
            .with_ffmpeg_binary(&ffmpeg)
            .with_audio_enabled(false)
            .with_frame_size(3840, 2160)
            .with_decode_queue_capacity(1),
    )
    .expect("open 4K playback");
    let frame = wait_for_frame(&session, Duration::from_secs(10));
    assert_eq!((frame.width, frame.height), (3840, 2160));
    assert!(frame.has_valid_pixels());
    assert_eq!(frame.pixels.len(), 3840 * 2160 * 4);
}

#[test]
fn release_gate_accepts_vfr_and_long_gop_seek() {
    if skip_without_ffmpeg() {
        return;
    }
    let directory = TempDir::new().expect("temporary fixture directory");
    let vfr = directory.path().join("vfr.mp4");
    let long_gop = directory.path().join("long-gop.mp4");
    let ffmpeg = ffmpeg_binary();
    let ffprobe = ffprobe_binary();

    // A timestamp gap is retained with VFR output, so frame timestamps are
    // not silently converted to a constant-rate stream by the fixture.
    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-vf",
            "setpts=if(lt(N\\,15)\\,PTS\\,PTS+0.2/TB)",
            "-t",
            "1.2",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-y",
            vfr.to_str().expect("UTF-8 path"),
        ],
    );
    let timestamps = run(
        &ffprobe,
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=best_effort_timestamp_time",
            "-of",
            "csv=p=0",
            vfr.to_str().expect("UTF-8 path"),
        ],
    );
    let times: Vec<f64> = String::from_utf8_lossy(&timestamps.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    let deltas: Vec<f64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        times.len() > 20,
        "VFR fixture has too few frames: {times:?}"
    );
    assert!(
        deltas
            .iter()
            .any(|delta| (*delta - 1.0 / 30.0).abs() > 0.05),
        "VFR timestamps unexpectedly became constant-rate: {deltas:?}"
    );

    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-g",
            "60",
            "-keyint_min",
            "60",
            "-sc_threshold",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-y",
            long_gop.to_str().expect("UTF-8 path"),
        ],
    );
    let project = media_project(&long_gop, 2.0, 320, 180, 30.0);
    let session = PlaybackSession::open_json(
        &project,
        PlaybackConfig::default()
            .with_ffmpeg_binary(&ffmpeg)
            .with_audio_enabled(false)
            .with_frame_size(320, 180)
            .with_decode_queue_capacity(1),
    )
    .expect("open long-GOP playback");
    session.seek(1.25).expect("seek into long GOP");
    let frame = wait_for_frame(&session, Duration::from_secs(10));
    assert!(
        frame.timeline_time >= 1.2,
        "seek landed before target: {frame:?}"
    );
    assert!(
        frame.source_time >= 1.2,
        "source seek landed before target: {frame:?}"
    );
    assert!(frame.pixels.iter().any(|pixel| *pixel != 0));
}

#[test]
fn release_gate_keeps_audio_clock_and_video_frames_on_one_timeline() {
    if skip_without_ffmpeg() {
        return;
    }
    let directory = TempDir::new().expect("temporary fixture directory");
    let source = directory.path().join("audio-sync.mp4");
    let ffmpeg = ffmpeg_binary();
    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1.2",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-shortest",
            "-pix_fmt",
            "yuv420p",
            "-y",
            source.to_str().expect("UTF-8 path"),
        ],
    );
    let project = media_project(&source, 1.2, 320, 180, 30.0);
    let timeline = PlaybackTimeline::from_project(
        &cut_domain::parse_project(&project).expect("valid sync project"),
    )
    .expect("build sync timeline");
    assert!(
        !timeline.audio_at(0.25).is_empty(),
        "audio clip was not mapped"
    );

    let session = PlaybackSession::open_json(
        &project,
        PlaybackConfig::default()
            .with_ffmpeg_binary(&ffmpeg)
            .with_frame_size(320, 180)
            .with_decode_queue_capacity(2),
    )
    .expect("open audio playback");
    session.play().expect("play audio/video");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut clocks = Vec::new();
    let mut frames = Vec::new();
    while Instant::now() < deadline
        && clocks
            .last()
            .is_none_or(|clock: &cut_playback::PlaybackClock| clock.position < 0.35)
    {
        if let Some(event) = session.next_event_timeout(Duration::from_millis(100)) {
            match event {
                PlaybackEvent::Clock(clock) => clocks.push(clock),
                PlaybackEvent::Frame(frame) => frames.push(frame.timeline_time),
                PlaybackEvent::Error(error) => panic!("audio/video playback error: {error}"),
                _ => {}
            }
        }
    }
    assert!(clocks.len() >= 2, "clock did not advance: {clocks:?}");
    assert!(frames
        .iter()
        .all(|time| *time <= clocks.last().unwrap().position + 0.1));
    assert!(matches!(
        clocks.last().unwrap().source,
        ClockSource::AudioDevice | ClockSource::MonotonicFallback
    ));

    let muted_project = {
        let mut value = project.clone();
        value["tracks"][0]["muted"] = json!(true);
        value["tracks"][1]["muted"] = json!(true);
        value
    };
    let muted = PlaybackTimeline::from_project(
        &cut_domain::parse_project(&muted_project).expect("valid muted project"),
    )
    .expect("build muted timeline");
    assert!(
        muted.audio_at(0.25).is_empty(),
        "muted audio track remained audible"
    );
}
