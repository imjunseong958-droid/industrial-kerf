//! A preset that carries how its media are made: saved from one project with
//! `edit`, applied in another with other footage bound to its slot, and made
//! there with `make`, against a real kerfd and with programs that stand in for
//! the models.
//!
//! Both kinds a making has travel here: a thing as a moved layer, whose matte
//! and plate are made of the footage the slot is bound to, and a shot from a
//! guide, whose scene comes with the preset.
//!
//! In a file of its own: it reads a models file of its own for the whole
//! process ([`Models::use_file`]), which the other tests' process must not see.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]

use kerf_engine::recipe::model::Models;
use kerf_mcp::permissions::{Mode, Permissions};
use kerf_mcp::KerfMcpRouter;
use kerf_model::{Catalog, Content, Project, Time};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The pattern's project: `s2` is the walk as layers, the man moved over the
/// plate with him taken out; `s1` plays the shot of the depth of `s3`.
const SOURCE: &str = "kerf 1
project name=\"A walk in layers\"
media walk path=\"footage/walk.mp4\" rate=16 dur=16f video=160x90
media man path=\"shots/man.mp4\" rate=16 dur=16f video=160x90
  generate model=matte prompt=\"man\" guide=footage/walk.mp4
media gone path=\"shots/gone.mp4\" rate=16 dur=16f video=160x90
  generate model=matte prompt=\"man\" guide=footage/walk.mp4 out={man.mp4}
  generate model=plate prompt=\"an empty dirt path\" guide=footage/walk.mp4 guide_mask={man.mp4} seed=7 reach=72
media depth path=\"shots/depth.mp4\" rate=12 dur=6f video=160x90
  render s3 {guide.mp4} pass=depth size=160x90 rate=12
  generate model=shot prompt=\"a red car on a wet street\" guide={guide.mp4} seed=7
sequence s1 rate=12 size=160x90
  track V1 video
    clip shot media=depth src=0f at=0f dur=6f
sequence s2 rate=16 size=160x90
  track V1 video
    clip plate media=gone src=0f at=0f dur=16f
  track V2 video
    clip mover media=walk src=0f at=0f dur=16f
      set x=0.25 scale=1.5
      fx key.trackmatte@1 matte=V3 composite=matte-luma
  track V3 video enabled=false
    clip moves media=man src=0f at=0f dur=16f
      set x=0.25 scale=1.5
sequence s3 rate=24 size=320x180
  track V1 video
    clip road gen=solid@1 at=0f dur=12f
      set color=#404448 three_d=true rotate_x=-84 y=0.75 z=4 scale=16
";

/// The project it is applied in: other footage, two seconds of it.
const ELSEWHERE: &str = "kerf 1
project name=\"Elsewhere\"
media other path=\"footage/other.mp4\" rate=16 dur=32f video=160x90
sequence main rate=16 size=160x90
  track V1 video
    clip old media=other src=0f at=0f dur=8f
sequence car rate=12 size=160x90
  track V1 video
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

fn footage(path: &Path, source: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", source])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .status()
        .expect("ffmpeg");
    assert!(made.success());
}

fn project(router: &KerfMcpRouter) -> Project {
    let document = router.runtime().get_state().unwrap().document;
    kerf_lang::parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|e| panic!("{e:?}\n{document}"))
        .project
}

/// The media a clip plays, and from where in it.
fn plays(project: &Project, clip: &str) -> (String, Time) {
    let clip = project
        .sequences
        .iter()
        .flat_map(|s| &s.tracks)
        .flat_map(|t| &t.clips)
        .find(|c| c.id == clip)
        .unwrap_or_else(|| panic!("no clip {clip}"));
    match &clip.content {
        Content::Media {
            media, source_in, ..
        } => (media.to_string(), *source_in),
        other => panic!("{other:?}"),
    }
}

/// What the last model of a media's first take was asked. A take one model made has its
/// request in its working folder, and one that several made has each in a folder of its
/// step's number: the last of those.
fn request(dir: &Path, media: &str) -> Value {
    let work = dir.join(format!("shots/{media}.takes/001.work"));
    let mut steps: Vec<_> = std::fs::read_dir(&work)
        .unwrap_or_else(|e| panic!("{}: {e}", work.display()))
        .filter_map(|entry| Some(entry.ok()?.path().join("request.json")))
        .filter(|file| file.is_file())
        .collect();
    steps.sort();
    let file = steps.pop().unwrap_or_else(|| work.join("request.json"));
    serde_json::from_slice(
        &std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display())),
    )
    .unwrap()
}

fn router(dir: &Path) -> KerfMcpRouter {
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = KerfMcpRouter::new(dir, Some(&kerfd)).unwrap();
    router.set_permissions(Some(Permissions {
        mode: Mode::Auto,
        makes: 8,
        ..Permissions::default()
    }));
    router
}

#[test]
fn a_preset_carries_how_its_media_are_made_and_they_are_made_where_it_is_applied() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let folders = tempfile::tempdir().unwrap();
    let (source, elsewhere) = (
        folders.path().join("source"),
        folders.path().join("elsewhere"),
    );
    footage(
        &source.join("footage/walk.mp4"),
        "testsrc=size=160x90:rate=16:duration=1",
    );
    footage(
        &elsewhere.join("footage/other.mp4"),
        "testsrc2=size=160x90:rate=16:duration=2",
    );
    std::fs::write(source.join("project.kerf"), SOURCE).unwrap();
    std::fs::write(elsewhere.join("project.kerf"), ELSEWHERE).unwrap();
    let models = folders.path().join("models.json");
    std::fs::write(
        &models,
        format!(
            r#"{{"matte": {{"run": {ANSWERS_ITS_GUIDE}, "takes": {{"about": "a matte of the thing the prompt names", "reads": ["guide"]}}}},
                "plate": {{"run": {ANSWERS_ITS_GUIDE}, "takes": {{"about": "the guide with what guide_mask is white on taken out", "reads": ["guide", "guide_mask"]}}}},
                "shot": {{"run": {ANSWERS_ITS_GUIDE}, "takes": {{"about": "a shot that follows a depth guide", "size": [160, 90], "rate": 12, "seconds": 5, "reads": ["guide"]}}}}}}"#
        ),
    )
    .unwrap();
    Models::use_file(&models);

    // ---- Saved where the pattern was made: each a preset of its own.
    let author = router(&source);
    let saved = ok(
        &author,
        "edit",
        json!({"savePreset": "layers.kerfpreset", "sequence": "s2", "name": "Moved"}),
    );
    assert_eq!(
        (saved["slots"].as_u64(), saved["made"].as_u64()),
        (Some(1), Some(2)),
        "{saved}"
    );
    let saved = ok(
        &author,
        "edit",
        json!({"savePreset": "car.kerfpreset", "sequence": "s1", "name": "Car"}),
    );
    assert_eq!(
        (saved["slots"].as_u64(), saved["made"].as_u64()),
        (Some(0), Some(1)),
        "{saved}"
    );
    let layers = source
        .join("layers.kerfpreset")
        .to_string_lossy()
        .into_owned();
    let car = source.join("car.kerfpreset").to_string_lossy().into_owned();
    let text = std::fs::read_to_string(&layers).unwrap();
    assert!(
        text.contains("guide=slot:slot1") && !text.contains("footage/walk.mp4"),
        "{text}"
    );

    // ---- Looked at where it is applied: one slot, which the makings read, and the media
    // the preset makes apart from it, with their steps.
    let applier = router(&elsewhere);
    let looked = ok(&applier, "look", json!({"preset": layers}));
    let slots = looked["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 1, "{looked}");
    assert_eq!(slots[0]["readBy"], json!(["man", "gone"]));
    assert_eq!(slots[0]["fits"][0]["media"], "other");
    let made = looked["made"].as_array().unwrap();
    assert_eq!(made.len(), 2, "{looked}");
    assert_eq!(made[0]["id"], "man");
    assert_eq!(made[0]["of"], "slot1");
    assert_eq!(
        made[0]["steps"],
        json!(["generate model=matte prompt=\"man\" guide=slot:slot1"])
    );
    // A made media takes no binding.
    let wrong = refused(
        &applier,
        "edit",
        json!({"preset": layers, "bindings": {"slot1": {"media": "other"}, "man": {"media": "other"}}}),
    );
    assert!(wrong.contains("takes no binding"), "{wrong}");

    // ---- A thing as a moved layer: applied with the other footage from its fifth frame.
    let applied = ok(
        &applier,
        "edit",
        json!({"preset": layers, "bindings": {"slot1": {"media": "other", "from": "4f"}}}),
    );
    assert_eq!(applied["status"], "accepted", "{applied}");
    let ids = applied["ids"].clone();
    assert_eq!(ids["slot1"], "other");
    let (man, gone) = (
        ids["man"].as_str().unwrap().to_owned(),
        ids["gone"].as_str().unwrap().to_owned(),
    );
    let now = project(&applier);
    let matte = now.media(&man).unwrap();
    assert_eq!(matte.path, format!("shots/{man}.mp4"));
    // Of the other footage, frame for frame.
    assert_eq!(matte.duration, Time::from_seconds(2.0).unwrap());
    assert_eq!(
        kerf_lang::print_step(&now.media(&gone).unwrap().steps[1]),
        "generate model=plate prompt=\"an empty dirt path\" guide=footage/other.mp4 guide_mask={man.mp4} seed=7 reach=72"
    );
    // The layers as in the pattern, each of them from the binding's start.
    let from = Time::from_seconds(0.25).unwrap();
    for (clip, media) in [
        ("mover", "other"),
        ("moves", man.as_str()),
        ("plate", gone.as_str()),
    ] {
        assert_eq!(
            plays(&now, ids[clip].as_str().unwrap()),
            (media.to_owned(), from),
            "{clip}"
        );
    }
    let main = now.sequence("main").unwrap();
    assert!(!main.tracks[2].enabled);
    assert_eq!(
        main.tracks[1].clips[0].effects[0].fx.as_str(),
        "key.trackmatte"
    );

    // Made there: each model is handed the other footage.
    for media in [&man, &gone] {
        let take = ok(&applier, "make", json!({"media": media, "wait": true}));
        assert_eq!(take["status"], "made", "{take}");
        assert_eq!(take["path"], format!("shots/{media}.takes/001.mp4"));
        let asked = request(&elsewhere, media);
        assert!(
            asked["guide"]
                .as_str()
                .unwrap()
                .ends_with("/footage/other.mp4"),
            "{asked}"
        );
    }
    assert!(request(&elsewhere, &gone)["guides"]["mask"]
        .as_str()
        .unwrap()
        .ends_with(&format!("/shots/{gone}.takes/001.work/man.mp4")));

    // ---- A blocked scene: the scene comes with the shot, by an id of this project.
    let applied = ok(&applier, "edit", json!({"preset": car, "sequence": "car"}));
    assert_eq!(applied["status"], "accepted", "{applied}");
    let ids = applied["ids"].clone();
    let (depth, scene) = (
        ids["depth"].as_str().unwrap().to_owned(),
        ids["s3"].as_str().unwrap().to_owned(),
    );
    assert_ne!(scene, "s3");
    let now = project(&applier);
    assert_eq!(now.sequence(&scene).unwrap().tracks.len(), 1);
    assert_eq!(
        kerf_lang::print_step(&now.media(&depth).unwrap().steps[0]),
        format!("render {scene} {{guide.mp4}} pass=depth size=160x90 rate=12")
    );
    // Asked for something else before it is made, by the ids the preset was given.
    let set = ok(
        &applier,
        "edit",
        json!({"script": format!("step {depth} set 2 prompt=\"a blue car on a dry road\" seed=9")}),
    );
    assert_eq!(set["status"], "accepted", "{set}");
    let take = ok(&applier, "make", json!({"media": depth, "wait": true}));
    assert_eq!(take["status"], "made", "{take}");
    let asked = request(&elsewhere, &depth);
    assert_eq!(
        (asked["prompt"].as_str(), asked["seed"].as_u64()),
        (Some("a blue car on a dry road"), Some(9)),
        "{asked}"
    );
    assert!(elsewhere
        .join(format!("shots/{depth}.takes/001.work/guide.mp4"))
        .is_file());
}
