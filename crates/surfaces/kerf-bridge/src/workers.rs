//! The threads a bridge starts itself.
//!
//! A bridge never waits for its threads, so nothing joins them: each is named for what it
//! does, which a debugger and a crash report show, and counted from before it starts until
//! its work is over, however it ends. Whoever let a bridge go can then find out that they
//! are done ([`Workers::ended`]), or how many are not.
//!
//! Only those are counted. What they have other crates start for the bridge (a playback's
//! threads, the render or making a tool of the agent's started) is not; of those, the ones
//! that draw are waited for with `drawing_settled`.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The threads one bridge started itself and whose work is not over yet. A copy counts the
/// same threads, and goes on counting them once the bridge is let go.
#[derive(Clone, Debug)]
pub struct Workers(Arc<Count>);

#[derive(Debug, Default)]
struct Count {
    running: Mutex<usize>,
    /// Rung when the last one running has ended.
    none: Condvar,
}

/// One thread, counted until this is dropped: at its end, or as it unwinds.
struct Running(Workers);

impl Drop for Running {
    fn drop(&mut self) {
        let mut running = self.0.count();
        *running -= 1;
        if *running == 0 {
            self.0 .0.none.notify_all();
        }
    }
}

impl Workers {
    /// Those of a bridge that has started none yet.
    pub(crate) fn new() -> Self {
        Self(Arc::default())
    }

    /// Run `work` on a thread of its own, named `name`: 15 bytes at most, which is all Linux
    /// shows of a name.
    pub(crate) fn spawn(&self, name: &str, work: impl FnOnce() + Send + 'static) {
        debug_assert!(name.len() <= 15, "`{name}` is longer than Linux shows");
        // Counted before it starts, so that it is never started and not yet counted.
        *self.count() += 1;
        let running = Running(self.clone());
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let _running = running;
                work();
            })
            // As `std::thread::spawn` does when the system has no thread to give.
            .expect("failed to spawn thread");
    }

    /// How many of the threads are still at work.
    pub fn running(&self) -> usize {
        *self.count()
    }

    /// Wait, for at most `within`, until the work of every thread is over: `false` when one
    /// is still at it. For after the bridge is let go; while it is open its own workers run.
    pub fn ended(&self, within: Duration) -> bool {
        let until = Instant::now().checked_add(within);
        let mut running = self.count();
        while *running > 0 {
            let left = match until {
                Some(until) => until.saturating_duration_since(Instant::now()),
                // No end, for a time too long to count to.
                None => Duration::MAX,
            };
            if left.is_zero() {
                return false;
            }
            running = self
                .0
                .none
                .wait_timeout(running, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    fn count(&self) -> MutexGuard<'_, usize> {
        self.0
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_thread_is_counted_until_it_has_ended_however_it_ends() {
        let workers = Workers::new();
        assert!(workers.ended(Duration::ZERO));

        // One that waits to be told to end, and says its own name.
        let (end, told) = mpsc::channel::<()>();
        let (name, named) = mpsc::channel();
        workers.spawn("bridge:waits", move || {
            name.send(thread::current().name().map(str::to_owned))
                .unwrap();
            let _ = told.recv();
        });
        assert_eq!(named.recv().unwrap().as_deref(), Some("bridge:waits"));
        // One that ends by unwinding, quietly: nothing is said of what was thrown.
        workers.spawn("bridge:unwinds", || std::panic::resume_unwind(Box::new(())));

        // The one that unwound is counted no more, once it has.
        let unwound = crate::tests_support::wait(Duration::from_secs(30), || {
            (workers.running() == 1).then_some(())
        });
        assert!(unwound.is_some(), "{} are counted", workers.running());
        // The one that waits is still counted, and a copy counts it too.
        assert!(!workers.clone().ended(Duration::from_millis(50)));
        end.send(()).unwrap();
        assert!(workers.ended(Duration::from_secs(30)));
        assert_eq!(workers.running(), 0);
    }
}
