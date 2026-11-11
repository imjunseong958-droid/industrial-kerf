//! A recipe's `render` step has Kerf draw a sequence of the project, which is how a guide blocked
//! with Kerf's own clips reaches the steps after it.

use kerf_engine::recipe::{self, model::Models};
use kerf_engine::CancellationToken;
use kerf_lang::parse_document;
use kerf_model::Catalog;

#[test]
fn a_recipe_renders_a_sequence_of_the_project_as_a_take() {
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).expect("a folder");
    std::fs::write(
        directory.join("shots/block.kerfrecipe"),
        "kerf-recipe 1\n# The blocking, as the guide a model would follow.\nrender s2 {out}\n",
    )
    .expect("a recipe");
    let project = parse_document(
        "kerf 1\nproject name=\"shots\"\n\
         media m1 path=\"shots/block.mp4\" rate=24 dur=1s video=160x90 recipe=\"shots/block.kerfrecipe\"\n\
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
