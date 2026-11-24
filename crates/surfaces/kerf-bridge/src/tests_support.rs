//! What the bridge's unit tests share: git kept away from the person's own
//! configuration, and waiting on work done by other threads.

use crate::Shared;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The state the bridge's workers share, for the project in `directory`.
pub(crate) fn shared(directory: &Path) -> Shared {
    Shared {
        directory: directory.to_owned(),
        media_status: Mutex::new(Default::default()),
        copies: AtomicUsize::new(0),
        revision: AtomicU64::new(0),
        closed: AtomicBool::new(false),
        agent_busy: AtomicBool::new(false),
        version_aborts: std::sync::Mutex::new(Vec::new()),
        preview_busy: AtomicBool::new(false),
        abort: Mutex::new(None),
        frame: Mutex::new(None),
        seek: Mutex::new(None),
        project: Mutex::new(None),
        model: Mutex::new(None),
        previews: AtomicU64::new(0),
        preview_cancel: Mutex::new(None),
        preview_turns: Default::default(),
        stale_previews: Mutex::new(Vec::new()),
        shown_preview: Mutex::new(None),
        check_busy: AtomicBool::new(false),
        check_cancel: Mutex::new(None),
        makings: Default::default(),
        others_makings: Default::default(),
        renders: Default::default(),
        tool_renders: Default::default(),
        analysis: Default::default(),
        workflows: Default::default(),
        drawers: Default::default(),
    }
}

/// A program of two cuts from one file and a caption: a preset of it has
/// two slots (two seconds and one second of picture) and one text.
pub(crate) const PROGRAM: &str = "kerf 1\nproject name=\"Library\"\n\
    media m1 path=\"media/shot.mov\" rate=24 dur=96f video=320x180 audio=2ch@48000\n\
    sequence s1 rate=24 size=320x180 audio=48000\n\
    \x20 track V1 video\n\
    \x20   clip c1 media=m1 src=0f at=0f dur=48f\n\
    \x20   clip c2 media=m1 src=48f at=48f dur=24f\n\
    \x20 track S1 subtitle\n\
    \x20   clip t1 caption=\"Hello\" at=0f dur=24f\n";

/// Ask `check` until it answers or `timeout` passes.
pub(crate) fn wait<T>(timeout: Duration, mut check: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
