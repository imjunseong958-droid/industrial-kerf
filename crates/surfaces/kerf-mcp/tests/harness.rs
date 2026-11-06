//! `look`, `edit`, `render`, and `note` against a real kerfd and real
//! footage.

use kerf_mcp::{KerfMcpRouter, McpServer};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// The scene `kerf-track`'s own solve tests film, known cameras and all.
#[path = "../../../vfx/kerf-track/src/camera/scene.rs"]
mod scene;

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

fn router() -> Option<(TempDir, KerfMcpRouter)> {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return None;
    }
    let directory = TempDir::new().unwrap();
    footage(directory.path());
    std::fs::write(
        directory.path().join("project.kerf"),
        "kerf 1
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
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(directory.path(), Some(&kerfd)).unwrap();
    Some((directory, router))
}

fn ok(router: &KerfMcpRouter, name: &str, arguments: Value) -> Value {
    let response = router.call(name, arguments);
    assert_eq!(response["ok"], true, "{name}: {response}");
    response["result"].clone()
}

fn error(router: &KerfMcpRouter, name: &str, arguments: Value) -> Value {
    let response = router.call(name, arguments);
    assert_eq!(response["ok"], false, "{name}: {response}");
    response["error"].clone()
}

fn has_image(value: &Value) -> bool {
    value["imageBase64"]
        .as_str()
        .is_some_and(|data| data.len() > 100)
}

/// A JPEG inlined as JPEG: `/9j/` is the base64 of its start marker.
fn is_jpeg(value: &Value) -> bool {
    value["mediaType"] == "image/jpeg"
        && value["imageBase64"]
            .as_str()
            .is_some_and(|data| data.starts_with("/9j/"))
}

#[test]
fn presets_share_native_edit_code_and_apply_replacement_media_as_a_proposal() {
    let Some((source_directory, source)) = router() else {
        return;
    };
    let revision = source.runtime().get_state().unwrap().revision;
    source
        .runtime()
        .apply_script_request("ui", "preset-trim", "trim c1 out -60f", revision)
        .unwrap();
    let before_save = source.runtime().get_document(false).unwrap();
    let saved = ok(
        &source,
        "edit",
        json!({"savePreset":"style.kerfpreset", "name":"Two seconds"}),
    );
    assert_eq!(saved["slots"], 1);
    assert_eq!(source.runtime().get_document(false).unwrap(), before_save);
    assert_eq!(
        ok(&source, "look", json!({"presets":true}))["presets"],
        json!(["style.kerfpreset"])
    );
    // The shared library a repository carries is listed first.
    std::fs::create_dir_all(source_directory.path().join("presets")).unwrap();
    ok(
        &source,
        "edit",
        json!({"savePreset":"presets/team.kerfpreset", "name":"Team"}),
    );
    assert_eq!(
        ok(&source, "look", json!({"presets":true}))["presets"],
        json!(["presets/team.kerfpreset", "style.kerfpreset"])
    );
    let overwrite = error(&source, "edit", json!({"savePreset":"style.kerfpreset"}));
    assert_eq!(overwrite["code"], "PRESET_IO");
    assert_eq!(
        error(&source, "edit", json!({"savePreset":"project.kerf"}))["code"],
        "INVALID_PRESET"
    );

    let Some((destination_directory, destination)) = router() else {
        return;
    };
    std::fs::copy(
        source_directory.path().join("style.kerfpreset"),
        destination_directory.path().join("style.kerfpreset"),
    )
    .unwrap();
    let inspected = ok(&destination, "look", json!({"preset":"style.kerfpreset"}));
    assert_eq!(inspected["slots"][0]["seconds"], 2.0);
    // 4s of 30 fps media fills a 2s slot from any frame up to 60.
    assert_eq!(
        inspected["slots"][0]["fits"],
        json!([{"media": "m1", "latestFrame": 60}])
    );
    assert_eq!(
        inspected["slots"][0]["clips"][0]["track"].as_str(),
        Some("V1")
    );
    assert_eq!(
        inspected["slots"][0]["clips"][0]["at"].as_str(),
        Some("00:00:00:00")
    );
    assert!(!inspected["document"].as_str().unwrap().contains("shot.mp4"));
    let before = destination.runtime().get_document(false).unwrap();
    let missing = error(&destination, "edit", json!({"preset":"style.kerfpreset"}));
    assert_eq!(missing["code"], "INVALID_PRESET");
    let short = error(
        &destination,
        "edit",
        json!({"preset":"style.kerfpreset", "bindings":{"slot1":{"media":"m1","from":"3s"}}}),
    );
    assert_eq!(short["code"], "INVALID_PRESET");
    assert_eq!(destination.runtime().get_document(false).unwrap(), before);
    assert!(destination
        .runtime()
        .get_state()
        .unwrap()
        .pending_proposal
        .is_none());

    let proposed = ok(
        &destination,
        "edit",
        json!({"preset":"style.kerfpreset", "bindings":{"slot1":{"media":"m1","from":"2s"}}}),
    );
    assert_eq!(proposed["check"]["valid"], true, "{proposed}");
    assert!(has_image(&proposed["preview"]["after"]));
    assert_eq!(destination.runtime().get_document(false).unwrap(), before);
    let frame = ok(
        &destination,
        "look",
        json!({"proposal":proposed["proposal"], "at":["0f"]}),
    );
    let path = frame["frames"][0]["path"].as_str().unwrap();
    let pixels = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            path,
            "-vf",
            "scale=1:1",
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(pixels.status.success());
    assert_eq!(pixels.stdout.len(), 3);
    assert!(
        pixels.stdout[2] > 200 && pixels.stdout[0] < 40,
        "replacement is blue, not original red: {:?}",
        pixels.stdout
    );
    let accepted = destination
        .runtime()
        .accept_proposal(
            proposed["proposal"].as_str().unwrap(),
            "ui",
            "accept-preset",
            before.0,
        )
        .unwrap();
    assert_eq!(accepted.duration, 2.0);
    assert_eq!(accepted.revision, before.0 + 1);
}

#[test]
fn lists_the_four_tools_and_the_reference() {
    let directory = TempDir::new().unwrap();
    let router = KerfMcpRouter::new(directory.path(), None).unwrap();
    let names = router
        .list_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["look", "edit", "render", "note"]);
    let agent = router.agent_tools();
    assert_eq!(
        agent.keys().collect::<Vec<_>>(),
        ["edit", "look", "note", "render"]
    );
    let server = McpServer::new(router);
    let initialized = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .unwrap();
    let instructions = initialized["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("# Kerf edit script"));
    assert!(instructions.contains("kerf://document"));
    assert!(initialized["result"]["capabilities"]["resources"].is_object());
    let reference = server
        .handle(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "resources/read",
            "params": {"uri": "kerf://reference"}}),
        )
        .unwrap();
    assert!(reference["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("`color.basic@1`"));
}

struct TestFont;

impl kerf_text::fonts::FontSource for TestFont {
    fn faces(&self) -> Vec<kerf_text::fonts::FaceInfo> {
        vec![kerf_text::fonts::FaceInfo {
            family: "Kerf Test Variable".into(),
            style: "Regular".into(),
            weight: 400,
            italic: false,
            index: 0,
            data: kerf_text::fonts::FaceData::Static(include_bytes!(
                "../../../foundation/kerf-text/testdata/KerfTestVariable.ttf"
            )),
            origin: "test",
        }]
    }
}

#[test]
fn look_says_what_axes_a_font_has_on_its_own_scale() {
    let Some((_directory, router)) = router() else {
        return;
    };
    kerf_text::fonts::add_source(&TestFont);
    let font = ok(&router, "look", json!({"font":"kerf test variable"}));
    assert_eq!(font["found"], true);
    assert_eq!(font["faces"][0]["style"], "Regular");
    assert_eq!(
        font["faces"][0]["axes"],
        json!([
            {"tag":"wght", "min":100.0, "default":400.0, "max":900.0},
            {"tag":"wdth", "min":50.0, "default":100.0, "max":200.0},
            {"tag":"opsz", "min":8.0, "default":14.0, "max":144.0},
            {"tag":"slnt", "min":-20.0, "default":0.0, "max":0.0},
            {"tag":"ital", "min":0.0, "default":0.0, "max":1.0},
        ])
    );
    // A font with no axes says so, and a generic name is the font it stands for.
    let plain = ok(&router, "look", json!({"font":"sans-serif"}));
    assert_eq!(plain["font"], "Inter");
    assert_eq!(plain["found"], true);
    assert!(plain["faces"]
        .as_array()
        .unwrap()
        .iter()
        .all(|face| face["axes"] == json!([])));
    // A name no font has: the families that contain it, and for nothing, all of them.
    let near = ok(&router, "look", json!({"font":"kerf test"}));
    assert_eq!(near["found"], false);
    assert_eq!(near["families"], json!(["Kerf Test Variable"]));
    let all = ok(&router, "look", json!({"font":""}));
    assert!(all["families"]
        .as_array()
        .unwrap()
        .contains(&json!("Inter")));
    assert_eq!(
        ok(&router, "look", json!({"font":"nothing like it"}))["families"],
        json!([])
    );
    // It stands alone.
    assert_eq!(
        error(&router, "look", json!({"font":"Inter", "at":["0f"]}))["code"],
        "INVALID_ARGUMENTS"
    );
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
    assert!(frames.iter().all(is_jpeg));
    assert_eq!(frames[1]["at"], "3s");

    let program = ok(&router, "look", json!({"at": ["00:00:01:00"]}));
    assert!(has_image(&program["frames"][0]), "{program}");

    let outside = error(&router, "look", json!({"media": "m1", "at": ["10s"]}));
    assert_eq!(outside["code"], "INVALID_ARGUMENTS");
    let untimed = error(&router, "look", json!({"media": "m1", "words": true}));
    assert_eq!(untimed["code"], "INVALID_ARGUMENTS");
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
    assert_eq!(content[1]["mimeType"], "image/jpeg");
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
    let first = proposal["proposal"].as_str().unwrap();
    assert_ne!(id, first);
    assert_eq!(revised["status"], "awaiting_review");
    // A revise is the proposal's next version; the first is kept.
    assert_eq!(proposal["version"], 1, "{proposal}");
    assert_eq!(revised["version"], 2, "{revised}");
    assert_eq!(revised["supersedes"], first);
    let state = router.runtime().get_state().unwrap();
    let open: Vec<_> = state
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
        .collect();
    assert_eq!(open.len(), 1, "one proposal, two versions: {open:#?}");
    assert_eq!(open[0].proposal_id, id);
    let versions: Vec<(u32, &str)> = open[0]
        .versions
        .iter()
        .map(|version| (version.version, version.proposal_id.as_str()))
        .collect();
    assert_eq!(versions, [(1, first), (2, id)]);
    assert!(open[0].versions[0].script.starts_with("ripple-delete c1"));
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
    // The frames either side of the cut the trim moved.
    assert_eq!(
        preview["after"]["frames"],
        json!(["00:00:00:00", "00:00:02:29"]),
        "{preview}"
    );
    assert!(is_jpeg(&preview["after"]), "contact sheet");
    assert!(has_image(&preview["after"]), "{preview}");

    // Nothing changes until the user accepts.
    assert_eq!(router.runtime().get_document(false).unwrap(), before);
    let context = kerf_mcp::harness::context(router.runtime()).unwrap();
    assert!(context.contains(&format!("Proposal {id}")), "{context}");

    // The runtime keeps several proposals open: another edit is a second
    // proposal beside the first.
    let second = ok(&router, "edit", json!({"script": "trim c1 in +1f"}));
    let second = second["proposal"].as_str().unwrap().to_owned();
    assert_ne!(second, id, "a separate proposal");
    router.runtime().reject_proposal(&second).unwrap();
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

#[test]
fn renders_are_waited_for_or_cancelled_before_the_process_exits() {
    let Some((directory, router)) = router() else {
        return;
    };
    let waited = ok(&router, "render", json!({}));
    router.finish_renders(false);
    assert_eq!(router.running_renders(), 0);
    let status = ok(&router, "render", json!({"job": waited["job"]}));
    assert_eq!(status["status"], "completed", "{status}");

    let stopped = ok(&router, "render", json!({"quality": "full"}));
    router.finish_renders(true);
    assert_eq!(router.running_renders(), 0);
    let status = ok(&router, "render", json!({"job": stopped["job"]}));
    assert!(
        matches!(status["status"].as_str(), Some("cancelled" | "completed")),
        "{status}"
    );
    // The job's worker ran to its end and took its cancel file with it.
    let left = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".cancel"))
        .count();
    assert_eq!(left, 0);
}

#[test]
fn look_scans_with_sheets_and_indexes_sound() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let index = ok(&router, "look", json!({"media": "m1"}));
    for key in ["loudest", "onsets"] {
        assert!(index[key].is_array(), "{key}: {index}");
    }
    // The tone starts the sound; nothing starts after 2s.
    assert_eq!(index["onsets"], json!(["00:00:00:00"]), "{index}");
    let later = ok(&router, "look", json!({"media": "m1", "from": "2s"}));
    assert_eq!(later["onsets"], json!([]), "{later}");

    let sheet = ok(&router, "look", json!({"media": "m1", "sheet": 4}));
    assert_eq!(
        sheet["frames"],
        json!(["00:00:00:15", "00:00:01:15", "00:00:02:15", "00:00:03:15"]),
        "{sheet}"
    );
    assert!(is_jpeg(&sheet), "{sheet}");
    let program = ok(
        &router,
        "look",
        json!({"sheet": 2, "from": "1s", "to": "3s", "width": 160}),
    );
    assert_eq!(
        program["frames"],
        json!(["00:00:01:15", "00:00:02:15"]),
        "{program}"
    );
    assert!(is_jpeg(&program), "{program}");

    let both = error(
        &router,
        "look",
        json!({"media": "m1", "sheet": 4, "at": ["1s"]}),
    );
    assert_eq!(both["code"], "INVALID_ARGUMENTS");
    let lonely = error(&router, "look", json!({"from": "1s"}));
    assert_eq!(lonely["code"], "INVALID_ARGUMENTS");
}

#[test]
fn imported_media_is_indexed_and_proxied_in_the_background() {
    let Some((directory, router)) = router() else {
        return;
    };
    let wide = directory.path().join("wide.mp4");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=25:duration=2",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-shortest"])
        .arg(&wide)
        .status()
        .unwrap();
    assert!(status.success());
    let revision = router.runtime().get_state().unwrap().revision;
    router
        .runtime()
        .import_media_request("ui", "import-wide", &wide.to_string_lossy(), revision)
        .unwrap();
    let cache = kerf_engine::derivatives::DerivativeCache::new(directory.path());
    let deadline = Instant::now() + Duration::from_secs(60);
    let proxy = loop {
        if let Some(proxy) = kerf_engine::cached_proxy(&wide, &cache) {
            break proxy;
        }
        assert!(Instant::now() < deadline, "no proxy was made");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(proxy.is_file());
    let indexed = cache
        .source_entries(&wide, "media-index")
        .unwrap()
        .into_iter()
        .any(|entry| entry.is_complete(&["index.json"]));
    assert!(indexed, "the index is made before the proxy");
}

#[test]
fn scripts_anchor_to_clips_and_caption_only_what_was_heard() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let appended = ok(
        &router,
        "edit",
        json!({"script": "insert media=m1 src=1s dur=1s at=end on=V1,A1 as=c2,a2"}),
    );
    let diff = appended["diff"].as_str().unwrap();
    assert!(
        diff.contains("clip c2 media=m1 src=00:00:01:00 at=00:00:04:00"),
        "{diff}"
    );
    // Extending keeps both scripts as they were written.
    let extended = ok(
        &router,
        "edit",
        json!({"script": "marker add at=c2.end as=done", "extend": appended["proposal"]}),
    );
    let Some(kerf_runtime::PendingProposal::Pending { script, source, .. }) =
        router.runtime().get_state().unwrap().pending_proposal
    else {
        panic!("expected a pending proposal");
    };
    assert_eq!(
        source,
        "insert media=m1 src=1s dur=1s at=end on=V1,A1 as=c2,a2\nmarker add at=c2.end as=done"
    );
    assert!(script.contains("marker add as=done at="), "{script}");
    router
        .runtime()
        .reject_proposal(extended["proposal"].as_str().unwrap())
        .unwrap();

    // An extension that would act otherwise after the operations it follows
    // is refused, and the proposal it extends stays.
    let two = ok(
        &router,
        "edit",
        json!({"script": "sequence add s2 rate=30 size=320x180\nuse s1\nmarker add at=0f as=k1"}),
    );
    let elsewhere = error(
        &router,
        "edit",
        json!({
            "script": "sequence add s3 rate=30 size=320x180\ntrack add V8 video",
            "extend": two["proposal"],
        }),
    );
    assert_eq!(elsewhere["code"], "SCRIPT_REFUSED");
    assert!(
        elsewhere["message"]
            .as_str()
            .unwrap()
            .contains("use <sequence>"),
        "{elsewhere}"
    );
    router
        .runtime()
        .reject_proposal(two["proposal"].as_str().unwrap())
        .unwrap();

    let unheard = error(
        &router,
        "edit",
        json!({"script": "track add S1 subtitle\ncaptions c1 on=S1"}),
    );
    assert_eq!(unheard["code"], "SCRIPT_REFUSED");
    let message = unheard["message"].as_str().unwrap();
    assert!(message.contains(" 2:1: no transcript of `m1`"), "{message}");
    let unknown = error(
        &router,
        "edit",
        json!({"script": "track add S1 subtitle\ncaptions c1 on=S1 size=2"}),
    );
    assert!(unknown["message"]
        .as_str()
        .unwrap()
        .contains("unknown key `size`"));
}

#[test]
fn edits_snap_to_what_the_index_found() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // The footage cuts from red to blue at 2s and falls silent after 3s;
    // looking at it makes its index.
    ok(&router, "look", json!({"media": "m1"}));
    let split = ok(
        &router,
        "edit",
        json!({"script": "split c1 at=00:00:01:25 snap=cut as=c1b"}),
    );
    let script = split["script"].as_str().unwrap();
    assert!(script.contains("split c1 at=00:00:02:00"), "{script}");
    router
        .runtime()
        .reject_proposal(split["proposal"].as_str().unwrap())
        .unwrap();
    let trimmed = ok(
        &router,
        "edit",
        json!({"script": "trim c1 out to=00:00:02:15 snap=pause within=1s"}),
    );
    // Asked for 2s 15f, the out point lands just inside the pause after 3s.
    let script = trimmed["script"].as_str().unwrap();
    let shortened: i64 = script
        .split_once("trim c1 out -")
        .and_then(|(_, rest)| rest.split_once('f'))
        .and_then(|(frames, _)| frames.parse().ok())
        .unwrap_or_else(|| panic!("{script}"));
    assert!((26..=30).contains(&shortened), "{script}");
    router
        .runtime()
        .reject_proposal(trimmed["proposal"].as_str().unwrap())
        .unwrap();

    // Starting a frame before the cut shows a sliver of the red shot.
    let sliver = ok(
        &router,
        "edit",
        json!({"script": "trim c1 in to=00:00:01:29"}),
    );
    let findings = sliver["check"]["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|finding| finding["code"] == "cut.near_scene_cut"),
        "{sliver}"
    );
}

#[test]
fn edits_are_checked_like_an_editor_would() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // Lifting the first second leaves nothing on screen there.
    let lifted = ok(&router, "edit", json!({"script": "lift from=0f to=1s"}));
    let findings = lifted["check"]["findings"].as_array().unwrap();
    let black = findings
        .iter()
        .find(|finding| finding["code"] == "picture.black")
        .unwrap_or_else(|| panic!("{lifted}"));
    assert!(
        black["message"]
            .as_str()
            .unwrap()
            .contains("from 00:00:00:00 to 00:00:01:00"),
        "{black}"
    );
    assert_eq!(lifted["check"]["valid"], true, "a warning: {lifted}");
}

#[test]
fn any_sequence_can_be_seen_and_rendered() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let script = "sequence add s2 rate=30 size=180x320
use s2
track add VS2 video
insert media=m1 src=1s dur=1s at=0f on=VS2 as=v2";
    let proposal = ok(&router, "edit", json!({"script": script}));
    let id = proposal["proposal"].as_str().unwrap().to_owned();
    assert_eq!(proposal["preview"]["sequence"], "s2", "{proposal}");
    assert!(has_image(&proposal["preview"]["after"]), "{proposal}");
    // A wide picture in a tall frame leaves its edges black.
    let edges = proposal["check"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["code"] == "picture.edges")
        .unwrap_or_else(|| panic!("{proposal}"))
        .clone();
    assert_eq!(edges["entity"], "v2", "{edges}");

    let frames = ok(
        &router,
        "look",
        json!({"at": ["15f"], "sequence": "s2", "proposal": id}),
    );
    assert_eq!(frames["sequence"], "s2");
    assert!(has_image(&frames["frames"][0]), "{frames}");
    let unknown = error(&router, "look", json!({"at": ["0f"], "sequence": "s9"}));
    assert_eq!(unknown["code"], "UNKNOWN_SEQUENCE");
    let mixed = error(&router, "look", json!({"media": "m1", "sequence": "s2"}));
    assert_eq!(mixed["code"], "INVALID_ARGUMENTS");

    let started = ok(
        &router,
        "render",
        json!({"sequence": "s2", "proposal": id, "quality": "full"}),
    );
    assert_eq!(started["sequence"], "s2");
    let finished = finish(&router, started["job"].as_str().unwrap());
    assert_eq!(finished["status"], "completed", "{finished}");
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(finished["output"].as_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "180,320");
}

#[test]
fn look_hears_what_the_sequence_says() {
    if std::env::var_os("KERF_WHISPER_MODEL").is_none() {
        eprintln!("skipping: KERF_WHISPER_MODEL is not set");
        return;
    }
    let Some((_directory, router)) = router() else {
        return;
    };
    let heard = ok(
        &router,
        "look",
        json!({"transcript": true, "from": "1s", "to": "3s"}),
    );
    assert_eq!(heard["sequence"], "s1");
    assert_eq!(heard["from"], "00:00:01:00");
    assert_eq!(heard["to"], "00:00:03:00");
    assert!(heard["transcript"].is_array(), "{heard}");
    let mixed = error(&router, "look", json!({"transcript": true, "at": ["1s"]}));
    assert_eq!(mixed["code"], "INVALID_ARGUMENTS");
}

fn finish(router: &KerfMcpRouter, job: &str) -> Value {
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
    // It is the proposal's second version; the first can still be seen.
    assert_eq!(extended["version"], 2, "{extended}");
    assert_eq!(extended["supersedes"], first_id.as_str());
    let earlier = ok(&router, "look", json!({"proposal": first_id}));
    let text = earlier["document"].as_str().unwrap();
    assert!(text.contains("at=00:00:00:00 dur=00:00:03:00"), "{text}");
    let context = kerf_mcp::harness::context(router.runtime()).unwrap();
    assert!(
        context.contains(&format!("Proposal {id} (v2, by Kerf,")),
        "{context}"
    );
    assert!(
        context.contains(&format!("Earlier versions: v1 {first_id}.")),
        "{context}"
    );
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
    let at = (0..=kerf_agent::MAX_TOOL_RESULT_IMAGES)
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
        kerf_agent::MAX_TOOL_RESULT_IMAGES
    );
}

#[test]
fn scopes_read_the_program_and_what_a_proposal_would_make_of_it() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let read = |arguments: Value| {
        let result = ok(&router, "look", arguments);
        assert!(result.get("frames").is_none(), "numbers, not pictures");
        result["readings"][0]["scopes"].clone()
    };
    let before = read(json!({"at": ["1s"], "scopes": true}));
    assert!(before["luma"]["mean"].as_f64().unwrap() > 1.0, "{before}");
    assert_eq!(before["histogram"].as_array().unwrap().len(), 16);
    assert!(before["zones"]["midtones"].as_f64().is_some());
    assert!(before["chroma"]["saturation"].as_f64().is_some());
    assert!(before["clipped"]["white"].as_array().is_some());

    let edit = ok(
        &router,
        "edit",
        json!({"script": "fx c1 add color.basic saturation=0"}),
    );
    let proposal = edit["proposal"]["id"]
        .as_str()
        .or_else(|| edit["proposal"].as_str())
        .unwrap_or_else(|| panic!("{edit}"));
    let after = read(json!({"at": ["1s"], "scopes": true, "proposal": proposal}));
    let saturation = |reading: &Value| reading["chroma"]["saturation"].as_f64().unwrap();
    assert!(
        saturation(&after) < saturation(&before) - 50.0,
        "{before} {after}"
    );

    for refused in [
        json!({"scopes": true}),
        json!({"scopes": true, "at": ["1s"], "media": "m1"}),
        json!({"scopes": true, "at": ["1s"], "transcript": true}),
        json!({"scopes": true, "at": ["99s"]}),
    ] {
        assert_eq!(
            error(&router, "look", refused.clone())["code"],
            "INVALID_ARGUMENTS",
            "{refused}"
        );
    }
}

#[test]
fn look_follows_a_point_and_gives_a_line_that_makes_a_clip_follow_it() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // A white square crossing the picture, and a null to follow it, over the first second.
    let added = ok(
        &router,
        "edit",
        json!({"script": "track add V2 video\ntrack add V3 video\noverwrite gen=solid dur=30f at=0f on=V2 as=sq\nset sq color=#ffffff scale=0.1 x={0f:-0.2 30f:0.2}\noverwrite gen=null dur=30f at=0f on=V3 as=n1"}),
    );
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    // Its top-left corner at the start: a tenth of the frame wide, centered, moved a fifth left.
    let followed = ok(
        &router,
        "look",
        json!({
            "track": {"x": 0.25, "y": 0.45, "clip": "n1"},
            "from": "0f",
            "to": "24f",
            "proposal": proposal,
        }),
    );
    assert!(followed["lostAt"].is_null(), "{followed}");
    assert_eq!(followed["frames"], 25);
    let points = followed["points"].as_array().unwrap();
    let last = points.last().unwrap();
    // It moved 0.4 of the frame over 30 frames, so 0.32 over 24.
    assert!(
        (last["x"].as_f64().unwrap() - 0.57).abs() < 0.02,
        "{followed}"
    );
    assert!(
        (last["y"].as_f64().unwrap() - 0.45).abs() < 0.02,
        "{followed}"
    );
    // A straight move is few keyframes, not one a frame.
    assert!(points.len() < 10, "{followed}");
    let script = followed["script"].as_str().unwrap();
    assert!(script.starts_with("set n1 x={0f:"), "{script}");
    // The line is one the language takes.
    ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": format!("{script}\nset sq parent=n1")}),
    );

    for refused in [
        json!({"track": {"x": 0.5, "y": 0.5}, "from": "0f"}),
        json!({"track": {"x": 1.5, "y": 0.5}, "from": "0f", "to": "2f"}),
        json!({"track": {"x": 0.5, "y": 0.5}, "from": "4f", "to": "2f"}),
        json!({"track": {"x": 0.5, "y": 0.5, "clip": "nobody"}, "from": "0f", "to": "2f"}),
        json!({"track": {"x": 0.5, "y": 0.5}, "from": "0f", "to": "2f", "media": "m1"}),
    ] {
        assert_eq!(
            error(&router, "look", refused.clone())["code"],
            "INVALID_ARGUMENTS",
            "{refused}"
        );
    }
}

#[test]
fn look_follows_a_surface_and_gives_lines_that_pin_and_mask_to_it() {
    let Some((directory, router)) = router() else {
        return;
    };
    // A textured picture panning right to left, 4 pixels a frame.
    let pan = directory.path().join("pan.mp4");
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg("mandelbrot=s=480x180:r=30:start_scale=0.6:end_scale=0.6:end_pts=1,crop=320:180:x='n*4':y=0,trim=duration=1")
        .args(["-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv444p"])
        .arg(&pan)
        .status()
        .expect("ffmpeg");
    assert!(made.success());
    let added = ok(
        &router,
        "edit",
        json!({"script": "media add p path=\"pan.mp4\" rate=30 dur=30f video=320x180\ntrack add V2 video\ntrack add V3 video\noverwrite media=p src=0f dur=30f at=0f on=V2 as=plate\noverwrite gen=solid dur=30f at=0f on=V3 as=sq\nset sq color=#ffffff"}),
    );
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    // Hide the white square for the tracking: it is what will be pinned.
    let hidden = ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": "set sq opacity=0"}),
    );
    let proposal = hidden["proposal"]["id"]
        .as_str()
        .or_else(|| hidden["proposal"].as_str())
        .unwrap_or(proposal);
    let corners = [[0.4, 0.3], [0.7, 0.3], [0.7, 0.7], [0.4, 0.7]];
    let followed = ok(
        &router,
        "look",
        json!({
            "track": {"corners": corners, "clip": "sq"},
            "from": "0f",
            "to": "20f",
            "proposal": proposal,
        }),
    );
    assert!(followed["lostAt"].is_null(), "{followed}");
    assert_eq!(followed["frames"], 21);
    // 80 pixels of 320 to the left by frame 20.
    let quads = followed["quads"].as_array().unwrap();
    let last = quads.last().unwrap();
    assert_eq!(last["frame"], 20);
    let found = last["corners"].as_array().unwrap();
    for (found, start) in found.iter().zip(corners) {
        let (x, y) = (found[0].as_f64().unwrap(), found[1].as_f64().unwrap());
        assert!((x - (start[0] - 0.25)).abs() < 0.01, "{followed}");
        assert!((y - start[1]).abs() < 0.01, "{followed}");
    }
    // A straight move is few keyframes.
    assert!(quads.len() < 8, "{followed}");
    // The lines are ones the language takes.
    let script = followed["script"].as_str().unwrap();
    assert!(
        script.starts_with("fx sq add distort.cornerpin@2 upper_left_x={0f:"),
        "{script}"
    );
    let outline = followed["outline"].as_str().unwrap();
    ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": format!("{script}\nmask plate add k1 points={outline} mode=none\nfx plate add image.fill@1 path=k1 method=smooth")}),
    );
    for refused in [
        json!({"track": {"corners": corners, "x": 0.5, "y": 0.5}, "from": "0f", "to": "2f"}),
        json!({"track": {"x": 0.5}, "from": "0f", "to": "2f"}),
        json!({"track": {"corners": [[0.1, 0.1], [2.0, 0.1], [0.9, 0.9], [0.1, 0.9]]}, "from": "0f", "to": "2f"}),
    ] {
        assert_eq!(
            error(&router, "look", refused.clone())["code"],
            "INVALID_ARGUMENTS",
            "{refused}"
        );
    }
}

#[test]
fn look_cuts_out_an_object_and_gives_a_line_that_masks_it() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // A white square crossing the red shot.
    let added = ok(
        &router,
        "edit",
        json!({"script": "track add V2 video\noverwrite gen=solid dur=30f at=0f on=V2 as=sq\nset sq color=#ffffff scale=0.3 x={0f:-0.2 30f:0.2}"}),
    );
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    let followed = ok(
        &router,
        "look",
        json!({
            "track": {"inside": [[0.3, 0.5]], "outside": [[0.05, 0.05]], "clip": "sq"},
            "from": "0f",
            "to": "24f",
            "proposal": proposal,
        }),
    );
    assert!(followed["lostAt"].is_null(), "{followed}");
    assert_eq!(followed["frames"], 25);
    // A square 0.3 of the frame each way covers 0.09 of it.
    let area = followed["area"].as_f64().unwrap();
    assert!((area - 0.09).abs() < 0.01, "{followed}");
    let script = followed["script"].as_str().unwrap();
    assert!(
        script.starts_with("mask sq add object points={0f:"),
        "{script}"
    );
    ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": script}),
    );
    // Where it ends, it has moved with the square: 0.32 of the frame to the right, at its middle.
    let outline = followed["outline"].as_str().unwrap();
    let last = outline
        .trim_matches(['{', '}'])
        .split(' ')
        .next_back()
        .and_then(|key| key.split(':').nth(1))
        .unwrap();
    let corners: Vec<[f64; 2]> = last
        .split(';')
        .map(|corner| {
            let (x, y) = corner.split_once(',').unwrap();
            [x.parse().unwrap(), y.parse().unwrap()]
        })
        .collect();
    let middle = corners.iter().map(|c| c[0]).sum::<f64>() / corners.len() as f64;
    assert!((middle - 0.62).abs() < 0.02, "{middle}: {followed}");
    for refused in [
        json!({"track": {"inside": [[0.5, 0.5]], "x": 0.5, "y": 0.5}, "from": "0f", "to": "2f"}),
        json!({"track": {"outside": [[0.5, 0.5]]}, "from": "0f", "to": "2f"}),
        json!({"track": {"inside": [[1.5, 0.5]]}, "from": "0f", "to": "2f"}),
        json!({"track": {"x": 0.5, "y": 0.5, "model": "mobilesam"}, "from": "0f", "to": "2f"}),
        json!({"track": {"inside": [[0.5, 0.5]], "model": "nobody"}, "from": "0f", "to": "2f"}),
    ] {
        assert_eq!(
            error(&router, "look", refused.clone())["code"],
            "INVALID_ARGUMENTS",
            "{refused}"
        );
    }
}

#[test]
fn look_lines_recordings_up_by_their_sound_and_gives_a_multicam() {
    let Some((directory, router)) = router() else {
        return;
    };
    // The same claps recorded by two cameras of different colors, the second begun 1.5 s later.
    let camera = |name: &str, color: &str, start: f64, duration: f64| {
        let made = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("color=c={color}:s=160x90:r=30:d={duration}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!(
                "aevalsrc='0.8*sin(2*PI*700*t)*(lt(mod(t+{start}\\,1.3)\\,0.05)+lt(mod(t+{start}\\,1.7)\\,0.03))':s=48000:d={duration}"
            ))
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(directory.path().join(name))
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
    };
    camera("a.mp4", "red", 0.0, 8.0);
    camera("b.mp4", "blue", 1.5, 5.0);
    let added = ok(
        &router,
        "edit",
        json!({"script": "media add ma path=\"a.mp4\" rate=30 dur=240f video=160x90 audio=1ch@48000\nmedia add mb path=\"b.mp4\" rate=30 dur=150f video=160x90 audio=1ch@48000"}),
    );
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    let found = ok(
        &router,
        "look",
        json!({"sync": ["ma", "mb"], "proposal": proposal}),
    );
    let rows = found["recordings"].as_array().unwrap();
    assert_eq!(rows[0]["at"], "0f", "{found}");
    assert!(
        (rows[1]["offset"].as_f64().unwrap() - 1.5).abs() < 0.04,
        "{found}"
    );
    assert_eq!(rows[1]["at"], "45f", "{found}");
    assert!(rows[1]["confidence"].as_f64().unwrap() > 0.2, "{found}");
    assert_eq!(found["sequence"], "multicam");
    // What it gives builds the sequence, and a clip of it shows one camera, then the other.
    let script = found["script"].as_str().unwrap();
    let built = ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": format!("{script}\nuse s1\ntrack add V9 video\noverwrite seq=multicam src=60f at=0f dur=30f on=V9 as=cam\nset cam angle=V1\nsplit cam at=15f as=cam2\nset cam2 angle=V2")}),
    );
    assert!(built.get("proposal").is_some(), "{built}");
    for refused in [
        json!({"sync": ["ma"]}),
        json!({"sync": ["ma", "nobody"], "proposal": proposal}),
    ] {
        assert_eq!(
            error(&router, "look", refused.clone())["code"],
            "INVALID_ARGUMENTS",
            "{refused}"
        );
    }
}

#[test]
fn raster_effects_are_looked_up_checked_and_previewed() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let hue = ok(&router, "look", json!({"effect": "color.huesaturation"}));
    assert_eq!(hue["engine"], "raster");
    assert_eq!(hue["category"], "Color Correction");
    assert!(
        hue["reference"]
            .as_str()
            .unwrap()
            .contains("`saturation=0` (-100..100"),
        "{hue}"
    );
    // FilmCraft's effects are Kerf's renderer's.
    let invert = ok(&router, "look", json!({"effect": "channel.invert"}));
    assert_eq!(invert["engine"], "kerf", "{invert}");
    assert!(
        invert["reference"]
            .as_str()
            .unwrap()
            .contains("`blend=0` (0..100"),
        "{invert}"
    );
    assert_eq!(
        error(&router, "look", json!({"effect": "channel.nothing"}))["code"],
        "INVALID_ARGUMENTS"
    );
    // Effects are found by the look wanted, each saying when to use it.
    let found = ok(&router, "look", json!({"effects": "neon glow"}));
    let doc = found["effects"][0]["doc"].as_str().unwrap();
    assert!(doc.contains("Glow") && doc.contains("neon"), "{found}");
    let retro = ok(&router, "look", json!({"effects": "retro tv transition"}));
    assert_eq!(retro["effects"][0]["id"], "tv-power", "{retro}");
    // An effect EffectCraft had says what replaced it.
    let gone = error(&router, "look", json!({"effect": "stylize.glow"}));
    assert!(gone.to_string().contains("light.wonderglow@1"), "{gone}");
    // A video editor's transitions are found by the moment too.
    let whip = ok(&router, "look", json!({"effects": "whip pan travel"}));
    assert_eq!(whip["effects"][0]["id"], "whip", "{whip}");
    let push = ok(&router, "look", json!({"effect": "push"}));
    assert_eq!(push["engine"], "kerf", "{push}");
    assert!(
        push["reference"]
            .as_str()
            .unwrap()
            .contains("`direction=from-west`"),
        "{push}"
    );

    let proposal = ok(
        &router,
        "edit",
        json!({"script": "fx c1 add stylize.findedges\n"}),
    );
    assert_eq!(proposal["check"]["valid"], true, "{proposal}");
    assert!(
        !proposal["check"]["findings"]
            .to_string()
            .contains("unrenderable"),
        "{proposal}"
    );
    assert!(has_image(&proposal["preview"]["after"]), "{proposal}");
}

/// An MCP server for `router` whose client said it is `client`.
fn served(router: &KerfMcpRouter, client: &str) -> McpServer {
    let server = McpServer::new(router.clone());
    let initialized = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05",
                       "clientInfo": {"name": client, "version": "1.0.0"}}}))
        .unwrap();
    assert_eq!(initialized["result"]["serverInfo"]["name"], "kerf-mcp");
    server
}

/// Call `name` through `server` as its client would.
fn call(server: &McpServer, name: &str, arguments: Value) -> Value {
    let response = server
        .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}}))
        .unwrap();
    let structured = response["result"]["structuredContent"].clone();
    assert_eq!(structured["ok"], true, "{name}: {structured}");
    structured["result"].clone()
}

#[test]
fn an_mcp_clients_proposals_are_by_the_client_and_keep_their_versions() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let server = served(&router, "claude-code");
    let first = call(
        &server,
        "edit",
        json!({"script": "trim c1 out -30f ripple"}),
    );
    assert_eq!(first["author"], "Claude Code", "{first}");
    let first_id = first["proposal"].as_str().unwrap().to_owned();

    // Kerf's own agent, or another client, proposes beside it.
    let kerf = KerfMcpRouter::new(router.directory(), None).unwrap();
    let beside = ok(&kerf, "edit", json!({"script": "trim c1 in +5f"}));
    assert_eq!(beside["author"], "Kerf");
    let beside_id = beside["proposal"].as_str().unwrap().to_owned();
    let cursor = KerfMcpRouter::new(router.directory(), None).unwrap();
    let cursor_server = served(&cursor, "cursor-vscode");
    let third = call(
        &cursor_server,
        "edit",
        json!({"script": "set c1 opacity=0.8"}),
    );
    assert_eq!(third["author"], "Cursor");

    // Claude Code revises its proposal, which is no longer the newest.
    let revised = call(
        &server,
        "edit",
        json!({"script": "trim c1 out -32f ripple", "revise": first_id}),
    );
    assert_eq!(revised["version"], 2, "{revised}");
    assert_eq!(revised["supersedes"], first_id.as_str());
    let revised_id = revised["proposal"].as_str().unwrap().to_owned();

    let state = router.runtime().get_state().unwrap();
    let open: Vec<_> = state
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
        .collect();
    assert_eq!(open.len(), 3, "{open:#?}");
    let claude = open
        .iter()
        .find(|entry| entry.proposal_id == revised_id)
        .expect("the revised proposal is open");
    assert_eq!(claude.author.kind, "mcp");
    assert_eq!(claude.author.name, "Claude Code");
    assert_eq!(claude.author.id.as_deref(), Some("claude-code"));
    assert_eq!(claude.version, 2);
    assert_eq!(claude.versions.len(), 2);
    assert_eq!(claude.versions[0].proposal_id, first_id);
    let by = |id: &str| {
        open.iter()
            .find(|entry| entry.proposal_id == id)
            .map(|entry| (entry.author.kind.clone(), entry.author.name.clone()))
    };
    assert_eq!(by(&beside_id), Some(("kerf".into(), "Kerf".into())));
    assert_eq!(
        by(third["proposal"].as_str().unwrap()),
        Some(("mcp".into(), "Cursor".into()))
    );

    // Extending the proposal Kerf made, though another is newer.
    let extended = ok(
        &kerf,
        "edit",
        json!({"script": "trim c1 in +1f", "extend": beside_id}),
    );
    assert_eq!(extended["version"], 2, "{extended}");
    // And v1 of Claude Code's is still the user's to see and accept.
    let v1 = call(&server, "look", json!({"proposal": first_id}));
    assert!(
        v1["document"]
            .as_str()
            .unwrap()
            .contains("at=00:00:00:00 dur=00:00:03:00"),
        "{v1}"
    );
}

#[test]
fn the_note_tool_writes_a_comment_by_the_client() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let server = served(&router, "codex-mcp-client");
    let proposal = call(
        &server,
        "edit",
        json!({"script": "trim c1 out -12f ripple"}),
    );
    let proposal_id = proposal["proposal"].as_str().unwrap().to_owned();
    let before = router.runtime().get_state().unwrap();

    let note = call(
        &server,
        "note",
        json!({"body": "End card holds a beat long, trimmed 12 frames",
               "at": "00:00:02:15", "duration": "15f", "proposal": proposal_id,
               "tags": ["#pacing"], "blocking": false}),
    );
    let comment = &note["comment"];
    assert_eq!(comment["author"]["kind"], "mcp", "{note}");
    assert_eq!(comment["author"]["name"], "Codex");
    assert_eq!(comment["author"]["id"], "codex-mcp-client");
    assert_eq!(comment["author"]["role"], "MCP");
    assert_eq!(comment["at"], 2.5);
    assert_eq!(comment["duration"], 0.5);
    assert_eq!(comment["frame"], 75);
    assert_eq!(comment["proposal"], proposal_id.as_str());
    assert_eq!(comment["tags"], json!(["pacing"]));
    assert_eq!(note["timecode"], "00:00:02:15");
    let root = comment["id"].as_str().unwrap().to_owned();

    // A reply to the reply joins the thread.
    let reply = call(
        &server,
        "note",
        json!({"body": "Two frames earlier", "parent": root}),
    );
    let reply_id = reply["comment"]["id"].as_str().unwrap().to_owned();
    let again = call(&server, "note", json!({"body": "Done", "parent": reply_id}));
    assert_eq!(again["comment"]["parent"], root.as_str(), "{again}");

    // The review log keeps all three; the document and the proposal are
    // as they were, and the bridge sees the log change.
    let log = router.runtime().comments().unwrap();
    assert_eq!(log.items.len(), 3);
    assert!(log.items.iter().all(|item| item.author.name == "Codex"));
    let after = router.runtime().get_state().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.document, before.document);
    assert!(after.review_revision > before.review_revision);
    assert_eq!(
        after
            .pending_proposal
            .as_ref()
            .map(|p| serde_json::to_value(p).unwrap()["freshness"].clone()),
        before
            .pending_proposal
            .as_ref()
            .map(|p| serde_json::to_value(p).unwrap()["freshness"].clone())
    );

    // The generated agent docs describe the tool.
    let docs = router.generate_agent_docs().unwrap();
    let protocol = std::fs::read_to_string(docs.directory.join("protocol.md")).unwrap();
    assert!(protocol.contains("### `note`"), "{protocol}");
    assert!(protocol.contains("## Proposals and notes"));

    // Kerf's own agent notes as Kerf.
    let kerf = KerfMcpRouter::new(router.directory(), None).unwrap();
    let own = ok(&kerf, "note", json!({"body": "Welcome"}));
    assert_eq!(own["comment"]["author"]["kind"], "kerf");
    assert_eq!(own["comment"]["at"], Value::Null);

    for (arguments, code) in [
        (json!({"body": "  "}), "INVALID_ARGUMENTS"),
        (json!({"body": "x", "duration": "1s"}), "INVALID_ARGUMENTS"),
        (json!({"body": "x", "at": "10s"}), "INVALID_ARGUMENTS"),
        (
            json!({"body": "x", "at": "1s", "duration": "0f"}),
            "INVALID_ARGUMENTS",
        ),
        (json!({"body": "x", "parent": "c-nope"}), "UNKNOWN_COMMENT"),
        (
            json!({"body": "x", "proposal": "proposal-nope"}),
            "UNKNOWN_PROPOSAL",
        ),
        (json!({"body": "x", "colour": "red"}), "INVALID_ARGUMENTS"),
    ] {
        assert_eq!(
            error(&router, "note", arguments.clone())["code"],
            code,
            "{arguments}"
        );
    }
}

/// Lossless footage of `surfaces` through `shots`, and a project of one sequence that shows it.
fn filmed(
    surfaces: &[scene::Surface],
    shots: &[scene::Shot],
    (width, height): (usize, usize),
) -> Option<(TempDir, KerfMcpRouter)> {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return None;
    }
    let directory = TempDir::new().unwrap();
    let mut encoder = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-r", "30"])
        .args(["-s", &format!("{width}x{height}"), "-i", "-"])
        .args(["-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv444p"])
        .arg(directory.path().join("shot.mp4"))
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("ffmpeg");
    let mut stdin = encoder.stdin.take().unwrap();
    for shot in shots {
        stdin
            .write_all(&scene::picture(surfaces, shot, width, height))
            .unwrap();
    }
    drop(stdin);
    assert!(encoder.wait().unwrap().success());
    let frames = shots.len();
    std::fs::write(
        directory.path().join("project.kerf"),
        format!(
            "kerf 1\nproject name=\"Filmed\"\nmedia m1 path=\"shot.mp4\" rate=30 dur={frames}f video={width}x{height}\nsequence s1 rate=30 size={width}x{height}\n  track V1 video\n    clip plate media=m1 at=0f dur={frames}f\n"
        ),
    )
    .unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(directory.path(), Some(&kerfd)).unwrap();
    Some((directory, router))
}

/// The middle of what is pure magenta in a frame of the sequence.
fn marker_at(project: &kerf_model::Project, at: i64) -> Option<[f64; 2]> {
    let frame = kerf_engine::still::draw_frame(
        project,
        Some("s1"),
        kerf_model::Rate::FPS_30.time_of(at),
        1.0,
    )
    .unwrap();
    let (mut x, mut y, mut count) = (0.0, 0.0, 0.0);
    for (index, pixel) in frame.rgba.chunks_exact(4).enumerate() {
        if pixel[0] > 200 && pixel[1] < 90 && pixel[2] > 200 {
            x += (index % frame.width as usize) as f64 + 0.5;
            y += (index / frame.width as usize) as f64 + 0.5;
            count += 1.0;
        }
    }
    (count > 0.0).then(|| [x / count, y / count])
}

#[test]
fn look_solves_a_shots_camera_and_a_clip_stays_where_it_is_pinned() {
    let (width, height) = (480, 270);
    let (w, h) = (width as f64, height as f64);
    // Wide, far from the 40 degrees a document's camera starts at.
    let focal = 260.0;
    let shots = scene::across_the_room(36, focal);
    let room = scene::room();
    let Some((directory, router)) = filmed(&room, &shots, (width, height)) else {
        return;
    };
    let started = Instant::now();
    let solved = ok(
        &router,
        "look",
        json!({"track": {"camera": true}, "from": "0f", "to": "36f"}),
    );
    eprintln!(
        "look took {:.0} ms a frame: {}",
        started.elapsed().as_secs_f64() * 1000.0 / 36.0,
        solved["fit"]
    );
    assert_eq!(solved["solved"], true, "{solved}");
    assert_eq!(solved["frames"], 36, "{solved}");
    assert!(solved["lostAt"].is_null(), "{solved}");
    assert_eq!(solved["solvedFrames"], 36, "{solved}");
    assert_eq!(solved["lensKnown"], true, "{solved}");
    let true_fov = (2.0 * (h / 2.0 / focal).atan()).to_degrees();
    let fov = solved["fov"].as_f64().unwrap();
    assert!((fov - true_fov).abs() < 1.0, "{fov} not {true_fov}");
    assert!(solved["fit"]["error"].as_f64().unwrap() < 0.4, "{solved}");
    assert!(
        solved["fit"]["agreeing"].as_f64().unwrap() > 0.6,
        "{solved}"
    );
    // Few keyframes for a smooth move, and every one of its channels is in the document.
    assert!(solved["keyframes"].as_i64().unwrap() < 36, "{solved}");
    let script = solved["script"].as_str().unwrap();
    assert!(script.contains("gen=camera@1"), "{script}");
    for name in ["x", "y", "z", "pan", "tilt", "roll"] {
        assert!(script.contains(&format!(" {name}={{0f:")), "{script}");
    }
    assert!(script.contains(" fov="), "{script}");

    // A place it found, which stays in view, on a spot of the scene that is known.
    let spot_of = |place: &Value| {
        let at = [
            (place["at"][0].as_f64().unwrap() - 0.5) * w,
            (place["at"][1].as_f64().unwrap() - 0.5) * h,
        ];
        scene::point_at(&room, &shots[0], at)
    };
    let in_view = |spot: [f64; 3]| {
        shots.iter().all(|shot| {
            shot.shows(spot)
                .is_some_and(|at| at[0].abs() < w / 2.0 - 12.0 && at[1].abs() < h / 2.0 - 12.0)
        })
    };
    let points = solved["points"].as_array().unwrap();
    assert!(points.len() >= 10, "{solved}");
    let place = points
        .iter()
        .filter(|place| place["frame"] == 0 && place["lastFrame"] == 35)
        .find(|place| spot_of(place).is_some_and(in_view))
        .unwrap_or_else(|| panic!("nothing stays in view: {solved}"));
    let spot = spot_of(place).unwrap();

    // On a clip, keyframes count from its start, and the range need not be all of it.
    let part = ok(
        &router,
        "look",
        json!({"track": {"camera": true, "clip": "plate"}, "from": "6f", "to": "30f"}),
    );
    assert_eq!(part["frames"], 25, "{part}");
    let line = part["script"].as_str().unwrap();
    assert!(line.starts_with("set plate x={6f:"), "{line}");

    let camera = ok(&router, "edit", json!({ "script": script }));
    let proposal = camera["proposal"]["id"]
        .as_str()
        .or_else(|| camera["proposal"].as_str())
        .unwrap_or_else(|| panic!("{camera}"));
    let pinned = ok(
        &router,
        "edit",
        json!({
            "extend": proposal,
            "script": format!(
                "track add V3 video\noverwrite gen=solid dur=36f at=0f on=V3 as=pin\nset pin color=#ff00ff scale=0.025 three_d=true x={} y={} z={}",
                place["x"], place["y"], place["z"]
            ),
        }),
    );
    let proposal = pinned["proposal"]["id"]
        .as_str()
        .or_else(|| pinned["proposal"].as_str())
        .unwrap_or(proposal);
    let document = ok(&router, "look", json!({ "proposal": proposal }));
    let text = document["document"]
        .as_str()
        .unwrap_or_else(|| panic!("{document}"));
    let mut project = kerf_lang::parse_document(text, kerf_model::Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    kerf_runtime::resolve_media_paths(&mut project, directory.path());

    // On each frame the clip is where the known spot shows through the known camera.
    let mut worst = 0.0f64;
    for frame in (0..36).step_by(5) {
        let found = marker_at(&project, frame as i64)
            .unwrap_or_else(|| panic!("no marker on frame {frame}"));
        let known = shots[frame].shows(spot).unwrap();
        let known = [known[0] + w / 2.0, known[1] + h / 2.0];
        let off = (found[0] - known[0]).hypot(found[1] - known[1]);
        worst = worst.max(off);
    }
    eprintln!("the pinned clip was at worst {worst:.2} pixels off");
    assert!(worst < 1.5, "at worst {worst} pixels off");
}

#[test]
fn look_says_plainly_when_a_shot_cannot_be_solved() {
    let (width, height) = (320, 180);
    // A camera on a tripod, panning across a flat wall and then a room.
    let pan = |frame: usize| {
        let a = -0.25 + 0.5 * frame as f64 / 23.0;
        scene::Shot::looking(
            [0.0, 0.0, 0.0],
            [10.0 * f64::sin(a), 0.0, 10.0 * f64::cos(a)],
            0.0,
            260.0,
        )
    };
    let shots: Vec<scene::Shot> = (0..24).map(pan).collect();
    let Some((_directory, router)) = filmed(&scene::wall(), &shots, (width, height)) else {
        return;
    };
    let turned = ok(
        &router,
        "look",
        json!({"track": {"camera": true}, "from": "0f", "to": "24f"}),
    );
    assert_eq!(turned["solved"], false, "{turned}");
    assert!(
        turned["reason"].as_str().unwrap().contains("only turns"),
        "{turned}"
    );
    assert!(turned["script"].is_null(), "{turned}");

    let held = vec![pan(0); 12];
    let Some((_directory, router)) = filmed(&scene::room(), &held, (width, height)) else {
        return;
    };
    let still = ok(
        &router,
        "look",
        json!({"track": {"camera": true}, "from": "0f", "to": "12f"}),
    );
    assert_eq!(still["solved"], false, "{still}");
    assert!(
        still["reason"].as_str().unwrap().contains("moves"),
        "{still}"
    );

    for refused in [
        json!({"track": {"camera": true, "x": 0.5, "y": 0.5}, "from": "0f", "to": "2f"}),
        json!({"track": {"camera": true, "model": "classical"}, "from": "0f", "to": "2f"}),
        json!({"track": {"camera": true}, "from": "0f"}),
        json!({"track": {"camera": true, "clip": "nobody"}, "from": "0f", "to": "2f"}),
        json!({"track": {"camera": true, "fov": 0.2}, "from": "0f", "to": "2f"}),
        json!({"track": {"x": 0.5, "y": 0.5, "fov": 40.0}, "from": "0f", "to": "2f"}),
    ] {
        assert_eq!(
            error(&router, "look", refused.clone())["code"],
            "INVALID_ARGUMENTS",
            "{refused}"
        );
    }
}
