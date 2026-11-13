//! The program itself, asked to end as Kerf asks a making that is cancelled:
//! it takes its job back from the server before it goes.
#![cfg(unix)]

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// One request to the server: `POST /prompt`, and its body.
fn asked(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "a request broke off");
        bytes.extend(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length:")?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(0);
    while bytes.len() < end + length {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "a request's body broke off");
        bytes.extend(&chunk[..read]);
    }
    let line = String::from_utf8_lossy(&bytes[..end])
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches(" HTTP/1.1")
        .to_owned();
    (line, bytes[end..end + length].to_vec())
}

/// A ComfyUI that queues a job and runs it for as long as it is asked after,
/// keeping what it was asked.
fn comfy() -> (String, Arc<Mutex<Vec<(String, Vec<u8>)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let kept = Arc::new(Mutex::new(Vec::new()));
    let keeping = kept.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let (line, body) = asked(&mut stream);
            let answer = match line.as_str() {
                "POST /prompt" => json!({"prompt_id": "job-1", "number": 3, "node_errors": {}}),
                // What the nodes left without a file to read cannot do without.
                "GET /object_info/GetVideoComponents" => json!({"GetVideoComponents": {
                    "input": {"required": {"video": ["VIDEO", {}]}}
                }}),
                "GET /object_info/WanVaceToVideo" => json!({"WanVaceToVideo": {"input": {
                    "required": {"positive": [], "negative": [], "vae": [], "width": [],
                                 "height": [], "length": [], "batch_size": [], "strength": []},
                    "optional": {"control_video": [], "control_masks": [], "reference_image": []}
                }}}),
                "GET /queue" => {
                    json!({"queue_running": [[3, "job-1", {}, {}, []]], "queue_pending": []})
                }
                _ => json!({}),
            }
            .to_string();
            keeping.lock().unwrap().push((line, body));
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 Answered\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                )
                .as_bytes(),
            );
        }
    });
    (url, kept)
}

#[test]
fn the_program_takes_its_job_back_on_sigterm() {
    let folder = tempfile::tempdir().unwrap();
    let request = folder.path().join("request.json");
    // A prompt alone: nothing to send up, and no FFmpeg to fit a guide with.
    std::fs::write(
        &request,
        json!({
            "kerf-request": 1,
            "model": "shot",
            "prompt": "a boat leaves, at dawn",
            "guide": null, "first": null, "last": null, "references": [],
            "seconds": 2.0, "size": null, "seed": 7, "more": {},
            "out": folder.path().join(".making-001.mp4"),
        })
        .to_string(),
    )
    .unwrap();
    let (url, kept) = comfy();
    let program = Command::new(env!("CARGO_BIN_EXE_kerf-comfy"))
        .args(["--workflow", "shot"])
        .arg(&request)
        .env("COMFY_URL", &url)
        .env_remove("COMFY_API_KEY")
        .env_remove("COMFY_API_HEADER")
        // No server a person has named is read.
        .env("KERF_COMFY", folder.path().join("no-such.json"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("kerf-comfy starts");

    let wait = |what: &dyn Fn() -> bool, why: &str| {
        let started = Instant::now();
        while !what() {
            assert!(started.elapsed() < Duration::from_secs(30), "{why}");
            thread::sleep(Duration::from_millis(10));
        }
    };
    let was_asked = |line: &str| kept.lock().unwrap().iter().any(|(asked, _)| asked == line);
    wait(&|| was_asked("POST /prompt"), "the job was never queued");

    // What Kerf sends a making that is cancelled.
    let signalled = Command::new("kill")
        .args(["-TERM", &program.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(signalled.success());
    let asked_at = Instant::now();
    let output = program.wait_with_output().expect("kerf-comfy ends");
    // Well inside the time Kerf gives a program to end in.
    assert!(
        asked_at.elapsed() < Duration::from_secs(4),
        "{:?}",
        asked_at.elapsed()
    );
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    // Once: a making's progress shows each line the program says.
    assert_eq!(
        said.matches("asked to stop: took job job-1 back from the server")
            .count(),
        1,
        "{said}"
    );

    let kept = kept.lock().unwrap();
    let body = |line: &str| -> Value {
        let (_, body) = kept
            .iter()
            .find(|(asked, _)| asked == line)
            .unwrap_or_else(|| panic!("the server was never asked {line}: {kept:?}"));
        serde_json::from_slice(body).unwrap()
    };
    assert_eq!(body("POST /queue"), json!({"delete": ["job-1"]}));
    assert_eq!(body("POST /interrupt"), json!({"prompt_id": "job-1"}));
    assert!(!folder.path().join(".making-001.mp4").exists());
}
