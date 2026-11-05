//! ffmpeg/ffprobe process helpers. Array-arg spawns only (no shell).
//!
//! Binary paths, hardware-decode args, and encoder listings are resolved once
//! per process. Re-running `ffmpeg -version` / `-encoders` on every probe or
//! render was the dominant engine-side overhead outside ffmpeg itself.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::error::{EngineError, Result};
use crate::resources::ffmpeg_thread_count;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A command for a console tool, for what runs inside the editor as well as
/// kerfd. The editor has no console, so on Windows each such tool it starts
/// would otherwise open a window of its own.
#[cfg(windows)]
pub fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
pub fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    Command::new(program)
}

fn bin_exists(candidate: &str) -> bool {
    hidden_command(candidate)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn resolve_ffmpeg_bin() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(value) = std::env::var("KERF_FFMPEG") {
            if !value.is_empty() {
                candidates.push(value);
            }
        }
        candidates.push("/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg".into());
        candidates.push("ffmpeg".into());
        for candidate in candidates {
            if bin_exists(&candidate) {
                return candidate;
            }
        }
        "ffmpeg".into()
    })
    .as_str()
}

pub fn resolve_ffprobe_bin() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let ffmpeg = resolve_ffmpeg_bin();
        let path = Path::new(ffmpeg);
        if let Some(parent) = path.parent() {
            if path.file_name().and_then(|name| name.to_str()) == Some("ffmpeg") {
                let sibling = parent.join("ffprobe");
                if bin_exists(&sibling.to_string_lossy()) {
                    return sibling.to_string_lossy().into_owned();
                }
            }
        }
        "ffprobe".into()
    })
    .as_str()
}

/// Cached `-hwaccel <name>` pair for video decode, or empty when none exist.
/// Encode output is unchanged; this only accelerates decode into software filters.
///
/// Auto-selects VideoToolbox on macOS and D3D11VA on Windows. Other GPUs
/// (`cuda`, `qsv`, `vaapi`, …) must be opted in with `KERF_HWACCEL=<name>`.
/// `KERF_HWACCEL=none` disables it.
pub fn hwaccel_decode_args() -> &'static [String] {
    static ARGS: OnceLock<Vec<String>> = OnceLock::new();
    ARGS.get_or_init(|| {
        if let Ok(value) = std::env::var("KERF_HWACCEL") {
            let trimmed = value.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                return Vec::new();
            }
            if !trimmed.eq_ignore_ascii_case("auto") {
                return vec!["-hwaccel".into(), trimmed.to_ascii_lowercase()];
            }
        }
        let ffmpeg = resolve_ffmpeg_bin();
        let output = match run_allow_failure(ffmpeg, &["-hide_banner".into(), "-hwaccels".into()]) {
            Ok(output) => format!("{}\n{}", output.stdout, output.stderr),
            Err(_) => return Vec::new(),
        };
        let listed = |name: &str| output.lines().any(|line| line.trim() == name);
        if cfg!(target_os = "macos") && listed("videotoolbox") {
            vec!["-hwaccel".into(), "videotoolbox".into()]
        } else if cfg!(windows) && listed("d3d11va") {
            vec!["-hwaccel".into(), "d3d11va".into()]
        } else {
            Vec::new()
        }
    })
}

/// Cached `ffmpeg -encoders` listing used by hardware encoder selection.
pub fn cached_encoder_listing() -> Result<&'static str> {
    static LISTING: OnceLock<String> = OnceLock::new();
    if let Some(listing) = LISTING.get() {
        return Ok(listing.as_str());
    }
    let ffmpeg = resolve_ffmpeg_bin();
    let output = run_allow_failure(ffmpeg, &["-hide_banner".into(), "-encoders".into()])?;
    let listing = format!("{}\n{}", output.stdout, output.stderr);
    Ok(LISTING.get_or_init(|| listing).as_str())
}

pub fn run(bin: &str, args: &[String]) -> Result<CommandOutput> {
    let output = hidden_command(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                EngineError::msg(format!("{bin} not found on PATH"))
            } else {
                EngineError::from(error)
            }
        })?;
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if code == 0 {
        Ok(CommandOutput {
            code,
            stdout,
            stderr,
        })
    } else {
        Err(EngineError::Ffmpeg {
            code,
            stderr: format!(
                "{bin} exited with code {code}\nargs: {}\nstderr: {}",
                args.join(" "),
                tail(&stderr, 2000)
            ),
        })
    }
}

pub fn run_allow_failure(bin: &str, args: &[String]) -> Result<CommandOutput> {
    let output = hidden_command(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    Ok(CommandOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Parse ffmpeg's `time=HH:MM:SS.xx` (or `time=N.NN`) into seconds.
pub fn parse_time_seconds(time_str: &str) -> Option<f64> {
    let mut parts = time_str.split(':');
    let first = parts.next()?;
    if let (Some(minutes), Some(seconds)) = (parts.next(), parts.next()) {
        if parts.next().is_some() {
            return None;
        }
        let hours: f64 = first.parse().ok()?;
        let minutes: f64 = minutes.parse().ok()?;
        let seconds: f64 = seconds.parse().ok()?;
        return Some(hours * 3600.0 + minutes * 60.0 + seconds);
    }
    time_str
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// The last `max` bytes of `value`, starting at a character boundary.
pub(crate) fn tail(value: &str, max: usize) -> &str {
    let mut start = value.len().saturating_sub(max);
    while !value.is_char_boundary(start) {
        start += 1;
    }
    &value[start..]
}

/// Prefix used on video decode/encode jobs: hwaccel (if any) plus a laptop-capped thread count.
pub fn video_decode_prefix() -> &'static [String] {
    static PREFIX: OnceLock<Vec<String>> = OnceLock::new();
    PREFIX.get_or_init(|| decode_prefix_with_threads(ffmpeg_thread_count()))
}

/// [`video_decode_prefix`] for a compiled program's arguments, or nothing
/// when the program reads no file (only titles and solids): its options
/// would land on the output.
pub fn program_decode_prefix(argv: &[String]) -> Vec<String> {
    if argv.iter().any(|arg| arg == "-i") {
        video_decode_prefix().to_vec()
    } else {
        Vec::new()
    }
}

fn decode_prefix_with_threads(threads: usize) -> Vec<String> {
    let mut args = hwaccel_decode_args().to_vec();
    args.extend(["-threads".into(), threads.max(1).to_string()]);
    args
}

#[cfg(test)]
mod tests {
    use super::{parse_time_seconds, tail};

    #[test]
    fn parses_hms_and_raw_seconds() {
        assert!((parse_time_seconds("00:01:02.50").unwrap() - 62.5).abs() < 1e-9);
        assert!((parse_time_seconds("12.34").unwrap() - 12.34).abs() < 1e-9);
        assert_eq!(parse_time_seconds("garbage"), None);
    }

    #[test]
    fn tail_keeps_whole_characters() {
        assert_eq!(tail("영상.mp4", 6), ".mp4");
        assert_eq!(tail("abc", 10), "abc");
    }
}
