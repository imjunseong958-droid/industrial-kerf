//! Collaboration through real repositories: two clones of one bare remote
//! standing in for GitHub.

use kerf_collab::git;
use kerf_collab::scaffold::{self, NewProject, Template};
use kerf_model::Catalog;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

/// Keep the person's global git configuration (signing, hooks, templates)
/// out of the tests.
fn hermetic() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let empty = std::env::temp_dir().join("kerf-collab-empty-gitconfig");
        fs::write(&empty, "").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_AUTHOR_NAME", "Test");
        std::env::set_var("GIT_AUTHOR_EMAIL", "test@example.com");
        std::env::set_var("GIT_COMMITTER_NAME", "Test");
        std::env::set_var("GIT_COMMITTER_EMAIL", "test@example.com");
    });
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A shared project published to a bare remote, and a second clone of it.
struct Team {
    _root: tempfile::TempDir,
    remote: PathBuf,
    alice: PathBuf,
    bob: PathBuf,
}

fn team() -> Team {
    hermetic();
    let root = tempfile::tempdir().unwrap();
    let remote = root.path().join("remote.git");
    fs::create_dir_all(&remote).unwrap();
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let created = scaffold::create(
        &NewProject {
            name: "Short Film".into(),
            location: root.path().join("alice"),
            template: Template::Vertical,
            rate: Some("24".into()),
            git: true,
            remote: Some(remote.to_string_lossy().into_owned()),
            github: None,
        },
        Catalog::builtin(),
    )
    .unwrap();
    let bob = git::clone(&remote.to_string_lossy(), &root.path().join("bob")).unwrap();
    Team {
        alice: created.path,
        bob,
        remote,
        _root: root,
    }
}

#[test]
fn a_new_shared_project_is_published_with_its_document_and_ignore_rules() {
    let team = team();
    let document = fs::read_to_string(team.alice.join("project.kerf")).unwrap();
    assert!(document.contains("project name=\"Short Film\""), "{document}");
    assert!(document.contains("size=1080x1920"), "{document}");
    assert!(document.contains("rate=24"), "{document}");
    assert!(team.alice.join("media").is_dir());
    assert!(team.alice.join("presets").is_dir());
    let ignore = fs::read_to_string(team.alice.join(".gitignore")).unwrap();
    assert!(ignore.contains(".kerf-runtime.*"));
    let attributes = fs::read_to_string(team.alice.join(".gitattributes")).unwrap();
    assert!(attributes.contains("*.kerf merge=kerf"));

    let status = git::status(&team.alice).unwrap();
    assert!(status.repository);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.ahead, status.behind, status.changed), (0, 0, false));
    assert_eq!(
        fs::read_to_string(team.bob.join("project.kerf")).unwrap(),
        document
    );
    assert_eq!(sh(&team.remote, &["log", "--format=%s", "-1"]), "Create Short Film");
}

#[test]
fn runtime_files_are_never_committed() {
    let team = team();
    for name in [".kerf-runtime.journal", ".kerfd.sock", "0123-1.mp4"] {
        fs::write(team.alice.join(name), "local").unwrap();
    }
    fs::create_dir_all(team.alice.join(".kerf/cache/derived")).unwrap();
    fs::write(team.alice.join(".kerf/cache/derived/x"), "cache").unwrap();
    assert!(!git::status(&team.alice).unwrap().changed);
    assert_eq!(git::commit(&team.alice, "nothing").unwrap(), None);
}

#[test]
fn edits_travel_through_push_fetch_and_integrate() {
    let team = team();
    fs::write(team.alice.join("presets/punch.kerfpreset"), "kerf 1\n").unwrap();
    let integration = git::integrate(&team.alice, Path::new("kerfd"), "Add punch preset").unwrap();
    assert!(integration.committed.is_some());
    assert!(integration.incoming.is_empty());
    assert!(git::push(&team.alice).unwrap().pushed);

    assert!(git::fetch(&team.bob).unwrap());
    assert_eq!(git::status(&team.bob).unwrap().behind, 1);
    let integration = git::integrate(&team.bob, Path::new("kerfd"), "").unwrap();
    assert!(integration.fast_forward);
    assert!(!integration.project_changed);
    assert_eq!(integration.incoming.len(), 1);
    assert_eq!(integration.incoming[0].subject, "Add punch preset");
    assert_eq!(integration.incoming[0].author, "Test");
    assert!(team.bob.join("presets/punch.kerfpreset").is_file());
}

#[test]
fn diverged_branches_merge_files_each_side_changed() {
    let team = team();
    fs::write(team.alice.join("presets/a.kerfpreset"), "kerf 1\n").unwrap();
    git::integrate(&team.alice, Path::new("kerfd"), "a").unwrap();
    git::push(&team.alice).unwrap();

    fs::create_dir_all(team.bob.join("presets")).unwrap();
    fs::write(team.bob.join("presets/b.kerfpreset"), "kerf 1\n").unwrap();
    git::fetch(&team.bob).unwrap();
    let integration = git::integrate(&team.bob, Path::new("kerfd"), "b").unwrap();
    assert!(integration.committed.is_some());
    assert!(!integration.fast_forward);
    assert!(team.bob.join("presets/a.kerfpreset").is_file());
    assert!(team.bob.join("presets/b.kerfpreset").is_file());
    assert_eq!(git::status(&team.bob).unwrap().ahead, 2);
    assert!(git::push(&team.bob).unwrap().pushed);
}

#[test]
fn a_merge_git_cannot_finish_is_aborted_and_reported() {
    let team = team();
    fs::write(team.alice.join("presets/same.kerfpreset"), "kerf 1\n# alice\n").unwrap();
    git::integrate(&team.alice, Path::new("kerfd"), "alice").unwrap();
    git::push(&team.alice).unwrap();

    fs::create_dir_all(team.bob.join("presets")).unwrap();
    fs::write(team.bob.join("presets/same.kerfpreset"), "kerf 1\n# bob\n").unwrap();
    git::fetch(&team.bob).unwrap();
    let error = git::integrate(&team.bob, Path::new("kerfd"), "bob").unwrap_err();
    assert_eq!(error.code, "MERGE_FAILED");
    assert!(error.message.contains("presets/same.kerfpreset"), "{}", error.message);
    let status = git::status(&team.bob).unwrap();
    assert!(!status.conflicted);
    assert!(!status.changed);
    assert_eq!(
        fs::read_to_string(team.bob.join("presets/same.kerfpreset")).unwrap(),
        "kerf 1\n# bob\n"
    );
}

#[test]
fn branches_publish_and_switch_back() {
    let team = team();
    let switched = git::switch(&team.alice, "alice/color", true, "").unwrap();
    assert_eq!(switched.branch, "alice/color");
    fs::write(team.alice.join("presets/grade.kerfpreset"), "kerf 1\n").unwrap();
    git::commit(&team.alice, "grade").unwrap();
    let pushed = git::push(&team.alice).unwrap();
    assert!(pushed.set_upstream);
    assert_eq!(
        git::status(&team.alice).unwrap().upstream.as_deref(),
        Some("origin/alice/color")
    );
    git::switch(&team.alice, "main", false, "").unwrap();
    assert!(!team.alice.join("presets/grade.kerfpreset").exists());

    git::fetch(&team.bob).unwrap();
    git::switch(&team.bob, "alice/color", false, "").unwrap();
    assert!(team.bob.join("presets/grade.kerfpreset").is_file());
    assert!(git::switch(&team.bob, "bad name", true, "").is_err());
}

#[test]
fn the_merge_driver_is_configured_for_the_repository() {
    let team = team();
    git::ensure_merge_driver(&team.alice, Path::new("/opt/kerf dir/kerfd")).unwrap();
    assert_eq!(
        sh(&team.alice, &["config", "merge.kerf.driver"]),
        "'/opt/kerf dir/kerfd' merge-driver %O %A %B %P"
    );
    assert_eq!(
        sh(&team.alice, &["check-attr", "merge", "--", "project.kerf"]),
        "project.kerf: merge: kerf"
    );
    git::ensure_merge_driver(&team.alice, Path::new("/opt/kerf dir/kerfd")).unwrap();
    let attributes = fs::read_to_string(team.alice.join(".git/info/attributes")).unwrap();
    assert_eq!(attributes.matches("project.kerf merge=kerf").count(), 1);
}

#[test]
fn files_too_large_for_github_are_refused_unless_lfs_stores_them() {
    let team = team();
    let big = team.alice.join("media/huge.mov");
    fs::File::create(&big)
        .unwrap()
        .set_len(git::LARGE_FILE_BYTES)
        .unwrap();
    let error = git::commit(&team.alice, "huge").unwrap_err();
    assert_eq!(error.code, "LARGE_FILE");
    assert!(error.message.contains("media/huge.mov"), "{}", error.message);
    assert!(sh(&team.alice, &["diff", "--cached", "--name-only"]).is_empty());
}

#[test]
fn a_local_project_without_git_is_just_a_folder() {
    hermetic();
    let root = tempfile::tempdir().unwrap();
    let created = scaffold::create(
        &NewProject {
            name: "Solo".into(),
            location: root.path().to_path_buf(),
            template: Template::Standard,
            rate: None,
            git: false,
            remote: None,
            github: None,
        },
        Catalog::builtin(),
    )
    .unwrap();
    assert!(created.path.join("project.kerf").is_file());
    assert!(!created.path.join(".git").exists());
    assert!(!git::status(&created.path).unwrap().repository);
    let again = scaffold::create(
        &NewProject {
            name: "Solo".into(),
            location: root.path().to_path_buf(),
            template: Template::Standard,
            rate: None,
            git: false,
            remote: None,
            github: None,
        },
        Catalog::builtin(),
    );
    assert_eq!(again.unwrap_err().code, "ALREADY_EXISTS");
}

#[test]
fn names_and_urls_become_folders_and_github_pages() {
    assert_eq!(
        git::repository_name("https://github.com/kerf/short-film.git").as_deref(),
        Some("short-film")
    );
    assert_eq!(
        git::repository_name("git@github.com:kerf/short-film.git").as_deref(),
        Some("short-film")
    );
    assert_eq!(
        git::github_web_url("git@github.com:kerf/short-film.git").as_deref(),
        Some("https://github.com/kerf/short-film")
    );
    assert_eq!(
        git::github_web_url("https://github.com/kerf/short-film").as_deref(),
        Some("https://github.com/kerf/short-film")
    );
    assert_eq!(git::github_web_url("https://gitlab.com/kerf/x.git"), None);
    assert_eq!(scaffold::repository_slug("내 영상 Project!"), "Project");
    assert_eq!(scaffold::folder_name("a/b: c").unwrap(), "a-b- c");
    assert!(scaffold::folder_name("  ").is_err());
}

#[test]
fn lfs_pointers_are_told_apart_from_media() {
    let root = tempfile::tempdir().unwrap();
    let pointer = root.path().join("a.mov");
    fs::write(
        &pointer,
        "version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 12345\n",
    )
    .unwrap();
    assert!(git::is_lfs_pointer(&pointer));
    let media = root.path().join("b.mov");
    fs::write(&media, [0u8; 16]).unwrap();
    assert!(!git::is_lfs_pointer(&media));
}
