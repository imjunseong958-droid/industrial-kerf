//! A render made first thing in a process, before the GPU's kernels are built, against the same
//! render made once they are: the same frames. They are not where the first is drawn by the CPU
//! until the kernels are built, since the CPU and the GPU draw a frame within 1 of 255 of each
//! other and not to the same bytes. On a machine with no GPU both are the CPU's, and there is
//! nothing to tell apart.
//!
//! The only test of this file, since the kernels are built once for the whole process.

use kerf_engine::process::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::render::{render_program, CacheFolder, RenderOptions, RenderPreset};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Large enough to be the GPU's, with edges and mixes that the CPU and the GPU round apart.
const TEXT: &str = "kerf 1
project name=\"t\"
sequence s1 rate=24 size=640x360 audio=48000
  track V1 video
    clip bg gen=solid@1 at=0f dur=6f
      set color=#3060a0
  track V2 video
    clip a gen=solid@1 at=0f dur=6f
      set color=#e08030 scale=0.6 rotation={0f:5 6f:45} opacity=0.7
  track V3 video
    clip b gen=solid@1 at=0f dur=6f
      set color=#30c060 scale=0.5 x=0.2 y=0.1 rotation={0f:-31 6f:-11} opacity=0.6 blend=screen
";

/// `project` rendered to `out` as a master, with nothing kept: every frame is drawn.
fn render(project: &Project, out: &Path) {
    render_program::<fn(f64)>(
        project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            preset: RenderPreset::ProRes422Hq,
            frame_cache: CacheFolder::Off,
            section_cache: CacheFolder::Off,
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
        .args(["-f", "rawvideo", "-pix_fmt", "yuv422p10le", "-"])
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn a_render_before_the_gpu_is_ready_is_the_render_after() {
    let project = parse_document(TEXT, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let folder = tempfile::tempdir().expect("a folder");
    let (first, second) = (
        folder.path().join("first.mov"),
        folder.path().join("second.mov"),
    );
    render(&project, &first);
    // Whatever was building the kernels behind that render has built them before the next.
    assert!(
        kerf_compose::gpu_settled(Duration::from_secs(600)),
        "the GPU's kernels are still being built"
    );
    render(&project, &second);
    let (first, second) = (frames(&first), frames(&second));
    assert!(!first.is_empty(), "the render has frames");
    let differ = first.iter().zip(&second).filter(|(a, b)| a != b).count();
    assert!(
        first.len() == second.len() && differ == 0,
        "{differ} of {} bytes differ between the first render and the second",
        first.len()
    );
}
