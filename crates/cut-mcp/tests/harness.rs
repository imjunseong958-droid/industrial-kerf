//! `look`, `edit`, and `render` against a real cutd and real footage.

use cut_mcp::{CutMcpRouter, McpServer};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok()
}

/// Four seconds of picture with a cut at 2s, and a tone that stops at 3s.
fn footage(directory: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=320x180:rate=30:duration=2",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=320x180:rate=30:duration=2",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=3",
        ])
        .args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono"])
        .args([
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v];[2:a][3:a]concat=n=2:v=0:a=1,atrim=0:4[a]",
        ])
        .args([
            "-map",
            "[v]",
            "-map",
            "[a]",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-t", "4"])
        .arg(directory.join("shot.mp4"))
        .status()
        .expect("ffmpeg");
    assert!(status.success());
}

fn router() -> Option<(TempDir, CutMcpRouter)> {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return None;
    }
    let directory = TempDir::new().unwrap();
    footage(directory.path());
    std::fs::write(
        directory.path().join("project.cut"),
        "cut 1
project name=\"Harness\"
media m1 path=\"shot.mp4\" rate=30 dur=00:00:04:00 video=320x180 audio=1ch@48000
sequence s1 rate=30 size=320x180
  track V1 video
    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00 link=L1
  track A1 audio
    clip a1 media=m1 at=00:00:00:00 dur=00:00:04:00 link=L1
",
    )
    .unwrap();
    let cutd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/cutd");
    let router = CutMcpRouter::new(directory.path(), Some(&cutd)).unwrap();
    Some((directory, router))
}

fn ok(router: &CutMcpRouter, name: &str, arguments: Value) -> Value {
    let response = router.call(name, arguments);
    assert_eq!(response["ok"], true, "{name}: {response}");
    response["result"].clone()
}

fn error(router: &CutMcpRouter, name: &str, arguments: Value) -> Value {
    let response = router.call(name, arguments);
    assert_eq!(response["ok"], false, "{name}: {response}");
    response["error"].clone()
}

fn has_image(value: &Value) -> bool {
    value["imageBase64"]
        .as_str()
        .is_some_and(|data| data.len() > 100)
}

#[test]
fn lists_three_tools_and_the_reference() {
    let directory = TempDir::new().unwrap();
    let router = CutMcpRouter::new(directory.path(), None).unwrap();
    let names = router
        .list_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["look", "edit", "render"]);
    let agent = router.agent_tools();
    assert_eq!(agent.keys().collect::<Vec<_>>(), ["edit", "look", "render"]);
    let server = McpServer::new(router);
    let initialized = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .unwrap();
    let instructions = initialized["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("# Cut edit script"));
    assert!(instructions.contains("cut://document"));
    assert!(initialized["result"]["capabilities"]["resources"].is_object());
    let reference = server
        .handle(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "resources/read",
            "params": {"uri": "cut://reference"}}),
        )
        .unwrap();
    assert!(reference["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("`color.basic`"));
}

#[test]
fn look_shows_the_document_media_index_and_frames() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let document = ok(&router, "look", json!({}));
    let text = document["document"].as_str().unwrap();
    assert!(text.contains("clip c1 media=m1"), "{text}");
    assert!(text.contains('#'), "annotated: {text}");
    assert!(document["proposal"].is_null());

    let index = ok(&router, "look", json!({"media": "m1"}));
    assert_eq!(index["end"], "00:00:04:00");
    let uses = index["uses"].as_array().unwrap();
    assert_eq!(uses.len(), 2);
    assert_eq!(uses[0]["clip"], "c1");
    assert_eq!(uses[0]["out"], "00:00:04:00");
    assert_eq!(index["cuts"], json!(["00:00:02:00"]), "{index}");
    let pauses = index["pauses"].as_array().unwrap();
    assert_eq!(pauses.len(), 1, "{index}");
    assert!(
        pauses[0]["from"].as_str().unwrap().starts_with("00:00:03:"),
        "{index}"
    );

    let source = ok(
        &router,
        "look",
        json!({"media": "m1", "at": ["15f", "3s"], "width": 160}),
    );
    let frames = source["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(has_image));
    assert_eq!(frames[1]["at"], "3s");

    let program = ok(&router, "look", json!({"at": ["00:00:01:00"]}));
    assert!(has_image(&program["frames"][0]), "{program}");

    let outside = error(&router, "look", json!({"media": "m1", "at": ["10s"]}));
    assert_eq!(outside["code"], "INVALID_ARGUMENTS");
    let unknown = error(&router, "look", json!({"media": "m9"}));
    assert_eq!(unknown["code"], "UNKNOWN_MEDIA");

    let server = McpServer::new(router);
    let response = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "look", "arguments": {"media": "m1", "at": ["1s"], "width": 160}}}))
        .unwrap();
    let content = response["result"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "{response}");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["mimeType"], "image/png");
    let frame = &response["result"]["structuredContent"]["result"]["frames"][0];
    assert!(frame.get("imageBase64").is_none());
    assert_eq!(frame["image"], 0);
}

#[test]
fn edit_stages_a_checked_proposal_with_a_preview() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let before = router.runtime().get_document(false).unwrap();

    let proposal = ok(&router, "edit", json!({"script": "ripple-delete c1\n"}));
    // The whole picture is gone, so there is nothing after to show.
    assert_eq!(proposal["preview"]["after"]["empty"], true, "{proposal}");

    let revised = ok(
        &router,
        "edit",
        json!({"script": "trim c1 out -30f ripple", "revise": proposal["proposal"]}),
    );
    let id = revised["proposal"].as_str().unwrap();
    assert_ne!(id, proposal["proposal"].as_str().unwrap());
    assert_eq!(revised["status"], "waiting for the user's review");
    let diff = revised["diff"].as_str().unwrap();
    assert!(
        diff.contains("+    clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:03:00"),
        "{diff}"
    );
    assert!(
        diff.contains("+    clip a1 media=m1"),
        "linked sound follows: {diff}"
    );
    assert_eq!(revised["check"]["valid"], true, "{revised}");
    let preview = &revised["preview"];
    assert_eq!(preview["from"], "00:00:00:00");
    assert_eq!(preview["to"], "00:00:04:00");
    assert!(has_image(&preview["before"]), "{preview}");
    assert!(has_image(&preview["after"]), "{preview}");

    // Nothing changes until the user accepts.
    assert_eq!(router.runtime().get_document(false).unwrap(), before);
    let context = cut_mcp::harness::context(router.runtime()).unwrap();
    assert!(context.contains(&format!("Proposal {id}")), "{context}");

    let pending = error(&router, "edit", json!({"script": "trim c1 in +1f"}));
    assert_eq!(pending["code"], "PROPOSAL_PENDING");
    let stale = error(
        &router,
        "edit",
        json!({"script": "trim c1 in +1f", "revise": "p0"}),
    );
    assert_eq!(stale["code"], "NOT_PENDING");
    router.runtime().reject_proposal(id).unwrap();

    let refused = error(
        &router,
        "edit",
        json!({"script": "trim c1 out -30f\ntrim c9 in +1f"}),
    );
    assert_eq!(refused["code"], "SCRIPT_REFUSED");
    assert!(
        refused["message"].as_str().unwrap().contains(" 2:"),
        "{refused}"
    );
    let unchanged = error(&router, "edit", json!({"script": "trim c1 out +0f"}));
    assert_eq!(unchanged["code"], "NO_CHANGE", "{unchanged}");
}

#[test]
fn render_runs_a_range_to_a_file() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let started = ok(&router, "render", json!({"from": "0f", "to": "1s"}));
    let finished = finish(&router, started["job"].as_str().unwrap());
    assert_eq!(finished["status"], "completed", "{finished}");
    assert!(Path::new(finished["output"].as_str().unwrap()).is_file());
    let invalid = error(&router, "render", json!({"quality": "best"}));
    assert_eq!(invalid["code"], "INVALID_ARGUMENTS");
}

fn finish(router: &CutMcpRouter, job: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = ok(router, "render", json!({"job": job}));
        if status["status"] == "completed" || status["status"] == "failed" {
            return status;
        }
        assert!(Instant::now() < deadline, "render did not finish: {status}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_proposal_can_be_seen_extended_and_rendered() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let first = ok(
        &router,
        "edit",
        json!({"script": "trim c1 out -30f ripple"}),
    );
    let first_id = first["proposal"].as_str().unwrap().to_owned();

    // `look` with the proposal sees what it would make of the project.
    let document = ok(&router, "look", json!({"proposal": first_id}));
    assert_eq!(document["proposal"], first_id.as_str());
    let text = document["document"].as_str().unwrap();
    assert!(text.contains("at=00:00:00:00 dur=00:00:03:00"), "{text}");
    assert!(text.contains('#'), "annotated: {text}");
    let index = ok(
        &router,
        "look",
        json!({"media": "m1", "proposal": first_id}),
    );
    assert_eq!(index["uses"][0]["out"], "00:00:03:00", "{index}");
    let frames = ok(
        &router,
        "look",
        json!({"at": ["00:00:02:15"], "proposal": first_id, "width": 160}),
    );
    assert!(has_image(&frames["frames"][0]), "{frames}");
    // The proposal is three seconds long; the project is still four.
    let beyond = error(
        &router,
        "look",
        json!({"at": ["00:00:03:15"], "proposal": first_id}),
    );
    assert_eq!(beyond["code"], "INVALID_ARGUMENTS");
    ok(
        &router,
        "look",
        json!({"at": ["00:00:03:15"], "width": 160}),
    );

    // `extend` runs more lines on what the proposal makes; the two become
    // one proposal, and the preview shows only this step.
    let extended = ok(
        &router,
        "edit",
        json!({"script": "trim c1 in +15f ripple", "extend": first_id}),
    );
    let id = extended["proposal"].as_str().unwrap().to_owned();
    assert_ne!(id, first_id);
    let script = extended["script"].as_str().unwrap();
    let lines = script.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "{script}");
    assert!(lines[0].starts_with("trim c1 out"), "{script}");
    assert!(lines[1].starts_with("trim c1 in"), "{script}");
    let diff = extended["diff"].as_str().unwrap();
    assert!(
        diff.contains("+    clip c1 media=m1 src=00:00:00:15 at=00:00:00:00 dur=00:00:02:15"),
        "{diff}"
    );
    assert_eq!(extended["preview"]["to"], "00:00:03:00", "{extended}");
    assert!(has_image(&extended["preview"]["before"]), "{extended}");
    assert!(has_image(&extended["preview"]["after"]), "{extended}");
    let replaced = error(&router, "look", json!({"proposal": first_id}));
    assert_eq!(replaced["code"], "NOT_PENDING");
    let both = error(
        &router,
        "edit",
        json!({"script": "trim c1 in +1f", "extend": id, "revise": id}),
    );
    assert_eq!(both["code"], "INVALID_ARGUMENTS");

    // `render` with the proposal renders what it would make.
    let started = ok(
        &router,
        "render",
        json!({"proposal": id, "from": "0f", "to": "1s"}),
    );
    assert_eq!(started["proposal"], id.as_str());
    let finished = finish(&router, started["job"].as_str().unwrap());
    assert_eq!(finished["status"], "completed", "{finished}");
    assert!(Path::new(finished["output"].as_str().unwrap()).is_file());
    let beyond = error(
        &router,
        "render",
        json!({"proposal": id, "from": "0f", "to": "3s"}),
    );
    assert!(
        beyond["message"].as_str().unwrap().contains("2.500s"),
        "{beyond}"
    );

    // Once the project changes, the proposal is stale until it is revised.
    let revision = router.runtime().get_state().unwrap().revision;
    router
        .runtime()
        .apply_script_request("ui", "ui-1", "set c1 opacity=0.5", revision)
        .unwrap();
    for (name, arguments) in [
        ("look", json!({"proposal": id})),
        ("render", json!({"proposal": id})),
        ("edit", json!({"script": "trim c1 in +1f", "extend": id})),
    ] {
        assert_eq!(error(&router, name, arguments)["code"], "PROPOSAL_STALE");
    }
    ok(
        &router,
        "edit",
        json!({"script": "trim c1 out -30f ripple", "revise": id}),
    );
}

#[test]
fn look_shows_no_more_frames_than_a_model_receives() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let at = (0..=cut_agent::MAX_TOOL_RESULT_IMAGES)
        .map(|frame| format!("{frame}f"))
        .collect::<Vec<_>>();
    let refused = error(&router, "look", json!({"media": "m1", "at": at}));
    assert_eq!(refused["code"], "INVALID_ARGUMENTS");
    let accepted = ok(
        &router,
        "look",
        json!({"media": "m1", "at": &at[1..], "width": 64}),
    );
    assert_eq!(
        accepted["frames"].as_array().unwrap().len(),
        cut_agent::MAX_TOOL_RESULT_IMAGES
    );
}
