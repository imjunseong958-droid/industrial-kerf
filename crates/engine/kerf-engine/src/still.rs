//! Frames drawn by the compositor, written as images.

use crate::error::{EngineError, Result};
use crate::ffmpeg::resolve_ffmpeg_bin;
pub use kerf_compose::Frame;
use kerf_compose::{ComposeOptions, Compositor};
use kerf_model::{Catalog, Project, Time};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

/// Write `frame` to `path` as a PNG.
pub fn write_png(frame: &Frame, path: &Path) -> Result<()> {
    let mut child = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin())
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
    let errors = crate::raster::drain_stderr(&mut child);
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
    let compositor = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: sequence.map(str::to_owned),
            captions: true,
            keep_frames: true,
            frame_folder: kerf_compose::default_frame_folder(),
            ..Default::default()
        },
    )?;
    Ok(compositor.frame(at, scale))
}

/// [`draw_frame`], of an HDR sequence, as the 16-bit signal its master gets rather than what a monitor
/// shows of it. For an SDR sequence, its 16-bit picture.
pub fn draw_signal(
    project: &Project,
    sequence: Option<&str>,
    at: Time,
    scale: f64,
) -> Result<Frame> {
    let compositor = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: sequence.map(str::to_owned),
            captions: true,
            keep_frames: true,
            frame_folder: kerf_compose::default_frame_folder(),
            ..Default::default()
        },
    )?;
    Ok(compositor.frame16(at, scale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::Rate;

    fn lit(frame: &Frame) -> usize {
        frame
            .rgba
            .as_chunks::<4>().0.iter()
            .filter(|pixel| pixel[..3].iter().any(|channel| *channel > 32))
            .count()
    }

    #[test]
    fn a_title_generator_draws_its_text_while_it_plays() {
        let project = kerf_lang::parse_document(
            "kerf 1\nproject name=\"Still\"\n\n\
             sequence s1 name=\"Main\" rate=24 size=320x180 audio=48000 start=00:00:00:00\n\
             \x20 track V1 video\n\
             \x20   clip t1 gen=title@1 at=00:00:01:00 dur=00:00:01:00\n\
             \x20     set text=\"Day One\" size=0.3\n",
            Catalog::builtin(),
        )
        .unwrap()
        .project;
        let during = draw_frame(&project, None, Rate::FPS_24.time_of(36), 1.0).unwrap();
        assert_eq!((during.width, during.height), (320, 180));
        assert!(lit(&during) > 50, "the title draws");
        let before = draw_frame(&project, Some("s1"), Rate::FPS_24.time_of(12), 1.0).unwrap();
        assert_eq!(lit(&before), 0, "black before the title");
        assert!(draw_frame(&project, Some("missing"), Time::ZERO, 1.0).is_err());
    }
}
