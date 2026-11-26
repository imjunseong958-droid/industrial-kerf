//! Rendering again after a small edit: only what changed is drawn, and the file comes out the same.
//! And where a render keeps what it drew when nothing names a folder.

use kerf_engine::process::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::render::{render_program, CacheFolder, RenderOptions};
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;

/// `project` rendered to `out`, keeping its frames in `frames`.
fn render(project: &Project, out: &Path, frames: &Path) {
    render_program::<fn(f64)>(
        project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            frame_cache: CacheFolder::At(frames.to_owned()),
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
}

/// The file's frames, as FFmpeg decodes them.
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn rendering_again_reads_what_was_drawn() {
    let dir = tempfile::tempdir().expect("a folder");
    let folder = dir.path().join("frames");
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
        .arg("testsrc2=s=320x180:r=24:d=1")
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
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n      fx color.lumetri@2 exposure=0.5\n      fx blur.unsharp@2\n    clip c2 media=m src=12f at=12f dur=12f\n      fx color.lumetri@2 exposure=0.2\n      fx blur.unsharp@2\n",
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
    // Every frame is drawn and kept: 24 are fewer than the writers' queue holds, so none is dropped.
    render(&project, &first, &folder);
    let stats = kerf_compose::frame_cache_stats(&folder);
    assert_eq!(
        (stats.hits, stats.writes, stats.dropped),
        (0, 24, 0),
        "{stats:?}"
    );
    render(&project, &second, &folder);
    let after = kerf_compose::frame_cache_stats(&folder);
    assert_eq!(after.hits - stats.hits, 24, "{after:?}");
    assert_eq!(after.writes, stats.writes, "{after:?}");
    assert_eq!(frames(&first), frames(&second));
    // A small edit to the second clip draws only it.
    let edited = apply_script(&project, Catalog::builtin(), "fx c2 set 1 exposure=0.3")
        .expect("edits")
        .project;
    render(&edited, &third, &folder);
    let last = kerf_compose::frame_cache_stats(&folder);
    assert_eq!(last.hits - after.hits, 12, "{last:?}");
    assert_eq!(last.writes - after.writes, 12, "{last:?}");
    assert_eq!(last.damaged, 0, "{last:?}");
}

/// Every variable Kerf's cache folder is found by, set to nothing, names no folder: a render keeps no
/// frames and no stretches in the folder it was started from.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn a_cache_variable_set_to_nothing_keeps_nothing_where_a_render_starts() {
    let dir = tempfile::tempdir().expect("a folder");
    let start = dir.path().join("start");
    std::fs::create_dir(&start).expect("a folder to start in");
    let project = dir.path().join("project.kerf");
    std::fs::write(
        &project,
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n",
    )
    .expect("writes the project");
    let output = Command::new(env!("CARGO_BIN_EXE_kerf-engine"))
        .arg("render")
        .arg(&project)
        .arg("--draft")
        .arg("--out")
        .arg(dir.path().join("out.mp4"))
        .current_dir(&start)
        .env_remove("KERF_FRAME_CACHE")
        .env_remove("KERF_SECTION_CACHE")
        .env("LOCALAPPDATA", "")
        .env("HOME", "")
        .env("XDG_CACHE_HOME", "")
        .output()
        .expect("kerf-engine runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let kept: Vec<_> = std::fs::read_dir(&start)
        .expect("reads the folder")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert!(kept.is_empty(), "{kept:?}");
}
