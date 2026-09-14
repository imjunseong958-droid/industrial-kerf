//! Descriptor-relative I/O for composition catalog, snapshots and derivatives.
//! All untrusted paths are project-relative; cache writes never follow links.

use crate::composition::{open_directory, open_local, relative_path};
use crate::McpError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
pub(super) const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_INPUT_BYTES: u64 = 1024 * 1024 * 1024;

pub(super) fn io(error: std::io::Error) -> McpError {
    McpError::new("COMPOSITION_IO", error.to_string())
}

pub(super) fn path(value: &str) -> Result<PathBuf, McpError> {
    relative_path(Path::new(""), value, false)
        .map_err(|message| McpError::new("INVALID_ARGUMENTS", message))
}

pub(super) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Hash/copy the same open regular file with hard byte limits. The copied bytes
/// must match the fingerprint established during input resolution.
pub(super) fn fingerprint(
    file: &mut File,
    mut output: Option<&mut File>,
) -> Result<(String, u64), McpError> {
    if !file.metadata().map_err(io)?.is_file() {
        return Err(McpError::new(
            "COMPOSITION_UNSAFE_FILE",
            "expected a regular file",
        ));
    }
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > MAX_FILE_BYTES {
            return Err(McpError::new(
                "COMPOSITION_LIMIT",
                "input/artifact exceeds 512 MiB",
            ));
        }
        digest.update(&buffer[..count]);
        if let Some(writer) = output.as_deref_mut() {
            writer.write_all(&buffer[..count]).map_err(io)?;
        }
    }
    Ok((format!("{:x}", digest.finalize()), size))
}

pub(super) struct Directory {
    pub file: File,
    pub path: PathBuf,
}

impl Directory {
    pub fn project(path: &Path) -> Result<Self, McpError> {
        let file = File::open(path).map_err(io)?;
        if !file.metadata().map_err(io)?.is_dir() {
            return Err(McpError::new(
                "COMPOSITION_IO",
                "project root must be a directory",
            ));
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn open(&self, relative: &Path) -> Result<File, McpError> {
        open_local(&self.file, relative).map_err(io)
    }

    pub fn directory(&self, relative: &Path) -> Result<Self, McpError> {
        Ok(Self {
            file: open_directory(&self.file, relative).map_err(io)?,
            path: self.path.join(relative),
        })
    }

    pub fn child(&self, name: &str) -> Result<Self, McpError> {
        let relative = path(name)?;
        if relative.components().count() != 1 {
            return Err(McpError::new(
                "COMPOSITION_IO",
                "expected a single directory name",
            ));
        }
        mkdir(&self.file, name)?;
        self.directory(&relative)
    }

    pub fn create_file(&self, relative: &Path) -> Result<File, McpError> {
        let parent = relative.parent().unwrap_or(Path::new(""));
        let mut directory = self.directory(Path::new(""))?;
        for component in parent.components() {
            let name = component
                .as_os_str()
                .to_str()
                .ok_or_else(|| McpError::new("COMPOSITION_IO", "non-UTF-8 path"))?;
            directory = directory.child(name)?;
        }
        let name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| McpError::new("COMPOSITION_IO", "file name required"))?;
        writable(&directory.file, name, true)
    }

    pub fn lock(&self, name: &str, deadline: Instant) -> Result<File, McpError> {
        let file = writable(&self.file, name, false)?;
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(McpError::new(
                            "COMPOSITION_TIMEOUT",
                            "cache/provenance lock timed out",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(io(error)),
            }
        }
    }

    pub fn atomic_write(&self, name: &str, bytes: &[u8]) -> Result<(), McpError> {
        let temporary = temporary_name("write");
        let mut file = self.create_file(Path::new(&temporary))?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
        self.rename(&temporary, name)
    }

    #[cfg(unix)]
    pub fn rename(&self, from: &str, to: &str) -> Result<(), McpError> {
        use std::os::fd::AsRawFd;
        let from = name(from)?;
        let to = name(to)?;
        // SAFETY: names are single components and descriptors are live.
        if unsafe {
            libc::renameat(
                self.file.as_raw_fd(),
                from.as_ptr(),
                self.file.as_raw_fd(),
                to.as_ptr(),
            )
        } != 0
        {
            return Err(io(std::io::Error::last_os_error()));
        }
        self.file.sync_all().map_err(io)
    }

    #[cfg(not(unix))]
    pub fn rename(&self, _from: &str, _to: &str) -> Result<(), McpError> {
        Err(unsupported())
    }
}

pub(super) fn temporary_name(prefix: &str) -> String {
    format!(
        ".{prefix}-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(unix)]
fn name(value: &str) -> Result<std::ffi::CString, McpError> {
    let relative = path(value)?;
    if relative.components().count() != 1 || relative.to_str() != Some(value) {
        return Err(McpError::new(
            "COMPOSITION_IO",
            "expected a single normal file name",
        ));
    }
    std::ffi::CString::new(value).map_err(|_| McpError::new("COMPOSITION_IO", "invalid name"))
}

#[cfg(unix)]
fn mkdir(root: &File, value: &str) -> Result<(), McpError> {
    use std::os::fd::AsRawFd;
    let value = name(value)?;
    // SAFETY: a live parent descriptor and a single-component C string.
    if unsafe { libc::mkdirat(root.as_raw_fd(), value.as_ptr(), 0o700) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(io(error));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn writable(root: &File, value: &str, exclusive: bool) -> Result<File, McpError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let value = name(value)?;
    let flags = libc::O_RDWR
        | libc::O_CREAT
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | libc::O_CLOEXEC
        | if exclusive { libc::O_EXCL } else { 0 };
    // SAFETY: flags request a new owned descriptor; symlinks are forbidden.
    let fd = unsafe { libc::openat(root.as_raw_fd(), value.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    // SAFETY: openat returned an owned descriptor, transferred exactly once.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata().map_err(io)?.is_file() {
        return Err(McpError::new(
            "COMPOSITION_UNSAFE_FILE",
            "cache file is not regular",
        ));
    }
    Ok(file)
}

/// Read directory names through the already validated descriptor. No pathname
/// traversal occurs during discovery, including if an ancestor is replaced.
#[cfg(unix)]
pub(super) fn names(root: &File, limit: usize) -> Result<Vec<String>, McpError> {
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;
    // SAFETY: open a separate directory description so scans don't share offsets.
    let fd = unsafe {
        libc::openat(
            root.as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    // SAFETY: fdopendir takes ownership on success.
    let pointer = unsafe { libc::fdopendir(fd) };
    if pointer.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(io(error));
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            // SAFETY: this stream uniquely owns a successful fdopendir result.
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = Stream(pointer);
    let mut result = Vec::new();
    loop {
        // errno is reset so EOF cannot hide an I/O error.
        set_errno_zero();
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(0) {
                return Err(io(error));
            }
            break;
        }
        // SAFETY: d_name is NUL-terminated and valid until the next readdir.
        let value = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
            .to_str()
            .map_err(|_| McpError::new("COMPOSITION_UNSAFE_FILE", "catalog names must be UTF-8"))?;
        if matches!(value, "." | "..") {
            continue;
        }
        if result.len() == limit {
            return Err(McpError::new(
                "COMPOSITION_LIMIT",
                "catalog exceeds entry count limit",
            ));
        }
        result.push(value.to_owned());
    }
    result.sort();
    Ok(result)
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
fn set_errno_zero() {
    // SAFETY: errno is a thread-local integer owned by libc.
    unsafe { *libc::__error() = 0 };
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))
))]
fn set_errno_zero() {
    // SAFETY: errno is a thread-local integer owned by libc.
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(not(unix))]
fn unsupported() -> McpError {
    McpError::new(
        "COMPOSITION_UNAVAILABLE",
        "composition storage requires Unix",
    )
}

#[cfg(not(unix))]
fn mkdir(_root: &File, _name: &str) -> Result<(), McpError> {
    Err(unsupported())
}
#[cfg(not(unix))]
fn writable(_root: &File, _name: &str, _exclusive: bool) -> Result<File, McpError> {
    Err(unsupported())
}
#[cfg(not(unix))]
pub(super) fn names(_root: &File, _limit: usize) -> Result<Vec<String>, McpError> {
    Err(unsupported())
}
