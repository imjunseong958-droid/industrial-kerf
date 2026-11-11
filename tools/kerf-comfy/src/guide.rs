//! A guide as the model takes it: at the model's rate and size, and as many
//! frames as are asked for.
//!
//! Kerf draws a guide at its sequence's size and rate, and a model follows
//! one frame for frame at its own: a guide at 24 frames a second handed to a
//! model that makes 16 would be followed at two thirds of its speed, and only
//! as far as the model's frames reach.
//! So FFmpeg makes the copy that goes up, and it is kept beside the request,
//! where a person can see what the model was given.

use std::path::Path;
use std::process::Command;

/// FFmpeg and ffprobe, as Kerf finds them (`kerf-engine/src/ffmpeg.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tools {
    ffmpeg: String,
    ffprobe: String,
}

impl Tools {
    /// `KERF_FFMPEG`, then beside this program, as a package carries them,
    /// then Homebrew's, then `PATH`; ffprobe is beside the FFmpeg found.
    pub fn find() -> Self {
        let suffix = std::env::consts::EXE_SUFFIX;
        let beside = std::env::current_exe()
            .ok()
            .and_then(|program| Some(program.parent()?.join(format!("ffmpeg{suffix}"))));
        let ffmpeg = std::env::var("KERF_FFMPEG")
            .ok()
            .filter(|named| !named.is_empty())
            .into_iter()
            .chain(beside.map(|file| file.to_string_lossy().into_owned()))
            .chain(["/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg".to_owned()])
            .find(|candidate| Path::new(candidate).is_file())
            .unwrap_or_else(|| "ffmpeg".to_owned());
        let ffprobe = Path::new(&ffmpeg)
            .parent()
            .map(|folder| folder.join(format!("ffprobe{suffix}")))
            .filter(|file| file.is_file())
            .map_or_else(
                || "ffprobe".to_owned(),
                |file| file.to_string_lossy().into_owned(),
            );
        Self { ffmpeg, ffprobe }
    }

    /// How long `file` plays, in seconds.
    pub fn seconds(&self, file: &Path) -> Result<f64, String> {
        let said = run(
            Command::new(&self.ffprobe)
                .args(["-v", "error", "-show_entries", "format=duration"])
                .args(["-of", "default=noprint_wrappers=1:nokey=1"])
                .arg(file),
            &self.ffprobe,
        )?;
        said.trim()
            .parse::<f64>()
            .ok()
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .ok_or_else(|| format!("ffprobe gave no length for the guide {}", file.display()))
    }

    /// Write `guide` to `to` as the model takes it.
    pub fn conform(&self, guide: &Path, to: &Path, shape: &Shape) -> Result<(), String> {
        run(
            Command::new(&self.ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(guide)
                .args(shape.arguments())
                .arg(to),
            &self.ffmpeg,
        )
        .map(drop)
    }
}

/// What a guide is made to fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub size: (u32, u32),
    pub fps: f64,
    pub frames: u32,
    /// The most frames its last one is held for, to reach `frames`: a count
    /// the model takes is a frame or a few past what a guide of the length
    /// asked has.
    pub hold: u32,
}

impl Shape {
    /// FFmpeg's arguments between the guide and the file it writes: the
    /// model's rate, the picture filling the model's size with what is over
    /// cut off evenly, and no sound.
    fn arguments(&self) -> Vec<String> {
        let (width, height) = self.size;
        let filter = format!(
            "fps={},scale={width}:{height}:force_original_aspect_ratio=increase:flags=bicubic,crop={width}:{height},setsar=1,tpad=stop_mode=clone:stop={}",
            self.fps, self.hold
        );
        ["-an", "-vf", &filter, "-frames:v", &self.frames.to_string()]
            .into_iter()
            .chain([
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "14", "-pix_fmt", "yuv420p",
            ])
            .map(str::to_owned)
            .collect()
    }
}

/// Run `command` to its end and give what it wrote; one that does not end
/// well is an error with what it said.
fn run(command: &mut Command, program: &str) -> Result<String, String> {
    let output = command.output().map_err(|error| {
        format!("{program} could not start: {error}; KERF_FFMPEG names where FFmpeg is")
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{program} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guide_is_made_to_fit_the_model() {
        let shape = Shape {
            size: (832, 480),
            fps: 16.0,
            frames: 65,
            hold: 4,
        };
        assert_eq!(
            shape.arguments(),
            [
                "-an",
                "-vf",
                "fps=16,scale=832:480:force_original_aspect_ratio=increase:flags=bicubic,crop=832:480,setsar=1,tpad=stop_mode=clone:stop=4",
                "-frames:v",
                "65",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "14",
                "-pix_fmt",
                "yuv420p"
            ]
        );
        // A rate that is not whole is written as it is.
        let ntsc = Shape {
            fps: 23.976,
            ..shape
        };
        assert!(ntsc.arguments()[2].starts_with("fps=23.976,"));
    }
}
