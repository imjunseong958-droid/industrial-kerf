//! The projects under `fixtures/shots` say how each of their media is made in the
//! document itself, as steps under the media's line. They are held here to what a
//! making would ask of them: they read, each step is written as Kerf writes it, and a
//! plan of every media can be made, with nothing run.

use kerf_engine::recipe;
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
fn models() -> impl kerf_engine::recipe::model::Generator {
    crate::models::not_run(&[
        "shot", "depth", "cameras", "matte", "shown", "into", "plate", "directed", "led", "still",
    ])
}

#[test]
fn the_car_on_a_road_is_made_by_steps_in_its_document() {
    let project = read("car");
    let folder = tempfile::tempdir().expect("a folder");
    let models = models();
    for media in &project.media {
        assert!(
            !media.steps.is_empty() && recipe::uncarried(media).is_none(),
            "{}",
            media.id
        );
        let plan = recipe::plan(&project, folder.path(), media.id.as_str(), &models)
            .unwrap_or_else(|error| panic!("{}: {error}", media.id));
        assert_eq!(plan.steps.len(), media.steps.len(), "{}", media.id);
        assert!(
            plan.steps[0].starts_with("Kerf renders "),
            "{}",
            plan.steps[0]
        );
        assert!(plan.notes.is_empty(), "{}: {:?}", media.id, plan.notes);
    }
    // The depth shot: the guide is the depth of `s2`, at the model's size and rate, and
    // the model is asked with the prompt and the seed the document has.
    let depth = recipe::plan(&project, folder.path(), "depth", &models).expect("a plan");
    assert_eq!(depth.steps.len(), 2);
    assert!(
        depth.steps[0].starts_with(
            "Kerf renders the depth of sequence s2 at 832x480, 16 frames a second to "
        ),
        "{}",
        depth.steps[0]
    );
    assert!(
        depth.steps[1].contains("asks model `shot` for prompt \"a red sports car drives"),
        "{}",
        depth.steps[1]
    );
    assert!(
        depth.steps[1].ends_with("guide {guide.mp4}, 4 s, seed 7"),
        "{}",
        depth.steps[1]
    );
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
    let models = models();
    let made: Vec<&str> = project
        .media
        .iter()
        .filter(|media| !media.steps.is_empty())
        .map(|media| media.id.as_str())
        .collect();
    assert_eq!(made, ["man", "shown", "shirt", "gone", "layers", "moved"]);
    assert!(project.media("walk").expect("the footage").steps.is_empty());
    for id in made {
        let plan = recipe::plan(&project, directory, id, &models)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert!(plan.notes.is_empty(), "{id}: {:?}", plan.notes);
    }
    // The man taken out: his matte first, by his name, and the plate that reads it.
    let gone = recipe::plan(&project, directory, "gone", &models).expect("a plan");
    assert_eq!(gone.steps.len(), 2);
    assert!(
        gone.steps[0].contains("asks model `matte` for prompt \"man\", guide footage/walk.mp4"),
        "{}",
        gone.steps[0]
    );
    assert!(
        gone.steps[1].contains("asks model `plate`"),
        "{}",
        gone.steps[1]
    );
    assert!(
        gone.steps[1].contains("guide_mask {man.mp4}"),
        "{}",
        gone.steps[1]
    );
    assert!(
        gone.steps[1].ends_with("seed 7, reach 72"),
        "{}",
        gone.steps[1]
    );
    // Without the footage, that is what is missing, named from the project's folder.
    let empty = tempfile::tempdir().expect("a folder");
    let missing = recipe::plan(&project, empty.path(), "gone", &models)
        .unwrap_err()
        .to_string();
    assert_eq!(
        missing,
        "the steps need `footage/walk.mp4`, which is not there"
    );
}

#[test]
fn scenes_of_solids_are_made_by_steps_in_their_document() {
    let project = read("solids");
    // Its meshes' files and its cast are the fixture's own. Two of its media read a take
    // made before them, which is not in the repository: a file stands in for each, in a copy.
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("models")).expect("a folder");
    for file in [
        "models/rocket.glb",
        "models/donut.glb",
        "models/saucer.glb",
        "cast.kerfcast",
    ] {
        std::fs::copy(fixture("solids").join(file), directory.join(file))
            .expect("the fixture's file");
    }
    for take in ["shots/snowman.takes/001.mp4", "shots/robot.takes/001.mp4"] {
        let file = directory.join(take);
        std::fs::create_dir_all(file.parent().expect("a folder")).expect("a folder");
        std::fs::write(file, "a take").expect("a file");
    }
    let models = models();
    let made: Vec<&str> = project
        .media
        .iter()
        .filter(|media| !media.steps.is_empty())
        .map(|media| media.id.as_str())
        .collect();
    // Nine scenes, six shots of a film, and one media for each other thing a step does.
    assert_eq!(made.len(), 26, "{made:?}");
    for id in &made {
        let plan = recipe::plan(&project, directory, id, &models)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(
            plan.steps.len(),
            project.media(id).expect("it").steps.len(),
            "{id}"
        );
    }
    let plan = |id: &str| {
        recipe::plan(&project, directory, id, &models)
            .expect("a plan")
            .steps
    };
    // A shot one model makes has its request in its take's working folder.
    assert!(plan("rocket_shot")[1].contains("/shots/rocket.takes/001.work/request.json"));
    // All four directions on one line: a depth guide, when the ball lands, and where it goes.
    let strike = plan("bowling_shot");
    assert!(strike[1].contains("asks model `directed`"), "{}", strike[1]);
    assert!(
        strike[1].ends_with("guide {guide.mp4}, seed 7, signal strike, moves ball"),
        "{}",
        strike[1]
    );
    // A still for the shot to look like, and the shot: two models, a request each.
    let donut = plan("donut_shot");
    assert!(
        donut[0].contains("/001.work/step1/request.json")
            && donut[0].contains("asks model `still`"),
        "{}",
        donut[0]
    );
    assert!(
        donut[2].contains("/001.work/step3/request.json") && donut[2].contains("ref {donut.png}"),
        "{}",
        donut[2]
    );
    // A longer shot in two parts, the second going on from the first, and the two joined.
    let long = plan("city_long");
    assert!(
        long[1].contains(", 2.5625 s of it from 2.5 s"),
        "{}",
        long[1]
    );
    assert!(
        long[3].contains("first {a.mp4}") && long[3].contains("/step4/request.json"),
        "{}",
        long[3]
    );
    assert!(
        long[4].starts_with("Kerf joins {a.mp4}, {b.mp4} end to end into "),
        "{}",
        long[4]
    );
    assert!(
        long[4].ends_with("each part after the first without its first frame"),
        "{}",
        long[4]
    );
    // A film's shot is asked with who is in it and how it looks, from the cast's file.
    let moon = plan("f6");
    assert!(
        moon[1].contains("Bolt: a chunky retro tin toy robot"),
        "{}",
        moon[1]
    );
    assert!(
        moon[1].contains("rocket: a white retro rocket ship"),
        "{}",
        moon[1]
    );
    assert!(
        moon[1].contains("1950s pulp science fiction film"),
        "{}",
        moon[1]
    );
    // A pass alone is a media Kerf draws, with no model asked.
    let ids = plan("pass_ids");
    assert_eq!(ids.len(), 1);
    assert!(
        ids[0].starts_with(
            "Kerf renders the ids of sequence saucer at 832x480, 16 frames a second to "
        ),
        "{}",
        ids[0]
    );
}
