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

/// A router on `directory` for a test. What it lets an agent do is the default, every edit proposed
/// and no making run, whatever the person running the tests lets their own agents do in their
/// `~/.kerf/agent.json`: a test that wants another way says so with `set_permissions`.
fn router_on(directory: &Path, kerfd: Option<&Path>) -> KerfMcpRouter {
    let router = KerfMcpRouter::new(directory, kerfd).unwrap();
    router.set_permissions(Some(kerf_mcp::permissions::Permissions::default()));
    router
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
    let router = router_on(directory.path(), Some(&kerfd));
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
    // What the preset calls `c1` and `a1` are clips of this project under
    // ids of its own. The answer says which each became, and those are the
    // names the script places them by; the target sequence keeps its own.
    let ids = proposed["ids"].as_object().unwrap();
    let script = proposed["script"].as_str().unwrap();
    for template in ["c1", "a1"] {
        let placed = ids[template].as_str().unwrap();
        assert_ne!(placed, template, "{ids:?}");
        assert!(
            script.contains(&format!("as={placed}")),
            "{template} became {placed}: {script}"
        );
    }
    assert_eq!(ids["s1"], "s1", "{ids:?}");
    // A slot became the media it was bound to.
    assert_eq!(ids["slot1"], "m1", "{ids:?}");
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
    let router = router_on(directory.path(), None);
    let names = router
        .list_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["look", "edit", "render", "note", "make"]);
    let agent = router.agent_tools();
    assert_eq!(
        agent.keys().collect::<Vec<_>>(),
        ["edit", "look", "make", "note", "render"]
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
    // Each face names the OpenType features it has, by tag: Inter has tabular figures and no
    // small capitals, and the test font has none at all.
    let has = |font: &Value, tag: &str| {
        let features = font["faces"][0]["features"].as_array().expect("features").clone();
        features.contains(&json!(tag))
    };
    assert!(has(&plain, "tnum") && has(&plain, "kern") && !has(&plain, "smcp"), "{plain}");
    assert!(has(&ok(&router, "look", json!({"font":"serif"})), "smcp"));
    assert_eq!(font["faces"][0]["features"], json!([]));
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
    assert_eq!(content.len(), 3, "{response}");
    // Each image comes after a line that says which it is.
    assert_eq!(content[1]["text"], "image 0: frames[0] at 1s");
    assert_eq!(content[2]["type"], "image");
    assert_eq!(content[2]["mimeType"], "image/jpeg");
    let frame = &response["result"]["structuredContent"]["result"]["frames"][0];
    assert!(frame.get("imageBase64").is_none());
    assert_eq!(frame["image"], 0);

    // An edit's sheets come the one before the edit first, each named with
    // the frames it shows.
    let response = server
        .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "edit", "arguments": {"script": "trim c1 out -30f ripple"}}}))
        .unwrap();
    let preview = &response["result"]["structuredContent"]["result"]["preview"];
    assert_eq!(preview["before"]["image"], 0, "{preview}");
    assert_eq!(preview["after"]["image"], 1, "{preview}");
    let content = response["result"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 5, "{response}");
    let label = |index: usize| content[index]["text"].as_str().unwrap();
    let frames = |side: &str| {
        preview[side]["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| frame.as_str().unwrap())
            .collect::<Vec<_>>()
            .join(", ")
    };
    assert_eq!(
        label(1),
        format!("image 0: preview.before, frames {}", frames("before"))
    );
    assert_eq!(
        label(3),
        format!("image 1: preview.after, frames {}", frames("after"))
    );
    assert_eq!(content[2]["type"], "image");
    assert_eq!(content[4]["type"], "image");
}

/// `look` reads a position of the sequence as `edit` does: a clip's or a
/// marker's edge, the end of the sequence, where a clip plays a moment of its
/// media, each with an amount.
#[test]
fn look_takes_the_anchors_edit_takes_for_a_position_of_the_sequence() {
    let Some((_directory, router)) = router_with(
        "    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00\n  marker beat1 at=00:00:01:00 name=\"beat\"\n",
    ) else {
        return;
    };
    let frames = ok(
        &router,
        "look",
        json!({"at": ["c1.end-1f", "c1.start+15f", "beat1", "end-2f", "c1@00:00:02:00"], "width": 160}),
    );
    let frames = frames["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 5);
    assert!(frames.iter().all(has_image), "{frames:?}");
    assert_eq!(frames[0]["at"], "c1.end-1f");

    // The last frame of the clip is blue and its first second red: the
    // anchors land where they say.
    let scopes = ok(
        &router,
        "look",
        json!({"scopes": true, "at": ["c1.end-1f", "c1.start+15f"]}),
    );
    let readings = scopes["readings"].as_array().unwrap();
    assert_eq!(readings.len(), 2, "{scopes}");
    assert_ne!(readings[0]["scopes"], readings[1]["scopes"], "{scopes}");

    let sheet = ok(
        &router,
        "look",
        json!({"sheet": 2, "from": "c1.start+1s", "to": "c1.end-1s", "width": 160}),
    );
    assert_eq!(
        sheet["frames"],
        json!(["00:00:01:15", "00:00:02:15"]),
        "{sheet}"
    );

    // The end of the sequence is past its last frame, and an anchor that
    // names nothing says what it looked for.
    let past = error(&router, "look", json!({"at": ["c1.end"]}));
    assert_eq!(past["code"], "INVALID_ARGUMENTS");
    let unknown = error(&router, "look", json!({"at": ["c9.end-1f"]}));
    assert_eq!(unknown["code"], "INVALID_ARGUMENTS");
    assert!(
        unknown["message"].as_str().unwrap().contains("c9"),
        "{unknown}"
    );
    // A media's own positions take no anchors of the sequence.
    let source = error(&router, "look", json!({"media": "m1", "at": ["c1.end-1f"]}));
    assert_eq!(source["code"], "INVALID_ARGUMENTS");
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
fn a_blocking_note_holds_a_full_render_back_and_not_a_draft() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let note = ok(
        &router,
        "note",
        json!({"body": "The flare hides the logo", "at": "1s", "blocking": true}),
    );
    let id = note["comment"]["id"].as_str().unwrap();
    let refused = error(&router, "render", json!({"quality": "full"}));
    assert_eq!(refused["code"], "BLOCKING_NOTES", "{refused}");
    let said = message(&refused);
    assert!(
        said.contains("\"The flare hides the logo\"")
            && said.contains(" at 00:00:01:00")
            && said.contains(id)
            && said.contains("resolve it in the review"),
        "{said}"
    );
    // A draft is a check, not a delivery.
    let draft = ok(&router, "render", json!({"quality": "draft"}));
    let finished = finish(&router, draft["job"].as_str().unwrap());
    assert_eq!(finished["status"], "completed", "{finished}");
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
    // Each frame has its time in its corner, white on a dark box over the red picture, where
    // this ffmpeg can draw text.
    let filters = Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .unwrap();
    if String::from_utf8_lossy(&filters.stdout).contains(" drawtext ") {
        let red_of = |crop: &str| {
            let pixel = Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-i"])
                .arg(sheet["path"].as_str().unwrap())
                .args(["-vf", &format!("crop={crop},scale=1:1")])
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            i32::from(pixel.stdout[0])
        };
        let (corner, middle) = (red_of("40:8:12:12"), red_of("40:8:140:100"));
        assert!(corner + 40 < middle, "corner {corner}, middle {middle}");
    }
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
    // 91 frames of sound are 48,533⅓ samples to a transcriber, mixed as the 48,533 whole ones
    // that end a little before the range does: it is heard to its end all the same.
    let odd = ok(&router, "look", json!({"transcript": true, "to": "91f"}));
    assert_eq!(odd["to"], "00:00:03:01");
    assert!(odd["transcript"].is_array(), "{odd}");
    let mixed = error(&router, "look", json!({"transcript": true, "at": ["1s"]}));
    assert_eq!(mixed["code"], "INVALID_ARGUMENTS");
}

#[test]
fn a_transcript_nothing_here_can_make_is_refused_before_any_work() {
    // With a model set, the test above hears the sequence.
    if std::env::var_os("KERF_WHISPER_MODEL").is_some() {
        return;
    }
    let Some((directory, router)) = router() else {
        return;
    };
    for arguments in [
        json!({"transcript": true}),
        json!({"media": "m1", "transcript": true}),
    ] {
        let refused = error(&router, "look", arguments);
        assert_eq!(refused["code"], "CAPABILITY_UNAVAILABLE", "{refused}");
        assert_eq!(refused["details"]["capability"], "transcription");
    }
    // The program's sound was not rendered for a transcript nobody could make of it.
    let rendered: Vec<String> = std::fs::read_dir(directory.path().join(".kerf/cache/derived"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("program-sound"))
        .collect();
    assert!(rendered.is_empty(), "{rendered:?}");
}

#[test]
fn look_says_how_loud_the_sequence_plays() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // A 440 Hz tone an eighth of full scale high, for three of four seconds.
    let heard = ok(&router, "look", json!({"loudness": true}));
    assert_eq!(heard["sequence"], "s1");
    let loudness = &heard["loudness"];
    let integrated = loudness["integrated"].as_f64().unwrap();
    assert!((-24.0..-18.0).contains(&integrated), "{heard}");
    assert_eq!(loudness["target"], -23.0);
    let peak = loudness["truePeak"].as_f64().unwrap();
    assert!((-20.0..-17.0).contains(&peak), "{heard}");
    assert_eq!(loudness["limit"], -1.0);
    assert!(loudness["momentaryMax"]["lufs"].is_number(), "{heard}");
    assert!(loudness["momentaryMax"]["ends"].is_string(), "{heard}");
    assert_eq!(loudness["seconds"], 4.0);
    // Asked again of the same revision, it is not measured again.
    assert_eq!(ok(&router, "look", json!({"loudness": true})), heard);

    // What a proposal would make of it: twelve decibels down, as it says.
    let proposal = ok(&router, "edit", json!({"script": "set a1 gain=-12"}));
    let quieter = ok(
        &router,
        "look",
        json!({"loudness": true, "proposal": proposal["proposal"]}),
    );
    let down = quieter["loudness"]["integrated"].as_f64().unwrap();
    assert!((down - (integrated - 12.0)).abs() < 0.3, "{quieter}");
    assert!(
        (quieter["loudness"]["truePeak"].as_f64().unwrap() - (peak - 12.0)).abs() < 0.3,
        "{quieter}"
    );

    // It is of a whole sequence: a media's own is in its fact sheet.
    for arguments in [
        json!({"loudness": true, "media": "m1"}),
        json!({"loudness": true, "at": ["1s"]}),
        json!({"loudness": true, "from": "1s", "to": "2s"}),
    ] {
        let refused = error(&router, "look", arguments);
        assert_eq!(refused["code"], "INVALID_ARGUMENTS", "{refused}");
    }
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
    // Built on the first version again, a script would leave out what the
    // second added and become the newest all the same: it is refused, and
    // told which to build on. Nothing is staged.
    let superseded = error(
        &router,
        "edit",
        json!({"script": "trim c1 in +1f", "extend": first_id}),
    );
    assert_eq!(superseded["code"], "PROPOSAL_SUPERSEDED", "{superseded}");
    let message = superseded["message"].as_str().unwrap();
    assert!(
        message.starts_with(&format!(
            "proposal {first_id} is version 1 of a proposal now at version 2"
        )),
        "{message}"
    );
    assert!(
        message.contains(&format!("pass the newest, {id}, as `extend`")),
        "{message}"
    );
    let open = router.runtime().get_state().unwrap().proposals;
    assert_eq!(open[0].proposal_id, id, "{open:#?}");
    assert_eq!(open[0].versions.len(), 2, "{open:#?}");

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
    // Of a proposal, each reading has what the project reads there beside it: a grade is
    // compared in one look. Of the project there is nothing to put beside it.
    let both = ok(
        &router,
        "look",
        json!({"at": ["1s"], "scopes": true, "proposal": proposal}),
    );
    assert_eq!(both["readings"][0]["before"], before, "{both}");
    let alone = ok(&router, "look", json!({"at": ["1s"], "scopes": true}));
    assert!(alone["readings"][0].get("before").is_none(), "{alone}");
    // An edit reads them itself where it is asked to: the grade and its numbers in one call.
    let graded = ok(
        &router,
        "edit",
        json!({"revise": proposal, "script": "fx c1 add color.basic saturation=0", "scopes": ["1s"]}),
    );
    assert_eq!(graded["scopes"][0]["at"], "1s", "{graded}");
    assert_eq!(graded["scopes"][0]["scopes"], after, "{graded}");
    assert_eq!(graded["scopes"][0]["before"], before, "{graded}");
    // A position that is not on the sequence is said, and the edit still stands.
    let off = ok(
        &router,
        "edit",
        json!({"revise": graded["proposal"], "script": "fx c1 add color.basic saturation=0", "scopes": ["99s"]}),
    );
    assert!(
        off["scopes"]["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("outside sequence")),
        "{off}"
    );
    assert_eq!(off["status"], "awaiting_review", "{off}");

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
fn look_reads_an_hdr_sequences_scopes_by_its_signal() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // A white solid is SDR white in a sequence of either kind, which only one of them can say.
    let script =
        "track add V2 video\noverwrite gen=solid dur=12f at=0f on=V2 as=w\nset w color=#ffffff";
    let added = ok(&router, "edit", json!({"script": script}));
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    let read = |proposal: &str| {
        ok(
            &router,
            "look",
            json!({"scopes": true, "at": ["2f"], "proposal": proposal}),
        )["readings"][0]["scopes"]
            .clone()
    };
    let sdr = read(proposal);
    assert_eq!(sdr["luma"]["max"], 100.0, "{sdr}");
    assert!(sdr.get("hdr").is_none(), "{sdr}");
    let extended = ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": "sequence set s1 color=rec2100-pq"}),
    );
    let proposal = extended["proposal"]
        .as_str()
        .unwrap_or_else(|| panic!("{extended}"));
    let hdr = read(proposal);
    let held = &hdr["hdr"];
    assert!(
        (hdr["luma"]["max"].as_f64().unwrap() - 58.07).abs() < 1.0,
        "{hdr}"
    );
    assert!(
        (held["peakNits"].as_f64().unwrap() - 203.0).abs() < 5.0,
        "{hdr}"
    );
    assert!(
        (held["referenceWhite"].as_f64().unwrap() - 58.07).abs() < 0.1,
        "{hdr}"
    );
    assert_eq!(held["outsideBt709"], 0.0, "{hdr}");
    // And a push past white reads as light a monitor's picture could not say.
    let extended = ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": "fx w add color.lumetri exposure=1"}),
    );
    let proposal = extended["proposal"]
        .as_str()
        .unwrap_or_else(|| panic!("{extended}"));
    let pushed = read(proposal);
    assert!(
        pushed["hdr"]["peakNits"].as_f64().unwrap() > 300.0,
        "{pushed}"
    );
    assert_eq!(pushed["hdr"]["overWhite"], 1.0, "{pushed}");
}

#[test]
fn look_says_which_text_is_too_faint_from_the_frames() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // The shot is red and then blue: a white title reads over both, and a red one over the red not at all.
    let added = ok(
        &router,
        "edit",
        json!({"script": "track add V2 video\noverwrite gen=title dur=30f at=0f on=V2 as=fine\nset fine text=\"Plain\" color=#ffffff\noverwrite gen=title dur=30f at=30f on=V2 as=lost\nset lost text=\"Plain\" color=#ff0000"}),
    );
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    let read = ok(
        &router,
        "look",
        json!({"contrast": true, "proposal": proposal}),
    );
    assert_eq!(
        (read["read"].as_u64(), read["of"].as_u64()),
        (Some(2), Some(2)),
        "{read}"
    );
    let found = read["contrast"].as_array().unwrap();
    assert_eq!(found.len(), 1, "{read}");
    assert_eq!(found[0]["code"], "text.contrast");
    assert_eq!(found[0]["entity"], "lost");
    // Where it was read is said as a position a script takes: a timecode of the sequence, on a
    // frame, and not the moment between two frames that was looked at, in flicks.
    let said = found[0]["message"].as_str().unwrap_or_default();
    assert!(said.contains("does not show at 00:00:01:03:"), "{said}");
    assert!(!said.contains("fl:"), "{said}");
    // Only where it was asked.
    let early = ok(
        &router,
        "look",
        json!({"contrast": true, "proposal": proposal, "from": "0f", "to": "20f"}),
    );
    assert_eq!(early["of"], 1, "{early}");
    assert_eq!(early["contrast"].as_array().unwrap().len(), 0, "{early}");
    for refused in [
        json!({"contrast": true, "at": ["0f"]}),
        json!({"contrast": true, "media": "m1"}),
        json!({"contrast": true, "sheet": 4}),
    ] {
        assert_eq!(
            error(&router, "look", refused.clone())["code"],
            "INVALID_ARGUMENTS",
            "{refused}"
        );
    }
}

#[test]
fn look_gives_a_fact_sheet_of_media_and_says_what_it_leaves_out() {
    let Some((directory, router)) = router() else {
        return;
    };
    let sheet = ok(&router, "look", json!({"media": "m1", "facts": true}));
    let facts = &sheet["facts"];
    // A flat red and a flat blue, cut at 2 seconds, with a tone that stops at 3.
    let shots = facts["shots"]
        .as_array()
        .unwrap_or_else(|| panic!("{sheet}"));
    assert_eq!(shots.len(), 2, "{sheet}");
    assert_eq!(shots[0]["camera"], json!(["unknown"]), "{sheet}");
    assert_eq!(facts["cuts"], 1, "{sheet}");
    assert_eq!(facts["pauses"]["count"], 1, "{sheet}");
    assert!(facts["loudness"]["integrated"].is_number(), "{sheet}");
    assert!(facts["loudness"]["truePeak"].is_number(), "{sheet}");
    assert_eq!(facts["sound"]["rate"], 48000, "{sheet}");
    assert_eq!(facts["picture"]["width"], 320, "{sheet}");
    for left_out in ["faces", "subject", "onScreenText"] {
        assert!(
            facts["notMeasured"][left_out].is_string(),
            "{left_out}: {sheet}"
        );
    }
    // The second look reads what the first found.
    let again = ok(&router, "look", json!({"media": "m1", "facts": true}));
    assert_eq!(again["facts"], sheet["facts"]);
    // Without asking, it is as it was.
    let plain = ok(&router, "look", json!({"media": "m1"}));
    assert!(plain.get("facts").is_none(), "{plain}");
    // A window going right over a test pattern, at 40 pixels a second of 320, an eighth of the frame.
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=black:size=640x360:rate=30:duration=3,geq=lum='128+60*sin(X*0.21)*sin(Y*0.13)+40*sin(X*0.07+Y*0.11)':cb=128:cr=128",
        ])
        .args(["-vf", "crop=320:180:x='t*40':y=90"])
        .arg(directory.path().join("pan.mp4"))
        .status()
        .unwrap();
    assert!(status.success());
    let added = ok(
        &router,
        "edit",
        json!({"script": "media add m2 path=\"pan.mp4\" rate=30 dur=90f video=320x180"}),
    );
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    let panning = ok(
        &router,
        "look",
        json!({"media": "m2", "facts": true, "proposal": proposal}),
    );
    let facts = &panning["facts"];
    let shots = facts["shots"]
        .as_array()
        .unwrap_or_else(|| panic!("{panning}"));
    assert_eq!(shots.len(), 1, "{panning}");
    assert_eq!(shots[0]["camera"][0], "pan right", "{panning}");
    let pan = shots[0]["pan"].as_f64().unwrap();
    assert!((pan - 0.125).abs() < 0.02, "{pan}: {panning}");
    assert!(facts.get("loudness").is_none(), "{panning}");
    assert!(facts.get("sound").is_none(), "{panning}");
    assert_eq!(
        error(&router, "look", json!({"facts": true}))["code"],
        "INVALID_ARGUMENTS"
    );
}

#[test]
fn look_measures_how_soft_an_objects_edge_is_at_each_corner() {
    let Some((_directory, router)) = router() else {
        return;
    };
    // A white clip over the red shot, cut by a mask that is sharp on its left and falls off over a tenth
    // of the frame on its right.
    let added = ok(
        &router,
        "edit",
        json!({"script": "track add V2 video\noverwrite gen=solid dur=12f at=0f on=V2 as=sq\nset sq color=#ffffff\nmask sq add soft points=\"0.3,0.25,0 0.7,0.25,64 0.7,0.75,64 0.3,0.75,0\""}),
    );
    let proposal = added["proposal"]["id"]
        .as_str()
        .or_else(|| added["proposal"].as_str())
        .unwrap_or_else(|| panic!("{added}"));
    let followed = ok(
        &router,
        "look",
        json!({
            "track": {"inside": [[0.45, 0.5]], "outside": [[0.05, 0.05]], "clip": "sq"},
            "from": "0f",
            "to": "4f",
            "proposal": proposal,
        }),
    );
    assert!(followed["lostAt"].is_null(), "{followed}");
    // With a clip the outline is in the line that masks it, and nowhere else.
    assert!(followed.get("outline").is_none(), "{followed}");
    let script = followed["script"].as_str().unwrap();
    let outline = script.split_once("points=").unwrap().1;
    let last = outline
        .trim_matches(['{', '}'])
        .split(' ')
        .next_back()
        .and_then(|key| key.split(':').nth(1))
        .unwrap();
    let corners: Vec<[f64; 3]> = last
        .split(';')
        .map(|corner| {
            let values: Vec<f64> = corner.split(',').map(|v| v.parse().unwrap()).collect();
            [values[0], values[1], values[2]]
        })
        .collect();
    // The corners down its left side and down its right: the left, straight and as hard all the way,
    // is the two at its ends.
    let mean = |wanted: &dyn Fn(f64) -> bool| {
        let widths: Vec<f64> = corners
            .iter()
            .filter(|corner| wanted(corner[0]))
            .map(|corner| corner[2])
            .collect();
        assert!(!widths.is_empty(), "{corners:?}");
        widths.iter().sum::<f64>() / widths.len() as f64
    };
    let (left, right) = (mean(&|x| x < 0.31), mean(&|x| x > 0.65));
    assert!(corners.len() < 40, "{corners:?}");
    assert!(left < 12.0, "{left}: {corners:?}");
    assert!(right > 40.0, "{right}: {corners:?}");
    // What it gives is a line a document takes.
    ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": script}),
    );
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
    // A square with hard edges says nothing of how soft they are, and its document stays as it was.
    let first_corner = script.split(['{', ';']).nth(1).unwrap();
    assert_eq!(first_corner.matches(',').count(), 1, "{first_corner}");
    ok(
        &router,
        "edit",
        json!({"extend": proposal, "script": script}),
    );
    // A square is the corners by its own, the same ones in each keyframe, and not the forty it
    // was followed by; and its outline is given once, in the line.
    assert!(followed.get("outline").is_none(), "{followed}");
    let outline = script.split_once("points=").unwrap().1;
    let counts: Vec<usize> = outline
        .trim_matches(['{', '}'])
        .split(' ')
        .map(|key| key.split(';').count())
        .collect();
    assert_eq!(followed["corners"], counts[0], "{followed}");
    assert!(counts[0] <= 12, "{counts:?}");
    assert!(counts.iter().all(|count| *count == counts[0]), "{counts:?}");
    assert_eq!(followed["keyframes"], counts.len(), "{followed}");
    // Without a clip it is `outline`, the same.
    let alone = ok(
        &router,
        "look",
        json!({
            "track": {"inside": [[0.3, 0.5]], "outside": [[0.05, 0.05]]},
            "from": "0f",
            "to": "24f",
            "proposal": proposal,
        }),
    );
    assert!(alone.get("script").is_none(), "{alone}");
    assert_eq!(alone["outline"].as_str(), Some(outline), "{alone}");
    // Where it ends, it has moved with the square: 0.32 of the frame to the right, at its middle.
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
    let found = ok(&router, "look", json!({"effects": "film grain"}));
    let doc = found["effects"][0]["doc"].as_str().unwrap();
    assert!(doc.contains("Film") && doc.contains("grain"), "{found}");
    let retro = ok(&router, "look", json!({"effects": "retro tv transition"}));
    assert_eq!(retro["effects"][0]["id"], "tv-power", "{retro}");
    // An effect EffectCraft had says what replaced it.
    let gone = error(&router, "look", json!({"effect": "stylize.glow"}));
    assert!(gone.to_string().contains("the prompt and references"), "{gone}");
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
        json!({"script": "fx c1 add stylize.mosaic\n"}),
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
    let kerf = router_on(router.directory(), None);
    let beside = ok(&kerf, "edit", json!({"script": "trim c1 in +5f"}));
    assert_eq!(beside["author"], "Kerf");
    let beside_id = beside["proposal"].as_str().unwrap().to_owned();
    let cursor = router_on(router.directory(), None);
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
    let kerf = router_on(router.directory(), None);
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
    let router = router_on(directory.path(), Some(&kerfd));
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
    for (index, pixel) in frame.rgba.as_chunks::<4>().0.iter().enumerate() {
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
fn look_solves_a_lens_that_zooms_and_a_clip_stays_where_it_is_pinned() {
    let (width, height) = (480, 270);
    let (w, h) = (width as f64, height as f64);
    let mut shots = scene::across_the_room(36, 260.0);
    for (frame, shot) in shots.iter_mut().enumerate() {
        let t = frame as f64 / 35.0;
        shot.focal = 220.0 * (1.6f64).powf(t * t * (3.0 - 2.0 * t));
    }
    let room = scene::room();
    let Some((directory, router)) = filmed(&room, &shots, (width, height)) else {
        return;
    };
    let solved = ok(
        &router,
        "look",
        json!({"track": {"camera": true}, "from": "0f", "to": "36f"}),
    );
    assert_eq!(solved["solved"], true, "{solved}");
    assert_eq!(solved["zooms"], true, "{solved}");
    assert!(solved["zoomOrPush"].is_null(), "{solved}");
    let true_fov = (2.0 * (h / 2.0 / 220.0).atan()).to_degrees();
    let fov = solved["fov"].as_f64().unwrap();
    assert!((fov - true_fov).abs() < 2.0, "{fov} not {true_fov}");
    let script = solved["script"].as_str().unwrap();
    assert!(script.contains(" fov={0f:"), "{script}");

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
    let place = solved["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|place| place["frame"] == 0 && place["lastFrame"] == 35)
        .find(|place| spot_of(place).is_some_and(in_view))
        .unwrap_or_else(|| panic!("nothing stays in view: {solved}"));
    let spot = spot_of(place).unwrap();
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
    let mut worst = 0.0f64;
    for frame in (0..36).step_by(5) {
        let found = marker_at(&project, frame as i64)
            .unwrap_or_else(|| panic!("no marker on frame {frame}"));
        let known = shots[frame].shows(spot).unwrap();
        let known = [known[0] + w / 2.0, known[1] + h / 2.0];
        worst = worst.max((found[0] - known[0]).hypot(found[1] - known[1]));
    }
    eprintln!("the pinned clip was at worst {worst:.2} pixels off, under a zoom");
    assert!(worst < 2.5, "at worst {worst} pixels off");
}

#[test]
fn look_says_when_a_shot_cannot_tell_a_zoom_from_a_push_in() {
    let (width, height) = (480, 270);
    let shots: Vec<scene::Shot> = (0..30)
        .map(|frame| {
            let t = frame as f64 / 29.0;
            scene::Shot::looking([0.0, 0.0, 3.0 * t], [0.0, 0.0, 10.0], 0.0, 300.0)
        })
        .collect();
    let Some((_directory, router)) = filmed(&scene::wall(), &shots, (width, height)) else {
        return;
    };
    let solved = ok(
        &router,
        "look",
        json!({"track": {"camera": true}, "from": "0f", "to": "30f"}),
    );
    assert_eq!(solved["solved"], true, "{solved}");
    assert_eq!(solved["zooms"], false, "{solved}");
    let said = solved["zoomOrPush"]
        .as_str()
        .unwrap_or_else(|| panic!("{solved}"));
    assert!(said.contains("cannot tell a zoom from a push in"), "{said}");
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

/// A router on a folder with footage and a document of `lines` under one video track.
fn router_with(lines: &str) -> Option<(TempDir, KerfMcpRouter)> {
    let (directory, router) = router()?;
    drop(router);
    std::fs::write(
        directory.path().join("project.kerf"),
        format!(
            "kerf 1
project name=\"Harness\"
media m1 path=\"shot.mp4\" rate=30 dur=00:00:04:00 video=320x180 audio=1ch@48000
sequence s1 rate=30 size=320x180
  track V1 video
{lines}"
        ),
    )
    .unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = router_on(directory.path(), Some(&kerfd));
    Some((directory, router))
}

fn message(value: &Value) -> String {
    value["message"].as_str().unwrap_or_default().to_owned()
}

/// A media that names a recipe and has no file yet: an agent is told so, and that a person makes
/// it. Once made, nothing is said.
#[test]
fn a_media_whose_recipe_has_not_run_is_named_for_a_person_to_make() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().unwrap();
    std::fs::create_dir(directory.path().join("shots")).unwrap();
    std::fs::write(
        directory.path().join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nrun vork render open.json --out {out}\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("project.kerf"),
        "kerf 1\nproject name=\"Harness\"\nmedia m1 path=\"shots/open.mp4\" rate=30 dur=00:00:04:00 video=320x180 recipe=\"shots/open.kerfrecipe\"\nsequence s1 rate=30 size=320x180\n",
    )
    .unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = router_on(directory.path(), Some(&kerfd));
    let looked = ok(&router, "look", json!({}));
    let to_make = &looked["toMake"];
    assert_eq!(to_make.as_array().map(Vec::len), Some(1), "{looked}");
    assert_eq!(to_make[0]["media"], json!("m1"));
    assert_eq!(to_make[0]["recipe"], json!("shots/open.kerfrecipe"));
    assert_eq!(to_make[0]["why"], json!("its file is not there yet"));
    assert!(
        to_make[0]["make"]
            .as_str()
            .is_some_and(|make| make.contains("kerf make m1") && make.contains("a person")),
        "{to_make}"
    );
    // Its file written after its recipe, it is made.
    std::fs::write(directory.path().join("shots/open.mp4"), "frames").unwrap();
    let looked = ok(&router, "look", json!({}));
    assert!(looked.get("toMake").is_none(), "{looked}");
}

/// What a waiting proposal would have made is said with the proposal, as it is with the project,
/// and a making asked of a media that is still in one says which, and what it waits for.
#[test]
fn a_waiting_proposal_says_what_it_would_have_made() {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join("project.kerf"),
        "kerf 1\nproject name=\"Harness\"\nsequence s1 rate=30 size=320x180\n",
    )
    .unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = router_on(directory.path(), Some(&kerfd));
    let staged = ok(
        &router,
        "edit",
        json!({
            "script": "media add m1 path=\"shots/open.mp4\" rate=30 dur=00:00:04:00 video=320x180 recipe=\"shots/open.kerfrecipe\"",
            "recipes": {"shots/open.kerfrecipe": "kerf-recipe 1\nrun vork render open.json --out {out}\n"},
        }),
    );
    assert_eq!(staged["status"], "awaiting_review", "{staged}");
    let id = staged["proposal"].as_str().unwrap().to_owned();
    // The edit says what it would have made, and that the proposal comes first.
    assert_eq!(staged["toMake"][0]["media"], json!("m1"), "{staged}");
    let hint = staged["toMake"][0]["make"].as_str().unwrap_or_default();
    assert!(
        hint.contains("once this proposal is accepted") && hint.contains("`make`"),
        "{staged}"
    );
    // Looked at again, by the proposal alone, it says the same.
    let looked = ok(&router, "look", json!({"proposal": id}));
    assert_eq!(looked["toMake"][0]["media"], json!("m1"), "{looked}");
    assert_eq!(
        looked["toMake"][0]["why"],
        json!("its file is not there yet"),
        "{looked}"
    );
    // The project has nothing to make until the proposal is accepted.
    let project = ok(&router, "look", json!({}));
    assert!(project.get("toMake").is_none(), "{project}");
    // So a making asked for now says where the media is and what it waits for.
    let refused = error(&router, "make", json!({"media": "m1"}));
    assert_eq!(refused["code"], json!("CANNOT_MAKE"), "{refused}");
    let said = message(&refused);
    assert!(said.contains(&id) && said.contains("accepted"), "{refused}");
    // A media that is nowhere is still only that.
    let nowhere = error(&router, "make", json!({"media": "m9"}));
    assert!(!message(&nowhere).contains("proposal"), "{nowhere}");
}

/// A media a recipe made: an agent sees its takes and which one the media is, and is told what the
/// recipe reads that has changed since, by name. A recipe that cannot run is said with why.
#[cfg(unix)]
#[test]
fn a_medias_takes_are_listed_and_what_changed_since_is_named() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().unwrap();
    let dir = directory.path();
    footage(dir);
    std::fs::create_dir(dir.join("shots")).unwrap();
    std::fs::write(dir.join("shots/scene.txt"), "a blocking").unwrap();
    std::fs::write(
        dir.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nneeds scene.txt\nrun cp ../shot.mp4 {out}\n",
    )
    .unwrap();
    let document = |path: &str| {
        format!(
            "kerf 1\nproject name=\"Harness\"\nmedia m1 path=\"{path}\" rate=30 dur=00:00:04:00 video=320x180 audio=1ch@48000 recipe=\"shots/open.kerfrecipe\"\nsequence s1 rate=30 size=320x180\n"
        )
    };
    // A person makes it twice, and the media is the first take.
    let unmade =
        kerf_lang::parse_document(&document("shots/open.mp4"), kerf_model::Catalog::builtin())
            .unwrap()
            .project;
    let models = kerf_engine::recipe::model::Models::default();
    let token = kerf_engine::CancellationToken::new();
    let make = || {
        let plan = kerf_engine::recipe::plan(&unmade, dir, "m1", &models).unwrap();
        kerf_engine::recipe::make(&unmade, dir, &plan, &models, &token, |_| {}).unwrap()
    };
    let (first, second) = (make(), make());
    std::fs::write(dir.join("project.kerf"), document(&first.stored)).unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = router_on(dir, Some(&kerfd));
    assert!(ok(&router, "look", json!({})).get("toMake").is_none());
    let looked = ok(&router, "look", json!({"media": "m1"}));
    assert_eq!(looked["recipe"], json!("shots/open.kerfrecipe"));
    // The take the media is not has the line that has the media be it, whole: its path alone
    // is not a line, since a take need not be as long as the one before it.
    let line = format!(
        "media replace m1 path={:?} rate=30 dur=120f video=320x180 audio=2ch@48000 recipe=\"shots/open.kerfrecipe\"",
        second.stored
    );
    assert_eq!(
        looked["takes"]["made"],
        json!([
            {"take": 1, "path": first.stored, "chosen": true},
            {"take": 2, "path": second.stored, "chosen": false, "use": line},
        ])
    );
    assert!(
        looked["takes"]["choose"]
            .as_str()
            .is_some_and(|choose| choose.contains("a take's `use`")),
        "{looked}"
    );
    // And it runs as it is, where the path alone is refused.
    let staged = ok(&router, "edit", json!({"script": line}));
    assert!(
        staged["diff"]
            .as_str()
            .is_some_and(|diff| diff.contains(&format!("+media m1 path={:?}", second.stored))),
        "{staged}"
    );
    router
        .runtime()
        .reject_proposal(staged["proposal"].as_str().unwrap())
        .unwrap();
    let short = error(
        &router,
        "edit",
        json!({"script": format!("media replace m1 path={:?}", second.stored)}),
    );
    assert!(message(&short).contains("missing `rate=`"), "{short}");
    // What the recipe reads changes: the media wants making, and says for what.
    std::fs::write(dir.join("shots/scene.txt"), "another blocking").unwrap();
    let looked = ok(&router, "look", json!({}));
    assert_eq!(
        looked["toMake"][0]["why"],
        json!("shots/scene.txt changed after its file was made")
    );
    let looked = ok(&router, "look", json!({"media": "m1"}));
    assert_eq!(
        looked["takes"]["made"][0]["changedSince"],
        json!(["shots/scene.txt"])
    );
    // A recipe that cannot run says why, since an agent may have written it.
    std::fs::write(
        dir.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nrun cp ../shot.mp4\n",
    )
    .unwrap();
    let why = ok(&router, "look", json!({}))["toMake"][0]["why"].to_string();
    assert!(
        why.contains("its recipe cannot run") && why.contains("names no `{out}`"),
        "{why}"
    );
}

/// A recipe that reads, and that a person could still not run: an agent is told what would refuse
/// it when it looks, and when it proposes the media, not after someone has tried to make it.
#[cfg(unix)]
#[test]
fn what_would_refuse_a_making_is_said_before_anyone_is_asked_to_make_it() {
    if !ffmpeg_available() {
        return;
    }
    // No models a person has set up are read.
    std::env::set_var("KERF_MODELS", "/no-such-kerf-models.json");
    let Some((directory, router)) = router() else {
        return;
    };
    let dir = directory.path();
    std::fs::create_dir(dir.join("shots")).unwrap();
    let recipe = |steps: &str| {
        std::fs::write(
            dir.join("shots/open.kerfrecipe"),
            format!("kerf-recipe 1\n{steps}\n"),
        )
        .unwrap();
    };
    // Proposed with a recipe that draws a guide and names it without its braces: the edit is
    // staged, and says what a making of it would be refused for.
    recipe("render s1 {guide.mp4}\nrun cp guide.mp4 {out}\nneeds guide.mp4");
    let staged = ok(
        &router,
        "edit",
        json!({"script": "media add m9 path=\"shots/open.mp4\" rate=30 dur=4s video=320x180 recipe=\"shots/open.kerfrecipe\""}),
    );
    let to_make = &staged["toMake"][0];
    assert_eq!(to_make["media"], "m9", "{staged}");
    assert_eq!(to_make["why"], "its file is not there yet");
    assert_eq!(
        to_make["cannot"],
        "the recipe needs `shots/guide.mp4`, which is not there: `guide.mp4` is a file a step makes, which a later step names in braces, `{guide.mp4}`"
    );
    assert!(
        to_make["make"]
            .as_str()
            .is_some_and(|make| make.starts_with("it cannot be made as it is")),
        "{to_make}"
    );
    let revision = router.runtime().get_state().unwrap().revision;
    router
        .runtime()
        .accept_proposal(
            staged["proposal"].as_str().unwrap(),
            "ui",
            "accept-m9",
            revision,
        )
        .unwrap();

    // The same when it looks. A sequence the project has not:
    recipe("render s7 {out}");
    let looked = ok(&router, "look", json!({}));
    assert_eq!(
        looked["toMake"][0]["cannot"],
        "the recipe renders sequence `s7`, which this project has not"
    );
    // A model that is not set up, without where the person keeps their models.
    recipe("generate model=dawn prompt=\"a boat\"");
    let looked = ok(&router, "look", json!({}));
    assert_eq!(
        looked["toMake"][0]["cannot"],
        "no model `dawn` is set up: none is; a person sets one up in their own models file"
    );
    // Mended, it can be made, and a person is asked to.
    recipe("render s1 {out}");
    let looked = ok(&router, "look", json!({}));
    let to_make = &looked["toMake"][0];
    assert!(to_make.get("cannot").is_none(), "{to_make}");
    assert!(
        to_make["make"]
            .as_str()
            .is_some_and(|make| make.contains("a person runs `kerf make m9`")),
        "{to_make}"
    );
    // A media that is not made yet says so, not what the file system said.
    let unmade = ok(&router, "look", json!({"media": "m9"}));
    assert_eq!(
        unmade["analysis"],
        "unavailable: `shots/open.mp4` is not made yet; a person makes it from its recipe with `kerf make m9`"
    );
}

/// A fresh folder: footage beside the document, and nothing in it yet.
#[test]
fn a_fresh_folder_says_what_footage_it_could_add_and_how() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().unwrap();
    footage(directory.path());
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = router_on(directory.path(), Some(&kerfd));
    // What is there and not in the document.
    let looked = ok(&router, "look", json!({}));
    assert_eq!(looked["unplaced"], json!(["shot.mp4"]), "{looked}");
    // The line to add it, measured from the file, with what a person has no way to guess.
    let file = ok(&router, "look", json!({"file": "shot.mp4"}));
    let line = file["add"].as_str().unwrap_or_default().to_owned();
    assert!(
        line.starts_with("media add shot path=\"shot.mp4\" rate=30"),
        "{file}"
    );
    assert!(line.contains("video=320x180"), "{line}");
    assert!(line.contains("audio="), "{line}");
    assert_eq!(file["seconds"], json!(4.0), "{file}");
    // Every frame of it: four seconds at 30 a second.
    assert!(line.contains(" dur=120f "), "{line}");
    // And it runs as it is.
    let staged = ok(&router, "edit", json!({"script": line}));
    assert!(staged["proposal"].is_string(), "{staged}");
    // A file that is not there says so.
    let missing = error(&router, "look", json!({"file": "nothing.mp4"}));
    assert!(message(&missing).contains("nothing.mp4"), "{missing}");
    // Footage in `media` is listed too, and a folder is looked at in one go: each file with
    // its line, under an id no other has, and the lines together as a script that runs.
    let media = directory.path().join("media");
    std::fs::create_dir(&media).unwrap();
    std::fs::copy(directory.path().join("shot.mp4"), media.join("shot.mp4")).unwrap();
    std::fs::copy(
        directory.path().join("shot.mp4"),
        media.join("wide shot.mp4"),
    )
    .unwrap();
    std::fs::write(media.join("notes.txt"), "not footage").unwrap();
    let looked = ok(&router, "look", json!({}));
    assert_eq!(
        looked["unplaced"],
        json!(["shot.mp4", "media/shot.mp4", "media/wide shot.mp4"]),
        "{looked}"
    );
    let folder = ok(&router, "look", json!({"file": "media"}));
    let files = folder["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{folder}");
    assert_eq!(files[0]["file"], "media/shot.mp4");
    assert_eq!(files[0]["seconds"], json!(4.0), "{folder}");
    let script = folder["script"].as_str().unwrap();
    let lines: Vec<&str> = script.lines().collect();
    assert_eq!(lines.len(), 2, "{script}");
    assert!(
        lines[0].starts_with("media add shot path=\"media/shot.mp4\" rate=30 dur=120f"),
        "{script}"
    );
    assert!(
        lines[1].starts_with("media add wide_shot path=\"media/wide shot.mp4\""),
        "{script}"
    );
    let staged = ok(&router, "edit", json!({"script": script}));
    assert!(staged["proposal"].is_string(), "{staged}");
    // The project's own folder is `.`, and two files of one name get two ids.
    std::fs::create_dir(directory.path().join("more")).unwrap();
    std::fs::copy(
        directory.path().join("shot.mp4"),
        directory.path().join("more/shot.mp4"),
    )
    .unwrap();
    let own = ok(&router, "look", json!({"file": "."}));
    assert_eq!(own["files"][0]["file"], "shot.mp4", "{own}");
    // With the line left short, the refusal says where the rest comes from.
    let short = error(
        &router,
        "edit",
        json!({"script": "media add m9 path=\"shot.mp4\""}),
    );
    assert!(message(&short).contains("`look` with `file`"), "{short}");
}

/// A refused edit says what to do next, not only what is wrong.
#[test]
fn a_refused_edit_says_what_to_do() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let refused = |script: &str| message(&error(&router, "edit", json!({"script": script})));
    let track = refused("overwrite gen=solid at=0f dur=24f on=V9");
    assert!(track.contains("V1"), "names the tracks it has: {track}");
    assert!(track.contains("track add V9 video"), "{track}");
    let audio = refused("overwrite media=m1 at=0f dur=24f on=A7");
    assert!(audio.contains("track add A7 audio"), "{audio}");
    let generator = refused("overwrite gen=solidd at=0f dur=24f on=V1");
    assert!(generator.contains("`solid`"), "{generator}");
    let transition = refused("transition add from=c1 to=c1 fx=dissolvee dur=4f");
    assert!(transition.contains("`dissolve`"), "{transition}");
    let effect = refused("fx c1 add blur.gausian");
    assert!(effect.contains("`blur.gaussian`"), "{effect}");
    let nothing_near = refused("fx c1 add zzzzzz");
    assert!(
        nothing_near.contains("`look` with `effects`"),
        "{nothing_near}"
    );
    let clip = refused("set c2 opacity=0.5");
    assert!(clip.contains("`c1`"), "{clip}");
    let sequence = refused("sequence add s1 rate=24 size=320x180");
    assert!(sequence.contains("sequence set s1"), "{sequence}");
}

/// What only a pending proposal has is found by saying so, wherever an agent trips on it.
#[test]
fn what_a_pending_proposal_holds_is_pointed_to() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let staged = ok(
        &router,
        "edit",
        json!({"script": "media add m2 path=\"shot.mp4\" rate=30 dur=4s video=320x180\noverwrite gen=solid at=4s dur=30f on=V1 as=later"}),
    );
    let id = staged["proposal"].as_str().unwrap().to_owned();
    // Building on it without saying so.
    let on = message(&error(
        &router,
        "edit",
        json!({"script": "set later opacity=0.5"}),
    ));
    assert!(on.contains(&id) && on.contains("`extend`"), "{on}");
    let media = message(&error(
        &router,
        "edit",
        json!({"script": "overwrite media=m2 at=0f dur=30f on=V1"}),
    ));
    assert!(media.contains(&id) && media.contains("`extend`"), "{media}");
    // Looking at it without saying so.
    let looked = message(&error(&router, "look", json!({"media": "m2"})));
    assert!(
        looked.contains(&id) && looked.contains("`proposal`"),
        "{looked}"
    );
    let frame = message(&error(&router, "look", json!({"at": ["4.5s"]})));
    assert!(
        frame.contains(&id) && frame.contains("`proposal`"),
        "{frame}"
    );
    // `revise` starts again from the project, which is not what it looks like from here.
    let revising = message(&error(
        &router,
        "edit",
        json!({"script": "set later opacity=0.5", "revise": id}),
    ));
    assert!(
        revising.contains(&id) && revising.contains("`extend`"),
        "{revising}"
    );
    // With `extend` the same line goes through.
    let extended = ok(
        &router,
        "edit",
        json!({"script": "set later opacity=0.5", "extend": id}),
    );
    assert!(extended["proposal"].is_string(), "{extended}");
}

#[test]
fn rendering_nothing_is_refused_and_says_where_the_cut_is() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().unwrap();
    footage(directory.path());
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = router_on(directory.path(), Some(&kerfd));
    let empty = message(&error(&router, "render", json!({})));
    assert!(empty.contains("empty"), "{empty}");
    let staged = ok(
        &router,
        "edit",
        json!({"script": "track add V1 video\noverwrite gen=solid at=0f dur=30f on=V1"}),
    );
    let id = staged["proposal"].as_str().unwrap();
    let pending = message(&error(&router, "render", json!({})));
    assert!(
        pending.contains(id) && pending.contains("`proposal`"),
        "{pending}"
    );
}

/// What is set and can do nothing is said in the check, as information, without failing anything.
#[test]
fn the_check_says_what_does_nothing() {
    let Some((_directory, router)) = router_with(
        "    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00
  track V2 video
    clip s1a gen=solid@1 at=00:00:00:00 dur=00:00:04:00
",
    ) else {
        return;
    };
    // Each in place of the last, since only so many proposals may wait.
    let last = std::cell::RefCell::new(None::<String>);
    let run = |script: &str| {
        let mut arguments = json!({"script": script});
        if let Some(id) = last.borrow().as_ref() {
            arguments["revise"] = json!(id);
        }
        let staged = ok(&router, "edit", arguments);
        *last.borrow_mut() = staged["proposal"].as_str().map(str::to_owned);
        staged
    };
    let says = |script: &str, wanted: &str| {
        let staged = run(script);
        let findings = staged["check"]["findings"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let found = findings.iter().find(|finding| {
            finding["code"] == "no_effect"
                && finding["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains(wanted)
        });
        assert!(
            found.is_some(),
            "`{script}` should say `{wanted}`: {findings:?}"
        );
        assert_eq!(found.unwrap()["severity"], "info", "{script}");
        assert_eq!(staged["check"]["valid"], true, "{script}");
    };
    says("set s1a z=0.5", "`three_d`");
    says("set s1a three_d=true bevel=0.02", "`depth`");
    says("set s1a three_d=true depth=0.1 bevel=0.08", "half");
    says(
        "set s1a three_d=true depth=0.1 bevel_profile=round",
        "`bevel`",
    );
    says("set s1a along={0f:0 30f:1}", "`path`");
    says("set s1a path=\"0,0 0.1,0.1\"", "`along`");
    says("set s1a orient=true", "`path`");
    says("set s1a rotation={0f:0 500f:90}", "past");
    says(
        "track add V3 video\noverwrite gen=light at=0f dur=30f on=V3 kind=ambient shadow=0.5 as=lamp",
        "`shadow`",
    );
    says(
        "track add V3 video\noverwrite gen=light at=0f dur=30f on=V3 size=3 as=lamp",
        "`size`",
    );
    says(
        "track add V3 video\noverwrite gen=light at=0f dur=30f on=V3 image=V1 as=lamp",
        "`image`",
    );
    says(
        "track add V3 video\noverwrite gen=camera at=0f dur=30f on=V3 focus=0.5 as=lens",
        "`focus`",
    );
    says(
        "track add V3 video\noverwrite gen=camera at=0f dur=30f on=V3 aperture=0.03 as=lens",
        "in depth",
    );
    says(
        "track add V3 video\noverwrite gen=camera at=0f dur=30f on=V3 as=lens\nmask lens add k1 rect=0,0,1,1",
        "draws nothing",
    );
    // A feature the title's font has not, by the font's name; one turned off asks nothing of it.
    says(
        "track add V3 video\noverwrite gen=title at=0f dur=30f on=V3 text=\"No. 10\" features=\"tnum smcp\" as=word",
        "Inter has no `smcp`",
    );
    // A scene that means what it says has none of them.
    let clean = run("set c1 three_d=true z=0.4\nset s1a three_d=true depth=0.1 bevel=0.02 bevel_profile=round\ntrack add V3 video\noverwrite gen=light at=0f dur=30f on=V3 kind=directional shadow=0.5 size=3 as=lamp\ntrack add V4 video\noverwrite gen=camera at=0f dur=30f on=V4 aperture=0.03 focus=0.1 as=lens\ntrack add V5 video\noverwrite gen=title at=0f dur=30f on=V5 text=\"No. 10\" features=\"tnum -smcp\" as=word");
    let findings = clean["check"]["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !findings
            .iter()
            .any(|finding| finding["code"] == "no_effect"),
        "{findings:?}"
    );
}

/// `look` says who draws a frame, why, and what it costs.
#[test]
fn look_says_who_draws_a_frame_and_what_it_costs() {
    let Some((_directory, router)) = router_with(
        "    clip c1 media=m1 at=00:00:00:00 dur=00:00:02:00
      fx color.basic@1 exposure=0.2
    clip c2 media=m1 src=00:00:02:00 at=00:00:02:00 dur=00:00:02:00
      fx noise.median@2
",
    ) else {
        return;
    };
    let costs = ok(&router, "look", json!({"cost": true, "at": ["10f", "70f"]}));
    assert!(
        (costs["frameMs"].as_f64().unwrap() - 1000.0 / 30.0).abs() < 0.1,
        "{costs}"
    );
    let rows = costs["costs"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{costs}");
    assert_eq!(rows[0]["at"], "10f");
    for row in rows {
        assert!(row["ms"].as_f64().unwrap() > 0.0, "{row}");
    }
    if rows[0]["draws"] == "gpu" {
        assert_eq!(rows[0]["because"], json!([]), "{row}", row = rows[0]);
        assert_eq!(rows[1]["draws"], "cpu", "{}", rows[1]);
        let because = rows[1]["because"].to_string();
        assert!(
            because.contains("c2") && because.contains("noise.median"),
            "{because}"
        );
    } else {
        // A machine with no GPU says that.
        assert!(
            rows[0]["because"].to_string().contains("no GPU"),
            "{}",
            rows[0]
        );
    }
    // Where it is not asked for sensibly, it says how.
    let alone = message(&error(&router, "look", json!({"cost": true})));
    assert!(alone.contains("`at`"), "{alone}");
    let on_media = message(&error(
        &router,
        "look",
        json!({"cost": true, "at": ["0f"], "media": "m1"}),
    ));
    assert!(on_media.contains("program"), "{on_media}");
}

/// `look` says what shows of each clip on a frame, and what hides it.
#[test]
fn look_says_what_shows_of_each_clip_and_what_hides_it() {
    // A floor higher than the foot of what stands on it, as a guide once had.
    let Some((_directory, router)) = router_with(
        "    clip sky gen=solid@1 at=0f dur=60f
      set color=#88aaff
  track V2 video
    clip floor gen=solid@1 at=0f dur=60f
      set color=#404040 three_d=true rotate_x=-80 y=0.1 z=0 scale=3
  track V3 video
    clip car gen=solid@1 at=0f dur=60f
      set color=#ff0000 three_d=true scale=0.2 y=0.2 z=0.5
  track V4 video
    clip cam gen=camera@1 at=0f dur=60f
",
    ) else {
        return;
    };
    let shows = ok(&router, "look", json!({"shows": true, "at": ["10f"]}));
    assert_eq!(shows["sequence"], "s1", "{shows}");
    let frame = &shows["frames"][0];
    assert_eq!(frame["at"], "10f");
    let clips = frame["clips"].as_array().unwrap();
    assert_eq!(
        clips.iter().map(|clip| &clip["clip"]).collect::<Vec<_>>(),
        ["sky", "floor", "car"],
        "{frame}"
    );
    let car = &clips[2];
    assert!(car["hidden"].as_f64().unwrap() > 0.5, "{car}");
    assert_eq!(car["by"][0]["clip"], "floor", "{car}");
    assert_eq!(car["box"].as_array().map(Vec::len), Some(4), "{car}");
    assert_eq!(clips[0]["edges"], json!(["left", "top", "right", "bottom"]));
    assert!(
        car.get("opacity").is_none() && car.get("edges").is_none(),
        "{car}"
    );
    // Where it is not asked for sensibly, it says how.
    let alone = message(&error(&router, "look", json!({"shows": true})));
    assert!(alone.contains("`at`"), "{alone}");
    let on_media = message(&error(
        &router,
        "look",
        json!({"shows": true, "at": ["0f"], "media": "m1"}),
    ));
    assert!(on_media.contains("program"), "{on_media}");
}

/// `look` writes a signal's keys from the onsets of a sound on the timeline, and the edit that
/// runs them has the signal follow it.
#[test]
fn look_writes_a_signals_keys_from_a_sounds_onsets() {
    // The footage's tone starts on its first frame, which the clip plays a second in.
    let Some((_directory, router)) = router_with(
        "    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00
  track V2 video
    clip cue gen=signal@1 at=0f dur=120f
  track A1 audio
    clip a1 media=m1 at=00:00:01:00 dur=00:00:02:00
",
    ) else {
        return;
    };
    let read = ok(
        &router,
        "look",
        json!({"signal": {"of": "a1", "clip": "cue"}}),
    );
    assert_eq!(
        read["script"], "set cue value={0f:0:hold 30f:1:hold 33f:0:hold}",
        "{read}"
    );
    assert_eq!((&read["onsets"], &read["outside"]), (&json!(1), &json!(0)));
    // A longer hold, for a model that makes fewer frames.
    let held = ok(
        &router,
        "look",
        json!({"signal": {"of": "a1", "clip": "cue", "hold": 6}}),
    );
    assert_eq!(
        held["script"],
        "set cue value={0f:0:hold 30f:1:hold 36f:0:hold}"
    );
    // The line is one an edit runs, and the document then holds the keys.
    ok(&router, "edit", json!({"script": read["script"]}));
    // What cannot be read is said.
    let wrong = |arguments: Value| message(&error(&router, "look", arguments));
    assert!(wrong(json!({"signal": {"of": "a1", "clip": "c1"}})).contains("is not a signal"));
    assert!(wrong(json!({"signal": {"of": "cue", "clip": "cue"}})).contains("plays no media"));
    assert!(wrong(json!({"signal": {"of": "a9", "clip": "cue"}})).contains("no clip `a9`"));
    assert!(
        wrong(json!({"signal": {"of": "a1", "clip": "cue", "hold": 0}})).contains("from 1 to 240")
    );
    assert!(
        wrong(json!({"signal": {"of": "a1", "clip": "cue"}, "at": ["0f"]})).contains("used alone")
    );
}

/// Looking something up by what it does reaches what the clip properties and generators can do.
#[test]
fn look_finds_what_is_in_properties_and_parameters() {
    let Some((_directory, router)) = router() else {
        return;
    };
    let found = |words: &str| ok(&router, "look", json!({"effects": words}));
    let ids = |value: &Value, list: &str| -> Vec<String> {
        value[list]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| row["id"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let bevel = found("bevel");
    assert!(
        ids(&bevel, "properties").contains(&"bevel".to_owned()),
        "{bevel}"
    );
    let shadow = found("shadow");
    assert!(
        ids(&shadow, "effects").contains(&"light".to_owned()),
        "{shadow}"
    );
    let focus = found("depth of field");
    assert!(
        ids(&focus, "effects").contains(&"camera".to_owned()),
        "{focus}"
    );
    // A property is looked up as an effect is.
    let property = ok(&router, "look", json!({"effect": "bevel"}));
    assert_eq!(property["property"], "bevel", "{property}");
    assert!(
        property["reference"]
            .as_str()
            .unwrap_or_default()
            .contains("`bevel=0`"),
        "{property}"
    );
    // A name near one says so.
    let near = message(&error(&router, "look", json!({"effect": "solidd"})));
    assert!(near.contains("`solid`"), "{near}");
}

/// A recipe an agent writes is part of its proposal: shown in it, kept beside the one before
/// it when it is written again, never over it, and taken back by undo.
#[test]
fn a_recipe_an_edit_writes_is_kept_beside_the_one_before_and_undo_goes_back() {
    let Some((directory, router)) = router() else {
        return;
    };
    let dir = directory.path();
    let first = "kerf-recipe 1\nrender s1 {out}\n";
    let staged = ok(
        &router,
        "edit",
        json!({
            "script": "media add m9 path=\"shots/open.mp4\" rate=30 dur=4s video=320x180 recipe=\"shots/open.kerfrecipe\"",
            "recipes": {"shots/open.kerfrecipe": first},
        }),
    );
    assert_eq!(staged["status"], "awaiting_review", "{staged}");
    assert_eq!(
        staged["recipes"][0],
        json!({"recipe": "shots/open.kerfrecipe", "keptAs": "shots/open.kerfrecipe", "written": true})
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(),
        first
    );
    // The person reads the recipe in the proposal before saying yes.
    let state = router.runtime().get_state().unwrap();
    let source = &state.proposals[0].source;
    assert!(
        source.contains(
            "# writes the recipe shots/open.kerfrecipe:\n#   kerf-recipe 1\n#   render s1 {out}\n"
        ),
        "{source}"
    );
    let accept = |proposal: &Value, key: &str| {
        let revision = router.runtime().get_state().unwrap().revision;
        router
            .runtime()
            .accept_proposal(proposal.as_str().unwrap(), "ui", key, revision)
            .unwrap()
    };
    accept(&staged["proposal"], "accept-first");

    // Written again with another text, alone: the first stays as it is, the second is kept
    // beside it, and the media moves to it in the proposal.
    let second = "kerf-recipe 1\nrender s1 {out} size=160x90\n";
    let staged = ok(
        &router,
        "edit",
        json!({"recipes": {"shots/open.kerfrecipe": second}}),
    );
    assert_eq!(
        staged["recipes"][0]["keptAs"], "shots/open@2.kerfrecipe",
        "{staged}"
    );
    assert!(staged["recipes"][0]["diff"]
        .as_str()
        .is_some_and(|diff| diff.contains("+render s1 {out} size=160x90")));
    assert!(
        staged["script"].as_str().is_some_and(|script| script
            .contains("media replace m9 path=\"shots/open.mp4\"")
            && script.contains("recipe=\"shots/open@2.kerfrecipe\"")),
        "{staged}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("shots/open.kerfrecipe")).unwrap(),
        first
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("shots/open@2.kerfrecipe")).unwrap(),
        second
    );
    let state = accept(&staged["proposal"], "accept-second");
    assert!(
        state
            .document
            .contains("recipe=\"shots/open@2.kerfrecipe\""),
        "{}",
        state.document
    );
    // Undo has the media be made from the first again.
    let state = router
        .runtime()
        .undo_request("ui", "undo-second", state.revision)
        .unwrap();
    assert!(
        state.document.contains("recipe=\"shots/open.kerfrecipe\""),
        "{}",
        state.document
    );
    // The second text again is the version that holds it, written once.
    let again = ok(
        &router,
        "edit",
        json!({"recipes": {"shots/open.kerfrecipe": second}}),
    );
    assert_eq!(
        again["recipes"][0],
        json!({"recipe": "shots/open.kerfrecipe", "keptAs": "shots/open@2.kerfrecipe", "written": false, "diff": kerf_lang::diff_text(first, second)})
    );

    // A text that is no recipe, and a file that is no recipe's, are refused with why.
    let wrong = error(
        &router,
        "edit",
        json!({"revise": again["proposal"], "recipes": {"shots/open.kerfrecipe": "kerf-recipe 1\nrender {out}\n"}}),
    );
    assert_eq!(wrong["code"], "INVALID_RECIPE", "{wrong}");
    assert!(
        wrong["message"]
            .as_str()
            .unwrap()
            .starts_with("shots/open.kerfrecipe: "),
        "{wrong}"
    );
    let outside = error(
        &router,
        "edit",
        json!({"revise": again["proposal"], "recipes": {"../open.kerfrecipe": first}}),
    );
    assert_eq!(outside["code"], "INVALID_RECIPE", "{outside}");
    let named = error(
        &router,
        "edit",
        json!({"revise": again["proposal"], "recipes": {"shots/open.json": first}}),
    );
    assert!(
        named["message"].as_str().unwrap().contains("`.kerfrecipe`"),
        "{named}"
    );
}

/// What the person lets an agent do on its own: accept its edits, and run what it asks to have
/// made, up to a limit; else it is asked for, in the review, and listed for a terminal.
#[test]
fn a_making_is_asked_for_or_runs_as_the_persons_permissions_say() {
    use kerf_mcp::permissions::{Mode, Permissions};
    let Some((directory, router)) = router() else {
        return;
    };
    let dir = directory.path();
    let given = |mode: Mode, makes: u32| {
        router.set_permissions(Some(Permissions {
            mode,
            makes,
            ..Permissions::default()
        }))
    };
    given(Mode::AcceptEdits, 0);
    let revision = router.runtime().get_state().unwrap().revision;
    let edited = ok(
        &router,
        "edit",
        json!({
            "script": "media add m9 path=\"shots/open.mp4\" rate=30 dur=4s video=320x180 recipe=\"shots/open.kerfrecipe\"",
            "recipes": {"shots/open.kerfrecipe": "kerf-recipe 1\nrender s1 {out} size=160x90\n"},
        }),
    );
    assert_eq!(edited["status"], "accepted", "{edited}");
    assert_eq!(edited["permissions"], "accept-edits");
    assert_eq!(edited["revision"], revision + 1);

    // Asked for: what it would run, and the person is asked in the review.
    let asked = ok(
        &router,
        "make",
        json!({"media": "m9", "why": "the first take"}),
    );
    assert_eq!(asked["status"], "asked", "{asked}");
    assert_eq!(asked["take"], 1);
    assert!(asked["steps"][0]
        .as_str()
        .is_some_and(|step| step.starts_with("Kerf renders sequence s1 at 160x90")));
    let requests = kerf_mcp::make_requests(router.runtime());
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "m9");
    assert_eq!(requests[0].1.id, asked["comment"].as_str().unwrap());
    assert!(
        requests[0]
            .1
            .body
            .contains("the first take\n1. Kerf renders"),
        "{}",
        requests[0].1.body
    );
    assert!(!dir.join("shots/open.takes/001.mp4").exists());

    // Let through: it runs, and the media is the take once it is made. Whoever watches the
    // session's makings, as an editor does to show them, is told from its start to its end.
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    router.watch_makings({
        let seen = seen.clone();
        std::sync::Arc::new(move |event| {
            use kerf_mcp::MakingEvent::{Began, Failed, Made, Said, Stopped};
            let said = match event {
                Began { media, take, .. } => format!("began {media} {take}"),
                Said { media, .. } => format!("said {media}"),
                Made { media, take } => format!("made {media} {take}"),
                Stopped { media } => format!("stopped {media}"),
                Failed { media, why } => format!("failed {media}: {why}"),
            };
            seen.lock().unwrap().push(said);
        })
    });
    given(Mode::Auto, 1);
    let made = ok(&router, "make", json!({"media": "m9", "wait": true}));
    assert_eq!(made["status"], "made", "{made}");
    {
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.first().map(String::as_str),
            Some("began m9 1"),
            "{seen:?}"
        );
        assert_eq!(
            seen.last().map(String::as_str),
            Some("made m9 1"),
            "{seen:?}"
        );
        assert!(seen.iter().any(|said| said == "said m9"), "{seen:?}");
    }
    assert_eq!(made["take"], 1);
    assert_eq!(made["path"], "shots/open.takes/001.mp4");
    assert!(dir.join("shots/open.takes/001.mp4").is_file());
    let document = router.runtime().get_state().unwrap().document;
    assert!(
        document.contains("path=\"shots/open.takes/001.mp4\""),
        "{document}"
    );
    // Past the limit, it is asked for again.
    let past = ok(&router, "make", json!({"media": "m9"}));
    assert_eq!(past["status"], "asked", "{past}");
    assert!(past["why"]
        .as_str()
        .is_some_and(|why| why.contains("the 1 its person lets")));

    // A making that is stopped leaves no take, and the media as it was.
    given(Mode::Auto, 5);
    ok(
        &router,
        "edit",
        json!({"recipes": {"shots/open.kerfrecipe": "kerf-recipe 1\nrun sleep 30\nrender s1 {out}\n"}}),
    );
    let making = ok(&router, "make", json!({"media": "m9"}));
    assert_eq!(making["status"], "making", "{making}");
    let job = making["job"].clone();
    let busy = error(&router, "make", json!({"media": "m9"}));
    assert_eq!(busy["code"], "MAKING", "{busy}");
    let started = Instant::now();
    let stopped = ok(&router, "make", json!({"job": job, "cancel": true}));
    assert_eq!(stopped["status"], "stopped", "{stopped}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(!dir.join("shots/open.takes/002.mp4").exists());
    assert!(!dir.join("shots/open.takes/002.work").exists());
    {
        // One asked for is not a making, and was not said to be; the one stopped was.
        let seen = seen.lock().unwrap();
        let after: Vec<&str> = seen
            .iter()
            .skip_while(|said| *said != "made m9 1")
            .skip(1)
            .map(String::as_str)
            .filter(|said| *said != "said m9")
            .collect();
        assert_eq!(after, ["began m9 2", "stopped m9"], "{seen:?}");
    }
    let document = router.runtime().get_state().unwrap().document;
    assert!(
        document.contains("path=\"shots/open.takes/001.mp4\""),
        "{document}"
    );
    assert_eq!(router.running_makings(), 0);

    // Another take, of another version of the recipe: the take before is chosen again with the
    // version it was made from, so that it is not asked for again.
    ok(
        &router,
        "edit",
        json!({"recipes": {"shots/open.kerfrecipe": "kerf-recipe 1\nrender s1 {out} size=96x54\n"}}),
    );
    let made = ok(&router, "make", json!({"media": "m9", "wait": true}));
    assert_eq!(made["path"], "shots/open.takes/002.mp4", "{made}");
    let looked = ok(&router, "look", json!({"media": "m9"}));
    let first = &looked["takes"]["made"][0];
    assert!(
        first["use"]
            .as_str()
            .is_some_and(|line| line.ends_with("recipe=\"shots/open.kerfrecipe\"")),
        "{looked}"
    );
    let revision = router.runtime().get_state().unwrap().revision;
    let state = router
        .runtime()
        .replace_media_request(
            "ui",
            "use-first",
            "m9",
            "shots/open.takes/001.mp4",
            revision,
        )
        .unwrap();
    assert!(
        state.document.contains("path=\"shots/open.takes/001.mp4\"")
            && state.document.contains("recipe=\"shots/open.kerfrecipe\""),
        "{}",
        state.document
    );
    assert!(ok(&router, "look", json!({})).get("toMake").is_none());

    // A person's making answers what was asked.
    kerf_mcp::answer_make_requests(router.runtime(), "m9");
    assert!(kerf_mcp::make_requests(router.runtime()).is_empty());
    let unknown = error(&router, "make", json!({"job": "make-none"}));
    assert_eq!(unknown["code"], "UNKNOWN_JOB");
}
