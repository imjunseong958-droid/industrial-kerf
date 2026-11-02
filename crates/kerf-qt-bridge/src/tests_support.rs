//! What the bridge's unit tests share: git kept away from the person's own
//! configuration, and waiting on work done by other threads.

use crate::Shared;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Mutex, Once};
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
        export_busy: AtomicBool::new(false),
        abort: Mutex::new(None),
        frame: Mutex::new(None),
        seek: Mutex::new(None),
        project: Mutex::new(None),
        model: Mutex::new(None),
        previews: AtomicU64::new(0),
        preview_cancel: Mutex::new(None),
        stale_previews: Mutex::new(Vec::new()),
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

/// Keep the person's global git configuration (signing, hooks, templates)
/// out of the tests, and give commits an author.
pub(crate) fn hermetic() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let empty = std::env::temp_dir().join("kerf-qt-bridge-empty-gitconfig");
        fs::write(&empty, "").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_AUTHOR_NAME", "Test");
        std::env::set_var("GIT_AUTHOR_EMAIL", "test@example.com");
        std::env::set_var("GIT_COMMITTER_NAME", "Test");
        std::env::set_var("GIT_COMMITTER_EMAIL", "test@example.com");
    });
}

/// Run git in `dir`, which must succeed; its trimmed output.
pub(crate) fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

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
