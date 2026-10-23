//! Baseline benches for the ffmpeg-CLI engine hot path.
//!
//! Run with `cargo bench -p kerf-engine --bench engine_hotpath`.
//! Range-render and seek benches need a working ffmpeg (`KERF_FFMPEG` optional).

use criterion::{criterion_group, criterion_main, Criterion};
use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::program::{compile_program, ProgramOptions};
use kerf_engine::render::{render_program, RenderOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn ffmpeg_available() -> bool {
    Command::new(resolve_ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn fixture_dir() -> PathBuf {
    std::env::temp_dir().join("kerf-engine-benches")
}

fn ensure_fixture(path: &Path, args: &[&str]) {
    if path.is_file() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let status = Command::new(resolve_ffmpeg_bin())
        .args(args)
        .status()
        .expect("spawn ffmpeg fixture");
    assert!(
        status.success(),
        "ffmpeg fixture failed for {}",
        path.display()
    );
}

fn long_source() -> PathBuf {
    let path = fixture_dir().join("long-610.mp4");
    ensure_fixture(
        &path,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x180:r=30:d=610",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "30",
            "-pix_fmt",
            "yuv420p",
            "-an",
            path.to_str().expect("utf8"),
        ],
    );
    path
}

fn short_source() -> PathBuf {
    let path = fixture_dir().join("short-2.mp4");
    ensure_fixture(
        &path,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "30",
            "-pix_fmt",
            "yuv420p",
            "-an",
            path.to_str().expect("utf8"),
        ],
    );
    path
}

/// One hundred one-second cuts of one source.
fn hundred_clips() -> kerf_model::Project {
    let mut text = String::from(
        "kerf 1\nproject name=\"bench\"\n\
         media a path=\"/tmp/a.mp4\" rate=30 dur=610s video=320x180\n\
         sequence s1 rate=30 size=320x180 audio=48000\n\
         \x20 track V1 video\n",
    );
    for index in 0..100 {
        text.push_str(&format!(
            "    clip c{index} media=a src={index}s at={index}s dur=1s\n"
        ));
    }
    kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
        .expect("bench project")
        .project
}

fn bench_compile_100(c: &mut Criterion) {
    let project = hundred_clips();
    c.bench_function("compile_program_100_clips", |bencher| {
        bencher.iter(|| {
            compile_program(
                &project,
                kerf_model::Catalog::builtin(),
                &ProgramOptions::default(),
            )
            .expect("compile")
        });
    });
}

fn bench_ffmpeg_spawn(c: &mut Criterion) {
    if !ffmpeg_available() {
        eprintln!("skipping ffmpeg_spawn: ffmpeg unavailable");
        return;
    }
    let ffmpeg = resolve_ffmpeg_bin();
    c.bench_function("ffmpeg_spawn_roundtrip", |bencher| {
        bencher.iter(|| {
            let status = Command::new(ffmpeg)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=64x36:r=30",
                    "-frames:v",
                    "1",
                    "-f",
                    "null",
                    "-",
                ])
                .status()
                .expect("spawn ffmpeg");
            assert!(status.success());
        });
    });
}

/// One 320x180 clip of `seconds` at 30 fps, without sound.
fn kernel_project(source: &Path, seconds: u32) -> kerf_model::Project {
    let text = format!(
        "kerf 1\nproject name=\"bench\"\n\
         media a path=\"{}\" rate=30 dur={seconds}s video=320x180\n\
         sequence s1 rate=30 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip full media=a src=0f at=0f dur={seconds}s\n",
        source.display()
    );
    kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
        .expect("bench project")
        .project
}

fn bench_range_render(c: &mut Criterion) {
    if !ffmpeg_available() {
        eprintln!("skipping range_render: ffmpeg unavailable");
        return;
    }
    let source = long_source();
    let project = kernel_project(&source, 610);
    let mut group = c.benchmark_group("render_program_range_5s");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(20));
    for start in [0.0_f64, 600.0] {
        group.bench_function(format!("t={start}"), |bencher| {
            bencher.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let output = tempfile::NamedTempFile::new().expect("output");
                    let path = output.path().with_extension("mp4");
                    let opts = RenderOptions {
                        output: path.to_string_lossy().into_owned(),
                        draft: true,
                        force_software: true,
                        ..RenderOptions::default()
                    };
                    let rate = kerf_model::Rate::FPS_30;
                    let from = rate.time_of((start * 30.0) as i64);
                    let range = Some((from, from + rate.time_of(150)));
                    let started = Instant::now();
                    render_program(&project, None, range, &opts, None::<fn(f64)>).expect("render");
                    total += started.elapsed();
                    let _ = std::fs::remove_file(&path);
                }
                total
            });
        });
    }
    group.finish();
}

fn wait_for_frame(
    session: &kerf_playback::PlaybackSession,
    timeout: Duration,
) -> kerf_playback::PlaybackFrame {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(event) = session.next_event_timeout(Duration::from_millis(50)) {
            if let kerf_playback::PlaybackEvent::Frame(frame) = event {
                return frame;
            }
            if let kerf_playback::PlaybackEvent::Error(error) = event {
                panic!("playback error: {error}");
            }
        }
    }
    panic!("no frame before timeout");
}

fn bench_seek_ttf(c: &mut Criterion) {
    if !ffmpeg_available() {
        eprintln!("skipping seek_ttf: ffmpeg unavailable");
        return;
    }
    let source = short_source();
    let project = kernel_project(&source, 2);
    let mut group = c.benchmark_group("seek_ttf");
    group.sample_size(20);
    group.bench_function("large_1_25", |bencher| {
        bencher.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let session = kerf_playback::PlaybackSession::open(
                    project.clone(),
                    kerf_playback::PlaybackConfig::default()
                        .with_audio_enabled(false)
                        .with_frame_size(320, 180)
                        .with_decode_queue_capacity(1),
                )
                .expect("open");
                let _ = wait_for_frame(&session, Duration::from_secs(5));
                let started = Instant::now();
                session.seek(1.25).expect("seek");
                let _ = wait_for_frame(&session, Duration::from_secs(5));
                total += started.elapsed();
            }
            total
        });
    });
    group.bench_function("small_plus_0_2", |bencher| {
        bencher.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let session = kerf_playback::PlaybackSession::open(
                    project.clone(),
                    kerf_playback::PlaybackConfig::default()
                        .with_audio_enabled(false)
                        .with_frame_size(320, 180)
                        .with_decode_queue_capacity(1),
                )
                .expect("open");
                let first = wait_for_frame(&session, Duration::from_secs(5));
                let started = Instant::now();
                session.seek(first.timeline_time + 0.2).expect("seek");
                let _ = wait_for_frame(&session, Duration::from_secs(5));
                total += started.elapsed();
            }
            total
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_compile_100,
    bench_ffmpeg_spawn,
    bench_range_render,
    bench_seek_ttf
);
criterion_main!(benches);
