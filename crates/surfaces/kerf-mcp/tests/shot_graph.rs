//! What a person does in the editor's graph of a shot, done by an agent with
//! `look` and `edit`, against a real kerfd: the graph read, a word of the
//! recipe set, an input wired in and let go, and a value set in the workflow
//! its model is run with.
//!
//! In a file of its own: it names a models file of its own in `KERF_MODELS`,
//! which the other tests' process must not see.

use kerf_mcp::KerfMcpRouter;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const RECIPE: &str = "kerf-recipe 1
# A basketball bouncing on a court.
# The court is blocked in Kerf and handed over as depth.
render court {depth.mp4} pass=depth size=832x480 rate=16

# The ball, made once and handed on as a reference.
generate model=still prompt=\"an orange basketball, studio light\" seed=3 out={ball.png}
generate model=video prompt=\"a basketball bounces on a court\" guide={depth.mp4} ref={ball.png} moves=ball signal=bounce seconds=4 seed=7 strength=0.8
";

const PROJECT: &str = "kerf 1
project name=\"Court\"
media shot path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480 recipe=\"shots/bounce.kerfrecipe\"
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
    response["error"]["message"].as_str().unwrap_or_default().to_owned()
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

#[test]
fn an_agent_reads_a_shots_graph_and_sets_what_a_person_sets_there() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    std::fs::create_dir(dir.join("shots")).unwrap();
    std::fs::write(dir.join("project.kerf"), PROJECT).unwrap();
    std::fs::write(dir.join("shots/bounce.kerfrecipe"), RECIPE).unwrap();
    // The person's own models: one run with kerf-comfy, which has a workflow to read, and one not.
    let models = dir.join("models.json");
    std::fs::write(
        &models,
        r#"{"still": {"run": ["a-still-program", "{request}"]},
            "video": {"run": ["kerf-comfy", "--workflow", "shot", "{request}"]}}"#,
    )
    .unwrap();
    std::env::set_var("KERF_MODELS", &models);
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(dir, Some(&kerfd)).unwrap();
    router.set_permissions(Some(kerf_mcp::permissions::Permissions::default()));

    // The graph, as the editor shows it: the steps, what they read, and what runs the model.
    let graph = ok(&router, "look", json!({"graph": "shot"}));
    assert_eq!(graph["text"], RECIPE);
    assert_eq!((graph["media"].as_str(), graph["name"].as_str()), (Some("shot"), Some("bounce")));
    let video = node(&graph, |node| node["model"] == "video");
    let (line, made) = (video["line"].as_u64().unwrap(), video["id"].as_str().unwrap().to_owned());
    assert_eq!(line, 8);
    assert_eq!(video["runner"], json!({"kind": "comfy", "workflow": "shot"}));
    assert!(node(&graph, |node| node["model"] == "still").get("runner").is_none());
    assert!(wired(&graph, "signal:bounce", &made, "signal"), "{graph}");
    assert!(wired(&graph, "moves:ball", &made, "moves"), "{graph}");
    assert!(wired(&graph, "sequence:court", "made:depth.mp4", "sequence"), "{graph}");
    assert!(graph["change"].as_str().unwrap().contains("recipeSet"));
    assert!(graph["takes"]["made"].as_array().is_some_and(Vec::is_empty), "{graph}");

    // Its takes held to its guide, as the graph's takes card holds them: the clip the line
    // says goes somewhere, in the sequence that has it. None is made yet.
    let held = ok(&router, "look", json!({"takes": "shot"}));
    assert_eq!((held["sequence"].as_str(), held["clips"][0].as_str()), (Some("court"), Some("ball")));
    assert_eq!(held["takes"], json!([]));
    assert_eq!(held["best"], Value::Null);

    // One word of one line: kept as the next version, comments and every other line as they
    // were, and the recipe before left as it is.
    let seeded = ok(
        &router,
        "edit",
        json!({"recipeSet": [{"media": "shot", "line": line, "key": "seed", "value": "8"}]}),
    );
    assert_eq!(seeded["status"], "awaiting_review", "{seeded}");
    assert_eq!(seeded["recipes"][0]["keptAs"], "shots/bounce@2.kerfrecipe", "{seeded}");
    assert_eq!(read(&dir.join("shots/bounce@2.kerfrecipe")), RECIPE.replace("seed=7", "seed=8"));
    assert_eq!(read(&dir.join("shots/bounce.kerfrecipe")), RECIPE);
    assert!(seeded["recipes"][0]["diff"].as_str().unwrap().contains("seed=8"), "{seeded}");

    // On top of it: one input let go, another wired in, and a value of the model's workflow.
    let workflow = ok(&router, "look", json!({"workflow": {"media": "shot", "line": line}}));
    assert_eq!((workflow["name"].as_str(), workflow["origin"].as_str()), (Some("shot"), Some("bundled")));
    assert!(workflow.get("text").is_none(), "the nodes say it");
    assert_eq!(workflow["blanks"]["seed"]["given"], "7");
    let loader = node(&workflow, |node| {
        node["inputs"].as_array().is_some_and(|inputs| inputs.iter().any(|input| input["name"] == "weight_dtype"))
    })["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let wired_again = ok(
        &router,
        "edit",
        json!({
            "extend": seeded["proposal"],
            "recipeSet": [
                {"media": "shot", "line": line, "key": "signal"},
                {"media": "shot", "line": line, "key": "signal", "value": "hit"},
            ],
            "workflowSet": [{"media": "shot", "line": line, "node": loader, "input": "weight_dtype", "value": "fp8_e4m3fn"}],
        }),
    );
    assert_eq!(wired_again["recipes"][0]["keptAs"], "shots/bounce@3.kerfrecipe", "{wired_again}");
    assert_eq!(
        wired_again["workflows"][0],
        json!({"media": "shot", "line": line, "of": "shot", "keptAs": "shots/bounce.video.comfy.json",
               "set": [{"node": loader, "input": "weight_dtype", "value": "fp8_e4m3fn"}]})
    );
    let third = read(&dir.join("shots/bounce@3.kerfrecipe"));
    assert_eq!(
        third,
        RECIPE.replace(
            "moves=ball signal=bounce seconds=4 seed=7 strength=0.8",
            "moves=ball seconds=4 seed=8 strength=0.8 signal=hit workflow=bounce.video.comfy.json"
        )
    );
    assert!(read(&dir.join("shots/bounce.video.comfy.json")).contains("\"weight_dtype\": \"fp8_e4m3fn\""));
    // The person reviewing it reads what was set, in the proposal's script.
    let state = router.runtime().get_state().unwrap();
    let script = &state.proposals[0].source;
    assert!(
        script.contains(&format!(
            "# writes the workflow shots/bounce.video.comfy.json, a copy of `shot` for line {line} of shot's recipe, with:\n#   node {loader} weight_dtype = \"fp8_e4m3fn\"\n"
        )),
        "{script}"
    );

    // What the proposal would make of the graph, and of the workflow.
    let proposed = ok(&router, "look", json!({"graph": "shot", "proposal": wired_again["proposal"]}));
    assert!(!wired(&proposed, "signal:bounce", &made, "signal"), "{proposed}");
    assert!(wired(&proposed, "signal:hit", &made, "signal"), "{proposed}");
    assert_eq!(node(&proposed, |node| node["model"] == "video")["workflow"], "bounce.video.comfy.json");
    let own = ok(
        &router,
        "look",
        json!({"workflow": {"media": "shot", "line": line}, "proposal": wired_again["proposal"]}),
    );
    assert_eq!((own["origin"].as_str(), own["file"].as_str()), (Some("own"), Some("shots/bounce.video.comfy.json")));
    assert_eq!(own["blanks"]["seed"]["given"], "8");
    // The project itself is as it was until the person says yes.
    let project = ok(&router, "look", json!({"graph": "shot"}));
    assert!(wired(&project, "signal:bounce", &made, "signal"));

    let revision = router.runtime().get_state().unwrap().revision;
    router
        .runtime()
        .accept_proposal(wired_again["proposal"].as_str().unwrap(), "ui", "accept-wired", revision)
        .unwrap();

    // A value set again is another copy: the one a recipe before names is not written over.
    let first_copy = read(&dir.join("shots/bounce.video.comfy.json"));
    let again = ok(
        &router,
        "edit",
        json!({"workflowSet": [{"media": "shot", "line": line, "node": loader, "input": "weight_dtype", "value": "default"}]}),
    );
    assert_eq!(again["workflows"][0]["keptAs"], "shots/bounce.video-2.comfy.json", "{again}");
    assert_eq!(again["recipes"][0]["keptAs"], "shots/bounce@4.kerfrecipe");
    assert_eq!(read(&dir.join("shots/bounce.video.comfy.json")), first_copy);
    // Back to the workflow as it comes: the word taken off the line.
    let back = ok(
        &router,
        "edit",
        json!({"revise": again["proposal"], "recipeSet": [{"media": "shot", "line": line, "key": "workflow"}]}),
    );
    let kept = back["recipes"][0]["keptAs"].as_str().unwrap();
    assert!(!read(&dir.join(kept)).contains("workflow="), "{back}");

    // What a line or a workflow cannot take is refused, with why, and nothing is kept of it.
    let files = || std::fs::read_dir(dir.join("shots")).unwrap().count();
    let before = files();
    let said = refused(
        &router,
        "edit",
        json!({"revise": back["proposal"], "recipeSet": [{"media": "shot", "line": line, "key": "seed", "value": "soon"}]}),
    );
    assert!(said.contains("`seed=soon` is not a whole number"), "{said}");
    let said = refused(
        &router,
        "edit",
        json!({"revise": back["proposal"], "workflowSet": [{"media": "shot", "line": line, "node": loader, "input": "unet_name", "value": "other"}]}),
    );
    assert!(said.contains("which the recipe fills"), "{said}");
    let said = refused(&router, "look", json!({"workflow": {"media": "shot", "line": line - 1}}));
    assert!(said.contains("model `still` is not run by kerf-comfy"), "{said}");
    // A shot that says where nothing goes has nothing to hold its takes to.
    std::fs::write(dir.join("shots/still.kerfrecipe"), "kerf-recipe 1\ngenerate model=still prompt=\"a ball\"\n").unwrap();
    let alone = ok(
        &router,
        "edit",
        json!({"revise": back["proposal"], "script": "media add m7 path=\"shots/still.png\" rate=24 dur=1f video=512x512 recipe=\"shots/still.kerfrecipe\""}),
    );
    let said = refused(&router, "look", json!({"takes": "m7", "proposal": alone["proposal"]}));
    assert!(said.contains("moves=<clip>"), "{said}");
    let said = refused(&router, "look", json!({"graph": "nothing"}));
    assert!(said.contains("no media `nothing`"), "{said}");
    // Only the recipe written here for that last shot is new.
    assert_eq!(files(), before + 1);
}
