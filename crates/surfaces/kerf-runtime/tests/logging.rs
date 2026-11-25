//! kerfd's log in its project folder grows only so far. Its own test binary, since the logger is
//! the process's.

use std::fs;
use std::path::Path;
use std::process::Command;

/// How long the log grows before it is set aside (`logging.rs`).
const SET_ASIDE_AT: usize = 1024 * 1024;

#[test]
fn the_project_log_is_set_aside_as_it_grows() {
    // The logger reads RUST_LOG, so this runs again in a process of its own without one, at the
    // level kerfd logs at when nothing says otherwise.
    let Some(directory) = std::env::var_os("KERF_TEST_PROJECT_LOG") else {
        let directory = tempfile::tempdir().expect("a folder");
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["the_project_log_is_set_aside_as_it_grows", "--exact"])
            .env("KERF_TEST_PROJECT_LOG", directory.path())
            .env_remove("RUST_LOG")
            .status()
            .unwrap();
        assert!(status.success(), "{status}");
        return;
    };
    let directory = Path::new(&directory);
    let log = directory.join(".kerf-runtime.log");
    let set_aside = directory.join(".kerf-runtime.log.old");
    // What an earlier kerfd left, as long as a log is let grow.
    fs::write(&log, vec![b'-'; SET_ASIDE_AT]).unwrap();
    kerf_runtime::logging::to_project(directory);

    log::warn!("first");
    assert_eq!(fs::read(&set_aside).unwrap(), vec![b'-'; SET_ASIDE_AT]);
    let first = fs::read_to_string(&log).unwrap();
    assert!(
        first.contains("first") && first.lines().count() == 1,
        "{first}"
    );

    let line = "x".repeat(1000);
    for number in 0..1100 {
        log::warn!("{number} {line}");
    }
    // Set aside once more, from its first line, and what was set aside before is gone.
    let older = fs::read_to_string(&set_aside).unwrap();
    assert!(older.starts_with(&first), "{}", &older[..200]);
    let newest = fs::read_to_string(&log).unwrap();
    assert!(newest.lines().last().unwrap().contains("1099 x"));
    // Neither goes more than a line past the limit.
    for kept in [&older, &newest] {
        assert!(kept.len() < SET_ASIDE_AT + 2 * line.len(), "{}", kept.len());
    }
    assert_eq!(older.lines().count() + newest.lines().count(), 1101);
}
