//! A crash that is not a panic, a bad memory access or an abort in native code such as the
//! editor's C++, leaves a report as a panic does, which `crash::reports` lists for the editor's
//! next start to tell of. Each test crashes a child process on purpose: this test binary, run
//! again for `crash_on_purpose` alone.

use kerf_runtime::crash;
use std::process::{Command, ExitStatus, Stdio};

/// What the child does, as `KERF_CRASH_ON_PURPOSE` says; run only as a child of the tests below.
#[test]
#[ignore = "crashes on purpose: the other tests run it in a process of its own"]
fn crash_on_purpose() {
    let Ok(how) = std::env::var("KERF_CRASH_ON_PURPOSE") else {
        return;
    };
    let folder = std::env::var_os("KERF_CRASH_DIR").expect("a crash folder");
    crash::install_in("editor", folder.into());
    without_a_crash_dialog();
    match how.as_str() {
        // SAFETY: none; this is the crash. Address 16 is never mapped.
        "segfault" => unsafe { std::ptr::write_volatile(std::ptr::without_provenance_mut::<u8>(16), 1) },
        // SAFETY: abort takes nothing; it is what Qt and std::terminate end a process with.
        "abort" => unsafe { libc::abort() },
        "fatal" => {
            crash::fatal("QWidget: Must construct a QApplication before a QWidget");
            // SAFETY: as above.
            unsafe { libc::abort() }
        }
        "panic, then abort" => {
            let _ = std::panic::catch_unwind(|| panic!("a panic that cannot unwind"));
            // SAFETY: as above.
            unsafe { libc::abort() }
        }
        other => panic!("no crash called {other:?}"),
    }
}

/// Windows would otherwise ask, on a desktop, whether to look for a solution online.
fn without_a_crash_dialog() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
        };
        extern "C" {
            fn _set_abort_behavior(flags: u32, mask: u32) -> u32;
        }
        // SAFETY: both only set how this process reports its own end.
        unsafe {
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
            // Neither "abort() has been called" nor a fault report to Windows.
            _set_abort_behavior(0, 0x1 | 0x2);
        }
    }
}

struct Crashed {
    status: ExitStatus,
    stderr: String,
    folder: tempfile::TempDir,
}

fn crash(how: &str) -> Crashed {
    let folder = tempfile::tempdir().expect("a folder");
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args(["crash_on_purpose", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env("KERF_CRASH_ON_PURPOSE", how)
        .env("KERF_CRASH_DIR", folder.path())
        .stdin(Stdio::null())
        .output()
        .expect("the child starts");
    Crashed {
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        folder,
    }
}

/// The one report the child left, and its text.
fn only_report(crashed: &Crashed) -> (crash::Report, String) {
    let reports = crash::reports(crashed.folder.path());
    assert_eq!(
        reports.len(),
        1,
        "{reports:?}\nthe child: {:?}\n{}",
        crashed.status,
        crashed.stderr
    );
    let text = std::fs::read_to_string(&reports[0].path).expect("the report");
    (reports[0].clone(), text)
}

fn backtrace(text: &str) -> &str {
    text.split_once("\nbacktrace:\n").map_or("", |(_, frames)| frames)
}

/// Each platform's backtrace names the binary a frame is in.
fn names_this_binary(frames: &str) -> bool {
    let binary = std::env::current_exe().expect("this test binary");
    let name = binary.file_stem().expect("a name").to_string_lossy();
    frames.contains(&*name)
}

#[cfg(unix)]
fn died_of(status: ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[test]
fn a_bad_memory_access_leaves_a_report_with_where_and_a_backtrace() {
    let crashed = crash("segfault");
    #[cfg(unix)]
    assert_eq!(died_of(crashed.status), Some(libc::SIGSEGV), "{}", crashed.stderr);
    #[cfg(windows)]
    assert_eq!(crashed.status.code(), Some(0xC000_0005_u32 as i32), "{}", crashed.stderr);
    let (report, text) = only_report(&crashed);
    assert_eq!(report.component, "editor");
    if cfg!(windows) {
        assert_eq!(report.message, "Access violation (exception 0xc0000005)", "{text}");
        assert!(text.contains(" writing 0x10\n"), "{text}");
    } else {
        assert_eq!(report.message, "Segmentation fault (SIGSEGV)", "{text}");
        assert!(text.contains("\nlocation: address 0x10\n"), "{text}");
    }
    assert!(text.contains(&format!("\nversion: {}\n", env!("CARGO_PKG_VERSION"))), "{text}");
    assert!(text.contains("\nthread: main\n"), "{text}");
    let frames = backtrace(&text);
    assert!(frames.lines().count() >= 3, "{text}");
    assert!(names_this_binary(frames), "{text}");
}

#[test]
fn an_abort_leaves_a_report_with_a_backtrace() {
    let crashed = crash("abort");
    assert!(!crashed.status.success());
    #[cfg(unix)]
    assert_eq!(died_of(crashed.status), Some(libc::SIGABRT), "{}", crashed.stderr);
    let (report, text) = only_report(&crashed);
    assert_eq!(report.component, "editor");
    assert_eq!(report.message, "Aborted (SIGABRT)", "{text}");
    assert!(names_this_binary(backtrace(&text)), "{text}");
}

// Qt's qFatal: the editor reports the message, and the abort that follows adds nothing.
#[test]
fn a_fatal_error_is_reported_once_with_its_message() {
    let crashed = crash("fatal");
    assert!(!crashed.status.success());
    let (report, text) = only_report(&crashed);
    assert_eq!(report.message, "QWidget: Must construct a QApplication before a QWidget");
    assert!(text.contains("crash_on_purpose"), "the backtrace names the function: {text}");
}

// A panic that cannot unwind out of an extern "C" function aborts: its report is the one.
#[test]
fn an_abort_right_after_a_panic_leaves_only_the_panics_report() {
    let crashed = crash("panic, then abort");
    assert!(!crashed.status.success());
    let (report, _) = only_report(&crashed);
    assert_eq!(report.message, "a panic that cannot unwind");
}
