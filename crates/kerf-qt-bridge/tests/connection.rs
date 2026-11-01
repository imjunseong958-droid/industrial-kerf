//! The editor's C ABI reaching a real kerfd that it starts itself, over
//! each platform's local transport: a Unix socket, or a named pipe on
//! Windows.
//!
//! Needs kerfd built beside this test: `cargo build -p kerf-runtime --bin kerfd`.

use kerf_qt_bridge::{
    kerf_bridge_free, kerf_bridge_new, kerf_bridge_poll, kerf_bridge_send, kerf_bridge_string_free,
};
use kerf_runtime::RuntimeClient;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn kerfd() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(format!("kerfd{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "build kerfd first: cargo build -p kerf-runtime --bin kerfd"
    );
    path
}

/// A short project folder, removed at the end, which ends its daemon.
struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("kq-connect-{:x}", stamp % 0xffff_ffff));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The editor's side: one bridge and the events it has sent.
struct Editor {
    bridge: *mut c_void,
    events: Vec<Value>,
}

impl Editor {
    fn open(directory: &Path) -> Self {
        let directory = CString::new(directory.to_string_lossy().as_ref()).unwrap();
        let binary = CString::new(kerfd().to_string_lossy().as_ref()).unwrap();
        // SAFETY: both strings live for the call.
        let bridge = unsafe { kerf_bridge_new(directory.as_ptr(), binary.as_ptr()) };
        assert!(!bridge.is_null());
        Self {
            bridge,
            events: Vec::new(),
        }
    }

    fn send(&self, command: Value) {
        let text = CString::new(command.to_string()).unwrap();
        // SAFETY: the bridge is live and only this thread uses it.
        assert!(unsafe { kerf_bridge_send(self.bridge, text.as_ptr()) });
    }

    /// The first event that `matches`, among those already sent or the
    /// next to come within 20 seconds. The runtime and media workers send
    /// theirs in either order.
    fn expect(&mut self, what: &str, matches: impl Fn(&Value) -> bool) -> Value {
        if let Some(event) = self.events.iter().find(|event| matches(event)) {
            return event.clone();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            // SAFETY: the bridge is live and only this thread uses it.
            let pointer = unsafe { kerf_bridge_poll(self.bridge) };
            if pointer.is_null() {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            let event = take(pointer);
            self.events.push(event.clone());
            if matches(&event) {
                return event;
            }
        }
        let recent = &self.events[self.events.len().saturating_sub(8)..];
        panic!("no event for {what}; recent events: {recent:?}");
    }

    fn state(&mut self, revision: u64) -> Value {
        self.expect(&format!("revision {revision}"), |event| {
            event["type"] == "state" && event["snapshot"]["revision"] == revision
        })["snapshot"]
            .clone()
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        // SAFETY: freed exactly once, by its only user.
        unsafe { kerf_bridge_free(self.bridge) };
    }
}

/// Take an owned JSON string from the bridge.
fn take(pointer: *mut c_char) -> Value {
    // SAFETY: the bridge returned a live, owned, NUL-terminated string.
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: returned exactly once.
    unsafe { kerf_bridge_string_free(pointer) };
    serde_json::from_str(&text).unwrap()
}

fn markers(snapshot: &Value) -> Vec<Value> {
    snapshot["timeline"]["markers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Print where a crash happened, on whichever thread: a Windows access
/// violation ends the process with no word of where it was.
#[cfg(windows)]
fn report_crashes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows_sys::Win32::Foundation::EXCEPTION_ACCESS_VIOLATION;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    };

    static REPORTED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "system" fn report(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: Windows passes a live exception record for this call.
        let record = unsafe { &*(*info).ExceptionRecord };
        if record.ExceptionCode == EXCEPTION_ACCESS_VIOLATION
            && REPORTED.fetch_add(1, Ordering::SeqCst) < 3
        {
            let [kind, address, ..] = record.ExceptionInformation;
            eprintln!(
                "access violation at {:?} ({} {address:#x}) on thread {:?}\n{}",
                record.ExceptionAddress,
                match kind {
                    0 => "reading",
                    1 => "writing",
                    _ => "executing",
                },
                std::thread::current().name(),
                std::backtrace::Backtrace::force_capture(),
            );
        }
        EXCEPTION_CONTINUE_SEARCH
    }

    // SAFETY: `report` lives for the whole process and only reads.
    unsafe { AddVectoredExceptionHandler(1, Some(report)) };
}

#[test]
fn the_editor_starts_kerfd_and_shares_the_project_with_another_client() {
    #[cfg(windows)]
    report_crashes();
    let project = Project::new();
    // No daemon runs yet: the editor starts one and waits for it.
    let mut editor = Editor::open(&project.0);
    editor.expect("kerfd to be ready", |event| {
        event["type"] == "connection" && event["status"] == "ready"
    });
    assert!(markers(&editor.state(0)).is_empty());

    // Another client, as the CLI or the agent's MCP server would be, edits
    // through the same daemon, and the editor follows.
    let client = RuntimeClient::new(&project.0).unwrap();
    let edited = client
        .apply_script_request("mcp", "external", "marker add at=1s name=\"external\"", 0)
        .unwrap();
    assert_eq!(edited.revision, 1);
    let followed = editor.state(1);
    assert_eq!(markers(&followed).len(), 1, "{followed}");

    // The editor's own edit reaches the other client.
    editor.send(json!({"type": "undo", "revision": 1}));
    let undone = editor.state(2);
    assert!(markers(&undone).is_empty(), "{undone}");
    let seen = client.get_state().unwrap();
    assert_eq!(seen.revision, 2);
    assert!(!seen.document.contains("external"), "{}", seen.document);
}
