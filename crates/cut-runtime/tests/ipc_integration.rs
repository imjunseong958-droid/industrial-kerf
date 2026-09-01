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
            json!({"type":"set_metadata","patch":{"left":true}}),
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
            json!({"type":"set_metadata","patch":{"right":true}}),
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

    // The real daemon commits this request and deliberately drops its first
    // response. RuntimeClient must reconnect, read authoritative state, and
    // replay the same stable key inside this single method call.
    let lost_key = "lost-response";
    let replay = right
        .apply_request(
            "mcp",
            lost_key,
            json!({"type":"set_metadata","patch":{"lost":true}}),
            1,
        )
        .unwrap();
    assert_eq!(replay.revision, 2);
    assert_eq!(replay.project["metadata"]["lost"], true);

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
    assert_eq!(recovered.project["metadata"]["lost"], true);
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
