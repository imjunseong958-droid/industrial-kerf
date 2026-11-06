//! Rendering again after a small edit: only what changed is drawn, and the file comes out the same.
//! The only test of this file, since it sets where frames are kept for the whole process.

use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::render::{render_program, RenderOptions};
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

fn render(project: &Project, out: &Path) -> f64 {
    let began = Instant::now();
    render_program::<fn(f64)>(
        project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
    began.elapsed().as_secs_f64()
}

/// The file's frames, as FFmpeg decodes them.
fn frames(path: &Path) -> Vec<u8> {
    let output = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success());
    output.stdout
}

#[test]
fn rendering_again_reads_what_was_drawn() {
    let dir = tempfile::tempdir().expect("a folder");
    let folder = dir.path().join("frames");
    std::env::set_var("KERF_FRAME_CACHE", &folder);
    let source = dir.path().join("source.mp4");
    let made = Command::new(resolve_ffmpeg_bin())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("testsrc2=s=1280x720:r=24:d=4")
        .args([
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .arg(&source)
        .status()
        .expect("ffmpeg runs");
    assert!(made.success());
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=96f video=1280x720\nsequence s1 rate=24 size=1280x720 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=48f\n      fx color.lumetri@2 exposure=0.5\n      fx light.wonderglow@1\n    clip c2 media=m src=48f at=48f dur=48f\n      fx color.lumetri@2 exposure=0.2\n      fx light.wonderglow@1\n",
            source.display()
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let (first, second, third) = (
        dir.path().join("first.mp4"),
        dir.path().join("second.mp4"),
        dir.path().join("third.mp4"),
    );
    let drawing = render(&project, &first);
    let stats = kerf_compose::frame_cache_stats(&folder);
    assert_eq!(stats.hits, 0, "{stats:?}");
    assert!(stats.writes + stats.dropped == 96, "{stats:?}");
    let reading = render(&project, &second);
    let after = kerf_compose::frame_cache_stats(&folder);
    eprintln!(
        "96 frames at 720p: drawn in {drawing:.2} s, read back in {reading:.2} s ({} of them)",
        after.hits - stats.hits
    );
    assert_eq!(after.hits - stats.hits, stats.writes, "{after:?}");
    assert_eq!(frames(&first), frames(&second));
    // A small edit to the second clip draws only it.
    let edited = apply_script(&project, Catalog::builtin(), "fx c2 set 1 exposure=0.3")
        .expect("edits")
        .project;
    let touched = render(&edited, &third);
    let last = kerf_compose::frame_cache_stats(&folder);
    eprintln!("after an edit to half of it: {touched:.2} s");
    assert!(last.hits - after.hits >= 40, "{last:?}");
    assert_eq!(
        kerf_compose::frame_cache_stats(&folder).damaged,
        0,
        "{last:?}"
    );
}
