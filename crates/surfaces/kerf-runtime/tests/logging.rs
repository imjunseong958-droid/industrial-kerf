//! kerfd's log in its project folder grows only so far. Its own test binary, since the logger is
//! the process's.

use std::fs;

/// How long the log grows before it is set aside (`logging.rs`).
const SET_ASIDE_AT: usize = 1024 * 1024;

#[test]
fn the_project_log_is_set_aside_as_it_grows() {
    let directory = tempfile::tempdir().expect("a folder");
    let log = directory.path().join(".kerf-runtime.log");
    let set_aside = directory.path().join(".kerf-runtime.log.old");
    // What an earlier kerfd left, as long as a log is let grow.
    fs::write(&log, vec![b'-'; SET_ASIDE_AT]).unwrap();
    kerf_runtime::logging::to_project(directory.path());

    // Errors, which any RUST_LOG but `off` lets through.
    log::error!("first");
    assert_eq!(fs::read(&set_aside).unwrap(), vec![b'-'; SET_ASIDE_AT]);
    let first = fs::read_to_string(&log).unwrap();
    assert!(
        first.contains("first") && first.lines().count() == 1,
        "{first}"
    );

    let line = "x".repeat(1000);
    for number in 0..1100 {
        log::error!("{number} {line}");
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
