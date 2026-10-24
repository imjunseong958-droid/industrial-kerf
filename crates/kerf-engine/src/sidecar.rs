//! Supervised sidecar processes: cancellation and stderr-driven progress.
//!
//! On Unix each sidecar runs in a process group of its own, so cancelling
//! it ends whatever it started too. That also keeps a terminal's Ctrl-C
//! from reaching it, which [`forward_termination_signals`] makes up for.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use crate::error::{EngineError, Result};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const STDERR_QUEUE_CAPACITY: usize = 128;
const MAX_STDERR_TAIL_BYTES: usize = 16 * 1024;

/// Cancellation handle shared with a running sidecar.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a non-cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. The active child is killed at its next
    /// polling point.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Return whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// How a sidecar exited, with the end of what it wrote to stderr.
pub(crate) struct Finished {
    pub status: ExitStatus,
    pub stderr: String,
}

/// Run `command` to completion with stdin and stdout closed, passing each
/// stderr line to `on_line` as it arrives. The child is killed and
/// `EngineError::Cancelled` returned when `cancellation` is requested.
pub(crate) fn run(
    mut command: Command,
    cancellation: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> Result<Finished> {
    if cancellation.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = start(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| EngineError::msg(format!("{program} could not start: {error}")))?;
    let Some(stderr) = child.stderr.take() else {
        kill(&mut child);
        return Err(EngineError::msg(format!("{program} stderr is unavailable")));
    };
    let (sender, receiver) = mpsc::sync_channel::<String>(STDERR_QUEUE_CAPACITY);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr)
            .lines()
            .map_while(std::io::Result::ok)
        {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let mut tail = String::new();
    let mut drain = |tail: &mut String| {
        for line in receiver.try_iter() {
            append_bounded(tail, &line);
            on_line(&line);
        }
    };
    let outcome = loop {
        drain(&mut tail);
        if cancellation.is_cancelled() {
            kill(&mut child);
            break Err(EngineError::Cancelled);
        }
        match try_wait(&mut child) {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                kill(&mut child);
                break Err(error.into());
            }
        }
    };
    // The reader ends once the child's stderr closes; keep draining so a
    // full queue cannot block it. Something the child started outside its
    // process group can hold stderr open long after it, so a run that
    // killed it does not wait: the reader stops on its own once stderr
    // closes or the queue is dropped.
    if outcome.is_ok() {
        while !reader.is_finished() {
            drain(&mut tail);
            thread::sleep(POLL_INTERVAL);
        }
        let _ = reader.join();
        drain(&mut tail);
    }
    outcome.map(|status| Finished {
        status,
        stderr: tail,
    })
}

/// The process groups of the sidecars running now, each named by its
/// leader, which stays unreaped while it is listed.
#[cfg(unix)]
static RUNNING: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

#[cfg(unix)]
fn running() -> std::sync::MutexGuard<'static, Vec<u32>> {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Start `command` as a sidecar: on Unix, leading a process group of its
/// own and listed as running.
fn start(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Listed under the lock, so a forwarded signal cannot miss it.
        let mut running = running();
        let child = command.process_group(0).spawn()?;
        running.push(child.id());
        Ok(child)
    }
    #[cfg(not(unix))]
    command.spawn()
}

/// Poll the sidecar. On Unix one that has exited leaves the running list
/// under the same lock, before its process id can be reused.
fn try_wait(child: &mut Child) -> std::io::Result<Option<ExitStatus>> {
    #[cfg(unix)]
    {
        let mut running = running();
        let status = child.try_wait()?;
        if status.is_some() {
            running.retain(|&id| id != child.id());
        }
        Ok(status)
    }
    #[cfg(not(unix))]
    child.try_wait()
}

/// Kill the sidecar and, on Unix, everything left in its process group,
/// then reap it.
fn kill(child: &mut Child) {
    #[cfg(unix)]
    {
        let mut running = running();
        signal_group(child.id(), libc::SIGKILL);
        running.retain(|&id| id != child.id());
    }
    #[cfg(not(unix))]
    let _ = child.kill();
    let _ = child.wait();
}

/// Send `signal` to the process group `leader` leads. Callers hold the
/// running lock and have not reaped `leader`, so the group is a sidecar's.
#[cfg(unix)]
fn signal_group(leader: u32, signal: libc::c_int) {
    // SAFETY: killpg takes no pointers and has no memory effects here.
    unsafe {
        libc::killpg(leader as libc::pid_t, signal);
    }
}

/// Pass the signals that end a process from a terminal or a supervisor
/// (SIGINT, SIGTERM, SIGHUP, and SIGQUIT) on to the running sidecars, then
/// let each end this process as it would have. Sidecars lead process
/// groups of their own, out of reach of a terminal's Ctrl-C, so a binary
/// that runs them calls this first; a library leaves its host's signals
/// alone. Does nothing elsewhere than Unix, where console Ctrl-C reaches
/// every process attached to the console.
pub fn forward_termination_signals() {
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
        let forwarding = signal_hook::iterator::Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM])
            .and_then(|mut signals| {
                thread::Builder::new()
                    .name("kerf-sidecar-signals".into())
                    .spawn(move || {
                        for signal in signals.forever() {
                            for &leader in running().iter() {
                                signal_group(leader, signal);
                            }
                            let _ = signal_hook::low_level::emulate_default_handler(signal);
                        }
                    })
            });
        if let Err(error) = forwarding {
            eprintln!("sidecars will not receive Ctrl-C or termination signals: {error}");
        }
    }
}

/// Append `line` to `tail`, keeping only the last `MAX_STDERR_TAIL_BYTES`.
fn append_bounded(tail: &mut String, line: &str) {
    tail.push_str(line);
    tail.push('\n');
    if tail.len() > MAX_STDERR_TAIL_BYTES {
        let mut start = tail.len() - MAX_STDERR_TAIL_BYTES;
        while !tail.is_char_boundary(start) {
            start += 1;
        }
        tail.drain(..start);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Run a shell that starts `sleep` in the background, reports its own
    /// process id and the sleeper's on stderr, and waits. Returns the run
    /// and the two ids once reported.
    fn run_sleeper(
        cancellation: &CancellationToken,
    ) -> (thread::JoinHandle<Result<Finished>>, u32, String) {
        let (sender, lines) = mpsc::channel();
        let cancellation = cancellation.clone();
        let runner = thread::spawn(move || {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30 & echo \"$$ $!\" >&2; wait"]);
            run(command, &cancellation, |line| {
                let _ = sender.send(line.to_owned());
            })
        });
        let line = lines
            .recv_timeout(Duration::from_secs(5))
            .expect("the shell reports its process ids");
        let (shell, sleeper) = line.split_once(' ').expect("two process ids");
        (runner, shell.parse().expect("shell id"), sleeper.to_owned())
    }

    /// Whether process `pid` ends within `limit`; a zombie awaiting its new
    /// parent counts as ended.
    fn ends_within(pid: &str, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .expect("ps runs");
            let state = String::from_utf8_lossy(&output.stdout);
            if state.trim().is_empty() || state.trim_start().starts_with('Z') {
                return true;
            }
            thread::sleep(POLL_INTERVAL);
        }
        false
    }

    #[test]
    fn cancelling_ends_what_the_sidecar_started() {
        let token = CancellationToken::new();
        let (runner, _, sleeper) = run_sleeper(&token);
        let start = Instant::now();
        token.cancel();
        let result = runner.join().expect("runner");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(ends_within(&sleeper, Duration::from_secs(3)));
    }

    #[test]
    fn a_signal_to_a_running_sidecar_reaches_what_it_started() {
        let token = CancellationToken::new();
        let (runner, shell, sleeper) = run_sleeper(&token);
        {
            let running = running();
            assert!(running.contains(&shell));
            signal_group(shell, libc::SIGTERM);
        }
        let finished = runner
            .join()
            .expect("runner")
            .expect("the shell ends on the signal");
        assert!(!finished.status.success());
        assert!(ends_within(&sleeper, Duration::from_secs(3)));
        assert!(!running().contains(&shell));
    }
}
