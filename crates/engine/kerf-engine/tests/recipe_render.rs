//! A `render` step has Kerf draw a sequence of the project, which is how a guide blocked
//! with Kerf's own clips reaches the steps after it.

use kerf_engine::recipe::{self, model::Models};
use kerf_engine::CancellationToken;
use kerf_lang::parse_document;
use kerf_model::Catalog;

#[test]
fn a_render_step_draws_a_sequence_of_the_project_as_a_take() {
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    let project = parse_document(
        "kerf 1\nproject name=\"shots\"\n\
         media m1 path=\"shots/block.mp4\" rate=24 dur=1s video=160x90\n\
         \x20 render s2 {out}\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         sequence s2 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=12f\n",
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let models = Models::default();
    let plan = recipe::plan(&project, directory, "m1", &models).expect("a plan");
    assert_eq!(
        plan.steps,
        [format!(
            "Kerf renders sequence s2 to {}",
            directory
                .join("shots/block.takes/.making-001.mp4")
                .display()
        )]
    );
    let mut said = Vec::new();
    let made = recipe::make(
        &project,
        directory,
        &plan,
        &models,
        &CancellationToken::new(),
        |line| said.push(line.to_owned()),
    )
    .expect("a take");
    assert_eq!(made.stored, "shots/block.takes/001.mp4");
    let take = directory.join(&made.stored);
    let info = kerf_engine::info(&take.to_string_lossy()).expect("a file that reads as media");
    assert_eq!((info.width, info.height), (Some(160), Some(90)));
    let seconds = info.duration.expect("a length");
    assert!((seconds - 0.5).abs() < 0.1, "{seconds}");
    assert!(
        said.iter().any(|line| line.starts_with("rendering s2:")),
        "{said:?}"
    );
}

/// The steps of a shot from a guide, held by the document under the media they make: Kerf
/// draws the guide from a sequence, as its depth and at the size and rate the step names,
/// and the model is handed the file. Nothing beside the project says how the shot is made.
#[cfg(unix)]
#[test]
fn a_guide_is_drawn_from_a_sequence_and_handed_to_a_model_by_steps_in_the_document() {
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    // A model that answers with its guide: what it was handed is the take.
    std::fs::write(
        directory.join("models.json"),
        r#"{"shot": {"run": ["sh", "-c", "cp \"$(sed -n 's/.*\"guide\": \"\\(.*\\)\".*/\\1/p' \"$1\")\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "shot", "{request}"]}}"#,
    )
    .expect("a models file");
    let models = Models::read(&directory.join("models.json")).expect("models");
    let document = "kerf 1\nproject name=\"shots\"\n\
         media depth path=\"shots/depth.mp4\" rate=12 dur=6f video=160x90\n\
         \x20 render s2 {guide.mp4} pass=depth size=160x90 rate=12\n\
         \x20 generate model=shot prompt=\"a red car on a wet street\" guide={guide.mp4} seed=7\n\
         sequence s1 rate=24 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         sequence s2 rate=24 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip road gen=solid@1 at=0f dur=12f\n\
         \x20     set color=#404448 three_d=true rotate_x=-84 y=0.75 z=4 scale=16\n";
    let project = parse_document(document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let plan = recipe::plan(&project, directory, "depth", &models).expect("a plan");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(
        plan.steps[0],
        format!(
            "Kerf renders the depth of sequence s2 at 160x90, 12 frames a second to {}",
            directory.join("shots/depth.takes/001.work/guide.mp4").display()
        )
    );
    assert!(plan.notes.is_empty(), "{:?}", plan.notes);
    let mut said = Vec::new();
    let made = recipe::make(&project, directory, &plan, &models, &CancellationToken::new(), |line| {
        said.push(line.to_owned());
    })
    .expect("a take");
    assert_eq!(made.stored, "shots/depth.takes/001.mp4");
    assert!(said.iter().any(|line| line.starts_with("rendering s2:")), "{said:?}");
    // The take is the guide, as the step said it is drawn: half a second at 12 a second.
    for file in ["shots/depth.takes/001.mp4", "shots/depth.takes/001.work/guide.mp4"] {
        let info = kerf_engine::info(&directory.join(file).to_string_lossy()).expect("media");
        assert_eq!((info.width, info.height), (Some(160), Some(90)), "{file}");
        assert!((info.fps.expect("a rate") - 12.0).abs() < 0.01, "{file}");
        assert!((info.duration.expect("a length") - 0.5).abs() < 0.1, "{file}");
    }
    // Once the media is the take, it is made; the scene its guide is drawn from changing
    // asks for it again, and says which sequence.
    let taken = |document: &str| {
        parse_document(
            &document.replace("path=\"shots/depth.mp4\"", "path=\"shots/depth.takes/001.mp4\""),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
    };
    let wants = |project: &kerf_model::Project| {
        recipe::unmade(project, directory, project.media("depth").expect("the media"))
            .map(|unmade| unmade.reason())
    };
    assert_eq!(wants(&taken(document)), None);
    assert_eq!(
        wants(&taken(&document.replace("z=4", "z=3"))),
        Some("sequence s2 changed after its file was made".to_owned())
    );
}
