//! A project worked on from a terminal, with no editor: `kerf load` takes a
//! `project.kerf` changed by hand, and `kerf proposals`, `kerf accept` and
//! `kerf reject` answer what was proposed. Run against a real kerfd.

use kerf_runtime::{ProposalFreshness, RuntimeClient};
use std::path::{Path, PathBuf};
use std::process::Command;

const DOCUMENT: &str = "kerf 1
project name=\"terminal\"
sequence s1 rate=24 size=320x180 audio=48000
  track V1 video
    clip c1 gen=solid@1 at=0f dur=24f
";

/// The kerfd the workspace built, as `kerf-mcp`'s tests find it.
fn kerfd() -> Option<PathBuf> {
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/debug")
        .join(format!("kerfd{}", std::env::consts::EXE_SUFFIX));
    if kerfd.is_file() {
        return Some(kerfd);
    }
    eprintln!(
        "skipping: no kerfd at {}; `cargo build -p kerf-runtime` builds it",
        kerfd.display()
    );
    None
}

/// Run `kerf` on the project in `directory`, with no terminal to ask at: what it printed,
/// what it said, and whether it ended well. The models it reads are the project folder's
/// `models.json`, and none where there is none: never those of whoever runs the tests.
fn kerf(directory: &Path, kerfd: &Path, words: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_kerf"))
        .env("KERF_MODELS", directory.join("models.json"))
        .arg("--dir")
        .arg(directory)
        .arg("--kerfd")
        .arg(kerfd)
        .args(words)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("kerf runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn proposals_are_listed_accepted_and_rejected_from_a_terminal() {
    let Some(kerfd) = kerfd() else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::write(directory.join("project.kerf"), DOCUMENT).unwrap();
    let runtime = RuntimeClient::connect_or_spawn(directory, Some(&kerfd)).unwrap();
    let _held = runtime.hold();
    let run = |words: &[&str]| kerf(directory, &kerfd, words);

    let (listed, _, ok) = run(&["proposals"]);
    assert!(ok);
    assert_eq!(listed, "no proposal is waiting\n");
    let (_, said, ok) = run(&["accept", "--yes"]);
    assert!(!ok);
    assert!(said.contains("no proposal is waiting"), "{said}");

    // What an agent's `edit` stages.
    let staged = runtime
        .stage_script_proposal("one", 0, "set c1 opacity=0.5", None)
        .unwrap();
    let first = staged.proposals[0].proposal_id.clone();
    let (listed, _, ok) = run(&["proposals"]);
    assert!(ok);
    assert!(listed.starts_with(&format!("{first}  v1  by ")), "{listed}");
    let (shown, _, ok) = run(&["proposals", &first]);
    assert!(ok);
    assert!(shown.contains("set c1 opacity=0.5"), "{shown}");
    assert!(shown.contains("+      set opacity=0.5"), "{shown}");

    // With nobody at a terminal to say yes, it is shown and left as it was.
    let (shown, said, ok) = run(&["accept"]);
    assert!(!ok);
    assert!(shown.contains("+      set opacity=0.5"), "{shown}");
    assert!(
        said.contains(&format!("not accepted; `kerf accept {first} --yes`")),
        "{said}"
    );
    assert_eq!(runtime.get_state().unwrap().revision, 0);

    let (accepted, said, ok) = run(&["accept", "--yes"]);
    assert!(ok, "{said}");
    assert!(
        accepted.ends_with(&format!("accepted {first} (revision 1)\n")),
        "{accepted}"
    );
    let file = std::fs::read_to_string(directory.join("project.kerf")).unwrap();
    assert!(file.contains("      set opacity=0.5"), "{file}");
    assert_eq!(run(&["proposals"]).0, "no proposal is waiting\n");

    // Two wait: one is named, by its id or the end of it.
    runtime
        .stage_script_proposal("two", 1, "set c1 opacity=0.25", None)
        .unwrap();
    let staged = runtime
        .stage_script_proposal("three", 1, "set c1 scale=0.5", None)
        .unwrap();
    let (newer, older) = (
        staged.proposals[0].proposal_id.clone(),
        staged.proposals[1].proposal_id.clone(),
    );
    let (_, said, ok) = run(&["accept", "--yes"]);
    assert!(!ok);
    assert!(said.contains("2 proposals wait; name one"), "{said}");
    let (rejected, said, ok) = run(&["reject", &older]);
    assert!(ok, "{said}");
    assert_eq!(
        rejected,
        format!("rejected {older}; the project is as it was (revision 1)\n")
    );
    let end = &newer[newer.len() - 4..];
    let (accepted, said, ok) = run(&["accept", end, "--yes"]);
    assert!(ok, "{said}");
    assert!(
        accepted.ends_with(&format!("accepted {newer} (revision 2)\n")),
        "{accepted}"
    );
    let file = std::fs::read_to_string(directory.join("project.kerf")).unwrap();
    assert!(file.contains("scale=0.5"), "{file}");
    assert!(!file.contains("opacity=0.25"), "{file}");
}

#[test]
fn a_proposal_made_before_the_project_changed_is_rejected_from_a_terminal() {
    let Some(kerfd) = kerfd() else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::write(directory.join("project.kerf"), DOCUMENT).unwrap();
    let runtime = RuntimeClient::connect_or_spawn(directory, Some(&kerfd)).unwrap();
    let _held = runtime.hold();

    let staged = runtime
        .stage_script_proposal("stale", 0, "set c1 opacity=0.5", None)
        .unwrap();
    let id = staged.proposals[0].proposal_id.clone();
    runtime
        .apply_script_request("ui", "later", "set c1 scale=0.5", 0)
        .unwrap();
    let before = runtime.get_state().unwrap();
    assert_eq!(before.revision, 1);
    assert_eq!(before.proposals[0].freshness, ProposalFreshness::Stale);

    let (rejected, said, ok) = kerf(directory, &kerfd, &["reject", &id]);
    assert!(ok, "{said}");
    assert_eq!(
        rejected,
        format!("rejected {id}; the project is as it was (revision 1)\n")
    );
    let after = runtime.get_state().unwrap();
    assert_eq!(after.revision, 1);
    assert_eq!(after.document, before.document);
    assert_eq!(after.proposals[0].status, "rejected");
}

#[test]
fn a_project_changed_by_hand_is_loaded_before_anything_is_made_from_it() {
    let Some(kerfd) = kerfd() else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::write(directory.join("project.kerf"), DOCUMENT).unwrap();
    let runtime = RuntimeClient::connect_or_spawn(directory, Some(&kerfd)).unwrap();
    let _held = runtime.hold();
    let run = |words: &[&str]| kerf(directory, &kerfd, words);
    runtime
        .stage_script_proposal("one", 0, "set c1 opacity=0.5", None)
        .unwrap();

    let (said, _, ok) = run(&["load"]);
    assert!(ok);
    assert_eq!(
        said,
        "project.kerf is the project as Kerf has it (revision 0); nothing to load\n"
    );

    // A media with a step under it and another clip, written by hand, as an agent with
    // only file tools would; its model is a program that is not there.
    std::fs::write(
        directory.join("models.json"),
        r#"{"absent": {"run": ["no-such-program-of-kerf", "{out}"]}}"#,
    )
    .unwrap();
    let edited = DOCUMENT.replace(
        "sequence s1",
        "media m1 path=\"shots/a.mp4\" rate=24 dur=24f video=320x180\n  generate model=absent prompt=\"a boat\"\nsequence s1",
    ) + "    clip c2 gen=solid@1 at=24f dur=24f\n";
    std::fs::write(directory.join("project.kerf"), &edited).unwrap();

    // Kerf has the project as it was: nothing is made from it, or answered, until the file
    // is loaded, and that is said before anything runs.
    for words in [
        &["make", "m1", "--yes"][..],
        &["takes", "m1"],
        &["accept", "--yes"],
    ] {
        let (_, said, ok) = run(words);
        assert!(!ok, "{words:?}");
        assert!(
            said.contains("project.kerf was changed outside Kerf")
                && said.contains("`kerf load` takes the file as the project"),
            "{words:?}: {said}"
        );
    }

    // What loading would change is shown, and nothing is loaded with nobody to say yes.
    let (shown, said, ok) = run(&["load"]);
    assert!(!ok);
    assert!(shown.contains("+media m1 path=\"shots/a.mp4\""), "{shown}");
    assert!(shown.contains("+    clip c2 gen=solid@1"), "{shown}");
    assert!(
        shown.contains("and drops the proposal that waits"),
        "{shown}"
    );
    assert!(
        said.contains("not loaded; `kerf load --yes` loads it without asking"),
        "{said}"
    );
    assert_eq!(runtime.get_state().unwrap().revision, 0);

    let (loaded, said, ok) = run(&["load", "--yes"]);
    assert!(ok, "{said}");
    assert!(
        loaded.ends_with("loaded project.kerf (revision 1)\n"),
        "{loaded}"
    );
    let state = runtime.get_state().unwrap();
    assert_eq!(state.projection_status, "current");
    assert!(state.document.contains("clip c2"), "{}", state.document);
    assert_eq!(run(&["proposals"]).0, "no proposal is waiting\n");

    // Now the steps are the file's: the making gets as far as its own step.
    let (_, said, ok) = run(&["make", "m1", "--yes"]);
    assert!(!ok);
    assert!(said.contains("could not start"), "{said}");
    assert_eq!(
        run(&["takes", "m1"]).0.trim_end(),
        "m1 has no takes yet; `kerf make m1` makes one"
    );

    // A file that does not read is refused with its line, and Kerf keeps what it has.
    std::fs::write(
        directory.join("project.kerf"),
        format!("{edited}    clip c3 gen=nothing@9 at=48f dur=24f\n"),
    )
    .unwrap();
    let (_, said, ok) = run(&["load", "--yes"]);
    assert!(!ok);
    assert!(said.contains("does not read"), "{said}");
    assert!(said.contains("unknown generator `nothing`"), "{said}");
    assert_eq!(runtime.get_state().unwrap().revision, 1);
}

/// A project written before steps were the document's: its media names a recipe file and
/// has no steps under its line.
const LEGACY: &str = "kerf 1
project name=\"legacy\"
media m1 path=\"shots/open.mp4\" rate=24 dur=24f video=160x90 recipe=\"shots/open.kerfrecipe\"
sequence s1 rate=24 size=160x90 audio=48000
  track V1 video
    clip c1 gen=solid@1 at=0f dur=24f
";

#[test]
fn a_media_whose_steps_are_in_a_recipe_file_is_made_once_they_are_carried() {
    let Some(kerfd) = kerfd() else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    std::fs::write(
        directory.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nrender s1 {out} size=160x90\n",
    )
    .unwrap();
    std::fs::write(directory.join("project.kerf"), LEGACY).unwrap();
    let runtime = RuntimeClient::connect_or_spawn(directory, Some(&kerfd)).unwrap();
    let _held = runtime.hold();
    let run = |words: &[&str]| kerf(directory, &kerfd, words);

    // Nothing is made from a recipe file: how its steps come into the document is said.
    let (_, said, ok) = run(&["make", "m1", "--yes"]);
    assert!(!ok);
    assert!(
        said.contains("the steps of media `m1` are in the recipe file `shots/open.kerfrecipe`")
            && said.contains("`kerf carry m1`"),
        "{said}"
    );
    assert!(!directory.join("shots/open.takes").exists());
    assert_eq!(runtime.get_state().unwrap().revision, 0);

    // Carried, the steps are under the media's line and its line names no recipe.
    let (carried, said, ok) = run(&["carry", "m1", "--yes"]);
    assert!(ok, "{said}");
    assert!(
        carried.contains("m1, from shots/open.kerfrecipe:"),
        "{carried}"
    );
    assert!(
        carried.contains("  step m1 add render s1 {out} size=160x90\n"),
        "{carried}"
    );
    assert!(carried.contains("carried m1 (revision 1)"), "{carried}");
    let document = runtime.get_state().unwrap().document;
    assert!(
        document.contains("\n  render s1 {out} size=160x90\n"),
        "{document}"
    );
    assert!(!document.contains("recipe="), "{document}");
    // The recipe file is left where it is.
    assert!(directory.join("shots/open.kerfrecipe").is_file());

    // And the media is made from them.
    let (made, said, ok) = run(&["make", "m1", "--yes"]);
    assert!(ok, "{said}");
    assert!(
        made.contains("steps   under `m1` in the document"),
        "{made}"
    );
    assert!(
        directory.join("shots/open.takes/001.mp4").is_file(),
        "{made}"
    );
}
