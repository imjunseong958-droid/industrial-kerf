//! Supervised sidecar processes: cancellation and stderr-driven progress.
//!
//! On Unix each sidecar runs in a process group of its own, so cancelling
//! it ends whatever it started too. That also keeps a terminal's Ctrl-C
//! from reaching it, which [`forward_termination_signals`] makes up for.
//! On Windows each runs in a job object of its own, which ends it and what
//! it started when it is cancelled, when it exits, and when the process
//! that ran it ends.
//!
//! A cancelled sidecar is asked to end before it is made to: on Unix its
//! group gets SIGTERM and [`GRACE`] to end in, and only what is still
//! there after that is killed. A program that has asked a server for
//! something, as a model's does, has that long to take it back; FFmpeg
//! ends at once on the signal. Windows has no such signal to send a job,
//! so there a cancelled sidecar is ended outright.
//!
//! A terminal's Ctrl-C ends the binary that runs sidecars, and what they
//! started with it, unless the binary holds [`stop_on_termination`]: then
//! the first one cancels that work, and so stops its sidecar as above.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{EngineError, Result};

#[cfg(windows)]
mod job;

const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long a cancelled sidecar has to end by itself once asked to, before
/// it is killed.
pub const GRACE: Duration = Duration::from_secs(5);
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

    /// Request cancellation. The active child is asked to end at its next
    /// polling point, and killed if it has not by [`GRACE`] after.
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
/// stderr line to `on_line` as it arrives. When `cancellation` is
/// requested the child is asked to end, killed if it has not within
/// [`GRACE`], and `EngineError::Cancelled` returned; what it says while it
/// ends still goes to `on_line`.
pub(crate) fn run(
    command: Command,
    cancellation: &CancellationToken,
    on_line: impl FnMut(&str),
) -> Result<Finished> {
    run_within(command, cancellation, GRACE, on_line)
}

/// [`run`], with `grace` for a cancelled child to end in.
fn run_within(
    mut command: Command,
    cancellation: &CancellationToken,
    grace: Duration,
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
            // Asked first, so that it can undo what it has under way and say so. It is
            // not reaped while it is waited for: its group stays its own to signal.
            let asked = Instant::now();
            ask_to_end(&child);
            while asked.elapsed() < grace && !has_ended(&child) {
                drain(&mut tail);
                thread::sleep(POLL_INTERVAL);
            }
            drain(&mut tail);
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

/// The job of each sidecar running now, by its process id, which cannot be
/// reused while its `Child` holds the process open.
#[cfg(windows)]
static JOBS: std::sync::Mutex<Vec<(u32, job::Job)>> = std::sync::Mutex::new(Vec::new());

#[cfg(windows)]
fn jobs() -> std::sync::MutexGuard<'static, Vec<(u32, job::Job)>> {
    JOBS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Start `command` as a sidecar: on Unix, leading a process group of its
/// own and listed as running; on Windows, in a job of its own.
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
    #[cfg(windows)]
    {
        let child = command.spawn()?;
        // A sidecar that cannot have a job still runs; cancelling it then
        // ends only the sidecar itself.
        if let Ok(job) = job::Job::assign(&child) {
            jobs().push((child.id(), job));
        }
        Ok(child)
    }
    #[cfg(not(any(unix, windows)))]
    command.spawn()
}

/// Poll the sidecar. On Unix one that has exited leaves the running list
/// under the same lock, before its process id can be reused. On Windows
/// its job closes, ending anything it left running.
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
    #[cfg(windows)]
    {
        let status = child.try_wait()?;
        if status.is_some() {
            jobs().retain(|(id, _)| *id != child.id());
        }
        Ok(status)
    }
    #[cfg(not(any(unix, windows)))]
    child.try_wait()
}

/// Ask the sidecar and what it started to end: SIGTERM to its process
/// group on Unix. Windows has nothing to ask a job with.
fn ask_to_end(child: &Child) {
    #[cfg(unix)]
    {
        let running = running();
        if running.contains(&child.id()) {
            signal_group(child.id(), libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    let _ = child;
}

/// Whether the sidecar has ended, without reaping it: it stays listed, and
/// its group stays its own, until [`kill`] or [`try_wait`] takes it. Where
/// nothing was asked of it there is nothing to wait for.
fn has_ended(child: &Child) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: `info` is zeroed, which waitid reads as no child having changed, and is
        // written only by waitid. WNOWAIT leaves the child to be reaped later.
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            let asked = libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            );
            asked != 0 || info.si_pid() != 0
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child;
        true
    }
}

/// Kill the sidecar and everything left in its process group on Unix, or
/// in its job on Windows, then reap it.
fn kill(child: &mut Child) {
    #[cfg(unix)]
    {
        let mut running = running();
        signal_group(child.id(), libc::SIGKILL);
        running.retain(|&id| id != child.id());
    }
    #[cfg(windows)]
    {
        let mut jobs = jobs();
        if let Some(index) = jobs.iter().position(|(id, _)| *id == child.id()) {
            jobs.remove(index).1.terminate();
        }
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
                            if stopped_instead(signal) {
                                continue;
                            }
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

/// The work that the first termination signal stops in place of ending this
/// process, while a [`StopOnTermination`] holds it.
#[cfg(unix)]
static STOPPABLE: std::sync::Mutex<Option<CancellationToken>> = std::sync::Mutex::new(None);

#[cfg(unix)]
fn stoppable() -> std::sync::MutexGuard<'static, Option<CancellationToken>> {
    STOPPABLE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether `signal` cancelled the work [`stop_on_termination`] named, which
/// it does once, and not SIGQUIT, which is asked for to end at once.
#[cfg(unix)]
fn stopped_instead(signal: libc::c_int) -> bool {
    if signal == libc::SIGQUIT {
        return false;
    }
    match stoppable().as_ref() {
        Some(token) if !token.is_cancelled() => {
            token.cancel();
            true
        }
        _ => false,
    }
}

/// While this is held, the first Ctrl-C, SIGTERM, or hangup cancels the
/// token it was made with in place of ending the process: the sidecar
/// running is asked to end and has [`GRACE`] to, as a cancelled one does,
/// and what it says on the way out is heard. A second ends the process as
/// [`forward_termination_signals`] does. It has an effect only in a binary
/// that called that, on Unix.
#[must_use = "the first signal stops the work only while this is held"]
pub struct StopOnTermination(());

/// Have the first termination signal cancel `token`; see [`StopOnTermination`].
pub fn stop_on_termination(token: &CancellationToken) -> StopOnTermination {
    #[cfg(unix)]
    {
        *stoppable() = Some(token.clone());
    }
    #[cfg(not(unix))]
    let _ = token;
    StopOnTermination(())
}

impl Drop for StopOnTermination {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            *stoppable() = None;
        }
    }
}
/// Read `child`'s stderr to its end on a thread of its own, keeping the
/// last of it to report.
pub(crate) fn drain_stderr(child: &mut Child) -> thread::JoinHandle<String> {
    let pipe = child.stderr.take();
    thread::spawn(move || {
        let mut tail = String::new();
        let Some(pipe) = pipe else {
            return tail;
        };
        for line in BufReader::new(pipe).split(b'\n') {
            let Ok(line) = line else {
                break;
            };
            append_bounded(&mut tail, &String::from_utf8_lossy(&line));
        }
        tail
    })
}

/// Append `line` to `tail`, keeping only the last `MAX_STDERR_TAIL_BYTES`.
pub(crate) fn append_bounded(tail: &mut String, line: &str) {
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

    /// Run a shell that says `ready`, and on SIGTERM says what `on_term` has it say and
    /// does what else it holds. Returns the run and the lines it writes.
    fn run_trapping(
        cancellation: &CancellationToken,
        on_term: &str,
        grace: Duration,
    ) -> (thread::JoinHandle<Result<Finished>>, mpsc::Receiver<String>) {
        let (sender, lines) = mpsc::channel();
        let cancellation = cancellation.clone();
        let script = format!("trap '{on_term}' TERM; echo ready >&2; while :; do sleep 0.05; done");
        let runner = thread::spawn(move || {
            let mut command = Command::new("sh");
            command.args(["-c", &script]);
            run_within(command, &cancellation, grace, |line| {
                let _ = sender.send(line.to_owned());
            })
        });
        assert_eq!(
            lines
                .recv_timeout(Duration::from_secs(5))
                .expect("the shell says it is ready"),
            "ready"
        );
        (runner, lines)
    }

    #[test]
    fn a_cancelled_sidecar_is_asked_to_end_and_what_it_says_is_heard() {
        // A program that takes its work back when asked, as a model's does a job on a server.
        let token = CancellationToken::new();
        let (runner, lines) = run_trapping(
            &token,
            "echo took the job back >&2; exit 0",
            Duration::from_secs(5),
        );
        let start = Instant::now();
        token.cancel();
        let result = runner.join().expect("runner");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        // It ended by itself, well inside the time it was given.
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
        // The shell may say that the `sleep` it waited on was ended too.
        let said: Vec<String> = lines.try_iter().collect();
        assert!(
            said.iter().any(|line| line == "took the job back"),
            "{said:?}"
        );
    }

    #[test]
    fn a_sidecar_that_does_not_end_when_asked_is_killed() {
        let token = CancellationToken::new();
        let grace = Duration::from_millis(300);
        let (runner, lines) = run_trapping(&token, "echo not yet >&2", grace);
        let start = Instant::now();
        token.cancel();
        let result = runner.join().expect("runner");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        // It was given the whole of its time, and no more than a moment past it.
        assert!(start.elapsed() >= grace, "{:?}", start.elapsed());
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
        // The shell may say that the `sleep` it waited on was ended too.
        let said: Vec<String> = lines.try_iter().collect();
        assert!(said.iter().any(|line| line == "not yet"), "{said:?}");
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

    #[test]
    fn the_first_signal_stops_the_work_held_and_a_second_ends_the_process() {
        let token = CancellationToken::new();
        {
            let _held = stop_on_termination(&token);
            assert!(!stopped_instead(libc::SIGQUIT));
            assert!(!token.is_cancelled());
            assert!(stopped_instead(libc::SIGINT));
            assert!(token.is_cancelled());
            // Already stopping: this one ends the process.
            assert!(!stopped_instead(libc::SIGINT));
        }
        // Nothing held: every signal ends it.
        assert!(!stopped_instead(libc::SIGTERM));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    /// Run PowerShell, which starts `ping` waiting half a minute, reports
    /// its own process id and ping's on stderr, and waits. Returns the run
    /// and the two ids once reported.
    fn run_pinger(
        cancellation: &CancellationToken,
    ) -> (thread::JoinHandle<Result<Finished>>, u32, u32) {
        let (sender, lines) = mpsc::channel();
        let cancellation = cancellation.clone();
        let runner = thread::spawn(move || {
            let mut command = Command::new("powershell");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$p = Start-Process ping -ArgumentList '-n','30','127.0.0.1' -NoNewWindow -PassThru; \
                 [Console]::Error.WriteLine(\"$PID $($p.Id)\"); $p.WaitForExit()",
            ]);
            run(command, &cancellation, |line| {
                let _ = sender.send(line.to_owned());
            })
        });
        let line = lines
            .recv_timeout(Duration::from_secs(30))
            .expect("PowerShell reports its process ids");
        let (shell, pinger) = line.trim().split_once(' ').expect("two process ids");
        (
            runner,
            shell.parse().expect("shell id"),
            pinger.parse().expect("ping id"),
        )
    }

    /// Whether process `pid` ends within `limit`.
    fn ends_within(pid: u32, limit: Duration) -> bool {
        // SAFETY: opens the process only to wait on it, and closes it.
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return true;
            }
            let ended = WaitForSingleObject(process, limit.as_millis() as u32) == WAIT_OBJECT_0;
            CloseHandle(process);
            ended
        }
    }

    #[test]
    fn cancelling_ends_what_the_sidecar_started() {
        let token = CancellationToken::new();
        let (runner, shell, pinger) = run_pinger(&token);
        assert!(jobs().iter().any(|(id, _)| *id == shell));
        let start = Instant::now();
        token.cancel();
        let result = runner.join().expect("runner");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(ends_within(pinger, Duration::from_secs(3)));
        assert!(!jobs().iter().any(|(id, _)| *id == shell));
    }

    #[test]
    fn a_sidecar_ends_when_its_job_closes() {
        // What closing kerfd's handles does to the sidecars it ran.
        let mut child = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .expect("ping starts");
        let job = job::Job::assign(&child).expect("a job");
        drop(job);
        assert!(ends_within(child.id(), Duration::from_secs(3)));
        let _ = child.wait();
    }
}
