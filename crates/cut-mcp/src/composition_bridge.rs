//! Version 1 local worker protocol. The administrator-configured executable is
//! a trusted renderer boundary, not an HTML/browser runtime embedded in Cut.

use crate::composition_files::{self as files, Directory};
use crate::McpError;
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) const BRIDGE_VERSION: u64 = 1;
pub(super) const MAX_REQUEST_BYTES: usize = 512 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_STDERR_BYTES: usize = 32 * 1024;

pub(super) struct Renderer {
    pub executable: PathBuf,
    pub version: String,
    pub fingerprint: String,
    pub timeout: Duration,
}

impl Renderer {
    pub fn from_env() -> Result<Self, McpError> {
        let executable = std::env::var_os("CUT_COMPOSITION_RENDERER")
            .map(PathBuf::from)
            .ok_or_else(unavailable)?;
        let version =
            std::env::var("CUT_COMPOSITION_RENDERER_VERSION").map_err(|_| unavailable())?;
        Self::new(executable, version, Duration::from_secs(120))
    }

    pub fn new(executable: PathBuf, version: String, timeout: Duration) -> Result<Self, McpError> {
        if !executable.is_absolute()
            || version.trim().is_empty()
            || version.len() > 128
            || version.chars().any(char::is_control)
        {
            return Err(unavailable());
        }
        // Resolving a configured executable is administrator configuration, not
        // an untrusted composition/media path. No PATH lookup or shell is used.
        let executable = executable.canonicalize().map_err(|_| unavailable())?;
        let mut file = File::open(&executable).map_err(|_| unavailable())?;
        if !file.metadata().map_err(|_| unavailable())?.is_file() {
            return Err(unavailable());
        }
        let (fingerprint, _) = files::fingerprint(&mut file, None)?;
        Ok(Self {
            executable,
            version,
            fingerprint,
            timeout,
        })
    }
}

fn unavailable() -> McpError {
    McpError::new("COMPOSITION_RENDERER_UNAVAILABLE",
        "configure CUT_COMPOSITION_RENDERER as an absolute executable path and CUT_COMPOSITION_RENDERER_VERSION as its pinned runtime version; Cut composition bridge v1 is required")
}

pub(super) fn interrupted(
    deadline: Instant,
    cancelled: Option<&AtomicBool>,
) -> Result<(), McpError> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(McpError::new(
            "CANCELLED",
            "composition rendering cancelled",
        ));
    }
    if Instant::now() >= deadline {
        return Err(McpError::new(
            "COMPOSITION_TIMEOUT",
            "composition rendering timed out",
        ));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn invoke(
    renderer: &Renderer,
    work: &Directory,
    request: &Value,
    deadline: Instant,
    cancelled: Option<&AtomicBool>,
) -> Result<Value, McpError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    let bytes = serde_json::to_vec(request)
        .map_err(|error| McpError::new("COMPOSITION_BRIDGE_PROTOCOL", error.to_string()))?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(McpError::new(
            "COMPOSITION_LIMIT",
            "bridge request exceeds 512 KiB",
        ));
    }
    interrupted(deadline, cancelled)?;
    let mut executable = File::open(&renderer.executable).map_err(|_| unavailable())?;
    if files::fingerprint(&mut executable, None)?.0 != renderer.fingerprint {
        return Err(McpError::new(
            "COMPOSITION_RENDERER_CHANGED",
            "renderer executable changed during resolution",
        ));
    }
    let mut command = Command::new(&renderer.executable);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let work_fd = work.file.as_raw_fd();
    // SAFETY: only async-signal-safe fchdir is called after fork. The descriptor
    // remains live until spawn returns; the child never uses a mutable path.
    unsafe {
        command.pre_exec(move || {
            if libc::fchdir(work_fd) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|error| {
        McpError::new(
            "COMPOSITION_RENDERER_UNAVAILABLE",
            format!("cannot launch configured renderer: {error}"),
        )
    })?;
    let mut worker = Worker(child);
    let mut stdin = worker.0.stdin.take().expect("piped stdin");
    let mut stdout = worker.0.stdout.take().expect("piped stdout");
    let mut stderr = worker.0.stderr.take().expect("piped stderr");
    for fd in [stdin.as_raw_fd(), stdout.as_raw_fd(), stderr.as_raw_fd()] {
        // SAFETY: owned live pipe descriptors; preserve all existing flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(files::io(std::io::Error::last_os_error()));
        }
    }
    let mut offset = 0;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut status = None;
    // Pump all three nonblocking pipes in one bounded loop. A worker that never
    // reads stdin or leaves descendant pipes open cannot outlive the deadline.
    loop {
        interrupted(deadline, cancelled)?;
        match stdin.write(&bytes[offset..]) {
            Ok(0) if offset != bytes.len() => {
                return Err(McpError::new(
                    "COMPOSITION_BRIDGE_PROTOCOL",
                    "renderer closed stdin early",
                ))
            }
            Ok(count) => offset += count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                return Err(McpError::new(
                    "COMPOSITION_BRIDGE_PROTOCOL",
                    format!("renderer stdin: {error}"),
                ))
            }
        }
        if offset == bytes.len() {
            // Send EOF without retaining a second pipe handle.
            break;
        }
        stdout_eof |= drain(&mut stdout, &mut out, MAX_RESPONSE_BYTES)?;
        stderr_eof |= drain(&mut stderr, &mut err, MAX_STDERR_BYTES)?;
        if worker.0.try_wait().map_err(files::io)?.is_some() {
            return Err(McpError::new(
                "COMPOSITION_BRIDGE_PROTOCOL",
                "renderer exited before receiving the request",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(stdin);
    loop {
        interrupted(deadline, cancelled)?;
        stdout_eof |= drain(&mut stdout, &mut out, MAX_RESPONSE_BYTES)?;
        stderr_eof |= drain(&mut stderr, &mut err, MAX_STDERR_BYTES)?;
        if status.is_none() {
            status = worker.0.try_wait().map_err(files::io)?;
        }
        if status.is_some() && stdout_eof && stderr_eof {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !status.expect("exited worker").success() {
        return Err(McpError::new(
            "COMPOSITION_RENDERER_FAILED",
            "renderer exited unsuccessfully",
        )
        .with_details(serde_json::json!({"stderr":String::from_utf8_lossy(&err)})));
    }
    let response: Value = serde_json::from_slice(&out).map_err(|error| {
        McpError::new(
            "COMPOSITION_BRIDGE_PROTOCOL",
            format!("expected one JSON response: {error}"),
        )
    })?;
    if response.get("bridgeVersion") != request.get("bridgeVersion")
        || response.get("rendererVersion") != Some(&Value::String(renderer.version.clone()))
        || response.get("cacheKey") != request.get("cacheKey")
        || response.get("inputFingerprint") != request.get("inputFingerprint")
        || response.get("artifact") != request.get("artifact")
        || response.get("ok") != Some(&Value::Bool(true))
    {
        return Err(McpError::new("COMPOSITION_BRIDGE_PROTOCOL", "renderer must acknowledge bridge/version, cache key, effective input fingerprint and exact artifact request"));
    }
    Ok(response)
}

#[cfg(unix)]
fn drain(reader: &mut impl Read, bytes: &mut Vec<u8>, limit: usize) -> Result<bool, McpError> {
    let mut buffer = [0_u8; 4096];
    // A finite read quota lets timeout/cancellation run even during output floods.
    for _ in 0..16 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if bytes.len() + count > limit {
                    return Err(McpError::new(
                        "COMPOSITION_BRIDGE_LIMIT",
                        "renderer stdout/stderr exceeded its byte limit",
                    ));
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
            Err(error) => return Err(files::io(error)),
        }
    }
    Ok(false)
}

#[cfg(unix)]
struct Worker(Child);

#[cfg(unix)]
impl Drop for Worker {
    fn drop(&mut self) {
        // The worker owns a process group. Kill remaining descendants on success
        // too, so they cannot mutate a supposedly completed artifact later.
        unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL) };
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(not(unix))]
pub(super) fn invoke(
    _renderer: &Renderer,
    _work: &Directory,
    _request: &Value,
    _deadline: Instant,
    _cancelled: Option<&AtomicBool>,
) -> Result<Value, McpError> {
    Err(McpError::new(
        "COMPOSITION_RENDERER_UNAVAILABLE",
        "composition bridge requires Unix process control",
    ))
}
