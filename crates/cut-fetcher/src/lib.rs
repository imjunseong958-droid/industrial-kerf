//! Array-argument yt-dlp adapter used by `lcut` to import URLs.
//!
//! The adapter owns process invocation and output discovery. It never invokes
//! a shell, and it deliberately keeps cookie paths out of error messages.

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// Quality policy: video up to 1080p plus best audio.
pub const FORMAT_STRING: &str = "bv*[height<=1080]+ba/b[height<=1080]";
pub const OUTPUT_TEMPLATE: &str = "%(title)s.%(ext)s";
pub const SUB_LANGS: &str = "ko.*,en.*";

/// Errors returned by the yt-dlp adapter.
#[derive(Debug)]
pub enum FetchError {
    Missing {
        binary: PathBuf,
        detail: String,
    },
    Process {
        message: String,
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    Io(io::Error),
    InvalidInput(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { binary, detail } => write!(
                formatter,
                "{} was not found on PATH ({}). Install it with brew install yt-dlp or pipx install yt-dlp.",
                binary.display(),
                detail
            ),
            Self::Process {
                message,
                exit_code,
                stderr_tail,
            } => write!(
                formatter,
                "{message} (exit code {:?}){}",
                exit_code,
                if stderr_tail.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr_tail}")
                }
            ),
            Self::Io(error) => error.fmt(formatter),
            Self::InvalidInput(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<io::Error> for FetchError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Options for one download.
#[derive(Clone, Debug, Default)]
pub struct FetchOptions {
    pub timeout: Option<Duration>,
    pub cookies: Option<PathBuf>,
    pub extra_args: Vec<String>,
    pub yt_dlp_binary: Option<PathBuf>,
}

/// Result of one download and optional subtitle discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchResult {
    pub path: PathBuf,
    pub subs_path: Option<PathBuf>,
    pub title: Option<String>,
}

/// Resolve yt-dlp's version or return a typed missing-binary error.
pub fn check_yt_dlp(binary: impl AsRef<Path>) -> Result<String, FetchError> {
    let binary = binary.as_ref();
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|error| FetchError::Missing {
            binary: binary.to_path_buf(),
            detail: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(FetchError::Missing {
            binary: binary.to_path_buf(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Build the exact argv used for a download.
pub fn build_yt_dlp_args(url: &str, options: &FetchOptions) -> Vec<String> {
    let mut args = vec![
        "--format".into(),
        FORMAT_STRING.into(),
        "--write-auto-subs".into(),
        "--write-subs".into(),
        "--sub-langs".into(),
        SUB_LANGS.into(),
        "--convert-subs".into(),
        "vtt".into(),
        "--no-playlist".into(),
        "--newline".into(),
        "--output".into(),
        OUTPUT_TEMPLATE.into(),
    ];
    if let Some(cookies) = &options.cookies {
        args.extend(["--cookies".into(), cookies.to_string_lossy().into_owned()]);
    }
    args.extend(options.extra_args.iter().cloned());
    args.push(url.to_owned());
    args
}

/// Download into a directory and discover the newest video/subtitle files.
/// `url` may also be a search term or extractor identifier; yt-dlp decides.
pub fn fetch_video(
    url: &str,
    output_directory: &Path,
    options: &FetchOptions,
) -> Result<FetchResult, FetchError> {
    if url.trim().is_empty() {
        return Err(FetchError::InvalidInput(
            "URL or search input is required".into(),
        ));
    }
    fs::create_dir_all(output_directory)?;
    let binary = options
        .yt_dlp_binary
        .clone()
        .unwrap_or_else(|| PathBuf::from("yt-dlp"));
    check_yt_dlp(&binary)?;
    let args = build_yt_dlp_args(url, options);
    let result = run_process(&binary, &args, output_directory, options.timeout)?;
    if result.timed_out {
        return Err(FetchError::Process {
            message: "yt-dlp timed out".into(),
            exit_code: result.code,
            stderr_tail: tail(&result.stderr),
        });
    }
    if result.code != Some(0) {
        return Err(FetchError::Process {
            message: format!("yt-dlp exited with code {:?}", result.code),
            exit_code: result.code,
            stderr_tail: tail(&result.stderr),
        });
    }

    let title = parse_title(&format!("{}\n{}", result.stdout, result.stderr));
    let files = fs::read_dir(output_directory)?.collect::<Result<Vec<_>, _>>()?;
    let mut videos = Vec::new();
    for entry in files {
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if !matches!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .as_deref(),
            Some("mkv" | "mp4" | "webm")
        ) {
            continue;
        }
        let modified = entry
            .metadata()?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        videos.push((path, modified));
    }
    videos.sort_by_key(|(_, modified)| *modified);
    let path = title
        .as_ref()
        .and_then(|title| {
            videos
                .iter()
                .map(|(path, _)| path)
                .find(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(title))
                .cloned()
        })
        .or_else(|| videos.last().map(|(path, _)| path.clone()))
        .ok_or_else(|| FetchError::Process {
            message: "yt-dlp finished but no video file was found".into(),
            exit_code: result.code,
            stderr_tail: tail(&result.stderr),
        })?;
    let base = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| FetchError::InvalidInput("downloaded video name is not UTF-8".into()))?;
    let names = fs::read_dir(output_directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    let subs_path = resolve_subs_path(output_directory, base, &names);
    Ok(FetchResult {
        path,
        subs_path,
        title,
    })
}

/// Choose VTT before SRT and Korean before English/other languages.
pub fn resolve_subs_path(directory: &Path, base: &str, files: &[String]) -> Option<PathBuf> {
    [".vtt", ".srt"].into_iter().find_map(|extension| {
        let prefix = format!("{base}.");
        let mut candidates = files
            .iter()
            .filter(|file| file.starts_with(&prefix) && file.ends_with(extension))
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|file| {
            let language = file
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(extension))
                .unwrap_or_default()
                .to_ascii_lowercase();
            if language.starts_with("ko") {
                0
            } else if language.starts_with("en") {
                1
            } else {
                2
            }
        });
        candidates.first().map(|file| directory.join(file))
    })
}

#[derive(Debug)]
struct ProcessResult {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    timed_out: bool,
}

fn run_process(
    binary: &Path,
    args: &[String],
    directory: &Path,
    timeout: Option<Duration>,
) -> Result<ProcessResult, FetchError> {
    let mut child = Command::new(binary)
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| FetchError::Missing {
            binary: binary.to_path_buf(),
            detail: error.to_string(),
        })?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdout_thread = thread::spawn(move || read_all(stdout));
    let stderr_thread = thread::spawn(move || read_all(stderr));
    let deadline = timeout.map(|duration| Instant::now() + duration);
    let mut timed_out = false;
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            timed_out = true;
            child.kill()?;
            break child.wait()?.code();
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout_thread
        .join()
        .map_err(|_| FetchError::InvalidInput("yt-dlp stdout reader panicked".into()))??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| FetchError::InvalidInput("yt-dlp stderr reader panicked".into()))??;
    Ok(ProcessResult {
        stdout,
        stderr,
        code,
        timed_out,
    })
}

fn read_all<R: Read>(mut reader: R) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_title(output: &str) -> Option<String> {
    for line in output.lines().rev() {
        for marker in [
            "[Merger] Merging formats into \"",
            "[download] Destination: ",
            "[download] ",
        ] {
            if let Some(value) = line.strip_prefix(marker) {
                let value = value
                    .strip_suffix(" has already been downloaded")
                    .unwrap_or(value)
                    .trim_matches('\"')
                    .trim();
                if !value.is_empty() {
                    return Some(strip_extension(value));
                }
            }
        }
    }
    None
}

fn strip_extension(value: &str) -> String {
    Path::new(value)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(value)
        .to_owned()
}

fn tail(value: &str) -> String {
    value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_quality_and_cookie_arguments() {
        let options = FetchOptions {
            cookies: Some(PathBuf::from("cookies.txt")),
            extra_args: vec![
                "--extractor-args".into(),
                "youtube:player_client=web".into(),
            ],
            ..FetchOptions::default()
        };
        let args = build_yt_dlp_args("https://youtu.be/example", &options);
        assert_eq!(args[0..2], ["--format", FORMAT_STRING]);
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--cookies", "cookies.txt"]));
        assert_eq!(args.last().unwrap(), "https://youtu.be/example");
    }

    #[test]
    fn prefers_korean_vtt_subtitles() {
        let files = vec![
            "clip.en.vtt".into(),
            "clip.ko.srt".into(),
            "clip.ko.vtt".into(),
        ];
        assert_eq!(
            resolve_subs_path(Path::new("out"), "clip", &files),
            Some(PathBuf::from("out/clip.ko.vtt"))
        );
    }
}
