//! The bridge's own surface: the commands it refuses at once, the folder it names, and the
//! editor's `--serve-mcp` argument.
//!
//! Needs kerfd built beside this test: `cargo build -p kerf-runtime --bin kerfd`.

use kerf_bridge::{serve_mcp, Bridge, SERVE_MCP};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        let directory = std::env::temp_dir().join(format!("kq-api-{:x}", stamp % 0xffff_ffff));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn what_is_no_command_or_cannot_be_taken_is_refused_and_the_rest_goes_on() {
    let project = Project::new();
    let mut bridge = Bridge::open(&project.0, &kerfd());
    assert_eq!(bridge.directory(), project.0);
    for refused in [
        json!("refresh"),
        json!({"type": "no_such_command"}),
        json!({"type": "edit", "ops": "not a list", "revision": 0}),
        json!({"type": "seek", "position": -1.0}),
        json!({"type": "agent", "text": "   "}),
        json!({"type": "agent_versions", "text": "Tighten the open", "count": 0}),
    ] {
        assert!(!bridge.send(&refused), "took {refused}");
    }
    assert!(bridge.send(&json!({"type": "refresh"})));
    assert!(bridge.send(&json!({"type": "seek", "position": 0.5, "scrub": true})));

    // The bridge still reaches kerfd.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut events = Vec::new();
    let state = loop {
        if let Some(event) = bridge.poll() {
            if event["type"] == "state" {
                break event;
            }
            events.push(event);
        } else {
            assert!(Instant::now() < deadline, "no state; events: {events:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    assert_eq!(state["snapshot"]["revision"], 0, "{state}");
    // No `scope_signal` was asked for, so there is no signal frame to take.
    assert!(bridge.signal().is_none());
}

#[test]
fn only_the_serve_argument_has_the_editor_serve_its_tools() {
    let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    assert_eq!(SERVE_MCP, "--serve-mcp");
    assert_eq!(serve_mcp(&args(&[])), None);
    assert_eq!(serve_mcp(&args(&["--dir", "/work/film"])), None);
    assert_eq!(serve_mcp(&args(&["/work/film", SERVE_MCP])), None);
    // The argument without `--dir <project>` ends the editor with a failure; it does not
    // start the editor.
    assert_eq!(serve_mcp(&args(&[SERVE_MCP])), Some(1));
    assert_eq!(serve_mcp(&args(&[SERVE_MCP, "--dir"])), Some(1));
}

#[test]
fn the_editor_runs_the_ffmpeg_kerfd_runs() {
    let found = kerf_bridge::ffmpeg_bin();
    // The engine's own, found once: the very same text.
    assert!(std::ptr::eq(
        found,
        kerf_engine::ffmpeg::resolve_ffmpeg_bin()
    ));
    // What is found runs; else it is plain `ffmpeg`, to say so as it fails.
    let runs = std::process::Command::new(found)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    assert!(runs || found == "ffmpeg", "{found}");
}

#[test]
fn what_is_answered_at_once_never_holds_the_caller_while_the_events_wait() {
    // The caller is the one that takes the events: an answer waiting for room in their queue
    // on its thread would wait for ever. More answers than the queue holds are asked for here
    // with none taken, from a thread of their own so a block fails the test instead of
    // hanging it.
    let project = Project::new();
    let bridge = Bridge::open(&project.0, &kerfd());
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bridge = bridge;
        for _ in 0..300 {
            bridge.send(&json!({"type": "models"}));
            bridge.send(&json!({"type": "conversations", "action": "list"}));
        }
        let _ = done.send(bridge);
    });
    let mut bridge = finished
        .recv_timeout(Duration::from_secs(30))
        .expect("a send waited while the events were not taken");
    // Taken, every models answer still comes.
    let mut models = 0;
    let deadline = Instant::now() + Duration::from_secs(60);
    while models < 300 && Instant::now() < deadline {
        match bridge.poll() {
            Some(event) if event["type"] == "models" => models += 1,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert_eq!(models, 300);
}
