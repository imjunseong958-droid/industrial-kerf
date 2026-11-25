//! Where the `log` records of kerfd, kerf-mcp and kerf go. With no logger installed the `log`
//! facade drops them all, so each of these programs installs one at its start, as the editor does.
//!
//! What is written is warnings and errors, unless `RUST_LOG` says otherwise (`RUST_LOG=debug`,
//! `RUST_LOG=kerf_runtime=info`). Only the first install in a process takes effect.

use env_logger::{Target, WriteStyle};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// What is written when `RUST_LOG` is not set.
const DEFAULT_FILTER: &str = "warn";

/// kerfd's log, in its project folder, out of Git with its other `.kerf-runtime.*` files.
const PROJECT_LOG: &str = ".kerf-runtime.log";

/// Where the log is set aside once it reaches [`SET_ASIDE_AT`], replacing the one set aside before.
const SET_ASIDE_LOG: &str = ".kerf-runtime.log.old";

/// How long the log grows before it is set aside, so a project keeps its newest one to two MiB of
/// lines rather than every line kerfd ever wrote.
const SET_ASIDE_AT: u64 = 1024 * 1024;

/// Log to stderr: kerf's, which a person reads, and kerf-mcp's, which the MCP host keeps. Never
/// stdout, which kerf-mcp speaks MCP on.
pub fn to_stderr() {
    let _ = builder().target(Target::Stderr).try_init();
}

/// Log to `.kerf-runtime.log` in the project folder `directory`: kerfd's log.
///
/// Not stderr: a client that starts kerfd sends its stderr to a file it removes once kerfd is up,
/// and reads that file, when kerfd stops before then, as the JSON error kerfd wrote last.
///
/// The file is made at the first line written, so a project kerfd has nothing to say about gets
/// none. A line that cannot be written there is dropped.
pub fn to_project(directory: &Path) {
    let log = ProjectLog {
        path: directory.join(PROJECT_LOG),
        set_aside: directory.join(SET_ASIDE_LOG),
        file: None,
        length: 0,
    };
    let _ = builder()
        .target(Target::Pipe(Box::new(log)))
        // Never colour escapes in a file, whatever RUST_LOG_STYLE says.
        .write_style(WriteStyle::Never)
        .try_init();
}

fn builder() -> env_logger::Builder {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(DEFAULT_FILTER))
}

/// kerfd's log file, appended to by each kerfd the project has had.
struct ProjectLog {
    path: PathBuf,
    set_aside: PathBuf,
    file: Option<File>,
    /// How long the file is, as of the last write.
    length: u64,
}

impl ProjectLog {
    fn file(&mut self) -> io::Result<&mut File> {
        let file = match self.file.take() {
            Some(file) if self.length < SET_ASIDE_AT => file,
            full => {
                // Closed before it is set aside.
                drop(full);
                if fs::metadata(&self.path).is_ok_and(|log| log.len() >= SET_ASIDE_AT) {
                    // Should this fail, the log grows on, and is set aside at a later line.
                    let _ = fs::rename(&self.path, &self.set_aside);
                }
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?;
                self.length = file.metadata()?.len();
                file
            }
        };
        Ok(self.file.insert(file))
    }
}

impl Write for ProjectLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.file()?.write(bytes)?;
        self.length += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), File::flush)
    }
}
