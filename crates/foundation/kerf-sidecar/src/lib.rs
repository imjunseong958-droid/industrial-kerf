//! Programs run beside Kerf as sidecars, under supervision: FFmpeg, whisper, a model's
//! adapter, Claude Code, the OpenTimelineIO bridge.
//!
//! On Unix each sidecar runs in a process group of its own, so stopping it ends whatever it
//! started too. That also keeps a terminal's Ctrl-C from reaching it, which
//! [`forward_termination_signals`] makes up for. On Windows each runs in a job object of its
//! own, which ends it and what it started when it is stopped, when it exits, and when the
//! process that ran it ends.
//!
//! A stopped sidecar is asked to end before it is made to ([`Stopper::stop`]): on Unix its
//! group gets SIGTERM and a grace, [`GRACE`], to end in, and only what is still there after
//! that is killed. A program that has asked a server for something, as a model's does, has
//! that long to take it back; FFmpeg ends at once on the signal. Windows has no such signal
//! to send a job, so there a stopped sidecar is ended outright.
//!
//! A sidecar that ends by itself takes what it left running with it: on Unix what is still in
//! its group is killed as it is waited for, and on Windows its job closes.
//!
//! The sidecars running now are listed once for the whole process, here, so that a forwarded
//! signal reaches each of them, whichever crate started it. That is why this is a crate of
//! `foundation`: a crate of any layer runs its sidecars through it, and none keeps a list of
//! its own.
//!
//! A terminal's Ctrl-C ends the binary that runs sidecars, and what they started with it,
//! unless the binary holds [`stop_on_termination`]: then the first one cancels that work, and
//! so stops its sidecar as above.

use std::io::{self, Read};
use std::process::{
    Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod job;

const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long a stopped sidecar has to end by itself once asked to, before it is killed.
pub const GRACE: Duration = Duration::from_secs(5);

/// Cancellation handle shared with work that runs a sidecar.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a non-cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. The work's sidecar is asked to end at the
    /// work's next polling point, and killed if it has not by [`GRACE`] after.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Return whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// A sidecar as it is listed while it runs: its process id, and the number
/// it was started as, which no other sidecar has, so that a process id
/// reused once it has ended is not taken for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Listed {
    id: u32,
    serial: u64,
    /// No cancellation reaches it, so the first termination signal is passed on to it even
    /// while [`stop_on_termination`] holds.
    uncancellable: bool,
}

impl Listed {
    fn of(child: &Child, uncancellable: bool) -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        Self {
            id: child.id(),
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            uncancellable,
        }
    }
}

/// The process groups of the sidecars running now, each named by its
/// leader, which stays unreaped while it is listed.
#[cfg(unix)]
static RUNNING: std::sync::Mutex<Vec<Listed>> = std::sync::Mutex::new(Vec::new());

#[cfg(unix)]
fn running() -> std::sync::MutexGuard<'static, Vec<Listed>> {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What holds each sidecar running now (its job, or its process where it
/// could have no job), by its process id, which cannot be reused while its
/// `Child` holds the process open.
#[cfg(windows)]
static JOBS: std::sync::Mutex<Vec<(Listed, job::Hold)>> = std::sync::Mutex::new(Vec::new());

#[cfg(windows)]
fn jobs() -> std::sync::MutexGuard<'static, Vec<(Listed, job::Hold)>> {
    JOBS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Start `command` as a sidecar: on Unix, leading a process group of its
/// own and listed as running; on Windows, in a job of its own.
#[expect(
    clippy::disallowed_methods,
    reason = "this is the supervised spawn the lint points everyone else to"
)]
fn start(command: &mut Command, uncancellable: bool) -> io::Result<(Child, Listed)> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Listed under the lock, so a forwarded signal cannot miss it.
        let mut running = running();
        let child = command.process_group(0).spawn()?;
        let listed = Listed::of(&child, uncancellable);
        running.push(listed);
        Ok((child, listed))
    }
    #[cfg(windows)]
    {
        let child = command.spawn()?;
        let listed = Listed::of(&child, uncancellable);
        // A sidecar that cannot have a job still runs; stopping it then
        // ends only the sidecar itself.
        if let Ok(hold) = job::Hold::of(&child) {
            jobs().push((listed, hold));
        }
        Ok((child, listed))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let child = command.spawn()?;
        let listed = Listed::of(&child, uncancellable);
        Ok((child, listed))
    }
}

/// A program run as a sidecar: on Unix it leads a process group of its
/// own, on Windows it is in a job of its own. Its pipes are taken as a
/// [`Child`]'s are.
///
/// It is waited for here and not through its `Child`, so that on Unix it
/// stays its group's leader, unreaped, for as long as anything may signal
/// that group. Waiting for it ends what it left running. Dropped before it
/// has been waited for, it is killed, and what it started with it.
pub struct Sidecar {
    child: Child,
    listed: Listed,
    /// Its stdin, when the command piped it.
    pub stdin: Option<ChildStdin>,
    /// Its stdout, when the command piped it.
    pub stdout: Option<ChildStdout>,
    /// Its stderr, when the command piped it.
    pub stderr: Option<ChildStderr>,
    /// Reaped, or about to be: no longer listed, and nothing more to stop.
    done: bool,
}

impl Sidecar {
    /// Start `command` as a sidecar. Give it a stdin of its own, null or a pipe: in a group of
    /// its own, a program that reads or sets up the terminal it inherited (FFmpeg does) is
    /// stopped there, and is never waited out.
    pub fn start(command: &mut Command) -> io::Result<Self> {
        Self::started(command, false)
    }

    /// Start `command` as a sidecar that no cancellation reaches: nothing it is run for can
    /// stop it. While [`stop_on_termination`] holds, the first termination signal cancels that
    /// work in place of ending this process, and is passed on to this sidecar all the same, as a
    /// terminal's Ctrl-C reaches a plain child in the terminal's own process group.
    pub fn start_uncancellable(command: &mut Command) -> io::Result<Self> {
        Self::started(command, true)
    }

    fn started(command: &mut Command, uncancellable: bool) -> io::Result<Self> {
        let (mut child, listed) = start(command, uncancellable)?;
        Ok(Self {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            child,
            listed,
            done: false,
        })
    }

    /// What stops it from another thread while this one waits for it.
    pub fn stopper(&self) -> Stopper {
        Stopper {
            listed: self.listed,
        }
    }

    /// How it exited, if it has; then, as [`Sidecar::wait`] does, what it
    /// left running is ended.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.done {
            return self.child.try_wait();
        }
        #[cfg(unix)]
        let exited = exited(self.child.id());
        #[cfg(not(unix))]
        let exited = self.child.try_wait()?.is_some();
        if !exited {
            return Ok(None);
        }
        self.end().map(Some)
    }

    /// Wait for it to exit, then end what it left running: on Unix what is
    /// still in its group is killed, and on Windows its job closes. Its
    /// [`Stopper`] stops it from another thread meanwhile.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if self.done {
            return self.child.wait();
        }
        #[cfg(unix)]
        wait_exited(self.child.id())?;
        #[cfg(not(unix))]
        self.child.wait()?;
        self.end()
    }

    /// End what the sidecar, which has exited, left running, and reap it.
    fn end(&mut self) -> io::Result<ExitStatus> {
        self.done = true;
        #[cfg(unix)]
        {
            let mut running = running();
            // Not reaped yet, so its group is still its own.
            signal_group(self.child.id(), libc::SIGKILL);
            running.retain(|listed| *listed != self.listed);
        }
        #[cfg(windows)]
        jobs().retain(|(listed, _)| *listed != self.listed);
        self.child.wait()
    }

    /// Kill the sidecar and everything left in its process group on Unix,
    /// or in its job on Windows, then reap it.
    pub fn kill(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        #[cfg(unix)]
        {
            let mut running = running();
            signal_group(self.child.id(), libc::SIGKILL);
            running.retain(|listed| *listed != self.listed);
        }
        #[cfg(windows)]
        {
            let mut jobs = jobs();
            if let Some(index) = jobs.iter().position(|(listed, _)| *listed == self.listed) {
                jobs.remove(index).1.terminate();
            }
        }
        // Best effort: the sidecar may have ended already, and what a caller is told is how
        // the run went, not how it was put out.
        #[cfg(not(unix))]
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Run `command` to its end as a sidecar that no cancellation reaches
/// ([`Sidecar::start_uncancellable`]), as [`Command::output`] runs a plain child: its stdin is
/// null, and what it writes to stdout and stderr is returned with how it exited. What it left
/// running is ended once it exits, so its pipes close then.
pub fn output(command: &mut Command) -> io::Result<Output> {
    let mut sidecar = Sidecar::start_uncancellable(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    // Both read at once, so that neither fills while the other is waited on.
    let stdout = read_to_end(sidecar.stdout.take());
    let stderr = read_to_end(sidecar.stderr.take());
    let status = sidecar.wait()?;
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Read `pipe` to its end on a thread of its own.
fn read_to_end<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            // What was read before an error is what there is.
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// What stops a running [`Sidecar`] from a thread other than the one that
/// waits for it. Once the sidecar has been waited for, it does nothing.
#[derive(Clone, Copy, Debug)]
pub struct Stopper {
    listed: Listed,
}

impl Stopper {
    /// Ask the sidecar and what it started to end: SIGTERM to its process
    /// group on Unix. Windows has nothing to ask a job with.
    pub fn ask_to_end(&self) {
        #[cfg(unix)]
        {
            let running = running();
            if running.contains(&self.listed) {
                signal_group(self.listed.id, libc::SIGTERM);
            }
        }
    }

    /// Whether the sidecar has ended. Where nothing could be asked of it
    /// there is nothing to wait for.
    pub fn has_ended(&self) -> bool {
        #[cfg(unix)]
        {
            let running = running();
            !running.contains(&self.listed) || exited(self.listed.id)
        }
        #[cfg(not(unix))]
        true
    }

    /// Kill the sidecar and everything left in its process group on Unix,
    /// or in its job on Windows. It is reaped where it is waited for.
    pub fn kill(&self) {
        #[cfg(unix)]
        {
            let running = running();
            if running.contains(&self.listed) {
                signal_group(self.listed.id, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        if let Some((_, hold)) = jobs().iter().find(|(listed, _)| *listed == self.listed) {
            hold.terminate();
        }
    }

    /// Stop the sidecar politely: ask it to end, give it `grace` to, and
    /// kill what is left once it has ended or that is over. Returns then.
    pub fn stop(&self, grace: Duration) {
        self.ask_to_end();
        self.kill_after(grace);
    }

    /// The rest of [`Stopper::stop`], for a sidecar already asked to end:
    /// wait until it has ended or `grace` is over, then kill what is left.
    pub fn kill_after(&self, grace: Duration) {
        let asked = Instant::now();
        while asked.elapsed() < grace && !self.has_ended() {
            thread::sleep(POLL_INTERVAL);
        }
        self.kill();
    }
}

/// Whether process `id`, a child of this one, has exited, without reaping
/// it: it stays listed, and its group stays its own, until it is reaped.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "looks with waitid, which std does not wrap, without reaping the child"
)]
fn exited(id: u32) -> bool {
    // SAFETY: `info` is zeroed, which waitid reads as no child having changed, and is
    // written only by waitid. WNOWAIT leaves the child to be reaped later.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let asked = libc::waitid(
            libc::P_PID,
            id as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        asked != 0 || (info.si_pid() != 0 && ended_by(info.si_code))
    }
}

/// Whether a child's change of state, by its `si_code`, is its end: macOS's
/// waitid reports a stopped child too.
#[cfg(unix)]
fn ended_by(code: libc::c_int) -> bool {
    matches!(code, libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED)
}

/// Wait for process `id`, a child of this one, to exit, without reaping it,
/// as [`exited`] looks.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "waits with waitid, which std does not wrap, without reaping the child"
)]
fn wait_exited(id: u32) -> io::Result<()> {
    loop {
        // SAFETY: as in `exited`; without WNOHANG it returns once the child has exited.
        let (asked, code) = unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            let asked = libc::waitid(
                libc::P_PID,
                id as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            );
            (asked, info.si_code)
        };
        if asked == 0 && ended_by(code) {
            return Ok(());
        }
        if asked == 0 {
            // Stopped, not ended: it is looked at again in a moment.
            thread::sleep(POLL_INTERVAL);
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Send `signal` to the process group `leader` leads. Callers hold the
/// running lock and have not reaped `leader`, so the group is a sidecar's.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "signals the group with killpg, which std does not wrap"
)]
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
                            if !pass_on(signal) {
                                continue;
                            }
                            if let Err(error) =
                                signal_hook::low_level::emulate_default_handler(signal)
                            {
                                log::warn!(
                                    "signal {signal} was passed on to the sidecars, and did not end this process: {error}"
                                );
                            }
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

/// Pass `signal` on to the running sidecars, and say whether it should now end this process as
/// it would have. While [`stop_on_termination`] holds, the first one cancels that work instead,
/// and reaches only the sidecars no cancellation reaches.
#[cfg(unix)]
fn pass_on(signal: libc::c_int) -> bool {
    let stopped = stopped_instead(signal);
    for listed in running().iter() {
        if !stopped || listed.uncancellable {
            signal_group(listed.id, signal);
        }
    }
    !stopped
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
/// running is asked to end and has [`GRACE`] to, as a stopped one does,
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

#[cfg(all(test, unix))]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::sync::mpsc;

    /// Start a shell as a sidecar that starts `sleep` in the background,
    /// reports its own process id and the sleeper's on stderr, and waits.
    /// Returns the sidecar and the two ids once reported.
    fn start_sleeper() -> (Sidecar, u32, String) {
        let mut sidecar = Sidecar::start(
            Command::new("sh")
                .args(["-c", "sleep 30 & echo \"$$ $!\" >&2; wait"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped()),
        )
        .expect("sh starts");
        let mut line = String::new();
        BufReader::new(sidecar.stderr.take().expect("stderr is piped"))
            .read_line(&mut line)
            .expect("the shell reports its process ids");
        let (shell, sleeper) = line.trim().split_once(' ').expect("two process ids");
        (
            sidecar,
            shell.parse().expect("shell id"),
            sleeper.to_owned(),
        )
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
    fn a_sidecar_stopped_from_another_thread_is_killed_once_its_grace_is_over() {
        // Waited for on one thread and stopped from another, as a Claude Code turn is.
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "trap '' TERM; echo ready; while :; do sleep 0.05; done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut sidecar = Sidecar::start(&mut command).expect("sh starts");
        let shell = sidecar.child.id();
        let mut ready = String::new();
        BufReader::new(sidecar.stdout.take().expect("stdout is piped"))
            .read_line(&mut ready)
            .expect("the shell says it is ready");
        let stopper = sidecar.stopper();
        let (waited, wait) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let _ = waited.send(sidecar.wait().is_ok());
        });
        let grace = Duration::from_millis(300);
        let start = Instant::now();
        stopper.stop(grace);
        // It does not end when asked, so it was given the whole of its grace, then killed.
        let given = start.elapsed();
        let ended = wait.recv_timeout(Duration::from_secs(5));
        if ended.is_err() {
            // Not left running past a failed test.
            signal_group(shell, libc::SIGKILL);
        }
        assert_eq!(ended, Ok(true), "the sidecar outlived its grace");
        assert!(given >= grace, "{given:?}");
        waiter.join().expect("waiter");
    }

    #[test]
    fn a_signal_to_a_running_sidecar_reaches_what_it_started() {
        let (mut sidecar, shell, sleeper) = start_sleeper();
        {
            let running = running();
            assert!(running.iter().any(|listed| listed.id == shell));
            signal_group(shell, libc::SIGTERM);
        }
        let status = sidecar.wait().expect("the shell ends on the signal");
        assert!(!status.success());
        assert!(ends_within(&sleeper, Duration::from_secs(3)));
        assert!(!running().iter().any(|listed| listed.id == shell));
    }

    /// Held by the tests that pass signals on or hold `stop_on_termination`, which are the
    /// process's own and would reach each other's sidecars.
    static SIGNALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn signals() -> std::sync::MutexGuard<'static, ()> {
        SIGNALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn output_ends_with_its_child_and_ends_what_it_left_running() {
        let _signals = signals();
        let started = Instant::now();
        let ran = output(Command::new("sh").args([
            "-c",
            "echo said; echo warned >&2; sleep 30 & echo $! >&2; exit 3",
        ]))
        .expect("sh runs");
        // A plain child's output would wait the half minute the sleeper holds its pipes.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(ran.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&ran.stdout), "said\n");
        let said = String::from_utf8_lossy(&ran.stderr);
        let mut lines = said.lines();
        assert_eq!(lines.next(), Some("warned"));
        let sleeper = lines.next().expect("the shell reports the sleeper's id");
        assert!(ends_within(sleeper, Duration::from_secs(3)));
    }

    #[test]
    fn the_first_signal_held_reaches_only_the_sidecars_no_cancellation_reaches() {
        let _signals = signals();
        let sleeper = || {
            let mut command = Command::new("sleep");
            command.arg("30").stdin(Stdio::null()).stdout(Stdio::null());
            command
        };
        let mut plain = Sidecar::start(&mut sleeper()).expect("sleep starts");
        let mut bare = Sidecar::start_uncancellable(&mut sleeper()).expect("sleep starts");
        let token = CancellationToken::new();
        let held = stop_on_termination(&token);
        assert!(
            !pass_on(libc::SIGTERM),
            "the first signal ended the process"
        );
        assert!(token.is_cancelled());
        let ended = bare
            .wait()
            .expect("the uncancellable sleeper is waited for");
        assert!(!ended.success());
        assert!(
            plain.try_wait().expect("looked at").is_none(),
            "the sleeper the work stops was signalled"
        );
        drop(held);
        plain.kill();
    }

    #[test]
    fn the_first_signal_stops_the_work_held_and_a_second_ends_the_process() {
        let _signals = signals();
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
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    /// Start PowerShell as a sidecar, which starts `ping` waiting half a
    /// minute, reports its own process id and ping's on stderr, and waits.
    /// Returns the sidecar and the two ids once reported.
    fn start_pinger() -> (Sidecar, u32, u32) {
        let mut sidecar = Sidecar::start(
            Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$p = Start-Process ping -ArgumentList '-n','30','127.0.0.1' -NoNewWindow -PassThru; \
                     [Console]::Error.WriteLine(\"$PID $($p.Id)\"); $p.WaitForExit()",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped()),
        )
        .expect("PowerShell starts");
        let mut line = String::new();
        BufReader::new(sidecar.stderr.take().expect("stderr is piped"))
            .read_line(&mut line)
            .expect("PowerShell reports its process ids");
        let (shell, pinger) = line.trim().split_once(' ').expect("two process ids");
        (
            sidecar,
            shell.parse().expect("shell id"),
            pinger.parse().expect("ping id"),
        )
    }

    /// Whether process `pid` ends within `limit`.
    #[expect(
        unsafe_code,
        reason = "OpenProcess and WaitForSingleObject are Win32 calls with no safe wrapper"
    )]
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
    fn stopping_ends_what_the_sidecar_started() {
        let (mut sidecar, shell, pinger) = start_pinger();
        assert!(jobs().iter().any(|(listed, _)| listed.id == shell));
        let start = Instant::now();
        // Nothing to ask a job with: it is ended outright, without waiting out the grace.
        sidecar.stopper().stop(GRACE);
        sidecar.wait().expect("PowerShell is waited for");
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(ends_within(pinger, Duration::from_secs(3)));
        assert!(!jobs().iter().any(|(listed, _)| listed.id == shell));
    }

    #[test]
    fn a_sidecar_ends_when_its_job_closes() {
        // What closing kerfd's handles does to the sidecars it ran.
        #[expect(
            clippy::disallowed_methods,
            reason = "a plain child put in a job by hand: the job is what is tested"
        )]
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
