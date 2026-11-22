//! The editor's bridge reaching a real kerfd that it starts itself, over
//! each platform's local transport: a Unix socket, or a named pipe on
//! Windows.
//!
//! Needs kerfd built beside this test: `cargo build -p kerf-runtime --bin kerfd`.

use kerf_bridge::Bridge;
use kerf_runtime::RuntimeClient;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Send `command` as the editor writes it, in JSON. One that does not read as a command is
/// refused, as one the bridge cannot take is.
fn send(bridge: &mut Bridge, command: &Value) -> bool {
    serde_json::from_value::<kerf_bridge::Command>(command.clone())
        .is_ok_and(|command| bridge.send(command))
}

/// The next event, in JSON, as the editor reads it.
fn poll(bridge: &mut Bridge) -> Option<Value> {
    bridge
        .poll()
        .map(|event| serde_json::to_value(event).expect("an event writes as JSON"))
}

fn kerfd() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/debug")
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
    bridge: Bridge,
    events: Vec<Value>,
}

impl Editor {
    fn open(directory: &Path) -> Self {
        Self {
            bridge: Bridge::open(directory, &kerfd()),
            events: Vec::new(),
        }
    }

    fn send(&mut self, command: Value) {
        assert!(send(&mut self.bridge, &command));
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
            let Some(event) = poll(&mut self.bridge) else {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
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

fn markers(snapshot: &Value) -> Vec<Value> {
    snapshot["timeline"]["markers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Access violations in this process, handled or not. Each audio thread
/// once read an audio DLL Windows had unloaded; that was only sometimes
/// fatal, but it showed every time.
#[cfg(windows)]
#[allow(unsafe_code)]
fn count_access_violations() -> &'static std::sync::atomic::AtomicUsize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows_sys::Win32::Foundation::EXCEPTION_ACCESS_VIOLATION;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    };

    static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

    // SAFETY: unsafe because it dereferences `info`; only Windows calls it, as a
    // vectored exception handler, with live exception pointers.
    unsafe extern "system" fn count(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: Windows passes a live exception record for this call.
        let record = unsafe { &*(*info).ExceptionRecord };
        if record.ExceptionCode == EXCEPTION_ACCESS_VIOLATION {
            let [kind, address, ..] = record.ExceptionInformation;
            eprintln!(
                "access violation at {:?} ({} {address:#x}) on thread {:?}",
                record.ExceptionAddress,
                if kind == 1 { "writing" } else { "reading" },
                std::thread::current().name(),
            );
            VIOLATIONS.fetch_add(1, Ordering::SeqCst);
        }
        EXCEPTION_CONTINUE_SEARCH
    }

    // SAFETY: `count` lives for the whole process and only reads.
    unsafe { AddVectoredExceptionHandler(1, Some(count)) };
    &VIOLATIONS
}

#[test]
fn the_editor_starts_kerfd_and_shares_the_project_with_another_client() {
    #[cfg(windows)]
    let violations = count_access_violations();
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

    // Edits replace the editor's playback session, and its audio thread
    // with it.
    #[cfg(windows)]
    assert_eq!(violations.load(std::sync::atomic::Ordering::SeqCst), 0);
}
