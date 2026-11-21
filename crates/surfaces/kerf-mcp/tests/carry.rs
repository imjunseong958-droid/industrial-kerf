//! A project made before the document held a making's steps has them in recipe
//! files. An agent carries one into the document with `edit` and `carry`,
//! against a real kerfd: the steps are written under the media, the takes the
//! recipe made stay and are not asked for again, and the next making and the
//! next change are of steps in the document. Nothing is made from a recipe's
//! file, so the take one made is written here as such a making left it.
//!
//! In a file of its own: it names a models file of its own in `KERF_MODELS`,
//! which the other tests' process must not see.
#![cfg(unix)]

use kerf_mcp::permissions::{Mode, Permissions};
use kerf_mcp::KerfMcpRouter;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const PROJECT: &str = "kerf 1
project name=\"A walk\"
media walk path=\"footage/walk.mp4\" rate=16 dur=16f video=160x90
media gone path=\"shots/gone.takes/001.mp4\" rate=16 dur=16f video=160x90 recipe=\"shots/gone.kerfrecipe\"
media open path=\"shots/open.mp4\" rate=16 dur=16f video=160x90 recipe=\"shots/open.kerfrecipe\"
sequence s1 rate=16 size=160x90
  track V1 video
    clip back media=walk src=0f at=0f dur=16f
";

const GONE: &str = "kerf-recipe 1
# The man taken out, and his shadow with him.
generate model=matte guide=../footage/walk.mp4 prompt=\"man\" out={man.mp4}
generate model=plate guide=../footage/walk.mp4 guide_mask={man.mp4} prompt=\"an empty dirt path\" reach=72 seed=7
";

/// A recipe that runs a program of its own choosing, which a document cannot name.
const OPEN: &str = "kerf-recipe 1
needs open.scene.json
run vork render open.scene.json --out {guide.mp4}
generate model=plate guide={guide.mp4}
";

/// A model's program that answers with its guide.
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

fn to_make<'a>(answer: &'a Value, id: &str) -> Option<&'a Value> {
    answer["toMake"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["media"] == id))
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// The take a making of recipe `shots/gone.kerfrecipe` left, before a document held steps:
/// its file, its record of what it was made from, and what it kept of its making.
fn legacy_take(dir: &Path) {
    let takes = dir.join("shots/gone.takes");
    std::fs::create_dir_all(takes.join("001.work")).unwrap();
    let footage = std::fs::read(dir.join("footage/walk.mp4")).unwrap();
    std::fs::write(takes.join("001.mp4"), &footage).unwrap();
    std::fs::write(takes.join("001.work/man.mp4"), &footage).unwrap();
    std::fs::write(takes.join("001.work/made-from.kerfrecipe"), GONE).unwrap();
    let made = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let record = json!({
        "kerf-take": 1,
        "made": made,
        "from": {
            "recipe": sha256(kerf_engine::recipe::Recipe::essence(GONE).as_bytes()),
            "file footage/walk.mp4": sha256(&footage),
        },
    });
    std::fs::write(
        takes.join("001.take.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
}

#[test]
fn an_agent_carries_a_recipe_into_the_document_and_its_takes_stay() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    std::fs::create_dir(dir.join("footage")).unwrap();
    std::fs::create_dir(dir.join("shots")).unwrap();
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
    std::fs::write(dir.join("shots/gone.kerfrecipe"), GONE).unwrap();
    std::fs::write(dir.join("shots/open.kerfrecipe"), OPEN).unwrap();
    legacy_take(dir);
    let models = dir.join("models.json");
    std::fs::write(
        &models,
        format!(r#"{{"matte": {{"run": {ANSWERS_ITS_GUIDE}}}, "plate": {{"run": {ANSWERS_ITS_GUIDE}}}}}"#),
    )
    .unwrap();
    std::env::set_var("KERF_MODELS", &models);
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(dir, Some(&kerfd)).unwrap();
    router.set_permissions(Some(Permissions {
        mode: Mode::Auto,
        makes: 8,
        ..Permissions::default()
    }));

    // As the project was: the media is a take of the recipe's file. What wants making says
    // the steps are in a file, and how they come into the document; nothing is made of it.
    let looked = ok(&router, "look", json!({}));
    let wants = to_make(&looked, "gone").unwrap_or_else(|| panic!("{looked}"));
    assert_eq!(wants["recipe"], "shots/gone.kerfrecipe");
    assert!(
        wants["carry"]
            .as_str()
            .unwrap()
            .contains("carry: [\"gone\"]"),
        "{wants}"
    );
    let unmade = refused(&router, "make", json!({"media": "gone", "wait": true}));
    assert!(
        unmade.contains("are in the recipe file `shots/gone.kerfrecipe`"),
        "{unmade}"
    );
    assert!(unmade.contains("`carry: [\"gone\"]`"), "{unmade}");
    assert!(!dir.join("shots/gone.takes/002.mp4").exists());

    // Carried: its steps are under the media, each file named from the project's folder,
    // and the line names no recipe. The take it had is held to the steps it became.
    let carried = ok(&router, "edit", json!({"carry": ["gone"]}));
    assert_eq!(carried["status"], "accepted", "{carried}");
    assert_eq!(
        carried["carried"],
        json!([{"media": "gone", "recipe": "shots/gone.kerfrecipe", "takesHeld": 1}])
    );
    let document = router.runtime().get_state().unwrap().document;
    assert!(
        document.contains(
            "media gone path=\"shots/gone.takes/001.mp4\" rate=16 dur=00:00:01:00 video=160x90\n\
             \x20 generate model=matte prompt=\"man\" guide=footage/walk.mp4 out={man.mp4}\n\
             \x20 generate model=plate prompt=\"an empty dirt path\" guide=footage/walk.mp4 guide_mask={man.mp4} seed=7 reach=72\n"
        ),
        "{document}"
    );
    assert!(to_make(&carried, "gone").is_none(), "{carried}");
    assert!(to_make(&ok(&router, "look", json!({})), "gone").is_none());
    // The recipe's file is left where it was, and nothing names it.
    assert_eq!(
        std::fs::read_to_string(dir.join("shots/gone.kerfrecipe")).unwrap(),
        GONE
    );
    let listed = ok(&router, "look", json!({"media": "gone"}));
    let takes = listed["takes"]["made"].as_array().unwrap();
    assert_eq!(takes.len(), 1, "{listed}");
    assert!(
        takes[0].get("changedSince").is_none() && takes[0].get("steps").is_none(),
        "{listed}"
    );

    // From here it is a media made by steps in the document: a name set by a script, and
    // the next take beside the one the recipe made.
    let seeded = ok(&router, "edit", json!({"script": "step gone set 2 seed=8"}));
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
    assert_eq!(second["path"], "shots/gone.takes/002.mp4");
    assert!(dir.join("shots/gone.takes/001.mp4").is_file());
    // Going back to the take the recipe made sets the steps it was made from, read as
    // steps of the document: its footage named from the project's folder.
    let listed = ok(&router, "look", json!({"media": "gone"}));
    let kept = &listed["takes"]["made"][0]["steps"];
    assert_eq!(
        kept[1],
        "generate model=plate prompt=\"an empty dirt path\" guide=footage/walk.mp4 guide_mask={man.mp4} seed=7 reach=72",
        "{listed}"
    );
    let back = ok(
        &router,
        "edit",
        json!({"restoreTake": {"media": "gone", "take": 1}}),
    );
    assert!(
        back["script"]
            .as_str()
            .unwrap()
            .contains("step gone set 2 generate model=plate"),
        "{back}"
    );
    let document = router.runtime().get_state().unwrap().document;
    assert!(
        document.contains("guide_mask={man.mp4} seed=7 reach=72\n"),
        "{document}"
    );

    // A recipe that runs a program is not carried: a document names none, and the media
    // still names its file. Nor is a recipe carried twice.
    let ran = refused(&router, "edit", json!({"carry": ["open"]}));
    assert!(
        ran.contains("`run vork` runs a program, and a document names none"),
        "{ran}"
    );
    assert!(router
        .runtime()
        .get_state()
        .unwrap()
        .document
        .contains("recipe=\"shots/open.kerfrecipe\""));
    let done = refused(&router, "edit", json!({"carry": ["gone"]}));
    assert!(
        done.contains("has its steps under its line in the document already"),
        "{done}"
    );
}

/// Carried in the same edit as the lines that change its steps: the carry comes first.
#[test]
fn a_recipe_is_carried_and_its_steps_changed_in_one_edit() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    std::fs::create_dir(dir.join("shots")).unwrap();
    std::fs::write(
        dir.join("project.kerf"),
        "kerf 1\nproject name=\"Shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=24f video=320x180 recipe=\"shots/open.kerfrecipe\"\nsequence s1 rate=24 size=320x180\n  track V1 video\n    clip slate gen=solid@1 at=0f dur=24f\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\n# The slate.\nrender s1 {out} size=160x90\n",
    )
    .unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(dir, Some(&kerfd)).unwrap();
    router.set_permissions(Some(Permissions::default()));
    let staged = ok(
        &router,
        "edit",
        json!({"carry": ["m1"], "script": "step m1 set 1 rate=12"}),
    );
    assert_eq!(staged["status"], "awaiting_review", "{staged}");
    assert_eq!(staged["carried"][0]["takesHeld"], 0);
    let script = staged["script"].as_str().unwrap();
    assert!(
        script.contains(
            "media replace m1 path=\"shots/open.mp4\" rate=24 dur=00:00:01:00 video=320x180\n"
        ),
        "{script}"
    );
    assert!(
        script.contains("step m1 add render s1 {out} size=160x90\n"),
        "{script}"
    );
    assert!(
        script.contains("step m1 set 1 render s1 {out} size=160x90 rate=12"),
        "{script}"
    );
    // Until the person says yes the project names its recipe, as it did.
    assert!(router
        .runtime()
        .get_state()
        .unwrap()
        .document
        .contains("recipe=\"shots/open.kerfrecipe\""));
}
