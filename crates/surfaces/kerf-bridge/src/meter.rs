//! The program meter: `meter {peak, rms, momentary, shortTerm, at}` about
//! 20 times a second while the program plays, and once with `null` values
//! when it stops.
//!
//! The playback session's level tap expects one reader (each reading folds
//! the blocks heard since the one before), so the media worker owns the
//! only [`Meter`] and polls it on its loop.

use crate::event::{Event, MeterReading};
use crate::wake::Outbox;
use kerf_playback::Levels;
use std::time::{Duration, Instant};

/// How often the meter is read while playing: 20 Hz.
pub(crate) const PERIOD: Duration = Duration::from_millis(50);

/// The quietest level reported, in dBFS: digital silence reads as this.
pub(crate) const FLOOR_DB: f64 = kerf_playback::LEVEL_FLOOR_DB as f64;

/// A level the tap gives in dBFS, to a hundredth of a decibel and never
/// below the floor.
pub(crate) fn dbfs(value: f32) -> f64 {
    let value = f64::from(value);
    if !value.is_finite() {
        return FLOOR_DB;
    }
    (value.max(FLOOR_DB) * 100.0).round() / 100.0
}

fn lufs(value: Option<f32>) -> Option<f64> {
    value
        .filter(|value| value.is_finite())
        .map(|value| (f64::from(value) * 100.0).round() / 100.0)
}

/// One reading as the editor gets it: peak and RMS of left and right in
/// dBFS (the tap's own values, at least the floor), BS.1770 momentary and short-term loudness in LUFS (`null` until
/// 400 ms or 3 s have been heard), and the program time it reaches.
pub(crate) fn event(levels: &Levels) -> Event {
    Event::Meter(MeterReading {
        peak: Some([dbfs(levels.peak[0]), dbfs(levels.peak[1])]),
        rms: Some([dbfs(levels.rms[0]), dbfs(levels.rms[1])]),
        momentary: lufs(levels.momentary),
        short_term: lufs(levels.short_term),
        at: Some(levels.at),
    })
}

/// What the meter says once nothing plays.
pub(crate) fn silent() -> Event {
    Event::Meter(MeterReading::default())
}

/// Reads the levels on the media worker's loop and sends them.
#[derive(Default)]
pub(crate) struct Meter {
    /// When the levels were last read.
    read: Option<Instant>,
    /// The newest reading sent, so a repeated one is not sent again.
    sent: Option<Levels>,
    /// Readings were sent since the meter last went quiet.
    live: bool,
}

impl Meter {
    /// Called on every turn of the loop at `now`: while `playing`, read
    /// `levels` once a period and send a reading that is new (replaceable,
    /// so a busy editor never holds the media worker up); once it stops,
    /// send the null reading, which the editor must not miss.
    pub(crate) fn tick(
        &mut self,
        now: Instant,
        playing: bool,
        levels: impl FnOnce() -> Option<Levels>,
        tx: &Outbox,
    ) {
        if !playing {
            if self.live {
                self.live = false;
                self.sent = None;
                self.read = None;
                let _ = tx.send(silent());
            }
            return;
        }
        if self
            .read
            .is_some_and(|read| now.duration_since(read) < PERIOD)
        {
            return;
        }
        self.read = Some(now);
        let Some(levels) = levels() else {
            return;
        };
        if self.sent == Some(levels) {
            return;
        }
        if tx.try_send(event(&levels)).is_ok() {
            self.live = true;
            self.sent = Some(levels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(peak: f32, at: f64) -> Levels {
        Levels {
            peak: [peak, peak - 6.0],
            rms: [peak - 3.0, peak - 9.0],
            momentary: Some(-17.84),
            short_term: None,
            at,
        }
    }

    #[test]
    fn a_full_queue_never_holds_the_media_worker_up() {
        let (tx, _rx) = crate::wake::outbox(1);
        tx.send(Event::ViewerReset).unwrap();
        let mut meter = Meter::default();
        let start = Instant::now();
        meter.tick(start, true, || Some(levels(-6.4, 1.0)), &tx);
        // Dropped, so the next period tries again.
        assert!(meter.sent.is_none());
    }
}
