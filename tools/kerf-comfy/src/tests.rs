//! A making from its request to its file, against a server that is a
//! thread here.

use super::*;
use request::Request;
use server::Address;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// What a request to the server was: `GET /queue`, its headers in small
/// letters, and its body.
#[derive(Clone, Debug)]
struct Asked {
    line: String,
    head: String,
    body: Vec<u8>,
}

/// Read one request from `stream`.
fn asked(stream: &mut TcpStream) -> Asked {
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
        .to_owned();
    Asked {
        line: line.trim_end_matches(" HTTP/1.1").to_owned(),
        head,
        body: bytes[end..end + length].to_vec(),
    }
}

/// A server that answers each request with what `answer` gives for it, a
/// status and a body, and keeps what it was asked.
fn serve(
    answer: impl Fn(&Asked, usize) -> (u16, Vec<u8>) + Send + 'static,
) -> (Server, Arc<Mutex<Vec<Asked>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let kept = Arc::new(Mutex::new(Vec::new()));
    let keeping = kept.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let asked = asked(&mut stream);
            let before = {
                let mut kept = keeping.lock().unwrap();
                let before = kept
                    .iter()
                    .filter(|earlier: &&Asked| earlier.line == asked.line)
                    .count();
                kept.push(asked.clone());
                before
            };
            let (status, body) = answer(&asked, before);
            let head = format!(
                "HTTP/1.1 {status} Answered\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream
                .write_all(head.as_bytes())
                .and_then(|()| stream.write_all(&body));
        }
    });
    let address = Address {
        url: url.trim_end_matches('/').to_owned(),
        key: Some("a key".to_owned()),
        header: None,
    };
    (Server::new(address), kept)
}

fn json_answer(value: serde_json::Value) -> (u16, Vec<u8>) {
    (200, value.to_string().into_bytes())
}

/// ComfyUI as this program meets it, for a job that is running the first
/// time it is asked after and has saved a video the second.
fn comfy(asked: &Asked, before: usize) -> (u16, Vec<u8>) {
    match asked.line.as_str() {
        "POST /upload/image" => {
            let body = String::from_utf8_lossy(&asked.body);
            let name = body
                .split("filename=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            json_answer(json!({"name": name, "subfolder": "", "type": "input"}))
        }
        "GET /object_info/GetVideoComponents" => json_answer(
            json!({"GetVideoComponents": {"input": {"required": {"video": ["VIDEO", {}]}}}}),
        ),
        "GET /object_info/WanVaceToVideo" => json_answer(json!({"WanVaceToVideo": {"input": {
            "required": {"positive": [], "negative": [], "vae": [], "width": [], "height": [], "length": [], "batch_size": [], "strength": []},
            "optional": {"control_video": [], "control_masks": [], "reference_image": []}
        }}})),
        "POST /prompt" => {
            json_answer(json!({"prompt_id": "job-1", "number": 3, "node_errors": {}}))
        }
        "GET /history/job-1" if before == 0 => json_answer(json!({})),
        "GET /history/job-1" => json_answer(json!({"job-1": {
            "outputs": {
                "8": {"images": [{"filename": "kerf-ref.png", "subfolder": "", "type": "input"}]},
                "16": {"images": [{"filename": "kerf_00001_.mp4", "subfolder": "video", "type": "output"}], "animated": [true]}
            },
            "status": {"status_str": "success", "completed": true, "messages": []}
        }})),
        "GET /queue" => {
            json_answer(json!({"queue_running": [[3, "job-1", {}, {}, []]], "queue_pending": []}))
        }
        "GET /view?filename=kerf_00001_.mp4&subfolder=video&type=output" => {
            (200, b"a shot".to_vec())
        }
        other => (404, format!("nothing answers {other}").into_bytes()),
    }
}

/// A request in `folder` that gives a reference picture and no guide, so no
/// FFmpeg is needed to make it.
fn request(folder: &Path, more: &str) -> Request {
    std::fs::write(folder.join("boat.png"), "a boat").unwrap();
    let text = json!({
        "kerf-request": 1,
        "model": "vace",
        "prompt": "a boat leaves, at dawn",
        "guide": null,
        "first": folder.join("boat.png"),
        "last": null,
        "references": [folder.join("boat.png")],
        "seconds": 4.0,
        "size": [1920, 1080],
        "seed": 7,
        "more": serde_json::from_str::<serde_json::Value>(more).unwrap(),
        "out": folder.join(".making-001.mp4"),
    });
    Request::parse(&text.to_string()).unwrap()
}

fn patience() -> Patience {
    Patience {
        most: Duration::from_secs(30),
        pause: Duration::from_millis(5),
    }
}

#[test]
fn a_request_becomes_a_job_and_the_job_the_shots_file() {
    let folder = tempfile::tempdir().unwrap();
    let request = request(
        folder.path(),
        r#"{"steps": "30", "strength": "0.6", "mood": "calm"}"#,
    );
    let (server, kept) = serve(comfy);
    let mut said = Vec::new();
    make(
        &request,
        folder.path(),
        &Workflow::named("wan-vace").unwrap(),
        &server,
        &Tools::find(),
        &patience(),
        &mut |line| said.push(line.to_owned()),
    )
    .unwrap();
    assert_eq!(std::fs::read(&request.out).unwrap(), b"a shot");

    let kept = kept.lock().unwrap();
    // Every request carries the key.
    assert!(
        kept.iter()
            .all(|asked| asked.head.contains("x-api-key: a key")),
        "{kept:?}"
    );
    // The reference went up under a name made of what it holds.
    let upload = kept
        .iter()
        .find(|asked| asked.line == "POST /upload/image")
        .unwrap();
    let named = format!("kerf-{}.png", hash(b"a boat"));
    assert!(String::from_utf8_lossy(&upload.body).contains(&format!("filename=\"{named}\"")));
    assert!(upload.body.windows(6).any(|window| window == b"a boat"));

    // The graph queued: no guide, so nothing loads one; the reference's name; what was asked.
    let queued = kept
        .iter()
        .find(|asked| asked.line == "POST /prompt")
        .unwrap();
    let queued: serde_json::Value = serde_json::from_slice(&queued.body).unwrap();
    let graph = &queued["prompt"];
    assert!(graph.get("6").is_none() && graph.get("7").is_none());
    assert_eq!(graph["8"]["inputs"]["image"], named);
    let vace = &graph["9"]["inputs"];
    assert_eq!(vace["reference_image"], json!(["8", 0]));
    assert!(vace.get("control_video").is_none());
    // 1920x1080 is past what the model makes, and four seconds is 65 of its frames.
    assert_eq!(
        (&vace["width"], &vace["height"], &vace["length"]),
        (&json!(848), &json!(480), &json!(65))
    );
    assert_eq!(vace["strength"], json!(0.6));
    assert_eq!(graph["12"]["inputs"]["steps"], json!(30));
    assert_eq!(graph["12"]["inputs"]["seed"], json!(7));
    assert_eq!(graph["4"]["inputs"]["text"], "a boat leaves, at dawn");
    // The same graph is kept beside the request.
    let beside: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.path().join(GRAPH_SENT)).unwrap()).unwrap();
    assert_eq!(&beside, graph);

    let said = said.join("\n");
    assert!(said.contains("1920x1080 was asked"), "{said}");
    assert!(said.contains("sent the ref, 1 kB"), "{said}");
    // The workflow has no blank for a first frame, nor for a mood.
    assert!(
        said.contains("reads no `mood`, `first`: left out"),
        "{said}"
    );
    assert!(
        said.contains("for 65 frames of 848x480 at 16 a second, seed 7: job job-1"),
        "{said}"
    );
    assert!(said.contains("running, 0 s"), "{said}");
    assert!(said.contains("made in 0 s"), "{said}");
}

#[test]
fn what_the_server_refuses_or_fails_is_said_and_leaves_no_file() {
    let folder = tempfile::tempdir().unwrap();
    let request = request(folder.path(), "{}");
    let workflow = Workflow::named("wan-vace").unwrap();
    let run = |server: &Server| {
        make(
            &request,
            folder.path(),
            &workflow,
            server,
            &Tools::find(),
            &patience(),
            &mut |_| {},
        )
        .unwrap_err()
    };

    // A graph the server will not take.
    let (server, _) = serve(|asked, before| {
        match asked.line.as_str() {
        "POST /prompt" => (
            400,
            json!({
                "error": {"message": "Prompt outputs failed validation"},
                "node_errors": {"1": {"class_type": "UNETLoader", "errors": [
                    {"message": "Value not in list", "details": "unet_name: 'wan2.1_vace_1.3B_fp16.safetensors' not in []"}
                ]}}
            })
            .to_string()
            .into_bytes(),
        ),
        _ => comfy(asked, before),
    }
    });
    let refused = run(&server);
    assert!(
        refused.contains("the server refused the workflow"),
        "{refused}"
    );
    assert!(
        refused.contains("node 1 (UNETLoader): Value not in list unet_name"),
        "{refused}"
    );

    // A job that ends badly.
    let (server, _) = serve(|asked, before| match asked.line.as_str() {
        "GET /history/job-1" => json_answer(json!({"job-1": {"outputs": {}, "status": {
            "status_str": "error",
            "messages": [["execution_error", {"node_id": "12", "node_type": "KSampler", "exception_message": "out of memory"}]]
        }}})),
        _ => comfy(asked, before),
    });
    assert_eq!(
        run(&server),
        "the server could not make it: KSampler (node 12): out of memory"
    );

    // A job that saves something else than the shot's kind of file.
    let (server, _) = serve(|asked, before| match asked.line.as_str() {
        "GET /history/job-1" => json_answer(json!({"job-1": {
            "outputs": {"16": {"images": [{"filename": "kerf_00001_.png", "subfolder": "", "type": "output"}]}},
            "status": {"status_str": "success", "completed": true, "messages": []}
        }})),
        _ => comfy(asked, before),
    });
    assert_eq!(
        run(&server),
        "the job made kerf_00001_.png, and the file asked for is a .mp4"
    );

    // A key the server does not take.
    let (server, _) = serve(|_, _| (401, b"<html>401 Authorization Required</html>".to_vec()));
    let unkeyed = run(&server);
    assert!(unkeyed.contains("refused the key (HTTP 401)"), "{unkeyed}");
    assert!(
        unkeyed.contains("sent as the header `X-API-Key`"),
        "{unkeyed}"
    );

    // A job the server forgets is not waited for for ever.
    let (server, kept) = serve(|asked, before| match asked.line.as_str() {
        "GET /history/job-1" => json_answer(json!({})),
        "GET /queue" => json_answer(json!({"queue_running": [], "queue_pending": []})),
        _ => comfy(asked, before),
    });
    assert!(run(&server).contains("no longer knows job job-1"));
    let asked_after = kept
        .lock()
        .unwrap()
        .iter()
        .filter(|asked| asked.line == "GET /queue")
        .count();
    assert_eq!(asked_after, TRIES as usize + 1);

    assert!(!request.out.exists());
}

#[test]
fn a_job_waited_for_too_long_is_taken_back() {
    let folder = tempfile::tempdir().unwrap();
    let request = request(folder.path(), "{}");
    let (server, kept) = serve(|asked, before| match asked.line.as_str() {
        "GET /history/job-1" => json_answer(json!({})),
        "GET /queue" => json_answer(
            json!({"queue_running": [], "queue_pending": [[4, "other", {}], [5, "job-1", {}]]}),
        ),
        "POST /queue" | "POST /interrupt" => json_answer(json!({})),
        _ => comfy(asked, before),
    });
    let mut said = Vec::new();
    let late = make(
        &request,
        folder.path(),
        &Workflow::named("wan-vace").unwrap(),
        &server,
        &Tools::find(),
        &Patience {
            most: Duration::from_millis(60),
            pause: Duration::from_millis(5),
        },
        &mut |line| said.push(line.to_owned()),
    )
    .unwrap_err();
    assert!(
        late.contains("gave the job up") && late.contains("COMFY_TIMEOUT"),
        "{late}"
    );
    assert!(
        said.iter().any(|line| line.starts_with("waiting, 1 ahead")),
        "{said:?}"
    );
    let kept = kept.lock().unwrap();
    let body = |line: &str| -> serde_json::Value {
        let asked = kept.iter().find(|asked| asked.line == line).unwrap();
        serde_json::from_slice(&asked.body).unwrap()
    };
    // Out of the queue, and stopped only if it is the one running.
    assert_eq!(body("POST /queue"), json!({"delete": ["job-1"]}));
    assert_eq!(body("POST /interrupt"), json!({"prompt_id": "job-1"}));
}
