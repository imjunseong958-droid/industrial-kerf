//! What Kerf or a model makes, written by an agent as steps under the media
//! in the document and made with `make`, against a real kerfd and with
//! programs that stand in for the models: no recipe file is written, by the
//! agent or by anything else.
//!
//! Both kinds a making has are made end to end here: a part of footage, where
//! one model's matte is read by the next, and a shot from a guide, which Kerf
//! draws from a sequence and hands to a model.
//!
//! In a file of its own: it reads a generation file of its own for the whole
//! process ([`Generator::use_file`]), which the other tests' process must not see.
#![cfg(unix)]

use kerf_generate::Generator;
use kerf_mcp::permissions::{Mode, Permissions};
use kerf_mcp::KerfMcpRouter;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const PROJECT: &str = "kerf 1
project name=\"A walk\"
media walk path=\"footage/walk.mp4\" rate=16 dur=16f video=160x90
sequence s1 rate=16 size=160x90
  track V1 video
    clip back media=walk src=0f at=0f dur=16f
sequence s2 rate=24 size=320x180
  track V1 video
    clip road gen=solid@1 at=0f dur=12f
      set color=#404448 three_d=true rotate_x=-84 y=0.75 z=4 scale=16
";

/// A model's program that answers with its guide: the file the request names as `guide`
/// is copied to where the request says the take goes.
const ANSWERS_ITS_GUIDE: &str = r#"["sh", "-c", "cp \"$(sed -n 's/.*\"guide\": \"\\(.*\\)\".*/\\1/p' \"$1\")\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "model", "{request}"]"#;

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

/// The entry of `toMake` for media `id`, in what `look` or an edit answered.
fn to_make<'a>(answer: &'a Value, id: &str) -> Option<&'a Value> {
    answer["toMake"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["media"] == id))
}

/// The lines of the document under media `id`: its steps, as they are written.
fn steps(router: &KerfMcpRouter, id: &str) -> Vec<String> {
    let document = router.runtime().get_state().unwrap().document;
    document
        .lines()
        .skip_while(|line| !line.starts_with(&format!("media {id} ")))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.trim().to_owned())
        .collect()
}

fn media_line(router: &KerfMcpRouter, id: &str) -> String {
    let document = router.runtime().get_state().unwrap().document;
    document
        .lines()
        .find(|line| line.starts_with(&format!("media {id} ")))
        .unwrap_or_else(|| panic!("no media {id} in {document}"))
        .to_owned()
}

/// Every file under `folder` whose name ends with `ending`.
fn files_ending(folder: &Path, ending: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_ending(&path, ending));
        } else if path.to_string_lossy().ends_with(ending) {
            found.push(path);
        }
    }
    found
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn an_agent_writes_a_makings_steps_in_the_document_and_makes_it() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    std::fs::create_dir(dir.join("footage")).unwrap();
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc=size=160x90:rate=16:duration=1"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(dir.join("footage/walk.mp4"))
        .status()
        .expect("ffmpeg");
    assert!(made.success());
    std::fs::write(dir.join("project.kerf"), PROJECT).unwrap();
    // The person's own models: three that are run here, each standing in for the model its
    // name says.
    let models = dir.join("generation.json");
    std::fs::write(
        &models,
        format!(
            r#"{{"providers": {{"matte": {{"run": {ANSWERS_ITS_GUIDE}, "takes": {{"about": "a matte of the thing the prompt names", "reads": ["guide"]}}}},
                "plate": {{"run": {ANSWERS_ITS_GUIDE}, "takes": {{"about": "the guide with what guide_mask is white on taken out", "reads": ["guide", "guide_mask"]}}}},
                "shot": {{"run": {ANSWERS_ITS_GUIDE}, "takes": {{"about": "a shot that follows a depth guide", "size": [160, 90], "rate": 12, "seconds": 5, "reads": ["guide"]}}}}}}}}"#
        ),
    )
    .unwrap();
    Generator::use_file(&models);
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(dir, Some(&kerfd)).unwrap();
    // The person lets an agent's edits through and its makings run, up to a limit of theirs.
    let given = |mode: Mode, makes: u32| {
        router.set_permissions(Some(Permissions {
            mode,
            makes,
            ..Permissions::default()
        }))
    };
    given(Mode::Auto, 8);

    // The models set up, each by what it says it makes, and for one that follows a guide
    // the step that draws a guide at its size and rate.
    let looked = ok(&router, "look", json!({}));
    let listed = looked["models"].as_array().unwrap();
    let shot = listed.iter().find(|model| model["name"] == "shot").unwrap();
    assert_eq!(
        shot["guide"],
        "render <sequence> {guide.mp4} size=160x90 rate=12"
    );

    // ---- A part of footage: the matte of a thing, and the plate a second model makes
    // with it. One edit writes the media and its steps; nothing else is written.
    let edited = ok(
        &router,
        "edit",
        json!({"script": "media add gone path=\"shots/gone.mp4\" rate=16 dur=16f video=160x90\n\
            step gone add generate model=matte guide=footage/walk.mp4 prompt=\"man\" out={man.mp4}\n\
            step gone add generate model=plate guide=footage/walk.mp4 guide_mask={man.mp4} prompt=\"an empty dirt path\" reach=72 seed=7"}),
    );
    assert_eq!(edited["status"], "accepted", "{edited}");
    assert!(edited.get("recipes").is_none(), "{edited}");
    let wants = to_make(&edited, "gone").unwrap_or_else(|| panic!("{edited}"));
    assert_eq!(wants["why"], "its file is not there yet");
    assert!(
        wants.get("cannot").is_none() && wants.get("recipe").is_none(),
        "{wants}"
    );
    assert!(
        wants["make"]
            .as_str()
            .unwrap()
            .starts_with("make it with `make`"),
        "{wants}"
    );
    assert_eq!(
        steps(&router, "gone"),
        [
            "generate model=matte prompt=\"man\" guide=footage/walk.mp4 out={man.mp4}",
            "generate model=plate prompt=\"an empty dirt path\" guide=footage/walk.mp4 guide_mask={man.mp4} seed=7 reach=72",
        ]
    );

    // Made right after the edit that wrote them: the take is the plate, whose model was
    // handed the footage and the matte the step before it made.
    let take = ok(
        &router,
        "make",
        json!({"media": "gone", "wait": true, "why": "the plate behind him"}),
    );
    assert_eq!(
        (take["status"].as_str(), take["take"].as_u64()),
        (Some("made"), Some(1)),
        "{take}"
    );
    assert_eq!(take["path"], "shots/gone.takes/001.mp4");
    assert!(dir.join("shots/gone.takes/001.work/man.mp4").is_file());
    // Two models made it, and what each was asked is kept, by its step's number: the
    // plate's request did not replace the matte's.
    let asked = |step: u32| -> Value {
        let file = dir.join(format!("shots/gone.takes/001.work/step{step}/request.json"));
        serde_json::from_slice(
            &std::fs::read(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display())),
        )
        .unwrap()
    };
    assert_eq!(
        (&asked(1)["model"], &asked(1)["prompt"]),
        (&json!("matte"), &json!("man"))
    );
    assert!(!dir.join("shots/gone.takes/001.work/request.json").exists());
    let request = asked(2);
    assert_eq!(request["model"], "plate");
    let handed = |name: &Value| name.as_str().unwrap_or_default().to_owned();
    assert!(
        handed(&request["guide"]).ends_with("/footage/walk.mp4"),
        "{request}"
    );
    assert!(
        handed(&request["guides"]["mask"]).ends_with("/shots/gone.takes/001.work/man.mp4"),
        "{request}"
    );
    // The media is the take now, and how it is made is still under it.
    assert!(media_line(&router, "gone").contains("path=\"shots/gone.takes/001.mp4\""));
    assert_eq!(steps(&router, "gone").len(), 2);
    assert!(to_make(&ok(&router, "look", json!({})), "gone").is_none());
    assert!(files_ending(dir, ".kerfrecipe")
        .iter()
        .all(|file| file.ends_with("made-from.kerfrecipe")));

    // One name changed for the next take: the edit says the media wants making, and why.
    let seeded = ok(&router, "edit", json!({"script": "step gone set 2 seed=8"}));
    assert_eq!(seeded["status"], "accepted", "{seeded}");
    assert_eq!(
        to_make(&seeded, "gone").unwrap()["why"],
        "its steps changed after its file was made"
    );
    let second = ok(&router, "make", json!({"media": "gone", "wait": true}));
    assert_eq!(
        (second["status"].as_str(), second["take"].as_u64()),
        (Some("made"), Some(2)),
        "{second}"
    );
    // A making never writes over the one before: both takes are there, and the first is
    // chosen again by a line that leaves the steps alone.
    let listed = ok(&router, "look", json!({"media": "gone"}));
    let takes = listed["takes"]["made"].as_array().unwrap();
    assert_eq!(takes.len(), 2, "{listed}");
    assert!(listed.get("recipe").is_none(), "{listed}");
    let first = &takes[0];
    assert_eq!(
        (first["chosen"].as_bool(), takes[1]["chosen"].as_bool()),
        (Some(false), Some(true))
    );
    let chooses = first["use"].as_str().unwrap();
    assert!(
        chooses.starts_with("media replace gone path=\"shots/gone.takes/001.mp4\""),
        "{chooses}"
    );
    assert!(!chooses.contains("recipe="), "{chooses}");
    // What the first take was made from is kept with it, seed and all.
    assert_eq!(first["changedSince"], json!(["its steps"]));
    assert!(
        first["steps"][1]
            .as_str()
            .unwrap()
            .ends_with("seed=7 reach=72"),
        "{first}"
    );
    assert!(takes[1].get("steps").is_none(), "{listed}");
    let chosen = ok(&router, "edit", json!({"script": chooses}));
    assert!(media_line(&router, "gone").contains("path=\"shots/gone.takes/001.mp4\""));
    assert!(steps(&router, "gone")[1].ends_with("seed=8 reach=72"));
    // The media is take 1, made with seed 7, and its steps say seed 8.
    assert_eq!(
        to_make(&chosen, "gone").unwrap()["why"],
        "its steps changed after its file was made"
    );
    // Going back to the take's own steps is an edit that sets them, which undo takes back.
    let back = ok(
        &router,
        "edit",
        json!({"restoreTake": {"media": "gone", "take": 1}}),
    );
    assert_eq!(back["status"], "accepted", "{back}");
    assert!(
        back["script"]
            .as_str()
            .unwrap()
            .contains("step gone set 2 generate model=plate"),
        "{back}"
    );
    assert!(steps(&router, "gone")[1].ends_with("seed=7 reach=72"));
    assert!(to_make(&back, "gone").is_none(), "{back}");

    // ---- A shot from a guide: the scene is sequence `s2`, and the steps under the media
    // say which pass of it is drawn, at what size and rate, and what the model is asked.
    let edited = ok(
        &router,
        "edit",
        json!({"script": "media add depth path=\"shots/depth.mp4\" rate=12 dur=6f video=160x90\n\
            step depth add render s2 {guide.mp4} pass=depth size=160x90 rate=12\n\
            step depth add generate model=shot prompt=\"a red car on a wet street\" guide={guide.mp4} seed=7"}),
    );
    assert_eq!(edited["status"], "accepted", "{edited}");
    let wants = to_make(&edited, "depth").unwrap();
    assert!(
        wants.get("cannot").is_none() && wants.get("notes").is_none(),
        "{wants}"
    );
    let take = ok(&router, "make", json!({"media": "depth", "wait": true}));
    assert_eq!(take["status"], "made", "{take}");
    assert!(take["steps"][0].as_str().is_some_and(|step| step
        .starts_with("Kerf renders the depth of sequence s2 at 160x90, 12 frames a second")));
    assert!(dir.join("shots/depth.takes/001.work/guide.mp4").is_file());
    assert!(dir.join("shots/depth.takes/001.mp4").is_file());

    // Read as the graph the editor shows: the sequence into the guide into the model, each
    // step by its number, as the document has them.
    let graph = ok(&router, "look", json!({"graph": "depth"}));
    assert!(
        graph.get("document").is_none() && graph.get("recipe").is_none(),
        "{graph}"
    );
    assert_eq!(
        graph["text"],
        "render s2 {guide.mp4} pass=depth size=160x90 rate=12\n\
         generate model=shot prompt=\"a red car on a wet street\" guide={guide.mp4} seed=7\n"
    );
    let nodes = graph["nodes"].as_array().unwrap();
    let line_of = |finds: &dyn Fn(&Value) -> bool| {
        nodes.iter().find(|node| finds(node)).unwrap()["line"].clone()
    };
    assert_eq!(line_of(&|node| node["id"] == "made:guide.mp4"), 1);
    assert_eq!(line_of(&|node| node["model"] == "shot"), 2);
    assert!(graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|edge| edge["from"] == "sequence:s2" && edge["to"] == "made:guide.mp4"));
    assert!(
        graph["change"]
            .as_str()
            .unwrap()
            .contains("step depth set <n>"),
        "{graph}"
    );
    assert_eq!(graph["takes"]["made"].as_array().map(Vec::len), Some(1));

    // The scene its guide is drawn from changes: the shot wants making, and says which
    // sequence. Nothing but the document ties the two.
    let moved = ok(&router, "edit", json!({"script": "use s2\nset road z=3"}));
    assert_eq!(
        to_make(&moved, "depth").unwrap()["why"],
        "sequence s2 changed after its file was made"
    );

    // One word of a step, set by a line of the edit's script; the rest of the step stays.
    let set = ok(
        &router,
        "edit",
        json!({"script": "step depth set 2 seed=9"}),
    );
    assert_eq!(set["status"], "accepted", "{set}");
    assert_eq!(
        steps(&router, "depth")[1],
        "generate model=shot prompt=\"a red car on a wet street\" guide={guide.mp4} seed=9"
    );
    // The inputs that wrote a recipe's file are no inputs of `edit` now.
    for gone in [
        json!({"recipes": {"shots/depth.kerfrecipe": "kerf-recipe 1\nrender s2 {out}\n"}}),
        json!({"recipeSet": [{"media": "depth", "line": 2, "key": "seed", "value": "10"}]}),
    ] {
        let response = router.call("edit", gone);
        assert_eq!(response["error"]["code"], "INVALID_ARGUMENTS", "{response}");
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("unknown field"),
            "{response}"
        );
    }
    assert!(steps(&router, "depth")[1].ends_with("seed=9"));

    // A step that is no step is refused where it is written, and the document is as it was.
    let wrong = refused(
        &router,
        "edit",
        json!({"script": "step depth add run vork render --out {out}"}),
    );
    assert!(wrong.contains("`run` is not a step"), "{wrong}");
    let short = refused(
        &router,
        "edit",
        json!({"script": "step depth add generate model=shot guide={guide.mp4}"}),
    );
    assert!(
        short.contains("writes `{out}`, the media's file, so it is the last"),
        "{short}"
    );
    assert_eq!(steps(&router, "depth").len(), 2);

    // ---- What may run is the person's to bound. Asked, nothing runs: the plan comes back
    // for the agent to say, with where the steps are and how the person makes it.
    given(Mode::Ask, 0);
    let asked = ok(
        &router,
        "make",
        json!({"media": "depth", "why": "another seed"}),
    );
    assert_eq!(asked["status"], "asked", "{asked}");
    let ask = asked["ask"].as_str().unwrap();
    assert!(
        ask.starts_with("Asks to make depth, as take 2, from the steps under its line in the document:\nanother seed\n1. Kerf renders"),
        "{ask}"
    );
    assert!(
        ask.ends_with("Make it with `kerf make depth`, or Make… in the editor's bin."),
        "{ask}"
    );
    assert!(!dir.join("shots/depth.takes/002.mp4").exists());
    // And past the limit they set, it is asked for again.
    given(Mode::Auto, 0);
    let past = ok(&router, "make", json!({"media": "depth"}));
    assert_eq!(past["status"], "asked", "{past}");

    // ---- What was made only to be looked at is taken out of the document once it has been
    // read. Its take stays where it was made.
    given(Mode::Auto, 8);
    ok(
        &router,
        "edit",
        json!({"script": "media add shown path=\"shots/shown.mp4\" rate=16 dur=16f video=160x90\n\
            step shown add generate model=matte guide=footage/walk.mp4 prompt=\"man, tree\""}),
    );
    let shown = ok(&router, "make", json!({"media": "shown", "wait": true}));
    assert_eq!(shown["status"], "made", "{shown}");
    ok(&router, "look", json!({"media": "shown", "at": ["0f"]}));
    let removed = ok(&router, "edit", json!({"script": "media remove shown"}));
    assert_eq!(removed["status"], "accepted", "{removed}");
    let document = router.runtime().get_state().unwrap().document;
    assert!(
        !document.contains("media shown") && !document.contains("man, tree"),
        "{document}"
    );
    assert!(dir.join("shots/shown.takes/001.mp4").is_file());
}
