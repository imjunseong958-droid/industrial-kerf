//! What a panic leaves behind: a plain-text report in the user's crash folder, with what panicked,
//! where, on which thread, and the backtrace, so a crash says where it happened after the process
//! is gone.
//!
//! [`install`] chains a panic hook in front of the one already set, so stderr still gets the usual
//! message. A panic that `catch_unwind` turns into an error, such as a failed editor task or one
//! media file's index in kerfd, is reported too: it is still a bug. It also installs a handler for
//! what is not a panic, a bad memory access or an abort in C or C++ code such as the editor's,
//! Qt's or FFmpeg's, which writes the same kind of report (`native.rs`). Reports stay on this
//! machine; nothing sends them anywhere. The folder keeps the newest [`KEPT`].

mod native;

use std::backtrace::Backtrace;
use std::fs;
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many reports the folder keeps; the oldest go first.
pub const KEPT: usize = 50;

/// One report in the crash folder.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Report {
    pub path: PathBuf,
    /// `kerfd`, `editor`, or `kerf`.
    pub component: String,
    /// When it was written, in milliseconds since 1970.
    pub at_ms: u128,
    /// The panic's message, first line.
    pub message: String,
}

/// The crash folder: `KERF_CRASH_DIR`, or the platform's place for an app's logs. `None` when there
/// is nowhere to put one.
pub fn folder() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("KERF_CRASH_DIR").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(value));
    }
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        var("LOCALAPPDATA").map(|folder| folder.join("Kerf").join("crashes"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Logs/Kerf"))
    } else {
        var("XDG_STATE_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".local/state")))
            .map(|folder| folder.join("kerf").join("crashes"))
    }
}

/// Report every panic and native crash in this process from now on as `component`, in
/// [`folder`]. Only the first call in a process takes effect.
pub fn install(component: &'static str) {
    if let Some(folder) = folder() {
        install_in(component, folder);
    }
}

/// [`install`] into `folder`.
pub fn install_in(component: &'static str, folder: PathBuf) {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let earlier = std::panic::take_hook();
        let hook_folder = folder.clone();
        std::panic::set_hook(Box::new(move |info| {
            // A report that cannot be written must not hide the panic itself.
            if write_report(&hook_folder, component, info).is_ok() {
                native::panic_reported();
            }
            earlier(info);
        }));
        native::install(component, &folder);
        // The native handler writes no more than one report and cannot prune; the next start
        // does.
        prune(&folder);
    });
}

/// Report `message` now as what ends the process, as Qt's `qFatal` does before it aborts; the
/// crash that follows writes no second report. On Windows Qt's abort is a fail-fast that no
/// handler in the process sees, so this is its only report. Returns where it went; nothing is
/// written before [`install`], or after another report of a native crash.
pub fn fatal(message: &str) -> Option<PathBuf> {
    native::fatal(message)
}

/// The reports in `folder`, newest first.
pub fn reports(folder: &Path) -> Vec<Report> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<Report> = entries
        .flatten()
        .filter_map(|entry| read_report(&entry.path()))
        .collect();
    found.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then_with(|| b.path.cmp(&a.path)));
    found
}

/// The text of a report.
pub fn report_text(
    component: &str,
    at_ms: u128,
    thread: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> String {
    format!(
        "Kerf crash report\n\
         component: {component}\n\
         version: {}\n\
         os: {} {}\n\
         at_ms: {at_ms}\n\
         thread: {thread}\n\
         location: {location}\n\
         message: {}\n\
         \n\
         {message}\n\
         \n\
         backtrace:\n\
         {backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        message.lines().next().unwrap_or(""),
    )
}

fn write_report(folder: &Path, component: &str, info: &PanicHookInfo<'_>) -> std::io::Result<()> {
    let at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_owned());
    let location = info.location().map_or_else(
        || "unknown".to_owned(),
        |at| format!("{}:{}:{}", at.file(), at.line(), at.column()),
    );
    let current = std::thread::current();
    let thread = current.name().unwrap_or("unnamed");
    let backtrace = Backtrace::force_capture().to_string();
    let text = report_text(component, at_ms, thread, &message, &location, &backtrace);
    fs::create_dir_all(folder)?;
    let path = folder.join(format!("{component}-{at_ms}-{}.txt", std::process::id()));
    // Two panics in the same millisecond in one process keep the first; the second is almost
    // always the same one again on another thread.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(text.as_bytes())?;
    prune(folder);
    Ok(())
}

/// Keep the newest [`KEPT`] reports.
fn prune(folder: &Path) {
    for old in reports(folder).iter().skip(KEPT) {
        let _ = fs::remove_file(&old.path);
    }
}

fn read_report(path: &Path) -> Option<Report> {
    if path.extension().is_none_or(|extension| extension != "txt") {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? != "Kerf crash report" {
        return None;
    }
    let mut report = Report {
        path: path.to_owned(),
        component: String::new(),
        at_ms: 0,
        message: String::new(),
    };
    for line in lines.take_while(|line| !line.is_empty()) {
        match line.split_once(": ") {
            Some(("component", value)) => report.component = value.to_owned(),
            Some(("at_ms", value)) => report.at_ms = value.parse().ok()?,
            Some(("message", value)) => report.message = value.to_owned(),
            _ => {}
        }
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reads_back_what_was_written() {
        let dir = tempfile::tempdir().expect("a folder");
        let text = report_text("kerfd", 42, "main", "boom\nmore", "src/x.rs:1:2", "0: here");
        assert!(text.contains("location: src/x.rs:1:2"), "{text}");
        assert!(text.contains("backtrace:\n0: here"), "{text}");
        fs::write(dir.path().join("kerfd-42-1.txt"), &text).expect("written");
        fs::write(dir.path().join("notes.txt"), "not a report").expect("written");
        assert_eq!(
            reports(dir.path()),
            [Report {
                path: dir.path().join("kerfd-42-1.txt"),
                component: "kerfd".into(),
                at_ms: 42,
                message: "boom".into(),
            }]
        );
    }

    #[test]
    fn the_folder_keeps_the_newest() {
        let dir = tempfile::tempdir().expect("a folder");
        for at in 0..KEPT as u128 + 3 {
            let text = report_text("kerf", at, "main", "boom", "here", "");
            fs::write(dir.path().join(format!("kerf-{at}-1.txt")), text).expect("written");
        }
        prune(dir.path());
        let kept = reports(dir.path());
        assert_eq!(kept.len(), KEPT);
        assert_eq!(kept.first().map(|r| r.at_ms), Some(KEPT as u128 + 2));
        assert_eq!(kept.last().map(|r| r.at_ms), Some(3));
    }
}
