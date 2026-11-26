//! Frames drawn by the compositor, written as images.

use crate::error::{EngineError, Result};
use crate::process::ffmpeg::resolve_ffmpeg_bin;
use kerf_compose::ComposeOptions;
pub use kerf_compose::{Compositor, Frame};
use kerf_model::{Catalog, Project, Time};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

/// Write `frame` to `path` as a PNG.
pub fn write_png(frame: &Frame, path: &Path) -> Result<()> {
    #[expect(
        clippy::disallowed_methods,
        reason = "short-lived and waited on: FFmpeg writes one frame it is given as a PNG"
    )]
    let mut child = crate::process::ffmpeg::hidden_command(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s"])
        .arg(format!("{}x{}", frame.width, frame.height))
        .args(["-i", "-", "-frames:v", "1", "-update", "1"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    // Read as it writes, so a full stderr cannot stop it taking the frame.
    let errors = crate::process::sidecar::drain_stderr(child.stderr.take());
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(&frame.rgba)?;
    let status = child.wait()?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() {
        return Err(EngineError::msg(format!(
            "ffmpeg could not write {}: {}",
            path.display(),
            errors.trim()
        )));
    }
    Ok(())
}

/// What draws the frames of `sequence` (the first when `None`) as [`draw_frame`] draws one,
/// for a run of them. Frames asked of it in order are decoded once through. Asked of
/// [`draw_frame`] one by one, each is decoded again from its media's last keyframe, so that a
/// run of them costs the more the further it is from one: thirty frames four seconds into a
/// clip with one keyframe took 22 s that way, and 2.6 s where every frame was one.
pub fn drawer(project: &Project, sequence: Option<&str>) -> Result<Compositor> {
    Ok(Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: sequence.map(str::to_owned),
            captions: true,
            keep_frames: true,
            frame_folder: kerf_compose::default_frame_folder(),
            ..Default::default()
        },
    )?)
}

/// The picture of `sequence` (the first when `None`) at sequence time `at`
/// as a render draws it, captions burned in, at `scale` of its canvas (1
/// is full size). Generators, titles among them, draw on their own; clips
/// whose media cannot be read draw nothing.
pub fn draw_frame(
    project: &Project,
    sequence: Option<&str>,
    at: Time,
    scale: f64,
) -> Result<Frame> {
    Ok(drawer(project, sequence)?.frame(at, scale))
}

/// [`draw_frame`], of an HDR sequence, as the 16-bit signal its master gets rather than what a monitor
/// shows of it. For an SDR sequence, its 16-bit picture.
pub fn draw_signal(
    project: &Project,
    sequence: Option<&str>,
    at: Time,
    scale: f64,
) -> Result<Frame> {
    Ok(drawer(project, sequence)?.frame16(at, scale))
}
