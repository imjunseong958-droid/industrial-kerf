//! Real-process proof for the project-scoped cutd boundary.

#![cfg(unix)]

use cut_runtime::{EndpointAddress, RuntimeClient, MAX_REQUEST_BYTES};
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn project_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("cutd-ipc-{label}-{stamp}"))
}

fn start_daemon(directory: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_cutd"))
        .arg("--dir")
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("CUTD_TEST_DROP_RESPONSE_KEY", "lost-response")
        .spawn()
        .expect("spawn cutd")
}

fn wait_until_ready(client: &RuntimeClient) {
    for _ in 0..250 {
        if client.get_state().is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("cutd did not become ready at {:?}", client.endpoint());
}

fn has_marker(project: &serde_json::Value, id: &str) -> bool {
    project["markers"]
        .as_array()
        .is_some_and(|markers| markers.iter().any(|marker| marker["id"] == id))
}

fn stop_daemon(mut daemon: Child) {
    let _ = daemon.kill();
    let _ = daemon.wait();
}

#[test]
fn two_clients_share_one_daemon_and_recover_after_lost_response_and_restart() {
    let directory = project_directory("shared");
    fs::create_dir_all(&directory).unwrap();
    let daemon = start_daemon(&directory);
    let left = RuntimeClient::new(&directory).unwrap();
    let right = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&left);
    assert_eq!(left.endpoint(), right.endpoint());

    // Both clients submit against the same revision at the same time.  The
    // mailbox admits exactly one and returns a structured revision conflict
    // to the other.
    let barrier = Arc::new(Barrier::new(3));
    let left_barrier = barrier.clone();
    let left_client = left.clone();
    let left_thread = thread::spawn(move || {
        left_barrier.wait();
        left_client.apply_request(
            "ui",
            "simultaneous-left",
            json!({"type":"add_marker","marker":{"id":"left","time":0}}),
            0,
        )
    });
    let right_barrier = barrier.clone();
    let right_client = right.clone();
    let right_thread = thread::spawn(move || {
        right_barrier.wait();
        right_client.apply_request(
            "mcp",
            "simultaneous-right",
            json!({"type":"add_marker","marker":{"id":"right","time":0}}),
            0,
        )
    });
    barrier.wait();
    let left_result = left_thread.join().unwrap();
    let right_result = right_thread.join().unwrap();
    assert_eq!(left_result.is_ok() as u8 + right_result.is_ok() as u8, 1);
    let conflict = left_result
        .err()
        .or_else(|| right_result.err())
        .expect("one simultaneous request conflicts");
    assert_eq!(conflict.code, "REVISION_CONFLICT");

    let revision_one = left.get_state().unwrap();
    assert_eq!(revision_one.revision, 1);

    // The real daemon commits this edit script and deliberately drops its
    // first response. RuntimeClient must reconnect, read authoritative
    // state, and replay the same stable key inside this single method call.
    let lost_key = "lost-response";
    let replay = right
        .apply_script_request(
            "mcp",
            lost_key,
            "marker add at=12f name=\"lost\" as=lost",
            1,
        )
        .unwrap();
    assert_eq!(replay.revision, 2);
    assert!(has_marker(&replay.project, "lost"));

    // A client has no daemon ownership. Dropping one leaves the other fully
    // usable on the same process.
    drop(left);
    assert_eq!(right.get_state().unwrap().revision, 2);

    // Terminate and explicitly restart the real cutd process. The second
    // client reconnects to the fresh endpoint and reads checkpoint state.
    stop_daemon(daemon);
    let restarted = start_daemon(&directory);
    wait_until_ready(&right);
    let recovered = right.get_state().unwrap();
    assert_eq!(recovered.revision, 2);
    assert!(has_marker(&recovered.project, "lost"));
    let (revision, document) = right.get_document(false).unwrap();
    assert_eq!(revision, 2);
    assert!(
        document.contains("marker lost at=00:00:00:12 name=\"lost\""),
        "{document}"
    );
    stop_daemon(restarted);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn real_daemon_rejects_an_oversized_request_before_reducer_execution() {
    let directory = project_directory("size");
    fs::create_dir_all(&directory).unwrap();
    let daemon = start_daemon(&directory);
    let client = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&client);
    let endpoint = match client.endpoint() {
        EndpointAddress::Unix { path } => path,
        EndpointAddress::WindowsNamedPipe { .. } => panic!("Unix integration test on Windows"),
    };
    let mut socket = UnixStream::connect(endpoint).unwrap();
    let oversized = vec![b'x'; MAX_REQUEST_BYTES + 1];
    socket.write_all(&oversized).unwrap();
    let _ = socket.shutdown(std::net::Shutdown::Write);
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    let response: serde_json::Value = serde_json::from_str(response.trim()).unwrap();
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "REQUEST_TOO_LARGE");
    assert_eq!(client.get_state().unwrap().revision, 0);
    stop_daemon(daemon);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_v2_project_round_trips_through_cutd_on_the_kernel() {
    let directory = project_directory("kernel");
    fs::create_dir_all(&directory).unwrap();
    let legacy = serde_json::to_vec_pretty(&json!({
        "version": 2,
        "name": "Round Trip",
        "fps": 24,
        "width": 1920,
        "height": 1080,
        "sources": [{"id": "src-1", "path": "/media/a.mov", "duration": 60, "width": 1920, "height": 1080, "hasAudio": true}],
        "tracks": [{"id": "v1", "type": "video", "clips": [
            {"id": "clip-a", "sourceId": "src-1", "in": 0, "out": 4, "timelineStart": 0},
            {"id": "clip-b", "sourceId": "src-1", "in": 10, "out": 12, "timelineStart": 4}
        ]}],
        "overlays": [], "gaps": [], "transitions": [], "effects": [], "markers": []
    }))
    .unwrap();
    fs::write(directory.join("project.cut.json"), &legacy).unwrap();

    let daemon = start_daemon(&directory);
    let client = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&client);
    let opened = client.get_state().unwrap();
    assert_eq!(opened.revision, 0);
    assert_eq!(opened.source_version, 3);
    assert!(opened
        .import_warnings
        .iter()
        .any(|warning| warning.contains("imported project.cut.json")));
    let imported = fs::read_to_string(directory.join("project.cut")).unwrap();
    assert!(imported.contains("clip clip-a media=src-1"), "{imported}");
    // The embedded sound became a linked clip on its own track.
    assert!(imported.contains("clip clip-a_a media=src-1"), "{imported}");

    // A ripple trim keeps picture and sound in sync; the clip after moves.
    let trimmed = client
        .apply_script_request("ui", "trim-a", "trim clip-a out -12f ripple", 0)
        .unwrap();
    assert_eq!(trimmed.revision, 1);
    assert!(trimmed.can_undo);
    let (_, after_trim) = client.get_document(false).unwrap();
    assert!(after_trim.contains("clip clip-b media=src-1 src=00:00:10:00 at=00:00:03:12"));
    assert_eq!(
        fs::read_to_string(directory.join("project.cut")).unwrap(),
        after_trim
    );

    // A v2 command from an older client still edits the model, and deleting
    // a clip retires its id and its sound's.
    let deleted = client
        .apply_request(
            "ui",
            "delete-b",
            json!({"type": "remove_clip", "clipId": "clip-b"}),
            1,
        )
        .unwrap();
    assert_eq!(deleted.revision, 2);
    let undone = client.undo_request("ui", "undo-delete", 2).unwrap();
    assert_eq!(undone.revision, 3);
    assert!(undone.can_redo);
    let (_, after_undo) = client.get_document(false).unwrap();
    assert_eq!(after_undo, after_trim);

    stop_daemon(daemon);
    let restarted = start_daemon(&directory);
    wait_until_ready(&client);
    let recovered = client.get_state().unwrap();
    assert_eq!(recovered.revision, 3);
    assert!(recovered.can_undo && recovered.can_redo);
    let (_, recovered_document) = client.get_document(false).unwrap();
    assert_eq!(recovered_document, after_trim);
    assert_eq!(
        fs::read_to_string(directory.join("project.cut")).unwrap(),
        after_trim
    );
    // History survived the restart: redo the delete, then undo the trim.
    let redone = client.redo_request("ui", "redo-delete", 3).unwrap();
    assert_eq!(redone.revision, 4);
    let (_, after_redo) = client.get_document(false).unwrap();
    assert!(!after_redo.contains("clip clip-b "), "{after_redo}");
    assert!(after_redo.contains("retired="), "{after_redo}");
    assert!(after_redo.contains("clip-b"), "{after_redo}");
    // A retired id cannot name a new clip.
    let reused = client
        .apply_script_request(
            "ui",
            "reuse-b",
            "insert media=src-1 src=0f dur=24f at=10s on=v1 as=clip-b",
            4,
        )
        .unwrap_err();
    assert_eq!(reused.code, "EDIT_REJECTED");
    assert!(
        reused.message.contains("never reused"),
        "{}",
        reused.message
    );
    stop_daemon(restarted);
    assert_eq!(
        fs::read(directory.join("project.cut.json")).unwrap(),
        legacy
    );
    let _ = fs::remove_dir_all(&directory);
}
