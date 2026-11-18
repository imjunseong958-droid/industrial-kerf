use super::model::{Ask, Models, Source};
use super::signal::Signal;
use super::*;
use kerf_model::{Project, Rate};

/// A project of `lines` after its header, read as a document is.
fn project(lines: &str) -> Project {
    kerf_lang::parse_document(
        &format!("kerf 1\nproject name=\"shots\"\n{lines}"),
        kerf_model::Catalog::builtin(),
    )
    .expect("a project")
    .project
}

/// `steps`, a step a line, as the document holds them under a media's line.
fn under(steps: &str) -> String {
    steps
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| format!("  {line}\n"))
        .collect()
}

/// A project of `lines`, whose first line is a media's, with `steps` under that media.
fn with_steps(lines: &str, steps: &str) -> Project {
    let (media, rest) = lines.split_once('\n').unwrap_or((lines, ""));
    project(&format!("{media}\n{}{rest}", under(steps)))
}

/// A project whose media `m1`, its file at `path`, is made by `steps`, a step a line.
fn shot(path: &str, steps: &str) -> Project {
    project(&format!(
        "media m1 path=\"{path}\" rate=24 dur=1s video=160x120\n{}",
        under(steps)
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
        "kerf-recipe 1\n\
         render s1 {guide.mp4} size=832x480\n\
         render s2 {block.mp4}\n\
         generate model=dawn prompt=\"a boat leaves, at dawn\" guide={guide.mp4} first=refs/boat.png ref={block.mp4} ref=refs/sea.png seconds=4 size=1280x720 seed=7 strength=0.6\n",
    )
    .unwrap();
    assert_eq!(recipe.steps.len(), 3);
    assert_eq!(
        recipe.steps[1],
        Step::Render {
            sequence: "s2".to_owned(),
            to: "block.mp4".to_owned(),
            size: None,
            rate: None,
            pass: Pass::Picture,
            matte: None,
            from: None,
            seconds: None,
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
    assert_eq!(recipe.sequences().collect::<Vec<_>>(), ["s1", "s2"]);
    // What it reads: what a model is handed that no step made.
    assert_eq!(recipe.reads(), ["refs/boat.png", "refs/sea.png"]);
    assert_eq!(recipe.makes(), ["block.mp4", "guide.mp4"]);
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
                pass: Pass::Picture,
                matte: None,
                from: None,
                seconds: None,
            },
            Step::Render {
                sequence: "s3".to_owned(),
                to: "out".to_owned(),
                size: Some((1280, 720)),
                rate: Some(Rate::FPS_23_976),
                pass: Pass::Picture,
                matte: None,
                from: None,
                seconds: None,
            }
        ]
    );
    // A matte is one clip of the sequence in white on black, and is asked for in place of a
    // pass.
    let matte = Recipe::parse("kerf-recipe 1\nrender s2 {out} matte=crate size=832x480\n").unwrap();
    assert!(matches!(
        &matte.steps[0],
        Step::Render { matte: Some(clip), pass: Pass::Picture, .. } if clip == "crate"
    ));
    assert!(Recipe::parse("kerf-recipe 1\nrender s2 {out} matte=crate pass=ids\n")
        .unwrap_err()
        .to_string()
        .contains("give one of the two"));
    // A pass of the same sequence is another file of the recipe's, for a model that reads it.
    let passes = Recipe::parse(
        "kerf-recipe 1\nrender s2 {guide.mp4} size=832x480 rate=16\nrender s2 {ids.mp4} pass=ids size=832x480 rate=16\nrender s2 {out} pass=depth\n",
    )
    .unwrap();
    let pass_of = |step: &Step| match step {
        Step::Render { pass, .. } => *pass,
        _ => panic!("a render step"),
    };
    assert_eq!(
        passes.steps.iter().map(pass_of).collect::<Vec<_>>(),
        [Pass::Picture, Pass::Ids, Pass::Depth]
    );
    // One model's picture can be the next one's first frame.
    let chained = Recipe::parse(
        "kerf-recipe 1\ngenerate model=still prompt=\"a boat\" out={first.png}\ngenerate model=dawn first={first.png}\n",
    )
    .unwrap();
    assert_eq!(chained.steps.len(), 2);
}

#[test]
fn nothing_in_a_step_is_expanded() {
    // A shell's words are words, and a prompt is handed over as it is written.
    let plain =
        Recipe::parse("kerf-recipe 1\ngenerate model=dawn prompt=\"$HOME;ls {a:1} `id`\"\n").unwrap();
    let Step::Generate(ask) = &plain.steps[0] else {
        panic!("a generate step");
    };
    assert_eq!(ask.prompt, "$HOME;ls {a:1} `id`");
}

#[test]
fn what_is_wrong_with_a_recipe_is_said() {
    let wrong = |text: &str| Recipe::parse(text).unwrap_err().to_string();
    assert!(wrong("render s1 {out}").contains("start with the line `kerf-recipe 1`"));
    assert!(wrong("kerf-recipe 1\n").contains("has no step"));
    assert!(wrong("kerf-recipe 1\nvork {out}").contains("line 2: `vork` is not a step"));
    assert!(wrong("kerf-recipe 1\ngenerate model=dawn prompt=\"open").contains("quote is not closed"));
    assert!(wrong("kerf-recipe 1\nrender s1 {guide.mp4}").contains("names no `{out}`"));
    assert!(wrong("kerf-recipe 1\nrender s1 {out}\nrender s2 {out}")
        .contains("step 1 writes `{out}`, the media's file, so it is the last: step 2 goes before it"));
    // A program to run, and a file said to be read, are a recipe file's alone.
    assert!(wrong("kerf-recipe 1\nrun vork {out}").contains("`run vork` runs a program, and a document names none"));
    assert!(wrong("kerf-recipe 1\nneeds a.json\nrender s1 {out}").contains("`needs a.json` is not a step"));
    assert!(wrong("kerf-recipe 1\nrender s2 guide.mp4").contains("names the file it makes"));
    assert!(wrong("kerf-recipe 1\nrender s2").contains("`render` is"));
    assert!(wrong("kerf-recipe 1\nrender s2 {out} size=big").contains("not a size such as 832x480"));
    assert!(wrong("kerf-recipe 1\nrender s2 {out} size=833x480")
        .contains("line 2: `size=833x480` has an odd side, which the file cannot have; `size=834x480` has none"));
    assert!(
        wrong("kerf-recipe 1\nrender s2 {out} rate=fast").contains("not a rate in frames a second")
    );
    assert!(
        wrong("kerf-recipe 1\nrender s2 {out} rate=0").contains("not a rate in frames a second")
    );
    assert!(wrong("kerf-recipe 1\nrender s2 {out} fps=16")
        .contains("`fps=16` is not something `render` takes; after the file it makes come `size=WxH`, `rate=<fps>`, `pass=ids|depth|relative|normals`, `matte=<clip>`, and the part of the sequence, `from=<seconds>` and `seconds=<seconds>`"));
    assert!(wrong("kerf-recipe 1\nrender s2 {out} pass=albedo")
        .contains("`pass=albedo` is not a pass Kerf draws: `ids`, which clip each pixel is of, `depth`, how near it is, `relative`"));
    assert!(
        wrong("kerf-recipe 1\nrender s2 {out} frames=4").contains("not something `render` takes")
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

/// What a take a recipe's file made is held to is the recipe's text with its comments and
/// spare space taken out: a record of one is read so still.
#[test]
fn the_essence_of_a_recipes_text_is_its_lines_and_not_how_they_were_typed() {
    assert_eq!(
        Recipe::essence("kerf-recipe 1\n\n# The opening shot.\n  render s1 {out}  \n"),
        Recipe::essence("kerf-recipe 1\n# Another word for it.\nrender s1 {out}\n")
    );
    assert_ne!(
        Recipe::essence("kerf-recipe 1\nrender s1 {out}\n"),
        Recipe::essence("kerf-recipe 1\nrender s2 {out}\n")
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
    assert!(wrong(r#"{"dawn": {"run": ["dawn-adapter"]}}"#).contains("names neither `{request}` nor `{out}`"));
}

#[test]
fn a_models_entry_can_say_what_the_model_takes() {
    let folder = tempfile::tempdir().unwrap();
    let some = models(
        folder.path(),
        r#"{
            "shot": {
                "run": ["kerf-comfy", "--workflow", "shot", "{request}"],
                "takes": {
                    "about": "follows a guide",
                    "size": [832, 480],
                    "rate": 16,
                    "seconds": 5.0625,
                    "reads": ["guide", "ref"],
                    "more": {"steps": 20, "strength": 1.0},
                    "frames": {"most": 81, "step": 4}
                }
            },
            "words": {"run": ["words", "{request}"], "takes": {"reads": []}},
            "dawn": {"run": ["dawn-adapter", "{request}"]}
        }"#,
    );
    let shot = some.takes("shot").unwrap();
    assert_eq!(shot.about.as_deref(), Some("follows a guide"));
    assert_eq!(
        (shot.size, shot.rate, shot.seconds),
        (Some((832, 480)), Some(16.0), Some(5.0625))
    );
    assert_eq!(
        shot.reads.as_deref(),
        Some(&["guide".to_owned(), "ref".to_owned()][..])
    );
    assert_eq!(shot.follows_a_guide(), Some(true));
    assert_eq!(shot.more["steps"], 20);
    // What Kerf has no name for is kept as it is written.
    assert_eq!(shot.other["frames"]["most"], 81);
    // A model that reads no file takes a prompt alone; one that does not say is not known.
    assert_eq!(some.takes("words").unwrap().follows_a_guide(), Some(false));
    assert_eq!(some.takes("dawn"), None);
    assert_eq!(some.takes("dusk"), None);
    // Saying what it takes changes nothing of how it is run.
    assert_eq!(
        some.command("shot").unwrap(),
        ["kerf-comfy", "--workflow", "shot", "{request}"]
    );
    let wrong = |takes: &str| {
        let file = folder.path().join("wrong.json");
        std::fs::write(
            &file,
            format!(r#"{{"shot": {{"run": ["a", "{{request}}"], "takes": {takes}}}}}"#),
        )
        .unwrap();
        Models::read(&file).unwrap_err().to_string()
    };
    assert!(wrong("16").contains("model `shot`: `takes` is not an object"));
    assert!(
        wrong(r#"{"size": "832x480"}"#).contains("`size` in `takes` is not a width and a height")
    );
    assert!(wrong(r#"{"size": [832, 0]}"#).contains("`size` in `takes`"));
    assert!(wrong(r#"{"rate": 0}"#).contains("`rate` in `takes` is not a number above 0"));
    assert!(
        wrong(r#"{"seconds": "five"}"#).contains("`seconds` in `takes` is not a number above 0")
    );
    assert!(wrong(r#"{"reads": ["guide", "depth"]}"#).contains(
        "not a list of what a `generate` line hands a model: guide, first, last, ref, signal"
    ));
    assert!(wrong(r#"{"about": 3}"#).contains("`about` in `takes` is not text"));
    assert!(wrong(r#"{"more": ["steps"]}"#).contains("`more` in `takes` is not an object"));
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
    let request = model::request(
            &ask,
            &|file| {
                within(Path::new("shots"), file, "file")
                    .map(|inside| Path::new("/project").join(inside))
            },
            &|name| Path::new("/project/shots/open.takes/001.work").join(name),
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(
        request,
        serde_json::json!({
            "kerf-request": 1,
            "model": "dawn",
            "prompt": "a boat",
            "guide": "/project/shots/open.takes/001.work/guide.mp4",
            "guides": {},
            "first": "/project/refs/boat.png",
            "last": null,
            "references": [],
            "seconds": 4.0,
            "size": [1280, 720],
            "seed": 7,
            "signals": [],
            "moves": [],
            "workflow": null,
            "more": {"strength": "0.6"},
            "out": "/project/shots/open.takes/001.work/out",
        })
    );
    assert_eq!(
        ask.shown(),
        "prompt \"a boat\", guide {guide.mp4}, first ../refs/boat.png, 4 s, 1280x720, seed 7, strength 0.6"
    );
}

/// A guide of twelve frames with a signal on it: `swell`, from its third
/// frame for six, whose value is `value`; media `m1` is made by `steps`.
fn with_a_signal(value: &str, solid: &str, steps: &str) -> Project {
    project(&format!(
        "media m1 path=\"shots/open.json\" rate=24 dur=1s video=160x120\n{}\
         sequence s2 rate=24 size=160x120 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=12f\n\
         \x20     set color={solid}\n\
         \x20 track V2 video\n\
         \x20   clip swell gen=signal@1 at=2f dur=6f\n\
         \x20     set value={value}\n",
        under(steps)
    ))
}

/// The step that asks a model of the wave `swell` says.
const WAVE: &str = "generate model=dawn prompt=\"a wave\" signal=swell seed=7";

#[test]
fn a_signal_is_what_its_clip_comes_to_on_each_frame() {
    let project = with_a_signal("{0f:0 4f:1}", "#102030", WAVE);
    let swell = Signal::read(&project, "swell").unwrap();
    assert_eq!(swell.name, "swell");
    assert_eq!(swell.rate, 24.0);
    // Nothing before the clip or after it; in it, its keys, the last one holding.
    assert_eq!(
        swell.values,
        [0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(swell.steady(), None);
    // One that never moves is said to hold, and one past 1 stops there.
    let held = Signal::read(&with_a_signal("0.5", "#102030", WAVE), "swell").unwrap();
    assert_eq!(held.steady(), None, "it is 0 outside its clip");
    let curved = Signal::read(
        &with_a_signal("{0f:0:0.3,4,0.7,4 5f:1}", "#102030", WAVE),
        "swell",
    )
    .unwrap();
    assert!(curved
        .values
        .iter()
        .all(|value| (0.0..=1.0).contains(value)));
    assert!(curved.values[3..7].contains(&1.0), "{:?}", curved.values);
    // Turned off, it is not there.
    let mut off = project.clone();
    let at = off.locate("swell").unwrap();
    off.clip_at_mut(at).enabled = false;
    assert_eq!(Signal::read(&off, "swell").unwrap().steady(), Some(0.0));
    // What is not a signal is said.
    let none = Signal::read(&project, "swel").unwrap_err();
    assert_eq!(none, "`signal=swel`: no clip `swel`; did you mean `swell`?");
    let other = Signal::read(&project, "g1").unwrap_err();
    assert!(
        other.contains("names a clip that is not a signal: a signal is a `gen=signal@1` clip"),
        "{other}"
    );
}

#[test]
fn a_line_hands_a_model_its_signals() {
    let project = with_a_signal("{0f:0 4f:1}", "#102030", WAVE);
    let words = ["model=dawn", "prompt=a wave", "signal=swell", "seed=7"].map(str::to_owned);
    let ask = Ask::parse(&words, &[]).unwrap();
    assert_eq!(ask.signals, ["swell"]);
    assert_eq!(ask.shown(), "prompt \"a wave\", seed 7, signal swell");
    let signals = [Signal::read(&project, "swell").unwrap()];
    let request = model::request(
            &ask,
            &|_| unreachable!(),
            &|name| Path::new("/work").join(name),
            &signals,
            &[],
        )
        .unwrap();
    assert_eq!(request["moves"], serde_json::json!([]));
    assert_eq!(
        request["signals"],
        serde_json::json!([{
            "name": "swell",
            "rate": 24.0,
            "values": [0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        }])
    );
    let unnamed = Ask::parse(&["model=dawn", "signal="].map(str::to_owned), &[]).unwrap_err();
    assert!(
        unnamed.contains("`signal=` names a signal clip"),
        "{unnamed}"
    );
}

/// A sky, and a small square that goes from the left of the frame to the right over half a
/// second and is gone for the rest; media `m1` is made by `steps`.
fn with_a_mover(steps: &str) -> Project {
    project(&format!(
        "media m1 path=\"shots/open.json\" rate=24 dur=1s video=160x120\n{}\
         sequence s2 rate=24 size=160x120 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip sky gen=solid@1 at=0f dur=24f\n\
         \x20 track V2 video\n\
         \x20   clip ball gen=solid@1 at=0f dur=12f\n\
         \x20     set color=#ff0000 scale=0.25 x={{0f:-0.25 11f:0.25}}\n\
         \x20 track V3 video\n\
         \x20   clip eye gen=camera@1 at=0f dur=24f\n",
        under(steps)
    ))
}

#[test]
fn a_line_hands_a_model_more_than_one_guide_each_by_its_name() {
    let made = ["picture.mp4".to_owned(), "depth.mp4".to_owned()];
    let words = [
        "model=dawn",
        "prompt=a boat",
        "guide={picture.mp4}",
        "guide_depth={depth.mp4}",
        "guide_ids=../passes/ids.mp4",
    ]
    .map(str::to_owned);
    let ask = Ask::parse(&words, &made).unwrap();
    assert_eq!(
        ask.guides,
        [
            ("depth".to_owned(), Source::Made("depth.mp4".to_owned())),
            ("ids".to_owned(), Source::File("../passes/ids.mp4".to_owned())),
        ]
    );
    assert_eq!(
        ask.shown(),
        "prompt \"a boat\", guide {picture.mp4}, guide_depth {depth.mp4}, guide_ids ../passes/ids.mp4"
    );
    // Each is a file the take is held to, and is in the request under its name.
    assert_eq!(ask.sources().count(), 3);
    let request = model::request(
            &ask,
            &|file| Ok(Path::new("/project/shots").join(file)),
            &|name| Path::new("/work").join(name),
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(request["guide"], "/work/picture.mp4");
    assert_eq!(
        request["guides"],
        serde_json::json!({"depth": "/work/depth.mp4", "ids": "/project/shots/../passes/ids.mp4"})
    );
    // A name is given once, and is letters and digits; anything else is the model's own.
    let twice = ["model=dawn", "guide_depth=a.mp4", "guide_depth=b.mp4"].map(str::to_owned);
    assert!(Ask::parse(&twice, &[]).unwrap_err().contains("given twice"));
    let other = Ask::parse(&["model=dawn", "guide_Scale=2"].map(str::to_owned), &[]).unwrap();
    assert!(other.guides.is_empty() && other.more.contains_key("guide_Scale"));
}

#[test]
fn a_line_hands_a_model_where_a_clip_goes() {
    let project = with_a_mover("");
    let words = ["model=dawn", "prompt=a ball", "moves=ball"].map(str::to_owned);
    let ask = Ask::parse(&words, &[]).unwrap();
    assert_eq!(ask.moves, ["ball"]);
    assert_eq!(ask.shown(), "prompt \"a ball\", moves ball");
    let ball = Moves::read(&project, "ball").unwrap();
    assert_eq!((ball.name.as_str(), ball.rate, ball.places.len()), ("ball", 24.0, 24));
    // A quarter of the frame each way, its middle going from a quarter across to three
    // quarters, and nowhere once its clip is over.
    let [x, y, wide, tall] = ball.places[0].unwrap();
    assert!((x - 0.25).abs() < 0.01 && (y - 0.5).abs() < 0.01, "{:?}", ball.places[0]);
    assert!((wide - 0.25).abs() < 0.01 && (tall - 0.25).abs() < 0.01, "{:?}", ball.places[0]);
    let [x, ..] = ball.places[11].unwrap();
    assert!((x - 0.75).abs() < 0.01, "{:?}", ball.places[11]);
    assert_eq!(ball.places[12], None);
    assert!(!ball.still());
    let request = model::request(
            &ask,
            &|_| unreachable!(),
            &|name| Path::new("/work").join(name),
            &[],
            std::slice::from_ref(&ball),
        )
        .unwrap();
    assert_eq!(request["moves"][0]["name"], "ball");
    assert_eq!(request["moves"][0]["frame"], serde_json::json!([160, 120]));
    assert_eq!(request["moves"][0]["places"][12], serde_json::Value::Null);
    assert_eq!(request["moves"][0]["places"].as_array().unwrap().len(), 24);
    // The sky is on every frame where it was, which says nothing of where anything goes.
    assert!(Moves::read(&project, "sky").unwrap().still());
    // What draws nothing is nowhere, and what is not there is said.
    let nowhere = Moves::read(&project, "eye").unwrap_err();
    assert!(nowhere.contains("draws nothing"), "{nowhere}");
    assert_eq!(
        Moves::sequence_of(&project, "bat").unwrap_err(),
        "`moves=bat`: no clip `bat`"
    );
    let unnamed = Ask::parse(&["model=dawn", "moves="].map(str::to_owned), &[]).unwrap_err();
    assert!(unnamed.contains("`moves=` names a clip"), "{unnamed}");
}

#[test]
fn a_take_is_held_to_the_sequence_a_clip_moves_in() {
    let folder = tempfile::tempdir().unwrap();
    let recipe =
        Recipe::parse("kerf-recipe 1\ngenerate model=dawn prompt=\"a ball\" moves=ball\n").unwrap();
    assert_eq!(recipe.moves(), ["ball"]);
    let from = takes::from(&with_a_mover(""), folder.path(), "kerf-recipe 1\n", &recipe).unwrap();
    assert_eq!(from.keys().collect::<Vec<_>>(), ["sequence s2", "steps"]);
}

#[test]
fn a_plan_reads_a_recipes_signals() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let some = models(
        directory,
        r#"{
            "shot": {"run": ["a", "{request}"], "takes": {"reads": ["guide"]}},
            "cued": {"run": ["b", "{request}"], "takes": {"reads": ["guide", "signal"]}},
            "led": {"run": ["c", "{request}"], "takes": {"reads": ["moves"]}},
            "two": {"run": ["d", "{request}"], "takes": {"reads": ["guide", "guide_depth"]}}
        }"#,
    );
    let planned = |project: &dyn Fn(&str) -> Project, steps: &str| {
        plan(&project(steps), directory, "m1", &some)
    };
    let waves = |steps: &str| with_a_signal("{0f:0 4f:1}", "#102030", steps);
    // A model that reads one is handed it with nothing to note.
    let plan = planned(&waves, "generate model=cued prompt=\"a wave\" signal=swell").unwrap();
    assert!(plan.notes.is_empty(), "{:?}", plan.notes);
    assert!(plan.steps[0].ends_with("signal swell"), "{}", plan.steps[0]);
    // One that does not is said to have no use for it.
    assert_eq!(
        planned(&waves, "generate model=shot prompt=\"a wave\" signal=swell")
            .unwrap()
            .notes,
        ["model `shot` has no use for `signal=`: of what a line hands it, it reads guide"]
    );
    // The same of where a clip goes: read by one model, of no use to another, and stopping
    // the plan where the clip is not there.
    let moving = with_a_mover;
    let plan = planned(&moving, "generate model=led prompt=\"a ball\" moves=ball").unwrap();
    assert!(plan.notes.is_empty(), "{:?}", plan.notes);
    assert!(plan.steps[0].ends_with("moves ball"), "{}", plan.steps[0]);
    assert_eq!(
        planned(&moving, "generate model=shot prompt=\"a ball\" moves=ball")
            .unwrap()
            .notes,
        ["model `shot` has no use for `moves=`: of what a line hands it, it reads guide"]
    );
    let missing = planned(&moving, "generate model=led moves=bat")
        .unwrap_err()
        .to_string();
    assert_eq!(missing, "`moves=bat`: no clip `bat`");
    // A second guide is read by a model that says it reads one of that name.
    std::fs::write(directory.join("shots/d.mp4"), "").unwrap();
    let both = "render s2 {g.mp4}\ngenerate model=two guide={g.mp4} guide_depth=shots/d.mp4";
    assert!(planned(&moving, both).unwrap().notes.is_empty());
    assert_eq!(
        planned(&moving, "render s2 {g.mp4}\ngenerate model=shot guide={g.mp4} guide_depth=shots/d.mp4")
            .unwrap()
            .notes,
        ["model `shot` has no use for `guide_depth=`: of what a line hands it, it reads guide"]
    );
    // A signal that is not there stops the plan, as a sequence that is not does.
    let missing = planned(&waves, "generate model=cued signal=surge")
        .unwrap_err()
        .to_string();
    assert_eq!(missing, "`signal=surge`: no clip `surge`");
    // One that never moves says nothing of when.
    let flat = |steps: &str| {
        project(&format!(
            "media m1 path=\"shots/open.json\" rate=24 dur=1s video=160x120\n{}\
             sequence s2 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip swell gen=signal@1 at=0f dur=12f\n\
             \x20     set value=0.5\n",
            under(steps)
        ))
    };
    assert_eq!(
        planned(&flat, "generate model=cued signal=swell").unwrap().notes,
        ["signal `swell` is 0.5 on every frame, which says nothing of when: keyframe its `value`, as `set swell value={0f:0 24f:1}`"]
    );
}

#[test]
fn a_take_is_held_to_its_signal_and_not_to_the_signals_sequence() {
    let folder = tempfile::tempdir().unwrap();
    let recipe =
        Recipe::parse("kerf-recipe 1\ngenerate model=dawn prompt=\"a wave\" signal=swell\n")
            .unwrap();
    assert_eq!(recipe.signals(), ["swell"]);
    let from = |project: &Project| {
        takes::from(project, folder.path(), "kerf-recipe 1\n", &recipe).unwrap()
    };
    let as_it_is = from(&with_a_signal("{0f:0 4f:1}", "#102030", WAVE));
    assert_eq!(
        as_it_is.keys().collect::<Vec<_>>(),
        ["signal swell", "steps"]
    );
    // Another clip of its sequence recolored: the signal is what it was.
    assert_eq!(from(&with_a_signal("{0f:0 4f:1}", "#ffffff", WAVE)), as_it_is);
    // Its own keys moved: it is not.
    assert_ne!(from(&with_a_signal("{0f:0 3f:1}", "#102030", WAVE)), as_it_is);
}

#[test]
fn a_plan_is_only_of_what_can_run_inside_the_project() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let project = project(
        "media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 render s1 {out}\n\
         media m2 path=\"a.mov\" rate=24 dur=1s video=160x120\n\
         media m3 path=\"shots/b.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/b.kerfrecipe\"\n\
         media m4 path=\"../c.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 render s1 {out}\n\
         media m6 path=\"shots/e.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 generate model=dawn first=../../far.json\n\
         media m7 path=\"shots/f.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 generate model=dawn first=shots/open.json\n\
         media m8 path=\"shots/g.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 render s9 {out}\n\
         media m9 path=\"shots/h.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 generate model=dawn prompt=\"a boat\"\n\
         media m10 path=\"shots/i.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 render s1 {guide.mp4} size=832x480 rate=16\n\
         \x20 render s1 {out}\n\
         sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n",
    );
    let none = Models::default();
    let made = plan(&project, directory, "m1", &none).unwrap();
    assert_eq!(made.take, 1);
    assert_eq!(made.file, directory.join("shots/open.takes/001.mp4"));
    assert_eq!(made.stored, "shots/open.takes/001.mp4");
    assert_eq!(
        made.command,
        format!(
            "Kerf renders sequence s1 to {}",
            directory.join("shots/open.takes/.making-001.mp4").display()
        )
    );
    let wrong = |id: &str| {
        plan(&project, directory, id, &none)
            .unwrap_err()
            .to_string()
    };
    assert!(wrong("m0").contains("no media `m0`"));
    assert!(wrong("m2").contains("has no steps that make it"));
    // A media whose steps are still in a recipe's file is not made from it, and is told
    // how they come into the document.
    assert_eq!(
        wrong("m3"),
        not_carried("m3", "shots/b.kerfrecipe").to_string()
    );
    assert!(wrong("m3").contains("`kerf carry m3`"));
    assert!(wrong("m4").contains("`../c.mp4` is not inside the project"));
    assert!(wrong("m6").contains("`../../far.json` is not inside the project"));
    assert!(wrong("m7").contains("need `shots/open.json`, which is not there"));
    assert!(wrong("m8").contains("render sequence `s9`, which this project has not"));
    assert!(wrong("m9").contains("no model `dawn` is set up"));
    // A guide's step says the size and the rate it is drawn at.
    let guided = plan(&project, directory, "m10", &none).unwrap();
    assert_eq!(
        guided.steps[0],
        format!(
            "Kerf renders sequence s1 at 832x480, 16 frames a second to {}",
            directory
                .join("shots/i.takes/001.work/guide.mp4")
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
                .join("shots/h.takes/001.work/request.json")
                .display()
        )]
    );
}

#[test]
fn a_plan_says_how_a_file_that_is_not_found_was_named() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::create_dir_all(directory.join("refs")).unwrap();
    std::fs::write(directory.join("refs/boat.png"), "a boat").unwrap();
    let lines = "media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120\n\
         sequence s1 rate=24 size=160x120 audio=48000\n\
         \x20 track V1 video\n";
    let some = models(
        directory,
        r#"{"dawn": {"run": ["dawn-adapter", "{request}"]}}"#,
    );
    let planned = |steps: &str| plan(&with_steps(lines, steps), directory, "m1", &some);
    let refused = |steps: &str| planned(steps).unwrap_err().to_string();
    // A file a step makes, named without its braces.
    assert_eq!(
        refused("render s1 {guide.mp4}\ngenerate model=dawn guide=guide.mp4"),
        "the steps need `guide.mp4`, which is not there: `guide.mp4` is a file a step makes, which a later step names in braces, `{guide.mp4}`"
    );
    // A file of the project named another way than from the project's folder.
    assert_eq!(
        refused("generate model=dawn first=./refs/sea.png"),
        "the steps need `refs/sea.png`, which is not there: it names `./refs/sea.png`, and a file is named from the project's folder"
    );
    // Named from a shot's own folder, as a recipe named it, it is not in the project.
    assert!(refused("generate model=dawn first=../refs/boat.png").contains("is not inside the project"));
    // Named from the project's folder, it is found.
    assert!(planned("generate model=dawn first=refs/boat.png").is_ok());
    // A value with spaces and no quotes says how one is written.
    let unquoted = Recipe::parse("kerf-recipe 1\ngenerate model=dawn prompt=a boat leaves")
        .unwrap_err()
        .to_string();
    assert!(
        unquoted.contains("`boat` is not `<name>=<value>`; a value that holds spaces is quoted"),
        "{unquoted}"
    );
}

#[test]
fn a_plan_notes_what_a_model_has_said_it_does_not_take() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::write(directory.join("shots/boat.png"), "a boat").unwrap();
    let lines = "media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120\n\
         sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         sequence s2 rate=24 size=832x480 audio=48000\n\
         \x20 track V1 video\n";
    let some = models(
        directory,
        r#"{
            "shot": {"run": ["a", "{request}"], "takes": {
                "size": [832, 480], "rate": 16, "seconds": 5.0625,
                "reads": ["guide", "ref"], "more": {"steps": 20, "strength": 1.0}}},
            "words": {"run": ["b", "{request}"], "takes": {"reads": []}},
            "dawn": {"run": ["c", "{request}"]}
        }"#,
    );
    let notes = |steps: &str| plan(&with_steps(lines, steps), directory, "m1", &some).unwrap().notes;
    // Steps written for their model have nothing to note.
    assert!(notes(
        "render s2 {guide.mp4} size=832x480 rate=16\n\
         generate model=shot prompt=\"a boat\" guide={guide.mp4} ref=shots/boat.png seconds=4 seed=7 steps=30"
    )
    .is_empty());
    // A guide drawn as its sequence is, for a model that reads it smaller and slower, and
    // from a sequence of another shape.
    assert_eq!(
        notes("render s1 {guide.mp4}\ngenerate model=shot prompt=\"a boat\" guide={guide.mp4}"),
        [
            "the guide `{guide.mp4}` is not drawn as model `shot` reads it, 832x480 at 16 frames a second: `render s1 {guide.mp4} size=832x480 rate=16` draws only what it reads, in less time",
            "sequence `s1` is 1920x1080, another shape than the 832x480 of its guide: the guide fills that size, and the sides of the sequence are cut off",
        ]
    );
    // The rate alone, where the size is the model's.
    assert_eq!(
        notes("render s2 {guide.mp4} size=832x480\ngenerate model=shot guide={guide.mp4}"),
        ["the guide `{guide.mp4}` is not drawn as model `shot` reads it, 832x480 at 16 frames a second: `render s2 {guide.mp4} size=832x480 rate=16` draws only what it reads, in less time"]
    );
    // A shot longer than it makes, a file it does not read, and a name it has not got.
    assert_eq!(
        notes("generate model=shot prompt=\"a boat\" first=shots/boat.png seconds=9 duration=9"),
        [
            "`seconds=9` is more than model `shot` makes at once, 5.0625 s: a longer shot is made in parts, each a guide of its part (`render <sequence> {b.mp4} from=<seconds> seconds=<seconds>`) and a `generate` that goes on from the part before (`first={a.mp4} from=<seconds>`), set end to end by `join {a.mp4} {b.mp4} {out} lap=1`",
            "model `shot` has no use for `first=`: of what a line hands it, it reads guide, ref",
            "model `shot` has nothing named `duration` to set: beside what every model is asked, a line sets its steps, strength",
        ]
    );
    assert_eq!(
        notes("generate model=words prompt=\"a boat\" ref=shots/boat.png"),
        ["model `words` has no use for `ref=`: it takes a prompt alone"]
    );
    // A model that has not said what it takes is asked nothing of.
    assert!(notes(
        "render s1 {guide.mp4}\ngenerate model=dawn guide={guide.mp4} first=shots/boat.png seconds=90 anything=1"
    )
    .is_empty());
}

#[cfg(unix)]
#[test]
fn each_making_is_a_take_and_a_failed_one_leaves_none() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    std::fs::write(directory.join("shots/scene.txt"), "first").unwrap();
    // Programs the person names, each handed where its file goes: one that copies the
    // file it is handed there, and ones that fail, write nothing, or cannot start.
    let some = models(
        directory,
        r#"{"copy": {"run": ["cp", "{ref}", "{out}"]},
            "fail": {"run": ["false", "{out}"]},
            "idle": {"run": ["true", "{out}"]},
            "absent": {"run": ["no-such-program-of-kerf", "{out}"]}}"#,
    );
    let token = CancellationToken::new();
    let run = |project: &Project| {
        let plan = plan(project, directory, "m1", &some)?;
        make(project, directory, &plan, &some, &token, |_| {})
    };
    // A file of the project is named from its folder, and a later step reads what an
    // earlier one made.
    let steps = "generate model=copy ref=shots/scene.txt out={guide.txt}\ngenerate model=copy ref={guide.txt}";
    let project = shot("shots/open.txt", steps);
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
    let project = shot(&first.stored, steps);
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
        ("generate model=fail", "false failed"),
        ("generate model=idle", "wrote nothing"),
        ("generate model=absent", "could not start"),
        (
            "generate model=copy ref=shots/scene.txt out={guide.txt}\ngenerate model=fail",
            "false failed",
        ),
    ] {
        let failed = run(&shot(&first.stored, steps)).unwrap_err().to_string();
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
    // A model that answers with what it was asked: it copies its request to where the request
    // says the file goes. The guide is a copy of the scene a program the person names makes.
    let some = models(
        directory,
        &format!(r#"{{"dawn": {{"run": {ECHO}}}, "copy": {{"run": ["cp", "{{ref}}", "{{out}}"]}}}}"#),
    );
    let project = shot(
        "shots/open.json",
        "generate model=copy ref=shots/scene.txt out={guide.txt}\n\
         generate model=dawn prompt=\"a boat leaves\" guide={guide.txt} first=refs/boat.png seed=7",
    );
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
    assert!(unset.contains("no model `copy` is set up"), "{unset}");
    assert!(!directory.join("shots/open.takes/002.work").exists());
}

#[cfg(unix)]
#[test]
fn a_line_hands_a_model_a_workflow_of_the_project_and_a_take_is_held_to_it() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    let steps = "generate model=dawn prompt=\"a boat leaves\" workflow=shots/open.dawn.comfy.json seed=7";
    // A model that answers with what it was asked. What it has said it reads says nothing
    // of a workflow, and nothing is noted of one: whether its program takes one is its own.
    let some = models(
        directory,
        r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"],
                     "takes": {"reads": ["guide"]}}}"#,
    );
    let project = shot("shots/open.json", steps);
    // It is a file the steps read: not there, they cannot run, and say which.
    let missing = plan(&project, directory, "m1", &some).unwrap_err().to_string();
    assert!(
        missing.contains("the steps need `shots/open.dawn.comfy.json`, which is not there"),
        "{missing}"
    );
    std::fs::write(directory.join("shots/open.dawn.comfy.json"), "{\"steps\": 20}").unwrap();
    let planned = plan(&project, directory, "m1", &some).unwrap();
    assert!(planned.notes.is_empty(), "{:?}", planned.notes);
    assert!(planned.command.contains("workflow shots/open.dawn.comfy.json"), "{}", planned.command);
    let token = CancellationToken::new();
    let made = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join(&made.stored)).unwrap()).unwrap();
    assert_eq!(
        request["workflow"],
        directory
            .join("shots/open.dawn.comfy.json")
            .to_string_lossy()
            .into_owned()
    );
    // The take is held to it: changed, the shot wants making, and says for what.
    let project = shot(&made.stored, steps);
    let wants = |project: &Project| unmade(project, directory, project.media("m1").unwrap());
    assert_eq!(wants(&project), None);
    std::fs::write(directory.join("shots/open.dawn.comfy.json"), "{\"steps\": 30}").unwrap();
    assert_eq!(
        wants(&project),
        Some(Unmade::Stale(vec!["shots/open.dawn.comfy.json".to_owned()]))
    );
    // A line with none asks with none.
    let ask = Ask::parse(&["model=dawn".to_owned()], &[]).unwrap();
    let request =
        model::request(&ask, &|_| unreachable!(), &|name| Path::new("/work").join(name), &[], &[])
            .unwrap();
    assert_eq!(request["workflow"], serde_json::Value::Null);
}

#[cfg(unix)]
#[test]
fn a_making_hands_its_model_the_signal_and_is_held_to_it() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    // A model that answers with what it was asked.
    let some = models(
        directory,
        r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"]}}"#,
    );
    let project = with_a_signal("{0f:0 4f:1}", "#102030", WAVE);
    let token = CancellationToken::new();
    let planned = plan(&project, directory, "m1", &some).unwrap();
    let made = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join(&made.stored)).unwrap()).unwrap();
    assert_eq!(
        request["signals"],
        serde_json::json!([{
            "name": "swell",
            "rate": 24.0,
            "values": [0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        }])
    );
    // The media that is this take wants nothing while its signal is what it was made from.
    let taken = |value: &str, solid: &str| {
        let mut project = with_a_signal(value, solid, WAVE);
        project.media[0].path = made.stored.clone();
        project
    };
    let wants = |project: &Project| unmade(project, directory, project.media("m1").unwrap());
    assert_eq!(wants(&taken("{0f:0 4f:1}", "#102030")), None);
    assert_eq!(wants(&taken("{0f:0 4f:1}", "#ffffff")), None);
    let moved = wants(&taken("{0f:0 2f:1}", "#102030")).unwrap();
    assert_eq!(
        moved.reason(),
        "signal swell changed after its file was made"
    );
}

#[cfg(unix)]
#[test]
fn a_cancelled_making_lets_its_model_take_back_what_it_asked_for() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    // A model that waits on a server, and when asked to end says so and leaves a mark, as a
    // program that takes its job back does.
    let mark = directory.join("taken-back");
    let script = format!(
        "trap 'echo took the job back >&2; : > \"{}\"; exit 1' TERM; echo waiting >&2; while :; do sleep 0.05; done",
        mark.display()
    );
    let some = models(
        directory,
        &serde_json::json!({"slow": {"run": ["sh", "-c", script, "slow", "{request}"]}})
            .to_string(),
    );
    let project = shot("shots/open.mp4", "generate model=slow prompt=\"a boat leaves\"");
    let planned = plan(&project, directory, "m1", &some).unwrap();
    let token = CancellationToken::new();
    let (cancel, said) = (token.clone(), std::sync::Mutex::new(Vec::new()));
    let started = std::time::Instant::now();
    let cancelled = make(&project, directory, &planned, &some, &token, |line| {
        // Cancelled once the model is known to be waiting.
        if line == "waiting" {
            cancel.cancel();
        }
        said.lock().unwrap().push(line.to_owned());
    })
    .unwrap_err();
    assert!(matches!(cancelled, EngineError::Cancelled), "{cancelled}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    // It was asked, not killed: it did what it does on the way out, and was heard.
    assert!(mark.exists());
    assert!(
        said.lock()
            .unwrap()
            .iter()
            .any(|line| line == "took the job back"),
        "{said:?}"
    );
    // No take, and nothing of one, is left.
    assert!(!directory.join("shots/open.takes/001.work").exists());
    assert!(!directory.join("shots/open.takes/.making-001.mp4").exists());
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
    // A program the person names, which copies the first file it is handed.
    let some = models(directory, r#"{"copy": {"run": ["cp", "{ref}", "{out}"]}}"#);
    let wants = |project: &Project| unmade(project, directory, project.media("m1").unwrap());
    assert_eq!(wants(&shot("shots/open.txt", "")), None, "no steps, nothing to make it");
    let short = shot("shots/open.txt", "generate model=copy ref=shots/scene.txt out={guide.txt}");
    assert!(
        matches!(wants(&short), Some(Unmade::Unreadable(why)) if why.contains("names no `{out}`"))
    );
    let steps = "generate model=copy ref=shots/scene.txt ref=refs/boat.png";
    let unset = shot("shots/open.txt", steps);
    assert_eq!(wants(&unset), Some(Unmade::Missing));

    let token = CancellationToken::new();
    let planned = plan(&unset, directory, "m1", &some).unwrap();
    let made = make(&unset, directory, &planned, &some, &token, |_| {}).unwrap();
    let project = shot(&made.stored, steps);
    assert_eq!(wants(&project), None);
    // A file written again with what it held is not changed.
    std::fs::write(directory.join("shots/scene.txt"), "a blocking").unwrap();
    assert_eq!(wants(&project), None);
    // What it reads, changed, is named.
    std::fs::write(directory.join("refs/boat.png"), "another boat").unwrap();
    assert_eq!(
        wants(&project),
        Some(Unmade::Stale(vec!["refs/boat.png".to_owned()]))
    );
    std::fs::write(directory.join("shots/scene.txt"), "another blocking").unwrap();
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
    let steps = "generate model=copy ref=shots/scene.txt ref=refs/boat.png seed=1";
    let project = shot(&made.stored, steps);
    assert_eq!(
        wants(&project).unwrap().reason(),
        "refs/boat.png, shots/scene.txt and its steps changed after its file was made"
    );
    // Made again, it is made from things as they stand.
    let planned = plan(&project, directory, "m1", &some).unwrap();
    let again = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
    assert_eq!(wants(&shot(&again.stored, steps)), None);
    // The take before is still the one that was made from what has changed.
    assert!(wants(&project).is_some());
}

/// A media's file that no take made is taken as it is: nothing says what it was made
/// from, and its time says nothing either.
#[test]
fn a_file_no_take_made_is_taken_as_it_is() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let file = directory.join("shots/open.mp4");
    std::fs::write(&file, "frames").unwrap();
    let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(earlier)
        .unwrap();
    let project = with_steps(
        "media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120\n\
         sequence s1 rate=24 size=160x120 audio=48000\n",
        "render s1 {out}",
    );
    let media = project.media("m1").unwrap();
    assert_eq!(unmade(&project, directory, media), None);
    assert!(takes(&project, directory, media).unwrap().is_empty());
}

#[test]
fn a_rendered_sequence_is_what_it_draws_and_nothing_else_of_the_project() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    let recipe = Recipe::parse("kerf-recipe 1\nrender s2 {out}\n").unwrap();
    let from =
        |lines: &str| takes::from(&project(lines), directory, "kerf-recipe 1\n", &recipe).unwrap();
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
        ["sequence s2", "steps"]
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

/// The takes a recipe's file made are found where its makings put them: every version of a
/// recipe, `open.kerfrecipe` and `open@2.kerfrecipe`, kept its takes in one folder, named
/// for what they shared. A carry finds them there.
#[test]
fn the_takes_of_every_version_of_a_recipe_are_found_in_one_folder() {
    let directory = Path::new("/project");
    let place = |recipe: &str| takes::Place::of(directory, recipe, "shots/open.mp4").unwrap();
    assert_eq!(
        place("shots/open.kerfrecipe").stored(1),
        "shots/open.takes/001.mp4"
    );
    assert_eq!(
        place("shots/open@2.kerfrecipe").stored(1),
        "shots/open.takes/001.mp4"
    );
    assert_eq!(
        place("shots/open@12.kerfrecipe").stored(1),
        "shots/open.takes/001.mp4"
    );
    assert_eq!(
        place("shots/a.b@3.kerfrecipe").stored(2),
        "shots/a.b.takes/002.mp4"
    );
    // A name that only looks like a version's is a name of its own.
    assert_eq!(place("open@1.kerfrecipe").stored(1), "open@1.takes/001.mp4");
    assert_eq!(place("open@v2.kerfrecipe").stored(1), "open@v2.takes/001.mp4");
    assert_eq!(place("@2.kerfrecipe").stored(1), "@2.takes/001.mp4");
    assert!(takes::Place::of(directory, "../open.kerfrecipe", "shots/open.mp4").is_err());
}

#[test]
fn a_cast_file_says_who_is_in_the_shots_and_how_they_look() {
    use super::cast::{Cast, Kind};
    let cast = Cast::parse(
        "kerf-cast 1\n# Who the shots show.\n\
         cast Ada \"an old keeper   in a wool coat\" ref=refs/ada.png ref=refs/ada-side.png ref=refs/ada.png\n\
         cast lamp ref=refs/lamp.png\n\
         style film \"shot on 35mm colour negative\"\n",
    )
    .unwrap();
    let ada = cast.of(Kind::Cast).next().unwrap();
    assert_eq!(ada.0, "Ada");
    assert_eq!(ada.1.words, "an old keeper in a wool coat");
    assert_eq!(ada.1.references, ["refs/ada.png", "refs/ada-side.png"]);
    assert_eq!(cast.of(Kind::Cast).count(), 2);
    assert_eq!(
        cast.of(Kind::Style)
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["film"]
    );

    for (text, why) in [
        ("cast Ada \"a keeper\"\n", "starts with `kerf-cast 1`"),
        (
            "kerf-cast 1\nlook film \"35mm\"\n",
            "`look` is not a cast file's word",
        ),
        (
            "kerf-cast 1\ncast \"an old keeper\"\n",
            "followed by a name",
        ),
        ("kerf-cast 1\ncast Ada\n", "`Ada` says nothing"),
        (
            "kerf-cast 1\ncast Ada \"a keeper\" \"in a coat\"\n",
            "one quoted text",
        ),
        (
            "kerf-cast 1\ncast Ada ref=../outside.png\n",
            "is not inside the project",
        ),
        (
            "kerf-cast 1\ncast Ada \"a\"\ncast Ada \"b\"\n",
            "cast `Ada` is said twice",
        ),
    ] {
        let error = Cast::parse(text).unwrap_err().to_string();
        assert!(error.contains(why), "{text:?}: {error}");
    }
    // A line names one by its name alone, once.
    for (line, why) in [
        (
            "generate model=dawn cast=\"the keeper\"",
            "names one of the project's `cast.kerfcast`",
        ),
        (
            "generate model=dawn style=film style=film",
            "`style=film` is given twice",
        ),
    ] {
        let error = Recipe::parse(&format!("kerf-recipe 1\n{line}\n"))
            .unwrap_err()
            .to_string();
        assert!(error.contains(why), "{line}: {error}");
    }
}

#[cfg(unix)]
#[test]
fn a_model_is_asked_with_the_cast_a_line_names_and_a_take_is_held_to_it() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::create_dir_all(directory.join("refs")).unwrap();
    for picture in ["ada.png", "lamp.png", "sea.png"] {
        std::fs::write(directory.join("refs").join(picture), picture).unwrap();
    }
    let some = models(
        directory,
        r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"]}}"#,
    );
    let project = shot(
        "shots/open.json",
        "generate model=dawn prompt=\"Ada wipes the great lens\" ref=refs/sea.png ref=refs/lamp.png cast=Ada cast=lamp style=film seed=7",
    );
    let cannot = |why: &str| {
        let error = plan(&project, directory, "m1", &some)
            .unwrap_err()
            .to_string();
        assert!(error.contains(why), "{error}");
    };
    // A name the project does not say is refused before anything runs, with the line that says it.
    cannot("the project has no `cast.kerfcast`");
    let write = |text: &str| std::fs::write(directory.join("cast.kerfcast"), text).unwrap();
    write("kerf-cast 1\ncast lamp \"a lamp room\"\nstyle film \"35mm\"\n");
    cannot(
        "which `cast.kerfcast` has not; it has lamp. Name one of those, or add the line `cast Ada",
    );
    write("kerf-cast 1\ncast Ada \"a keeper\" ref=refs/gone.png\ncast lamp \"a lamp room\"\nstyle film \"35mm\"\n");
    cannot("cast `Ada` in `cast.kerfcast` names the picture `refs/gone.png`, which is not there");

    let cast = |ada: &str, more: &str| {
        write(&format!(
            "kerf-cast 1\ncast Ada \"{ada}\" ref=refs/ada.png\n\
             cast lamp \"the lamp room of a lighthouse.\" ref=refs/lamp.png\n\
             style film \"shot on 35mm colour negative\"\n{more}"
        ));
    };
    cast("an old keeper in a wool coat", "");
    let token = CancellationToken::new();
    let planned = plan(&project, directory, "m1", &some).unwrap();
    // The person is shown what the model is asked, the cast's words among it.
    assert!(
        planned.steps[0].contains("Ada: an old keeper in a wool coat."),
        "{}",
        planned.steps[0]
    );
    let made = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join(&made.stored)).unwrap()).unwrap();
    assert_eq!(
        request["prompt"],
        "Ada wipes the great lens. Ada: an old keeper in a wool coat. \
         lamp: the lamp room of a lighthouse. shot on 35mm colour negative."
    );
    // The line's own pictures first, then the cast's, each once.
    let path = |name: &str| directory.join(name).to_string_lossy().into_owned();
    assert_eq!(
        request["references"],
        serde_json::json!([
            path("refs/sea.png"),
            path("refs/lamp.png"),
            path("refs/ada.png")
        ])
    );

    let mut taken = project.clone();
    taken.media[0].path = made.stored.clone();
    let wants = || unmade(&taken, directory, taken.media("m1").unwrap());
    assert_eq!(wants(), None);
    // Someone the shot does not show is added, and it is not asked for again.
    cast(
        "an old keeper in a wool coat",
        "cast gull \"a herring gull\"\nstyle night \"moonlight\"\n",
    );
    assert_eq!(wants(), None);
    // What it shows is said another way, or shown by another picture, and it is.
    cast("a young keeper in oilskins", "");
    assert_eq!(
        wants().unwrap().reason(),
        "cast Ada changed after its file was made"
    );
    cast("an old keeper in a wool coat", "");
    assert_eq!(wants(), None);
    std::fs::write(directory.join("refs/ada.png"), "another picture of her").unwrap();
    assert_eq!(
        wants().unwrap().reason(),
        "refs/ada.png changed after its file was made"
    );
    // One the file no longer says is why the steps cannot run.
    write("kerf-cast 1\ncast lamp \"a lamp room\" ref=refs/lamp.png\nstyle film \"35mm\"\n");
    let gone = wants().unwrap().reason();
    assert!(
        gone.starts_with("its steps cannot run: a step names `cast=Ada`"),
        "{gone}"
    );
}

#[cfg(unix)]
#[test]
fn another_roll_of_the_steps_is_the_same_shot_with_its_seeds_on() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    let some = models(
        directory,
        r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"]}}"#,
    );
    let mut project = shot("shots/open.json", "generate model=dawn prompt=\"a boat leaves\" seed=7");
    let token = CancellationToken::new();
    assert_eq!(next_roll(&project, directory, "m1").unwrap(), 0);
    let mut seeds = Vec::new();
    for _ in 0..3 {
        let roll = next_roll(&project, directory, "m1").unwrap();
        let planned = plan_rolled(&project, directory, "m1", &some, roll).unwrap();
        // The person is shown the seed this one is asked with.
        assert!(
            planned.steps[0].contains(&format!("seed {}", 7 + roll)),
            "{}",
            planned.steps[0]
        );
        let made = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
        let request: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join(&made.stored)).unwrap()).unwrap();
        seeds.push(request["seed"].as_u64().unwrap());
        project.media[0].path = made.stored;
    }
    assert_eq!(seeds, [7, 8, 9]);
    let media = project.media("m1").unwrap();
    let all = takes(&project, directory, media).unwrap();
    assert_eq!(
        all.iter().map(|take| take.rolled).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    // A roll is another of the same shot: none of them is asked for again.
    assert!(all.iter().all(|take| take.changed.is_empty()));
    assert_eq!(unmade(&project, directory, media), None);
}

/// A take keeps the steps it was made from, each seed as that take had it, and the workflow
/// file its line named: what going back to it writes.
#[cfg(unix)]
#[test]
fn a_take_keeps_the_steps_and_the_workflow_it_was_made_from() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::write(directory.join("shots/own.json"), "{\"steps\": 20}").unwrap();
    let some = models(
        directory,
        r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"]}}"#,
    );
    let mut project = shot(
        "shots/open.json",
        "generate model=dawn prompt=\"a boat leaves\" seed=7 workflow=shots/own.json",
    );
    let text = steps_text(&project.media[0].steps);
    let token = CancellationToken::new();
    for _ in 0..2 {
        let roll = next_roll(&project, directory, "m1").unwrap();
        let planned = plan_rolled(&project, directory, "m1", &some, roll).unwrap();
        let made = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
        project.media[0].path = made.stored;
    }
    // The file changes after the takes are made: what they keep is what they were made with.
    std::fs::write(directory.join("shots/own.json"), "{\"steps\": 30}").unwrap();
    let media = project.media("m1").unwrap();
    let all = takes(&project, directory, media).unwrap();
    assert!(all.iter().all(|take| take.kept));
    let first = takes::kept(directory, media, 1).unwrap();
    assert_eq!(first.recipe, text);
    assert_eq!(first.files, [("shots/own.json".to_owned(), "{\"steps\": 20}".to_owned())]);
    // The second roll's seed is the one it was asked with.
    let second = takes::kept(directory, media, 2).unwrap();
    assert_eq!(second.recipe, text.replace("seed=7", "seed=8"));
    assert_eq!(takes::kept(directory, media, 3), None);
    // A take whose working folder is gone has nothing kept, and says so.
    std::fs::remove_dir_all(directory.join("shots/open.takes/001.work")).unwrap();
    assert_eq!(takes::kept(directory, media, 1), None);
    assert!(!takes(&project, directory, media).unwrap()[0].kept);
}

#[cfg(unix)]
#[test]
fn the_shots_of_a_project_are_read_at_once() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::write(
        directory.join("cast.kerfcast"),
        "kerf-cast 1\ncast Ada \"an old keeper\"\nstyle film \"35mm\"\n",
    )
    .unwrap();
    let some = models(
        directory,
        r#"{"dawn": {"run": ["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "dawn", "{request}"]}}"#,
    );
    let mut project = project(
        "media m1 path=\"shots/open.json\" rate=24 dur=4s video=160x120\n\
         \x20 generate model=dawn prompt=\"she waits\" cast=Ada style=film seed=7\n\
         media m2 path=\"shots/pier.json\" rate=24 dur=2s video=160x120\n\
         \x20 generate model=dawn prompt=\"an empty pier\"\n\
         media m3 path=\"footage/sea.mp4\" rate=24 dur=9s video=160x120\n\
         media m4 path=\"shots/old.json\" rate=24 dur=1s video=160x120 recipe=\"shots/old.kerfrecipe\"\n\
         sequence s1 rate=24 size=160x120 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 at=0f src=0f dur=96f\n",
    );
    let token = CancellationToken::new();
    let planned = plan(&project, directory, "m1", &some).unwrap();
    let made = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
    project.media[0].path = made.stored;
    let all = shots(&project, directory);
    // A media nothing makes is not a shot; one whose steps are still in a recipe's file is,
    // and wants carrying.
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].line(), "  m1  take 1 of 1  4s  Ada, film");
    assert_eq!(
        all[1].line(),
        "! m2  no take  2s  (no clip plays it)  to make: its file is not there yet"
    );
    assert_eq!(
        all[2].line(),
        "! m4  no take  1s  (no clip plays it)  to carry: its steps are in shots/old.kerfrecipe; `kerf carry m4` writes them under it"
    );
    assert_eq!(all[2].recipe.as_deref(), Some("shots/old.kerfrecipe"));
    assert_eq!(
        super::shots::summary(&all),
        "3 shots, 2 to make (m2, m4); 4s of 7s made"
    );
    assert_eq!(super::shots::summary(&all[..1]), "1 shot, 4s, all made");
}

#[test]
fn a_cast_is_written_as_it_reads_and_added_to_without_changing_what_is_there() {
    use super::cast::{Cast, Entry, Kind};
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    // No file is no cast, and adding to none starts one.
    assert_eq!(Cast::read(directory).unwrap().entries().count(), 0);
    let added = Cast::add(
        directory,
        &[
            "cast lamp \"a lamp room\"".to_owned(),
            "style film \"35mm\"".to_owned(),
        ],
    )
    .unwrap();
    assert_eq!(added, ["cast lamp", "style film"]);
    // What a person wrote there, its comments among it, stays as it is.
    std::fs::write(
        directory.join("cast.kerfcast"),
        "kerf-cast 1\n# The keeper.\ncast Ada \"an old keeper\" ref=refs/ada.png",
    )
    .unwrap();
    Cast::add(directory, &["cast gull \"a herring gull\"".to_owned()]).unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.join("cast.kerfcast")).unwrap(),
        "kerf-cast 1\n# The keeper.\ncast Ada \"an old keeper\" ref=refs/ada.png\ncast gull \"a herring gull\"\n"
    );
    // One the file has is not said again, and a line that does not read adds nothing.
    for (line, why) in [
        ("cast Ada \"a young keeper\"", "has a cast `Ada` already"),
        ("cast \"no name\"", "followed by a name"),
    ] {
        let error = Cast::add(directory, &[line.to_owned()])
            .unwrap_err()
            .to_string();
        assert!(error.contains(why), "{line}: {error}");
    }
    // They are kept in the order the file says them, and read the moment it changes.
    let cast = Cast::read(directory).unwrap();
    let names: Vec<&str> = cast.entries().map(|(_, name, _)| name).collect();
    assert_eq!(names, ["Ada", "gull"]);
    // A cast of entries is the file an editor writes: words and paths that hold spaces and
    // quotes come back as they went.
    let said = Cast::of_entries([
        (
            Kind::Cast,
            "Ada".to_owned(),
            Entry {
                words: "an old keeper, \"the last\"".to_owned(),
                references: vec!["refs/ada front.png".to_owned()],
            },
        ),
        (
            Kind::Style,
            "film".to_owned(),
            Entry {
                words: "35mm".to_owned(),
                references: Vec::new(),
            },
        ),
    ])
    .unwrap();
    assert_eq!(
        said.text(),
        "kerf-cast 1\ncast Ada \"an old keeper, \\\"the last\\\"\" ref=\"refs/ada front.png\"\nstyle film \"35mm\"\n"
    );
    said.write(directory).unwrap();
    assert_eq!(Cast::read(directory).unwrap(), said);
    assert_eq!(Cast::parse(&said.text()).unwrap(), said);
    // Written over a file, what a person wrote there that is not the cast stays: its
    // comments and empty lines where they are, and a line that is unchanged as it was
    // written. One taken out leaves its line, and a new one follows what the file holds.
    assert_eq!(
        std::fs::read_to_string(directory.join("cast.kerfcast")).unwrap(),
        "kerf-cast 1\n# The keeper.\ncast Ada \"an old keeper, \\\"the last\\\"\" ref=\"refs/ada front.png\"\nstyle film \"35mm\"\n"
    );
    std::fs::write(
        directory.join("cast.kerfcast"),
        "# A lighthouse.\nkerf-cast 1\n\n# Who.\ncast   Ada \"an old keeper\"\n  # Gone soon.\ncast gull \"a herring gull\"\n\n# How.\nstyle film \"35mm\"\n",
    )
    .unwrap();
    let entry = |words: &str| Entry {
        words: words.to_owned(),
        references: Vec::new(),
    };
    let set = Cast::of_entries([
        (Kind::Cast, "Ada".to_owned(), entry("an old keeper")),
        (Kind::Style, "film".to_owned(), entry("16mm")),
        (Kind::Cast, "lamp".to_owned(), entry("a lamp room")),
    ])
    .unwrap();
    set.write(directory).unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.join("cast.kerfcast")).unwrap(),
        "# A lighthouse.\nkerf-cast 1\n\n# Who.\ncast   Ada \"an old keeper\"\n  # Gone soon.\n\n# How.\nstyle film \"16mm\"\ncast lamp \"a lamp room\"\n"
    );
    assert_eq!(Cast::read(directory).unwrap(), set);
    // A file that is not a cast file has nothing to keep.
    std::fs::write(directory.join("cast.kerfcast"), "# Notes.\nnot a cast\n").unwrap();
    set.write(directory).unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.join("cast.kerfcast")).unwrap(),
        set.text()
    );
    // One that says nothing, or two of a name, is refused as a file's line is.
    let nothing = Cast::of_entries([(Kind::Cast, "Ada".to_owned(), Entry::default())]);
    assert!(nothing
        .unwrap_err()
        .to_string()
        .contains("`Ada` says nothing"));
}

/// A pass of a sequence with nothing in depth is black on every frame, and a nested
/// sequence is one flat clip in it: the plan says so before a model is handed it.
#[test]
fn a_pass_of_nothing_in_depth_is_said_to_be_black() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let some = models(
        directory,
        r#"{"shot": {"run": ["a", "{request}"], "takes": {"reads": ["guide"]}}}"#,
    );
    let scenes = "media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120\n\
         sequence set rate=24 size=160x120 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip box gen=mesh@1 at=0f dur=24f\n\
         \x20 track V2 video\n\
         \x20   clip eye gen=camera@1 at=0f dur=24f\n\
         sequence flat rate=24 size=160x120 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip sky gen=solid@1 at=0f dur=24f\n\
         sequence nest rate=24 size=160x120 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip inner seq=set src=0f at=0f dur=24f\n";
    let planned = |steps: &str| {
        let project = with_steps(scenes, &format!("{steps}\ngenerate model=shot guide={{g.mp4}}"));
        plan(&project, directory, "m1", &some).unwrap().notes
    };
    // A mesh is in depth whether it says so or not; a picture needs nothing in depth.
    assert!(planned("render set {g.mp4} pass=depth").is_empty());
    assert!(planned("render flat {g.mp4}").is_empty());
    assert_eq!(
        planned("render flat {g.mp4} pass=depth"),
        ["the depth of sequence `flat` into `{g.mp4}` is black: none of its clips is in depth (`three_d=true`, or a `gen=mesh` clip)"]
    );
    assert_eq!(
        planned("render nest {g.mp4} pass=normals"),
        ["the normals of sequence `nest` into `{g.mp4}` is black: none of its clips is in depth, and a nested sequence is drawn flat, as one clip: render `set` itself"]
    );
}

/// A name a line takes once, given twice, is refused rather than its first value dropped:
/// `out={lens.txt} out={out}` wrote no lens. Names that hold a list repeat.
#[test]
fn a_name_given_twice_on_a_line_is_refused() {
    let words = |line: &str| line.split(' ').map(str::to_owned).collect::<Vec<_>>();
    let twice = Ask::parse(&words("model=set out={lens.txt} out={out}"), &[]).unwrap_err();
    assert!(
        twice.starts_with("`out=` is given twice; a line takes one, the file it makes"),
        "{twice}"
    );
    let seed = Ask::parse(&words("model=shot seed=1 seed=2"), &[]).unwrap_err();
    assert_eq!(seed, "`seed=` is given twice; a line takes one");
    let more = Ask::parse(&words("model=shot steps=20 steps=30"), &[]).unwrap_err();
    assert!(more.starts_with("`steps=` is given twice"), "{more}");
    let listed = Ask::parse(
        &words("model=shot ref=a.png ref=b.png signal=s moves=m moves=n"),
        &[],
    )
    .unwrap();
    assert_eq!(listed.references.len(), 2);
    assert_eq!(listed.moves, ["m", "n"]);
}

/// A set a model made is shown by the mesh clips that read its file, and those play it.
#[test]
fn a_mesh_clip_that_reads_a_set_plays_it() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let read = |file: &str| {
        let project = project(&format!(
            "media m1 path=\"shots/set.takes/001.glb\" rate=16 dur=1f video=832x480\n\
             \x20 generate model=set\n\
             sequence s1 rate=16 size=832x480 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip room gen=mesh@1 at=0f dur=16f\n\
             \x20     set file=\"{file}\"\n"
        ));
        shots(&project, directory)[0].clips
    };
    assert_eq!(read("shots/set.takes/001.glb"), 1);
    assert_eq!(read("models/car.glb"), 0);
}

/// A shot longer than a model makes at once is written in parts: a guide of each part, a
/// `generate` that goes on from the part before, and a `join` that sets them end to end.
#[test]
fn a_longer_shot_is_parts_set_end_to_end() {
    let text = "kerf-recipe 1\n\
        render court {d1.mp4} pass=depth rate=16 seconds=5.0625\n\
        render court {d2.mp4} pass=depth rate=16 from=5 seconds=5.0625\n\
        generate model=video prompt=\"a ball\" guide={d1.mp4} out={a.mp4}\n\
        generate model=video prompt=\"a ball\" guide={d2.mp4} first={a.mp4} from=5 out={b.mp4}\n\
        join {a.mp4} {b.mp4} {out} lap=1\n";
    let recipe = Recipe::parse(text).unwrap();
    assert_eq!(recipe.steps.len(), 5);
    let Step::Render { from, seconds, .. } = &recipe.steps[1] else {
        panic!("a render step");
    };
    assert_eq!((*from, *seconds), (Some(5.0), Some(5.0625)));
    let Step::Generate(second) = &recipe.steps[3] else {
        panic!("a generate step");
    };
    assert_eq!(second.from, Some(5.0));
    assert_eq!(second.first, Some(Source::Made("a.mp4".into())));
    assert_eq!(
        recipe.steps[4],
        Step::Join { parts: vec!["a.mp4".into(), "b.mp4".into()], to: "out".into(), lap: 1 }
    );
    assert_eq!(recipe.makes(), ["a.mp4", "b.mp4", "d1.mp4", "d2.mp4"]);
    // The request of a part says where it starts; of a whole shot it says nothing of it.
    let place = |name: &str| PathBuf::from(format!("/takes/{name}"));
    let asked = model::request(second, &|file| Ok(PathBuf::from(file)), &place, &[], &[]).unwrap();
    assert_eq!(asked["from"], 5.0);
    assert_eq!(asked["first"], "/takes/a.mp4");

    let wrong = |line: &str| {
        Recipe::parse(&format!("kerf-recipe 1\nrender court {{a.mp4}}\nrender court {{b.mp4}}\n{line}\n"))
            .unwrap_err()
            .to_string()
    };
    assert!(wrong("join {a.mp4} {out}").contains("two or more parts"));
    assert!(wrong("join {a.mp4} {c.mp4} {out}").contains("no step before this one makes `{c.mp4}`"));
    assert!(wrong("join {a.mp4} {b.mp4} {a.mp4}").contains("which is one of its parts"));
    assert!(wrong("join {a.mp4} {b.mp4} {out} lap=some").contains("`lap=some` is not a whole number"));
    assert!(wrong("join {a.mp4} {b.mp4} {out} fade=1").contains("is not something `join` takes"));
    assert!(wrong("render court {out} from=-1").contains("`from=-1` is not a number of seconds"));
    assert!(wrong("render court {out} seconds=0").contains("`seconds=0` is not a number of seconds"));
    assert!(wrong("generate model=m prompt=\"p\" from=soon").contains("`from=soon` is not a number of seconds"));
}

/// Parts are joined by ffmpeg, each after the first without the frames it shares with the
/// one before, and a picture in motion gives its last frame to the part that goes on from it.
#[test]
fn parts_are_joined_without_the_frames_they_share() {
    let ffmpeg = crate::ffmpeg::resolve_ffmpeg_bin();
    if std::process::Command::new(ffmpeg).arg("-version").output().is_err() {
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    let part = |name: &str, color: &str| {
        let file = folder.path().join(name);
        let made = std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("color=c={color}:s=64x36:r=16:d=1"))
            .args(["-pix_fmt", "yuv420p"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(made.success());
        file
    };
    let frames = |file: &Path| -> u32 {
        let counted = std::process::Command::new(crate::ffmpeg::resolve_ffprobe_bin())
            .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
            .args(["-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
            .arg(file)
            .output()
            .unwrap();
        String::from_utf8_lossy(&counted.stdout).trim().parse().unwrap()
    };
    let (a, b) = (part("a.mp4", "red"), part("b.mp4", "blue"));
    let out = folder.path().join("out.mp4");
    assert!(join_command(&[a.clone(), b.clone()], &out, 1).status().unwrap().success());
    assert_eq!(frames(&out), frames(&a) + frames(&b) - 1);
    assert!(join_command(&[a.clone(), b.clone(), a.clone()], &out, 0).status().unwrap().success());
    assert_eq!(frames(&out), 2 * frames(&a) + frames(&b));
    // The last frame of the red part is red, and the first of the blue one blue.
    let still = folder.path().join("a.last.png");
    assert!(in_motion(&a) && !in_motion(&still));
    assert!(still_command(&a, &still, true).status().unwrap().success());
    assert!(still.is_file());
}

/// A model that answers with what it was asked: it copies its request to where the request
/// says the file goes.
#[cfg(unix)]
const ECHO: &str = r#"["sh", "-c", "cp \"$1\" \"$(sed -n 's/.*\"out\": \"\\(.*\\)\".*/\\1/p' \"$1\")\"", "echo", "{request}"]"#;

/// A part of footage whose steps the document holds: the matte a model makes of it, and
/// the plate a second model makes with that matte.
#[cfg(unix)]
fn gone() -> Project {
    project(
        "media walk path=\"footage/walk.json\" rate=16 dur=81f video=832x480\n\
         media gone path=\"shots/gone.json\" rate=16 dur=81f video=832x480\n\
         \x20 generate model=matte guide=footage/walk.json prompt=\"man\" out={man.json}\n\
         \x20 generate model=plate guide=footage/walk.json guide_mask={man.json} prompt=\"an empty dirt path\" reach=72 seed=7\n\
         sequence s1 rate=16 size=832x480\n",
    )
}

/// Steps the document holds under a media are planned and made as a recipe's are: with no
/// file beside the project to say them, each file named from the project's folder, and a
/// later step handed what an earlier one made.
#[cfg(unix)]
#[test]
fn steps_the_document_holds_make_a_take_and_a_later_one_reads_an_earlier() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("footage")).unwrap();
    std::fs::write(directory.join("footage/walk.json"), "a man walks").unwrap();
    let some = models(directory, &format!(r#"{{"matte": {{"run": {ECHO}}}, "plate": {{"run": {ECHO}}}}}"#));
    let mut project = gone();
    let media = |project: &Project| project.media("gone").unwrap().clone();
    // Its steps are the document's, and footage has none.
    assert_eq!(uncarried(&media(&project)), None);
    assert_eq!(written(&project, "gone").unwrap(), steps_text(&media(&project).steps));
    assert!(written(&project, "walk").unwrap_err().to_string().contains("has no steps that make it"));
    // Nothing is made yet, and nothing but the document says how it will be.
    assert_eq!(unmade(&project, directory, &media(&project)), Some(Unmade::Missing));
    let planned = plan(&project, directory, "gone", &some).unwrap();
    assert_eq!(planned.steps.len(), 2);
    assert_eq!(planned.file, directory.join("shots/gone.takes/001.json"));
    let token = CancellationToken::new();
    let made = make(&project, directory, &planned, &some, &token, |_| {}).unwrap();
    assert_eq!(made.stored, "shots/gone.takes/001.json");
    assert!(!directory.join("shots/gone.kerfrecipe").exists());
    let path = |name: &str| directory.join(name).to_string_lossy().into_owned();
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(directory.join(name)).unwrap()).unwrap()
    };
    // The plate's model was handed the footage, named from the project's folder, and the
    // matte the step before it made.
    let plate = read(&made.stored);
    assert_eq!(plate["model"], "plate");
    assert_eq!(plate["guide"], path("footage/walk.json"));
    assert_eq!(plate["guides"]["mask"], path("shots/gone.takes/001.work/man.json"));
    assert_eq!(plate["more"]["reach"], "72");
    let matte = read("shots/gone.takes/001.work/man.json");
    assert_eq!((&matte["model"], &matte["prompt"]), (&"matte".into(), &"man".into()));

    // The media is the take, and is held to what it was made from: its steps, and the
    // footage they read.
    project.media[1].path = made.stored;
    assert_eq!(unmade(&project, directory, &media(&project)), None);
    let all = takes(&project, directory, &media(&project)).unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].chosen && all[0].current && all[0].kept && all[0].changed.is_empty());
    // What the take keeps of how it was made is the steps as the document had them.
    let kept = takes::kept(directory, &media(&project), 1).unwrap();
    assert_eq!(kept.recipe, steps_text(&media(&project).steps));
    assert!(kept.recipe.ends_with(
        "generate model=plate prompt=\"an empty dirt path\" guide=footage/walk.json guide_mask={man.json} seed=7 reach=72\n"
    ), "{}", kept.recipe);

    // Another making of the same steps is another roll, with its seed one on.
    assert_eq!(next_roll(&project, directory, "gone").unwrap(), 1);
    let rolled = plan_rolled(&project, directory, "gone", &some, 1).unwrap();
    assert_eq!(rolled.take, 2);
    assert!(rolled.steps[1].contains("seed 8"), "{}", rolled.steps[1]);

    // An edit that changes a step asks for the media again, and says it is its steps.
    let edited = kerf_lang::apply_script(
        &project,
        kerf_model::Catalog::builtin(),
        "step gone set 2 reach=96",
    )
    .unwrap()
    .project;
    assert_eq!(
        unmade(&edited, directory, &media(&edited)),
        Some(Unmade::Stale(vec!["its steps".to_owned()]))
    );
    let all = takes(&edited, directory, &media(&edited)).unwrap();
    assert!(!all[0].current);
    assert_eq!(all[0].changed, ["its steps"]);
    assert_eq!(next_roll(&edited, directory, "gone").unwrap(), 0);
    // Undone, it is made again: the document as it was is what the take was made from.
    assert_eq!(unmade(&project, directory, &media(&project)), None);
    // And so does the footage changing under it.
    std::fs::write(directory.join("footage/walk.json"), "another man walks").unwrap();
    assert_eq!(
        unmade(&project, directory, &media(&project)),
        Some(Unmade::Stale(vec!["footage/walk.json".to_owned()]))
    );
}

/// What would refuse a making of steps the document holds is said of them, not of a recipe.
#[cfg(unix)]
#[test]
fn what_steps_in_the_document_cannot_do_is_said_of_them() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    let some = models(directory, &format!(r#"{{"matte": {{"run": {ECHO}}}, "plate": {{"run": {ECHO}}}}}"#));
    let wrong = |project: &Project, id: &str| plan(project, directory, id, &some).unwrap_err().to_string();
    // The footage is not there.
    let missing = wrong(&gone(), "gone");
    assert_eq!(missing, "the steps need `footage/walk.json`, which is not there");
    // Steps that stop short of the media's file.
    let short = project(
        "media depth path=\"shots/depth.mp4\" rate=16 dur=65f video=832x480\n\
         \x20 render s2 {guide.mp4} pass=depth\n\
         sequence s1 rate=16 size=832x480\n",
    );
    let why = wrong(&short, "depth");
    assert!(why.contains("the last of its steps names no `{out}`"), "{why}");
    assert_eq!(
        unmade(&short, directory, short.media("depth").unwrap()).map(|unmade| unmade.reason()),
        Some(format!("its steps cannot run: {why}"))
    );
    // A sequence the project has not.
    let unknown = project(
        "media depth path=\"shots/depth.mp4\" rate=16 dur=65f video=832x480\n\
         \x20 render s2 {out} pass=depth\n\
         sequence s1 rate=16 size=832x480\n",
    );
    assert_eq!(
        wrong(&unknown, "depth"),
        "the steps render sequence `s2`, which this project has not"
    );
    // Footage, which nothing makes.
    assert!(wrong(&gone(), "walk").contains("has no steps that make it"));
}

/// Where the takes of a media whose steps the document holds are kept goes by the file
/// its line names, and is the same folder once the media is one of them.
#[test]
fn the_takes_of_steps_in_the_document_are_kept_by_the_medias_file() {
    let directory = Path::new("/project");
    let stored = |media: &str| takes::Place::of_media(directory, media).unwrap().stored(2);
    assert_eq!(stored("shots/depth.mp4"), "shots/depth.takes/002.mp4");
    assert_eq!(stored("shots/depth.takes/001.mp4"), "shots/depth.takes/002.mp4");
    assert_eq!(stored("shots/depth.takes/014.mp4"), "shots/depth.takes/002.mp4");
    assert_eq!(stored("set.glb"), "set.takes/002.glb");
    // The folder a recipe of the same name keeps its takes in: a shot carried from a
    // recipe into the document keeps them.
    assert_eq!(
        takes::Place::of(directory, "shots/depth@2.kerfrecipe", "shots/depth.mp4").unwrap().stored(2),
        stored("shots/depth.mp4")
    );
    // A file that only looks like a take's is the media's own.
    assert_eq!(stored("shots/b.takes/cut.mp4"), "shots/b.takes/cut.takes/002.mp4");
    assert!(takes::Place::of_media(directory, "../outside.mp4").is_err());
}

/// A sequence a step draws reads the files its mesh clips name, and nothing of the
/// document says when one changes: a take of steps the document holds is held to them.
#[test]
fn a_take_of_steps_in_the_document_is_held_to_the_meshes_its_guide_draws() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("models")).unwrap();
    std::fs::write(directory.join("models/car.glb"), "a car").unwrap();
    let project = project(
        "media depth path=\"shots/depth.mp4\" rate=16 dur=65f video=832x480\n\
         \x20 render s2 {out} pass=depth\n\
         sequence s1 rate=16 size=832x480\n\
         sequence s2 rate=16 size=832x480\n\
         \x20 track V1 video\n\
         \x20   clip car gen=mesh@1 at=0f dur=16f\n\
         \x20     set file=\"models/car.glb\"\n",
    );
    let media = project.media("depth").unwrap();
    let recipe = Recipe::of_steps(&media.steps);
    let from = || takes::from(&project, directory, &steps_text(&media.steps), &recipe).unwrap();
    let held = from();
    assert!(held.contains_key("steps") && held.contains_key("sequence s2"), "{held:?}");
    let before = held["file models/car.glb"].clone();
    std::fs::write(directory.join("models/car.glb"), "another car").unwrap();
    assert_ne!(from()["file models/car.glb"], before);
    // What the document's steps come to, read as a recipe's text is, is those steps.
    assert_eq!(Recipe::parse(&steps_text(&media.steps)).unwrap(), recipe);
}

/// A change made to the text of steps the document holds comes to the lines of an edit
/// that makes it, and a text the document has no place for is refused.
#[test]
fn a_change_to_the_text_of_steps_is_the_edit_that_makes_it() {
    let was = "kerf-recipe 1\nrender s2 {guide.mp4} pass=depth\ngenerate model=shot prompt=\"a car\" guide={guide.mp4} seed=7\n";
    // One name set on one step is that step set, written whole.
    let seeded = text::changed(was, 3, text::Change::Set { key: "seed", nth: 0, value: "8" }).unwrap();
    assert_eq!(
        step_lines("depth", was, &seeded).unwrap(),
        ["step depth set 2 generate model=shot prompt=\"a car\" guide={guide.mp4} seed=8"]
    );
    assert!(step_lines("depth", was, was).unwrap().is_empty());
    // A step added is every step written again, last first out so that none reads a
    // file whose step is gone.
    let added = text::added(was, 2, "generate model=still prompt=\"a red car\" out={car.png}").unwrap();
    assert_eq!(
        step_lines("depth", was, &added).unwrap(),
        [
            "step depth remove 2",
            "step depth remove 1",
            "step depth add render s2 {guide.mp4} pass=depth",
            "step depth add generate model=still prompt=\"a red car\" out={car.png}",
            "step depth add generate model=shot prompt=\"a car\" guide={guide.mp4} seed=7",
        ]
    );
    // And the lines make it: the document then holds what the text said.
    let project = project(
        "media depth path=\"shots/depth.mp4\" rate=16 dur=65f video=832x480\n\
         \x20 render s2 {guide.mp4} pass=depth\n\
         \x20 generate model=shot prompt=\"a car\" guide={guide.mp4} seed=7\n\
         sequence s1 rate=16 size=832x480\n",
    );
    assert_eq!(steps_text(&project.media[0].steps), was);
    let edited = kerf_lang::apply_script(
        &project,
        kerf_model::Catalog::builtin(),
        &step_lines("depth", was, &added).unwrap().join("\n"),
    )
    .unwrap()
    .project;
    assert_eq!(steps_text(&edited.media[0].steps), added);
    // A program to run is not a step.
    let ran = document_steps("kerf-recipe 1\nrun vork render --out {out}\n").unwrap_err().to_string();
    assert!(ran.contains("`run vork` runs a program, and a document names none"), "{ran}");
}

/// What the document writes of a step, the engine reads as the same step, whatever its
/// words hold: the two read a line by rules of their own, and agree on what is written.
#[test]
fn a_step_the_document_writes_is_the_step_a_making_reads() {
    let document = project(
        "media m3 path=\"shots/open.mp4\" rate=16 dur=65f video=832x480\n\
         \x20 generate model=dawn prompt=\"a \\\"wet\\\" street, C:\\\\reel\" ref=\"refs/a b.png\" ref=\"refs/a\u{a0}c.png\" tint=#ff0000 knob=\"x=1\" out={first.png}\n\
         \x20 generate model=dawn prompt=\"\" first={first.png} empty=\"\"\n\
         sequence s1 rate=16 size=832x480\n",
    );
    let steps = &document.media[0].steps;
    let read = Recipe::parse(&steps_text(steps)).unwrap();
    assert_eq!(read, Recipe::of_steps(steps));
    let Step::Generate(ask) = &read.steps[0] else {
        panic!("a model's step");
    };
    assert_eq!(ask.prompt, "a \"wet\" street, C:\\reel");
    assert_eq!(
        ask.references,
        [Source::File("refs/a b.png".to_owned()), Source::File("refs/a\u{a0}c.png".to_owned())]
    );
    assert_eq!(ask.more["knob"], "x=1");
    assert_eq!(document_steps(&steps_text(steps)).unwrap(), *steps);
}

/// The recipe `shots/gone.kerfrecipe` takes are carried from: a matte of the footage, and
/// a plate made with it, its files named from the recipe's own folder.
#[cfg(unix)]
const GONE: &str = "kerf-recipe 1\n# The man taken out.\ngenerate model=matte guide=../footage/walk.json prompt=\"man\" out={man.json}\ngenerate model=plate guide=../footage/walk.json guide_mask={man.json} prompt=\"an empty path\" reach=72 seed=7\n";

/// Two takes of media `gone` in `directory` as a making from a recipe's file left them, the
/// second a roll, and the project as it was then: its media naming `shots/gone.kerfrecipe`,
/// which holds [`GONE`], and the second take. Take `n` was made from the recipe's text
/// `made_from[n - 1]`. Nothing makes a take from a recipe now, so they are made from steps
/// that say what the recipe says, and their records and what they keep are then written as
/// the making of a recipe wrote them.
#[cfg(unix)]
fn takes_of_a_recipe(directory: &Path, made_from: [&str; 2]) -> Project {
    use sha2::{Digest, Sha256};
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    std::fs::create_dir_all(directory.join("footage")).unwrap();
    std::fs::write(directory.join("footage/walk.json"), "a man walks").unwrap();
    std::fs::write(directory.join("shots/gone.kerfrecipe"), GONE).unwrap();
    let some = models(directory, &format!(r#"{{"matte": {{"run": {ECHO}}}, "plate": {{"run": {ECHO}}}}}"#));
    let mut steps = project(
        "media gone path=\"shots/gone.json\" rate=16 dur=81f video=832x480\n\
         \x20 generate model=matte guide=footage/walk.json prompt=\"man\" out={man.json}\n\
         \x20 generate model=plate guide=footage/walk.json guide_mask={man.json} prompt=\"an empty path\" reach=72 seed=7\n\
         sequence s1 rate=16 size=832x480\n",
    );
    let token = CancellationToken::new();
    for (roll, text) in (0..2).zip(made_from) {
        let planned = plan_rolled(&steps, directory, "gone", &some, roll).unwrap();
        let made = make(&steps, directory, &planned, &some, &token, |_| {}).unwrap();
        // Its record is held to the recipe's text where it is now held to steps, and what it
        // keeps is that text, its seed as the roll had it.
        let record = directory.join(format!("shots/gone.takes/{:03}.take.json", made.take));
        let mut held: serde_json::Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
        let from = held["from"].as_object_mut().unwrap();
        assert!(from.remove("steps").is_some());
        from.insert(
            "recipe".to_owned(),
            hex::encode(Sha256::digest(Recipe::essence(text))).into(),
        );
        std::fs::write(&record, serde_json::to_vec_pretty(&held).unwrap()).unwrap();
        std::fs::write(
            directory.join(format!("shots/gone.takes/{:03}.work/made-from.kerfrecipe", made.take)),
            text.replace("seed=7", &format!("seed={}", 7 + roll)),
        )
        .unwrap();
        steps.media[0].path = made.stored;
    }
    project(&format!(
        "media gone path=\"{}\" rate=16 dur=81f video=832x480 recipe=\"shots/gone.kerfrecipe\"\n\
         sequence s1 rate=16 size=832x480\n",
        steps.media[0].path
    ))
}

/// A recipe carried into the document: its lines become the steps under the media, each
/// file named from the project's folder, and the takes it made are held to the steps it
/// became, so that none is asked for again for being carried.
#[cfg(unix)]
#[test]
fn a_recipe_is_carried_into_the_document_and_its_takes_with_it() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    let before = takes_of_a_recipe(directory, [GONE, GONE]);
    assert_eq!(before.media[0].path, "shots/gone.takes/002.json");
    // Not carried, nothing is made from it, and it says how it is carried. Its takes are
    // there to see and to choose, held to nothing.
    let to_carry = Unmade::Unreadable(not_carried("gone", "shots/gone.kerfrecipe").to_string());
    assert!(to_carry.reason().contains("`kerf carry gone`"));
    assert_eq!(unmade(&before, directory, &before.media[0]), Some(to_carry));
    let listed = takes(&before, directory, &before.media[0]).unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|take| (take.number, take.stored.as_str(), take.chosen))
            .collect::<Vec<_>>(),
        [
            (1, "shots/gone.takes/001.json", false),
            (2, "shots/gone.takes/002.json", true)
        ]
    );
    assert!(
        listed.iter().all(|take| !take.current && !take.kept && take.changed.is_empty()),
        "{listed:?}"
    );

    let carried = carry(&before, directory, "gone").unwrap();
    assert_eq!(carried.recipe, "shots/gone.kerfrecipe");
    assert_eq!(
        carried.script,
        [
            "media replace gone path=\"shots/gone.takes/002.json\" rate=16 dur=00:00:05:01 video=832x480",
            "step gone add generate model=matte prompt=\"man\" guide=footage/walk.json out={man.json}",
            "step gone add generate model=plate prompt=\"an empty path\" guide=footage/walk.json guide_mask={man.json} seed=7 reach=72",
        ]
    );
    assert!(carried.notes.is_empty(), "{:?}", carried.notes);
    let after = kerf_lang::apply_script(&before, kerf_model::Catalog::builtin(), &carried.script.join("\n"))
        .unwrap()
        .project;
    let media = after.media("gone").unwrap();
    assert_eq!((uncarried(media), media.recipe.as_deref()), (None, None));
    // Its takes are where they were. Carried and not yet marked, the take reads as made
    // from something else: its record is of the recipe's text.
    assert_eq!(takes(&after, directory, media).unwrap().len(), 2);
    assert!(unmade(&after, directory, media).is_some());
    assert_eq!(takes::mark_carried(&before, &after, directory, "gone").unwrap(), 2);
    assert_eq!(unmade(&after, directory, media), None);
    let all = takes(&after, directory, media).unwrap();
    assert!(all.iter().all(|take| take.current && take.changed.is_empty()), "{all:?}");
    // Another making goes on from the rolls the recipe had.
    assert_eq!(next_roll(&after, directory, "gone").unwrap(), 2);
    // What a take keeps of its making reads as the steps it became, its seed as it had it.
    let kept = takes::kept(directory, media, 2).unwrap().recipe;
    assert_eq!(
        kept,
        steps_text(&media.steps).replace("seed=7", "seed=8"),
        "the second take was a roll"
    );
    // The footage changing still asks for it.
    std::fs::write(directory.join("footage/walk.json"), "another man").unwrap();
    assert_eq!(
        unmade(&after, directory, media),
        Some(Unmade::Stale(vec!["footage/walk.json".to_owned()]))
    );
    std::fs::write(directory.join("footage/walk.json"), "a man walks").unwrap();
    // Undone, the media names its recipe again, and is again one to carry.
    assert!(matches!(
        unmade(&before, directory, &before.media[0]),
        Some(Unmade::Unreadable(why)) if why.contains("`kerf carry gone`")
    ));
    // The recipe's file is left where it was.
    assert_eq!(std::fs::read_to_string(directory.join("shots/gone.kerfrecipe")).unwrap(), GONE);
}

/// A take a recipe's file made from its text as it was before it was written again was
/// stale already, and carrying the recipe does not make it current.
#[cfg(unix)]
#[test]
fn a_take_that_was_stale_before_its_recipe_was_carried_stays_so() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    let older = GONE.replace("reach=72", "reach=60");
    let before = takes_of_a_recipe(directory, [&older, GONE]);
    let carried = carry(&before, directory, "gone").unwrap();
    let after = kerf_lang::apply_script(&before, kerf_model::Catalog::builtin(), &carried.script.join("\n"))
        .unwrap()
        .project;
    assert_eq!(takes::mark_carried(&before, &after, directory, "gone").unwrap(), 1);
    let media = after.media("gone").unwrap();
    let all = takes(&after, directory, media).unwrap();
    assert!(!all[0].current, "{all:?}");
    assert!(all[0].changed.contains(&"its steps".to_owned()), "{all:?}");
    assert!(all[1].current && all[1].changed.is_empty(), "{all:?}");
    // The media is the second take, which is made from things as they stand.
    assert_eq!(unmade(&after, directory, media), None);
    // Chosen, the first wants making.
    let mut first = after.clone();
    first.media[0].path = "shots/gone.takes/001.json".to_owned();
    assert!(matches!(
        unmade(&first, directory, &first.media[0]),
        Some(Unmade::Stale(what)) if what.contains(&"its steps".to_owned())
    ));
}

/// What does not carry is said: a program to run, which a document cannot name, and a file
/// the recipe said it reads that no step of the document does.
#[test]
fn what_a_recipe_cannot_carry_into_the_document_is_said() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    let recipe = |text: &str| std::fs::write(directory.join("shots/open.kerfrecipe"), text).unwrap();
    let project = project(
        "media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120 recipe=\"shots/open.kerfrecipe\"\n\
         media m2 path=\"shots/b.mp4\" rate=24 dur=1s video=160x120\n\
         \x20 render s2 {out}\n\
         sequence s1 rate=24 size=160x120\n\
         sequence s2 rate=24 size=160x120\n\
         \x20 track V1 video\n\
         \x20   clip car gen=mesh@1 at=0f dur=12f\n\
         \x20     set file=\"models/car.glb\"\n",
    );
    recipe("kerf-recipe 1\nneeds open.scene.json\nrun vork render open.scene.json --out {guide.mp4}\ngenerate model=dawn guide={guide.mp4}\n");
    let ran = carry(&project, directory, "m1").unwrap_err().to_string();
    assert!(ran.contains("shots/open.kerfrecipe: line 3: `run vork` runs a program, and a document names none"), "{ran}");
    // A mesh its guide draws is found from the clip; a file nothing else names is said.
    recipe("kerf-recipe 1\nneeds ../models/car.glb look.cube\nrender s2 {guide.mp4} pass=depth\ngenerate model=dawn guide={guide.mp4} first=../refs/boat.png\n");
    let carried = carry(&project, directory, "m1").unwrap();
    assert_eq!(
        carried.script[1..],
        [
            "step m1 add render s2 {guide.mp4} pass=depth",
            "step m1 add generate model=dawn guide={guide.mp4} first=refs/boat.png",
        ]
    );
    assert_eq!(carried.notes.len(), 1, "{:?}", carried.notes);
    assert!(carried.notes[0].starts_with("`shots/look.cube`, which the recipe said it reads"), "{:?}", carried.notes);
    // Steps that are the document's already have nothing to carry, nor has footage.
    assert!(carry(&project, directory, "m2").unwrap_err().to_string().contains("under its line in the document already"));
    recipe("kerf-recipe 1\nrender s2 {guide.mp4}\n");
    assert!(carry(&project, directory, "m1").unwrap_err().to_string().contains("names no `{out}`"));
}

/// A program that was not written for Kerf, a renderer, is one the person names in their
/// models file as a model is, and is handed its arguments: what a recipe's `run` did, with
/// the program and the shape of its command the person's own and a step only filling it.
#[cfg(unix)]
#[test]
fn a_program_the_person_names_is_handed_its_arguments_by_a_step() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("scenes")).unwrap();
    std::fs::write(directory.join("scenes/open.scene.json"), "a blocking").unwrap();
    // `previs` copies its scene to where its file goes, with its profile after it.
    let some = models(
        directory,
        &format!(
            r#"{{"previs": {{"run": ["sh", "-c", "cat \"$1\" > \"$3\" && printf ' as %s at %s' \"$2\" \"$4\" >> \"$3\"", "previs", "{{ref}}", "{{profile}}", "{{out}}", "{{frame_end}}"],
                            "takes": {{"about": "a rough render of a scene file", "more": {{"profile": "std/previs", "frame_end": 95}}}}}},
                "dawn": {{"run": {ECHO}}}}}"#
        ),
    );
    let project = |steps: &str| {
        project(&format!(
            "media m1 path=\"shots/open.json\" rate=24 dur=1s video=160x120\n{steps}sequence s1 rate=24 size=160x120\n"
        ))
    };
    let shot = project(
        "\x20 generate model=previs ref=scenes/open.scene.json frame_end=47 out={guide.txt}\n\
         \x20 generate model=dawn prompt=\"a boat leaves\" guide={guide.txt} seed=7\n",
    );
    let token = CancellationToken::new();
    let planned = plan(&shot, directory, "m1", &some).unwrap();
    assert!(planned.steps[0].contains("asks model `previs`"), "{}", planned.steps[0]);
    let made = make(&shot, directory, &planned, &some, &token, |_| {}).unwrap();
    // The scene was handed over as a file, the step's own number in place of the entry's,
    // and the entry's own where the step said none.
    let guide = std::fs::read_to_string(directory.join("shots/open.takes/001.work/guide.txt")).unwrap();
    assert_eq!(guide, "a blocking as std/previs at 47");
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join(&made.stored)).unwrap()).unwrap();
    assert!(request["guide"].as_str().unwrap().ends_with("shots/open.takes/001.work/guide.txt"));
    // The scene is a file the take is held to.
    let mut taken = shot.clone();
    taken.media[0].path = made.stored;
    std::fs::write(directory.join("scenes/open.scene.json"), "another blocking").unwrap();
    assert_eq!(
        unmade(&taken, directory, &taken.media[0]),
        Some(Unmade::Stale(vec!["scenes/open.scene.json".to_owned()]))
    );
    // A blank nothing fills is refused with the word that fills it, and a value that
    // would read as a switch of the program is not handed over.
    let wrong = |steps: &str| {
        let shot = project(steps);
        let planned = plan(&shot, directory, "m1", &some).unwrap();
        make(&shot, directory, &planned, &some, &token, |_| {}).unwrap_err().to_string()
    };
    let unfilled = wrong("\x20 generate model=previs\n");
    assert!(unfilled.contains("is handed `{ref}`, and the step gives none: add `ref=<file>`"), "{unfilled}");
    let switch = wrong("\x20 generate model=previs ref=scenes/open.scene.json profile=--exec\n");
    assert!(switch.contains("`profile=--exec` would read as an option"), "{switch}");
    // An entry that names neither its request nor where its file goes is refused.
    let file = directory.join("bad.json");
    std::fs::write(&file, r#"{"previs": {"run": ["vork", "render"]}}"#).unwrap();
    assert!(Models::read(&file).unwrap_err().to_string().contains("names neither `{request}` nor `{out}`"));
}
