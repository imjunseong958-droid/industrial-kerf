//! What a ComfyUI server says each input of a workflow takes, read by an agent
//! with `look` and held to by its `edit`, against a real kerfd, the real
//! `kerf-comfy`, and a server that is a thread here.
//!
//! The server is named as a person names theirs, in the file `KERF_COMFY`
//! names, and it is the only one asked: what `~/.kerf/comfy.json` or
//! `COMFY_URL` names on the machine this runs on is not read.
//!
//! In a file of its own: it reads a models file of its own for the whole
//! process ([`Models::use_file`]) and gives the programs they run a server of
//! its own in `KERF_COMFY` ([`model::set_program_env`]), which the other
//! tests' process must not see, and what a server said is kept for as long
//! as a process runs.
//!
//! It starts `target/debug/kerf-comfy` as the others start `target/debug/kerfd`:
//! a run of the whole workspace builds both, and a run of this package alone
//! comes after `cargo build -p kerf-runtime --bin kerfd` and
//! `cargo build -p kerf-comfy`.

use kerf_engine::recipe::model::{self, Models};
use kerf_mcp::KerfMcpRouter;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const PROJECT: &str = "kerf 1
project name=\"Court\"
media shot path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480
  generate model=video prompt=\"a basketball bounces on a court\" seed=7
sequence main rate=24 size=1920x1080
  track V1 video
    clip slate gen=solid@1 at=0f dur=24f
";

fn ok(router: &KerfMcpRouter, name: &str, arguments: Value) -> Value {
    let response = router.call(name, arguments);
    assert_eq!(response["ok"], true, "{name}: {response}");
    response["result"].clone()
}

fn refused(router: &KerfMcpRouter, name: &str, arguments: Value) -> String {
    let response = router.call(name, arguments);
    assert_eq!(response["ok"], false, "{name}: {response}");
    response["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// A ComfyUI that says what each kind of node takes, and keeps what it was asked. Its
/// loader lists the weights it has and how it reads them; of every other kind it has, it
/// says nothing of what its inputs take.
fn comfy() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let kept = Arc::new(Mutex::new(Vec::new()));
    let keeping = kept.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let (mut bytes, mut chunk) = (Vec::new(), [0u8; 4096]);
            while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0, "a request broke off");
                bytes.extend(&chunk[..read]);
            }
            let line = String::from_utf8_lossy(&bytes)
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches(" HTTP/1.1")
                .to_owned();
            let answer = match line.strip_prefix("GET /object_info/") {
                Some("UNETLoader") => json!({"UNETLoader": {
                    "input": {"required": {
                        "unet_name": [["a.safetensors", "b.safetensors"], {}],
                        "weight_dtype": [["default", "fp8_e4m3fn"], {}],
                    }},
                    "output": ["MODEL"],
                }}),
                Some(class) => json!({class: {"input": {"required": {}}}}),
                None => json!({}),
            }
            .to_string();
            keeping.lock().unwrap().push(line);
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
fn a_blank_is_the_steps_to_fill_whatever_the_server_lists_for_its_input() {
    let built = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug");
    let program = built.join(format!("kerf-comfy{}", std::env::consts::EXE_SUFFIX));
    assert!(
        program.is_file(),
        "{} is not built: run `cargo build -p kerf-comfy`, or the whole workspace's tests",
        program.display()
    );
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    std::fs::create_dir(dir.join("shots")).unwrap();
    std::fs::write(dir.join("project.kerf"), PROJECT).unwrap();
    // The person's own models: one run with the kerf-comfy built beside this test.
    let models = dir.join("models.json");
    std::fs::write(
        &models,
        json!({"video": {"run": [program, "--workflow", "shot", "{request}"]}}).to_string(),
    )
    .unwrap();
    Models::use_file(&models);
    // The server, named as a person names theirs: in a file, which here is this test's own.
    // Nothing a shell names is read in its place.
    let (url, asked) = comfy();
    let named = dir.join("comfy.json");
    std::fs::write(&named, json!({"url": url}).to_string()).unwrap();
    model::set_program_env("KERF_COMFY", &named);
    for names_one in ["COMFY_URL", "COMFY_API_KEY", "COMFY_API_HEADER"] {
        model::remove_program_env(names_one);
    }
    let router = KerfMcpRouter::new(dir, Some(&built.join("kerfd"))).unwrap();
    router.set_permissions(Some(kerf_mcp::permissions::Permissions::default()));

    // The workflow, with what the server said each input takes beside it: a blank's input
    // and a value's alike.
    let workflow = ok(
        &router,
        "look",
        json!({"workflow": {"media": "shot", "line": 1}}),
    );
    assert_eq!(workflow["taken"], true, "{workflow}");
    assert!(workflow.get("takenError").is_none(), "{workflow}");
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|line| line == "GET /object_info/UNETLoader"),
        "the server named in KERF_COMFY was not the one asked: {asked:?}"
    );
    let loader = workflow["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["class"] == "UNETLoader")
        .expect("a loader");
    let input = |name: &str| {
        loader["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["name"] == name)
            .unwrap_or_else(|| panic!("no input `{name}` in {loader}"))
    };
    assert_eq!(input("unet_name")["blanks"], json!(["unet"]));
    assert_eq!(
        input("unet_name")["choices"],
        json!(["a.safetensors", "b.safetensors"])
    );
    assert_eq!(
        input("weight_dtype")["choices"],
        json!(["default", "fp8_e4m3fn"])
    );
    let loader = loader["id"].as_str().unwrap().to_owned();
    let set = |input: &str, value: &str| json!({"workflowSet": [{"media": "shot", "line": 1, "node": loader, "input": input, "value": value}]});

    // A value for the blank's input is refused as the step's to fill, with where it is
    // set, and not for being off the server's list: one that is on it is refused the same.
    let files = || std::fs::read_dir(dir.join("shots")).unwrap().count();
    for value in ["other", "a.safetensors"] {
        let said = refused(&router, "edit", set("unet_name", value));
        assert!(
            said.contains("`unet_name` of node 1 is the blank `{unet}`, which the step fills"),
            "{value}: {said}"
        );
    }
    // A value that is the workflow's own is held to the server's list.
    let said = refused(&router, "edit", set("weight_dtype", "fp64"));
    assert!(
        said.contains("takes one of default, fp8_e4m3fn, not \"fp64\""),
        "{said}"
    );
    assert_eq!(files(), 0, "nothing is kept of what was refused");
    let kept = ok(&router, "edit", set("weight_dtype", "fp8_e4m3fn"));
    assert_eq!(
        kept["workflows"][0]["keptAs"], "shots/shot.video.comfy.json",
        "{kept}"
    );
}
