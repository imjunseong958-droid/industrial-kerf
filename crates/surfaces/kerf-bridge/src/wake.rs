//! How the bridge's workers hand the editor its events, and wake it as they do: the
//! editor waits on a [`Wake`] instead of asking on a timer.

use crate::Event;
use std::sync::mpsc::{self, Receiver, SyncSender};

/// Rung when the bridge has something for whoever empties it: an event put on its queue,
/// or a new playback or signal frame kept. Rings that come while nobody waits are heard as
/// one, so whoever waits takes everything there is ([`crate::Bridge::poll`] until `None`,
/// [`crate::Bridge::frame`], [`crate::Bridge::signal`]) each time it wakes. A clone rings
/// and waits on the same bell.
#[derive(Clone)]
pub struct Wake {
    bell: async_channel::Sender<()>,
    heard: async_channel::Receiver<()>,
}

impl Wake {
    fn new() -> Self {
        let (bell, heard) = async_channel::bounded(1);
        Self { bell, heard }
    }

    /// Wakes whoever waits, or the next to wait. Never blocks.
    pub fn ring(&self) {
        // Full, it has rung already, and that ring is heard as this one.
        let _ = self.bell.try_send(());
    }

    /// Returns once the bell has rung since it last returned: at once when it already has.
    pub async fn wait(&self) {
        // Never closed: this end holds the bell as well.
        let _ = self.heard.recv().await;
    }
}

/// The workers' end of the bridge's event queue: each event put on it rings the
/// [`Wake`], after the event is there to take.
#[derive(Clone)]
pub(crate) struct Outbox {
    queue: SyncSender<Event>,
    wake: Wake,
}

/// An event that was not put on the queue: it was full ([`Outbox::try_send`]), or
/// nobody takes events from it any more.
#[derive(Debug)]
pub(crate) struct NotSent;

/// An event queue with room for `bound` events: the workers' end, and the end the
/// events are taken from. [`Outbox::wake`] is its bell.
pub(crate) fn outbox(bound: usize) -> (Outbox, Receiver<Event>) {
    let (queue, events) = mpsc::sync_channel(bound);
    let outbox = Outbox {
        queue,
        wake: Wake::new(),
    };
    (outbox, events)
}

impl Outbox {
    /// Puts `event` on the queue, waiting for room, as [`SyncSender::send`] does.
    pub(crate) fn send(&self, event: Event) -> Result<(), NotSent> {
        let sent = self.queue.send(event).map_err(|_| NotSent);
        self.wake.ring();
        sent
    }

    /// Puts `event` on the queue unless it is full, as [`SyncSender::try_send`] does.
    pub(crate) fn try_send(&self, event: Event) -> Result<(), NotSent> {
        self.queue.try_send(event).map_err(|_| NotSent)?;
        self.wake.ring();
        Ok(())
    }

    /// Rings with no event: something is kept for the taking beside the queue (a
    /// playback or signal frame).
    pub(crate) fn ring(&self) {
        self.wake.ring();
    }

    /// The bell this queue rings.
    pub(crate) fn wake(&self) -> Wake {
        self.wake.clone()
    }
}
