use super::*;
// Only the tests that run models use these, and their models are Unix programs.
#[cfg(unix)]
use super::model::Models;
#[cfg(unix)]
use kerf_model::Project;

/// A project of `lines` after its header, read as a document is.
#[cfg(unix)]
fn project(lines: &str) -> Project {
    kerf_lang::parse_document(
        &format!("kerf 1\nproject name=\"shots\"\n{lines}"),
        kerf_model::Catalog::builtin(),
    )
    .expect("a project")
    .project
}

/// `steps`, a step a line, as the document holds them under a media's line.
#[cfg(unix)]
fn under(steps: &str) -> String {
    steps
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| format!("  {line}\n"))
        .collect()
}

/// A project whose media `m1`, its file at `path`, is made by `steps`, a step a line.
#[cfg(unix)]
fn shot(path: &str, steps: &str) -> Project {
    project(&format!(
        "media m1 path=\"{path}\" rate=24 dur=1s video=160x120\n{}",
        under(steps)
    ))
}

/// A models file in `folder` naming `models`, read.
#[cfg(unix)]
fn models(folder: &Path, models: &str) -> Models {
    let file = folder.join("models.json");
    std::fs::write(&file, models).unwrap();
    Models::read(&file).unwrap()
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
    let project = shot(
        "shots/open.mp4",
        "generate model=slow prompt=\"a boat leaves\"",
    );
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
