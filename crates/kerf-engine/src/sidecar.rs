//! Supervised sidecar processes: cancellation and stderr-driven progress.

use std::io::{BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};
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
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| EngineError::msg(format!("{program} could not start: {error}")))?;
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
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
            let _ = child.kill();
            let _ = child.wait();
            break Err(EngineError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error.into());
            }
        }
    };
    // The reader ends once the child's stderr closes; keep draining so a
    // full queue cannot block it. A killed child's own children can hold
    // stderr open long after it, so a run that killed it does not wait: the
    // reader stops on its own once they close it or the queue is dropped.
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
