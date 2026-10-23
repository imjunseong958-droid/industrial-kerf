//! Frames drawn by the compositor, written as images.

use crate::error::{EngineError, Result};
use crate::ffmpeg::resolve_ffmpeg_bin;
use kerf_compose::Frame;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `frame` to `path` as a PNG.
pub fn write_png(frame: &Frame, path: &Path) -> Result<()> {
    let mut child = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s"])
        .arg(format!("{}x{}", frame.width, frame.height))
        .args(["-i", "-", "-frames:v", "1", "-update", "1"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(&frame.rgba)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(EngineError::msg(format!(
            "ffmpeg could not write {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}
