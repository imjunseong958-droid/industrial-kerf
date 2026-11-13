//! A making from its request to its file, against a server that is a
//! thread here.

use super::*;
use request::Request;
use server::Address;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
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
        &AtomicBool::new(false),
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
fn a_signal_reaches_the_workflow_as_a_number_for_each_frame() {
    let folder = tempfile::tempdir().unwrap();
    let mut request = request(folder.path(), "{}");
    // A second each at 0, 1, 1 and 0, and one more signal than the workflow reads.
    request.signals = serde_json::from_value(json!([
        {"name": "swell", "rate": 1.0, "values": [0.0, 1.0, 1.0, 0.0]},
        {"name": "spare", "rate": 1.0, "values": [1.0]},
    ]))
    .unwrap();
    let workflow = Workflow::parse(
        r#"{
            "kerf-comfy": 1, "fps": 16, "frames": {"most": 81, "step": 4}, "size": [832, 480],
            "workflow": {
                "1": {"class_type": "Words", "inputs": {"text": "{prompt}"}},
                "2": {"class_type": "Values", "inputs": {"values": "{signal}", "count": "{frames}"}},
                "3": {"class_type": "Sampler", "inputs": {"words": ["1", 0], "cue": ["2", 0]}}
            }
        }"#,
    )
    .unwrap();
    let (server, kept) = serve(comfy);
    let mut said = Vec::new();
    make(
        &request,
        folder.path(),
        &workflow,
        &server,
        &Tools::find(),
        &patience(),
        &AtomicBool::new(false),
        &mut |line| said.push(line.to_owned()),
    )
    .unwrap();
    let kept = kept.lock().unwrap();
    let queued = kept
        .iter()
        .find(|asked| asked.line == "POST /prompt")
        .unwrap();
    let queued: serde_json::Value = serde_json::from_slice(&queued.body).unwrap();
    let node = &queued["prompt"]["2"]["inputs"];
    // Four seconds is 65 of the model's frames, each with the signal at its time.
    assert_eq!(node["count"], json!(65));
    let values: Vec<&str> = node["values"].as_str().unwrap().split(',').collect();
    assert_eq!(values.len(), 65);
    assert_eq!(
        [values[0], values[8], values[16], values[32], values[40], values[64]],
        ["0", "0.5", "1", "1", "0.5", "0"]
    );
    let said = said.join("\n");
    assert!(
        said.contains("handed signal `swell` as 65 values"),
        "{said}"
    );
    assert!(
        said.contains("reads no `first`, `ref`, `signal2`: left out"),
        "{said}"
    );
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
            &AtomicBool::new(false),
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
        &AtomicBool::new(false),
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

#[test]
fn a_job_is_taken_back_when_the_program_is_asked_to_end() {
    let folder = tempfile::tempdir().unwrap();
    let request = request(folder.path(), "{}");
    // A job that runs for as long as it is asked after.
    let (server, kept) = serve(|asked, before| match asked.line.as_str() {
        "GET /history/job-1" => json_answer(json!({})),
        "POST /queue" | "POST /interrupt" => json_answer(json!({})),
        _ => comfy(asked, before),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let asking = stop.clone();
    // Asked once the job is queued, as a making cancelled in Kerf asks.
    let watching = kept.clone();
    let asker = thread::spawn(move || {
        while !watching
            .lock()
            .unwrap()
            .iter()
            .any(|asked| asked.line == "POST /prompt")
        {
            thread::sleep(Duration::from_millis(2));
        }
        asking.store(true, Ordering::SeqCst);
    });
    let mut said = Vec::new();
    let started = Instant::now();
    let stopped = make(
        &request,
        folder.path(),
        &Workflow::named("wan-vace").unwrap(),
        &server,
        &Tools::find(),
        // A pause far longer than the test: the asking is what ends the wait.
        &Patience {
            most: Duration::from_secs(600),
            pause: Duration::from_secs(60),
        },
        &stop,
        &mut |line| said.push(line.to_owned()),
    )
    .unwrap_err();
    asker.join().unwrap();
    assert_eq!(
        stopped,
        "asked to stop: took job job-1 back from the server"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    // It is the error the program ends with, said once as its last line, and not said before.
    assert!(!said.contains(&stopped), "{said:?}");
    let kept = kept.lock().unwrap();
    let body = |line: &str| -> serde_json::Value {
        let asked = kept.iter().find(|asked| asked.line == line).unwrap();
        serde_json::from_slice(&asked.body).unwrap()
    };
    assert_eq!(body("POST /queue"), json!({"delete": ["job-1"]}));
    assert_eq!(body("POST /interrupt"), json!({"prompt_id": "job-1"}));
    assert!(!request.out.exists());

    // Asked before anything is queued, it queues nothing.
    let (server, kept) = serve(comfy);
    let early = make(
        &request,
        folder.path(),
        &Workflow::named("wan-vace").unwrap(),
        &server,
        &Tools::find(),
        &patience(),
        &AtomicBool::new(true),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(early.contains("asked to stop before"), "{early}");
    assert!(!kept
        .lock()
        .unwrap()
        .iter()
        .any(|asked| asked.line == "POST /prompt"));
}

/// ComfyUI for a job that reads a guide and saves a video of it and a text.
fn reader(asked: &Asked, _before: usize) -> (u16, Vec<u8>) {
    match asked.line.as_str() {
        "POST /upload/image" => {
            json_answer(json!({"name": "kerf-guide.mp4", "subfolder": "", "type": "input"}))
        }
        "POST /prompt" => {
            json_answer(json!({"prompt_id": "job-1", "number": 3, "node_errors": {}}))
        }
        "GET /history/job-1" => json_answer(json!({"job-1": {
            "outputs": {
                "1": {"images": [{"filename": "kerf-guide.mp4", "subfolder": "", "type": "input"}], "animated": [true]},
                "7": {"images": [{"filename": "kerf_00001_.mp4", "subfolder": "video", "type": "output"}], "animated": [true]},
                "8": {"text": ["{'extrinsics': tensor([[[[1.0, 0.0, 0.0, 0.0]]]])}"]},
                "9": {"text": ["{'extrinsics': …}"], "files": [{"filename": "kerf-cameras_00001.txt", "subfolder": "", "type": "output"}]}
            },
            "status": {"status_str": "success", "completed": true, "messages": []}
        }})),
        "GET /view?filename=kerf_00001_.mp4&subfolder=video&type=output" => {
            (200, b"its depth".to_vec())
        }
        "GET /view?filename=kerf-cameras_00001.txt&subfolder=&type=output" => {
            (200, b"its cameras".to_vec())
        }
        other => (404, format!("nothing answers {other}").into_bytes()),
    }
}

/// A guide of ten frames of 322x180 at 12 a second in `folder`, where FFmpeg is installed.
fn guide(folder: &Path) -> Option<std::path::PathBuf> {
    let file = folder.join("footage.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=size=322x180:rate=12")
        .args(["-frames:v", "10", "-pix_fmt", "yuv420p"])
        .arg(&file)
        .status();
    matches!(made, Ok(status) if status.success()).then_some(file)
}

#[test]
fn a_file_made_of_a_guide_has_the_guides_rate_size_and_frames() {
    let folder = tempfile::tempdir().unwrap();
    let Some(footage) = guide(folder.path()) else {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    };
    let asking = |out: &str, more: serde_json::Value| {
        Request::parse(
            &json!({
                "kerf-request": 1, "model": "cameras", "guide": footage,
                "more": more, "out": folder.path().join(out),
            })
            .to_string(),
        )
        .unwrap()
    };
    let made = |request: &Request, workflow: &str| {
        let (server, kept) = serve(reader);
        let mut said = Vec::new();
        make(
            request,
            folder.path(),
            &Workflow::named(workflow).unwrap(),
            &server,
            &Tools::find(),
            &patience(),
            &AtomicBool::new(false),
            &mut |line| said.push(line.to_owned()),
        )
        .unwrap();
        let kept = kept.lock().unwrap();
        let queued = kept
            .iter()
            .find(|asked| asked.line == "POST /prompt")
            .unwrap();
        let queued: serde_json::Value = serde_json::from_slice(&queued.body).unwrap();
        (queued["prompt"].clone(), said.join("\n"))
    };

    // The file asked for is the one fetched, by its kind: the cameras as text.
    let cameras = asking("cameras.txt", json!({}));
    let (graph, said) = made(&cameras, "cameras");
    assert_eq!(std::fs::read(&cameras.out).unwrap(), b"its cameras");
    // The guide's own rate, each of its frames, and its size with its sides made even: not a
    // model's. It went up as it is, but for that.
    assert!(
        said.contains("for 10 frames of 322x180 at 12 a second"),
        "{said}"
    );
    assert_eq!(graph["6"]["inputs"]["fps"], json!(12.0));
    assert_eq!(graph["1"]["inputs"]["file"], "kerf-guide.mp4");
    let sent = Tools::find()
        .picture(&folder.path().join(GUIDE_SENT))
        .unwrap();
    assert_eq!((sent.size, sent.frames, sent.fps), ((322, 180), 10, 12.0));
    // Depth Anything 3 finds a camera's path with its weights as 32-bit floats, and no other way.
    assert_eq!(graph["3"]["inputs"]["weight_dtype"], "fp32");
    assert_eq!(graph["4"]["inputs"]["mode"], "multiview");
    assert_eq!(graph["4"]["inputs"]["mode.pose_method"], "cam_dec");

    // The same workflow's video, and a line's own values in its blanks.
    let depth = asking(
        "depth.mp4",
        json!({"pose": "ray_pose", "resolution": "756"}),
    );
    let (graph, _) = made(&depth, "cameras");
    assert_eq!(std::fs::read(&depth.out).unwrap(), b"its depth");
    assert_eq!(graph["4"]["inputs"]["mode.pose_method"], "ray_pose");
    assert_eq!(graph["4"]["inputs"]["resolution"], json!(756));

    // MoGe refines three times as it comes, and once where a line says so; `seconds` reads
    // the guide's first frames only.
    let (graph, _) = made(&asking("moge.mp4", json!({})), "depth");
    assert_eq!(graph["4"]["inputs"]["refine_steps"], json!(3));
    assert_eq!(graph["5"]["inputs"]["output"], "depth");
    let mut short = asking(
        "short.mp4",
        json!({"refine_steps": "1", "output": "normal_opengl"}),
    );
    short.seconds = Some(0.5);
    let (graph, said) = made(&short, "depth");
    assert_eq!(graph["4"]["inputs"]["refine_steps"], json!(1));
    assert_eq!(graph["5"]["inputs"]["output"], "normal_opengl");
    assert!(said.contains("for 6 frames of 322x180"), "{said}");

    // With no guide there is nothing to make a file of, and nothing is asked of the server.
    let mut none = asking("depth.mp4", json!({}));
    none.guide = None;
    let (server, kept) = serve(reader);
    let refused = make(
        &none,
        folder.path(),
        &Workflow::named("depth").unwrap(),
        &server,
        &Tools::find(),
        &patience(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(refused.contains("`guide=<file>`"), "{refused}");
    assert!(kept.lock().unwrap().is_empty());
}
