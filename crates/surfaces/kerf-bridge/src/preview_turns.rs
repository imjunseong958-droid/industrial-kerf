//! The order previews and the stops for them were sent in.
//!
//! A `preview` reaches its render through the runtime worker's queue, and
//! `preview_stop` and `preview_exit` act where they are sent, so a stop
//! sent after a preview can get there first. Each preview therefore has a
//! turn, counted as it is sent, and a stop names the last turn sent before
//! it: a preview of that turn or an earlier one never starts, or stops if it
//! has, and says so itself when it is over.

use std::sync::{Mutex, MutexGuard, PoisonError};

#[derive(Default)]
pub(crate) struct PreviewTurns(Mutex<Turns>);

#[derive(Default)]
struct Turns {
    /// Previews sent; the turn of the last one.
    sent: u64,
    /// Previews the runtime worker took off its queue.
    taken: u64,
    /// Previews sent and not over: waiting in the queue, or rendering.
    open: u64,
    /// Every preview up to this turn was asked to stop.
    stopped: u64,
}

impl PreviewTurns {
    fn turns(&self) -> MutexGuard<'_, Turns> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Count a preview as sent when `queue` puts it on the runtime worker's
    /// queue. The worker cannot take it before it is counted.
    pub(crate) fn send(&self, queue: impl FnOnce() -> bool) -> bool {
        let mut turns = self.turns();
        let queued = queue();
        if queued {
            turns.sent += 1;
            turns.open += 1;
        }
        queued
    }

    /// The turn of the preview the runtime worker has just taken off its
    /// queue, which hands them over in the order they were sent.
    pub(crate) fn take(&self) -> u64 {
        let mut turns = self.turns();
        turns.taken += 1;
        turns.taken
    }

    /// Ask every preview sent so far to stop. `true` when one of them is not
    /// over yet: it answers for the stop when it is.
    pub(crate) fn stop(&self) -> bool {
        let mut turns = self.turns();
        turns.stopped = turns.sent;
        turns.open > 0
    }

    /// Whether the preview of `turn` was asked to stop.
    pub(crate) fn stopped(&self, turn: u64) -> bool {
        self.turns().stopped >= turn
    }

    /// The preview of `turn` is over: rendered, refused, failed or stopped.
    /// Whether it was asked to stop, decided here so that a stop sent from
    /// now on finds it over and answers for itself.
    pub(crate) fn over(&self, turn: u64) -> bool {
        let mut turns = self.turns();
        turns.open = turns.open.saturating_sub(1);
        turns.stopped >= turn
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewTurns;

    #[test]
    fn a_stop_covers_the_previews_sent_before_it_and_none_after() {
        let turns = PreviewTurns::default();
        assert!(!turns.stop(), "nothing to stop: the stop answers itself");

        // Sent, then stopped before the worker has taken it.
        assert!(turns.send(|| true));
        assert!(turns.stop(), "the preview on its way answers");
        let first = turns.take();
        assert!(turns.stopped(first));
        assert!(turns.over(first));
        assert!(!turns.stop(), "it is over");

        // One sent after the stop is not stopped by it.
        assert!(turns.send(|| true));
        let second = turns.take();
        assert!(!turns.stopped(second));
        // A stop while it renders: it answers when it is over.
        assert!(turns.stop());
        assert!(turns.stopped(second));
        assert!(turns.over(second));

        // One that ended before the stop was sent is not what answers.
        assert!(turns.send(|| true));
        let third = turns.take();
        assert!(!turns.over(third));
        assert!(!turns.stop());
    }

    #[test]
    fn a_preview_the_queue_refused_was_never_sent() {
        let turns = PreviewTurns::default();
        assert!(!turns.send(|| false));
        assert!(!turns.stop());
        assert!(turns.send(|| true));
        assert_eq!(turns.take(), 1);
    }
}
