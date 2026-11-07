//! What it costs to play a sequence as the editor plays it, kept up for as long as asked.
//!
//! ```text
//! cargo run --release -p kerf-playback --example playback_bench -- <project folder> [seconds]
//! ```
//!
//! Opens `project.kerf` the way the editor's media worker does, plays it from the start, and takes
//! each frame as it comes. Every 5 seconds it prints the frames shown and dropped since the last line,
//! the size they came at, the share of a core it used (400% is four cores), and the process's memory.
//! The machine warms over a long run, so a number from the start and one from the end are not the
//! same run's. `KERF_FRAME_CACHE=off` leaves the disk out of it.

use kerf_lang::parse_document;
use kerf_model::Catalog;
use kerf_playback::{preview_plan, PlaybackConfig, PlaybackEvent, PlaybackSession};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// Seconds of CPU the process has used, and its resident memory in MB, as `ps` says.
fn usage() -> (f64, f64) {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "time=,rss=", "-p", &pid])
        .output()
        .expect("ps");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut fields = text.split_whitespace();
    let time = fields.next().unwrap_or("0:0");
    let seconds = time.split(':').fold(0.0, |total, part| {
        total * 60.0 + part.parse::<f64>().unwrap_or(0.0)
    });
    let rss = fields
        .next()
        .and_then(|rss| rss.parse::<f64>().ok())
        .unwrap_or(0.0);
    (seconds, rss / 1024.0)
}

fn main() {
    let folder = std::env::args().nth(1).expect("a project folder");
    let seconds: f64 = std::env::args()
        .nth(2)
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(60.0);
    let folder = Path::new(&folder);
    let text = std::fs::read_to_string(folder.join("project.kerf")).expect("project.kerf");
    let mut project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|problems| panic!("{problems:?}"))
        .project;
    project.resolve_paths(Catalog::builtin(), folder);

    let plan = preview_plan(&project);
    let composited = plan.composited();
    println!("plays {:?}", plan.mode);
    let session = PlaybackSession::open(
        project,
        PlaybackConfig::default()
            .with_frame_size(1920, 1080)
            .with_composited(composited),
    )
    .expect("opens");
    session.play().expect("plays");

    let started = Instant::now();
    let (mut shown, mut last_shown, mut dropped, mut last_dropped) = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut size = (0, 0);
    let (mut cpu_then, _) = usage();
    let mut then = Instant::now();
    let mut peak = 0.0_f64;
    println!("  time  shown/s  dropped/s      size    cpu   memory");
    while started.elapsed().as_secs_f64() < seconds {
        if let Some(event) = session.next_event_timeout(Duration::from_millis(50)) {
            match event {
                PlaybackEvent::Frame(frame) => {
                    shown += 1;
                    size = (frame.width, frame.height);
                }
                PlaybackEvent::Clock(clock) => dropped = dropped.max(clock.dropped),
                PlaybackEvent::Ended => break,
                PlaybackEvent::Error(error) => panic!("{error}"),
                _ => {}
            }
        }
        if then.elapsed() >= Duration::from_secs(5) {
            let spent = then.elapsed().as_secs_f64();
            let (cpu, memory) = usage();
            peak = peak.max(memory);
            println!(
                "{:6.0}s {:8.1} {:10.1} {:>5}x{:<4} {:4.0}% {:6.0} MB",
                started.elapsed().as_secs_f64(),
                (shown - last_shown) as f64 / spent,
                (dropped - last_dropped) as f64 / spent,
                size.0,
                size.1,
                (cpu - cpu_then) / spent * 100.0,
                memory
            );
            (last_shown, last_dropped, cpu_then, then) = (shown, dropped, cpu, Instant::now());
        }
    }
    println!(
        "{shown} shown, {dropped} dropped over {:.0} s, at most {peak:.0} MB",
        started.elapsed().as_secs_f64()
    );
}
