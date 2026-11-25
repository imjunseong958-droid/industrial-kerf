#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use kerf_fetcher::{fetch_video, FetchError, FetchOptions};

/// Stands in for yt-dlp. It answers `--version`; asked to download, it
/// writes a partial file and starts a long FFmpeg of its own, as yt-dlp
/// does to merge: with pipes of its own, and deaf to SIGTERM, so that only
/// a kill ends it. It writes that FFmpeg's process id to `ffmpeg.pid` and
/// waits. Asked to end, it removes its partial file.
const STAND_IN_YT_DLP: &str = r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2026.11.01; exit 0; fi
trap 'rm -f video.mp4.part; exit 143' TERM
: >video.mp4.part
sh -c 'trap "" TERM; exec sleep 600' </dev/null >/dev/null 2>&1 &
echo $! >ffmpeg.pid
wait
"#;

/// Whether process `pid` ends within `limit`; a zombie awaiting its new
/// parent counts as ended. `ps` that says nothing, not even why, has found
/// no such process.
fn ends_within(pid: &str, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .expect("ps runs");
        let state = String::from_utf8_lossy(&output.stdout);
        let gone = state.trim().is_empty() && output.stderr.is_empty();
        if gone || state.trim_start().starts_with('Z') {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_timed_out_download_is_asked_to_end_and_takes_its_ffmpeg_with_it() {
    let bin = tempfile::tempdir().unwrap();
    let yt_dlp = bin.path().join("yt-dlp");
    fs::write(&yt_dlp, STAND_IN_YT_DLP).unwrap();
    fs::set_permissions(&yt_dlp, fs::Permissions::from_mode(0o755)).unwrap();
    let output = tempfile::tempdir().unwrap();

    let result = fetch_video(
        "https://example.com/watch?v=stand-in",
        output.path(),
        &FetchOptions {
            timeout: Some(Duration::from_secs(2)),
            yt_dlp_binary: Some(yt_dlp),
            ..FetchOptions::default()
        },
    );

    let ffmpeg = fs::read_to_string(output.path().join("ffmpeg.pid"))
        .expect("the stand-in started its FFmpeg before the timeout");
    let ffmpeg = ffmpeg.trim();
    let ended = ends_within(ffmpeg, Duration::from_secs(5));
    if !ended {
        // Not left running past this test.
        let _ = Command::new("kill").args(["-KILL", ffmpeg]).status();
    }
    assert!(ended, "the FFmpeg yt-dlp started outlived the timeout");
    assert!(
        !output.path().join("video.mp4.part").exists(),
        "yt-dlp was not asked to end, so it left its partial file"
    );
    match result {
        Err(FetchError::Process { message, .. }) => assert_eq!(message, "yt-dlp timed out"),
        other => panic!("expected the download to time out, got {other:?}"),
    }
}
