//! Frames kept in memory by what they depend on, across compositors: a compositor is made again for
//! every edit, and most of a sequence does not change with one. The key is worked out by
//! `Drawing::frame_key`.

use crate::Frame;
use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

/// How many bytes of frames are kept.
const BUDGET: usize = 768 << 20;

#[derive(Default)]
struct Frames {
    /// Oldest first.
    kept: VecDeque<(u64, Frame)>,
    bytes: usize,
}

static FRAMES: Mutex<Option<Frames>> = Mutex::new(None);

fn with<T>(act: impl FnOnce(&mut Frames) -> T) -> T {
    let mut frames = FRAMES.lock().unwrap_or_else(PoisonError::into_inner);
    act(frames.get_or_insert_with(Frames::default))
}

/// The frame kept under `key`, which then counts as the newest.
pub(crate) fn get(key: u64) -> Option<Frame> {
    with(|frames| {
        let place = frames.kept.iter().position(|(kept, _)| *kept == key)?;
        let entry = frames.kept.remove(place)?;
        let frame = entry.1.clone();
        frames.kept.push_back(entry);
        Some(frame)
    })
}

/// Whether a frame is kept under `key`.
pub(crate) fn has(key: u64) -> bool {
    with(|frames| frames.kept.iter().any(|(kept, _)| *kept == key))
}

/// Keep `frame` under `key`, letting the oldest go past the budget.
pub(crate) fn put(key: u64, frame: Frame) {
    with(|frames| {
        if frames.kept.iter().any(|(kept, _)| *kept == key) {
            return;
        }
        frames.bytes += frame.rgba.len();
        frames.kept.push_back((key, frame));
        while frames.bytes > BUDGET && frames.kept.len() > 1 {
            if let Some((_, oldest)) = frames.kept.pop_front() {
                frames.bytes -= oldest.rgba.len();
            }
        }
    });
}
