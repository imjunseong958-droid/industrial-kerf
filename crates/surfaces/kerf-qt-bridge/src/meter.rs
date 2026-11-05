//! The program meter: `meter {peak, rms, momentary, shortTerm, at}` about
//! 20 times a second while the program plays, and once with `null` values
//! when it stops.
//!
//! The playback session's level tap expects one reader (each reading folds
//! the blocks heard since the one before), so the media worker owns the
//! only [`Meter`] and polls it on its loop.

use kerf_playback::Levels;
use serde_json::{json, Value};
use std::sync::mpsc::SyncSender;
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

fn lufs(value: Option<f32>) -> Value {
    value
        .filter(|value| value.is_finite())
        .map_or(Value::Null, |value| {
            json!((f64::from(value) * 100.0).round() / 100.0)
        })
}

/// One reading as the editor gets it: peak and RMS of left and right in
/// dBFS (the tap's own values, at least the floor), BS.1770 momentary and short-term loudness in LUFS (`null` until
/// 400 ms or 3 s have been heard), and the program time it reaches.
pub(crate) fn event(levels: &Levels) -> Value {
    json!({
        "type": "meter",
        "peak": [dbfs(levels.peak[0]), dbfs(levels.peak[1])],
        "rms": [dbfs(levels.rms[0]), dbfs(levels.rms[1])],
        "momentary": lufs(levels.momentary),
        "shortTerm": lufs(levels.short_term),
        "at": levels.at,
    })
}

/// What the meter says once nothing plays.
pub(crate) fn silent() -> Value {
    json!({
        "type": "meter",
        "peak": Value::Null,
        "rms": Value::Null,
        "momentary": Value::Null,
        "shortTerm": Value::Null,
        "at": Value::Null,
    })
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
        tx: &SyncSender<Value>,
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
    use std::cell::Cell;
    use std::sync::mpsc;

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
    fn levels_read_in_dbfs_and_lufs() {
        assert_eq!(dbfs(0.0), 0.0);
        assert_eq!(dbfs(-6.4012), -6.4);
        assert_eq!(dbfs(-500.0), FLOOR_DB);
        assert_eq!(dbfs(f32::NAN), FLOOR_DB);
        assert_eq!(dbfs(f32::NEG_INFINITY), FLOOR_DB);
        let reading = event(&levels(-6.4, 17.32));
        assert_eq!(reading["type"], "meter");
        assert_eq!(reading["peak"], json!([-6.4, -12.4]));
        assert_eq!(reading["rms"], json!([-9.4, -15.4]));
        assert_eq!(reading["momentary"], -17.84);
        assert_eq!(reading["shortTerm"], Value::Null);
        assert_eq!(reading["at"], 17.32);
    }

    /// A fake session whose level tap gives a new reading each read.
    struct Session {
        reads: Cell<u32>,
    }

    impl Session {
        fn levels(&self) -> Option<Levels> {
            let reads = self.reads.get() + 1;
            self.reads.set(reads);
            Some(levels(-6.4, f64::from(reads) * 0.05))
        }
    }

    #[test]
    fn the_meter_reads_at_20_hz_while_playing_and_goes_quiet_once() {
        let (tx, rx) = mpsc::sync_channel(64);
        let session = Session {
            reads: Cell::new(0),
        };
        let mut meter = Meter::default();
        let start = Instant::now();
        // The media worker's loop turns about every millisecond for one
        // second of playing.
        for ms in 0..1000 {
            let now = start + Duration::from_millis(ms);
            meter.tick(now, true, || session.levels(), &tx);
        }
        assert_eq!(session.reads.get(), 20, "one read a period");
        let readings: Vec<Value> = rx.try_iter().collect();
        assert_eq!(readings.len(), 20);
        assert!(readings.iter().all(|reading| reading["peak"][0] == -6.4));

        // Paused: one null reading, then nothing.
        for ms in 1000..1200 {
            let now = start + Duration::from_millis(ms);
            meter.tick(now, false, || session.levels(), &tx);
        }
        let quiet: Vec<Value> = rx.try_iter().collect();
        assert_eq!(quiet, [silent()]);
        assert_eq!(
            session.reads.get(),
            20,
            "nothing reads the tap while paused"
        );
    }

    #[test]
    fn a_repeated_reading_is_sent_once_and_a_paused_meter_never_sends() {
        let (tx, rx) = mpsc::sync_channel(64);
        let mut meter = Meter::default();
        let start = Instant::now();
        meter.tick(start, false, || panic!("not read"), &tx);
        assert!(rx.try_recv().is_err(), "nothing to say before playing");
        for step in 0..5 {
            let now = start + PERIOD * step;
            meter.tick(now, true, || Some(levels(-12.0, 1.0)), &tx);
        }
        assert_eq!(rx.try_iter().count(), 1);
        // Nothing heard yet: no reading, and nothing to clear.
        let mut fresh = Meter::default();
        fresh.tick(start, true, || None, &tx);
        fresh.tick(start + PERIOD, false, || None, &tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_full_queue_never_holds_the_media_worker_up() {
        let (tx, _rx) = mpsc::sync_channel(1);
        tx.send(json!({"type": "clock"})).unwrap();
        let mut meter = Meter::default();
        let start = Instant::now();
        meter.tick(start, true, || Some(levels(-6.4, 1.0)), &tx);
        // Dropped, so the next period tries again.
        assert!(meter.sent.is_none());
    }
}
