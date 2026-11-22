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

/// Whether `program -version` starts and ends well.
fn runs(program: &str) -> bool {
    Command::new(program)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// FFmpeg and ffprobe, as Kerf finds them (`kerf-engine/src/process/ffmpeg.rs`).
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
        let candidates: Vec<String> = std::env::var("KERF_FFMPEG")
            .ok()
            .filter(|named| !named.is_empty())
            .into_iter()
            .chain(beside.map(|file| file.to_string_lossy().into_owned()))
            .chain(["/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg".to_owned()])
            .collect();
        Self::first_that_runs(&candidates, runs)
    }

    /// The first of `candidates` whose FFmpeg and the ffprobe beside it both run, as
    /// the engine asks of them: one that is there but cannot start, such as a Homebrew
    /// FFmpeg whose libraries an upgrade took away, is passed over. Else `PATH`'s.
    fn first_that_runs(candidates: &[String], runs: impl Fn(&str) -> bool) -> Self {
        let suffix = std::env::consts::EXE_SUFFIX;
        candidates
            .iter()
            .filter(|candidate| Path::new(candidate.as_str()).is_file())
            .find_map(|ffmpeg| {
                let ffprobe = Path::new(ffmpeg)
                    .parent()?
                    .join(format!("ffprobe{suffix}"))
                    .to_string_lossy()
                    .into_owned();
                (runs(ffmpeg) && runs(&ffprobe)).then(|| Self {
                    ffmpeg: ffmpeg.clone(),
                    ffprobe,
                })
            })
            .unwrap_or_else(|| Self {
                ffmpeg: "ffmpeg".to_owned(),
                ffprobe: "ffprobe".to_owned(),
            })
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

    /// What `file`'s picture is: its size, its rate, and how many frames it has.
    pub fn picture(&self, file: &Path) -> Result<Picture, String> {
        let said = run(
            Command::new(&self.ffprobe)
                .args(["-v", "error", "-select_streams", "v:0", "-count_packets"])
                .args([
                    "-show_entries",
                    "stream=width,height,r_frame_rate,nb_read_packets",
                ])
                .args(["-of", "default=noprint_wrappers=1"])
                .arg(file),
            &self.ffprobe,
        )?;
        Picture::parse(&said)
            .ok_or_else(|| format!("ffprobe found no picture in the guide {}", file.display()))
    }

    /// A small black picture at `to`, for a node that will not run without one.
    pub fn black(&self, to: &Path) -> Result<(), String> {
        run(
            Command::new(&self.ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(["-f", "lavfi", "-i", "color=black:s=16x16", "-frames:v", "1"])
                .arg(to),
            &self.ffmpeg,
        )
        .map(drop)
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

/// A file's picture, as ffprobe says it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub size: (u32, u32),
    pub fps: f64,
    pub frames: u32,
}

impl Picture {
    /// Read ffprobe's `name=value` lines.
    fn parse(said: &str) -> Option<Self> {
        let value = |name: &str| {
            said.lines()
                .find_map(|line| line.trim().strip_prefix(name)?.strip_prefix('='))
        };
        // A rate is two whole numbers: `24000/1001`.
        let (over, under) = value("r_frame_rate")?.split_once('/')?;
        let fps = over.parse::<f64>().ok()? / under.parse::<f64>().ok()?;
        let picture = Self {
            size: (
                value("width")?.parse().ok()?,
                value("height")?.parse().ok()?,
            ),
            fps,
            frames: value("nb_read_packets")?.parse().ok()?,
        };
        (fps.is_finite()
            && fps > 0.0
            && picture.frames > 0
            && picture.size.0 > 0
            && picture.size.1 > 0)
            .then_some(picture)
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
impl Tools {
    /// A grey video at `to` of `size`, `fps` frames a second: each frame as light all over
    /// as its number in `levels` says, 0 black and 1 white. It is how a number a frame
    /// reaches a node that reads a mask a frame.
    pub fn levels(
        &self,
        levels: &[f64],
        (width, height): (u32, u32),
        fps: f64,
        to: &Path,
    ) -> Result<(), String> {
        let pixels = (width * height) as usize;
        let frames: Vec<Vec<u8>> = levels
            .iter()
            .map(|level| vec![(level.clamp(0.0, 1.0) * 255.0).round() as u8; pixels])
            .collect();
        self.greys(&frames, (width, height), fps, to)
    }

    /// A grey video at `to` of `size`, `fps` frames a second, of `frames`, each `width` by
    /// `height` bytes row by row.
    pub fn greys(
        &self,
        frames: &[Vec<u8>],
        (width, height): (u32, u32),
        fps: f64,
        to: &Path,
    ) -> Result<(), String> {
        use std::io::Write;
        let mut child = Command::new(&self.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "gray", "-color_range", "pc"])
            .args([
                "-s",
                &format!("{width}x{height}"),
                "-r",
                &fps.to_string(),
                "-i",
                "-",
            ])
            .args(["-c:v", "libx264", "-crf", "10", "-pix_fmt", "yuv420p"])
            .arg(to)
            .stdin(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| {
                format!(
                    "{} could not start: {error}; KERF_FFMPEG names where FFmpeg is",
                    self.ffmpeg
                )
            })?;
        let mut stdin = child
            .stdin
            .take()
            .expect("the command above pipes stdin, and nothing took the pipe before this");
        for frame in frames {
            // FFmpeg ending early is said by its status, below.
            if stdin.write_all(frame).is_err() {
                break;
            }
        }
        drop(stdin);
        let output = child
            .wait_with_output()
            .map_err(|error| format!("{} failed: {error}", self.ffmpeg))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{} failed ({}): {}",
                self.ffmpeg,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

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

    /// An FFmpeg that is there but does not start (Homebrew's, after an upgrade
    /// took a library it loads) is passed over for the next, and then for `PATH`'s.
    #[test]
    fn an_ffmpeg_that_does_not_start_is_passed_over() {
        let folder = std::env::temp_dir().join(format!("kerf-comfy-tools-{}", std::process::id()));
        let suffix = std::env::consts::EXE_SUFFIX;
        let (broken, working) = (folder.join("broken"), folder.join("working"));
        for place in [&broken, &working] {
            std::fs::create_dir_all(place).unwrap();
            for tool in ["ffmpeg", "ffprobe"] {
                std::fs::write(place.join(format!("{tool}{suffix}")), "").unwrap();
            }
        }
        let path = |place: &Path, tool: &str| {
            place
                .join(format!("{tool}{suffix}"))
                .to_string_lossy()
                .into_owned()
        };
        let candidates = [path(&broken, "ffmpeg"), path(&working, "ffmpeg")];
        let starts = |program: &str| !program.contains("broken");
        assert_eq!(
            Tools::first_that_runs(&candidates, starts),
            Tools {
                ffmpeg: path(&working, "ffmpeg"),
                ffprobe: path(&working, "ffprobe"),
            }
        );
        // An ffprobe that does not start takes its FFmpeg with it.
        let probe_broken =
            |program: &str| program != path(&working, "ffprobe") && !program.contains("broken");
        assert_eq!(
            Tools::first_that_runs(&candidates, probe_broken),
            Tools {
                ffmpeg: "ffmpeg".into(),
                ffprobe: "ffprobe".into(),
            }
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
