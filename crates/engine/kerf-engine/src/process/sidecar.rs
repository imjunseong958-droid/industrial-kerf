//! Sidecar processes run to their end: cancellation and stderr-driven progress.
//!
//! The supervision itself is `kerf_sidecar`'s, re-exported here: the process group or job
//! each sidecar runs in, how a cancelled one is asked to end and then killed, and the
//! termination signals passed on to them. It lives in `foundation` so that crates below the
//! engine run theirs through the same list of running sidecars.
//!
//! `run` runs one to its end, reading its stderr. A program that is
//! talked to over its pipes, as Claude Code is, is started as a [`Sidecar`]
//! and waited for on its own thread, while its [`Stopper`] stops it from
//! any other.

use std::io::{BufRead, BufReader, Read};
use std::process::{ChildStderr, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub use kerf_sidecar::{
    forward_termination_signals, output, stop_on_termination, CancellationToken, Sidecar,
    StopOnTermination, Stopper, GRACE,
};

use crate::error::{EngineError, Result};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long what a cancelled sidecar said on its way out is read for, once it has ended.
const LAST_WORDS: Duration = Duration::from_millis(500);
const STDERR_QUEUE_CAPACITY: usize = 128;
const MAX_STDERR_TAIL_BYTES: usize = 16 * 1024;

/// How a sidecar exited, with the end of what it wrote to stderr.
pub struct Finished {
    pub status: ExitStatus,
    pub stderr: String,
}

/// Run `command` to completion with stdin and stdout closed, passing each
/// stderr line to `on_line` as it arrives. When `cancellation` is
/// requested the child is asked to end, killed if it has not within
/// [`GRACE`], and `EngineError::Cancelled` returned; what it says while it
/// ends still goes to `on_line`.
pub fn run(
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
    let mut sidecar = Sidecar::start(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| EngineError::msg(format!("{program} could not start: {error}")))?;
    let Some(stderr) = sidecar.stderr.take() else {
        sidecar.kill();
        return Err(EngineError::msg(format!("{program} stderr is unavailable")));
    };
    let stopper = sidecar.stopper();
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
    // What it has said, waiting up to `wait` for it to say something: lines are taken as they
    // come, or a sidecar that says more than the queue holds in a poll waits on its stderr.
    let mut drain = |tail: &mut String, wait: Duration| {
        let first = match receiver.recv_timeout(wait) {
            Ok(line) => Some(line),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            // Its stderr is closed, perhaps while it runs on: there is nothing to wait for.
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                thread::sleep(wait);
                None
            }
        };
        for line in first.into_iter().chain(receiver.try_iter()) {
            append_bounded(tail, &line);
            on_line(&line);
        }
    };
    let outcome = loop {
        drain(&mut tail, Duration::ZERO);
        if cancellation.is_cancelled() {
            // Asked first, so that it can undo what it has under way and say so. It is
            // not reaped while it is waited for: its group stays its own to signal.
            let asked = Instant::now();
            stopper.ask_to_end();
            while asked.elapsed() < grace && !stopper.has_ended() {
                drain(&mut tail, POLL_INTERVAL);
            }
            // What it said on its way out may not have been read from its stderr yet: it is,
            // to the end. Its stderr closes once nothing of its group is left writing to it:
            // waited for before what is left of it is killed, when it ended by itself, and after.
            // A moment each, in case something outside its group holds the pipe open.
            let mut read_out = || {
                let since = Instant::now();
                while !reader.is_finished() && since.elapsed() < LAST_WORDS {
                    drain(&mut tail, POLL_INTERVAL);
                }
                drain(&mut tail, Duration::ZERO);
            };
            if stopper.has_ended() {
                read_out();
            }
            sidecar.kill();
            read_out();
            break Err(EngineError::Cancelled);
        }
        match sidecar.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => drain(&mut tail, POLL_INTERVAL),
            Err(error) => {
                sidecar.kill();
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
            drain(&mut tail, POLL_INTERVAL);
        }
        // It only forwards lines: what it sent is drained below, whether or not it panicked.
        let _ = reader.join();
        drain(&mut tail, Duration::ZERO);
    }
    outcome.map(|status| Finished {
        status,
        stderr: tail,
    })
}

/// [`output`] of a run that `cancellation` stops: when it is asked for, the child is asked to
/// end, killed if it has not within [`GRACE`], and `EngineError::Cancelled` returned.
pub fn output_cancellable(
    command: &mut Command,
    cancellation: &CancellationToken,
) -> Result<Output> {
    if cancellation.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let mut sidecar = Sidecar::start(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    let stdout = read_to_end(sidecar.stdout.take());
    let stderr = read_to_end(sidecar.stderr.take());
    let stopper = sidecar.stopper();
    loop {
        if cancellation.is_cancelled() {
            stopper.stop(GRACE);
            sidecar.kill();
            return Err(EngineError::Cancelled);
        }
        if let Some(status) = sidecar.try_wait()? {
            return Ok(Output {
                status,
                stdout: stdout.join().unwrap_or_default(),
                stderr: stderr.join().unwrap_or_default(),
            });
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Read `pipe` to its end on a thread of its own: what was read before an error is what there is.
fn read_to_end<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// Read a child's stderr, `pipe`, to its end on a thread of its own, keeping
/// the last of it to report.
pub(crate) fn drain_stderr(pipe: Option<ChildStderr>) -> thread::JoinHandle<String> {
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
    #[expect(
        clippy::disallowed_methods,
        reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
    )]
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
    fn a_cancellable_output_returns_what_was_written_or_ends_when_cancelled() {
        let ran = output_cancellable(
            Command::new("sh").args(["-c", "echo said; echo warned >&2; exit 3"]),
            &CancellationToken::new(),
        )
        .expect("the shell ends by itself");
        assert_eq!(ran.status.code(), Some(3));
        assert_eq!(
            (&ran.stdout[..], &ran.stderr[..]),
            (&b"said\n"[..], &b"warned\n"[..])
        );

        let token = CancellationToken::new();
        let runner = {
            let token = token.clone();
            thread::spawn(move || {
                output_cancellable(Command::new("sh").args(["-c", "sleep 30"]), &token)
            })
        };
        thread::sleep(Duration::from_millis(300));
        let asked = Instant::now();
        token.cancel();
        let result = runner.join().expect("runner");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(
            asked.elapsed() < Duration::from_secs(3),
            "{:?}",
            asked.elapsed()
        );
    }

    #[test]
    fn a_sidecar_that_says_a_lot_is_heard_as_fast_as_it_says_it() {
        // As many lines as a peak pass says of an hour and a half of sound.
        let mut command = Command::new("sh");
        command.args(["-c", "seq 100000 >&2"]);
        let mut heard = 0;
        let started = Instant::now();
        let finished = run(command, &CancellationToken::new(), |_| heard += 1)
            .expect("the shell ends by itself");
        assert!(finished.status.success());
        assert_eq!(heard, 100_000);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_sidecar_that_ends_by_itself_takes_what_it_left_running_with_it() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 >/dev/null 2>&1 & echo $! >&2"]);
        let mut said = Vec::new();
        let finished = run(command, &CancellationToken::new(), |line| {
            said.push(line.to_owned())
        })
        .expect("the shell ends by itself");
        assert!(finished.status.success());
        let sleeper = said.first().expect("the shell reports the sleeper's id");
        assert!(ends_within(sleeper, Duration::from_secs(3)));
    }

    #[test]
    fn everything_a_cancelled_sidecar_says_on_its_way_out_is_heard() {
        // Many lines, said at once as it ends: more than are read in the time it takes to end.
        let token = CancellationToken::new();
        let (sender, lines) = mpsc::channel();
        let runner = {
            let token = token.clone();
            thread::spawn(move || {
                let mut command = Command::new("sh");
                command.args([
                    "-c",
                    "trap 'printf \"line %s\\n\" $(seq 0 499) >&2; echo done >&2; exit 1' TERM; echo waiting >&2; while :; do sleep 0.05; done",
                ]);
                run(command, &token, |line| {
                    let _ = sender.send(line.to_owned());
                })
            })
        };
        assert_eq!(
            lines.recv_timeout(Duration::from_secs(5)).as_deref(),
            Ok("waiting")
        );
        token.cancel();
        assert!(matches!(
            runner.join().expect("runner"),
            Err(EngineError::Cancelled)
        ));
        let said: Vec<String> = lines.try_iter().collect();
        let heard = |line: &str| said.iter().any(|said| said == line);
        assert!(
            heard("line 0") && heard("line 499") && heard("done"),
            "{said:?}"
        );
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
    fn cancelling_ends_what_the_sidecar_started() {
        let token = CancellationToken::new();
        let (runner, _, pinger) = run_pinger(&token);
        let start = Instant::now();
        token.cancel();
        let result = runner.join().expect("runner");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(ends_within(pinger, Duration::from_secs(3)));
    }
}
