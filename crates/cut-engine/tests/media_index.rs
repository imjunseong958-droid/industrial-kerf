//! The media index and proxies against real footage.

use cut_engine::derivatives::DerivativeCache;
use cut_engine::{cached_proxy, info, make_proxy, media_index, CancellationToken, PROXY_WIDTH};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Ten seconds at 1280x720: red, then blue from 5s; a click every half
/// second for six seconds, then silence.
fn footage(directory: &Path) -> Option<std::path::PathBuf> {
    let path = directory.join("clicks.mp4");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=1280x720:rate=25:duration=5",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=1280x720:rate=25:duration=5",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "aevalsrc='if(lt(t,6)*lt(mod(t,0.5),0.03),0.8*sin(2*PI*1000*t),0)':s=48000:d=10",
        ])
        .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0[v]"])
        .args([
            "-map",
            "[v]",
            "-map",
            "2:a",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .args(["-pix_fmt", "yuv420p", "-c:a", "aac"])
        .arg(&path)
        .status()
        .ok()?;
    status.success().then_some(path)
}

#[test]
fn one_pass_finds_cuts_pauses_onsets_and_tempo_and_a_proxy_is_made() {
    let directory = TempDir::new().unwrap();
    let Some(path) = footage(directory.path()) else {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    };
    let cache = DerivativeCache::new(directory.path());
    let index = media_index(&path, &cache, &CancellationToken::new()).unwrap();
    assert_eq!(index.cuts.len(), 1, "{index:?}");
    assert!((index.cuts[0] - 5.0).abs() < 0.1, "{index:?}");
    // The last click is at 5.5s; silence follows to the end.
    let (from, to) = *index.pauses.last().unwrap();
    assert!((from - 5.53).abs() < 0.05 && to == 10.0, "{index:?}");
    assert!((10..=13).contains(&index.onsets.len()), "{index:?}");
    let tempo = index.tempo.clone().expect("a steady click has a tempo");
    assert!((tempo.bpm - 120.0).abs() < 2.0, "{tempo:?}");
    // Cached: the same index without running FFmpeg again.
    assert_eq!(
        media_index(&path, &cache, &CancellationToken::new()).unwrap(),
        index
    );

    assert!(cached_proxy(&path, &cache).is_none());
    let proxy = make_proxy(&path, &cache, &CancellationToken::new())
        .unwrap()
        .expect("wider than a proxy");
    assert_eq!(cached_proxy(&path, &cache), Some(proxy.clone()));
    let (source, proxied) = (
        info(&path.to_string_lossy()).unwrap(),
        info(&proxy.to_string_lossy()).unwrap(),
    );
    assert_eq!(proxied.width, Some(PROXY_WIDTH));
    assert_eq!(proxied.fps, source.fps);
    assert_eq!(proxied.has_audio, Some(true));
    assert!((proxied.duration.unwrap() - source.duration.unwrap()).abs() < 0.1);
}

#[test]
fn a_single_shot_has_no_cuts() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("shot.mp4");
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:size=320x180:rate=25:duration=2",
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .status()
        .is_ok_and(|status| status.success());
    if !made {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let cache = DerivativeCache::new(directory.path());
    let index = media_index(&path, &cache, &CancellationToken::new()).unwrap();
    assert!(
        index.cuts.is_empty() && index.onsets.is_empty(),
        "{index:?}"
    );
    // Too small to need a proxy.
    assert!(make_proxy(&path, &cache, &CancellationToken::new())
        .unwrap()
        .is_none());
}
