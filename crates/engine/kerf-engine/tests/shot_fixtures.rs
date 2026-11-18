//! The projects under `fixtures/shots` say how each of their media is made in the
//! document itself, as steps under the media's line. They are held here to what a
//! making would ask of them: they read, each step is written as Kerf writes it, and a
//! plan of every media can be made, with nothing run.

use kerf_engine::recipe::{self, model::Models, Home};
use kerf_lang::{parse_document, print_step};
use kerf_model::{Catalog, Project};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/shots")
        .join(name)
}

/// The fixture's project, read, with each step under a media checked against the line the
/// file has for it.
fn read(name: &str) -> Project {
    let text = std::fs::read_to_string(fixture(name).join("project.kerf")).expect("the fixture");
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{name}: {errors:?}"))
        .project;
    let written: Vec<&str> = text
        .lines()
        .skip_while(|line| !line.starts_with("media "))
        .take_while(|line| !line.starts_with("sequence "))
        .filter(|line| line.starts_with("  "))
        .map(str::trim)
        .collect();
    let printed: Vec<String> = project
        .media
        .iter()
        .flat_map(|media| media.steps.iter().map(print_step))
        .collect();
    assert_eq!(written, printed, "{name}: each step as Kerf writes it");
    project
}

/// The models the fixtures name, none of which is run here.
fn models(folder: &Path) -> Models {
    let file = folder.join("models.json");
    let named = ["shot", "depth", "cameras", "matte", "shown", "into", "plate"]
        .map(|name| format!(r#""{name}": {{"run": ["no-such-program", "{{request}}"]}}"#))
        .join(", ");
    std::fs::write(&file, format!("{{{named}}}")).expect("a models file");
    Models::read(&file).expect("models")
}

#[test]
fn the_car_on_a_road_is_made_by_steps_in_its_document() {
    let project = read("car");
    let folder = tempfile::tempdir().expect("a folder");
    let models = models(folder.path());
    for media in &project.media {
        assert_eq!(Home::of(media), Some(Home::Document), "{}", media.id);
        let plan = recipe::plan(&project, folder.path(), media.id.as_str(), &models)
            .unwrap_or_else(|error| panic!("{}: {error}", media.id));
        assert_eq!(plan.recipe, None);
        assert!(plan.steps[0].starts_with("Kerf renders "), "{}", plan.steps[0]);
        assert!(plan.notes.is_empty(), "{}: {:?}", media.id, plan.notes);
    }
    // The depth shot: the guide is the depth of `s2`, at the model's size and rate, and
    // the model is asked with the prompt and the seed the document has.
    let depth = recipe::plan(&project, folder.path(), "depth", &models).expect("a plan");
    assert_eq!(depth.steps.len(), 2);
    assert!(
        depth.steps[0].starts_with("Kerf renders the depth of sequence s2 at 832x480, 16 frames a second to "),
        "{}",
        depth.steps[0]
    );
    assert!(depth.steps[1].contains("asks model `shot` for prompt \"a red sports car drives"), "{}", depth.steps[1]);
    assert!(depth.steps[1].ends_with("guide {guide.mp4}, 4 s, seed 7"), "{}", depth.steps[1]);
    assert_eq!(depth.stored, "shots/depth.takes/001.mp4");
}

#[test]
fn a_walk_in_layers_is_made_by_steps_in_its_document() {
    let project = read("walk");
    // The footage is not in the repository: a file stands in for it, in a copy.
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("footage")).expect("a folder");
    std::fs::write(directory.join("footage/walk.mp4"), "footage").expect("a file");
    let models = models(directory);
    let made: Vec<&str> = project
        .media
        .iter()
        .filter(|media| Home::of(media).is_some())
        .map(|media| media.id.as_str())
        .collect();
    assert_eq!(made, ["man", "shown", "shirt", "gone", "layers", "moved"]);
    assert_eq!(Home::of(project.media("walk").expect("the footage")), None);
    for id in made {
        let plan = recipe::plan(&project, directory, id, &models)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert!(plan.notes.is_empty(), "{id}: {:?}", plan.notes);
    }
    // The man taken out: his matte first, by his name, and the plate that reads it.
    let gone = recipe::plan(&project, directory, "gone", &models).expect("a plan");
    assert_eq!(gone.steps.len(), 2);
    assert!(gone.steps[0].contains("asks model `matte` for prompt \"man\", guide footage/walk.mp4"), "{}", gone.steps[0]);
    assert!(gone.steps[1].contains("asks model `plate`"), "{}", gone.steps[1]);
    assert!(gone.steps[1].contains("guide_mask {man.mp4}"), "{}", gone.steps[1]);
    assert!(gone.steps[1].ends_with("seed 7, reach 72"), "{}", gone.steps[1]);
    // Without the footage, that is what is missing, named from the project's folder.
    let empty = tempfile::tempdir().expect("a folder");
    let missing = recipe::plan(&project, empty.path(), "gone", &models).unwrap_err().to_string();
    assert_eq!(missing, "the steps need `footage/walk.mp4`, which is not there");
}
