use super::model::{Ask, Models, Source};
use super::*;
use kerf_model::Project;

/// A project of `lines` after its header, read as a document is.
fn project(lines: &str) -> Project {
    kerf_lang::parse_document(
        &format!("kerf 1\nproject name=\"shots\"\n{lines}"),
        kerf_model::Catalog::builtin(),
    )
    .expect("a project")
    .project
}

/// A project whose media `m1` is made by `shots/open.kerfrecipe`.
fn shot(path: &str) -> Project {
    project(&format!(
        "media m1 path=\"{path}\" rate=24 dur=1s video=160x120 recipe=\"shots/open.kerfrecipe\"\n"
    ))
}

/// A models file in `folder` naming `models`, read.
fn models(folder: &Path, models: &str) -> Models {
    let file = folder.join("models.json");
    std::fs::write(&file, models).unwrap();
    Models::read(&file).unwrap()
}

#[test]
fn a_recipe_is_steps_and_the_last_writes_the_file() {
    let recipe = Recipe::parse(
        "kerf-recipe 1\n\n# The opening shot.\nneeds \"open scene.json\" look.cube\n\
         run vork render \"open scene.json\" --out {guide.mp4} --note \"a \\\"quoted\\\" word\"\n\
         render s2 {block.mp4}\n\
         generate model=dawn prompt=\"a boat leaves, at dawn\" guide={guide.mp4} first=refs/boat.png ref={block.mp4} ref=refs/sea.png seconds=4 size=1280x720 seed=7 strength=0.6\n",
    )
    .unwrap();
    assert_eq!(recipe.needs, ["open scene.json", "look.cube"]);
    assert_eq!(
        recipe.steps[0],
        Step::Run {
            program: "vork".to_owned(),
            arguments: [
                "render",
                "open scene.json",
                "--out",
                "{guide.mp4}",
                "--note",
                "a \"quoted\" word"
            ]
            .map(str::to_owned)
            .to_vec(),
        }
    );
    assert_eq!(
        recipe.steps[1],
        Step::Render {
            sequence: "s2".to_owned(),
            to: "block.mp4".to_owned(),
            size: None,
            rate: None,
        }
    );
    let Step::Generate(ask) = &recipe.steps[2] else {
        panic!("a generate step");
    };
    assert_eq!(ask.model, "dawn");
    assert_eq!(ask.prompt, "a boat leaves, at dawn");
    assert_eq!(ask.guide, Some(Source::Made("guide.mp4".to_owned())));
    assert_eq!(ask.first, Some(Source::File("refs/boat.png".to_owned())));
    assert_eq!(
        ask.references,
        [
            Source::Made("block.mp4".to_owned()),
            Source::File("refs/sea.png".to_owned())
        ]
    );
    assert_eq!(
        (ask.seconds, ask.size, ask.seed),
        (Some(4.0), Some((1280, 720)), Some(7))
    );
    assert_eq!(ask.more.get("strength").map(String::as_str), Some("0.6"));
    assert_eq!(ask.out, "out");
    assert_eq!(recipe.models().collect::<Vec<_>>(), ["dawn"]);
    assert_eq!(recipe.sequences().collect::<Vec<_>>(), ["s2"]);
    // What it reads: what `needs` names and what a model is handed that no step made.
    assert_eq!(
        recipe.reads(),
        [
            "look.cube",
            "open scene.json",
            "refs/boat.png",
            "refs/sea.png"
        ]
    );
    // A guide is drawn at the size and the rate a model takes, in either order.
    let sized = Recipe::parse(
        "kerf-recipe 1\nrender s2 {guide.mp4} size=832x480 rate=16\nrender s3 {out} rate=23.976 size=1280x720\n",
    )
    .unwrap();
    assert_eq!(
        sized.steps,
        [
            Step::Render {
                sequence: "s2".to_owned(),
                to: "guide.mp4".to_owned(),
                size: Some((832, 480)),
                rate: Some(Rate::new(16, 1).unwrap()),
            },
            Step::Render {
                sequence: "s3".to_owned(),
                to: "out".to_owned(),
                size: Some((1280, 720)),
                rate: Some(Rate::FPS_23_976),
            }
        ]
    );
    // One model's picture can be the next one's first frame.
    let chained = Recipe::parse(
        "kerf-recipe 1\ngenerate model=still prompt=\"a boat\" out={first.png}\ngenerate model=dawn first={first.png}\n",
    )
    .unwrap();
    assert_eq!(chained.steps.len(), 2);
}

#[test]
fn nothing_in_a_recipe_is_expanded() {
    // A shell's words are words, and braces around anything but a file's name stand for themselves.
    let plain = Recipe::parse("kerf-recipe 1\nrun echo $HOME;ls {\"a\":1} --out={out}").unwrap();
    let Step::Run { arguments, .. } = &plain.steps[0] else {
        panic!("a run step");
    };
    assert_eq!(arguments, &["$HOME;ls", "{a:1}", "--out={out}"]);
    let place = |name: &str| PathBuf::from(format!("/takes/{name}"));
    assert_eq!(placed("--out={out}", &place), "--out=/takes/out");
    assert_eq!(placed("{a:1} {b.mp4}{", &place), "{a:1} /takes/b.mp4{");
}

#[test]
fn what_is_wrong_with_a_recipe_is_said() {
    let wrong = |text: &str| Recipe::parse(text).unwrap_err().to_string();
    assert!(wrong("run vork {out}").contains("starts with `kerf-recipe 1`"));
    assert!(wrong("kerf-recipe 1\n# nothing\n").contains("has no step"));
    assert!(wrong("kerf-recipe 1\nneeds a.json\n").contains("has no step"));
    assert!(wrong("kerf-recipe 1\nvork {out}")
        .contains("line 2 of the recipe: `vork` is not a recipe's word"));
    assert!(wrong("kerf-recipe 1\nrun vork \"open {out}").contains("quote is not closed"));
    assert!(wrong("kerf-recipe 1\nrun vork render").contains("names no `{out}`"));
    assert!(wrong("kerf-recipe 1\nrun vork {guide.mp4}").contains("names no `{out}`"));
    assert!(wrong("kerf-recipe 1\nrun a {out}\nrun b {out}")
        .contains("line 3 of the recipe: the step before it writes `{out}`"));
    assert!(wrong("kerf-recipe 1\nrender s2 guide.mp4").contains("names the file it makes"));
    assert!(wrong("kerf-recipe 1\nrender s2").contains("`render` is"));
    assert!(wrong("kerf-recipe 1\nrender s2 {out} size=big").contains("not a size such as 832x480"));
    assert!(wrong("kerf-recipe 1\nrender s2 {out} size=833x480")
        .contains("line 2 of the recipe: `size=833x480` has an odd side, which the file cannot have; `size=834x480` has none"));
    assert!(
        wrong("kerf-recipe 1\nrender s2 {out} rate=fast").contains("not a rate in frames a second")
    );
    assert!(
        wrong("kerf-recipe 1\nrender s2 {out} rate=0").contains("not a rate in frames a second")
    );
    assert!(wrong("kerf-recipe 1\nrender s2 {out} fps=16")
        .contains("`fps=16` is not something `render` takes; after the file it makes come `size=WxH` and `rate=<fps>`"));
    assert!(
        wrong("kerf-recipe 1\nrender s2 {out} seconds=4").contains("not something `render` takes")
    );
    assert!(wrong("kerf-recipe 1\ngenerate prompt=\"a boat\"").contains("names its model"));
    assert!(
        wrong("kerf-recipe 1\ngenerate model=dawn guide={guide.mp4}")
            .contains("no step before this one makes `{guide.mp4}`")
    );
    assert!(
        wrong("kerf-recipe 1\ngenerate model=dawn boat").contains("`boat` is not `<name>=<value>`")
    );
    assert!(wrong("kerf-recipe 1\ngenerate model=dawn seconds=soon")
        .contains("not a length in seconds"));
    assert!(wrong("kerf-recipe 1\ngenerate model=dawn size=big").contains("not a size"));
    assert!(wrong("kerf-recipe 1\ngenerate model=dawn seed=-1").contains("not a whole number"));
    assert!(wrong("kerf-recipe 1\ngenerate model=dawn out=first.png").contains("in braces"));
}

#[test]
fn a_comment_changed_is_not_the_recipe_changed() {
    assert_eq!(
        Recipe::essence("kerf-recipe 1\n\n# The opening shot.\n  run a {out}  \n"),
        Recipe::essence("kerf-recipe 1\n# Another word for it.\nrun a {out}\n")
    );
    assert_ne!(
        Recipe::essence("kerf-recipe 1\nrun a {out}\n"),
        Recipe::essence("kerf-recipe 1\nrun b {out}\n")
    );
}

#[test]
fn a_path_stays_inside_the_project() {
    let inside = |folder: &str, path: &str| {
        within(Path::new(folder), path, "file").map(|path| stored(&path))
    };
    assert_eq!(inside("shots", "open.json").unwrap(), "shots/open.json");
    assert_eq!(
        inside("shots", "../refs/boat.png").unwrap(),
        "refs/boat.png"
    );
    assert_eq!(inside("", "./a/./b.png").unwrap(), "a/b.png");
    assert!(inside("shots", "../../elsewhere.png")
        .unwrap_err()
        .to_string()
        .contains("not inside the project"));
    assert!(inside("shots", "/etc/passwd").is_err());
}

#[test]
fn the_models_are_the_persons_own_file() {
    let folder = tempfile::tempdir().unwrap();
    // No file is no models, and asking for one says where one is set up.
    let none = Models::read(&folder.path().join("models.json")).unwrap();
    assert_eq!(none.names().count(), 0);
    let missing = none.command("dawn").unwrap_err().to_string();
    assert!(missing.contains("no model `dawn` is set up"), "{missing}");
    assert!(missing.contains("models.json names none"), "{missing}");
    let some = models(
        folder.path(),
        r#"{"dawn": {"run": ["dawn-adapter", "--request", "{request}"]}, "still": {"run": ["still", "{request}"]}}"#,
    );
    assert_eq!(some.names().collect::<Vec<_>>(), ["dawn", "still"]);
    assert_eq!(
        some.command("dawn").unwrap(),
        ["dawn-adapter", "--request", "{request}"]
    );
    assert!(some
        .command("dusk")
        .unwrap_err()
        .to_string()
        .contains("names dawn, still"));
    let wrong = |text: &str| {
        let file = folder.path().join("wrong.json");
        std::fs::write(&file, text).unwrap();
        Models::read(&file).unwrap_err().to_string()
    };
    assert!(wrong("not json").contains("wrong.json"));
    assert!(wrong(r#"{"dawn": {}}"#).contains("model `dawn` has no `run`"));
    assert!(wrong(r#"{"dawn": {"run": []}}"#).contains("model `dawn` has no `run`"));
    assert!(wrong(r#"{"dawn": {"run": ["dawn-adapter"]}}"#).contains("names no `{request}`"));
}

#[test]
fn a_request_says_where_every_file_is() {
    let made = ["guide.mp4".to_owned()];
    let words = [
        "model=dawn",
        "prompt=a boat",
        "guide={guide.mp4}",
        "first=../refs/boat.png",
        "seconds=4",
        "size=1280x720",
        "seed=7",
        "strength=0.6",
    ]
    .map(str::to_owned);
    let ask = Ask::parse(&words, &made).unwrap();
    let request = ask
        .request(
            &|file| {
                within(Path::new("shots"), file, "file")
                    .map(|inside| Path::new("/project").join(inside))
            },
            &|name| Path::new("/project/shots/open.takes/001.work").join(name),
        )
        .unwrap();
    assert_eq!(
        request,
        serde_json::json!({
            "kerf-request": 1,
            "model": "dawn",
            "prompt": "a boat",
            "guide": "/project/shots/open.takes/001.work/guide.mp4",
            "first": "/project/refs/boat.png",
            "last": null,
            "references": [],
            "seconds": 4.0,
            "size": [1280, 720],
            "seed": 7,
            "more": {"strength": "0.6"},
            "out": "/project/shots/open.takes/001.work/out",
        })
    );
    assert_eq!(
        ask.shown(),
        "prompt \"a boat\", guide {guide.mp4}, first ../refs/boat.png, 4 s, 1280x720, seed 7, strength 0.6"
    );
}

#[test]
fn a_plan_is_only_of_what_can_run_inside_the_project() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let write = |name: &str, steps: &str| {
        std::fs::write(directory.join(name), format!("kerf-recipe 1\n{steps}\n")).unwrap();
    };
    write(
        "shots/open.kerfrecipe",
        "run vork render open.json --out {out}",
    );
    write(
        "shots/far.kerfrecipe",
        "needs ../../far.json\nrun vork {out}",
    );
    write("shots/need.kerfrecipe", "needs open.json\nrun vork {out}");
    write("shots/seq.kerfrecipe", "render s9 {out}");
    write(
        "shots/guide.kerfrecipe",
        "render s1 {guide.mp4} size=832x480 rate=16\nrun follow {guide.mp4} {out}",
    );
    write(
        "shots/ask.kerfrecipe",
        "generate model=dawn prompt=\"a boat\"",
    );
    let project = project(
        "media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/open.kerfrecipe\"\n\
         media m2 path=\"a.mov\" rate=24 dur=1s video=160x120\n\
         media m3 path=\"shots/b.mp4\" rate=24 dur=1s video=160x120 recipe=\"../elsewhere/run.kerfrecipe\"\n\
         media m5 path=\"shots/d.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/gone.kerfrecipe\"\n\
         media m6 path=\"shots/e.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/far.kerfrecipe\"\n\
         media m7 path=\"shots/f.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/need.kerfrecipe\"\n\
         media m8 path=\"shots/g.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/seq.kerfrecipe\"\n\
         media m9 path=\"shots/h.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/ask.kerfrecipe\"\n\
         media m10 path=\"shots/i.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/guide.kerfrecipe\"\n\
         sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n",
    );
    let none = Models::default();
    let made = plan(&project, directory, "m1", &none).unwrap();
    assert_eq!(made.recipe, directory.join("shots/open.kerfrecipe"));
    assert_eq!(made.take, 1);
    assert_eq!(made.file, directory.join("shots/open.takes/001.mp4"));
    assert_eq!(made.stored, "shots/open.takes/001.mp4");
    assert_eq!(
        made.command,
        format!(
            "vork render open.json --out {}",
            directory.join("shots/open.takes/.making-001.mp4").display()
        )
    );
    let wrong = |id: &str| {
        plan(&project, directory, id, &none)
            .unwrap_err()
            .to_string()
    };
    assert!(wrong("m0").contains("no media `m0`"));
    assert!(wrong("m2").contains("names no recipe"));
    assert!(wrong("m3").contains("recipe `../elsewhere/run.kerfrecipe` is not inside"));
    assert!(wrong("m5").contains("cannot read the recipe"));
    assert!(wrong("m6").contains("`../../far.json` is not inside the project"));
    assert!(wrong("m7").contains("needs `shots/open.json`, which is not there"));
    assert!(wrong("m8").contains("renders sequence `s9`, which this project has not"));
    assert!(wrong("m9").contains("no model `dawn` is set up"));
    // A guide's step says the size and the rate it is drawn at.
    let guided = plan(&project, directory, "m10", &none).unwrap();
    assert_eq!(
        guided.steps[0],
        format!(
            "Kerf renders sequence s1 at 832x480, 16 frames a second to {}",
            directory
                .join("shots/guide.takes/001.work/guide.mp4")
                .display()
        )
    );
    // With the model set up, the plan says what its program is handed.
    let some = models(
        directory,
        r#"{"dawn": {"run": ["dawn-adapter", "{request}"]}}"#,
    );
    let asked = plan(&project, directory, "m9", &some).unwrap();
    assert_eq!(
        asked.steps,
        [format!(
            "dawn-adapter {} asks model `dawn` for prompt \"a boat\"",
            directory
                .join("shots/ask.takes/001.work/request.json")
                .display()
        )]
    );
}

#[cfg(unix)]
#[test]
fn each_making_is_a_take_and_a_failed_one_leaves_none() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    std::fs::write(directory.join("shots/scene.txt"), "first").unwrap();
    let recipe = |steps: &str| {
        std::fs::write(
            directory.join("shots/open.kerfrecipe"),
            format!("kerf-recipe 1\n{steps}\n"),
        )
        .unwrap();
    };
    let none = Models::default();
    let token = CancellationToken::new();
    let run = |project: &Project| {
        let plan = plan(project, directory, "m1", &none)?;
        make(project, directory, &plan, &none, &token, |_| {})
    };
    // It runs in the recipe's folder, so `scene.txt` is found beside it, and a later step reads
    // what an earlier one made.
    recipe("needs scene.txt\nrun cp scene.txt {guide.txt}\nrun cp {guide.txt} {out}");
    let project = shot("shots/open.txt");
    let first = run(&project).unwrap();
    assert_eq!(
        first,
        Made {
            take: 1,
            stored: "shots/open.takes/001.txt".to_owned()
        }
    );
    let read = |name: &str| std::fs::read_to_string(directory.join(name)).unwrap();
    assert_eq!(read("shots/open.takes/001.txt"), "first");
    assert_eq!(read("shots/open.takes/001.work/guide.txt"), "first");
    assert!(!directory.join("shots/open.takes/.making-001.txt").exists());

    // The next making leaves the first as it was.
    std::fs::write(directory.join("shots/scene.txt"), "second").unwrap();
    let project = shot(&first.stored);
    let second = run(&project).unwrap();
    assert_eq!(second.stored, "shots/open.takes/002.txt");
    assert_eq!(read("shots/open.takes/001.txt"), "first");
    assert_eq!(read("shots/open.takes/002.txt"), "second");

    let all = takes(&project, directory, project.media("m1").unwrap()).unwrap();
    assert_eq!(
        all.iter().map(|take| take.number).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        all.iter().map(|take| take.chosen).collect::<Vec<_>>(),
        [true, false]
    );
    assert!(all.iter().all(|take| take.made.is_some()));
    // The first was made from a scene that has changed since; the second from it as it is.
    assert_eq!(all[0].changed, ["shots/scene.txt"]);
    assert!(all[1].changed.is_empty());
    assert_eq!(
        take(&project, directory, project.media("m1").unwrap(), 2).unwrap(),
        all[1]
    );
    let absent = take(&project, directory, project.media("m1").unwrap(), 5)
        .unwrap_err()
        .to_string();
    assert!(absent.contains("no take 5; it has 1, 2"), "{absent}");

    // A making that fails, writes nothing, or cannot start leaves no take and nothing of one.
    for (steps, why) in [
        ("run cp missing.txt {out}", "cp failed"),
        ("run true {out}", "wrote nothing"),
        ("run no-such-program-of-kerf {out}", "could not start"),
        (
            "run cp scene.txt {guide.txt}\nrun false {out}",
            "false failed",
        ),
    ] {
        recipe(steps);
        let failed = run(&project).unwrap_err().to_string();
        assert!(failed.contains(why), "{failed}");
    }
    let left: Vec<String> = std::fs::read_dir(directory.join("shots/open.takes"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with("001") && !name.starts_with("002"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[cfg(unix)]
#[test]
fn a_model_is_handed_its_request_and_writes_the_take() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::create_dir_all(directory.join("refs")).unwrap();
    std::fs::write(directory.join("refs/boat.png"), "a boat").unwrap();
    std::fs::write(directory.join("shots/scene.txt"), "a blocking").unwrap();
    std::fs::write(
        directory.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nrun cp scene.txt {guide.txt}\n\
         generate model=dawn prompt=\"a boat leaves\" guide={guide.txt} first=../refs/boat.png seed=7\n",
    )
    .unwrap();
    // A model that answers with what it was asked: it copies its request to where the request
    // says the file goes.
    let some = models(
        directory,
        r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"]}}"#,
    );
    let project = shot("shots/open.json");
    let token = CancellationToken::new();
    let plan = plan(&project, directory, "m1", &some).unwrap();
    let made = make(&project, directory, &plan, &some, &token, |_| {}).unwrap();
    assert_eq!(made.stored, "shots/open.takes/001.json");
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join(&made.stored)).unwrap()).unwrap();
    assert_eq!(request["model"], "dawn");
    assert_eq!(request["prompt"], "a boat leaves");
    assert_eq!(request["seed"], 7);
    let path = |name: &str| directory.join(name).to_string_lossy().into_owned();
    assert_eq!(
        request["guide"],
        path("shots/open.takes/001.work/guide.txt")
    );
    assert_eq!(request["first"], path("refs/boat.png"));
    assert_eq!(request["out"], path("shots/open.takes/.making-001.json"));
    // A model that is not set up is said before anything runs.
    let none = Models::default();
    let unset = make(&project, directory, &plan, &none, &token, |_| {})
        .unwrap_err()
        .to_string();
    assert!(unset.contains("no model `dawn` is set up"), "{unset}");
    assert!(!directory.join("shots/open.takes/002.work").exists());
}

#[cfg(unix)]
#[test]
fn a_media_wants_making_when_what_its_take_was_made_from_changes() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::create_dir_all(directory.join("refs")).unwrap();
    std::fs::write(directory.join("shots/scene.txt"), "a blocking").unwrap();
    std::fs::write(directory.join("refs/boat.png"), "a boat").unwrap();
    let recipe =
        |text: &str| std::fs::write(directory.join("shots/open.kerfrecipe"), text).unwrap();
    let wants = |project: &Project| unmade(project, directory, project.media("m1").unwrap());
    let unset = shot("shots/open.txt");
    assert_eq!(wants(&unset), None, "no recipe to read, nothing to make it");
    recipe("kerf-recipe 1\nrun cp scene.txt\n");
    assert!(
        matches!(wants(&unset), Some(Unmade::Unreadable(why)) if why.contains("names no `{out}`"))
    );
    recipe("kerf-recipe 1\n# The opening shot.\nneeds scene.txt ../refs/boat.png\nrun cp scene.txt {out}\n");
    assert_eq!(wants(&unset), Some(Unmade::Missing));

    let none = Models::default();
    let token = CancellationToken::new();
    let planned = plan(&unset, directory, "m1", &none).unwrap();
    let made = make(&unset, directory, &planned, &none, &token, |_| {}).unwrap();
    let project = shot(&made.stored);
    assert_eq!(wants(&project), None);
    // A comment is not the recipe, and a file written again with what it held is not changed.
    recipe("kerf-recipe 1\n# Another word for it.\nneeds scene.txt ../refs/boat.png\nrun cp scene.txt {out}\n");
    std::fs::write(directory.join("shots/scene.txt"), "a blocking").unwrap();
    assert_eq!(wants(&project), None);
    // What it reads, changed, is named.
    std::fs::write(directory.join("refs/boat.png"), "another boat").unwrap();
    assert_eq!(
        wants(&project),
        Some(Unmade::Stale(vec!["refs/boat.png".to_owned()]))
    );
    std::fs::write(directory.join("shots/scene.txt"), "another blocking").unwrap();
    recipe("kerf-recipe 1\nneeds scene.txt ../refs/boat.png\nrun cp scene.txt {out} \n# and why\n");
    let stale = wants(&project).unwrap();
    assert_eq!(
        stale,
        Unmade::Stale(vec![
            "refs/boat.png".to_owned(),
            "shots/scene.txt".to_owned()
        ])
    );
    assert_eq!(
        stale.reason(),
        "refs/boat.png and shots/scene.txt changed after its file was made"
    );
    recipe("kerf-recipe 1\nneeds scene.txt ../refs/boat.png\nrun cp -f scene.txt {out}\n");
    assert_eq!(
        wants(&project).unwrap().reason(),
        "refs/boat.png, shots/scene.txt and the recipe changed after its file was made"
    );
    // Made again, it is made from things as they stand.
    let planned = plan(&project, directory, "m1", &none).unwrap();
    let again = make(&project, directory, &planned, &none, &token, |_| {}).unwrap();
    assert_eq!(wants(&shot(&again.stored)), None);
    // The take before is still the one that was made from what has changed.
    assert!(wants(&project).is_some());
}

#[test]
fn a_file_no_take_made_is_held_to_its_recipe_by_their_times() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let (recipe, file) = (
        directory.join("shots/open.kerfrecipe"),
        directory.join("shots/open.mp4"),
    );
    std::fs::write(&recipe, "kerf-recipe 1\nrun a {out}\n").unwrap();
    std::fs::write(&file, "frames").unwrap();
    let project = shot("shots/open.mp4");
    let wants = || unmade(&project, directory, project.media("m1").unwrap());
    let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let set = |path: &Path, time| {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap()
    };
    set(&recipe, earlier);
    assert_eq!(wants(), None);
    set(&file, earlier - std::time::Duration::from_secs(3600));
    assert_eq!(wants(), Some(Unmade::Stale(vec!["the recipe".to_owned()])));
}

#[test]
fn a_rendered_sequence_is_what_it_draws_and_nothing_else_of_the_project() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    let recipe =
        Recipe::parse("kerf-recipe 1\nrender s2 {guide.mp4}\nrun a {guide.mp4} {out}\n").unwrap();
    let from = |lines: &str| {
        takes::from(
            &project(lines),
            directory,
            Path::new("shots"),
            "kerf-recipe 1\n",
            &recipe,
        )
        .unwrap()
    };
    let document = |cut: &str, guide: &str, nested: &str| {
        format!(
            "media a path=\"a.mp4\" rate=24 dur=10s video=160x120\n\
             media b path=\"b.mp4\" rate=24 dur=10s video=160x120\n\
             sequence s1 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=a src=0f at=0f dur={cut}\n\
             sequence s2 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip g1 media=b src=0f at=0f dur={guide}\n\
             \x20   clip g2 seq=s3 src=0f at=5s dur=1s\n\
             sequence s3 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip n1 media=b src={nested} at=0f dur=2s\n"
        )
    };
    let as_it_is = from(&document("3s", "2s", "0f"));
    assert_eq!(
        as_it_is.keys().collect::<Vec<_>>(),
        ["recipe", "sequence s2"]
    );
    // The cut it is used in is trimmed: the guide is what it was.
    assert_eq!(from(&document("1s", "2s", "0f")), as_it_is);
    // The guide itself, or a sequence it nests, is not.
    assert_ne!(from(&document("3s", "1s", "0f")), as_it_is);
    assert_ne!(from(&document("3s", "2s", "1s")), as_it_is);
    // Nor is a file it plays, grown.
    std::fs::write(directory.join("b.mp4"), "frames").unwrap();
    assert_ne!(from(&document("3s", "2s", "0f")), as_it_is);
}
