//! The launcher: new projects, presets, recent projects.

use crate::support::{event, EXISTS};
use kerf_bridge::event::{InspectedSlot, PresetMade, PresetSlot, SlotClip};
use kerf_bridge::{CheckedFile, GitTools, PresetCheck};
use kerf_editor::folders::needs_start;
use kerf_editor::launcher::{
    clone_problem, details_problem, git_address, github_account, launcher_escape, tools_hint,
    LauncherEscape, Mode, Plan,
};
use kerf_editor::preset_file::{existing_preset, write_document};
use kerf_editor::presets::{
    export_path, file_fit, import_refusal, made_kind, made_line, needs_line, presets_in,
    read_by_line, seconds, slot_read_by, start_frame, uses_line, Fit,
};
use kerf_editor::workspace::folder_media;
use proptest::prelude::*;
use serde_json::json;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- the launcher

#[test]
fn esc_gives_a_launcher_opened_from_an_editor_back() {
    use LauncherEscape::*;
    // A form open goes first, wherever the launcher came from.
    assert_eq!(launcher_escape(true, false, true), PutFormAway);
    assert_eq!(launcher_escape(true, false, false), PutFormAway);
    // A form whose task runs stays.
    assert_eq!(launcher_escape(true, true, true), Nothing);
    // No form: from an editor, it is given up; the first-run one stays.
    assert_eq!(launcher_escape(false, false, true), GiveBack);
    assert_eq!(launcher_escape(false, true, true), GiveBack);
    assert_eq!(launcher_escape(false, false, false), Nothing);
    assert_eq!(launcher_escape(false, true, false), Nothing);
}

proptest! {
    #[test]
    fn the_launcher_the_editor_starts_with_is_never_given_up(
        form in proptest::bool::ANY, busy in proptest::bool::ANY,
    ) {
        prop_assert_ne!(launcher_escape(form, busy, false), LauncherEscape::GiveBack);
    }
}

// ---------------------------------------------------------------- presets' words

#[test]
fn a_refusal_to_save_over_a_preset_names_it() {
    assert_eq!(
        existing_preset(
            "presets/Intro.kerfpreset already exists; choose another name or replace it"
        ),
        Some("presets/Intro.kerfpreset")
    );
    assert_eq!(
        existing_preset("cannot save presets/x.kerfpreset: denied"),
        None
    );
    assert_eq!(existing_preset(EXISTS), None);
}

proptest! {
    #[test]
    fn any_preset_path_comes_back_from_its_refusal(path in "[a-zA-Z0-9 /._-]{1,40}") {
        prop_assume!(!path.trim().is_empty());
        let message = format!("{path}{EXISTS}");
        prop_assert_eq!(existing_preset(&message), Some(path.as_str()));
    }

    #[test]
    fn an_export_path_is_a_preset_file_and_stays_one(stem in "[a-zA-Z0-9 _-]{1,20}", dot in proptest::bool::ANY) {
        let typed = if dot { format!("/tmp/{stem}.") } else { format!("/tmp/{stem}") };
        let path = export_path(Path::new(&typed)).expect("a name with no extension takes one");
        prop_assert_eq!(path.extension().and_then(|e| e.to_str()), Some("kerfpreset"));
        prop_assert_eq!(export_path(&path), Ok(path.clone()));
        prop_assert!(!path.to_string_lossy().contains(".."));
    }

    #[test]
    fn a_start_never_leaves_the_media(seconds in -1e6f64..1e6, fps in -10f64..240.0, latest in -100i64..100_000) {
        let frame = start_frame(seconds, fps, latest);
        prop_assert!(frame >= 0);
        prop_assert!(frame <= latest.max(0));
    }

    #[test]
    fn a_typed_start_reads_back(value in 0f64..10_000.0) {
        let typed = format!("{value:.3}");
        let read = seconds(&typed).expect("a number of seconds reads");
        prop_assert!((read - value).abs() < 0.001);
        prop_assert_eq!(seconds(&format!("{typed} s")), Some(read));
    }
}

/// A media a preset makes, as the bridge reads it from the file.
fn made(id: &str, steps: &[&str], of: Option<&str>, clips: usize) -> PresetMade {
    PresetMade {
        id: id.into(),
        of: of.map(str::to_owned),
        steps: steps.iter().map(|step| (*step).to_owned()).collect(),
        clips,
    }
}

/// A media slot, as the bridge reads it from the file.
fn slot(id: &str, seconds: f64, video: bool, audio: bool, clips: usize) -> PresetSlot {
    PresetSlot {
        id: id.into(),
        seconds,
        video,
        audio,
        clips,
    }
}

#[test]
fn what_a_made_media_is_reads_from_its_steps() {
    let made = |steps: &[&str], of: Option<&str>| made("m", steps, of, 0);
    assert_eq!(made_kind(&made(&[], None)), "made from its steps");
    assert_eq!(made_kind(&made(&[], Some("slot1"))), "made of slot1");
    assert_eq!(
        made_kind(&made(
            &["step m add generate model=upscaler"],
            Some("slot1")
        )),
        "an upscaler of slot1"
    );
    assert_eq!(
        made_kind(&made(
            &["render guide", "step m add generate model=flux seed=4"],
            None
        )),
        "a flux from a guide sequence"
    );
    assert_eq!(
        made_kind(&made(&["step m add generate model=flux"], None)),
        "a flux"
    );
    // `model=` inside another word is not a model.
    assert_eq!(
        made_kind(&made(&["step m add generate submodel=flux"], None)),
        "made from its steps"
    );
    assert_eq!(
        made_line(&self::made("", &[], Some("slot2"), 2)),
        "Made of slot2 · 2 clips"
    );
}

#[test]
fn a_slot_is_read_by_the_media_whose_steps_name_it() {
    let preset = [
        made(
            "sky",
            &["step sky add generate model=flux image=slot:slot1"],
            None,
            0,
        ),
        made(
            "sea",
            &["step sea add grade from=slot:slot10, gain=2"],
            None,
            0,
        ),
        made("sun", &["step sun add blend a=slot:slot1}"], None, 0),
    ];
    assert_eq!(
        slot_read_by(&preset, &slot("slot1", 0.0, false, false, 0)),
        ["sky", "sun"],
        "slot:slot10 is another slot"
    );
    assert_eq!(
        slot_read_by(&preset, &slot("slot10", 0.0, false, false, 0)),
        ["sea"]
    );
    // The bridge's own answer wins where it gives one.
    let said = InspectedSlot {
        id: "slot1".into(),
        read_by: vec!["only".into()],
        ..InspectedSlot::default()
    };
    assert_eq!(slot_read_by(&preset, &said), ["only"]);
    assert_eq!(read_by_line(&[]), "");
    assert_eq!(read_by_line(&["sky".into()]), "sky is made of this");
    assert_eq!(
        read_by_line(&["sky".into(), "sea".into(), "sun".into()]),
        "sky, sea and sun are made of this"
    );
}

#[test]
fn a_slot_says_what_it_needs_and_where_it_plays() {
    assert_eq!(
        needs_line(&slot("slot1", 2.0, true, true, 3)),
        "Needs 2.00 s · picture and sound · 3 clips"
    );
    assert_eq!(
        needs_line(&slot("slot2", 4.5, false, true, 1)),
        "Needs 4.50 s · sound · 1 clip"
    );
    let place = |track: &str, at: &str, sequence: &str, name: Option<&str>| SlotClip {
        track: track.into(),
        at: at.into(),
        sequence: sequence.into(),
        name: name.map(str::to_owned),
        ..SlotClip::default()
    };
    let slot = InspectedSlot {
        clips: vec![
            place("V1", "00:00:01:00", "main", Some("Open")),
            place("A2", "00:00:04:00", "titles", None),
        ],
        ..InspectedSlot::default()
    };
    assert_eq!(
        uses_line(&slot, "main"),
        "V1 00:00:01:00 “Open” · titles › A2 00:00:04:00"
    );
}

#[test]
fn a_chosen_file_fits_a_slot_or_says_why_not() {
    let picture = slot("slot1", 4.0, true, false, 0);
    let sound = slot("slot3", 4.0, false, true, 0);
    let files = vec![PathBuf::from("/m/a.mov"), PathBuf::from("/m/b.wav")];
    let checked = |files: Vec<CheckedFile>| {
        Ok(PresetCheck {
            files,
            ..PresetCheck::default()
        })
    };
    let probed = |path: &str, seconds: f64, video: bool, audio: bool, fits: &[&str]| CheckedFile {
        path: path.into(),
        seconds,
        video,
        audio,
        fits: fits.iter().map(|fit| (*fit).to_owned()).collect(),
        error: None,
    };
    let answer = checked(vec![
        probed("/m/a.mov", 6.0, true, false, &["slot1"]),
        probed("/m/b.wav", 3.0, false, true, &[]),
    ]);
    let fit = file_fit(&answer, &files, &picture, Path::new("/m/a.mov"));
    assert!(fit.fits);
    assert_eq!(fit.words, "✓ a.mov · 6.00 s");
    assert_eq!(fit.spare, 2.0);
    let fit = file_fit(&answer, &files, &picture, Path::new("/m/b.wav"));
    assert!(!fit.fits);
    assert_eq!(fit.words, "✕ b.wav has no picture");
    assert_eq!(fit.spare, 0.0);
    assert_eq!(
        file_fit(&answer, &files, &sound, Path::new("/m/a.mov")).words,
        "✕ a.mov has no sound"
    );
    assert_eq!(
        file_fit(&answer, &files, &sound, Path::new("/m/b.wav")).words,
        "✕ b.wav is shorter than 4.00 s"
    );
    // A probe's error says its first line; a file the answer leaves out is unread.
    let failed = checked(vec![CheckedFile {
        path: "/m/a.mov".into(),
        error: Some("ffmpeg failed\nargs: -v quiet".into()),
        ..CheckedFile::default()
    }]);
    assert_eq!(
        file_fit(&failed, &files, &picture, Path::new("/m/a.mov")).words,
        "✕ a.mov: ffmpeg failed"
    );
    assert_eq!(
        file_fit(&checked(vec![]), &files, &picture, Path::new("/m/a.mov")).words,
        "✕ Kerf could not read a.mov"
    );
    assert_eq!(
        file_fit(
            &Err("no such preset\nmore".into()),
            &files,
            &picture,
            Path::new("/m/a.mov")
        )
        .words,
        "✕ a.mov could not be checked: no such preset"
    );
}

#[test]
fn the_presets_of_recent_projects_are_found_by_name() {
    let root = tempfile::tempdir().unwrap();
    let film = root.path().join("film");
    let promo = root.path().join("promo");
    std::fs::create_dir_all(film.join("presets")).unwrap();
    std::fs::create_dir_all(promo.join("presets")).unwrap();
    for (folder, name) in [
        (&film, "Teaser.kerfpreset"),
        (&film, "Intro.kerfpreset"),
        (&film, "notes.txt"),
        (&promo, "Promo cut.kerfpreset"),
    ] {
        std::fs::write(folder.join("presets").join(name), "kerf 1\n").unwrap();
    }
    let found = presets_in(&[film.clone(), root.path().join("gone"), promo.clone()]);
    let named: Vec<(&str, &str)> = found
        .iter()
        .map(|choice| (choice.name.as_str(), choice.project.as_str()))
        .collect();
    assert_eq!(
        named,
        [
            ("Intro", "film"),
            ("Teaser", "film"),
            ("Promo cut", "promo")
        ]
    );
    assert_eq!(found[0].path, film.join("presets/Intro.kerfpreset"));
}

#[test]
fn an_exported_document_is_written_whole_and_replaces_the_old() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("editing.kerfpreset");
    std::fs::write(&path, "old and much longer than what replaces it").unwrap();
    write_document(&path, "kerf 1\nproject name=\"x\"\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "kerf 1\nproject name=\"x\"\n"
    );
    let left: Vec<_> = std::fs::read_dir(root.path()).unwrap().collect();
    assert_eq!(left.len(), 1, "nothing is left beside it");
    assert!(write_document(&root.path().join("missing/editing.kerfpreset"), "x").is_err());
}

// ---------------------------------------------------------------- the launcher

#[test]
fn create_says_why_it_cannot_make_the_project_yet() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("Taken")).unwrap();
    std::fs::write(root.path().join("Taken/project.kerf"), "kerf 1\n").unwrap();
    let at = root.path();
    assert_eq!(
        details_problem("  ", at, "").as_deref(),
        Some("Name the project")
    );
    assert_eq!(
        details_problem("Film", Path::new(""), "").as_deref(),
        Some("Choose where to save the project")
    );
    assert!(details_problem("Film", Path::new("Videos/Kerf"), "")
        .unwrap()
        .starts_with("Choose a full location, such as "));
    assert_eq!(
        details_problem("Taken", at, "").as_deref(),
        Some("“Taken” is already in that location; choose another name")
    );
    assert_eq!(
        details_problem("Film", at, "not an address").as_deref(),
        Some("The remote must be a Git address")
    );
    assert_eq!(
        details_problem("Film", at, "git@github.com:you/film.git"),
        None
    );
    // An empty folder of that name is no obstacle.
    std::fs::create_dir_all(at.join("Empty")).unwrap();
    assert_eq!(details_problem("Empty", at, ""), None);
}

#[test]
fn clone_says_why_it_cannot_clone_yet() {
    let root = tempfile::tempdir().unwrap();
    let at = root.path();
    std::fs::create_dir_all(at.join("film/media")).unwrap();
    assert_eq!(
        clone_problem("", at).as_deref(),
        Some("Enter the repository's address")
    );
    assert_eq!(
        clone_problem("not a url", at).as_deref(),
        Some("Enter a Git address such as https://github.com/you/film.git")
    );
    assert_eq!(
        clone_problem("https://github.com/you/film.git", Path::new("rel")).as_deref(),
        Some("Choose where to put the copy")
    );
    assert_eq!(
        clone_problem("https://github.com/you/film.git", at).as_deref(),
        Some("“film” is already in that location")
    );
    assert_eq!(clone_problem("git@github.com:you/other.git", at), None);
}

#[test]
fn the_sharing_hint_says_what_is_missing() {
    assert_eq!(
        tools_hint(None, None),
        "Looking for Git and the GitHub CLI…"
    );
    assert_eq!(
        tools_hint(None, Some("boom")),
        "Kerf could not look for Git and the GitHub CLI: boom"
    );
    // What the bridge found: Git, Git LFS, the GitHub CLI and who it is signed in as.
    let tools = |git, git_lfs, gh, user: Option<&str>| GitTools {
        git,
        git_lfs,
        gh,
        gh_user: user.map(str::to_owned),
        ..GitTools::default()
    };
    assert!(tools_hint(Some(&tools(false, false, false, None)), None)
        .starts_with("Install Git to share projects"));
    let signed_in = tools(true, true, true, Some("kerf-tester"));
    assert_eq!(
        tools_hint(Some(&signed_in), None),
        "Signed in to GitHub as @kerf-tester."
    );
    assert_eq!(github_account(&signed_in), Some("kerf-tester"));
    let signed_out = tools(true, false, true, None);
    assert_eq!(
        tools_hint(Some(&signed_out), None),
        "Sign in with `gh auth login` to create a GitHub repository here. Git LFS is not installed: media of 100 MB or more cannot be shared."
    );
    assert_eq!(github_account(&signed_out), None);
    let no_gh = tools(true, true, false, Some("stale"));
    assert!(tools_hint(Some(&no_gh), None).starts_with("Install the GitHub CLI"));
    assert_eq!(github_account(&no_gh), None);
}

#[test]
fn the_create_task_carries_sharing_the_preset_and_kerfd() {
    let base = Plan {
        name: " Film ".into(),
        location: PathBuf::from("/v/Kerf"),
        template: "vertical".into(),
        rate: "25".into(),
        ..Plan::default()
    };
    let plain = base.request();
    assert_eq!(
        plain,
        json!({"type": "create", "name": "Film", "location": "/v/Kerf",
            "template": "vertical", "rate": "25", "git": false})
    );
    // GitHub wins over a remote, and either turns Git on.
    let github = Plan {
        github: true,
        remote: "git@example.com:me/film.git".into(),
        ..base.clone()
    }
    .request();
    assert_eq!(github["git"], true);
    assert_eq!(github["github"], json!({"private": true}));
    assert!(github.get("remote").is_none());
    let remote = Plan {
        remote: " git@example.com:me/film.git ".into(),
        ..base.clone()
    }
    .request();
    assert_eq!(remote["git"], true);
    assert_eq!(remote["remote"], "git@example.com:me/film.git");
    // A kerfd named by its path is passed on; a bare name is looked up as
    // the bridge would.
    let kerfd = Plan {
        kerfd: Some(PathBuf::from("/opt/kerf/kerfd")),
        preset: Some(json!({"path": "/p/a.kerfpreset", "bindings": {}})),
        ..base.clone()
    }
    .request();
    assert_eq!(kerfd["kerfd"], "/opt/kerf/kerfd");
    assert_eq!(kerfd["preset"]["path"], "/p/a.kerfpreset");
    let bare = Plan {
        kerfd: Some(PathBuf::from("kerfd")),
        ..base
    }
    .request();
    assert!(bare.get("kerfd").is_none());
}

#[test]
fn the_launchers_way_is_kept_by_its_name() {
    for mode in [Mode::New, Mode::Preset, Mode::Clone] {
        assert_eq!(Mode::from_wire(mode.wire()), Some(mode));
    }
    assert_eq!(Mode::from_wire(""), None);
    assert_eq!(Mode::from_wire("recent"), None);
    assert!(git_address("https://github.com/you/film.git"));
    assert!(!git_address("film"));
}

// ---------------------------------------------------------------- review fixes

fn fit(fits: bool, spare: f64) -> Fit {
    Fit {
        fits,
        words: String::new(),
        spare,
    }
}

#[test]
fn a_start_is_asked_for_only_while_its_field_shows() {
    // A file with length to spare: the start is read, and kept to the spare.
    let spare = fit(true, 2.0);
    assert!(spare.start_open());
    assert_eq!(spare.start(""), Some(0.0));
    assert_eq!(spare.start("1.5 s"), Some(1.5));
    assert_eq!(spare.start("3"), Some(2.0));
    assert_eq!(spare.start("1,5"), None);
    // A file that exactly fits hides the field: whatever it still says
    // (typed for the file chosen before) holds nothing back.
    let exact = fit(true, 0.0);
    assert!(!exact.start_open());
    assert_eq!(exact.start("1,5"), Some(0.0));
    assert_eq!(exact.start("4"), Some(0.0));
    let refused = fit(false, 0.0);
    assert!(!refused.start_open());
    assert_eq!(refused.start("x"), Some(0.0));
}

proptest! {
    #[test]
    fn a_start_is_never_past_the_spare(typed in "\\PC{0,8}", spare in 0.0f64..100.0, fits: bool) {
        let fit = fit(fits, spare);
        match fit.start(&typed) {
            Some(from) => prop_assert!((0.0..=spare).contains(&from)),
            // Only a field that shows can hold Create back.
            None => prop_assert!(fit.start_open() && seconds(&typed).is_none()),
        }
        if !fit.start_open() {
            prop_assert_eq!(fit.start(&typed), Some(0.0));
        }
    }
}

#[test]
fn an_import_refused_is_said_under_its_slot() {
    assert_eq!(
        import_refusal(&event(json!({"type": "error",
            "message": "cannot import: not a media file\nffprobe: invalid data"})))
        .as_deref(),
        Some("cannot import: not a media file")
    );
    assert_eq!(
        import_refusal(&event(json!({"type": "error", "message": ""}))).as_deref(),
        Some("The file could not be imported")
    );
    // Kerf's own errors, a refused render, and other events are not it.
    assert_eq!(
        import_refusal(&event(
            json!({"type": "error", "command": "agent", "message": "x"})
        )),
        None
    );
    assert_eq!(
        import_refusal(&event(
            json!({"type": "error", "command": "export", "message": "x"})
        )),
        None
    );
    assert_eq!(
        import_refusal(&event(json!({"type": "state", "snapshot": {}}))),
        None
    );
}

#[cfg(unix)]
#[test]
fn a_name_that_is_not_utf8_is_refused_not_sent() {
    use kerf_editor::presets::path_refusal;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let odd = PathBuf::from(OsStr::from_bytes(b"/m/a\xff.mov"));
    assert_eq!(
        path_refusal(&odd).as_deref(),
        Some("Kerf cannot use “/m/a\u{FFFD}.mov”: its name is not UTF-8")
    );
    // The create task does not stop the editor over one: it names it as
    // near as text can.
    let request = Plan {
        name: "Film".into(),
        location: odd.clone(),
        kerfd: Some(odd.clone()),
        ..Plan::default()
    }
    .request();
    assert_eq!(request["location"], "/m/a\u{FFFD}.mov");
    assert_eq!(request["kerfd"], "/m/a\u{FFFD}.mov");
    // A preset whose name is not text is not offered: the bridge could not
    // be asked to read it.
    let root = tempfile::tempdir().unwrap();
    let presets = root.path().join("Film/presets");
    std::fs::create_dir_all(&presets).unwrap();
    std::fs::write(presets.join("Intro.kerfpreset"), "{}").unwrap();
    let mut name = b"Odd\xff".to_vec();
    name.extend_from_slice(b".kerfpreset");
    if std::fs::write(presets.join(OsStr::from_bytes(&name)), "{}").is_ok() {
        let found = presets_in(&[root.path().join("Film")]);
        let names: Vec<&str> = found.iter().map(|choice| choice.name.as_str()).collect();
        assert_eq!(names, ["Intro"]);
    }
}

#[test]
fn a_folder_without_project_kerf_starts_with_its_media() {
    let folder = tempfile::tempdir().unwrap();
    for name in ["b.MOV", "a.mp4", "notes.txt", "song.wav"] {
        std::fs::write(folder.path().join(name), b"").unwrap();
    }
    std::fs::create_dir(folder.path().join("clips.mp4")).unwrap();
    assert!(needs_start(folder.path()));
    let names: Vec<String> = folder_media(folder.path())
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["a.mp4", "b.MOV", "song.wav"]);

    std::fs::write(folder.path().join("project.kerf"), "kerf 1\n").unwrap();
    assert!(!needs_start(folder.path()));
}
