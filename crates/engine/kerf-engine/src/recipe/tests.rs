#[cfg(unix)]
use super::model::{Catalog, CatalogModel, Generation, Generator};
use super::*;
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

/// A generator of the models `names`, each of whose files `make` writes.
#[cfg(unix)]
struct Closure<F> {
    catalog: Catalog,
    make: F,
}

#[cfg(unix)]
impl<F: Fn(&Generation) -> Result<()>> Closure<F> {
    fn new(names: &[&str], make: F) -> Self {
        let models = names
            .iter()
            .map(|name| CatalogModel {
                provider: (*name).to_owned(),
                name: (*name).to_owned(),
                spec: None,
                by: "a test".to_owned(),
            })
            .collect();
        Self {
            catalog: Catalog {
                models,
                ..Catalog::default()
            },
            make,
        }
    }
}

#[cfg(unix)]
impl<F: Fn(&Generation) -> Result<()>> Generator for Closure<F> {
    fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn generate(&self, generation: &Generation, _: &mut dyn FnMut(&str)) -> Result<()> {
        (self.make)(generation)
    }
}

#[cfg(unix)]
#[test]
fn a_model_is_found_by_its_name_or_with_its_providers_where_two_have_it() {
    let model = |provider: &str, name: &str| CatalogModel {
        provider: provider.to_owned(),
        name: name.to_owned(),
        spec: None,
        by: provider.to_owned(),
    };
    let catalog = Catalog {
        models: vec![
            model("pod", "wan-vace"),
            model("farm", "wan-vace"),
            model("pod", "still"),
        ],
        ..Catalog::default()
    };
    assert_eq!(catalog.find("still").unwrap().provider, "pod");
    assert_eq!(catalog.find("farm/wan-vace").unwrap().provider, "farm");
    let shared = catalog.find("wan-vace").unwrap_err().to_string();
    assert!(
        shared.contains("`pod/wan-vace` or `farm/wan-vace`"),
        "{shared}"
    );
    assert!(matches!(
        catalog.find("dawn"),
        Err(EngineError::NoModel { name, .. }) if name == "dawn"
    ));
    assert_eq!(catalog.names(), ["pod/wan-vace", "farm/wan-vace", "still"]);
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
    // Models of the person's: one that copies the file it is handed to where its file goes,
    // and ones that fail, or end well and write nothing.
    let some = Closure::new(&["copy", "fail", "idle"], |generation: &Generation| {
        let request = generation.request;
        match request.model.as_str() {
            "copy" => {
                std::fs::copy(&request.references[0], &request.out)?;
                Ok(())
            }
            "fail" => Err(EngineError::msg("the model failed")),
            _ => Ok(()),
        }
    });
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

    // A making that fails, writes nothing, or asks a model no one has set up leaves no take and
    // nothing of one.
    for (steps, why) in [
        ("generate model=fail", "the model failed"),
        ("generate model=idle", "wrote nothing"),
        ("generate model=absent", "no model `absent` is set up"),
        (
            "generate model=copy ref=shots/scene.txt out={guide.txt}\ngenerate model=fail",
            "the model failed",
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
