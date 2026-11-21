//! What a person does in the editor's graph of a shot, done by an agent with
//! `look` and `edit`, against a real kerfd: the graph read, a word of a step
//! set, an input wired in and let go, a step added and one taken out, and a
//! value set in the workflow its model is run with.
//!
//! In a file of its own: it names a models file of its own in `KERF_MODELS`,
//! and no ComfyUI server in `KERF_COMFY`, which the other tests' process must
//! not see. What a server says of a workflow's inputs is read in
//! `workflow_server.rs`, from one that is a thread there.

use kerf_mcp::KerfMcpRouter;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The steps of the shot, as the graph's `text` lists them.
const STEPS: &str = "render court {depth.mp4} pass=depth size=832x480 rate=16
generate model=still prompt=\"an orange basketball, studio light\" seed=3 out={ball.png}
generate model=video prompt=\"a basketball bounces on a court\" guide={depth.mp4} ref={ball.png} signal=bounce moves=ball seconds=4 seed=7 strength=0.8
";

const PROJECT: &str = "kerf 1
project name=\"Court\"
media shot path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480
  render court {depth.mp4} pass=depth size=832x480 rate=16
  generate model=still prompt=\"an orange basketball, studio light\" seed=3 out={ball.png}
  generate model=video prompt=\"a basketball bounces on a court\" guide={depth.mp4} ref={ball.png} signal=bounce moves=ball seconds=4 seed=7 strength=0.8
media card path=\"shots/card.mp4\" rate=24 dur=4f video=96x54
  render tiny {out} size=96x54
sequence tiny rate=24 size=96x54
  track V1 video
    clip slate gen=solid@1 at=0f dur=4f
sequence main rate=24 size=1920x1080 audio=48000
  track V1 video
    clip play media=shot src=0f at=0f dur=96f
sequence court name=\"The court\" rate=24 size=1920x1080 audio=48000
  track V1 video
    clip floor gen=solid@1 at=0f dur=96f
      set color=#8a5a2b three_d=true rotate_x=70 y=0.35 scale=1.6
  track V3 video
    clip ball gen=solid@1 at=0f dur=96f
      set color=#e07a20 three_d=true scale=0.12 path=\"-0.42,-0.3 -0.28,0.3 -0.12,-0.12 0.04,0.3 0.2,0.02 0.34,0.3 0.44,0.2\" along={0f:0 95f:1}
  track V4 video
    clip bounce gen=signal@1 at=0f dur=96f
      set value={0f:0 11f:0 12f:1 15f:1 16f:0}
  track V5 video
    clip hit gen=signal@1 at=0f dur=96f
      set value={0f:0 43f:0 44f:1}
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

fn read(file: &Path) -> String {
    std::fs::read_to_string(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()))
}

/// The node of `graph` that `finds`.
fn node(graph: &Value, finds: impl Fn(&Value) -> bool) -> &Value {
    graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| finds(node))
        .unwrap_or_else(|| panic!("no such node in {graph}"))
}

/// Whether `graph` has `from` read by `to` under `port`.
fn wired(graph: &Value, from: &str, to: &str, port: &str) -> bool {
    graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|edge| edge["from"] == from && edge["to"] == to && edge["port"] == port)
}

/// The steps under media `id` in the document `of` gives, a line each.
fn steps_in(document: &str, id: &str) -> Vec<String> {
    document
        .lines()
        .skip_while(|line| !line.starts_with(&format!("media {id} ")))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.trim().to_owned())
        .collect()
}

/// The document as proposal `proposal` would have it.
fn proposed(router: &KerfMcpRouter, proposal: &Value) -> String {
    ok(router, "look", json!({"proposal": proposal}))["document"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn an_agent_reads_a_shots_graph_and_sets_what_a_person_sets_there() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    std::fs::create_dir(dir.join("shots")).unwrap();
    std::fs::write(dir.join("project.kerf"), PROJECT).unwrap();
    // The person's own models: one run with kerf-comfy, which has a workflow to read, and one not.
    let models = dir.join("models.json");
    std::fs::write(
        &models,
        r#"{"still": {"run": ["a-still-program", "{request}"]},
            "video": {"run": ["kerf-comfy", "--workflow", "shot", "{request}"]}}"#,
    )
    .unwrap();
    std::env::set_var("KERF_MODELS", &models);
    // No server a person has named is asked what the workflow's nodes take: the file named
    // here is not there, and nothing else names one.
    std::env::set_var("KERF_COMFY", dir.join("no-such-comfy.json"));
    for names_one in ["COMFY_URL", "COMFY_API_KEY", "COMFY_API_HEADER"] {
        std::env::remove_var(names_one);
    }
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(dir, Some(&kerfd)).unwrap();
    router.set_permissions(Some(kerf_mcp::permissions::Permissions::default()));

    // The graph, as the editor shows it: the steps, what they read, and what runs the model,
    // each node by its step's number.
    let graph = ok(&router, "look", json!({"graph": "shot"}));
    assert_eq!(graph["text"], STEPS);
    assert!(
        graph.get("recipe").is_none() && graph.get("document").is_none(),
        "{graph}"
    );
    assert_eq!(
        (graph["media"].as_str(), graph["name"].as_str()),
        (Some("shot"), Some("shot"))
    );
    let video = node(&graph, |node| node["model"] == "video");
    let (line, made) = (
        video["line"].as_u64().unwrap(),
        video["id"].as_str().unwrap().to_owned(),
    );
    assert_eq!(line, 3);
    assert_eq!(
        node(&graph, |node| node["id"] == "made:depth.mp4")["line"],
        1
    );
    assert_eq!(
        video["runner"],
        json!({"kind": "comfy", "workflow": "shot"})
    );
    assert!(node(&graph, |node| node["model"] == "still")
        .get("runner")
        .is_none());
    assert!(wired(&graph, "signal:bounce", &made, "signal"), "{graph}");
    assert!(wired(&graph, "moves:ball", &made, "moves"), "{graph}");
    assert!(
        wired(&graph, "sequence:court", "made:depth.mp4", "sequence"),
        "{graph}"
    );
    assert!(
        graph["change"]
            .as_str()
            .unwrap()
            .contains("step shot set <n>"),
        "{graph}"
    );
    assert!(
        graph["takes"]["made"].as_array().is_some_and(Vec::is_empty),
        "{graph}"
    );

    // Its takes held to its guide, as the graph's takes card holds them: the clip the step
    // says goes somewhere, in the sequence that has it. None is made yet.
    let held = ok(&router, "look", json!({"takes": "shot"}));
    assert_eq!(
        (held["sequence"].as_str(), held["clips"][0].as_str()),
        (Some("court"), Some("ball"))
    );
    assert_eq!(held["takes"], json!([]));
    assert_eq!(held["best"], Value::Null);

    // One word of one step, by a line of the script: every other word and step as it was,
    // and no file written.
    let files = || std::fs::read_dir(dir.join("shots")).unwrap().count();
    let before = files();
    let seeded = ok(
        &router,
        "edit",
        json!({"script": format!("step shot set {line} seed=8")}),
    );
    assert_eq!(seeded["status"], "awaiting_review", "{seeded}");
    assert!(seeded.get("recipes").is_none(), "{seeded}");
    assert_eq!(
        steps_in(&proposed(&router, &seeded["proposal"]), "shot")[2],
        STEPS.lines().nth(2).unwrap().replace("seed=7", "seed=8")
    );
    assert_eq!(files(), before);

    // On top of it: one input let go, another wired in, and a value of the model's workflow.
    let workflow = ok(
        &router,
        "look",
        json!({"workflow": {"media": "shot", "line": line}}),
    );
    assert_eq!(
        (workflow["name"].as_str(), workflow["origin"].as_str()),
        (Some("shot"), Some("bundled"))
    );
    assert!(workflow.get("text").is_none(), "the nodes say it");
    // With no server named, none was asked: the workflow is read without, and says why.
    assert!(
        workflow.get("taken").is_none() && workflow["takenError"].is_string(),
        "{workflow}"
    );
    assert_eq!(workflow["blanks"]["seed"]["given"], "7");
    let loader = node(&workflow, |node| {
        node["inputs"]
            .as_array()
            .is_some_and(|inputs| inputs.iter().any(|input| input["name"] == "weight_dtype"))
    })["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let wired_again = ok(
        &router,
        "edit",
        json!({
            "extend": seeded["proposal"],
            "script": format!("step shot set {line} signal=\"\"\nstep shot set {line} signal=hit"),
            "workflowSet": [{"media": "shot", "line": line, "node": loader, "input": "weight_dtype", "value": "fp8_e4m3fn"}],
        }),
    );
    assert_eq!(
        wired_again["workflows"][0],
        json!({"media": "shot", "line": line, "of": "shot", "keptAs": "shots/shot.video.comfy.json",
               "set": [{"node": loader, "input": "weight_dtype", "value": "fp8_e4m3fn"}]})
    );
    assert_eq!(
        steps_in(&proposed(&router, &wired_again["proposal"]), "shot")[2],
        STEPS.lines().nth(2).unwrap().replace(
            "signal=bounce moves=ball seconds=4 seed=7 strength=0.8",
            "signal=hit moves=ball seconds=4 seed=8 workflow=shots/shot.video.comfy.json strength=0.8"
        )
    );
    assert!(
        read(&dir.join("shots/shot.video.comfy.json")).contains("\"weight_dtype\": \"fp8_e4m3fn\"")
    );
    // The person reviewing it reads what was set, in the proposal's script.
    let state = router.runtime().get_state().unwrap();
    let script = &state.proposals[0].source;
    assert!(
        script.contains(&format!(
            "# writes the workflow shots/shot.video.comfy.json, a copy of `shot` for step {line} of shot, with:\n#   node {loader} weight_dtype = \"fp8_e4m3fn\"\n"
        )),
        "{script}"
    );

    // What the proposal would make of the graph, and of the workflow.
    let shown = ok(
        &router,
        "look",
        json!({"graph": "shot", "proposal": wired_again["proposal"]}),
    );
    assert!(!wired(&shown, "signal:bounce", &made, "signal"), "{shown}");
    assert!(wired(&shown, "signal:hit", &made, "signal"), "{shown}");
    assert_eq!(
        node(&shown, |node| node["model"] == "video")["workflow"],
        "shots/shot.video.comfy.json"
    );
    let own = ok(
        &router,
        "look",
        json!({"workflow": {"media": "shot", "line": line}, "proposal": wired_again["proposal"]}),
    );
    assert_eq!(
        (own["origin"].as_str(), own["file"].as_str()),
        (Some("own"), Some("shots/shot.video.comfy.json"))
    );
    assert_eq!(own["blanks"]["seed"]["given"], "8");
    // The project itself is as it was until the person says yes.
    let project = ok(&router, "look", json!({"graph": "shot"}));
    assert!(wired(&project, "signal:bounce", &made, "signal"));

    let revision = router.runtime().get_state().unwrap().revision;
    router
        .runtime()
        .accept_proposal(
            wired_again["proposal"].as_str().unwrap(),
            "ui",
            "accept-wired",
            revision,
        )
        .unwrap();

    // A value set again is another copy: the one the steps before name is not written over.
    let first_copy = read(&dir.join("shots/shot.video.comfy.json"));
    let again = ok(
        &router,
        "edit",
        json!({"workflowSet": [{"media": "shot", "line": line, "node": loader, "input": "weight_dtype", "value": "default"}]}),
    );
    assert_eq!(
        again["workflows"][0]["keptAs"], "shots/shot.video-2.comfy.json",
        "{again}"
    );
    assert!(
        again["script"]
            .as_str()
            .unwrap()
            .contains("workflow=shots/shot.video-2.comfy.json"),
        "{again}"
    );
    assert_eq!(read(&dir.join("shots/shot.video.comfy.json")), first_copy);
    // Back to the workflow as it comes: the word taken off the step.
    let back = ok(
        &router,
        "edit",
        json!({"revise": again["proposal"], "script": format!("step shot set {line} workflow=\"\"")}),
    );
    assert!(
        !steps_in(&proposed(&router, &back["proposal"]), "shot")[2].contains("workflow="),
        "{back}"
    );

    // A step added and one taken out, as a person does in the graph. Later lines count steps
    // as the earlier ones left them.
    let stepped = ok(
        &router,
        "edit",
        json!({"revise": back["proposal"], "script": format!(
            "step shot add 2 render court {{ids.mp4}} pass=ids size=832x480 rate=16\nstep shot set {} guide_ids={{ids.mp4}}",
            line + 1
        )}),
    );
    let now = steps_in(&proposed(&router, &stepped["proposal"]), "shot");
    assert_eq!(
        now[1], "render court {ids.mp4} pass=ids size=832x480 rate=16",
        "{now:?}"
    );
    assert!(now[3].contains("guide_ids={ids.mp4}"), "{now:?}");
    let shown = ok(
        &router,
        "look",
        json!({"graph": "shot", "proposal": stepped["proposal"]}),
    );
    assert!(wired(&shown, "made:ids.mp4", &made, "guide_ids"), "{shown}");
    assert_eq!(
        node(&shown, |node| node["model"] == "video")["line"],
        line + 1
    );
    let said = refused(
        &router,
        "edit",
        json!({"extend": stepped["proposal"], "script": "step shot remove 1"}),
    );
    assert!(said.contains("{depth.mp4}"), "{said}");
    let back = ok(
        &router,
        "edit",
        json!({"revise": stepped["proposal"], "script": format!("step shot set {line} workflow=\"\"")}),
    );

    // What a step or a workflow cannot take is refused, with why, and nothing is kept of it.
    let before = files();
    let said = refused(
        &router,
        "edit",
        json!({"revise": back["proposal"], "script": format!("step shot set {line} seed=soon")}),
    );
    assert!(said.contains("`seed=soon` is not a whole number"), "{said}");
    let said = refused(
        &router,
        "edit",
        json!({"revise": back["proposal"], "workflowSet": [{"media": "shot", "line": line, "node": loader, "input": "unet_name", "value": "other"}]}),
    );
    assert!(said.contains("which the step fills"), "{said}");
    let said = refused(
        &router,
        "look",
        json!({"workflow": {"media": "shot", "line": line - 1}}),
    );
    assert!(
        said.contains("model `still` is not run by kerf-comfy"),
        "{said}"
    );
    // A shot that says where nothing goes has nothing to hold its takes to.
    let alone = ok(
        &router,
        "edit",
        json!({"revise": back["proposal"], "script": "media add m7 path=\"shots/still.png\" rate=24 dur=1f video=512x512\nstep m7 add generate model=still prompt=\"a ball\""}),
    );
    let said = refused(
        &router,
        "look",
        json!({"takes": "m7", "proposal": alone["proposal"]}),
    );
    assert!(said.contains("moves=<clip>"), "{said}");
    let said = refused(&router, "look", json!({"graph": "nothing"}));
    assert!(said.contains("no media `nothing`"), "{said}");
    assert_eq!(files(), before);

    // A take keeps the steps it was made from, and an agent goes back to them: lines of the
    // edit that write them under the media again.
    let (_, document) = router.runtime().get_document(false).unwrap();
    let project = kerf_runtime::parse_document(&document).unwrap();
    let none = kerf_engine::recipe::model::Models::default();
    let planned = kerf_engine::recipe::plan(&project, dir, "card", &none).unwrap();
    kerf_engine::recipe::make(
        &project,
        dir,
        &planned,
        &none,
        &kerf_engine::CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    let larger = ok(
        &router,
        "edit",
        json!({"revise": alone["proposal"], "script": "step card set 1 size=160x90"}),
    );
    let revision = router.runtime().get_state().unwrap().revision;
    router
        .runtime()
        .accept_proposal(
            larger["proposal"].as_str().unwrap(),
            "ui",
            "accept-larger",
            revision,
        )
        .unwrap();
    let listed = ok(&router, "look", json!({"graph": "card"}));
    assert!(listed.get("recipe").is_none(), "{listed}");
    assert_eq!(listed["text"], "render tiny {out} size=160x90\n");
    assert_eq!(listed["takes"]["made"][0]["kept"], true, "{listed}");
    let back = ok(
        &router,
        "edit",
        json!({"restoreTake": {"media": "card", "take": 1}}),
    );
    assert!(back.get("recipes").is_none(), "{back}");
    assert!(
        back["script"]
            .as_str()
            .unwrap()
            .contains("step card set 1 render tiny {out} size=96x54"),
        "{back}"
    );
    assert_eq!(
        steps_in(&proposed(&router, &back["proposal"]), "card"),
        ["render tiny {out} size=96x54"]
    );
    let said = refused(
        &router,
        "edit",
        json!({"revise": back["proposal"], "restoreTake": {"media": "card", "take": 5}}),
    );
    assert!(said.contains("take 5 keeps nothing of"), "{said}");
}
