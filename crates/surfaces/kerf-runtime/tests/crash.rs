//! A panic leaves a report, and a release build can catch one. Its own test binary, since the hook
//! is the process's.

use kerf_runtime::crash;
use std::panic;
use std::path::Path;

#[test]
fn a_caught_panic_leaves_a_report_with_where_and_a_backtrace() {
    let dir = tempfile::tempdir().expect("a folder");
    crash::install_in("kerfd", dir.path().to_owned());
    let caught = std::thread::Builder::new()
        .name("media index".into())
        .spawn(|| panic::catch_unwind(|| panic!("boom in file {}", 3)).is_err())
        .expect("a thread")
        .join()
        .expect("the thread returns");
    assert!(caught, "the panic is caught, as kernel.rs and tasks.rs catch theirs");
    let reports = crash::reports(dir.path());
    assert_eq!(reports.len(), 1, "{reports:?}");
    let report = &reports[0];
    assert_eq!(report.component, "kerfd");
    assert_eq!(report.message, "boom in file 3");
    let text = std::fs::read_to_string(&report.path).expect("the report");
    assert!(text.contains("thread: media index"), "{text}");
    // Windows names the file with backslashes.
    assert!(text.replace('\\', "/").contains("location: crates/surfaces/kerf-runtime/tests/crash.rs:"), "{text}");
    assert!(
        text.contains("a_caught_panic_leaves_a_report_with_where_and_a_backtrace"),
        "the backtrace names the function: {text}"
    );
}

/// `catch_unwind` does nothing in a build that aborts on panic, and a stripped binary's backtrace
/// names no function. Tests always unwind, whatever the profile says, so the profile itself is
/// what is held.
#[test]
fn release_builds_unwind_and_keep_symbols() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("the workspace manifest");
    let release: Vec<&str> = text
        .lines()
        .skip_while(|line| line.trim() != "[profile.release]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(str::trim)
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect();
    assert!(!release.is_empty(), "no [profile.release] in {}", manifest.display());
    let setting = |key: &str| {
        release
            .iter()
            .find_map(|line| line.strip_prefix(key)?.trim_start().strip_prefix('='))
            .map(|value| value.trim().trim_matches('"'))
    };
    assert_ne!(setting("panic"), Some("abort"), "{release:?}");
    assert!(
        matches!(setting("strip"), None | Some("none" | "false" | "debuginfo")),
        "{release:?}"
    );
}
