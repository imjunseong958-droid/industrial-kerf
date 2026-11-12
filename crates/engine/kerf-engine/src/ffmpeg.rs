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
        let program = std::env::current_exe().ok();
        let beside = program.as_deref().and_then(Path::parent);
        ffmpeg_candidates(std::env::var("KERF_FFMPEG").ok(), beside)
            .into_iter()
            .find(|candidate| bin_exists(candidate))
            .unwrap_or_else(|| "ffmpeg".into())
    })
    .as_str()
}

/// Where to look for ffmpeg, in order: `KERF_FFMPEG`; beside the program that is running, as
/// each package carries it (kerfd, kerf and the editor sit beside it); Homebrew's
/// `ffmpeg-full`; and then `PATH`. Windows looks beside the program by itself, but macOS and
/// Linux would not, and a Mac without Homebrew has no ffmpeg on `PATH`: there kerfd imported
/// media with no probe, at a made-up size, rate and length.
fn ffmpeg_candidates(configured: Option<String>, beside: Option<&Path>) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    candidates.extend(configured.filter(|value| !value.is_empty()));
    if let Some(folder) = beside {
        let bundled = folder.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        candidates.push(bundled.to_string_lossy().into_owned());
    }
    candidates.push("/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg".into());
    candidates.push("ffmpeg".into());
    candidates
}

pub fn resolve_ffprobe_bin() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let sibling = ffprobe_beside(resolve_ffmpeg_bin());
        match sibling {
            Some(sibling) if bin_exists(&sibling) => sibling,
            _ => "ffprobe".into(),
        }
    })
    .as_str()
}

/// The ffprobe that comes with the ffmpeg at `ffmpeg`, when that is a path.
fn ffprobe_beside(ffmpeg: &str) -> Option<String> {
    let path = Path::new(ffmpeg);
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty())?;
    (path.file_stem()? == "ffmpeg").then(|| {
        parent
            .join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX))
            .to_string_lossy()
            .into_owned()
    })
}

/// Cached `-hwaccel <name>` pair for video decode, or empty when none exist.
/// Encode output is unchanged; this only accelerates decode into software filters.
///
/// Auto-selects VideoToolbox on macOS, but not in a virtual machine (see
/// [`runs_in_a_guest`]), and D3D11VA on Windows. Other GPUs (`cuda`, `qsv`,
/// `vaapi`, …) must be opted in with `KERF_HWACCEL=<name>`, which also asks
/// for VideoToolbox in a guest. `KERF_HWACCEL=none` disables it.
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
        if cfg!(target_os = "macos") && listed("videotoolbox") && !runs_in_a_guest() {
            vec!["-hwaccel".into(), "videotoolbox".into()]
        } else if cfg!(windows) && listed("d3d11va") {
            vec!["-hwaccel".into(), "d3d11va".into()]
        } else {
            Vec::new()
        }
    })
}

/// Whether this Mac runs under a hypervisor, by `kern.hv_vmm_present`.
///
/// VideoToolbox in a guest sets up paravirtualized decoding when a session
/// is first made. On GitHub's macOS runners that set-up stops answering
/// after some sessions, and from then on every ffmpeg asked for
/// VideoToolbox waits in it for ever
/// (`VTDecompressionSessionCreateWithOptions`, in
/// `VTParavirtualizationGuestSupportSetUpWithHandlers`) and stays behind
/// when killed, so whatever reads its frames waits too. A guest has no
/// decoder of its own to gain from: it decodes in software.
fn runs_in_a_guest() -> bool {
    cfg!(target_os = "macos")
        && Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.hv_vmm_present"])
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "1")
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

fn decode_prefix_with_threads(threads: usize) -> Vec<String> {
    let mut args = hwaccel_decode_args().to_vec();
    args.extend(["-threads".into(), threads.max(1).to_string()]);
    args
}

#[cfg(test)]
mod tests {
    use super::{ffmpeg_candidates, ffprobe_beside, parse_time_seconds, tail};
    use std::path::Path;

    // A package keeps ffmpeg beside kerfd, kerf and the editor; it is found there before
    // Homebrew's or PATH's, and after one KERF_FFMPEG names.
    #[test]
    fn ffmpeg_is_looked_for_beside_the_program_before_path() {
        let folder = Path::new("/Applications/Industrial Kerf.app/Contents/MacOS");
        let bundled = folder
            .join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX))
            .to_string_lossy()
            .into_owned();
        let homebrew = "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg".to_owned();
        assert_eq!(
            ffmpeg_candidates(None, Some(folder)),
            [bundled.clone(), homebrew.clone(), "ffmpeg".into()]
        );
        assert_eq!(
            ffmpeg_candidates(Some("/opt/ffmpeg/bin/ffmpeg".into()), Some(folder)),
            ["/opt/ffmpeg/bin/ffmpeg".into(), bundled, homebrew.clone(), "ffmpeg".into()]
        );
        assert_eq!(
            ffmpeg_candidates(Some(String::new()), None),
            [homebrew, "ffmpeg".to_owned()]
        );
    }

    #[test]
    fn ffprobe_is_the_one_beside_ffmpeg() {
        let exe = std::env::consts::EXE_SUFFIX;
        let probe = Path::new("/pkg/bin").join(format!("ffprobe{exe}"));
        assert_eq!(
            ffprobe_beside(&format!("/pkg/bin/ffmpeg{exe}")),
            Some(probe.to_string_lossy().into_owned())
        );
        assert_eq!(ffprobe_beside("ffmpeg"), None);
        assert_eq!(ffprobe_beside("/usr/bin/avconv"), None);
    }

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
